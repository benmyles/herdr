use std::collections::HashMap;

use ratatui::style::Color;

use crate::app::state::Palette;
use crate::app::AppState;

#[derive(Debug)]
struct SpaceGroup {
    members: Vec<usize>,
    parent: Option<usize>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum SpaceKey<'a> {
    Worktree(&'a str),
    Workspace(&'a str),
}

/// Transient presentation metadata shared by the grid and both sidebar modes.
///
/// A normal workspace is its own space. Workspaces carrying the same worktree
/// space key share a color and are emitted together, with the primary checkout
/// before its linked worktrees. Nothing here is persisted or exposed through
/// the runtime protocol.
pub(super) struct SpacePresentation {
    workspace_order: Vec<usize>,
    workspace_colors: Vec<Color>,
    pin_colors: Vec<Color>,
}

impl SpacePresentation {
    pub(super) fn new(app: &AppState) -> Self {
        let mut group_indices = HashMap::<SpaceKey<'_>, usize>::new();
        let mut groups = Vec::<SpaceGroup>::new();

        for (ws_idx, workspace) in app.workspaces.iter().enumerate() {
            let (key, is_parent) = workspace
                .worktree_space()
                .map(|space| {
                    (
                        SpaceKey::Worktree(space.key.as_str()),
                        !space.is_linked_worktree,
                    )
                })
                .unwrap_or_else(|| (SpaceKey::Workspace(workspace.id.as_str()), true));
            let group_idx = *group_indices.entry(key).or_insert_with(|| {
                groups.push(SpaceGroup {
                    members: Vec::new(),
                    parent: None,
                });
                groups.len() - 1
            });
            let group = &mut groups[group_idx];
            group.members.push(ws_idx);
            if is_parent && group.parent.is_none() {
                group.parent = Some(ws_idx);
            }
        }

        let mut workspace_order = Vec::with_capacity(app.workspaces.len());
        let mut workspace_colors = vec![app.palette.accent; app.workspaces.len()];
        for (space_idx, group) in groups.iter().enumerate() {
            let pinned_order = group.members.iter().find_map(|ws_idx| {
                let workspace = &app.workspaces[*ws_idx];
                app.pinned_spaces
                    .iter()
                    .find(|pin| pin.matches_workspace(workspace))
                    .map(|pin| pin.order)
            });
            let color = palette_space_color(&app.palette, pinned_order.unwrap_or(space_idx));
            if let Some(parent) = group.parent {
                workspace_order.push(parent);
            }
            for &ws_idx in &group.members {
                workspace_colors[ws_idx] = color;
                if Some(ws_idx) != group.parent {
                    workspace_order.push(ws_idx);
                }
            }
        }

        let pin_colors = app
            .pinned_spaces
            .iter()
            .map(|pin| palette_space_color(&app.palette, pin.order))
            .collect();
        Self {
            workspace_order,
            workspace_colors,
            pin_colors,
        }
    }

    pub(super) fn workspace_order(&self) -> &[usize] {
        &self.workspace_order
    }

    pub(super) fn color(&self, ws_idx: usize) -> Color {
        self.workspace_colors
            .get(ws_idx)
            .copied()
            .unwrap_or(Color::Reset)
    }

    pub(super) fn pin_color(&self, pin_idx: usize) -> Color {
        self.pin_colors
            .get(pin_idx)
            .copied()
            .unwrap_or(Color::Reset)
    }
}

fn palette_space_color(palette: &Palette, index: usize) -> Color {
    let colors = [
        palette.blue,
        palette.mauve,
        palette.teal,
        palette.peach,
        palette.green,
        palette.yellow,
        palette.red,
    ];
    color_variant(colors[index % colors.len()], index / colors.len())
}

fn color_variant(color: Color, round: usize) -> Color {
    if round == 0 {
        return color;
    }
    let Color::Rgb(red, green, blue) = color else {
        return color;
    };

    let step = round.div_ceil(2).saturating_mul(28).min(196) as u16;
    let adjust = |channel: u8| {
        let channel = u16::from(channel);
        if round % 2 == 1 {
            (channel + (255 - channel) * step / 255).min(255) as u8
        } else {
            (channel * (255 - step) / 255) as u8
        }
    };
    Color::Rgb(adjust(red), adjust(green), adjust(blue))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::{Workspace, WorktreeSpaceMembership};

    fn member(name: &str, key: &str, linked: bool) -> Workspace {
        let mut workspace = Workspace::test_new(name);
        workspace.worktree_space = Some(WorktreeSpaceMembership {
            key: key.into(),
            label: key.into(),
            repo_root: format!("/repo/{key}").into(),
            checkout_path: format!("/repo/{name}").into(),
            is_linked_worktree: linked,
        });
        workspace
    }

    #[test]
    fn worktree_space_members_are_adjacent_and_parent_first() {
        let mut app = AppState::test_new();
        app.workspaces = vec![
            member("issue", "repo", true),
            Workspace::test_new("notes"),
            member("main", "repo", false),
            member("review", "repo", true),
        ];

        let spaces = SpacePresentation::new(&app);

        assert_eq!(spaces.workspace_order(), &[2, 0, 3, 1]);
        assert_eq!(spaces.color(0), spaces.color(2));
        assert_eq!(spaces.color(2), spaces.color(3));
        assert_ne!(spaces.color(1), spaces.color(2));
    }

    #[test]
    fn ordinary_workspaces_receive_distinct_theme_colors() {
        let mut app = AppState::test_new();
        app.workspaces = (0..50)
            .map(|idx| Workspace::test_new(&format!("space-{idx}")))
            .collect();

        let spaces = SpacePresentation::new(&app);
        let colors = (0..app.workspaces.len())
            .map(|ws_idx| spaces.color(ws_idx))
            .collect::<std::collections::HashSet<_>>();

        assert_eq!(colors.len(), app.workspaces.len());
    }
}
