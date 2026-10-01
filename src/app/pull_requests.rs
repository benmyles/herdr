//! GitHub pull requests for workspace branches. The GitHub CLI looks them up
//! on a worker thread while a client is attached: about once a minute, and
//! right away for a workspace whose branch Herdr has not looked up yet.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use super::App;
use crate::api::schema::WorkspacePullRequest;
use crate::events::AppEvent;
use crate::workspace::{PullRequestLookup, Workspace};

const PULL_REQUEST_REFRESH_INTERVAL: Duration = Duration::from_secs(60);
/// Without `gh`, look again this rarely in case it gets installed.
const GH_MISSING_RETRY: Duration = Duration::from_secs(10 * 60);

/// The last lookup for one workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PullRequestEntry {
    /// The workspace's branch when Herdr looked; the entry is stale once the
    /// branch changes.
    branch: String,
    pull_request: Option<WorkspacePullRequest>,
}

/// One checkout to look up, shared by every workspace open on it.
#[derive(Debug)]
pub(crate) struct PullRequestJob {
    workspace_ids: Vec<String>,
    checkout: PathBuf,
    branch: String,
}

#[derive(Debug)]
pub(crate) struct PullRequestResult {
    workspace_ids: Vec<String>,
    branch: String,
    lookup: PullRequestLookup,
}

impl App {
    /// The pull request for `workspace`'s current branch, if one was found.
    pub(crate) fn pull_request_for(&self, workspace: &Workspace) -> Option<&WorkspacePullRequest> {
        let entry = self.pull_requests.get(&workspace.id)?;
        (workspace.branch().as_deref() == Some(entry.branch.as_str()))
            .then_some(entry.pull_request.as_ref())
            .flatten()
    }

    fn pull_request_jobs(&self) -> Vec<PullRequestJob> {
        let mut jobs: Vec<PullRequestJob> = Vec::new();
        for workspace in &self.state.workspaces {
            let (Some(branch), Some(checkout)) = (
                workspace.branch(),
                workspace
                    .resolved_identity_cwd_from(&self.state.terminals, &self.terminal_runtimes),
            ) else {
                continue;
            };
            match jobs
                .iter_mut()
                .find(|job| job.checkout == checkout && job.branch == branch)
            {
                Some(job) => job.workspace_ids.push(workspace.id.clone()),
                None => jobs.push(PullRequestJob {
                    workspace_ids: vec![workspace.id.clone()],
                    checkout,
                    branch,
                }),
            }
        }
        jobs
    }

    /// Whether some workspace's branch has never been looked up.
    fn pull_request_lookup_pending(&self) -> bool {
        self.state.workspaces.iter().any(|workspace| {
            workspace.branch().is_some_and(|branch| {
                self.pull_requests
                    .get(&workspace.id)
                    .is_none_or(|entry| entry.branch != branch)
            })
        })
    }

    pub(crate) fn pull_request_refresh_deadline(&self) -> Option<Instant> {
        if self.pull_request_refresh_in_flight
            || !self
                .state
                .workspaces
                .iter()
                .any(|workspace| workspace.branch().is_some())
        {
            return None;
        }
        Some(if self.pull_request_lookup_pending() {
            Instant::now()
        } else {
            self.next_pull_request_refresh
        })
    }

    pub(crate) fn start_pull_request_refresh_if_due(&mut self, now: Instant) {
        if self.pull_request_refresh_in_flight
            || (now < self.next_pull_request_refresh && !self.pull_request_lookup_pending())
        {
            return;
        }
        let jobs = self.pull_request_jobs();
        if jobs.is_empty() {
            self.pull_requests.clear();
            self.next_pull_request_refresh = now + PULL_REQUEST_REFRESH_INTERVAL;
            return;
        }
        self.pull_request_refresh_in_flight = true;
        let event_tx = self.event_tx.clone();
        std::thread::spawn(move || {
            let results = jobs
                .into_iter()
                .map(|job| PullRequestResult {
                    lookup: crate::workspace::lookup_pull_request(&job.checkout),
                    workspace_ids: job.workspace_ids,
                    branch: job.branch,
                })
                .collect();
            let _ = event_tx.blocking_send(AppEvent::PullRequestsRefreshed(results));
        });
    }

    /// Stores finished lookups. Returns whether any workspace's pull request
    /// changed.
    pub(crate) fn handle_pull_requests_refreshed(
        &mut self,
        results: Vec<PullRequestResult>,
    ) -> bool {
        self.pull_request_refresh_in_flight = false;
        let gh_missing = results
            .iter()
            .any(|result| result.lookup == PullRequestLookup::GhMissing);
        self.next_pull_request_refresh = Instant::now()
            + if gh_missing {
                GH_MISSING_RETRY
            } else {
                PULL_REQUEST_REFRESH_INTERVAL
            };
        let mut changed = false;
        for result in results {
            let pull_request = match result.lookup {
                // A checkout that moved to another branch while `gh` ran
                // gets looked up again once Herdr sees the new branch.
                PullRequestLookup::Branch {
                    branch,
                    pull_request,
                } if branch == result.branch => pull_request,
                _ => None,
            };
            for workspace_id in result.workspace_ids {
                let entry = PullRequestEntry {
                    branch: result.branch.clone(),
                    pull_request: pull_request.clone(),
                };
                if self.pull_requests.get(&workspace_id) != Some(&entry) {
                    changed |= entry.pull_request.is_some()
                        || self
                            .pull_requests
                            .get(&workspace_id)
                            .is_some_and(|previous| previous.pull_request.is_some());
                    self.pull_requests.insert(workspace_id, entry);
                }
            }
        }
        let workspaces = &self.state.workspaces;
        self.pull_requests
            .retain(|id, _| workspaces.iter().any(|workspace| &workspace.id == id));
        if changed {
            self.render_dirty.request_generic();
            self.render_notify.notify_one();
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::schema::{PullRequestChecks, PullRequestReview, PullRequestState};

    fn app() -> App {
        let (_api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(
            &crate::config::Config::default(),
            crate::app::AppPolicy::TEST,
            None,
            api_rx,
            crate::api::EventHub::default(),
        );
        let mut workspace = Workspace::test_new("api");
        workspace.id = "w1".into();
        app.state.workspaces = vec![workspace];
        app
    }

    fn pull_request(number: u64) -> WorkspacePullRequest {
        WorkspacePullRequest {
            number,
            url: format!("https://github.com/o/r/pull/{number}"),
            title: "Fix".into(),
            state: PullRequestState::Open,
            checks: PullRequestChecks::Passing,
            review: PullRequestReview::None,
        }
    }

    #[tokio::test]
    async fn results_follow_the_workspace_branch() {
        let mut app = app();
        let branch = app.state.workspaces[0].branch().expect("test branch");
        assert!(app.pull_request_lookup_pending());
        assert!(app.handle_pull_requests_refreshed(vec![PullRequestResult {
            workspace_ids: vec!["w1".into(), "gone".into()],
            branch: branch.clone(),
            lookup: PullRequestLookup::Branch {
                branch: branch.clone(),
                pull_request: Some(pull_request(7)),
            },
        }]));
        assert!(!app.pull_request_lookup_pending());
        assert_eq!(
            app.pull_request_for(&app.state.workspaces[0])
                .map(|pr| pr.number),
            Some(7)
        );
        assert!(!app.pull_requests.contains_key("gone"));

        // The same answer again changes nothing.
        assert!(!app.handle_pull_requests_refreshed(vec![PullRequestResult {
            workspace_ids: vec!["w1".into()],
            branch: branch.clone(),
            lookup: PullRequestLookup::Branch {
                branch: branch.clone(),
                pull_request: Some(pull_request(7)),
            },
        }]));

        // A lookup that saw another branch clears it.
        assert!(app.handle_pull_requests_refreshed(vec![PullRequestResult {
            workspace_ids: vec!["w1".into()],
            branch: branch.clone(),
            lookup: PullRequestLookup::Branch {
                branch: "other".into(),
                pull_request: Some(pull_request(8)),
            },
        }]));
        assert_eq!(app.pull_request_for(&app.state.workspaces[0]), None);
    }

    #[tokio::test]
    async fn missing_gh_waits_longer_before_trying_again() {
        let mut app = app();
        let branch = app.state.workspaces[0].branch().expect("test branch");
        app.handle_pull_requests_refreshed(vec![PullRequestResult {
            workspace_ids: vec!["w1".into()],
            branch,
            lookup: PullRequestLookup::GhMissing,
        }]);
        assert!(app.next_pull_request_refresh > Instant::now() + PULL_REQUEST_REFRESH_INTERVAL);
        assert!(!app.pull_request_lookup_pending());
    }
}
