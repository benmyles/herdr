use ratatui::style::Color;

use crate::app::state::Palette;

/// One workspace, in session order, as seen by space presentation.
pub(crate) struct SpaceWorkspace {
    /// The space's color slot, or `None` for the neutral `other` space.
    /// Servers without spaces give every workspace its own slot.
    pub(crate) color_slot: Option<usize>,
}

/// Transient presentation metadata shared by both sidebar modes and the live
/// agent grid. The client and the server derive the same colors from the
/// same space color slots.
pub(crate) struct SpacePresentation {
    workspace_colors: Vec<Color>,
}

impl SpacePresentation {
    pub(crate) fn new(
        palette: &Palette,
        workspaces: impl IntoIterator<Item = SpaceWorkspace>,
    ) -> Self {
        let workspace_colors = workspaces
            .into_iter()
            .map(|workspace| space_slot_color(palette, workspace.color_slot))
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

    fn workspace(_space_id: &str, color_slot: Option<usize>) -> SpaceWorkspace {
        SpaceWorkspace { color_slot }
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
