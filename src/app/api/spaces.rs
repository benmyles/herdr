use crate::api::schema::{
    ClosedSpaceMemberInfo, EmptyParams, EventData, EventEnvelope, EventKind, ResponseResult,
    SpaceAssignParams, SpaceCreateParams, SpaceInfo, SpaceMemberTarget, SpaceMoveParams,
    SpaceRenameParams, SpaceTarget,
};
use crate::app::spaces::SpaceError;
use crate::app::App;

use super::responses::{encode_error, encode_success};

fn space_error(id: String, error: SpaceError) -> String {
    encode_error(id, error.code(), error.to_string())
}

impl App {
    pub(crate) fn space_infos(&self) -> Vec<SpaceInfo> {
        (0..self.state.spaces.len())
            .map(|idx| self.space_info(idx))
            .collect()
    }

    pub(crate) fn space_info(&self, space_idx: usize) -> SpaceInfo {
        let space = &self.state.spaces[space_idx];
        SpaceInfo {
            space_id: space.id.clone(),
            name: space.name.clone(),
            color: space.color,
            built_in: space.is_other(),
            workspace_ids: self
                .state
                .workspaces
                .iter()
                .enumerate()
                .filter(|(_, workspace)| workspace.space_id == space.id)
                .map(|(idx, _)| self.public_workspace_id(idx))
                .collect(),
            closed: space
                .closed
                .iter()
                .map(|member| ClosedSpaceMemberInfo {
                    member_id: member.id.clone(),
                    label: member
                        .custom_name
                        .clone()
                        .unwrap_or_else(|| member.label.clone()),
                    cwd: member.cwd.display().to_string(),
                    branch: member.branch.clone(),
                })
                .collect(),
        }
    }

    fn space_info_response(&self, id: String, space_id: &str) -> String {
        match self.state.space_index(space_id) {
            Some(idx) => encode_success(
                id,
                ResponseResult::SpaceInfo {
                    space: self.space_info(idx),
                },
            ),
            None => space_error(id, SpaceError::NotFound(space_id.to_owned())),
        }
    }

    /// Workspace order changed under a space operation; tell subscribers the
    /// same way a workspace reorder does.
    fn emit_space_workspace_reorder(&mut self, before: &[String]) {
        let after = self
            .state
            .workspaces
            .iter()
            .map(|workspace| workspace.id.clone())
            .collect::<Vec<_>>();
        if after == before {
            return;
        }
        let workspaces = (0..self.state.workspaces.len())
            .map(|idx| self.workspace_info(idx))
            .collect();
        self.emit_event(EventEnvelope {
            event: EventKind::WorkspaceReordered,
            data: EventData::WorkspaceReordered {
                workspace_ids: after,
                before_workspace_id: None,
                workspaces,
            },
        });
    }

    fn workspace_order_ids(&self) -> Vec<String> {
        self.state
            .workspaces
            .iter()
            .map(|workspace| workspace.id.clone())
            .collect()
    }

    pub(super) fn handle_space_list(&self, id: String, _params: EmptyParams) -> String {
        encode_success(
            id,
            ResponseResult::SpaceList {
                spaces: self.space_infos(),
            },
        )
    }

    pub(super) fn handle_space_create(&mut self, id: String, params: SpaceCreateParams) -> String {
        let workspace_id = match params.workspace_id.as_deref() {
            Some(workspace_id) => match self.parse_workspace_id(workspace_id) {
                Some(idx) => Some(self.state.workspaces[idx].id.clone()),
                None => {
                    return space_error(id, SpaceError::WorkspaceNotFound(workspace_id.to_owned()))
                }
            },
            None => None,
        };
        let before = self.workspace_order_ids();
        let space_id = match self.state.create_space(&params.name) {
            Ok(space_id) => space_id,
            Err(error) => return space_error(id, error),
        };
        if let Some(workspace_id) = workspace_id {
            if let Err(error) = self
                .state
                .assign_workspace_to_space(&workspace_id, &space_id, None)
            {
                return space_error(id, error);
            }
            self.emit_space_workspace_reorder(&before);
        }
        self.schedule_session_save();
        self.space_info_response(id, &space_id)
    }

    pub(super) fn handle_space_rename(&mut self, id: String, params: SpaceRenameParams) -> String {
        if let Err(error) = self.state.rename_space(&params.space_id, &params.name) {
            return space_error(id, error);
        }
        self.schedule_session_save();
        self.space_info_response(id, &params.space_id)
    }

    pub(super) fn handle_space_delete(&mut self, id: String, target: SpaceTarget) -> String {
        let before = self.workspace_order_ids();
        if let Err(error) = self.state.delete_space(&target.space_id) {
            return space_error(id, error);
        }
        self.emit_space_workspace_reorder(&before);
        self.schedule_session_save();
        encode_success(id, ResponseResult::Ok {})
    }

    pub(super) fn handle_space_move(&mut self, id: String, params: SpaceMoveParams) -> String {
        let before = self.workspace_order_ids();
        if let Err(error) = self
            .state
            .move_space(&params.space_id, params.before_space_id.as_deref())
        {
            return space_error(id, error);
        }
        self.emit_space_workspace_reorder(&before);
        self.schedule_session_save();
        encode_success(
            id,
            ResponseResult::SpaceList {
                spaces: self.space_infos(),
            },
        )
    }

    pub(super) fn handle_space_assign(&mut self, id: String, params: SpaceAssignParams) -> String {
        let Some(ws_idx) = self.parse_workspace_id(&params.workspace_id) else {
            return space_error(id, SpaceError::WorkspaceNotFound(params.workspace_id));
        };
        let workspace_id = self.state.workspaces[ws_idx].id.clone();
        let before_workspace_id = match params.before_workspace_id.as_deref() {
            Some(before) => match self.parse_workspace_id(before) {
                Some(idx) => Some(self.state.workspaces[idx].id.clone()),
                None => return space_error(id, SpaceError::WorkspaceNotFound(before.to_owned())),
            },
            None => None,
        };
        let before = self.workspace_order_ids();
        match self.state.assign_workspace_to_space(
            &workspace_id,
            &params.space_id,
            before_workspace_id.as_deref(),
        ) {
            Ok(false) => {}
            Ok(true) => {
                self.emit_space_workspace_reorder(&before);
                if let Some(ws_idx) = self.parse_workspace_id(&workspace_id) {
                    self.emit_event(EventEnvelope {
                        event: EventKind::WorkspaceUpdated,
                        data: EventData::WorkspaceUpdated {
                            workspace: self.workspace_info(ws_idx),
                        },
                    });
                }
                self.schedule_session_save();
            }
            Err(error) => return space_error(id, error),
        }
        self.space_info_response(id, &params.space_id)
    }

    pub(super) fn handle_space_member_open(
        &mut self,
        id: String,
        target: SpaceMemberTarget,
    ) -> String {
        let member = match self
            .state
            .take_closed_member(&target.space_id, &target.member_id)
        {
            Ok(member) => member,
            Err(error) => return space_error(id, error),
        };
        let ws_idx = match self.create_workspace_with_launch_env(
            member.cwd.clone(),
            target.focus,
            Vec::new(),
        ) {
            Ok(ws_idx) => ws_idx,
            Err(err) => {
                self.state.restore_closed_member(&target.space_id, member);
                return encode_error(id, "space_member_open_failed", err.to_string());
            }
        };
        let workspace_id = {
            let workspace = &mut self.state.workspaces[ws_idx];
            workspace.space_id.clone_from(&target.space_id);
            workspace.worktree_space.clone_from(&member.worktree_space);
            if let Some(name) = member.custom_name.clone() {
                workspace.set_custom_name(name);
            }
            workspace.id.clone()
        };
        self.state.normalize_spaces();
        let Some(ws_idx) = self.parse_workspace_id(&workspace_id) else {
            return space_error(id, SpaceError::WorkspaceNotFound(workspace_id));
        };
        self.emit_workspace_open_events(ws_idx);
        self.schedule_session_save();
        encode_success(
            id,
            ResponseResult::WorkspaceInfo {
                workspace: self.workspace_info(ws_idx),
            },
        )
    }

    pub(super) fn handle_space_member_remove(
        &mut self,
        id: String,
        target: SpaceMemberTarget,
    ) -> String {
        if let Err(error) = self
            .state
            .take_closed_member(&target.space_id, &target.member_id)
        {
            return space_error(id, error);
        }
        self.schedule_session_save();
        self.space_info_response(id, &target.space_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::schema::{ErrorResponse, SuccessResponse};
    use crate::workspace::Workspace;

    fn app_with_workspaces(workspaces: Vec<Workspace>) -> App {
        let (_api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(
            &crate::config::Config::default(),
            crate::app::AppPolicy::TEST,
            None,
            api_rx,
            crate::api::EventHub::default(),
        );
        app.state.workspaces = workspaces;
        app.state.active = Some(0);
        app.state.selected = 0;
        app.state.ensure_test_terminals();
        app.state.normalize_spaces();
        app
    }

    fn success(response: &str) -> ResponseResult {
        serde_json::from_str::<SuccessResponse>(response)
            .unwrap_or_else(|err| panic!("expected success, got {response}: {err}"))
            .result
    }

    fn create(app: &mut App, name: &str, workspace_id: Option<String>) -> SpaceInfo {
        let ResponseResult::SpaceInfo { space } = success(&app.handle_space_create(
            "create".into(),
            SpaceCreateParams {
                name: name.into(),
                workspace_id,
            },
        )) else {
            panic!("expected space info");
        };
        space
    }

    #[test]
    fn create_files_the_workspace_and_lists_other_last() {
        let mut app = app_with_workspaces(vec![Workspace::test_new("a"), Workspace::test_new("b")]);
        let b = app.public_workspace_id(1);
        let space = create(&mut app, "knowledge", Some(b.clone()));
        assert_eq!(space.workspace_ids, vec![b.clone()]);
        assert!(!space.built_in);
        assert_eq!(
            app.workspace_info(0).space_id.as_deref(),
            Some(space.space_id.as_str())
        );

        let ResponseResult::SpaceList { spaces } =
            success(&app.handle_space_list("list".into(), EmptyParams::default()))
        else {
            panic!("expected space list");
        };
        assert_eq!(spaces.len(), 2);
        assert!(spaces[1].built_in);
        assert_eq!(spaces[1].workspace_ids.len(), 1);
        app.state.assert_invariants_for_test();
    }

    #[test]
    fn errors_use_stable_codes() {
        let mut app = app_with_workspaces(vec![Workspace::test_new("a")]);
        let response = app.handle_space_rename(
            "rename".into(),
            SpaceRenameParams {
                space_id: crate::space::OTHER_SPACE_ID.into(),
                name: "x".into(),
            },
        );
        let error: ErrorResponse = serde_json::from_str(&response).expect("error response");
        assert_eq!(error.error.code, "space_is_built_in");
        let response = app.handle_space_assign(
            "assign".into(),
            SpaceAssignParams {
                workspace_id: "missing".into(),
                space_id: crate::space::OTHER_SPACE_ID.into(),
                before_workspace_id: None,
            },
        );
        let error: ErrorResponse = serde_json::from_str(&response).expect("error response");
        assert_eq!(error.error.code, "workspace_not_found");
    }

    #[tokio::test]
    async fn closed_members_reopen_in_their_space() {
        use super::super::test_support::{exiting_test_command, shutdown_test_runtimes};

        let mut app = app_with_workspaces(vec![Workspace::test_new("a"), Workspace::test_new("b")]);
        let a = app.public_workspace_id(0);
        let space = create(&mut app, "feature", Some(a));
        app.state.selected = 0;
        app.state.close_selected_workspace();
        let closed = app.space_info(0).closed;
        assert_eq!(closed.len(), 1);
        app.state.spaces[0].closed[0].cwd =
            std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("/"));
        app.state.default_shell = exiting_test_command().into();
        app.state.shell_mode = crate::config::ShellModeConfig::NonLogin;

        let ResponseResult::WorkspaceInfo { workspace } = success(&app.handle_space_member_open(
            "open".into(),
            SpaceMemberTarget {
                space_id: space.space_id.clone(),
                member_id: closed[0].member_id.clone(),
                focus: true,
            },
        )) else {
            panic!("expected workspace info");
        };
        assert_eq!(workspace.space_id.as_deref(), Some(space.space_id.as_str()));
        assert!(app.space_info(0).closed.is_empty());
        assert_eq!(
            app.space_info(0).workspace_ids,
            vec![workspace.workspace_id]
        );
        app.state.assert_invariants_for_test();
        shutdown_test_runtimes(&mut app);
    }
}
