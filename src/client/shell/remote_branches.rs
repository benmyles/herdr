//! The new worktree dialog's remote branch tab: lists one repo's remote
//! branches and checks the picked one out as a worktree for the space.

use super::*;
use crossterm::event::{KeyCode, KeyModifiers};

/// How long ago `committed_at` was, as of `now` (both Unix seconds).
pub(super) fn branch_age(committed_at: i64, now: i64) -> String {
    let minutes = now.saturating_sub(committed_at).max(0) / 60;
    match minutes {
        0 => "just now".into(),
        1..=59 => format!("{minutes}m ago"),
        60..=1439 => format!("{}h ago", minutes / 60),
        1440..=20159 => format!("{}d ago", minutes / 1440),
        20160..=86399 => format!("{}w ago", minutes / 10080),
        86400..=525599 => format!("{}mo ago", minutes / 43200),
        _ => format!("{}y ago", minutes / 525600),
    }
}

/// Where checking out `branch` puts the worktree: named like a new worktree
/// of the branch without the repo's branch prefix, as the server does.
pub(super) fn remote_branch_checkout_preview(
    template: &str,
    space_name: &str,
    repo: &crate::protocol::ClientShellRepo,
    branch: &str,
) -> String {
    if template.is_empty() {
        return "chosen by the server".to_owned();
    }
    let prefix = repo.settings.branch_prefix.as_str();
    let name = branch
        .strip_prefix(prefix)
        .filter(|name| !prefix.is_empty() && !name.is_empty())
        .unwrap_or(branch);
    crate::worktree::expand_space_path_template(template, space_name, &repo.name, name)
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

impl ClientShellState {
    pub(super) fn endpoint_supports_remote_branches(&self) -> bool {
        use crate::api::schema::{Method, RepoRemoteBranchesParams, SpaceWorktreeCheckoutParams};

        [
            Method::RepoRemoteBranches(RepoRemoteBranchesParams::default()),
            Method::SpaceWorktreeCheckout(SpaceWorktreeCheckoutParams {
                space_id: String::new(),
                repo: String::new(),
                branch: String::new(),
                focus: false,
            }),
        ]
        .iter()
        .all(|method| self.supports_endpoint_method(method))
    }

    pub(super) fn select_space_worktree_tab(
        &mut self,
        tab: SpaceWorktreeTab,
        outcome: &mut ClientShellInput,
    ) {
        let first_repo = self.endpoint_repos().first().map(|repo| repo.name.clone());
        let Some(ClientShellOverlay::SpaceWorktree(dialog)) = self.overlay.as_mut() else {
            return;
        };
        let unavailable = tab == SpaceWorktreeTab::RemoteBranch && !dialog.remote.available;
        if dialog.creating || dialog.tab == tab || unavailable {
            return;
        }
        dialog.tab = tab;
        dialog.error = None;
        dialog.offer_without_sync = false;
        if tab == SpaceWorktreeTab::RemoteBranch && dialog.remote.repo.is_none() {
            if let Some(repo) = dialog.selected_repo.clone().or(first_repo) {
                self.list_remote_branches(repo, outcome);
            }
        }
    }

    fn toggle_space_worktree_tab(&mut self, outcome: &mut ClientShellInput) {
        let Some(ClientShellOverlay::SpaceWorktree(dialog)) = self.overlay.as_ref() else {
            return;
        };
        let next = match dialog.tab {
            SpaceWorktreeTab::NewBranch => SpaceWorktreeTab::RemoteBranch,
            SpaceWorktreeTab::RemoteBranch => SpaceWorktreeTab::NewBranch,
        };
        self.select_space_worktree_tab(next, outcome);
    }

    /// Lists `repo`'s remote branches from what is already fetched, then
    /// fetches once per dialog and lists again.
    fn list_remote_branches(&mut self, repo: String, outcome: &mut ClientShellInput) {
        use crate::api::schema::{Method, RepoRemoteBranchesParams};

        let Some(ClientShellOverlay::SpaceWorktree(dialog)) = self.overlay.as_mut() else {
            return;
        };
        let picker = &mut dialog.remote;
        let fetch = !picker.fetched.contains(&repo);
        picker.repo = Some(repo.clone());
        picker.remote = None;
        picker.branches.clear();
        picker.truncated = false;
        picker.selected = 0;
        picker.fetch_error = None;
        picker.loading = true;
        picker.fetching = false;
        dialog.error = None;
        for fetch in [false, true].into_iter().take(if fetch { 2 } else { 1 }) {
            let sent = self.push_endpoint_method_with_kind(
                Method::RepoRemoteBranches(RepoRemoteBranchesParams {
                    repo: repo.clone(),
                    fetch,
                }),
                PendingEndpointKind::RepoRemoteBranches {
                    repo: repo.clone(),
                    fetch,
                },
                outcome,
            );
            if let Some(ClientShellOverlay::SpaceWorktree(dialog)) = self.overlay.as_mut() {
                match (sent, fetch) {
                    (false, false) => dialog.remote.loading = false,
                    (true, true) => {
                        dialog.remote.fetching = true;
                        dialog.remote.fetched.push(repo.clone());
                    }
                    _ => {}
                }
            }
        }
    }

    fn select_remote_branch_repo(&mut self, index: usize, outcome: &mut ClientShellInput) {
        let Some(name) = self
            .endpoint_repos()
            .get(index)
            .map(|repo| repo.name.clone())
        else {
            return;
        };
        let Some(ClientShellOverlay::SpaceWorktree(dialog)) = self.overlay.as_mut() else {
            return;
        };
        if dialog.creating || dialog.remote.repo.as_deref() == Some(name.as_str()) {
            return;
        }
        // The new branch tab follows, so switching back keeps the repo.
        dialog.selected_repo = Some(name.clone());
        self.list_remote_branches(name, outcome);
    }

    fn cycle_remote_branch_repo(&mut self, delta: isize, outcome: &mut ClientShellInput) {
        let repos = self.endpoint_repos();
        let Some(ClientShellOverlay::SpaceWorktree(dialog)) = self.overlay.as_ref() else {
            return;
        };
        if repos.len() < 2 {
            return;
        }
        let current = dialog
            .remote
            .repo
            .as_deref()
            .and_then(|name| repos.iter().position(|repo| repo.name == name))
            .unwrap_or(0) as isize;
        let next = (current + delta).rem_euclid(repos.len() as isize) as usize;
        self.select_remote_branch_repo(next, outcome);
    }

    fn move_remote_branch_selection(&mut self, delta: isize) {
        let Some(ClientShellOverlay::SpaceWorktree(dialog)) = self.overlay.as_mut() else {
            return;
        };
        let picker = &mut dialog.remote;
        let filtered = picker.filtered_indices();
        if filtered.is_empty() {
            return;
        }
        let current = filtered
            .iter()
            .position(|index| *index == picker.selected)
            .unwrap_or(0);
        let next = (current as isize + delta).clamp(0, filtered.len() as isize - 1) as usize;
        picker.selected = filtered[next];
    }

    /// Checks out the highlighted branch, or files its existing checkout
    /// under the space.
    pub(super) fn submit_remote_branch(&mut self, outcome: &mut ClientShellInput) {
        use crate::api::schema::{Method, SpaceWorktreeCheckoutParams, SpaceWorktreeOpenParams};

        outcome.repaint = true;
        let Some(ClientShellOverlay::SpaceWorktree(dialog)) = self.overlay.as_mut() else {
            return;
        };
        if dialog.creating {
            return;
        }
        let (Some(repo), Some(index)) =
            (dialog.remote.repo.clone(), dialog.remote.selected_index())
        else {
            dialog.error = Some("pick a branch".into());
            return;
        };
        let branch = &dialog.remote.branches[index];
        dialog.remote.selected = index;
        let (method, kind) = match branch.checkout_path.clone() {
            Some(path) => (
                Method::SpaceWorktreeOpen(SpaceWorktreeOpenParams {
                    space_id: dialog.space_id.clone(),
                    path,
                    focus: true,
                }),
                PendingEndpointKind::SpaceWorktreeOpen,
            ),
            None => (
                Method::SpaceWorktreeCheckout(SpaceWorktreeCheckoutParams {
                    space_id: dialog.space_id.clone(),
                    repo: repo.clone(),
                    branch: branch.name.clone(),
                    focus: false,
                }),
                PendingEndpointKind::SpaceWorktreeCreate { repo: repo.clone() },
            ),
        };
        dialog.error = None;
        dialog.failures.clear();
        dialog.fetch_failed = false;
        dialog.warnings.clear();
        dialog.created_tabs.clear();
        let sent = self.push_endpoint_method_with_kind(method, kind, outcome);
        if let Some(ClientShellOverlay::SpaceWorktree(dialog)) = self.overlay.as_mut() {
            dialog.creating = sent;
            dialog.in_flight = if sent { vec![repo] } else { Vec::new() };
        }
    }

    /// Keys for the dialog's tab switch and its remote branch tab. Returns
    /// whether the key was handled here.
    pub(super) fn route_space_worktree_tab_key(
        &mut self,
        key: &crate::input::TerminalKey,
        outcome: &mut ClientShellInput,
    ) -> bool {
        let (code, modifiers) = crate::config::normalize_key_combo((key.code, key.modifiers));
        let ctrl = modifiers == KeyModifiers::CONTROL;
        let Some(ClientShellOverlay::SpaceWorktree(dialog)) = self.overlay.as_ref() else {
            return false;
        };
        if dialog.creating {
            return false;
        }
        if code == KeyCode::Char('t') && ctrl {
            self.toggle_space_worktree_tab(outcome);
            outcome.repaint = true;
            return true;
        }
        if dialog.tab != SpaceWorktreeTab::RemoteBranch {
            return false;
        }
        outcome.repaint = true;
        match code {
            KeyCode::Esc => self.overlay = None,
            KeyCode::Enter => self.submit_remote_branch(outcome),
            KeyCode::Up => self.move_remote_branch_selection(-1),
            KeyCode::Down => self.move_remote_branch_selection(1),
            KeyCode::PageUp => self.move_remote_branch_selection(-10),
            KeyCode::PageDown => self.move_remote_branch_selection(10),
            KeyCode::Char('p') if ctrl => self.move_remote_branch_selection(-1),
            KeyCode::Char('n') if ctrl => self.move_remote_branch_selection(1),
            KeyCode::Tab => self.cycle_remote_branch_repo(1, outcome),
            KeyCode::BackTab => self.cycle_remote_branch_repo(-1, outcome),
            _ => {
                let Some(ClientShellOverlay::SpaceWorktree(dialog)) = self.overlay.as_mut() else {
                    return true;
                };
                if dialog.remote.query.handle_key(key) == Some(true) {
                    dialog.error = None;
                    if let Some(first) = dialog.remote.filtered_indices().first().copied() {
                        dialog.remote.selected = first;
                    }
                }
            }
        }
        true
    }

    /// Pasted or typed text for the remote branch tab's filter.
    pub(super) fn insert_remote_branch_text(&mut self, text: &str) -> bool {
        let Some(ClientShellOverlay::SpaceWorktree(dialog)) = self.overlay.as_mut() else {
            return false;
        };
        if dialog.tab != SpaceWorktreeTab::RemoteBranch {
            return false;
        }
        if !dialog.creating && dialog.remote.query.insert(text) {
            dialog.error = None;
            if let Some(first) = dialog.remote.filtered_indices().first().copied() {
                dialog.remote.selected = first;
            }
        }
        true
    }

    /// Returns whether the click was on a remote branch tab target.
    pub(super) fn click_remote_branch_hit(
        &mut self,
        hit: ClientOverlayHit,
        outcome: &mut ClientShellInput,
    ) -> bool {
        match hit {
            ClientOverlayHit::SpaceWorktreeTab(tab) => self.select_space_worktree_tab(tab, outcome),
            ClientOverlayHit::RemoteBranchRepo(index) => {
                self.select_remote_branch_repo(index, outcome)
            }
            ClientOverlayHit::RemoteBranch(index) => {
                let Some(ClientShellOverlay::SpaceWorktree(dialog)) = self.overlay.as_mut() else {
                    return true;
                };
                if dialog.creating {
                    return true;
                }
                // A click highlights; clicking the highlighted branch picks it.
                if dialog.remote.selected_index() == Some(index) {
                    self.submit_remote_branch(outcome);
                } else {
                    dialog.remote.selected = index;
                    dialog.error = None;
                }
            }
            _ => return false,
        }
        true
    }

    /// Wheel and scroll for the remote branch tab. Returns whether it is
    /// showing.
    pub(super) fn scroll_remote_branches(&mut self, delta: isize) -> bool {
        let showing = matches!(
            self.overlay.as_ref(),
            Some(ClientShellOverlay::SpaceWorktree(dialog))
                if dialog.tab == SpaceWorktreeTab::RemoteBranch
        );
        if showing {
            self.move_remote_branch_selection(delta);
        }
        showing
    }

    pub(super) fn handle_remote_branches_result(
        &mut self,
        repo: String,
        fetch: bool,
        result: Result<crate::api::schema::ResponseResult, ClientShellEndpointError>,
    ) -> bool {
        use crate::api::schema::ResponseResult;

        let Some(ClientShellOverlay::SpaceWorktree(dialog)) = self.overlay.as_mut() else {
            return false;
        };
        let picker = &mut dialog.remote;
        if picker.repo.as_deref() != Some(repo.as_str()) {
            return false;
        }
        if fetch {
            picker.fetching = false;
        }
        match result {
            Ok(ResponseResult::RepoRemoteBranches {
                remote,
                branches,
                truncated,
                fetch_error,
                ..
            }) => {
                // A highlight the user moved stays on its branch across the
                // re-list; otherwise the newest branch is highlighted.
                let moved = picker.selected_index() != picker.filtered_indices().first().copied();
                let selected = picker
                    .branches
                    .get(picker.selected)
                    .filter(|_| moved)
                    .map(|branch| branch.name.clone());
                picker.selected = selected
                    .and_then(|name| branches.iter().position(|branch| branch.name == name))
                    .unwrap_or(0);
                picker.branches = branches;
                picker.remote = Some(remote);
                picker.truncated = truncated;
                picker.loading = false;
                picker.listed_at = unix_now();
                if fetch {
                    picker.fetch_error = fetch_error;
                }
                if let Some(index) = picker.selected_index() {
                    picker.selected = index;
                }
            }
            Ok(_) => {
                picker.loading = false;
                dialog.error = Some("endpoint returned an unexpected result".into());
            }
            Err(error) => {
                picker.loading = false;
                if fetch && !picker.branches.is_empty() {
                    picker.fetch_error = Some(error.message);
                } else {
                    dialog.error = Some(error.message);
                }
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::branch_age;

    #[test]
    fn branch_ages_pick_the_largest_whole_unit() {
        let now = 1_000_000_000;
        assert_eq!(branch_age(now, now), "just now");
        assert_eq!(branch_age(now + 30, now), "just now");
        assert_eq!(branch_age(now - 5 * 60, now), "5m ago");
        assert_eq!(branch_age(now - 3 * 3600, now), "3h ago");
        assert_eq!(branch_age(now - 2 * 86400, now), "2d ago");
        assert_eq!(branch_age(now - 15 * 86400, now), "2w ago");
        assert_eq!(branch_age(now - 90 * 86400, now), "3mo ago");
        assert_eq!(branch_age(now - 800 * 86400, now), "2y ago");
    }
}
