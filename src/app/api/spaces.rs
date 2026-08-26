use crate::api::schema::{
    EventData, EventEnvelope, EventKind, PinnedSpaceInfo, ResponseResult, SpacePinParams,
    SpaceTarget,
};
use crate::app::App;
use crate::space::{PinnedSpace, PinnedSpaceKey};

use super::responses::{encode_error, encode_success};

impl App {
    pub(super) fn handle_space_pin(&mut self, id: String, params: SpacePinParams) -> String {
        let Some(ws_idx) = self.parse_workspace_id(&params.workspace_id) else {
            return encode_error(
                id,
                "workspace_not_found",
                format!("workspace {} not found", params.workspace_id),
            );
        };
        let Some(workspace) = self.state.workspaces.get(ws_idx) else {
            return encode_error(
                id,
                "workspace_not_found",
                format!("workspace {} not found", params.workspace_id),
            );
        };
        let key = PinnedSpaceKey::from_workspace(workspace);
        if let Some(pin_idx) = self
            .state
            .pinned_spaces
            .iter()
            .position(|pin| pin.key == key)
        {
            return encode_success(
                id,
                ResponseResult::PinnedSpaceInfo {
                    space: self.pinned_space_info(pin_idx),
                },
            );
        }

        let descriptor_idx = match &key {
            PinnedSpaceKey::Workspace { .. } => ws_idx,
            PinnedSpaceKey::Worktree { key } => self
                .state
                .workspaces
                .iter()
                .enumerate()
                .find(|(_, candidate)| {
                    candidate
                        .worktree_space()
                        .is_some_and(|space| space.key == *key && !space.is_linked_worktree)
                })
                .map(|(idx, _)| idx)
                .unwrap_or(ws_idx),
        };
        let descriptor = &self.state.workspaces[descriptor_idx];
        let label = descriptor.display_name_from(&self.state.terminals, &self.terminal_runtimes);
        let cwd = descriptor
            .resolved_identity_cwd_from(&self.state.terminals, &self.terminal_runtimes)
            .unwrap_or_else(|| descriptor.identity_cwd.clone());
        self.state.pinned_spaces.push(PinnedSpace::new(
            key,
            label,
            cwd,
            descriptor_idx,
            descriptor.worktree_space().cloned(),
        ));
        let pin_idx = self.state.pinned_spaces.len() - 1;
        let affected = self
            .state
            .workspaces
            .iter()
            .enumerate()
            .filter_map(|(idx, workspace)| {
                self.state.pinned_spaces[pin_idx]
                    .matches_workspace(workspace)
                    .then_some(idx)
            })
            .collect::<Vec<_>>();
        self.schedule_session_save();
        for idx in affected {
            self.emit_event(EventEnvelope {
                event: EventKind::WorkspaceUpdated,
                data: EventData::WorkspaceUpdated {
                    workspace: self.workspace_info(idx),
                },
            });
        }
        encode_success(
            id,
            ResponseResult::PinnedSpaceInfo {
                space: self.pinned_space_info(pin_idx),
            },
        )
    }

    pub(super) fn handle_space_unpin(&mut self, id: String, target: SpaceTarget) -> String {
        let Some(pin_idx) = self
            .state
            .pinned_spaces
            .iter()
            .position(|pin| pin.id == target.space_id)
        else {
            return encode_error(
                id,
                "space_not_found",
                format!("space {} not found", target.space_id),
            );
        };
        let pin = self.state.pinned_spaces.remove(pin_idx);
        let affected = self
            .state
            .workspaces
            .iter()
            .enumerate()
            .filter_map(|(idx, workspace)| pin.matches_workspace(workspace).then_some(idx))
            .collect::<Vec<_>>();
        self.schedule_session_save();
        for idx in affected {
            self.emit_event(EventEnvelope {
                event: EventKind::WorkspaceUpdated,
                data: EventData::WorkspaceUpdated {
                    workspace: self.workspace_info(idx),
                },
            });
        }
        encode_success(id, ResponseResult::Ok {})
    }

    pub(super) fn handle_space_open(&mut self, id: String, target: SpaceTarget) -> String {
        let Some(pin_idx) = self
            .state
            .pinned_spaces
            .iter()
            .position(|pin| pin.id == target.space_id)
        else {
            return encode_error(
                id,
                "space_not_found",
                format!("space {} not found", target.space_id),
            );
        };
        if let Some(ws_idx) =
            self.state.workspaces.iter().position(|workspace| {
                self.state.pinned_spaces[pin_idx].matches_workspace(workspace)
            })
        {
            self.state.switch_workspace(ws_idx);
            return encode_success(
                id,
                ResponseResult::PinnedSpaceInfo {
                    space: self.pinned_space_info(pin_idx),
                },
            );
        }

        let pin = self.state.pinned_spaces[pin_idx].clone();
        let ws_idx = match self.create_workspace_with_launch_env(pin.cwd.clone(), true, Vec::new())
        {
            Ok(ws_idx) => ws_idx,
            Err(err) => return encode_error(id, "space_open_failed", err.to_string()),
        };
        if let Some(workspace) = self.state.workspaces.get_mut(ws_idx) {
            match &pin.key {
                PinnedSpaceKey::Workspace { workspace_id } => {
                    workspace.id.clone_from(workspace_id);
                }
                PinnedSpaceKey::Worktree { .. } => {
                    workspace.worktree_space.clone_from(&pin.worktree_space);
                }
            }
            workspace.set_custom_name(pin.label.clone());
        }
        self.emit_workspace_open_events(ws_idx);
        self.schedule_session_save();
        encode_success(
            id,
            ResponseResult::PinnedSpaceInfo {
                space: self.pinned_space_info(pin_idx),
            },
        )
    }

    pub(super) fn pinned_space_info(&self, pin_idx: usize) -> PinnedSpaceInfo {
        let pin = &self.state.pinned_spaces[pin_idx];
        let workspace_ids = self
            .state
            .workspaces
            .iter()
            .enumerate()
            .filter(|(_, workspace)| pin.matches_workspace(workspace))
            .map(|(idx, _)| self.public_workspace_id(idx))
            .collect::<Vec<_>>();
        PinnedSpaceInfo {
            space_id: pin.id.clone(),
            label: pin.label.clone(),
            cwd: pin.cwd.display().to_string(),
            live: !workspace_ids.is_empty(),
            workspace_ids,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::schema::{ResponseResult, SuccessResponse};
    use crate::workspace::{Workspace, WorktreeSpaceMembership};

    fn app_with_workspaces(workspaces: Vec<Workspace>) -> App {
        let (_api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(
            &crate::config::Config::default(),
            true,
            None,
            api_rx,
            crate::api::EventHub::default(),
        );
        app.state.workspaces = workspaces;
        app.state.active = Some(0);
        app.state.selected = 0;
        app.state.ensure_test_terminals();
        app
    }

    fn pin(app: &mut App, ws_idx: usize) -> String {
        let response = app.handle_space_pin(
            "pin".into(),
            SpacePinParams {
                workspace_id: app.public_workspace_id(ws_idx),
            },
        );
        let success: SuccessResponse = serde_json::from_str(&response).expect("pin response");
        let ResponseResult::PinnedSpaceInfo { space } = success.result else {
            panic!("expected pinned space response");
        };
        space.space_id
    }

    #[test]
    fn pin_is_idempotent_and_survives_last_workspace_close() {
        let mut app = app_with_workspaces(vec![Workspace::test_new("project")]);
        let space_id = pin(&mut app, 0);
        assert_eq!(pin(&mut app, 0), space_id);
        assert_eq!(app.state.pinned_spaces.len(), 1);
        assert!(app.workspace_info(0).pinned);

        app.state.close_selected_workspace();
        assert!(app.state.workspaces.is_empty());
        assert_eq!(app.state.pinned_spaces.len(), 1);
        assert!(!app.pinned_space_info(0).live);
        app.state.assert_invariants_for_test();

        let response = app.handle_space_unpin("unpin".into(), SpaceTarget { space_id });
        let success: SuccessResponse = serde_json::from_str(&response).expect("unpin response");
        assert!(matches!(success.result, ResponseResult::Ok {}));
        assert!(app.state.pinned_spaces.is_empty());
    }

    #[test]
    fn pinning_any_worktree_member_pins_the_group_once() {
        let membership = WorktreeSpaceMembership {
            key: "repo-key".into(),
            label: "repo".into(),
            repo_root: "/repo".into(),
            checkout_path: "/repo".into(),
            is_linked_worktree: false,
        };
        let mut parent = Workspace::test_new("parent");
        parent.worktree_space = Some(membership.clone());
        let mut child = Workspace::test_new("child");
        child.worktree_space = Some(WorktreeSpaceMembership {
            checkout_path: "/repo/child".into(),
            is_linked_worktree: true,
            ..membership
        });
        let mut app = app_with_workspaces(vec![parent, child]);

        let child_pin = pin(&mut app, 1);
        let parent_pin = pin(&mut app, 0);

        assert_eq!(child_pin, parent_pin);
        assert_eq!(app.state.pinned_spaces.len(), 1);
        assert_eq!(app.pinned_space_info(0).workspace_ids.len(), 2);
        assert!(app.workspace_info(0).pinned);
        assert!(app.workspace_info(1).pinned);
    }

    #[tokio::test]
    async fn opening_dormant_pin_materializes_the_original_workspace_identity() {
        use super::super::test_support::{exiting_test_command, shutdown_test_runtimes};

        let mut app = app_with_workspaces(vec![Workspace::test_new("project")]);
        let original_workspace_id = app.state.workspaces[0].id.clone();
        let space_id = pin(&mut app, 0);
        app.state.close_selected_workspace();
        app.state.pinned_spaces[0].cwd =
            std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("/"));
        app.state.default_shell = exiting_test_command().into();
        app.state.shell_mode = crate::config::ShellModeConfig::NonLogin;

        let response = app.handle_space_open("open".into(), SpaceTarget { space_id });
        let success: SuccessResponse = serde_json::from_str(&response).expect("open response");
        let ResponseResult::PinnedSpaceInfo { space } = success.result else {
            panic!("expected opened pinned space response");
        };

        assert!(space.live);
        assert_eq!(app.state.workspaces.len(), 1);
        assert_eq!(app.state.workspaces[0].id, original_workspace_id);
        assert_eq!(app.state.active, Some(0));
        assert!(app.workspace_info(0).pinned);
        shutdown_test_runtimes(&mut app);
    }
}
