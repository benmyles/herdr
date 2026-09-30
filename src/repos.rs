//! Repos registered for creating worktrees from a space.
//!
//! Server-owned and machine-local: every endpoint (local or over SSH) keeps
//! its own list in `repos.toml` beside its config, because repo roots are
//! paths on that machine.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tracing::warn;

pub(crate) const REPOS_FILE_NAME: &str = "repos.toml";
const MAX_NAME_CHARS: usize = 64;
const FILE_HEADER: &str =
    "# Repos for creating worktrees from a space. Managed from settings → repos.\n\n";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Repo {
    pub name: String,
    /// Main checkout as configured; may start with `~`.
    pub root: String,
    #[serde(rename = "base")]
    pub base_branch: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote: Option<String>,
    #[serde(flatten)]
    pub settings: crate::api::schema::RepoSettings,
}

impl Repo {
    pub(crate) fn root_path(&self) -> PathBuf {
        crate::worktree::expand_tilde_absolute_path(&self.root)
    }

    pub(crate) fn info(&self) -> crate::api::schema::RepoInfo {
        crate::api::schema::RepoInfo {
            name: self.name.clone(),
            root: self.root.clone(),
            root_path: self.root_path().display().to_string(),
            base_branch: self.base_branch.clone(),
            remote: self.remote.clone(),
            settings: self.settings.clone(),
        }
    }

    /// The branch for a worktree named `name`.
    pub(crate) fn branch_for(&self, name: &str) -> String {
        format!("{}{name}", self.settings.branch_prefix)
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct ReposFile {
    #[serde(default, rename = "repo")]
    repos: Vec<Repo>,
}

pub(crate) fn default_path() -> PathBuf {
    crate::config::config_path()
        .parent()
        .map(|dir| dir.join(REPOS_FILE_NAME))
        .unwrap_or_else(|| crate::config::config_dir().join(REPOS_FILE_NAME))
}

pub(crate) fn load(path: &Path) -> Vec<Repo> {
    let contents = match std::fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(err) => {
            warn!(path = %path.display(), err = %err, "failed to read repos");
            return Vec::new();
        }
    };
    match toml::from_str::<ReposFile>(&contents) {
        Ok(file) => file.repos,
        Err(err) => {
            warn!(path = %path.display(), err = %err, "failed to parse repos");
            Vec::new()
        }
    }
}

pub(crate) fn save(path: &Path, repos: &[Repo]) -> std::io::Result<()> {
    let body = toml::to_string_pretty(&ReposFile {
        repos: repos.to_vec(),
    })
    .map_err(std::io::Error::other)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, format!("{FILE_HEADER}{body}"))?;
    if let Err(err) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(err);
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RepoError {
    pub code: &'static str,
    pub message: String,
}

impl RepoError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub(crate) fn not_found(name: &str) -> Self {
        Self::new("repo_not_found", format!("repo {name} not found"))
    }
}

pub(crate) fn find<'a>(repos: &'a [Repo], name: &str) -> Option<&'a Repo> {
    repos
        .iter()
        .find(|repo| repo.name.eq_ignore_ascii_case(name.trim()))
}

pub(crate) fn position(repos: &[Repo], name: &str) -> Option<usize> {
    repos
        .iter()
        .position(|repo| repo.name.eq_ignore_ascii_case(name.trim()))
}

/// Trims and checks a repo name; `except` is the repo being renamed.
pub(crate) fn validated_name(
    repos: &[Repo],
    name: &str,
    except: Option<usize>,
) -> Result<String, RepoError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(RepoError::new("invalid_repo_name", "repo name is required"));
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(RepoError::new(
            "invalid_repo_name",
            format!("repo names are limited to {MAX_NAME_CHARS} characters"),
        ));
    }
    if name
        .chars()
        .any(|ch| ch.is_control() || ch == '/' || ch == '\\')
    {
        return Err(RepoError::new(
            "invalid_repo_name",
            "repo names can't contain slashes or control characters",
        ));
    }
    if repos
        .iter()
        .enumerate()
        .any(|(index, repo)| Some(index) != except && repo.name.eq_ignore_ascii_case(name))
    {
        return Err(RepoError::new(
            "duplicate_repo_name",
            format!("a repo named {name} already exists"),
        ));
    }
    Ok(name.to_owned())
}

pub(crate) fn validated_branch(name: &str, what: &str) -> Result<String, RepoError> {
    let name = name.trim();
    let invalid = name.is_empty()
        || name.starts_with('-')
        || name.contains("..")
        || name.ends_with(".lock")
        || name.ends_with('/')
        || name
            .chars()
            .any(|ch| ch.is_whitespace() || ch.is_control() || "~^:?*[\\".contains(ch));
    if invalid {
        return Err(RepoError::new(
            "invalid_branch",
            format!("'{name}' is not a valid {what}"),
        ));
    }
    Ok(name.to_owned())
}

const MAX_COMMAND_CHARS: usize = 4096;
const MAX_COPY_FILES: usize = 32;

/// Trims every setting and rejects values Herdr could not use safely.
pub(crate) fn validated_settings(
    settings: crate::api::schema::RepoSettings,
) -> Result<crate::api::schema::RepoSettings, RepoError> {
    let branch_prefix = settings.branch_prefix.trim().to_owned();
    if !branch_prefix.is_empty() {
        // A prefix must still make a valid branch once a name follows it.
        validated_branch(&format!("{branch_prefix}x"), "branch prefix").map_err(|_| {
            RepoError::new(
                "invalid_branch_prefix",
                format!("'{branch_prefix}' is not a valid branch prefix"),
            )
        })?;
    }
    let copy_files = settings
        .copy_files
        .iter()
        .flat_map(|entry| entry.split_whitespace())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if copy_files.len() > MAX_COPY_FILES {
        return Err(RepoError::new(
            "invalid_copy_files",
            format!("copy at most {MAX_COPY_FILES} file patterns"),
        ));
    }
    for pattern in &copy_files {
        let path = Path::new(pattern);
        let escapes = pattern.starts_with('~')
            || path.is_absolute()
            || path
                .components()
                .any(|component| !matches!(component, std::path::Component::Normal(_)));
        if escapes {
            return Err(RepoError::new(
                "invalid_copy_files",
                format!("copy files must be paths inside the repo, not {pattern}"),
            ));
        }
    }
    let command = |value: &str, what: &str| {
        let value = value.trim();
        if value.chars().count() > MAX_COMMAND_CHARS || value.contains('\0') {
            return Err(RepoError::new(
                "invalid_repo_command",
                format!("the {what} command is too long or contains a NUL byte"),
            ));
        }
        Ok(value.to_owned())
    };
    Ok(crate::api::schema::RepoSettings {
        branch_prefix,
        copy_files,
        on_create: command(&settings.on_create, "create")?,
        on_remove: command(&settings.on_remove, "remove")?,
        start_command: command(&settings.start_command, "start")?,
    })
}

/// What a path looks like as a repo root, with defaults for the fields the
/// user left blank.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InspectedRoot {
    pub root: PathBuf,
    pub name: String,
    pub remote: Option<String>,
    pub base_branch: String,
}

pub(crate) fn inspect_root(path: &Path, remote: Option<&str>) -> Result<InspectedRoot, RepoError> {
    // Reasons come first: the dialog shows one or two lines, and paths are long.
    if !path.is_absolute() {
        return Err(RepoError::new(
            "invalid_repo_root",
            format!("use an absolute path or ~/…, not {}", path.display()),
        ));
    }
    if !path.is_dir() {
        return Err(RepoError::new(
            "repo_root_not_found",
            format!("no such directory: {}", path.display()),
        ));
    }
    let space = crate::workspace::git_space_metadata(path).ok_or_else(|| {
        RepoError::new(
            "not_git_worktree",
            format!("not inside a Git repo: {}", path.display()),
        )
    })?;
    if space.is_linked_worktree {
        return Err(RepoError::new(
            "linked_worktree_source",
            "pick the repo's main checkout, not one of its linked worktrees",
        ));
    }
    // Drops trailing separators so `~/code/x/` and `~/code/x` are one repo.
    let root = space.repo_root.components().collect::<PathBuf>();
    let name = if space.repo_name.starts_with('.') {
        crate::workspace::fallback_label_from_cwd(&root)
    } else {
        space.repo_name.clone()
    };
    let remotes = git_lines(&root, &["remote"]);
    let remote = match remote.map(str::trim) {
        Some("") => None,
        Some(remote) => Some(remote.to_owned()),
        None if remotes.iter().any(|remote| remote == "origin") => Some("origin".to_owned()),
        None if remotes.len() == 1 => remotes.first().cloned(),
        None => None,
    };
    let remote_default = remote.as_deref().and_then(|remote| {
        git_lines(
            &root,
            &[
                "symbolic-ref",
                "--quiet",
                "--short",
                &format!("refs/remotes/{remote}/HEAD"),
            ],
        )
        .into_iter()
        .next()
        .and_then(|name| name.strip_prefix(&format!("{remote}/")).map(str::to_owned))
    });
    let base_branch = remote_default
        .or_else(|| crate::workspace::git_branch(&root))
        .unwrap_or_else(|| "main".to_owned());
    Ok(InspectedRoot {
        root,
        name,
        remote,
        base_branch,
    })
}

/// A root as typed: `~` expands, relative paths stay relative so
/// `inspect_root` can reject them instead of resolving them against the
/// server's working directory.
pub(crate) fn typed_root(root: &str) -> PathBuf {
    crate::worktree::expand_tilde_path(root.trim())
}

/// `~/…` when the path is under the home directory, so the file stays
/// readable and survives a home directory move.
pub(crate) fn display_root(path: &Path) -> String {
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        if let Ok(rest) = path.strip_prefix(&home) {
            if rest.as_os_str().is_empty() {
                return "~".to_owned();
            }
            return format!("~/{}", rest.display());
        }
    }
    path.display().to_string()
}

fn git_lines(root: &Path, args: &[&str]) -> Vec<String> {
    let Ok(output) = crate::noninteractive_process::command("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .stdin(std::process::Stdio::null())
        .output()
    else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo(name: &str) -> Repo {
        Repo {
            name: name.into(),
            root: format!("~/code/{name}"),
            base_branch: "main".into(),
            remote: Some("origin".into()),
            settings: Default::default(),
        }
    }

    #[test]
    fn settings_round_trip_beside_the_repo_fields() {
        let dir = std::env::temp_dir().join(format!(
            "herdr-repo-settings-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let path = dir.join(REPOS_FILE_NAME);
        let mut configured = repo("pyshiftup");
        configured.settings = crate::api::schema::RepoSettings {
            branch_prefix: "ben/".into(),
            copy_files: vec![".env*".into()],
            on_create: "just setup".into(),
            on_remove: "just teardown".into(),
            start_command: "claude".into(),
        };
        let repos = vec![configured.clone(), repo("plain")];
        save(&path, &repos).expect("save repos");
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("on_create = \"just setup\""), "{text}");
        assert!(!text.contains("start_command = \"\""), "{text}");
        assert_eq!(load(&path), repos);
        assert_eq!(configured.branch_for("kb"), "ben/kb");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn settings_validation_trims_and_rejects_escaping_paths() {
        use crate::api::schema::RepoSettings;
        let settings = validated_settings(RepoSettings {
            branch_prefix: " ben/ ".into(),
            copy_files: vec![".env .env.local".into(), "config/dev.toml".into()],
            on_create: "  npm ci  ".into(),
            ..RepoSettings::default()
        })
        .expect("valid settings");
        assert_eq!(settings.branch_prefix, "ben/");
        assert_eq!(
            settings.copy_files,
            [".env", ".env.local", "config/dev.toml"]
        );
        assert_eq!(settings.on_create, "npm ci");
        for bad in ["../secrets", "/etc/passwd", "~/.ssh/id_rsa", "a/../../b"] {
            let err = validated_settings(RepoSettings {
                copy_files: vec![bad.into()],
                ..RepoSettings::default()
            })
            .expect_err(bad);
            assert_eq!(err.code, "invalid_copy_files");
        }
        let err = validated_settings(RepoSettings {
            branch_prefix: "bad prefix".into(),
            ..RepoSettings::default()
        })
        .expect_err("space in prefix");
        assert_eq!(err.code, "invalid_branch_prefix");
    }

    #[test]
    fn repos_round_trip_through_the_file() {
        let dir = std::env::temp_dir().join(format!(
            "herdr-repos-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let path = dir.join(REPOS_FILE_NAME);
        assert!(load(&path).is_empty());
        let mut local = repo("guided-selling");
        local.remote = None;
        let repos = vec![repo("pyshiftup"), local];
        save(&path, &repos).expect("save repos");
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("[[repo]]"), "{text}");
        assert!(text.contains("base = \"main\""), "{text}");
        assert_eq!(load(&path), repos);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn names_are_trimmed_unique_and_path_safe() {
        let repos = vec![repo("pyshiftup")];
        assert_eq!(
            validated_name(&repos, "  guided  ", None).unwrap(),
            "guided"
        );
        assert_eq!(
            validated_name(&repos, "PyShiftUp", None).unwrap_err().code,
            "duplicate_repo_name"
        );
        assert_eq!(
            validated_name(&repos, "PyShiftUp", Some(0)).unwrap(),
            "PyShiftUp"
        );
        assert_eq!(
            validated_name(&repos, "a/b", None).unwrap_err().code,
            "invalid_repo_name"
        );
        assert!(find(&repos, "PYSHIFTUP").is_some());
    }

    #[test]
    fn branch_names_reject_obvious_garbage() {
        assert_eq!(validated_branch(" main ", "base").unwrap(), "main");
        assert!(validated_branch("release/2.0", "base").is_ok());
        for bad in ["", "has space", "-x", "a..b", "x.lock", "a:b"] {
            assert!(validated_branch(bad, "base").is_err(), "{bad}");
        }
    }

    #[test]
    fn inspect_detects_root_remote_and_default_branch() {
        let base = std::env::temp_dir().join(format!(
            "herdr-repos-inspect-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let origin = base.join("origin");
        std::fs::create_dir_all(&origin).unwrap();
        let git = |dir: &Path, args: &[&str]| {
            let status = std::process::Command::new("git")
                .arg("-C")
                .arg(dir)
                .args(args)
                .status()
                .unwrap();
            assert!(status.success(), "git {}", args.join(" "));
        };
        git(&origin, &["init", "--quiet", "--initial-branch=trunk"]);
        git(
            &origin,
            &[
                "-c",
                "user.email=h@example.invalid",
                "-c",
                "user.name=H",
                "commit",
                "--quiet",
                "--allow-empty",
                "-m",
                "one",
            ],
        );
        git(&base, &["clone", "--quiet", "origin", "My Repo"]);
        let clone = base.join("My Repo");
        std::fs::create_dir_all(clone.join("src")).unwrap();

        let inspected = inspect_root(&clone.join("src"), None).expect("inspect");
        assert_eq!(
            crate::worktree::canonical_or_original(&inspected.root),
            crate::worktree::canonical_or_original(&clone)
        );
        assert_eq!(inspected.name, "My Repo");
        assert_eq!(inspected.remote.as_deref(), Some("origin"));
        assert_eq!(inspected.base_branch, "trunk");

        let without_remote = inspect_root(&clone, Some("")).expect("inspect");
        assert_eq!(without_remote.remote, None);
        assert_eq!(without_remote.base_branch, "trunk");

        assert_eq!(
            inspect_root(&base, None).unwrap_err().code,
            "not_git_worktree"
        );
        let _ = std::fs::remove_dir_all(base);
    }
}
