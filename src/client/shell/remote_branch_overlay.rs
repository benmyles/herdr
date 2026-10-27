use super::repo_overlays::{tail_fit, wrap_words};
use super::*;

/// The new worktree dialog's tab bar, on the row below its title.
pub(super) fn render_space_worktree_tabs(
    b: &mut Buffer,
    inner: Rect,
    dialog: &ClientSpaceWorktreeOverlay,
    p: &Palette,
    hits: &mut Vec<(Rect, ClientOverlayHit)>,
) {
    let y = inner.y + 1;
    let mut x = inner.x;
    for (tab, label) in [
        (SpaceWorktreeTab::NewBranch, " new branch "),
        (SpaceWorktreeTab::RemoteBranch, " remote branch "),
    ] {
        let width = display_width(label).min(inner.right().saturating_sub(x));
        let rect = Rect::new(x, y, width, 1);
        let style = if dialog.tab == tab {
            Style::default()
                .fg(contrast(p))
                .bg(p.accent)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(p.overlay1).bg(p.panel_bg)
        };
        b.set_style(rect, style);
        put_text(b, rect.x, rect.y, rect.width, label, style);
        hits.push((rect, ClientOverlayHit::SpaceWorktreeTab(tab)));
        x = x.saturating_add(width + 1);
    }
    put_right_text(
        b,
        Rect::new(inner.x, y, inner.width, 1),
        y,
        "ctrl+t switches ",
        Style::default().fg(p.overlay0).bg(p.panel_bg),
    );
    put_text(
        b,
        inner.x,
        y + 1,
        inner.width,
        &"─".repeat(inner.width as usize),
        Style::default().fg(p.surface0).bg(p.panel_bg),
    );
}

/// The remote branch tab, drawn into `inner` below the tab bar.
pub(super) fn render_remote_branch_tab(
    b: &mut Buffer,
    popup: Rect,
    inner: Rect,
    dialog: &ClientSpaceWorktreeOverlay,
    s: &ClientShellSnapshot,
    p: &Palette,
    mut hits: Vec<(Rect, ClientOverlayHit)>,
) -> Option<OverlayRender> {
    let picker = &dialog.remote;
    let dim = Style::default().fg(p.overlay0).bg(p.panel_bg);
    let label = Style::default().fg(p.overlay0).bg(p.panel_bg);

    // Repo chips; tab cycles them.
    let y = inner.y + 3;
    put_text(b, inner.x, y, 7, " repo", label);
    let mut x = inner.x + 7;
    for (index, repo) in s.repos.iter().enumerate() {
        let text = format!(" {} ", repo.name);
        let width = display_width(&text).min(inner.right().saturating_sub(x));
        if width == 0 {
            break;
        }
        let rect = Rect::new(x, y, width, 1);
        let style = if picker.repo.as_deref() == Some(repo.name.as_str()) {
            Style::default()
                .fg(p.text)
                .bg(p.surface1)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(p.subtext0).bg(p.panel_bg)
        };
        b.set_style(rect, style);
        put_text(b, rect.x, rect.y, rect.width, &text, style);
        hits.push((rect, ClientOverlayHit::RemoteBranchRepo(index)));
        x = x.saturating_add(width + 1);
    }
    if s.repos.len() > 1 && x + 18 < inner.right() {
        put_right_text(
            b,
            Rect::new(inner.x, y, inner.width, 1),
            y,
            "tab next repo ",
            dim,
        );
    }

    // Filter and list status.
    let search_y = y + 1;
    let filtered = picker.filtered_indices();
    let remote = picker.remote.as_deref().unwrap_or("remote");
    let status = if picker.loading {
        "loading…".to_owned()
    } else if picker.fetching {
        format!("{} · fetching {remote}…", filtered.len())
    } else if picker.truncated {
        format!("{} of the newest {}", filtered.len(), picker.branches.len())
    } else {
        format!("{} branches", filtered.len())
    };
    put_text(b, inner.x, search_y, 3, " / ", dim);
    let search_width = inner
        .width
        .saturating_sub(5 + display_width(&status))
        .max(1);
    let search_style = Style::default().fg(p.text).bg(p.panel_bg);
    let cursor = text_editor::render(
        b,
        Rect::new(inner.x + 3, search_y, search_width, 1),
        &picker.query,
        search_style,
    );
    if picker.query.is_empty() {
        put_text(
            b,
            inner.x + 3,
            search_y,
            search_width,
            "filter by branch or author",
            dim,
        );
    }
    put_right_text(
        b,
        Rect::new(inner.x, search_y, inner.width, 1),
        search_y,
        &format!("{status} "),
        dim,
    );
    put_text(
        b,
        inner.x,
        search_y + 1,
        inner.width,
        &"─".repeat(inner.width as usize),
        Style::default().fg(p.surface0).bg(p.panel_bg),
    );

    // Branches, newest first. The bottom keeps a checkout line, a status
    // line, and the buttons.
    let list = Rect::new(
        inner.x,
        search_y + 2,
        inner.width,
        inner.bottom().saturating_sub(search_y + 2 + 5),
    );
    let selected = picker.selected_index();
    let selected_position = selected
        .and_then(|selected| filtered.iter().position(|index| *index == selected))
        .unwrap_or(0);
    let visible = usize::from(list.height).max(1);
    let start = selected_position
        .saturating_sub(visible.saturating_sub(1))
        .min(filtered.len().saturating_sub(visible));
    for (row, index) in filtered
        .iter()
        .copied()
        .skip(start)
        .take(visible)
        .enumerate()
    {
        let branch = &picker.branches[index];
        let rect = Rect::new(list.x, list.y + row as u16, list.width, 1);
        let is_selected = Some(index) == selected;
        let style = if is_selected {
            Style::default()
                .fg(contrast(p))
                .bg(p.accent)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(p.text).bg(p.panel_bg)
        };
        let quiet = if is_selected {
            style.remove_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(p.overlay1).bg(p.panel_bg)
        };
        b.set_style(rect, style);
        let age = super::super::remote_branches::branch_age(branch.committed_at, picker.listed_at);
        let mut right = match branch.author.as_str() {
            "" => age,
            author => format!("{author} · {age}"),
        };
        if branch.checkout_path.is_some() {
            right = format!("checked out · {right}");
        }
        let right = format!("{right} ");
        let right_width = display_width(&right);
        let marker = if is_selected { " ▸ " } else { "   " };
        put_text(b, rect.x, rect.y, 3, marker, style);
        let room = rect.width.saturating_sub(3 + right_width + 1);
        let name_width = display_width(&branch.name).min(room);
        put_text(b, rect.x + 3, rect.y, name_width, &branch.name, style);
        let subject_x = rect.x + 3 + name_width + 2;
        let subject_width = room.saturating_sub(name_width + 2);
        if subject_width > 4 && !branch.subject.is_empty() {
            put_text(b, subject_x, rect.y, subject_width, &branch.subject, quiet);
        }
        put_right_text(b, rect, rect.y, &right, quiet);
        hits.push((rect, ClientOverlayHit::RemoteBranch(index)));
    }
    if filtered.is_empty() && !picker.loading {
        let text = if picker.repo.is_none() {
            " add a repo with a remote to list its branches"
        } else if picker.branches.is_empty() {
            if picker.fetching {
                " no remote branches known yet"
            } else {
                " no remote branches"
            }
        } else {
            " no matching branches"
        };
        put_text(b, list.x, list.y, list.width, text, dim);
    }

    // What the pick does.
    let checkout_y = inner.bottom().saturating_sub(5);
    let value = Style::default().fg(p.subtext0).bg(p.panel_bg);
    let picked = selected.and_then(|index| picker.branches.get(index));
    let repo = picker
        .repo
        .as_deref()
        .and_then(|name| s.repos.iter().find(|repo| repo.name == name));
    if let (Some(branch), Some(repo)) = (picked, repo) {
        let (heading, path) = match branch.checkout_path.as_deref() {
            Some(path) => (" opens", path.to_owned()),
            None => (
                " checkout",
                super::super::remote_branches::remote_branch_checkout_preview(
                    &s.worktree_path_template,
                    &dialog.space_name,
                    repo,
                    &branch.name,
                ),
            ),
        };
        put_text(b, inner.x, checkout_y, 11, heading, label);
        let width = inner.width.saturating_sub(12);
        put_text(
            b,
            inner.x + 11,
            checkout_y,
            width,
            &tail_fit(&path, width),
            value,
        );
    }

    let status_y = checkout_y + 2;
    if dialog.creating {
        put_text(
            b,
            inner.x,
            status_y,
            inner.width,
            if picked.is_some_and(|branch| branch.checkout_path.is_some()) {
                " opening…"
            } else {
                " checking out…"
            },
            Style::default().fg(p.accent).bg(p.panel_bg),
        );
    } else {
        let message = match (dialog.error.as_deref(), picker.fetch_error.as_deref()) {
            (Some(error), _) => Some((error.to_owned(), p.red)),
            (None, Some(error)) => Some((
                format!("{error}; showing the branches already fetched"),
                p.yellow,
            )),
            (None, None) => None,
        };
        if let Some((message, color)) = message {
            for (offset, line) in wrap_words(&message, inner.width.saturating_sub(2))
                .iter()
                .take(2)
                .enumerate()
            {
                put_text(
                    b,
                    inner.x + 1,
                    status_y + offset as u16,
                    inner.width.saturating_sub(1),
                    line,
                    Style::default().fg(color).bg(p.panel_bg),
                );
            }
        }
    }

    let primary_label = if picked.is_some_and(|branch| branch.checkout_path.is_some()) {
        " ↵ open checkout "
    } else {
        " ↵ check out and open "
    };
    let buttons = row(
        inner,
        &[display_width(primary_label), 12],
        2,
        inner.height.saturating_sub(1),
    );
    let [primary, cancel] = buttons.as_slice() else {
        return None;
    };
    button(
        b,
        *primary,
        primary_label,
        Style::default()
            .fg(contrast(p))
            .bg(p.accent)
            .add_modifier(Modifier::BOLD),
    );
    button(
        b,
        *cancel,
        " esc cancel ",
        Style::default()
            .fg(p.text)
            .bg(p.surface0)
            .add_modifier(Modifier::BOLD),
    );
    Some(OverlayRender {
        area: popup,
        primary: *primary,
        cancel: *cancel,
        overlay_hits: hits,
        cursor: cursor.filter(|_| !dialog.creating),
        ..OverlayRender::default()
    })
}
