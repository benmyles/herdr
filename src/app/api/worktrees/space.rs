use std::path::PathBuf;

use crate::api::schema::{
    RemoteBranchInfo, RepoRemoteBranchesParams, ResponseResult, SpaceWorktreeCheckoutParams,
    SpaceWorktreeCreateParams, SpaceWorktreeCreatedInfo, WorktreeBranchSource, WorktreeSyncInfo,
};
use crate::app::App;
use crate::events::{ApiWorktreeAddRequest, AppEvent, WorktreeAddResult};
use crate::worktree::{BranchSource, SpaceWorktreeReport};

use super::super::responses::{encode_error, encode_success};

/// A space worktree to create, from `space.worktree.create` or
/// `space.worktree.checkout`.
struct SpaceWorktreeRequest {
    space_id: String,
    repo: String,
    /// Names the checkout folder.
    name: String,
    /// The branch as-is; `None` derives it from the name and branch prefix.
    branch: Option<String>,
    sync: bool,
    focus: bool,
}

fn sync_info(report: &SpaceWorktreeReport) -> WorktreeSyncInfo {
    let (branch_source, start_point) = match &report.branch_source {
        BranchSource::New { start_point } => (WorktreeBranchSource::New, Some(start_point.clone())),
        BranchSource::Local => (WorktreeBranchSource::Local, None),
        BranchSource::Remote { upstream } => (WorktreeBranchSource::Remote, Some(upstream.clone())),
        BranchSource::ExistingCheckout => (WorktreeBranchSource::Existing, None),
    };
    WorktreeSyncInfo {
        fetched: report.fetched,
        root_updated: report.root_updated,
        branch_source,
        start_point,
        warnings: report.warnings.clone(),
    }
}

impl App {
    /// The checkout path a space worktree gets, before any git work.
    pub(crate) fn space_worktree_checkout_path(
        &self,
        space_name: &str,
        repo_name: &str,
        name: &str,
    ) -> PathBuf {
        PathBuf::from(crate::worktree::expand_space_path_template(
            &self.state.worktree_path_template,
            space_name,
            repo_name,
            name,
        ))
    }

    pub(super) fn start_space_worktree_create(
        &mut self,
        id: String,
        params: SpaceWorktreeCreateParams,
        respond_to: std::sync::mpsc::Sender<String>,
    ) {
        let request = SpaceWorktreeRequest {
            space_id: params.space_id,
            repo: params.repo,
            name: params.name.trim().to_owned(),
            branch: None,
            sync: params.sync,
            focus: params.focus,
        };
        if request.name.is_empty() {
            Self::send_api_response(
                respond_to,
                encode_error(id, "invalid_request", "worktree name is required"),
            );
            return;
        }
        self.start_space_worktree(id, request, respond_to);
    }

    /// An existing branch as a new worktree. It is named after the branch
    /// without the repo's branch prefix, so its checkout lands where a
    /// worktree created under that name would.
    pub(super) fn start_space_worktree_checkout(
        &mut self,
        id: String,
        params: SpaceWorktreeCheckoutParams,
        respond_to: std::sync::mpsc::Sender<String>,
    ) {
        let branch = params.branch.trim().to_owned();
        if branch.is_empty() {
            Self::send_api_response(
                respond_to,
                encode_error(id, "invalid_request", "branch is required"),
            );
            return;
        }
        let prefix = crate::repos::find(&self.state.repos, &params.repo)
            .map(|repo| repo.settings.branch_prefix.as_str())
            .unwrap_or_default();
        let name = branch
            .strip_prefix(prefix)
            .filter(|name| !prefix.is_empty() && !name.is_empty())
            .unwrap_or(&branch)
            .to_owned();
        let request = SpaceWorktreeRequest {
            space_id: params.space_id,
            repo: params.repo,
            name,
            branch: Some(branch),
            // The branch list fetched already; checking out needs no base.
            sync: false,
            focus: params.focus,
        };
        self.start_space_worktree(id, request, respond_to);
    }

    fn start_space_worktree(
        &mut self,
        id: String,
        params: SpaceWorktreeRequest,
        respond_to: std::sync::mpsc::Sender<String>,
    ) {
        let fail = |respond_to, code: &str, message: String| {
            Self::send_api_response(respond_to, encode_error(id.clone(), code, message));
        };
        let Some(space_idx) = self.state.space_index(&params.space_id) else {
            fail(
                respond_to,
                "space_not_found",
                format!("space {} not found", params.space_id),
            );
            return;
        };
        let Some(repo) = crate::repos::find(&self.state.repos, &params.repo).cloned() else {
            fail(
                respond_to,
                "repo_not_found",
                format!("repo {} not found", params.repo),
            );
            return;
        };
        let name = params.name;
        let repo_root = repo.root_path();
        let Some(git_space) = crate::workspace::git_space_metadata(&repo_root) else {
            fail(
                respond_to,
                "repo_unavailable",
                format!("{} is not a Git repo on this machine", repo.root),
            );
            return;
        };
        let checkout_path = self.space_worktree_checkout_path(
            &self.state.spaces[space_idx].name,
            &repo.name,
            &name,
        );
        if !checkout_path.is_absolute() {
            fail(
                respond_to,
                "invalid_worktree_path",
                format!(
                    "[worktrees] path must produce an absolute path, got {}",
                    checkout_path.display()
                ),
            );
            return;
        }
        let checkout_key = crate::worktree::canonical_or_original(&checkout_path);
        if self
            .pending_api_worktree_creates
            .contains_key(&checkout_key)
            || self
                .pending_api_worktree_remove_paths
                .contains_key(&checkout_key)
        {
            fail(
                respond_to,
                "worktree_operation_in_progress",
                "worktree operation is already in progress for this checkout".into(),
            );
            return;
        }
        let operation_id = self.next_api_worktree_operation_id();
        self.pending_api_worktree_creates
            .insert(checkout_key.clone(), operation_id);

        let branch = params.branch.unwrap_or_else(|| repo.branch_for(&name));
        let space_name = self.state.spaces[space_idx].name.clone();
        let settings = &repo.settings;
        let setup = crate::events::WorktreeSetupPlan {
            on_create: (!settings.on_create.is_empty()).then(|| settings.on_create.clone()),
            start_command: (!settings.start_command.is_empty())
                .then(|| settings.start_command.clone()),
            env: crate::worktree::hook_env(&repo, &checkout_path, &branch, &space_name),
        };
        let plan = crate::worktree::SpaceWorktreePlan {
            repo_root: git_space.repo_root.clone(),
            checkout_path: checkout_path.clone(),
            branch,
            base_branch: repo.base_branch.clone(),
            remote: repo.remote.clone(),
            sync: params.sync,
            copy_files: repo.settings.copy_files.clone(),
        };
        let api_request = ApiWorktreeAddRequest {
            id,
            operation_id,
            checkout_key,
            source_workspace_id: None,
            source_existing_membership: None,
            source_checkout_path: git_space.repo_root.clone(),
            source_repo_root: git_space.repo_root,
            repo_key: git_space.key,
            repo_name: repo.name.clone(),
            label: Some(repo.name),
            focus: params.focus,
            space_id: Some(params.space_id),
            setup: Some(setup),
            respond_to,
        };
        let event_tx = self.event_tx.clone();
        std::thread::spawn(move || {
            let outcome = crate::worktree::create_space_worktree(&plan);
            let result = outcome
                .as_ref()
                .map(|_| ())
                .map_err(|failure| failure.message.clone());
            let _ = event_tx.blocking_send(AppEvent::WorktreeAddFinished(Box::new(
                WorktreeAddResult {
                    path: checkout_path,
                    api_request: Some(api_request),
                    result,
                    space_outcome: Some(outcome.map_err(|failure| failure.code)),
                },
            )));
        });
    }

    pub(super) fn start_repo_remote_branches(
        &mut self,
        id: String,
        params: RepoRemoteBranchesParams,
        respond_to: std::sync::mpsc::Sender<String>,
    ) {
        let Some(repo) = crate::repos::find(&self.state.repos, &params.repo).cloned() else {
            Self::send_api_response(
                respond_to,
                encode_error(
                    id,
                    "repo_not_found",
                    format!("repo {} not found", params.repo),
                ),
            );
            return;
        };
        let Some(remote) = repo.remote.clone().filter(|remote| !remote.is_empty()) else {
            Self::send_api_response(
                respond_to,
                encode_error(
                    id,
                    "repo_has_no_remote",
                    format!("repo {} has no remote", repo.name),
                ),
            );
            return;
        };
        let root = repo.root_path();
        std::thread::spawn(move || {
            let response = match crate::worktree::list_remote_branches(&root, &remote, params.fetch)
            {
                Ok(list) => encode_success(
                    id,
                    ResponseResult::RepoRemoteBranches {
                        repo: repo.name,
                        remote,
                        branches: list
                            .branches
                            .into_iter()
                            .map(|branch| RemoteBranchInfo {
                                name: branch.name,
                                committed_at: branch.committed_at,
                                author: branch.author,
                                subject: branch.subject,
                                checkout_path: branch
                                    .checkout
                                    .map(|path| path.display().to_string()),
                            })
                            .collect(),
                        truncated: list.truncated,
                        fetch_error: list.fetch_error,
                    },
                ),
                Err(err) => encode_error(id, "repo_unavailable", err),
            };
            Self::send_api_response(respond_to, response);
        });
    }

    pub(super) fn finish_space_worktree_create(
        &mut self,
        api: ApiWorktreeAddRequest,
        result: WorktreeAddResult,
    ) {
        let report = match (result.result, result.space_outcome) {
            (Ok(()), Some(Ok(report))) => report,
            (Err(message), Some(Err(code))) => {
                Self::send_api_response(api.respond_to, encode_error(api.id, code, message));
                return;
            }
            (Err(message), _) => {
                Self::send_api_response(
                    api.respond_to,
                    encode_error(api.id, "worktree_create_failed", message),
                );
                return;
            }
            (Ok(()), _) => {
                Self::send_api_response(
                    api.respond_to,
                    encode_error(
                        api.id,
                        "worktree_create_failed",
                        "worktree creation finished without a report",
                    ),
                );
                return;
            }
        };
        let order_before = self
            .state
            .workspaces
            .iter()
            .map(|workspace| workspace.id.clone())
            .collect::<Vec<_>>();
        // The space may have been deleted while git ran; `other` always exists.
        let space_id = api
            .space_id
            .clone()
            .filter(|space_id| self.state.space_index(space_id).is_some())
            .unwrap_or_else(|| crate::space::OTHER_SPACE_ID.to_owned());
        // A reused checkout was set up when it was first created.
        let setup = api
            .setup
            .filter(|_| report.branch_source != BranchSource::ExistingCheckout);
        let launch_env = setup
            .iter()
            .flat_map(|setup| &setup.env)
            .map(|(key, value)| ((*key).to_owned(), value.clone()))
            .collect();
        let (ws_idx, created) = match self.open_workspace_idx_for_checkout(&result.path) {
            Some(ws_idx) => {
                if api.focus {
                    self.state.switch_workspace(ws_idx);
                }
                (ws_idx, false)
            }
            None => match self.create_workspace_with_launch_env(
                result.path.clone(),
                api.focus,
                launch_env,
            ) {
                Ok(ws_idx) => (ws_idx, true),
                Err(err) => {
                    Self::send_api_response(
                        api.respond_to,
                        encode_error(
                            api.id,
                            "worktree_open_failed",
                            format!("created worktree but failed to open workspace: {err}"),
                        ),
                    );
                    return;
                }
            },
        };
        let membership = crate::workspace::WorktreeSpaceMembership {
            key: api.repo_key,
            label: api.repo_name,
            repo_root: api.source_repo_root,
            checkout_path: result.path,
            is_linked_worktree: true,
        };
        self.set_worktree_membership(ws_idx, membership, !created);
        let workspace_id = {
            let workspace = &mut self.state.workspaces[ws_idx];
            if created {
                if let Some(label) = api.label {
                    workspace.set_custom_name(label);
                }
                workspace.space_id.clone_from(&space_id);
            }
            workspace.id.clone()
        };
        if created {
            self.state.normalize_spaces();
        } else if self.state.workspaces[ws_idx].space_id != space_id {
            let _ = self
                .state
                .assign_workspace_to_space(&workspace_id, &space_id, None);
        }
        self.state.mark_session_dirty();
        let Some(ws_idx) = self
            .state
            .workspaces
            .iter()
            .position(|workspace| workspace.id == workspace_id)
        else {
            Self::send_api_response(
                api.respond_to,
                encode_error(
                    api.id,
                    "worktree_open_failed",
                    "created worktree but its workspace disappeared",
                ),
            );
            return;
        };
        if created {
            self.emit_workspace_open_events(ws_idx);
        } else {
            self.emit_space_workspace_reorder(&order_before);
        }
        let Some(worktree) = self.worktree_info_for_workspace(ws_idx) else {
            Self::send_api_response(
                api.respond_to,
                encode_error(
                    api.id,
                    "worktree_open_failed",
                    "created worktree but failed to record workspace membership",
                ),
            );
            return;
        };
        if report.branch_source != BranchSource::ExistingCheckout {
            self.emit_worktree_created_event(ws_idx, worktree.clone());
        }
        if let Some(setup) = setup {
            self.start_worktree_setup(ws_idx, setup);
        }
        let tab_idx = self.state.workspaces[ws_idx].active_tab;
        let (Some(tab), Some(root_pane)) = (
            self.tab_info(ws_idx, tab_idx),
            self.root_pane_info(ws_idx, tab_idx),
        ) else {
            Self::send_api_response(
                api.respond_to,
                encode_error(
                    api.id,
                    "worktree_open_failed",
                    "created worktree but its workspace has no active pane",
                ),
            );
            return;
        };
        let response = encode_success(
            api.id,
            ResponseResult::SpaceWorktreeCreated(Box::new(SpaceWorktreeCreatedInfo {
                workspace: self.workspace_info(ws_idx),
                tab,
                root_pane,
                worktree,
                sync: sync_info(&report),
            })),
        );
        Self::send_api_response(api.respond_to, response);
    }
}

impl App {
    /// The configured repo a checkout belongs to, matched by Git common dir.
    pub(in crate::app::api) fn configured_repo_for_key(
        &self,
        key: &str,
    ) -> Option<&crate::repos::Repo> {
        self.state.repos.iter().find(|repo| {
            crate::workspace::git_space_metadata(&repo.root_path())
                .is_some_and(|space| space.key == key)
        })
    }

    pub(in crate::app::api) fn handle_space_worktree_open(
        &mut self,
        id: String,
        params: crate::api::schema::SpaceWorktreeOpenParams,
    ) -> String {
        if self.state.space_index(&params.space_id).is_none() {
            return encode_error(
                id,
                "space_not_found",
                format!("space {} not found", params.space_id),
            );
        }
        let path = crate::worktree::expand_tilde_path(params.path.trim());
        if !path.is_absolute() {
            return encode_error(id, "invalid_request", "worktree path must be absolute");
        }
        let Some(git_space) = crate::workspace::git_space_metadata(&path) else {
            return encode_error(
                id,
                "not_git_worktree",
                format!("{} is not a Git checkout", path.display()),
            );
        };
        let checkout = git_space.repo_root.clone();
        let repo = self.configured_repo_for_key(&git_space.key).cloned();
        let label = repo
            .as_ref()
            .map(|repo| repo.name.clone())
            .unwrap_or_else(|| git_space.repo_name.clone());
        let main_root = repo
            .as_ref()
            .map(crate::repos::Repo::root_path)
            .unwrap_or_else(|| {
                let common = std::path::Path::new(&git_space.key);
                match common.file_name().and_then(|name| name.to_str()) {
                    Some(".git") => common.parent().unwrap_or(common).to_path_buf(),
                    _ => checkout.clone(),
                }
            });
        let order_before = self
            .state
            .workspaces
            .iter()
            .map(|workspace| workspace.id.clone())
            .collect::<Vec<_>>();
        let (ws_idx, created) = match self.open_workspace_idx_for_checkout(&checkout) {
            Some(ws_idx) => (ws_idx, false),
            None => match self.create_workspace_with_options(checkout.clone(), params.focus) {
                Ok(ws_idx) => (ws_idx, true),
                Err(err) => {
                    return encode_error(id, "worktree_open_failed", err.to_string());
                }
            },
        };
        if self.state.workspaces[ws_idx].worktree_space().is_none() {
            let membership = crate::workspace::WorktreeSpaceMembership {
                key: git_space.key,
                label: label.clone(),
                repo_root: main_root,
                checkout_path: checkout,
                is_linked_worktree: git_space.is_linked_worktree,
            };
            self.set_worktree_membership(ws_idx, membership, !created);
        }
        let workspace_id = self.state.workspaces[ws_idx].id.clone();
        if created {
            let workspace = &mut self.state.workspaces[ws_idx];
            workspace.set_custom_name(label);
            workspace.space_id.clone_from(&params.space_id);
            self.state.normalize_spaces();
        } else if let Err(error) =
            self.state
                .assign_workspace_to_space(&workspace_id, &params.space_id, None)
        {
            return encode_error(id, error.code(), error.to_string());
        }
        self.state.mark_session_dirty();
        let Some(ws_idx) = self.parse_workspace_id(&workspace_id) else {
            return encode_error(id, "worktree_open_failed", "workspace disappeared");
        };
        if created {
            self.emit_workspace_open_events(ws_idx);
        } else {
            self.emit_space_workspace_reorder(&order_before);
            if params.focus {
                self.state.switch_workspace(ws_idx);
            }
        }
        self.schedule_session_save();
        encode_success(
            id,
            ResponseResult::WorkspaceInfo {
                workspace: self.workspace_info(ws_idx),
            },
        )
    }
}
