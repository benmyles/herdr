//! Lists the branches on a repo's remote for checking one out as a worktree.
//!
//! Runs on a worker thread: it may fetch from the network.

use std::path::{Path, PathBuf};
use std::time::Duration;

use super::space_create::Git;

/// Shorter than a create's fetch: someone is watching the list load.
const LIST_FETCH_TIMEOUT: Duration = Duration::from_secs(30);

/// Enough for any branch someone would pick by name; the rest are older.
const MAX_REMOTE_BRANCHES: usize = 2000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RemoteBranch {
    /// Without the remote, e.g. `feature/login`.
    pub name: String,
    pub committed_at: i64,
    pub author: String,
    pub subject: String,
    /// Where a local branch of the same name is checked out.
    pub checkout: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RemoteBranchList {
    /// Most recently committed first.
    pub branches: Vec<RemoteBranch>,
    pub truncated: bool,
    pub fetch_error: Option<String>,
}

pub(crate) fn list_remote_branches(
    repo_root: &Path,
    remote: &str,
    fetch: bool,
) -> Result<RemoteBranchList, String> {
    let git = Git::new(repo_root);
    let fetch_error = fetch
        .then(|| git.fetch(remote, LIST_FETCH_TIMEOUT).err())
        .flatten()
        .map(|err| format!("couldn't fetch {remote}: {err}"));
    let prefix = format!("refs/remotes/{remote}/");
    let output = git.stdout(&[
        "for-each-ref",
        "--sort=-committerdate",
        "--format=%(refname)%00%(symref)%00%(committerdate:unix)%00%(authorname)%00%(subject)",
        &prefix,
    ])?;
    let checkouts = super::list_existing_worktrees(repo_root, false)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|entry| Some((entry.branch?, entry.path)))
        .collect::<Vec<_>>();
    let mut branches = Vec::new();
    let mut truncated = false;
    for line in output.lines() {
        let mut fields = line.split('\0');
        let (Some(refname), Some(symref), Some(date), Some(author), Some(subject)) = (
            fields.next(),
            fields.next(),
            fields.next(),
            fields.next(),
            fields.next(),
        ) else {
            continue;
        };
        // `<remote>/HEAD` points at another branch in the list.
        if !symref.is_empty() {
            continue;
        }
        let Some(name) = refname.strip_prefix(&prefix) else {
            continue;
        };
        if branches.len() == MAX_REMOTE_BRANCHES {
            truncated = true;
            break;
        }
        branches.push(RemoteBranch {
            name: name.to_owned(),
            committed_at: date.trim().parse().unwrap_or(0),
            author: author.to_owned(),
            subject: subject.to_owned(),
            checkout: checkouts
                .iter()
                .find(|(branch, _)| branch == name)
                .map(|(_, path)| path.clone()),
        });
    }
    Ok(RemoteBranchList {
        branches,
        truncated,
        fetch_error,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worktree::space_create::tests::{commit, git, origin_and_root};

    #[test]
    fn lists_remote_branches_newest_first_with_their_checkouts() {
        let (base, origin, root) = origin_and_root("remote-list");
        git(&origin, &["switch", "--quiet", "-c", "older"]);
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(&origin)
            .args(["commit", "--quiet", "--allow-empty", "-m", "older work"])
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_COMMITTER_DATE", "2001-01-01T00:00:00Z")
            .status()
            .unwrap();
        assert!(status.success());
        git(
            &origin,
            &["switch", "--quiet", "-c", "feature/newer", "main"],
        );
        commit(&origin, "newer");
        git(&origin, &["switch", "--quiet", "main"]);

        let unfetched = list_remote_branches(&root, "origin", false).expect("list");
        assert_eq!(
            unfetched
                .branches
                .iter()
                .map(|branch| branch.name.as_str())
                .collect::<Vec<_>>(),
            ["main"],
            "only refs known before the fetch, without origin/HEAD"
        );
        assert_eq!(
            unfetched.branches[0].checkout.as_deref(),
            Some(root.as_path())
        );

        let fetched = list_remote_branches(&root, "origin", true).expect("list");
        assert_eq!(fetched.fetch_error, None);
        let names = fetched
            .branches
            .iter()
            .map(|branch| branch.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(names.len(), 3, "{names:?}");
        assert_eq!(names.last(), Some(&"older"), "{names:?}");
        assert!(names.contains(&"feature/newer"), "{names:?}");
        let newer = &fetched.branches[names.iter().position(|n| *n == "feature/newer").unwrap()];
        assert_eq!(newer.subject, "newer");
        assert_eq!(newer.author, "Herdr Test");
        assert_eq!(newer.checkout, None);
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn failed_fetch_still_lists_known_branches() {
        let (base, _origin, root) = origin_and_root("remote-list-fail");
        git(
            &root,
            &["remote", "set-url", "origin", "/nonexistent/herdr-remote"],
        );
        let list = list_remote_branches(&root, "origin", true).expect("list");
        assert!(list
            .fetch_error
            .as_deref()
            .is_some_and(|error| error.starts_with("couldn't fetch origin")));
        assert_eq!(list.branches.len(), 1);
        let _ = std::fs::remove_dir_all(base);
    }
}
