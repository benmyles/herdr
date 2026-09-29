//! Creates a worktree for a configured repo on behalf of a space.
//!
//! Runs on a worker thread: it may fetch from the network. The flow is
//! deliberately conservative about the repo's main checkout (the "root"): it is
//! only fast-forwarded, and only when it is clean and on the base branch.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const FETCH_TIMEOUT: Duration = Duration::from_secs(90);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SpaceWorktreePlan {
    pub repo_root: PathBuf,
    pub checkout_path: PathBuf,
    pub branch: String,
    pub base_branch: String,
    pub remote: Option<String>,
    pub sync: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BranchSource {
    /// A new branch started from `start_point`.
    New { start_point: String },
    /// An existing local branch was checked out.
    Local,
    /// A branch that only existed on the remote, now tracked locally.
    Remote { upstream: String },
    /// The checkout already existed as a worktree of this repo and was reused.
    ExistingCheckout,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SpaceWorktreeReport {
    pub fetched: bool,
    pub root_updated: bool,
    pub branch_source: BranchSource,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SpaceWorktreeFailure {
    pub code: &'static str,
    pub message: String,
}

impl SpaceWorktreeFailure {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

pub(crate) fn create_space_worktree(
    plan: &SpaceWorktreePlan,
) -> Result<SpaceWorktreeReport, SpaceWorktreeFailure> {
    let git = Git::new(&plan.repo_root);
    if !git.succeeds(&["check-ref-format", "--branch", &plan.branch]) {
        return Err(SpaceWorktreeFailure::new(
            "invalid_worktree_name",
            format!("'{}' is not a valid branch name", plan.branch),
        ));
    }
    let worktrees = super::list_existing_worktrees(&plan.repo_root, false)
        .map_err(|err| SpaceWorktreeFailure::new("repo_unavailable", err))?;
    let checkout_key = super::canonical_or_original(&plan.checkout_path);
    if worktrees
        .iter()
        .any(|entry| super::canonical_or_original(&entry.path) == checkout_key)
    {
        return Ok(SpaceWorktreeReport {
            fetched: false,
            root_updated: false,
            branch_source: BranchSource::ExistingCheckout,
            warnings: Vec::new(),
        });
    }
    if directory_has_entries(&plan.checkout_path) {
        return Err(SpaceWorktreeFailure::new(
            "checkout_path_exists",
            format!(
                "{} already exists and is not a worktree of this repo",
                plan.checkout_path.display()
            ),
        ));
    }
    if let Some(entry) = worktrees
        .iter()
        .find(|entry| entry.branch.as_deref() == Some(plan.branch.as_str()))
    {
        return Err(SpaceWorktreeFailure::new(
            "branch_checked_out",
            format!(
                "branch {} is already checked out at {}",
                plan.branch,
                entry.path.display()
            ),
        ));
    }

    let mut report = SpaceWorktreeReport {
        fetched: false,
        root_updated: false,
        branch_source: BranchSource::Local,
        warnings: Vec::new(),
    };
    let remote = plan.remote.as_deref().filter(|remote| !remote.is_empty());
    if plan.sync {
        if let Some(remote) = remote {
            git.fetch(remote).map_err(|err| {
                SpaceWorktreeFailure::new(
                    "sync_fetch_failed",
                    format!("couldn't fetch {remote}: {err}"),
                )
            })?;
            report.fetched = true;
            fast_forward_base(&git, plan, remote, &worktrees, &mut report);
        }
    }

    let local_base = git.has_ref(&format!("refs/heads/{}", plan.base_branch));
    let remote_base = remote
        .map(|remote| format!("{remote}/{}", plan.base_branch))
        .filter(|name| git.has_ref(&format!("refs/remotes/{name}")));
    if let Some(parent) = plan.checkout_path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| {
            SpaceWorktreeFailure::new(
                "worktree_create_failed",
                format!("couldn't create {}: {err}", parent.display()),
            )
        })?;
    }
    let path = plan.checkout_path.display().to_string();
    let remote_branch = remote
        .map(|remote| format!("{remote}/{}", plan.branch))
        .filter(|name| git.has_ref(&format!("refs/remotes/{name}")));
    let branch = plan.branch.as_str();
    let (args, source) = if git.has_ref(&format!("refs/heads/{branch}")) {
        (
            owned(&["worktree", "add", &path, branch]),
            BranchSource::Local,
        )
    } else if let Some(upstream) = remote_branch {
        (
            owned(&["worktree", "add", "--track", "-b", branch, &path, &upstream]),
            BranchSource::Remote { upstream },
        )
    } else {
        let start_point = match (plan.sync, local_base, remote_base) {
            (true, _, Some(remote_base)) | (false, false, Some(remote_base)) => remote_base,
            (_, true, _) => plan.base_branch.clone(),
            (_, false, None) => {
                return Err(SpaceWorktreeFailure::new(
                    "base_branch_not_found",
                    format!(
                        "base branch {} doesn't exist in this repo",
                        plan.base_branch
                    ),
                ));
            }
        };
        (
            owned(&[
                "worktree",
                "add",
                "--no-track",
                "-b",
                branch,
                &path,
                &start_point,
            ]),
            BranchSource::New { start_point },
        )
    };
    let args = args.iter().map(String::as_str).collect::<Vec<_>>();
    git.run(&args)
        .map_err(|err| SpaceWorktreeFailure::new("worktree_create_failed", err))?;
    report.branch_source = source;
    Ok(report)
}

fn owned(args: &[&str]) -> Vec<String> {
    args.iter().map(|arg| (*arg).to_owned()).collect()
}

/// Brings the local base branch up to the fetched remote when that is a pure
/// fast-forward that can't disturb anyone's working tree.
fn fast_forward_base(
    git: &Git,
    plan: &SpaceWorktreePlan,
    remote: &str,
    worktrees: &[super::ExistingWorktree],
    report: &mut SpaceWorktreeReport,
) {
    let base = plan.base_branch.as_str();
    let upstream = format!("{remote}/{base}");
    let local_ref = format!("refs/heads/{base}");
    if !git.has_ref(&format!("refs/remotes/{upstream}")) {
        report.warnings.push(format!(
            "{upstream} doesn't exist; started from local {base}"
        ));
        return;
    }
    if !git.has_ref(&local_ref) {
        return;
    }
    let Some(before) = git.rev_parse(&local_ref) else {
        return;
    };
    let Some(target) = git.rev_parse(&format!("refs/remotes/{upstream}")) else {
        return;
    };
    if before == target {
        return;
    }
    if !git.succeeds(&["merge-base", "--is-ancestor", &local_ref, &upstream]) {
        if !git.succeeds(&["merge-base", "--is-ancestor", &upstream, &local_ref]) {
            report.warnings.push(format!(
                "local {base} has diverged from {upstream}; left it as is"
            ));
        }
        return;
    }
    let root_key = super::canonical_or_original(&plan.repo_root);
    let checked_out_at = worktrees
        .iter()
        .find(|entry| entry.branch.as_deref() == Some(base))
        .map(|entry| super::canonical_or_original(&entry.path));
    match checked_out_at {
        Some(path) if path == root_key => {
            if !git.is_clean() {
                report.warnings.push(format!(
                    "{} has uncommitted changes; didn't update its {base}",
                    plan.repo_root.display()
                ));
                return;
            }
            if let Err(err) = git.run(&["merge", "--ff-only", "--quiet", &upstream]) {
                report
                    .warnings
                    .push(format!("couldn't fast-forward {base}: {err}"));
                return;
            }
        }
        Some(path) => {
            report.warnings.push(format!(
                "{base} is checked out at {}; didn't update it",
                path.display()
            ));
            return;
        }
        None => {
            if let Err(err) = git.run(&["update-ref", &local_ref, &target, &before]) {
                report
                    .warnings
                    .push(format!("couldn't fast-forward {base}: {err}"));
                return;
            }
        }
    }
    report.root_updated = true;
}

fn directory_has_entries(path: &Path) -> bool {
    match std::fs::read_dir(path) {
        Ok(mut entries) => entries.next().is_some(),
        Err(_) => path.exists(),
    }
}

struct Git<'a> {
    repo_root: &'a Path,
}

impl<'a> Git<'a> {
    fn new(repo_root: &'a Path) -> Self {
        Self { repo_root }
    }

    fn command(&self) -> Command {
        let mut command = crate::noninteractive_process::command("git");
        command
            .arg("-C")
            .arg(self.repo_root)
            .env("LC_ALL", "C")
            .env("GIT_TERMINAL_PROMPT", "0")
            .stdin(Stdio::null());
        command
    }

    fn succeeds(&self, args: &[&str]) -> bool {
        self.command()
            .args(args)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    }

    fn has_ref(&self, name: &str) -> bool {
        self.succeeds(&["show-ref", "--verify", "--quiet", name])
    }

    fn rev_parse(&self, name: &str) -> Option<String> {
        let output = self
            .command()
            .args(["rev-parse", "--verify", "--quiet", name])
            .output()
            .ok()?;
        output
            .status
            .success()
            .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
            .filter(|sha| !sha.is_empty())
    }

    fn is_clean(&self) -> bool {
        self.command()
            .args(["status", "--porcelain", "--untracked-files=no"])
            .output()
            .is_ok_and(|output| output.status.success() && output.stdout.is_empty())
    }

    fn run(&self, args: &[&str]) -> Result<(), String> {
        let output = self
            .command()
            .args(args)
            .output()
            .map_err(|err| err.to_string())?;
        if output.status.success() {
            return Ok(());
        }
        Err(failure_message(
            &output.stdout,
            &output.stderr,
            output.status,
        ))
    }

    fn fetch(&self, remote: &str) -> Result<(), String> {
        let mut child = self
            .command()
            .args(["fetch", "--quiet", remote])
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|err| err.to_string())?;
        let deadline = Instant::now() + FETCH_TIMEOUT;
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    let mut stderr = Vec::new();
                    if let Some(mut pipe) = child.stderr.take() {
                        let _ = std::io::Read::read_to_end(&mut pipe, &mut stderr);
                    }
                    return if status.success() {
                        Ok(())
                    } else {
                        Err(failure_message(&[], &stderr, status))
                    };
                }
                Ok(None) if Instant::now() >= deadline => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!(
                        "timed out after {} seconds",
                        FETCH_TIMEOUT.as_secs()
                    ));
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(50)),
                Err(err) => return Err(err.to_string()),
            }
        }
    }
}

fn failure_message(stdout: &[u8], stderr: &[u8], status: std::process::ExitStatus) -> String {
    let stderr = String::from_utf8_lossy(stderr);
    let stdout = String::from_utf8_lossy(stdout);
    let message = if stderr.trim().is_empty() {
        stdout.trim()
    } else {
        stderr.trim()
    };
    // Git's first `fatal:`/`error:` line names the problem; later lines are
    // generic advice ("Please make sure you have the correct access rights").
    let lines = message
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty());
    let message = lines
        .clone()
        .find_map(|line| {
            line.strip_prefix("fatal: ")
                .or_else(|| line.strip_prefix("error: "))
        })
        .or_else(|| lines.clone().next_back())
        .unwrap_or_default()
        .trim_end_matches('.');
    if message.is_empty() {
        format!("git failed with {status}")
    } else {
        message.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let path = std::env::temp_dir().join(format!(
            "herdr-space-wt-{name}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).unwrap();
        crate::worktree::canonical_or_original(&path)
    }

    fn git(dir: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    }

    fn commit(dir: &Path, file: &str) {
        std::fs::write(dir.join(file), file).unwrap();
        git(dir, &["add", file]);
        git(dir, &["commit", "--quiet", "-m", file]);
    }

    /// An "origin" repo plus a clone of it acting as the configured root.
    fn origin_and_root(name: &str) -> (PathBuf, PathBuf, PathBuf) {
        let base = temp_dir(name);
        let origin = base.join("origin");
        std::fs::create_dir_all(&origin).unwrap();
        git(&origin, &["init", "--quiet", "--initial-branch=main"]);
        git(&origin, &["config", "user.email", "herdr@example.invalid"]);
        git(&origin, &["config", "user.name", "Herdr Test"]);
        commit(&origin, "one");
        let root = base.join("root");
        git(
            &base,
            &["clone", "--quiet", origin.to_str().unwrap(), "root"],
        );
        git(&root, &["config", "user.email", "herdr@example.invalid"]);
        git(&root, &["config", "user.name", "Herdr Test"]);
        (base, origin, root)
    }

    fn plan(root: &Path, checkout: PathBuf, branch: &str, sync: bool) -> SpaceWorktreePlan {
        SpaceWorktreePlan {
            repo_root: root.to_path_buf(),
            checkout_path: checkout,
            branch: branch.into(),
            base_branch: "main".into(),
            remote: Some("origin".into()),
            sync,
        }
    }

    #[test]
    fn sync_fast_forwards_clean_root_and_branches_from_remote_base() {
        let (base, origin, root) = origin_and_root("ff");
        commit(&origin, "two");
        let origin_head = git(&origin, &["rev-parse", "HEAD"]);
        let checkout = base.join("wt/knowledge/root/knowledge");

        let report = create_space_worktree(&plan(&root, checkout.clone(), "knowledge", true))
            .expect("create worktree");

        assert!(report.fetched);
        assert!(report.root_updated, "{report:?}");
        assert_eq!(
            report.branch_source,
            BranchSource::New {
                start_point: "origin/main".into()
            }
        );
        assert_eq!(git(&root, &["rev-parse", "HEAD"]), origin_head);
        assert_eq!(git(&checkout, &["rev-parse", "HEAD"]), origin_head);
        assert_eq!(git(&checkout, &["branch", "--show-current"]), "knowledge");
        // Branches start untracked so the first push chooses the upstream.
        assert!(!git(&root, &["config", "--list"]).contains("branch.knowledge.merge"));
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn sync_leaves_a_dirty_root_alone_but_still_starts_from_the_remote() {
        let (base, origin, root) = origin_and_root("dirty");
        let root_head = git(&root, &["rev-parse", "HEAD"]);
        commit(&origin, "two");
        let origin_head = git(&origin, &["rev-parse", "HEAD"]);
        std::fs::write(root.join("one"), "edited").unwrap();

        let checkout = base.join("wt/x");
        let report =
            create_space_worktree(&plan(&root, checkout.clone(), "x", true)).expect("create");

        assert!(!report.root_updated);
        assert_eq!(report.warnings.len(), 1, "{report:?}");
        assert!(report.warnings[0].contains("uncommitted changes"));
        assert_eq!(git(&root, &["rev-parse", "HEAD"]), root_head);
        assert_eq!(git(&checkout, &["rev-parse", "HEAD"]), origin_head);
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn root_on_another_branch_updates_base_ref_without_checkout() {
        let (base, origin, root) = origin_and_root("offbase");
        git(&root, &["switch", "--quiet", "-c", "side"]);
        commit(&origin, "two");
        let origin_head = git(&origin, &["rev-parse", "HEAD"]);

        let report =
            create_space_worktree(&plan(&root, base.join("wt/y"), "y", true)).expect("create");

        assert!(report.root_updated);
        assert_eq!(git(&root, &["rev-parse", "main"]), origin_head);
        assert_eq!(git(&root, &["branch", "--show-current"]), "side");
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn without_sync_starts_from_local_base() {
        let (base, origin, root) = origin_and_root("nosync");
        let root_head = git(&root, &["rev-parse", "HEAD"]);
        commit(&origin, "two");
        let checkout = base.join("wt/z");

        let report =
            create_space_worktree(&plan(&root, checkout.clone(), "z", false)).expect("create");

        assert!(!report.fetched);
        assert_eq!(
            report.branch_source,
            BranchSource::New {
                start_point: "main".into()
            }
        );
        assert_eq!(git(&checkout, &["rev-parse", "HEAD"]), root_head);
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn existing_local_and_remote_branches_are_checked_out() {
        let (base, origin, root) = origin_and_root("existing");
        git(&root, &["branch", "local-only"]);
        git(&origin, &["branch", "remote-only"]);

        let local = create_space_worktree(&plan(&root, base.join("wt/a"), "local-only", true))
            .expect("local");
        assert_eq!(local.branch_source, BranchSource::Local);

        let remote = create_space_worktree(&plan(&root, base.join("wt/b"), "remote-only", true))
            .expect("remote");
        assert_eq!(
            remote.branch_source,
            BranchSource::Remote {
                upstream: "origin/remote-only".into()
            }
        );
        assert_eq!(
            git(
                &base.join("wt/b"),
                &["rev-parse", "--abbrev-ref", "@{upstream}"]
            ),
            "origin/remote-only"
        );
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn existing_checkout_is_reused_and_foreign_directories_are_refused() {
        let (base, _origin, root) = origin_and_root("reuse");
        let checkout = base.join("wt/c");
        create_space_worktree(&plan(&root, checkout.clone(), "c", false)).expect("first");
        let again = create_space_worktree(&plan(&root, checkout, "c", false)).expect("again");
        assert_eq!(again.branch_source, BranchSource::ExistingCheckout);

        let foreign = base.join("wt/foreign");
        std::fs::create_dir_all(&foreign).unwrap();
        std::fs::write(foreign.join("file"), "x").unwrap();
        let err = create_space_worktree(&plan(&root, foreign, "d", false)).unwrap_err();
        assert_eq!(err.code, "checkout_path_exists");
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn fetch_failure_and_bad_names_are_reported() {
        let (base, _origin, root) = origin_and_root("fail");
        git(
            &root,
            &["remote", "set-url", "origin", "/nonexistent/herdr-origin"],
        );
        let err = create_space_worktree(&plan(&root, base.join("wt/e"), "e", true)).unwrap_err();
        assert_eq!(err.code, "sync_fetch_failed");
        assert!(
            err.message
                .contains("does not appear to be a git repository"),
            "{err:?}"
        );

        let err =
            create_space_worktree(&plan(&root, base.join("wt/f"), "bad name", false)).unwrap_err();
        assert_eq!(err.code, "invalid_worktree_name");
        let _ = std::fs::remove_dir_all(base);
    }
}
