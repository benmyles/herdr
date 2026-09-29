use std::collections::HashMap;

use ratatui::style::Color;

use crate::app::state::Palette;

/// One workspace, in session order, as seen by space presentation.
pub(crate) struct SpaceWorkspace<'a> {
    /// Space the workspace is filed under. Servers without spaces pass the
    /// workspace's own id, so every workspace is its own space.
    pub(crate) space_id: &'a str,
    /// The space's color slot, or `None` for the neutral `other` space.
    pub(crate) color_slot: Option<usize>,
}

/// Space grouping for a session's workspaces, independent of any theme.
/// Members of one space are emitted together in first-seen space order.
pub(crate) struct SpaceLayout {
    workspace_order: Vec<usize>,
    workspace_slots: Vec<Option<usize>>,
}

impl SpaceLayout {
    pub(crate) fn new<'a>(workspaces: impl IntoIterator<Item = SpaceWorkspace<'a>>) -> Self {
        let mut group_indices = HashMap::<&'a str, usize>::new();
        let mut groups = Vec::<Vec<usize>>::new();
        let mut workspace_slots = Vec::new();

        for (index, workspace) in workspaces.into_iter().enumerate() {
            let group_index = *group_indices.entry(workspace.space_id).or_insert_with(|| {
                groups.push(Vec::new());
                groups.len() - 1
            });
            groups[group_index].push(index);
            workspace_slots.push(workspace.color_slot);
        }

        Self {
            workspace_order: groups.into_iter().flatten().collect(),
            workspace_slots,
        }
    }

    /// Workspace indices with the members of each space adjacent.
    pub(crate) fn workspace_order(&self) -> &[usize] {
        &self.workspace_order
    }
}

/// Transient presentation metadata shared by both sidebar modes and the live
/// agent grid. The client and the server derive the same colors from the
/// same space color slots.
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
            .map(|slot| space_slot_color(palette, *slot))
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

/// Color for a space slot; `None` is the neutral `other` space.
pub(crate) fn space_slot_color(palette: &Palette, slot: Option<usize>) -> Color {
    slot.map_or(palette.overlay1, |slot| space_color(palette, slot))
}

/// Theme color for the space slot `index`, cycling through lighter and darker
/// variants once the base accents are exhausted so nearby spaces stay distinct.
pub(crate) fn space_color(palette: &Palette, index: usize) -> Color {
    // Green and red are left out: agent marks use them for done and waiting.
    let colors = [
        palette.blue,
        palette.mauve,
        palette.teal,
        palette.peach,
        palette.yellow,
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

    fn workspace(space_id: &str, color_slot: Option<usize>) -> SpaceWorkspace<'_> {
        SpaceWorkspace {
            space_id,
            color_slot,
        }
    }

    #[test]
    fn space_members_share_their_space_color_and_other_is_neutral() {
        let palette = Palette::catppuccin();
        let spaces = SpacePresentation::new(
            &palette,
            [
                workspace("knowledge", Some(3)),
                workspace("knowledge", Some(3)),
                workspace("billing", Some(0)),
                workspace("other", None),
            ],
        );

        assert_eq!(spaces.color(0), space_color(&palette, 3));
        assert_eq!(spaces.color(1), spaces.color(0));
        assert_eq!(spaces.color(2), space_color(&palette, 0));
        assert_eq!(spaces.color(3), palette.overlay1);
    }

    #[test]
    fn layout_keeps_space_members_adjacent_in_first_seen_order() {
        let layout = SpaceLayout::new([
            workspace("a", Some(0)),
            workspace("b", Some(1)),
            workspace("a", Some(0)),
            workspace("other", None),
        ]);

        assert_eq!(layout.workspace_order(), &[0, 2, 1, 3]);
    }

    #[test]
    fn space_slots_receive_distinct_theme_colors_without_red_or_green() {
        let palette = Palette::catppuccin();
        let colors = (0..50)
            .map(|slot| space_color(&palette, slot))
            .collect::<std::collections::HashSet<_>>();

        assert_eq!(colors.len(), 50);
        assert!(!colors.contains(&palette.red));
        assert!(!colors.contains(&palette.green));
    }

    #[test]
    fn muting_blends_rgb_toward_the_background() {
        let muted = mute_color(Color::Rgb(200, 100, 0), Color::Rgb(0, 0, 100), 25);
        assert_eq!(muted, Color::Rgb(50, 25, 75));
        assert_eq!(
            mute_color(Color::Indexed(4), Color::Rgb(0, 0, 100), 25),
            Color::Indexed(4)
        );
    }
}
