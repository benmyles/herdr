use std::collections::HashMap;

use ratatui::style::Color;

use crate::app::state::Palette;

/// Logical space identity for one workspace. Workspaces carrying the same
/// worktree key form one space; every other workspace is its own space.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum SpaceKey<'a> {
    Worktree(&'a str),
    Workspace(&'a str),
}

/// One workspace, in session order, as seen by space presentation.
pub(crate) struct SpaceWorkspace<'a> {
    pub(crate) key: SpaceKey<'a>,
    /// Whether this workspace is the primary checkout of its space.
    pub(crate) is_parent: bool,
    /// Saved order of the pin covering this workspace, if any. A pinned space
    /// keeps the same color while it is live and after it becomes dormant.
    pub(crate) pinned_order: Option<usize>,
}

struct SpaceGroup {
    members: Vec<usize>,
    parent: Option<usize>,
    pinned_order: Option<usize>,
}

/// Space grouping for a session's workspaces, independent of any theme.
///
/// Members of one space are emitted together, primary checkout first. Each
/// workspace gets its space's color slot: the saved pin order when the space
/// is pinned, otherwise the space's position in session order.
pub(crate) struct SpaceLayout {
    workspace_order: Vec<usize>,
    workspace_slots: Vec<usize>,
}

impl SpaceLayout {
    pub(crate) fn new<'a>(workspaces: impl IntoIterator<Item = SpaceWorkspace<'a>>) -> Self {
        let mut group_indices = HashMap::<SpaceKey<'a>, usize>::new();
        let mut groups = Vec::<SpaceGroup>::new();
        let mut workspace_count = 0;

        for (index, workspace) in workspaces.into_iter().enumerate() {
            workspace_count = index + 1;
            let group_index = *group_indices.entry(workspace.key).or_insert_with(|| {
                groups.push(SpaceGroup {
                    members: Vec::new(),
                    parent: None,
                    pinned_order: None,
                });
                groups.len() - 1
            });
            let group = &mut groups[group_index];
            group.members.push(index);
            if workspace.is_parent && group.parent.is_none() {
                group.parent = Some(index);
            }
            if group.pinned_order.is_none() {
                group.pinned_order = workspace.pinned_order;
            }
        }

        let mut workspace_order = Vec::with_capacity(workspace_count);
        let mut workspace_slots = vec![0; workspace_count];
        for (space_index, group) in groups.iter().enumerate() {
            let slot = group.pinned_order.unwrap_or(space_index);
            if let Some(parent) = group.parent {
                workspace_order.push(parent);
            }
            for &index in &group.members {
                workspace_slots[index] = slot;
                if Some(index) != group.parent {
                    workspace_order.push(index);
                }
            }
        }

        Self {
            workspace_order,
            workspace_slots,
        }
    }

    /// Workspace indices with the members of each space adjacent.
    pub(crate) fn workspace_order(&self) -> &[usize] {
        &self.workspace_order
    }
}

/// Transient presentation metadata shared by both sidebar modes and the live
/// agent grid. Nothing here is persisted or exposed through the runtime
/// protocol; the client and the server derive the same colors from the same
/// session order.
pub(crate) struct SpacePresentation {
    workspace_colors: Vec<Color>,
}

impl SpacePresentation {
    pub(crate) fn new<'a>(
        palette: &Palette,
        workspaces: impl IntoIterator<Item = SpaceWorkspace<'a>>,
    ) -> Self {
        let workspace_colors = SpaceLayout::new(workspaces)
            .workspace_slots
            .iter()
            .map(|&slot| space_color(palette, slot))
            .collect();
        Self { workspace_colors }
    }

    pub(crate) fn color(&self, workspace_index: usize) -> Color {
        self.workspace_colors
            .get(workspace_index)
            .copied()
            .unwrap_or(Color::Reset)
    }
}

/// Theme color for the space at `index`, cycling through lighter and darker
/// variants once the base accents are exhausted so nearby spaces stay distinct.
pub(crate) fn space_color(palette: &Palette, index: usize) -> Color {
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

/// Blend `color` toward `background`, keeping `percent` of the original.
///
/// Muting by blending keeps secondary space labels readable: terminal faint
/// stacked on an already muted color becomes illegible on many themes.
/// Non-RGB colors cannot be blended and are returned unchanged.
pub(crate) fn mute_color(color: Color, background: Color, percent: u8) -> Color {
    let (Color::Rgb(red, green, blue), Color::Rgb(bg_red, bg_green, bg_blue)) = (color, background)
    else {
        return color;
    };
    let percent = u16::from(percent.min(100));
    let blend = |channel: u8, backdrop: u8| {
        ((u16::from(channel) * percent + u16::from(backdrop) * (100 - percent)) / 100) as u8
    };
    Color::Rgb(
        blend(red, bg_red),
        blend(green, bg_green),
        blend(blue, bg_blue),
    )
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

    fn workspace(key: SpaceKey<'_>, is_parent: bool) -> SpaceWorkspace<'_> {
        SpaceWorkspace {
            key,
            is_parent,
            pinned_order: None,
        }
    }

    #[test]
    fn worktree_space_members_are_adjacent_and_parent_first() {
        let palette = Palette::catppuccin();
        let spaces = SpacePresentation::new(
            &palette,
            [
                workspace(SpaceKey::Worktree("repo"), false),
                workspace(SpaceKey::Workspace("notes"), true),
                workspace(SpaceKey::Worktree("repo"), true),
                workspace(SpaceKey::Worktree("repo"), false),
            ],
        );

        assert_eq!(spaces.color(0), spaces.color(2));
        assert_eq!(spaces.color(2), spaces.color(3));
        assert_ne!(spaces.color(1), spaces.color(2));
    }

    #[test]
    fn layout_orders_worktree_members_parent_first() {
        let layout = SpaceLayout::new([
            workspace(SpaceKey::Worktree("repo"), false),
            workspace(SpaceKey::Workspace("notes"), true),
            workspace(SpaceKey::Worktree("repo"), true),
            workspace(SpaceKey::Worktree("repo"), false),
        ]);

        assert_eq!(layout.workspace_order(), &[2, 0, 3, 1]);
    }

    #[test]
    fn ordinary_workspaces_receive_distinct_theme_colors() {
        let palette = Palette::catppuccin();
        let ids = (0..50)
            .map(|index| format!("w_{index}"))
            .collect::<Vec<_>>();
        let spaces = SpacePresentation::new(
            &palette,
            ids.iter()
                .map(|id| workspace(SpaceKey::Workspace(id.as_str()), true)),
        );
        let colors = (0..ids.len())
            .map(|index| spaces.color(index))
            .collect::<std::collections::HashSet<_>>();

        assert_eq!(colors.len(), ids.len());
    }

    #[test]
    fn muting_blends_rgb_toward_the_background() {
        let muted = mute_color(Color::Rgb(200, 100, 0), Color::Rgb(0, 0, 100), 25);
        assert_eq!(muted, Color::Rgb(50, 25, 75));
        assert_eq!(
            mute_color(Color::Indexed(4), Color::Rgb(0, 0, 0), 25),
            Color::Indexed(4)
        );
    }

    #[test]
    fn pinned_space_keeps_its_saved_color_slot() {
        let palette = Palette::catppuccin();
        let spaces = SpacePresentation::new(
            &palette,
            [
                workspace(SpaceKey::Workspace("first"), true),
                SpaceWorkspace {
                    key: SpaceKey::Workspace("pinned"),
                    is_parent: true,
                    pinned_order: Some(4),
                },
            ],
        );

        assert_eq!(spaces.color(1), space_color(&palette, 4));
        assert_eq!(spaces.color(0), space_color(&palette, 0));
    }
}
