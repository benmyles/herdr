//! Spaces: user-named containers that organize workspaces.
//!
//! A workspace belongs to exactly one space through `Workspace::space_id`.
//! `AppState::workspaces` stays grouped by space in `AppState::spaces` order,
//! so public workspace numbers, indexed jumps, and sidebar order all follow the
//! space layout without a second ordering to keep in sync.

use crate::space::{ClosedMember, Space, OTHER_SPACE_ID};

use super::state::AppState;

const MAX_SPACE_NAME_CHARS: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SpaceError {
    NotFound(String),
    WorkspaceNotFound(String),
    MemberNotFound(String),
    InvalidName(String),
    DuplicateName(String),
    BuiltIn,
}

impl SpaceError {
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::NotFound(_) => "space_not_found",
            Self::WorkspaceNotFound(_) => "workspace_not_found",
            Self::MemberNotFound(_) => "space_member_not_found",
            Self::InvalidName(_) => "invalid_space_name",
            Self::DuplicateName(_) => "duplicate_space_name",
            Self::BuiltIn => "space_is_built_in",
        }
    }
}

impl std::fmt::Display for SpaceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(id) => write!(f, "space {id} not found"),
            Self::WorkspaceNotFound(id) => write!(f, "workspace {id} not found"),
            Self::MemberNotFound(id) => write!(f, "space member {id} not found"),
            Self::InvalidName(reason) => write!(f, "invalid space name: {reason}"),
            Self::DuplicateName(name) => write!(f, "a space named {name:?} already exists"),
            Self::BuiltIn => write!(f, "the other space cannot be changed"),
        }
    }
}

impl AppState {
    pub(crate) fn space_index(&self, space_id: &str) -> Option<usize> {
        self.spaces.iter().position(|space| space.id == space_id)
    }

    pub(crate) fn space(&self, space_id: &str) -> Option<&Space> {
        self.spaces.iter().find(|space| space.id == space_id)
    }

    /// Keep spaces and workspace order consistent: `other` exists and is last,
    /// every workspace names an existing space, and workspaces are grouped by
    /// space in space order with their relative order preserved.
    pub(crate) fn normalize_spaces(&mut self) {
        match self.space_index(OTHER_SPACE_ID) {
            Some(index) if index + 1 == self.spaces.len() => {}
            Some(index) => {
                let other = self.spaces.remove(index);
                self.spaces.push(other);
            }
            None => self.spaces.push(Space::other(0)),
        }
        for workspace in &mut self.workspaces {
            if !self
                .spaces
                .iter()
                .any(|space| space.id == workspace.space_id)
            {
                workspace.space_id = OTHER_SPACE_ID.to_owned();
            }
        }
        let ranks = self
            .spaces
            .iter()
            .enumerate()
            .map(|(rank, space)| (space.id.clone(), rank))
            .collect::<std::collections::HashMap<_, _>>();
        let grouped = self
            .workspaces
            .windows(2)
            .all(|pair| ranks[&pair[0].space_id] <= ranks[&pair[1].space_id]);
        if grouped {
            return;
        }
        self.reorder_workspaces_preserving_focus(|workspaces| {
            workspaces.sort_by_key(|workspace| ranks[&workspace.space_id]);
        });
    }

    /// Apply a reorder of `workspaces` while keeping the active and selected
    /// workspaces pointed at the same logical workspace.
    fn reorder_workspaces_preserving_focus(
        &mut self,
        reorder: impl FnOnce(&mut Vec<crate::workspace::Workspace>),
    ) {
        let active_id = self
            .active
            .and_then(|idx| self.workspaces.get(idx))
            .map(|workspace| workspace.id.clone());
        let selected_id = self
            .workspaces
            .get(self.selected)
            .map(|workspace| workspace.id.clone());
        reorder(&mut self.workspaces);
        self.active = active_id.and_then(|id| self.workspaces.iter().position(|ws| ws.id == id));
        self.selected = selected_id
            .and_then(|id| self.workspaces.iter().position(|ws| ws.id == id))
            .unwrap_or(0);
        self.mark_session_dirty();
    }

    fn validated_space_name(
        &self,
        name: &str,
        renaming: Option<&str>,
    ) -> Result<String, SpaceError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(SpaceError::InvalidName("name is required".into()));
        }
        if name.chars().count() > MAX_SPACE_NAME_CHARS {
            return Err(SpaceError::InvalidName(format!(
                "names are at most {MAX_SPACE_NAME_CHARS} characters"
            )));
        }
        // Space names become checkout directories under the worktree root.
        if name
            .chars()
            .any(|ch| ch.is_control() || ch == '/' || ch == '\\')
            || name == "."
            || name == ".."
        {
            return Err(SpaceError::InvalidName(
                "names cannot contain slashes or control characters".into(),
            ));
        }
        if self.spaces.iter().any(|space| {
            Some(space.id.as_str()) != renaming && space.name.eq_ignore_ascii_case(name)
        }) {
            return Err(SpaceError::DuplicateName(name.to_owned()));
        }
        Ok(name.to_owned())
    }

    pub(crate) fn create_space(&mut self, name: &str) -> Result<String, SpaceError> {
        let name = self.validated_space_name(name, None)?;
        let user_spaces = self
            .spaces
            .iter()
            .filter(|space| !space.is_other())
            .cloned()
            .collect::<Vec<_>>();
        let space = Space::new(name, crate::space::next_color_slot(&user_spaces));
        let id = space.id.clone();
        let insert_at = self
            .space_index(OTHER_SPACE_ID)
            .unwrap_or(self.spaces.len());
        self.spaces.insert(insert_at, space);
        self.normalize_spaces();
        self.mark_session_dirty();
        Ok(id)
    }

    pub(crate) fn rename_space(&mut self, space_id: &str, name: &str) -> Result<(), SpaceError> {
        if space_id == OTHER_SPACE_ID {
            return Err(SpaceError::BuiltIn);
        }
        let index = self
            .space_index(space_id)
            .ok_or_else(|| SpaceError::NotFound(space_id.to_owned()))?;
        let name = self.validated_space_name(name, Some(space_id))?;
        self.spaces[index].name = name;
        self.mark_session_dirty();
        Ok(())
    }

    /// Delete a space. Its live workspaces move to `other`; nothing is closed
    /// and no files are touched. Closed members are forgotten.
    pub(crate) fn delete_space(&mut self, space_id: &str) -> Result<(), SpaceError> {
        if space_id == OTHER_SPACE_ID {
            return Err(SpaceError::BuiltIn);
        }
        let index = self
            .space_index(space_id)
            .ok_or_else(|| SpaceError::NotFound(space_id.to_owned()))?;
        self.spaces.remove(index);
        for workspace in &mut self.workspaces {
            if workspace.space_id == space_id {
                workspace.space_id = OTHER_SPACE_ID.to_owned();
            }
        }
        self.normalize_spaces();
        self.mark_session_dirty();
        Ok(())
    }

    /// Move a space before another, or to the end of the user spaces.
    /// `other` always stays last.
    pub(crate) fn move_space(
        &mut self,
        space_id: &str,
        before_space_id: Option<&str>,
    ) -> Result<bool, SpaceError> {
        if space_id == OTHER_SPACE_ID {
            return Err(SpaceError::BuiltIn);
        }
        let index = self
            .space_index(space_id)
            .ok_or_else(|| SpaceError::NotFound(space_id.to_owned()))?;
        if let Some(before) = before_space_id {
            if self.space_index(before).is_none() {
                return Err(SpaceError::NotFound(before.to_owned()));
            }
        }
        if before_space_id == Some(space_id) {
            return Ok(false);
        }
        let original = self
            .spaces
            .iter()
            .map(|space| space.id.clone())
            .collect::<Vec<_>>();
        let space = self.spaces.remove(index);
        let insert_at = before_space_id
            .and_then(|before| self.space_index(before))
            .unwrap_or_else(|| {
                self.space_index(OTHER_SPACE_ID)
                    .unwrap_or(self.spaces.len())
            });
        self.spaces.insert(insert_at, space);
        self.normalize_spaces();
        let changed = !self
            .spaces
            .iter()
            .map(|space| space.id.as_str())
            .eq(original.iter().map(String::as_str));
        if changed {
            self.mark_session_dirty();
        }
        Ok(changed)
    }

    /// File a workspace under a space, before another workspace of that space
    /// or at its end.
    pub(crate) fn assign_workspace_to_space(
        &mut self,
        workspace_id: &str,
        space_id: &str,
        before_workspace_id: Option<&str>,
    ) -> Result<bool, SpaceError> {
        if self.space_index(space_id).is_none() {
            return Err(SpaceError::NotFound(space_id.to_owned()));
        }
        let Some(index) = self
            .workspaces
            .iter()
            .position(|workspace| workspace.id == workspace_id)
        else {
            return Err(SpaceError::WorkspaceNotFound(workspace_id.to_owned()));
        };
        let before_workspace_id = before_workspace_id.filter(|before| *before != workspace_id);
        if let Some(before) = before_workspace_id {
            let Some(target) = self
                .workspaces
                .iter()
                .find(|workspace| workspace.id == before)
            else {
                return Err(SpaceError::WorkspaceNotFound(before.to_owned()));
            };
            if target.space_id != space_id {
                return Err(SpaceError::WorkspaceNotFound(before.to_owned()));
            }
        }

        let original = self
            .workspaces
            .iter()
            .map(|workspace| (workspace.id.clone(), workspace.space_id.clone()))
            .collect::<Vec<_>>();
        let space_id = space_id.to_owned();
        let before_workspace_id = before_workspace_id.map(str::to_owned);
        self.reorder_workspaces_preserving_focus(|workspaces| {
            let mut workspace = workspaces.remove(index);
            workspace.space_id = space_id.clone();
            let insert_at = before_workspace_id
                .and_then(|before| workspaces.iter().position(|ws| ws.id == before))
                .or_else(|| {
                    workspaces
                        .iter()
                        .rposition(|ws| ws.space_id == space_id)
                        .map(|last| last + 1)
                })
                .unwrap_or(workspaces.len());
            workspaces.insert(insert_at, workspace);
        });
        self.normalize_spaces();
        Ok(!self
            .workspaces
            .iter()
            .map(|workspace| (workspace.id.as_str(), workspace.space_id.as_str()))
            .eq(original
                .iter()
                .map(|(id, space)| (id.as_str(), space.as_str()))))
    }

    /// Remember a workspace that is about to close so its space can reopen
    /// it. Only user spaces keep closed members; `other` lets them go.
    pub(crate) fn retain_closed_member(&mut self, ws_idx: usize) {
        let Some(workspace) = self.workspaces.get(ws_idx) else {
            return;
        };
        if workspace.space_id == OTHER_SPACE_ID {
            return;
        }
        let Some(space_index) = self.space_index(&workspace.space_id) else {
            return;
        };
        let cwd = workspace
            .worktree_space()
            .map(|space| space.checkout_path.clone())
            .unwrap_or_else(|| workspace.identity_cwd.clone());
        let label = workspace
            .worktree_space()
            .map(|space| space.label.clone())
            .unwrap_or_else(|| workspace.display_name_from_terminals(&self.terminals));
        let member = ClosedMember::new(
            label,
            cwd,
            workspace.custom_name.clone(),
            workspace.branch(),
            workspace.worktree_space.clone(),
        );
        let closed = &mut self.spaces[space_index].closed;
        closed.retain(|existing| existing.cwd != member.cwd);
        closed.push(member);
        self.mark_session_dirty();
    }

    /// Take a closed member out of its space so it can be reopened.
    pub(crate) fn take_closed_member(
        &mut self,
        space_id: &str,
        member_id: &str,
    ) -> Result<ClosedMember, SpaceError> {
        let space_index = self
            .space_index(space_id)
            .ok_or_else(|| SpaceError::NotFound(space_id.to_owned()))?;
        let member_index = self.spaces[space_index]
            .closed
            .iter()
            .position(|member| member.id == member_id)
            .ok_or_else(|| SpaceError::MemberNotFound(member_id.to_owned()))?;
        self.mark_session_dirty();
        Ok(self.spaces[space_index].closed.remove(member_index))
    }

    /// Put a member back after a failed reopen.
    pub(crate) fn restore_closed_member(&mut self, space_id: &str, member: ClosedMember) {
        if let Some(space_index) = self.space_index(space_id) {
            self.spaces[space_index].closed.push(member);
            self.mark_session_dirty();
        }
    }

    /// Whether the session has anything worth keeping without a workspace.
    pub(crate) fn has_space_content(&self) -> bool {
        self.spaces
            .iter()
            .any(|space| !space.is_other() || !space.closed.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::Workspace;

    fn state_with(workspaces: &[(&str, &str)]) -> AppState {
        let mut state = AppState::test_new();
        state.workspaces = workspaces
            .iter()
            .map(|(id, space)| {
                let mut workspace = Workspace::test_new(id);
                workspace.id = (*id).to_owned();
                workspace.space_id = (*space).to_owned();
                workspace
            })
            .collect();
        state.active = (!state.workspaces.is_empty()).then_some(0);
        state
    }

    fn order(state: &AppState) -> Vec<(&str, &str)> {
        state
            .workspaces
            .iter()
            .map(|workspace| (workspace.id.as_str(), workspace.space_id.as_str()))
            .collect()
    }

    #[test]
    fn normalize_adds_other_last_and_files_unknown_spaces_there() {
        let mut state = state_with(&[("w1", "gone"), ("w2", OTHER_SPACE_ID)]);
        state.normalize_spaces();
        assert_eq!(state.spaces.len(), 1);
        assert!(state.spaces[0].is_other());
        assert_eq!(order(&state), vec![("w1", "other"), ("w2", "other")]);
    }

    #[test]
    fn workspaces_follow_space_order_and_keep_focus() {
        let mut state = state_with(&[("w1", OTHER_SPACE_ID), ("w2", OTHER_SPACE_ID)]);
        state.normalize_spaces();
        state.active = Some(1);
        state.selected = 1;
        let knowledge = state.create_space("knowledge").unwrap();
        assert!(state
            .assign_workspace_to_space("w2", &knowledge, None)
            .unwrap());
        assert_eq!(
            order(&state),
            vec![("w2", knowledge.as_str()), ("w1", "other")]
        );
        assert_eq!(state.active, Some(0));
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn assign_inserts_before_a_member_or_at_the_end_of_the_space() {
        let mut state = state_with(&[
            ("w1", OTHER_SPACE_ID),
            ("w2", OTHER_SPACE_ID),
            ("w3", OTHER_SPACE_ID),
        ]);
        state.normalize_spaces();
        let space = state.create_space("feature").unwrap();
        state.assign_workspace_to_space("w1", &space, None).unwrap();
        state.assign_workspace_to_space("w3", &space, None).unwrap();
        state
            .assign_workspace_to_space("w2", &space, Some("w3"))
            .unwrap();
        assert_eq!(
            order(&state),
            vec![
                ("w1", space.as_str()),
                ("w2", space.as_str()),
                ("w3", space.as_str())
            ]
        );
        assert!(!state
            .assign_workspace_to_space("w2", &space, Some("w3"))
            .unwrap());
        assert_eq!(
            state.assign_workspace_to_space("w2", "nope", None),
            Err(SpaceError::NotFound("nope".into()))
        );
    }

    #[test]
    fn names_are_validated_and_unique() {
        let mut state = state_with(&[]);
        state.normalize_spaces();
        state.create_space("Knowledge").unwrap();
        assert_eq!(
            state.create_space(" knowledge "),
            Err(SpaceError::DuplicateName("knowledge".into()))
        );
        assert_eq!(
            state.create_space("Other"),
            Err(SpaceError::DuplicateName("Other".into()))
        );
        assert!(matches!(
            state.create_space("a/b"),
            Err(SpaceError::InvalidName(_))
        ));
        assert!(matches!(
            state.create_space("  "),
            Err(SpaceError::InvalidName(_))
        ));
        assert_eq!(
            state.rename_space(OTHER_SPACE_ID, "x"),
            Err(SpaceError::BuiltIn)
        );
    }

    #[test]
    fn spaces_get_distinct_colors_and_other_stays_last() {
        let mut state = state_with(&[]);
        state.normalize_spaces();
        let a = state.create_space("a").unwrap();
        let b = state.create_space("b").unwrap();
        let c = state.create_space("c").unwrap();
        let colors = state
            .spaces
            .iter()
            .filter(|space| !space.is_other())
            .map(|space| space.color)
            .collect::<Vec<_>>();
        assert_eq!(colors, vec![0, 1, 2]);
        assert!(state.move_space(&c, Some(&a)).unwrap());
        assert!(state.move_space(&a, None).unwrap());
        let ids = state
            .spaces
            .iter()
            .map(|space| space.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            ids,
            vec![c.as_str(), b.as_str(), a.as_str(), OTHER_SPACE_ID]
        );
        assert_eq!(
            state.move_space(OTHER_SPACE_ID, None),
            Err(SpaceError::BuiltIn)
        );
        state.delete_space(&b).unwrap();
        assert_eq!(
            state
                .create_space("d")
                .map(|id| state.space(&id).unwrap().color),
            Ok(1)
        );
    }

    #[test]
    fn deleting_a_space_moves_live_members_to_other() {
        let mut state = state_with(&[("w1", OTHER_SPACE_ID), ("w2", OTHER_SPACE_ID)]);
        state.normalize_spaces();
        let space = state.create_space("feature").unwrap();
        state.assign_workspace_to_space("w2", &space, None).unwrap();
        state.delete_space(&space).unwrap();
        assert_eq!(order(&state), vec![("w2", "other"), ("w1", "other")]);
        assert_eq!(state.spaces.len(), 1);
        assert_eq!(state.delete_space(OTHER_SPACE_ID), Err(SpaceError::BuiltIn));
    }

    #[test]
    fn closed_members_are_kept_in_user_spaces_only() {
        let mut state = state_with(&[("w1", OTHER_SPACE_ID), ("w2", OTHER_SPACE_ID)]);
        state.normalize_spaces();
        let space = state.create_space("feature").unwrap();
        state.assign_workspace_to_space("w1", &space, None).unwrap();
        state.retain_closed_member(0);
        state.retain_closed_member(1);
        assert_eq!(state.space(&space).unwrap().closed.len(), 1);
        assert!(state.space(OTHER_SPACE_ID).unwrap().closed.is_empty());
        assert!(state.has_space_content());

        let member_id = state.space(&space).unwrap().closed[0].id.clone();
        let member = state.take_closed_member(&space, &member_id).unwrap();
        assert!(state.space(&space).unwrap().closed.is_empty());
        assert_eq!(
            state.take_closed_member(&space, &member_id),
            Err(SpaceError::MemberNotFound(member_id.clone()))
        );
        state.restore_closed_member(&space, member);
        assert_eq!(state.space(&space).unwrap().closed.len(), 1);
    }
}
