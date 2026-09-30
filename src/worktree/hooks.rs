//! Repo settings applied around the worktrees Herdr creates and removes:
//! files copied from the main checkout and commands run in the worktree.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

/// Longest a repo command may run before Herdr stops it.
const HOOK_TIMEOUT: Duration = Duration::from_secs(60 * 60);
const POLL_INTERVAL: Duration = Duration::from_millis(200);
/// Upper bound on files copied for one pattern, so `*` in a huge folder
/// cannot stall worktree creation.
const MAX_COPIED_FILES: usize = 1000;

/// One repo command run in a worktree, with its output kept in a log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorktreeHook {
    pub command: String,
    pub cwd: PathBuf,
    pub env: Vec<(&'static str, String)>,
    pub log_path: PathBuf,
}

impl WorktreeHook {
    /// Runs the command to completion. An error carries the last line the
    /// command printed, or how it ended when it printed nothing.
    pub(crate) fn run(&self) -> Result<(), String> {
        if let Some(parent) = self.log_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|err| format!("couldn't create {}: {err}", parent.display()))?;
        }
        let mut log = std::fs::File::create(&self.log_path)
            .map_err(|err| format!("couldn't write {}: {err}", self.log_path.display()))?;
        let _ = writeln!(log, "$ {}", self.command);
        let stdout = log.try_clone().map_err(|err| err.to_string())?;
        let stderr = log.try_clone().map_err(|err| err.to_string())?;
        let mut process = crate::platform::detached_custom_command_process(&self.command);
        process
            .current_dir(&self.cwd)
            .stdin(Stdio::null())
            .stdout(stdout)
            .stderr(stderr)
            .envs(self.env.iter().map(|(key, value)| (*key, value)));
        let mut child = process
            .spawn()
            .map_err(|err| format!("couldn't start the command: {err}"))?;
        let started = Instant::now();
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if started.elapsed() >= HOOK_TIMEOUT => {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = writeln!(
                        log,
                        "herdr: stopped after {} minutes",
                        HOOK_TIMEOUT.as_secs() / 60
                    );
                    return Err(format!(
                        "stopped after {} minutes",
                        HOOK_TIMEOUT.as_secs() / 60
                    ));
                }
                Ok(None) => std::thread::sleep(POLL_INTERVAL),
                Err(err) => return Err(format!("couldn't wait for the command: {err}")),
            }
        };
        if status.success() {
            return Ok(());
        }
        let ended = match status.code() {
            Some(code) => format!("exited with status {code}"),
            None => "was stopped by a signal".to_owned(),
        };
        let _ = writeln!(log, "herdr: the command {ended}");
        Err(last_output_line(&self.log_path).unwrap_or(ended))
    }
}

/// The last non-empty line a command printed, skipping Herdr's own lines.
fn last_output_line(log_path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(log_path).ok()?;
    text.lines()
        .skip(1)
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("herdr: "))
        .last()
        .map(|line| line.chars().take(200).collect())
}

/// Where the output of `phase` (`create` or `remove`) for one worktree
/// goes under `log_dir`, laid out like the worktrees themselves.
pub(crate) fn hook_log_path(
    log_dir: &Path,
    space: &str,
    repo: &str,
    name: &str,
    phase: &str,
) -> PathBuf {
    log_dir
        .join(super::path_component(space))
        .join(super::path_component(repo))
        .join(format!("{}-{phase}.log", super::path_component(name)))
}

/// What a repo command knows about the worktree it runs in.
pub(crate) fn hook_env(
    repo: &crate::repos::Repo,
    checkout: &Path,
    branch: &str,
    space: &str,
) -> Vec<(&'static str, String)> {
    let root = repo.root_path().display().to_string();
    vec![
        ("HERDR_REPO", repo.name.clone()),
        ("HERDR_REPO_ROOT", root.clone()),
        // The name the worktree-setup plugin used for the main checkout.
        ("HERDR_MAIN_REPO", root),
        ("HERDR_WORKTREE", checkout.display().to_string()),
        ("HERDR_BRANCH", branch.to_owned()),
        ("HERDR_BASE_BRANCH", repo.base_branch.clone()),
        ("HERDR_SPACE", space.to_owned()),
    ]
}

/// Copies what `patterns` name from `root` into `checkout` at the same
/// relative paths, leaving files the checkout already has. Missing sources
/// are skipped quietly; other problems come back as warnings.
pub(crate) fn copy_repo_files(root: &Path, checkout: &Path, patterns: &[String]) -> Vec<String> {
    let mut warnings = Vec::new();
    for pattern in patterns {
        let mut copied = 0;
        for relative in matching_paths(root, pattern) {
            if copied >= MAX_COPIED_FILES {
                warnings.push(format!(
                    "copied only the first {MAX_COPIED_FILES} files matching {pattern}"
                ));
                break;
            }
            if let Err(err) = copy_entry(
                &root.join(&relative),
                &checkout.join(&relative),
                &mut copied,
            ) {
                warnings.push(format!("couldn't copy {}: {err}", relative.display()));
            }
        }
    }
    warnings
}

fn copy_entry(source: &Path, target: &Path, copied: &mut usize) -> std::io::Result<()> {
    if source.is_dir() {
        std::fs::create_dir_all(target)?;
        for entry in std::fs::read_dir(source)? {
            if *copied >= MAX_COPIED_FILES {
                break;
            }
            let entry = entry?;
            copy_entry(&entry.path(), &target.join(entry.file_name()), copied)?;
        }
        return Ok(());
    }
    if target.exists() {
        return Ok(());
    }
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::copy(source, target)?;
    *copied += 1;
    Ok(())
}

/// Paths under `root` matching `pattern`, relative to `root`. `*` and `?`
/// match within one path component; `.` and `..` never match.
fn matching_paths(root: &Path, pattern: &str) -> Vec<PathBuf> {
    let mut found = vec![PathBuf::new()];
    for component in pattern.split('/').filter(|component| !component.is_empty()) {
        let mut next = Vec::new();
        for base in &found {
            if !component.contains(['*', '?']) {
                let candidate = base.join(component);
                if root.join(&candidate).exists() {
                    next.push(candidate);
                }
                continue;
            }
            let Ok(entries) = std::fs::read_dir(root.join(base)) else {
                continue;
            };
            let mut names = entries
                .flatten()
                .filter_map(|entry| entry.file_name().into_string().ok())
                .filter(|name| wildcard_match(component, name))
                .collect::<Vec<_>>();
            names.sort();
            next.extend(names.into_iter().map(|name| base.join(name)));
        }
        found = next;
    }
    found.retain(|path| !path.as_os_str().is_empty());
    found
}

fn wildcard_match(pattern: &str, name: &str) -> bool {
    let pattern = pattern.chars().collect::<Vec<_>>();
    let name = name.chars().collect::<Vec<_>>();
    let (mut p, mut n) = (0, 0);
    let (mut star, mut resume) = (None, 0);
    while n < name.len() {
        match pattern.get(p) {
            Some('?') => {
                p += 1;
                n += 1;
            }
            Some('*') => {
                star = Some(p);
                resume = n;
                p += 1;
            }
            Some(&ch) if ch == name[n] => {
                p += 1;
                n += 1;
            }
            _ => match star {
                Some(star) => {
                    p = star + 1;
                    resume += 1;
                    n = resume;
                }
                None => return false,
            },
        }
    }
    pattern[p..].iter().all(|ch| *ch == '*')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "herdr-hooks-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn wildcards_match_within_one_component() {
        assert!(wildcard_match(".env*", ".env"));
        assert!(wildcard_match(".env*", ".env.local"));
        assert!(!wildcard_match(".env*", "env"));
        assert!(wildcard_match("*.toml", "dev.toml"));
        assert!(wildcard_match("a?c", "abc"));
        assert!(!wildcard_match("a?c", "ac"));
        assert!(wildcard_match("*", "anything"));
    }

    #[test]
    fn copies_matching_files_without_overwriting_the_checkout() {
        let root = temp_dir("root");
        let checkout = temp_dir("checkout");
        std::fs::write(root.join(".env"), "SECRET=1").unwrap();
        std::fs::write(root.join(".env.local"), "LOCAL=1").unwrap();
        std::fs::create_dir_all(root.join("config/local")).unwrap();
        std::fs::write(root.join("config/local/dev.toml"), "dev").unwrap();
        std::fs::write(checkout.join(".env.local"), "KEEP").unwrap();

        let warnings = copy_repo_files(
            &root,
            &checkout,
            &[".env*".into(), "config/local".into(), "missing.txt".into()],
        );

        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(
            std::fs::read_to_string(checkout.join(".env")).unwrap(),
            "SECRET=1"
        );
        assert_eq!(
            std::fs::read_to_string(checkout.join(".env.local")).unwrap(),
            "KEEP"
        );
        assert_eq!(
            std::fs::read_to_string(checkout.join("config/local/dev.toml")).unwrap(),
            "dev"
        );
        assert!(!checkout.join("missing.txt").exists());
        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(checkout);
    }

    #[cfg(unix)]
    #[test]
    fn hooks_log_output_and_report_the_last_line_on_failure() {
        let dir = temp_dir("run");
        let hook = WorktreeHook {
            command: "echo \"setting up $HERDR_REPO\"; echo 'npm ERR! missing script' >&2; exit 3"
                .into(),
            cwd: dir.clone(),
            env: vec![("HERDR_REPO", "alpha".into())],
            log_path: dir.join("logs/alpha-create.log"),
        };
        assert_eq!(hook.run(), Err("npm ERR! missing script".to_owned()));
        let log = std::fs::read_to_string(&hook.log_path).unwrap();
        assert!(log.contains("setting up alpha"), "{log}");
        assert!(log.contains("exited with status 3"), "{log}");

        let ok = WorktreeHook {
            command: "pwd > where.txt".into(),
            ..hook
        };
        assert_eq!(ok.run(), Ok(()));
        assert!(std::fs::read_to_string(dir.join("where.txt")).is_ok());
        let _ = std::fs::remove_dir_all(dir);
    }
}
