use super::*;

use crate::client::shell::command_palette::{ClientCommandPaletteOverlay, PaletteMatch};

/// Width of the right-aligned kind column; "workspace" is the widest kind.
const KIND_WIDTH: u16 = 9;

pub(crate) struct RenderedCommandPalette {
    pub(crate) area: Rect,
    /// Each drawn result row and its index in the ranked results.
    pub(crate) rows: Vec<(Rect, usize)>,
    pub(crate) cursor: Option<crate::protocol::CursorState>,
}

/// Draws the palette in the upper third of the screen: the query, then the
/// ranked results with matched title characters highlighted, each row led by
/// the letter that picks it and naming where it leads, its keybinding, and
/// its kind.
pub(crate) fn render_command_palette(
    b: &mut Buffer,
    overlay: &ClientCommandPaletteOverlay,
    results: &[PaletteMatch<'_>],
    total: usize,
    p: &Palette,
) -> Option<RenderedCommandPalette> {
    use ratatui::text::{Line, Span};

    let a = b.area;
    let width = a.width.saturating_sub(4).min(100);
    let height = a.height.saturating_sub(2).min(24);
    if width < 24 || height < 7 {
        return None;
    }
    let q = Rect::new(
        a.x + (a.width - width) / 2,
        a.y + (a.height - height) / 3,
        width,
        height,
    );
    let i = panel(b, q, p.accent, p.panel_bg)?;
    put_text(
        b,
        q.x + 2,
        q.y,
        q.width.saturating_sub(4),
        " Command palette ",
        Style::default().fg(p.accent).bg(p.panel_bg),
    );

    let count = if overlay.query.is_empty() {
        total.to_string()
    } else {
        format!("{} of {total}", results.len())
    };
    put_text(
        b,
        i.x,
        i.y,
        3,
        " › ",
        Style::default()
            .fg(p.accent)
            .bg(p.panel_bg)
            .add_modifier(Modifier::BOLD),
    );
    let input = Rect::new(
        i.x + 3,
        i.y,
        i.width.saturating_sub(4 + display_width(&count)),
        1,
    );
    let cursor = text_editor::render(
        b,
        input,
        &overlay.query,
        Style::default().fg(p.text).bg(p.panel_bg),
    );
    // Drawn after the editor, which clears its row; the cursor stays at its start.
    if overlay.query.is_empty() {
        put_text(
            b,
            input.x,
            input.y,
            input.width,
            "jump to an agent, terminal, workspace, setting, or command",
            Style::default().fg(p.overlay0).bg(p.panel_bg),
        );
    }
    put_right_text(
        b,
        i,
        i.y,
        &count,
        Style::default().fg(p.overlay0).bg(p.panel_bg),
    );
    put_text(
        b,
        i.x,
        i.y + 1,
        i.width,
        &"─".repeat(i.width as usize),
        Style::default().fg(p.surface1).bg(p.panel_bg),
    );

    let body = Rect::new(i.x, i.y + 2, i.width, i.height.saturating_sub(3));
    let visible = usize::from(body.height);
    let selected = overlay.selected.min(results.len().saturating_sub(1));
    let scroll = overlay
        .scroll
        .max((selected + 1).saturating_sub(visible))
        .min(selected)
        .min(results.len().saturating_sub(visible));
    if results.is_empty() {
        put_text(
            b,
            body.x,
            body.y,
            body.width,
            " no matches",
            Style::default().fg(p.overlay0).bg(p.panel_bg),
        );
    }
    let mut rows = Vec::new();
    for (index, found) in results.iter().enumerate().skip(scroll).take(visible) {
        let entry = found.entry;
        let rect = Rect::new(body.x, body.y + (index - scroll) as u16, body.width, 1);
        rows.push((rect, index));
        let is_selected = index == selected;
        let dim = |style: Style| {
            if entry.stale {
                style.add_modifier(Modifier::DIM)
            } else {
                style
            }
        };
        let on_row = |fg| {
            dim(if is_selected {
                Style::default().fg(contrast(p)).bg(p.accent)
            } else {
                Style::default().fg(fg).bg(p.panel_bg)
            })
        };
        b.set_style(rect, on_row(p.text));

        let (marker, marker_color) = if entry.current {
            ("◆", p.accent)
        } else if let Some(status) = entry.status {
            (status_dot(status), status_color(status, p))
        } else {
            (" ", p.text)
        };
        let letter = crate::client::shell::command_palette::pick_letter(index)
            .map_or_else(|| " ".to_owned(), String::from);
        let mut left = vec![
            Span::raw(" "),
            Span::styled(letter, on_row(p.mauve).add_modifier(Modifier::BOLD)),
            Span::raw(" "),
            Span::styled(marker, on_row(marker_color)),
            Span::raw(" "),
        ];
        let title = on_row(p.text).add_modifier(Modifier::BOLD);
        let highlight = if is_selected {
            title.add_modifier(Modifier::UNDERLINED)
        } else {
            title.fg(p.accent)
        };
        let mut run = String::new();
        let mut run_highlighted = false;
        for (position, character) in entry.title.chars().enumerate() {
            let highlighted = found.highlights.binary_search(&position).is_ok();
            if highlighted != run_highlighted && !run.is_empty() {
                let style = if run_highlighted { highlight } else { title };
                left.push(Span::styled(std::mem::take(&mut run), style));
            }
            run_highlighted = highlighted;
            run.push(character);
        }
        if !run.is_empty() {
            left.push(Span::styled(
                run,
                if run_highlighted { highlight } else { title },
            ));
        }
        if !entry.detail.is_empty() {
            left.push(Span::styled(
                format!("  {}", entry.detail),
                on_row(p.overlay0),
            ));
        }

        let mut right = Vec::new();
        if let Some(hint) = entry.hint.as_deref() {
            right.push(Span::styled(
                format!("{hint}  "),
                on_row(p.mauve).add_modifier(Modifier::BOLD),
            ));
        }
        right.push(Span::styled(
            format!(
                "{:>width$} ",
                entry.kind.label(),
                width = usize::from(KIND_WIDTH)
            ),
            on_row(p.overlay0),
        ));
        let right = Line::from(right);
        let right_width = (right.width() as u16).min(rect.width);
        // The title keeps at least a third of the row when a hint is long.
        let left_width = rect.width.saturating_sub(right_width).max(rect.width / 3);
        b.set_line(rect.x, rect.y, &Line::from(left), left_width);
        if left_width + right_width <= rect.width {
            b.set_line(rect.right() - right_width, rect.y, &right, right_width);
        }
    }

    put_text(
        b,
        i.x,
        i.bottom() - 1,
        i.width,
        " ↑↓ select · ↵ open · ctrl+letter pick · tab complete · esc close",
        Style::default().fg(p.overlay0).bg(p.panel_bg),
    );
    Some(RenderedCommandPalette {
        area: q,
        rows,
        cursor,
    })
}
