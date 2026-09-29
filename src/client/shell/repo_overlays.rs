use super::*;

const MAX_VISIBLE_REPOS: usize = 6;

fn title_style(p: &Palette) -> Style {
    Style::default()
        .fg(p.text)
        .bg(p.panel_bg)
        .add_modifier(Modifier::BOLD)
}

fn label_style(p: &Palette) -> Style {
    Style::default().fg(p.overlay0).bg(p.panel_bg)
}

fn primary_style(p: &Palette) -> Style {
    Style::default()
        .fg(contrast(p))
        .bg(p.accent)
        .add_modifier(Modifier::BOLD)
}

fn secondary_style(p: &Palette) -> Style {
    Style::default()
        .fg(p.text)
        .bg(p.surface0)
        .add_modifier(Modifier::BOLD)
}

/// Keeps the end of a long path, which is the part that tells paths apart.
pub(super) fn tail_fit(text: &str, width: u16) -> String {
    let width = usize::from(width);
    if usize::from(display_width(text)) <= width {
        return text.to_owned();
    }
    let mut tail = String::new();
    let mut used = 1;
    for ch in text.chars().rev() {
        let ch_width = usize::from(display_width(ch.encode_utf8(&mut [0; 4])));
        if used + ch_width > width {
            break;
        }
        used += ch_width;
        tail.insert(0, ch);
    }
    format!("…{tail}")
}

pub(super) fn wrap_words(text: &str, width: u16) -> Vec<String> {
    let width = usize::from(width.max(1));
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        let candidate = usize::from(display_width(&line))
            + usize::from(!line.is_empty())
            + usize::from(display_width(word));
        if !line.is_empty() && candidate > width {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// `main · origin`, or just `main` for a repo without a remote.
pub(super) fn repo_branch_summary(repo: &crate::protocol::ClientShellRepo) -> String {
    match repo.remote.as_deref() {
        Some(remote) => format!("{} · {remote}", repo.base_branch),
        None => repo.base_branch.clone(),
    }
}

/// One repo per row: name, root, and base/remote on the right.
pub(super) fn render_repo_rows(
    b: &mut Buffer,
    area: Rect,
    repos: &[crate::protocol::ClientShellRepo],
    selected: usize,
    p: &Palette,
) -> Vec<(Rect, usize)> {
    let visible = usize::from(area.height).clamp(1, MAX_VISIBLE_REPOS);
    let start = selected
        .saturating_sub(visible.saturating_sub(1))
        .min(repos.len().saturating_sub(visible));
    let name_width = repos
        .iter()
        .map(|repo| display_width(&repo.name))
        .max()
        .unwrap_or(0)
        .clamp(8, 22);
    let mut hits = Vec::new();
    for (row, (index, repo)) in repos
        .iter()
        .enumerate()
        .skip(start)
        .take(visible)
        .enumerate()
    {
        let rect = Rect::new(area.x, area.y + row as u16, area.width, 1);
        let is_selected = index == selected;
        let style = if is_selected {
            Style::default()
                .fg(contrast(p))
                .bg(p.accent)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(p.text).bg(p.panel_bg)
        };
        let dim = if is_selected {
            style.remove_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(p.overlay1).bg(p.panel_bg)
        };
        b.set_style(rect, style);
        let marker = if is_selected { " ▸ " } else { "   " };
        put_text(b, rect.x, rect.y, rect.width, marker, style);
        put_text(
            b,
            rect.x + 3,
            rect.y,
            name_width.min(rect.width.saturating_sub(3)),
            &repo.name,
            style,
        );
        let summary = repo_branch_summary(repo);
        let summary_width = display_width(&summary) + 1;
        let root_x = rect.x + 3 + name_width + 2;
        let root_width = rect
            .right()
            .saturating_sub(root_x)
            .saturating_sub(summary_width + 1);
        if root_x < rect.right() {
            put_text(
                b,
                root_x,
                rect.y,
                root_width,
                &tail_fit(&repo.root, root_width),
                dim,
            );
        }
        put_right_text(b, rect, rect.y, &format!("{summary} "), dim);
        hits.push((rect, index));
    }
    hits
}

pub(super) fn render_space_worktree_overlay(
    b: &mut Buffer,
    dialog: &ClientSpaceWorktreeOverlay,
    s: &ClientShellSnapshot,
    p: &Palette,
) -> Option<OverlayRender> {
    let repos = &s.repos;
    if repos.is_empty() {
        let popup = popup(b.area, 64, 9)?;
        let inner = panel(b, popup, p.accent, p.panel_bg)?;
        put_text(
            b,
            inner.x,
            inner.y,
            inner.width,
            &format!("new worktree in {}", dialog.space_name),
            title_style(p),
        );
        put_text(
            b,
            inner.x,
            inner.y + 2,
            inner.width,
            " no repos yet. Add the repos you create worktrees from;",
            Style::default().fg(p.subtext0).bg(p.panel_bg),
        );
        put_text(
            b,
            inner.x,
            inner.y + 3,
            inner.width,
            " they are kept per machine and listed in settings → repos.",
            Style::default().fg(p.subtext0).bg(p.panel_bg),
        );
        let buttons = row(inner, &[16, 12], 2, inner.height.saturating_sub(1));
        let [add, cancel] = buttons.as_slice() else {
            return None;
        };
        button(b, *add, " ↵ add repo ", primary_style(p));
        button(b, *cancel, " esc cancel ", secondary_style(p));
        return Some(OverlayRender {
            area: popup,
            primary: *add,
            cancel: *cancel,
            overlay_hits: vec![(*add, ClientOverlayHit::SpaceWorktreeAddRepo)],
            ..OverlayRender::default()
        });
    }

    let list_height = repos.len().min(MAX_VISIBLE_REPOS) as u16;
    let popup = popup(b.area, 84, 17 + list_height)?;
    let inner = panel(b, popup, p.accent, p.panel_bg)?;
    let mut hits = Vec::new();
    put_text(
        b,
        inner.x,
        inner.y,
        inner.width,
        &format!("new worktree in {}", dialog.space_name),
        title_style(p),
    );
    put_text(
        b,
        inner.x,
        inner.y + 2,
        inner.width,
        " repo",
        label_style(p),
    );
    let add_label = " + add repo ";
    let add = Rect::new(
        inner.right().saturating_sub(display_width(add_label) + 1),
        inner.y + 2,
        display_width(add_label),
        1,
    );
    put_text(
        b,
        add.x,
        add.y,
        add.width,
        add_label,
        Style::default().fg(p.accent).bg(p.panel_bg),
    );
    hits.push((add, ClientOverlayHit::SpaceWorktreeAddRepo));
    let list = Rect::new(inner.x, inner.y + 3, inner.width, list_height);
    let selected = super::super::space_worktrees::selected_repo_index(dialog, repos);
    for (rect, index) in render_repo_rows(b, list, repos, selected, p) {
        hits.push((rect, ClientOverlayHit::SpaceWorktreeRepo(index)));
    }

    let mut y = list.bottom() + 1;
    put_text(b, inner.x, y, inner.width, " name", label_style(p));
    y += 1;
    let input = Rect::new(inner.x + 1, y, inner.width.saturating_sub(2), 1);
    let name_focused = dialog.field == SpaceWorktreeField::Name;
    let input_style =
        Style::default()
            .fg(p.text)
            .bg(if name_focused { p.surface0 } else { p.panel_bg });
    b.set_style(input, input_style);
    let cursor = text_editor::render(
        b,
        Rect::new(input.x + 1, input.y, input.width.saturating_sub(1), 1),
        &dialog.name,
        input_style,
    );
    hits.push((input, ClientOverlayHit::SpaceWorktreeName));

    let repo = &repos[selected];
    y += 2;
    let sync_available = repo.remote.is_some();
    let sync_on = dialog.sync && sync_available;
    let sync_text = match repo.remote.as_deref() {
        Some(remote) => format!(
            " [{}] sync {} with {remote} first",
            if sync_on { "x" } else { " " },
            repo.base_branch
        ),
        None => " [ ] sync (this repo has no remote)".to_owned(),
    };
    let sync_rect = Rect::new(inner.x, y, display_width(&sync_text) + 1, 1);
    let sync_focused = dialog.field == SpaceWorktreeField::Sync;
    put_text(
        b,
        sync_rect.x,
        sync_rect.y,
        sync_rect.width,
        &sync_text,
        Style::default()
            .fg(if !sync_available {
                p.overlay0
            } else if sync_focused {
                p.accent
            } else {
                p.text
            })
            .bg(p.panel_bg)
            .add_modifier(if sync_focused {
                Modifier::BOLD
            } else {
                Modifier::empty()
            }),
    );
    hits.push((sync_rect, ClientOverlayHit::SpaceWorktreeSync));

    y += 2;
    let name = dialog.name.trim();
    let preview = super::super::space_worktrees::space_worktree_preview(
        &s.worktree_path_template,
        &dialog.space_name,
        repo,
        name,
        sync_on,
    );
    let value_style = Style::default().fg(p.subtext0).bg(p.panel_bg);
    put_text(b, inner.x, y, 11, " branch", label_style(p));
    put_text(
        b,
        inner.x + 11,
        y,
        inner.width.saturating_sub(11),
        &preview.branch,
        if preview.valid || name.is_empty() {
            value_style
        } else {
            value_style.fg(p.red)
        },
    );
    put_text(b, inner.x, y + 1, 11, " checkout", label_style(p));
    let checkout_width = inner.width.saturating_sub(12);
    put_text(
        b,
        inner.x + 11,
        y + 1,
        checkout_width,
        &tail_fit(&preview.checkout, checkout_width),
        value_style,
    );

    let status_y = y + 3;
    if dialog.creating {
        let text = if sync_on {
            " syncing and creating…"
        } else {
            " creating…"
        };
        put_text(
            b,
            inner.x,
            status_y,
            inner.width,
            text,
            Style::default().fg(p.accent).bg(p.panel_bg),
        );
    } else if let Some(error) = dialog.error.as_deref() {
        let style = Style::default()
            .fg(if dialog.offer_without_sync {
                p.yellow
            } else {
                p.red
            })
            .bg(p.panel_bg);
        for (offset, line) in wrap_words(error, inner.width.saturating_sub(2))
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
                style,
            );
        }
    }
    let primary_label = if dialog.offer_without_sync {
        format!(" ↵ create from local {} ", repo.base_branch)
    } else {
        " ↵ create and open ".to_owned()
    };
    let buttons = row(
        inner,
        &[display_width(&primary_label), 12],
        2,
        inner.height.saturating_sub(1),
    );
    let [primary, cancel] = buttons.as_slice() else {
        return None;
    };
    button(b, *primary, &primary_label, primary_style(p));
    button(b, *cancel, " esc cancel ", secondary_style(p));
    Some(OverlayRender {
        area: popup,
        primary: *primary,
        cancel: *cancel,
        overlay_hits: hits,
        cursor: cursor.filter(|_| name_focused && !dialog.creating),
        ..OverlayRender::default()
    })
}

pub(super) fn render_repo_edit_overlay(
    b: &mut Buffer,
    edit: &ClientRepoEditOverlay,
    p: &Palette,
) -> Option<OverlayRender> {
    let popup = popup(b.area, 76, 14)?;
    let inner = panel(b, popup, p.accent, p.panel_bg)?;
    let title = match edit.original_name.as_deref() {
        Some(name) => format!("edit repo {name}"),
        None => "add repo".to_owned(),
    };
    put_text(b, inner.x, inner.y, inner.width, &title, title_style(p));
    let adding = edit.original_name.is_none();
    let placeholders = if adding {
        [
            "path to the main checkout, e.g. ~/code/project",
            "auto: the folder name",
            "auto: the remote's default branch",
            "auto: origin",
        ]
    } else {
        ["", "", "", "none"]
    };
    let label_width = 14u16;
    let mut hits = Vec::new();
    let mut cursor = None;
    for (index, label) in REPO_EDIT_FIELDS.iter().enumerate() {
        let y = inner.y + 2 + index as u16;
        put_text(
            b,
            inner.x,
            y,
            label_width,
            &format!(" {label}"),
            label_style(p),
        );
        let focused = edit.field == index;
        let field = Rect::new(
            inner.x + label_width,
            y,
            inner.width.saturating_sub(label_width + 1),
            1,
        );
        let style = Style::default()
            .fg(p.text)
            .bg(if focused { p.surface0 } else { p.panel_bg });
        b.set_style(field, style);
        let editor = &edit.fields[index];
        let field_cursor = text_editor::render(
            b,
            Rect::new(field.x + 1, field.y, field.width.saturating_sub(1), 1),
            editor,
            style,
        );
        if editor.is_empty() {
            put_text(
                b,
                field.x + 1,
                field.y,
                field.width.saturating_sub(1),
                placeholders[index],
                style.fg(p.overlay0),
            );
        }
        if focused {
            cursor = field_cursor;
        }
        hits.push((field, ClientOverlayHit::RepoEditField(index)));
    }
    let hint = if adding {
        " blank fields are detected from the repo"
    } else {
        " changes apply to new worktrees"
    };
    put_text(
        b,
        inner.x,
        inner.y + 7,
        inner.width,
        hint,
        Style::default().fg(p.overlay1).bg(p.panel_bg),
    );
    if edit.saving {
        put_text(
            b,
            inner.x,
            inner.y + 9,
            inner.width,
            " saving…",
            Style::default().fg(p.accent).bg(p.panel_bg),
        );
    } else if let Some(error) = edit.error.as_deref() {
        for (offset, line) in wrap_words(error, inner.width.saturating_sub(2))
            .iter()
            .take(2)
            .enumerate()
        {
            put_text(
                b,
                inner.x + 1,
                inner.y + 9 + offset as u16,
                inner.width.saturating_sub(1),
                line,
                Style::default().fg(p.red).bg(p.panel_bg),
            );
        }
    }
    let buttons = row(inner, &[10, 12], 2, inner.height.saturating_sub(1));
    let [save, cancel] = buttons.as_slice() else {
        return None;
    };
    button(b, *save, " ↵ save ", primary_style(p));
    button(b, *cancel, " esc cancel ", secondary_style(p));
    Some(OverlayRender {
        area: popup,
        primary: *save,
        cancel: *cancel,
        overlay_hits: hits,
        cursor: cursor.filter(|_| !edit.saving),
        ..OverlayRender::default()
    })
}

pub(super) fn render_existing_worktree_overlay(
    b: &mut Buffer,
    picker: &ClientSpaceWorktreeOpenOverlay,
    s: &ClientShellSnapshot,
    p: &Palette,
) -> Option<OverlayRender> {
    let filtered = picker.filtered_indices();
    let height = (filtered.len().max(1).saturating_mul(2) + 9).clamp(13, 28) as u16;
    let popup = popup(b.area, 92, height)?;
    let inner = panel(b, popup, p.accent, p.panel_bg)?;
    put_text(
        b,
        inner.x,
        inner.y,
        inner.width,
        &format!("add existing worktree to {}", picker.space_name),
        title_style(p),
    );
    let search = Rect::new(inner.x, inner.y + 1, inner.width, 1);
    let count = if picker.loading > 0 {
        "loading…".to_owned()
    } else {
        format!("{} checkouts", filtered.len())
    };
    put_text(
        b,
        search.x,
        search.y,
        search.width,
        " / ",
        Style::default().fg(p.overlay0).bg(p.panel_bg),
    );
    let cursor = text_editor::render(
        b,
        Rect::new(
            search.x + 3,
            search.y,
            search.width.saturating_sub(4 + display_width(&count)),
            1,
        ),
        &picker.query,
        Style::default().fg(p.text).bg(p.panel_bg),
    );
    if picker.query.is_empty() {
        put_text(
            b,
            search.x + 3,
            search.y,
            20,
            "filter",
            Style::default().fg(p.overlay0).bg(p.panel_bg),
        );
    }
    put_right_text(
        b,
        search,
        search.y,
        &format!("{count} "),
        Style::default().fg(p.overlay0).bg(p.panel_bg),
    );
    put_text(
        b,
        inner.x,
        inner.y + 2,
        inner.width,
        &"─".repeat(inner.width as usize),
        Style::default().fg(p.surface1).bg(p.panel_bg),
    );
    let body = Rect::new(
        inner.x,
        inner.y + 3,
        inner.width,
        inner.height.saturating_sub(6),
    );
    let visible = usize::from(body.height / 2).max(1);
    let selected_position = filtered
        .iter()
        .position(|index| *index == picker.selected)
        .unwrap_or(0);
    let start = selected_position
        .saturating_sub(visible.saturating_sub(1))
        .min(filtered.len().saturating_sub(visible));
    let mut hits = Vec::new();
    for (row, index) in filtered
        .iter()
        .copied()
        .skip(start)
        .take(visible)
        .enumerate()
    {
        let entry = &picker.entries[index];
        let rect = Rect::new(body.x, body.y + row as u16 * 2, body.width, 2);
        let selected =
            index == picker.selected || (row == 0 && !filtered.contains(&picker.selected));
        let style = if selected {
            Style::default().fg(contrast(p)).bg(p.accent)
        } else {
            Style::default().fg(p.text).bg(p.panel_bg)
        };
        b.set_style(rect, style);
        let branch = entry.branch.as_deref().unwrap_or("detached");
        put_text(
            b,
            rect.x,
            rect.y,
            rect.width,
            &format!(" {} · {branch}", entry.repo),
            style.add_modifier(Modifier::BOLD),
        );
        let status = match entry.open_workspace_id.as_deref() {
            Some(workspace_id) => {
                let space = s
                    .workspaces
                    .iter()
                    .find(|workspace| workspace.workspace_id == workspace_id)
                    .and_then(|workspace| workspace.space_id.as_deref())
                    .and_then(|space_id| s.spaces.iter().find(|space| space.space_id == space_id));
                match space {
                    Some(space) if space.space_id == picker.space_id => "already here".to_owned(),
                    Some(space) if !space.built_in => format!("open in {}", space.name),
                    _ => "open".to_owned(),
                }
            }
            None if !entry.is_linked_worktree => "main checkout".to_owned(),
            None => String::new(),
        };
        if !status.is_empty() {
            put_right_text(b, rect, rect.y, &format!("{status} "), style);
        }
        let path_width = rect.width.saturating_sub(2);
        put_text(
            b,
            rect.x + 1,
            rect.y + 1,
            path_width,
            &tail_fit(&entry.path, path_width),
            if selected {
                style
            } else {
                Style::default().fg(p.overlay0).bg(p.panel_bg)
            },
        );
        hits.push((rect, ClientOverlayHit::ExistingWorktree(index)));
    }
    if filtered.is_empty() && picker.loading == 0 {
        put_text(
            b,
            body.x,
            body.y,
            body.width,
            if picker.entries.is_empty() {
                " no checkouts found"
            } else {
                " no matching checkouts"
            },
            Style::default().fg(p.overlay0).bg(p.panel_bg),
        );
    }
    if picker.opening {
        put_text(
            b,
            inner.x,
            inner.bottom().saturating_sub(2),
            inner.width,
            " opening…",
            Style::default().fg(p.accent).bg(p.panel_bg),
        );
    } else if let Some(error) = picker.error.as_deref() {
        put_text(
            b,
            inner.x,
            inner.bottom().saturating_sub(2),
            inner.width,
            &format!(" {error}"),
            Style::default().fg(p.red).bg(p.panel_bg),
        );
    }
    let buttons = row(inner, &[12, 12], 2, inner.height.saturating_sub(1));
    let [primary, cancel] = buttons.as_slice() else {
        return None;
    };
    button(b, *primary, " ↵ add ", primary_style(p));
    button(b, *cancel, " esc cancel ", secondary_style(p));
    Some(OverlayRender {
        area: popup,
        primary: *primary,
        cancel: *cancel,
        overlay_hits: hits,
        cursor: cursor.filter(|_| !picker.opening),
        ..OverlayRender::default()
    })
}
