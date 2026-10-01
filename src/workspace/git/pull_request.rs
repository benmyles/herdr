//! The GitHub pull request for a checkout's branch, looked up with the GitHub
//! CLI (`gh`). Runs on a worker thread: `gh` talks to the network.

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::api::schema::{
    PullRequestChecks, PullRequestReview, PullRequestState, WorkspacePullRequest,
};

const GH_TIMEOUT: Duration = Duration::from_secs(20);

/// What one lookup found for a checkout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PullRequestLookup {
    /// The branch the checkout was on, and its pull request if it has one.
    Branch {
        branch: String,
        pull_request: Option<WorkspacePullRequest>,
    },
    /// Not on a branch that could have a pull request: detached, the
    /// default branch, or not a Git checkout.
    NoBranch,
    /// `gh` is not installed, so no lookup can succeed.
    GhMissing,
}

/// Looks up the pull request for the branch checked out at `checkout`.
pub(crate) fn lookup_pull_request(checkout: &Path) -> PullRequestLookup {
    let Some(branch) = git_output(checkout, &["branch", "--show-current"]) else {
        return PullRequestLookup::NoBranch;
    };
    let default_branch = git_output(
        checkout,
        &[
            "symbolic-ref",
            "--quiet",
            "--short",
            "refs/remotes/origin/HEAD",
        ],
    )
    .and_then(|head| head.split_once('/').map(|(_, branch)| branch.to_owned()));
    if is_default_branch(&branch, default_branch.as_deref()) {
        return PullRequestLookup::NoBranch;
    }
    let mut command = crate::noninteractive_process::command("gh");
    command
        .args([
            "pr",
            "view",
            "--json",
            "number,url,title,state,isDraft,reviewDecision,statusCheckRollup",
        ])
        .current_dir(checkout)
        .env("GH_PROMPT_DISABLED", "1")
        .env("GH_NO_UPDATE_NOTIFIER", "1")
        .env("NO_COLOR", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let output = match run_with_timeout(command, GH_TIMEOUT) {
        Ok(output) => output,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return PullRequestLookup::GhMissing;
        }
        Err(_) => None,
    };
    PullRequestLookup::Branch {
        branch,
        // `gh` exits non-zero when the branch has no pull request.
        pull_request: output.and_then(|stdout| parse_pull_request(&stdout)),
    }
}

/// The default branch never heads a pull request worth looking up.
fn is_default_branch(branch: &str, default_branch: Option<&str>) -> bool {
    match default_branch {
        Some(default_branch) => branch == default_branch,
        None => matches!(branch, "main" | "master"),
    }
}

fn git_output(checkout: &Path, args: &[&str]) -> Option<String> {
    let output = crate::noninteractive_process::command("git")
        .arg("-C")
        .arg(checkout)
        .args(args)
        .env("LC_ALL", "C")
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    (output.status.success() && !text.is_empty()).then_some(text)
}

/// Runs `command` and returns its stdout when it succeeds in time. A command
/// that can't start is an error; one that fails or times out is `Ok(None)`.
fn run_with_timeout(mut command: Command, timeout: Duration) -> std::io::Result<Option<String>> {
    let mut child = command.spawn()?;
    let mut stdout = child.stdout.take();
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        if let Some(stdout) = stdout.as_mut() {
            let _ = std::io::Read::read_to_string(stdout, &mut text);
        }
        text
    });
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait()? {
            Some(status) => break status,
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Ok(None);
            }
            None => std::thread::sleep(Duration::from_millis(50)),
        }
    };
    let text = reader.join().unwrap_or_default();
    Ok(status.success().then_some(text))
}

/// Reads `gh pr view --json` output.
pub(crate) fn parse_pull_request(json: &str) -> Option<WorkspacePullRequest> {
    let value: Value = serde_json::from_str(json).ok()?;
    let number = value.get("number")?.as_u64()?;
    let url = value.get("url")?.as_str()?.to_owned();
    let draft = value.get("isDraft").and_then(Value::as_bool) == Some(true);
    let state = match value.get("state").and_then(Value::as_str) {
        Some("OPEN") if draft => PullRequestState::Draft,
        Some("OPEN") => PullRequestState::Open,
        Some("MERGED") => PullRequestState::Merged,
        Some("CLOSED") => PullRequestState::Closed,
        _ => PullRequestState::Unknown,
    };
    let review = match value.get("reviewDecision").and_then(Value::as_str) {
        Some("APPROVED") => PullRequestReview::Approved,
        Some("CHANGES_REQUESTED") => PullRequestReview::ChangesRequested,
        Some("REVIEW_REQUIRED") => PullRequestReview::ReviewRequired,
        Some("") | None => PullRequestReview::None,
        Some(_) => PullRequestReview::Unknown,
    };
    let checks = value
        .get("statusCheckRollup")
        .and_then(Value::as_array)
        .map(|checks| rollup(checks))
        .unwrap_or_default();
    Some(WorkspacePullRequest {
        number,
        url,
        title: value
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        state,
        checks,
        review,
    })
}

/// One result for all checks: any failure fails, then anything unfinished is
/// pending, then everything passed.
fn rollup(checks: &[Value]) -> PullRequestChecks {
    let mut pending = false;
    for check in checks {
        let field = |name| check.get(name).and_then(Value::as_str).unwrap_or_default();
        // Check runs report a status and, once completed, a conclusion;
        // commit statuses report only a state.
        let outcome = if check.get("status").is_some() {
            if field("status") != "COMPLETED" {
                pending = true;
                continue;
            }
            field("conclusion")
        } else {
            field("state")
        };
        match outcome {
            "SUCCESS" | "NEUTRAL" | "SKIPPED" => {}
            "PENDING" | "EXPECTED" | "QUEUED" | "IN_PROGRESS" | "WAITING" => pending = true,
            _ => return PullRequestChecks::Failing,
        }
    }
    match (checks.is_empty(), pending) {
        (true, _) => PullRequestChecks::None,
        (false, true) => PullRequestChecks::Pending,
        (false, false) => PullRequestChecks::Passing,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pr(fields: &str) -> Option<WorkspacePullRequest> {
        parse_pull_request(&format!(
            r#"{{"number":42,"url":"https://github.com/o/r/pull/42","title":"Fix it",{fields}}}"#
        ))
    }

    #[test]
    fn reads_state_draft_and_review() {
        let open = pr(r#""state":"OPEN","isDraft":false,"reviewDecision":"APPROVED""#).unwrap();
        assert_eq!(open.number, 42);
        assert_eq!(open.url, "https://github.com/o/r/pull/42");
        assert_eq!(open.title, "Fix it");
        assert_eq!(open.state, PullRequestState::Open);
        assert_eq!(open.review, PullRequestReview::Approved);
        assert_eq!(open.checks, PullRequestChecks::None);

        let draft = pr(r#""state":"OPEN","isDraft":true,"reviewDecision":"""#).unwrap();
        assert_eq!(draft.state, PullRequestState::Draft);
        assert_eq!(draft.review, PullRequestReview::None);
        assert_eq!(
            pr(r#""state":"MERGED""#).unwrap().state,
            PullRequestState::Merged
        );
        assert_eq!(
            pr(r#""state":"CLOSED","reviewDecision":"CHANGES_REQUESTED""#)
                .unwrap()
                .review,
            PullRequestReview::ChangesRequested
        );
        assert_eq!(parse_pull_request("no pull requests found"), None);
    }

    #[test]
    fn checks_roll_up_failure_first_then_pending() {
        let checks = |rollup: &str| {
            pr(&format!(r#""state":"OPEN","statusCheckRollup":[{rollup}]"#))
                .unwrap()
                .checks
        };
        let passed = r#"{"status":"COMPLETED","conclusion":"SUCCESS"}"#;
        let skipped = r#"{"status":"COMPLETED","conclusion":"SKIPPED"}"#;
        let running = r#"{"status":"IN_PROGRESS","conclusion":""}"#;
        let failed = r#"{"status":"COMPLETED","conclusion":"FAILURE"}"#;
        let status_pending = r#"{"state":"PENDING"}"#;
        let status_ok = r#"{"state":"SUCCESS"}"#;
        assert_eq!(checks(""), PullRequestChecks::None);
        assert_eq!(
            checks(&format!("{passed},{skipped},{status_ok}")),
            PullRequestChecks::Passing
        );
        assert_eq!(
            checks(&format!("{passed},{running}")),
            PullRequestChecks::Pending
        );
        assert_eq!(
            checks(&format!("{passed},{status_pending}")),
            PullRequestChecks::Pending
        );
        assert_eq!(
            checks(&format!("{running},{failed}")),
            PullRequestChecks::Failing
        );
        assert_eq!(
            checks(r#"{"status":"COMPLETED","conclusion":"CANCELLED"}"#),
            PullRequestChecks::Failing
        );
    }

    #[test]
    fn default_branches_are_skipped() {
        assert!(is_default_branch("main", Some("main")));
        assert!(!is_default_branch("master", Some("main")));
        assert!(is_default_branch("master", None));
        assert!(!is_default_branch("feature", None));
    }
}
