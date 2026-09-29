//! Dragging workspaces between spaces and reordering spaces.

use super::*;

impl ClientShellState {
    /// Whether the active endpoint files workspaces into spaces.
    fn dragging_into_spaces(&self) -> bool {
        self.snapshot
            .as_deref()
            .is_some_and(|snapshot| !snapshot.spaces.is_empty())
            && self.active_endpoint_supports_spaces()
    }

    fn workspace_space_id(&self, workspace_id: &str) -> Option<&str> {
        self.snapshot
            .as_deref()?
            .workspaces
            .iter()
            .find(|workspace| workspace.workspace_id == workspace_id)?
            .space_id
            .as_deref()
    }

    /// First row below a space's block: header, members, closed members and
    /// the "+ worktree" row.
    fn space_block_bottom(&self, header: &SpaceHeaderHit) -> u16 {
        let endpoint = &self.active_endpoint_id;
        let members = self
            .hits
            .workspaces
            .iter()
            .filter(|hit| &hit.endpoint_id == endpoint)
            .filter(|hit| self.workspace_space_id(&hit.workspace_id) == Some(&header.space_id))
            .map(|hit| hit.rect.bottom());
        let closed = self
            .hits
            .closed_members
            .iter()
            .filter(|hit| &hit.endpoint_id == endpoint && hit.space_id == header.space_id)
            .map(|hit| hit.rect.bottom());
        let add = self
            .hits
            .add_worktree
            .iter()
            .filter(|hit| &hit.endpoint_id == endpoint && hit.space_id == header.space_id)
            .map(|hit| hit.rect.bottom());
        members
            .chain(closed)
            .chain(add)
            .fold(header.rect.bottom(), u16::max)
    }

    fn active_space_headers(&self) -> Vec<SpaceHeaderHit> {
        self.hits
            .space_headers
            .iter()
            .filter(|hit| hit.endpoint_id == self.active_endpoint_id)
            .cloned()
            .collect()
    }

    fn in_workspace_list(&self, point: (u16, u16)) -> bool {
        self.hits.workspace_body.height > 0
            && point.1 >= self.hits.workspace_body.y.saturating_sub(1)
            && point.1 < self.hits.new_workspace.y
    }

    pub(super) fn workspace_drop_target_at(
        &self,
        point: (u16, u16),
    ) -> Option<WorkspaceDropTarget> {
        if !self.dragging_into_spaces() {
            return self.legacy_workspace_drop_target_at(point).map(
                |(before_workspace_id, row)| WorkspaceDropTarget {
                    space_id: None,
                    before_workspace_id,
                    row,
                },
            );
        }
        if !self.in_workspace_list(point) {
            return None;
        }
        let headers = self.active_space_headers();
        // Pointing at a header files the workspace at the end of that space.
        if let Some(header) = headers
            .iter()
            .find(|header| super::contains(header.rect, point))
        {
            return Some(WorkspaceDropTarget {
                space_id: Some(header.space_id.clone()),
                before_workspace_id: None,
                row: self.space_block_bottom(header),
            });
        }
        let mut slots = Vec::new();
        for header in &headers {
            for hit in self.hits.workspaces.iter().filter(|hit| {
                hit.endpoint_id == self.active_endpoint_id
                    && self.workspace_space_id(&hit.workspace_id) == Some(&header.space_id)
            }) {
                slots.push(WorkspaceDropTarget {
                    space_id: Some(header.space_id.clone()),
                    before_workspace_id: Some(hit.workspace_id.clone()),
                    row: hit.rect.y.saturating_sub(1),
                });
            }
            slots.push(WorkspaceDropTarget {
                space_id: Some(header.space_id.clone()),
                before_workspace_id: None,
                row: self.space_block_bottom(header),
            });
        }
        slots
            .into_iter()
            .enumerate()
            .min_by_key(|(index, slot)| (point.1.abs_diff(slot.row), *index))
            .map(|(_, slot)| slot)
    }

    /// `space.assign` for a drop, or `None` when it would change nothing.
    pub(super) fn space_assign_method(
        &self,
        source_workspace_id: &str,
        space_id: &str,
        before_workspace_id: Option<&str>,
    ) -> Option<crate::api::schema::Method> {
        let snapshot = self.snapshot.as_deref()?;
        let members = snapshot
            .workspaces
            .iter()
            .filter(|workspace| workspace.space_id.as_deref() == Some(space_id))
            .map(|workspace| workspace.workspace_id.as_str())
            .collect::<Vec<_>>();
        if let Some(position) = members.iter().position(|id| *id == source_workspace_id) {
            let unchanged = match before_workspace_id {
                Some(before) => {
                    before == source_workspace_id
                        || members.get(position + 1).copied() == Some(before)
                }
                None => position + 1 == members.len(),
            };
            if unchanged {
                return None;
            }
        }
        Some(crate::api::schema::Method::SpaceAssign(
            crate::api::schema::SpaceAssignParams {
                workspace_id: source_workspace_id.to_owned(),
                space_id: space_id.to_owned(),
                before_workspace_id: before_workspace_id.map(str::to_owned),
            },
        ))
    }

    pub(super) fn workspace_drop_method(
        &self,
        source_workspace_id: &str,
        target: &WorkspaceDropTarget,
    ) -> Option<crate::api::schema::Method> {
        match target.space_id.as_deref() {
            Some(space_id) => self.space_assign_method(
                source_workspace_id,
                space_id,
                target.before_workspace_id.as_deref(),
            ),
            None => self
                .workspace_move_method(source_workspace_id, target.before_workspace_id.as_deref()),
        }
    }

    pub(super) fn endpoint_space_is_draggable(&self, press: &ClientSpacePress) -> bool {
        press.endpoint_id == self.active_endpoint_id
            && self.active_endpoint_supports_spaces()
            && self.snapshot.as_deref().is_some_and(|snapshot| {
                snapshot
                    .spaces
                    .iter()
                    .any(|space| space.space_id == press.space_id && !space.built_in)
            })
    }

    /// Slot before a user space header, or after the last one (`None`).
    pub(super) fn space_drop_target_at(&self, point: (u16, u16)) -> Option<(Option<String>, u16)> {
        if !self.in_workspace_list(point) {
            return None;
        }
        let snapshot = self.snapshot.as_deref()?;
        let built_in = |space_id: &str| {
            snapshot
                .spaces
                .iter()
                .any(|space| space.space_id == space_id && space.built_in)
        };
        let headers = self.active_space_headers();
        let user_headers = headers
            .iter()
            .filter(|header| !built_in(&header.space_id))
            .collect::<Vec<_>>();
        let last = user_headers.last()?;
        let mut slots = user_headers
            .iter()
            .map(|header| {
                (
                    Some(header.space_id.clone()),
                    header.rect.y.saturating_sub(1),
                )
            })
            .collect::<Vec<_>>();
        slots.push((None, self.space_block_bottom(last)));
        slots
            .into_iter()
            .enumerate()
            .min_by_key(|(index, (_, row))| (point.1.abs_diff(*row), *index))
            .map(|(_, slot)| slot)
    }

    /// `space.move` for a header drop, or `None` when nothing would move.
    pub(super) fn space_move_method(
        &self,
        space_id: &str,
        before_space_id: Option<&str>,
    ) -> Option<crate::api::schema::Method> {
        let snapshot = self.snapshot.as_deref()?;
        let order = snapshot
            .spaces
            .iter()
            .filter(|space| !space.built_in)
            .map(|space| space.space_id.as_str())
            .collect::<Vec<_>>();
        let position = order.iter().position(|id| *id == space_id)?;
        let unchanged = match before_space_id {
            Some(before) => before == space_id || order.get(position + 1).copied() == Some(before),
            None => position + 1 == order.len(),
        };
        (!unchanged).then(|| {
            crate::api::schema::Method::SpaceMove(crate::api::schema::SpaceMoveParams {
                space_id: space_id.to_owned(),
                before_space_id: before_space_id.map(str::to_owned),
            })
        })
    }
}
