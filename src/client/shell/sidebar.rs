use super::*;
use ratatui::{
    text::Line,
    widgets::{Paragraph, Widget},
};

fn workspace_selection_background(palette: &Palette) -> ratatui::style::Color {
    if palette.selection_bg == ratatui::style::Color::Reset {
        palette.active_row_bg
    } else {
        palette.selection_bg
    }
}

pub(in crate::client::shell) fn workspace_active_background(
    palette: &Palette,
    navigating: bool,
) -> ratatui::style::Color {
    // The fallback cursor shares the active-row color; only fill the cursor while navigating.
    if navigating && palette.selection_bg == ratatui::style::Color::Reset {
        palette.sidebar_bg
    } else {
        palette.active_row_bg
    }
}

fn space_workspaces(
    snapshot: &ClientShellSnapshot,
) -> impl Iterator<Item = crate::ui::SpaceWorkspace> + '_ {
    snapshot
        .workspaces
        .iter()
        .enumerate()
        .map(|(index, workspace)| {
            match workspace.space_id.as_deref().and_then(|space_id| {
                snapshot
                    .spaces
                    .iter()
                    .find(|space| space.space_id == space_id)
            }) {
                Some(space) => crate::ui::SpaceWorkspace {
                    color_slot: (!space.built_in).then_some(space.color),
                },
                // Servers without spaces: every workspace is its own space.
                None => crate::ui::SpaceWorkspace {
                    color_slot: Some(index),
                },
            }
        })
}

/// Color of a space header.
pub(in crate::client::shell) fn space_header_color(
    space: &crate::protocol::ClientShellSpace,
    palette: &Palette,
) -> ratatui::style::Color {
    crate::ui::space_slot_color(palette, (!space.built_in).then_some(space.color))
}

/// Space colors for one endpoint's workspaces, indexed like `snapshot.workspaces`.
pub(in crate::client::shell) fn space_presentation(
    snapshot: &ClientShellSnapshot,
    palette: &Palette,
) -> crate::ui::SpacePresentation {
    crate::ui::SpacePresentation::new(palette, space_workspaces(snapshot))
}

pub(in crate::client::shell) fn collapsed_sidebar_sections(
    area: Rect,
) -> (Rect, Option<u16>, Rect) {
    let content = Rect::new(area.x, area.y, area.width.saturating_sub(1), area.height);
    if content.is_empty() {
        return (Rect::default(), None, Rect::default());
    }
    if content.height < 7 {
        return (content, None, Rect::default());
    }
    let workspace_height = content.height.div_ceil(2);
    let divider_y = content.y + workspace_height;
    let detail_height = content.height.saturating_sub(workspace_height + 1);
    (
        Rect::new(content.x, content.y, content.width, workspace_height),
        Some(divider_y),
        Rect::new(content.x, divider_y + 1, content.width, detail_height),
    )
}

pub(crate) fn render_collapsed_sidebar(
    buffer: &mut Buffer,
    area: Rect,
    snapshot: &ClientShellSnapshot,
    config: &ClientShellConfig,
    selected_workspace_id: Option<&str>,
    hits: &mut ShellHitMap,
) {
    let palette = &config.palette;
    let selection_background = workspace_selection_background(palette);
    let active_background = workspace_active_background(palette, selected_workspace_id.is_some());
    render_sidebar_background(buffer, area, palette);
    let spaces = space_presentation(snapshot, palette);
    let (workspace_area, divider_y, detail_area) = collapsed_sidebar_sections(area);
    for (index, workspace) in snapshot
        .workspaces
        .iter()
        .take(workspace_area.height as usize)
        .enumerate()
    {
        let rect = Rect::new(
            workspace_area.x,
            workspace_area.y + index as u16,
            workspace_area.width,
            1,
        );
        let selected = selected_workspace_id == Some(workspace.workspace_id.as_str());
        if selected {
            buffer.set_style(rect, Style::default().bg(selection_background));
        } else if workspace.focused {
            buffer.set_style(rect, Style::default().bg(active_background));
        }
        let space_color = spaces.color(index);
        let number_style = if selected {
            Style::default()
                .fg(space_color)
                .bg(selection_background)
                .add_modifier(Modifier::BOLD)
        } else if workspace.focused {
            Style::default()
                .fg(space_color)
                .bg(active_background)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(space_color)
        };
        put_text(
            buffer,
            rect.x,
            rect.y,
            rect.width.min(2),
            &format!("{:<2}", index + 1),
            number_style,
        );
        let status = workspace.agent_status;
        put_text(
            buffer,
            rect.x.saturating_add(2),
            rect.y,
            rect.width.saturating_sub(2),
            status_icon(status, config.status_indicators),
            Style::default().fg(status_color(status, palette)),
        );
        hits.workspaces.push(WorkspaceHit {
            rect,
            endpoint_id: ClientEndpointId::Local,
            workspace_id: workspace.workspace_id.clone(),
            indented: false,
        });
    }

    if let Some(divider_y) = divider_y {
        put_text(
            buffer,
            workspace_area.x,
            divider_y,
            workspace_area.width,
            &"─".repeat(workspace_area.width as usize),
            Style::default().fg(palette.surface_dim),
        );
    }

    let detail_content = Rect::new(
        detail_area.x,
        detail_area.y,
        detail_area.width,
        detail_area.height.saturating_sub(1),
    );
    for (index, pane_id) in super::ordered_agent_pane_ids(snapshot, config.agent_panel_sort)
        .into_iter()
        .take(detail_content.height as usize)
        .enumerate()
    {
        let Some(agent) = snapshot
            .agents
            .iter()
            .find(|agent| agent.pane_id == pane_id)
        else {
            continue;
        };
        let rect = Rect::new(
            detail_content.x,
            detail_content.y + index as u16,
            detail_content.width,
            1,
        );
        if agent.focused {
            buffer.set_style(rect, Style::default().bg(palette.active_row_bg));
        }
        let space_color = snapshot
            .workspaces
            .iter()
            .position(|workspace| workspace.workspace_id == agent.workspace_id)
            .map_or(palette.overlay0, |workspace_index| {
                spaces.color(workspace_index)
            });
        put_text(
            buffer,
            rect.x,
            rect.y,
            rect.width.min(2),
            &format!("{:<2}", index + 1),
            if agent.focused {
                Style::default()
                    .fg(space_color)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(space_color)
            },
        );
        put_text(
            buffer,
            rect.x.saturating_add(2),
            rect.y,
            rect.width.saturating_sub(2),
            status_icon(agent.agent_status, config.status_indicators),
            Style::default().fg(status_color(agent.agent_status, palette)),
        );
        hits.agents.push((rect, pane_id));
    }
    hits.sidebar_toggle = if area.is_empty() || workspace_area.width == 0 {
        Rect::default()
    } else {
        Rect::new(
            workspace_area.x + workspace_area.width / 2,
            area.bottom().saturating_sub(1),
            1,
            1,
        )
    };
    put_text(
        buffer,
        hits.sidebar_toggle.x,
        hits.sidebar_toggle.y,
        hits.sidebar_toggle.width,
        "»",
        if super::super::global_menu::global_menu_attention(snapshot) {
            Style::default()
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(palette.overlay0)
        },
    );
}

pub(crate) fn render_sidebar(
    buffer: &mut Buffer,
    area: Rect,
    snapshot: &ClientShellSnapshot,
    config: &ClientShellConfig,
    state: &mut ShellRenderState<'_>,
    hits: &mut ShellHitMap,
) {
    let palette = &config.palette;
    render_sidebar_background(buffer, area, palette);
    hits.sidebar_divider = if area.is_empty() {
        Rect::default()
    } else {
        Rect::new(area.right().saturating_sub(1), area.y, 1, area.height)
    };
    let (workspace_area, detail_area) =
        crate::ui::expanded_sidebar_sections(area, state.sidebar_section_split);
    hits.sidebar_section_divider =
        crate::ui::sidebar_section_divider_rect(area, state.sidebar_section_split);
    put_text(
        buffer,
        workspace_area.x,
        workspace_area.y,
        workspace_area.width,
        " spaces",
        Style::default()
            .fg(palette.overlay0)
            .add_modifier(Modifier::BOLD),
    );

    let rows = sidebar_rows(
        snapshot,
        state.collapsed_groups,
        state.dragged_workspace_id.is_some(),
    );
    let spaces = space_presentation(snapshot, palette);
    let body = Rect::new(
        workspace_area.x,
        workspace_area.y.saturating_add(WORKSPACE_HEADER_ROWS),
        workspace_area.width,
        workspace_area
            .height
            .saturating_sub(WORKSPACE_HEADER_ROWS + 1),
    );
    hits.workspace_body = body;
    let row_heights = rows
        .iter()
        .map(|row| match row {
            SidebarRow::Workspace(entry) => snapshot
                .workspaces
                .get(entry.index)
                .map(|workspace| {
                    workspace_rows(
                        workspace,
                        workspace.agent_status,
                        entry.indented,
                        &config.spaces,
                    )
                    .len()
                    .max(1)
                    .min(u16::MAX as usize) as u16
                })
                .unwrap_or(1),
            SidebarRow::SpaceHeader { .. }
            | SidebarRow::ClosedMember { .. }
            | SidebarRow::AddWorktree { .. } => 1,
        })
        .collect::<Vec<_>>();
    let gaps = (0..rows.len())
        .map(|index| sidebar_row_gap(&rows, index, config.spaces.row_gap))
        .collect::<Vec<_>>();
    let mut metrics = super::scroll::list_scroll_metrics(
        &row_heights,
        &gaps,
        body.height,
        *state.workspace_scroll,
    );
    if !body.is_empty() && std::mem::take(state.reveal_focused_workspace) {
        if let Some(target) = rows.iter().position(|row| {
            matches!(row, SidebarRow::Workspace(entry) if snapshot.workspaces[entry.index].focused)
        }) {
            *state.workspace_scroll = super::scroll::list_scroll_start_to_reveal(
                &row_heights,
                &gaps,
                body.height,
                *state.workspace_scroll,
                target,
            );
            metrics = super::scroll::list_scroll_metrics(
                &row_heights,
                &gaps,
                body.height,
                *state.workspace_scroll,
            );
        }
    }
    hits.workspace_max_scroll = metrics.max_offset_from_bottom;
    hits.workspace_scroll_metrics = Some(metrics);
    *state.workspace_scroll = metrics
        .max_offset_from_bottom
        .saturating_sub(metrics.offset_from_bottom);
    let show_scrollbar = metrics.max_offset_from_bottom > 0 && body.width > 1;
    let content_width = body.width.saturating_sub(u16::from(show_scrollbar));
    let mut y = body.y;
    for (row_index, row) in rows.iter().enumerate().skip(*state.workspace_scroll) {
        let row_height = row_heights[row_index].min(body.height);
        if y.saturating_add(row_height) > body.bottom() {
            break;
        }
        let rect = Rect::new(body.x, y, content_width, row_height);
        match row {
            SidebarRow::SpaceHeader {
                space_index,
                collapsed,
            } => {
                let space = &snapshot.spaces[*space_index];
                render_space_header(buffer, rect, snapshot, space, *collapsed, config);
                hits.space_headers.push(SpaceHeaderHit {
                    rect,
                    endpoint_id: ClientEndpointId::Local,
                    space_id: space.space_id.clone(),
                });
            }
            SidebarRow::ClosedMember {
                space_index,
                member_index,
                last_child,
            } => {
                let space = &snapshot.spaces[*space_index];
                let member = &space.closed[*member_index];
                render_closed_member(buffer, rect, space, member, *last_child, palette);
                hits.closed_members.push(ClosedMemberHit {
                    rect,
                    endpoint_id: ClientEndpointId::Local,
                    space_id: space.space_id.clone(),
                    member_id: member.member_id.clone(),
                });
            }
            SidebarRow::AddWorktree { space_index } => {
                let space = &snapshot.spaces[*space_index];
                render_add_worktree_row(buffer, rect, space, palette);
                hits.add_worktree.push(AddWorktreeHit {
                    rect,
                    endpoint_id: ClientEndpointId::Local,
                    space_id: space.space_id.clone(),
                });
            }
            SidebarRow::Workspace(entry) => {
                let Some(workspace) = snapshot.workspaces.get(entry.index) else {
                    continue;
                };
                let status = workspace.agent_status;
                let tokens = workspace_rows(workspace, status, entry.indented, &config.spaces);
                let selected = state.selected_workspace_id.is_some_and(|target| {
                    target.matches(state.active_endpoint_id, &workspace.workspace_id)
                });
                let dragged = state.dragged_workspace_id == Some(workspace.workspace_id.as_str());
                if selected {
                    buffer.set_style(rect, Style::default().bg(palette.selection_bg));
                } else if dragged {
                    buffer.set_style(rect, Style::default().bg(palette.surface1));
                } else if workspace.focused {
                    buffer.set_style(rect, Style::default().bg(palette.active_row_bg));
                }
                render_workspace_rows(
                    buffer,
                    rect,
                    status,
                    config.status_indicators,
                    entry,
                    tokens,
                    workspace.focused,
                    selected,
                    state.selected_workspace_id.is_some(),
                    dragged,
                    WorkspaceRowColors {
                        palette,
                        space: spaces.color(entry.index),
                    },
                );
                hits.workspaces.push(WorkspaceHit {
                    rect,
                    endpoint_id: ClientEndpointId::Local,
                    workspace_id: workspace.workspace_id.clone(),
                    indented: entry.indented,
                });
            }
        }
        y = y.saturating_add(row_height + gaps[row_index]);
    }

    if show_scrollbar {
        let track = Rect::new(body.right().saturating_sub(1), body.y, 1, body.height);
        hits.workspace_scrollbar = track;
        super::scroll::render_list_scrollbar(buffer, track, metrics, palette);
    }

    if let Some(row) = state.workspace_drop_indicator_row {
        render_drop_indicator(buffer, workspace_area, body, row, palette);
    }

    let footer_y = workspace_area.bottom().saturating_sub(1);
    if config.mouse_capture {
        hits.new_workspace = Rect::new(
            workspace_area.x,
            footer_y,
            5.min(workspace_area.width),
            u16::from(workspace_area.height > 0),
        );
        put_text(
            buffer,
            workspace_area.x,
            footer_y,
            workspace_area.width,
            " new",
            Style::default().fg(palette.overlay0),
        );
        let attention = super::super::global_menu::global_menu_attention(snapshot);
        let launcher_width = if attention { 8 } else { 6 }.min(workspace_area.width);
        hits.global_launcher = Rect::new(
            workspace_area.right().saturating_sub(launcher_width),
            footer_y,
            launcher_width,
            1,
        );
        if attention {
            let start_x = workspace_area.right().saturating_sub(6);
            put_text(
                buffer,
                start_x,
                footer_y,
                2,
                "● ",
                Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD),
            );
            put_text(
                buffer,
                start_x.saturating_add(2),
                footer_y,
                4,
                "menu",
                Style::default().fg(palette.overlay0),
            );
        } else {
            put_right_text(
                buffer,
                workspace_area,
                footer_y,
                "menu",
                Style::default().fg(palette.overlay0),
            );
        }
    }

    *state.agent_marks_animating |= super::render_agent_panel(
        buffer,
        detail_area,
        snapshot,
        config,
        state.agent_grid,
        state.agent_clock,
        state.agent_scroll,
        hits,
    );

    hits.sidebar_toggle = Rect::new(
        area.right().saturating_sub(2),
        area.bottom().saturating_sub(1),
        u16::from(area.width > 1),
        u16::from(area.height > 0),
    );
    put_text(
        buffer,
        hits.sidebar_toggle.x,
        hits.sidebar_toggle.y,
        hits.sidebar_toggle.width,
        "«",
        Style::default().fg(palette.overlay0),
    );
}

/// One sidebar row in space order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::client::shell) enum SidebarRow {
    SpaceHeader {
        space_index: usize,
        collapsed: bool,
    },
    Workspace(WorkspaceEntry),
    ClosedMember {
        space_index: usize,
        member_index: usize,
        last_child: bool,
    },
    /// "+ worktree" at the end of an expanded space.
    AddWorktree {
        space_index: usize,
    },
}

/// Sidebar rows: each space's header, then its live members, then its closed
/// members, then "+ worktree" when the server can create worktrees. Empty
/// `other` is hidden unless `show_empty_other` (a drag needs it as a target).
/// Servers without spaces list workspaces flat, as before spaces existed.
pub(in crate::client::shell) fn sidebar_rows(
    snapshot: &ClientShellSnapshot,
    collapsed_groups: &HashSet<String>,
    show_empty_other: bool,
) -> Vec<SidebarRow> {
    // Servers without space worktrees send no path template.
    let add_rows = !snapshot.worktree_path_template.is_empty();
    let mut rows = Vec::new();
    let mut listed = vec![false; snapshot.workspaces.len()];
    for (space_index, space) in snapshot.spaces.iter().enumerate() {
        let members = snapshot
            .workspaces
            .iter()
            .enumerate()
            .filter(|(_, workspace)| workspace.space_id.as_deref() == Some(&space.space_id))
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        for &index in &members {
            listed[index] = true;
        }
        if space.built_in && members.is_empty() && space.closed.is_empty() && !show_empty_other {
            continue;
        }
        let collapsed = collapsed_groups.contains(&space.space_id);
        rows.push(SidebarRow::SpaceHeader {
            space_index,
            collapsed,
        });
        if collapsed {
            // Keep the focused member visible so the focus never disappears.
            if let Some(index) = members
                .iter()
                .copied()
                .find(|index| snapshot.workspaces[*index].focused)
            {
                rows.push(SidebarRow::Workspace(WorkspaceEntry {
                    index,
                    indented: true,
                    last_child: true,
                }));
            }
            continue;
        }
        let total = members.len() + space.closed.len() + usize::from(add_rows);
        for (position, index) in members.iter().copied().enumerate() {
            rows.push(SidebarRow::Workspace(WorkspaceEntry {
                index,
                indented: true,
                last_child: position + 1 == total,
            }));
        }
        for member_index in 0..space.closed.len() {
            rows.push(SidebarRow::ClosedMember {
                space_index,
                member_index,
                last_child: members.len() + member_index + 1 == total,
            });
        }
        if add_rows {
            rows.push(SidebarRow::AddWorktree { space_index });
        }
    }
    for (index, listed) in listed.into_iter().enumerate() {
        if !listed {
            rows.push(SidebarRow::Workspace(WorkspaceEntry {
                index,
                indented: false,
                last_child: false,
            }));
        }
    }
    rows
}

/// Workspace rows in sidebar order, for navigation and selection.
pub(crate) fn workspace_entries(
    snapshot: &ClientShellSnapshot,
    collapsed_groups: &HashSet<String>,
) -> Vec<WorkspaceEntry> {
    sidebar_rows(snapshot, collapsed_groups, false)
        .into_iter()
        .filter_map(|row| match row {
            SidebarRow::Workspace(entry) => Some(entry),
            _ => None,
        })
        .collect()
}

/// Blank rows after `rows[index]`: one before each space header after the
/// first, and the configured row gap between members.
pub(in crate::client::shell) fn sidebar_row_gap(
    rows: &[SidebarRow],
    index: usize,
    row_gap: u16,
) -> u16 {
    match rows.get(index + 1) {
        Some(SidebarRow::SpaceHeader { .. }) => 1,
        Some(_) if !matches!(rows[index], SidebarRow::SpaceHeader { .. }) => row_gap,
        _ => 0,
    }
}

/// The most urgent status among a space's live members.
pub(in crate::client::shell) fn space_status(
    snapshot: &ClientShellSnapshot,
    space_id: &str,
) -> Option<crate::api::schema::AgentStatus> {
    snapshot
        .workspaces
        .iter()
        .filter(|workspace| workspace.space_id.as_deref() == Some(space_id))
        .map(|workspace| workspace.agent_status)
        .max_by_key(|status| status_priority(*status))
}

pub(in crate::client::shell) fn space_member_count(
    snapshot: &ClientShellSnapshot,
    space: &crate::protocol::ClientShellSpace,
) -> usize {
    snapshot
        .workspaces
        .iter()
        .filter(|workspace| workspace.space_id.as_deref() == Some(space.space_id.as_str()))
        .count()
        + space.closed.len()
}

/// Render a space header: name in the space color, and when collapsed its most
/// urgent status and member count. Returns the header rect for hit testing.
pub(in crate::client::shell) fn render_space_header(
    buffer: &mut Buffer,
    rect: Rect,
    snapshot: &ClientShellSnapshot,
    space: &crate::protocol::ClientShellSpace,
    collapsed: bool,
    config: &ClientShellConfig,
) {
    let palette = &config.palette;
    let color = space_header_color(space, palette);
    let mut x = rect.x.saturating_add(1);
    if collapsed {
        if let Some(status) = space_status(snapshot, &space.space_id) {
            x = put_segment(
                buffer,
                x,
                rect.y,
                rect.right(),
                status_icon(status, config.status_indicators),
                Style::default().fg(status_color(status, palette)),
            );
            x = put_segment(buffer, x, rect.y, rect.right(), " ", Style::default());
        }
    }
    let suffix = if collapsed {
        format!("  {}", space_member_count(snapshot, space))
    } else {
        String::new()
    };
    let available = rect.right().saturating_sub(2).saturating_sub(x) as usize;
    let name = crate::ui::truncate_end(
        &space.name,
        available.saturating_sub(display_width(&suffix) as usize),
    );
    x = put_segment(
        buffer,
        x,
        rect.y,
        rect.right().saturating_sub(2),
        &name,
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    );
    put_segment(
        buffer,
        x,
        rect.y,
        rect.right().saturating_sub(2),
        &suffix,
        Style::default().fg(muted_space_color(color, palette)),
    );
    put_text(
        buffer,
        rect.right().saturating_sub(1),
        rect.y,
        1,
        if collapsed { "▸" } else { "▾" },
        Style::default().fg(color),
    );
}

/// Render a closed member: tree prefix, dimmed label, and a `closed` tag.
/// Draws the drag insertion line through the blank cells of `row`, so the
/// text it passes over stays readable.
pub(in crate::client::shell) fn render_drop_indicator(
    buffer: &mut Buffer,
    workspace_area: Rect,
    body: Rect,
    row: u16,
    palette: &Palette,
) {
    if row < workspace_area.y.saturating_add(1) || row >= workspace_area.bottom().saturating_sub(1)
    {
        return;
    }
    let style = Style::default().fg(palette.accent);
    for x in body.x..body.right() {
        let cell = &mut buffer[(x, row)];
        if cell.symbol() == " " {
            cell.set_symbol("─").set_style(style);
        }
    }
}

pub(in crate::client::shell) fn render_add_worktree_row(
    buffer: &mut Buffer,
    rect: Rect,
    space: &crate::protocol::ClientShellSpace,
    palette: &Palette,
) {
    let x = put_segment(
        buffer,
        rect.x,
        rect.y,
        rect.right(),
        " └─ ",
        Style::default().fg(space_header_color(space, palette)),
    );
    put_text(
        buffer,
        x,
        rect.y,
        rect.right().saturating_sub(x),
        "+ worktree",
        Style::default().fg(palette.overlay0),
    );
}

pub(in crate::client::shell) fn render_closed_member(
    buffer: &mut Buffer,
    rect: Rect,
    space: &crate::protocol::ClientShellSpace,
    member: &crate::protocol::ClientShellClosedMember,
    last_child: bool,
    palette: &Palette,
) {
    let color = space_header_color(space, palette);
    let muted = muted_space_color(color, palette);
    let x = put_segment(
        buffer,
        rect.x,
        rect.y,
        rect.right(),
        if last_child {
            " └─   "
        } else {
            " ├─   "
        },
        Style::default().fg(color),
    );
    let tag = "closed";
    let tag_width = display_width(tag);
    let available = rect.right().saturating_sub(x).saturating_sub(tag_width + 2) as usize;
    put_text(
        buffer,
        x,
        rect.y,
        available as u16,
        &crate::ui::truncate_end(&member.label, available),
        Style::default().fg(muted),
    );
    put_text(
        buffer,
        rect.right().saturating_sub(tag_width + 1),
        rect.y,
        tag_width,
        tag,
        Style::default().fg(palette.overlay0),
    );
}

pub(in crate::client::shell) fn workspace_rows(
    workspace: &ClientShellWorkspace,
    status: crate::api::schema::AgentStatus,
    indented: bool,
    config: &SpacesSidebarConfig,
) -> Vec<Vec<crate::ui::ResolvedToken>> {
    // Space members show their own label with branch details below; the
    // space header above already names the feature.
    let _ = indented;
    let token_values = workspace.tokens.iter().cloned().collect::<HashMap<_, _>>();
    let mut rows = crate::ui::sidebar_space_rows(
        config,
        crate::ui::SpaceTokenContext {
            workspace: &workspace.label,
            branch: workspace.branch.as_deref(),
            state_text: status_text(status),
            ahead_behind: workspace.git_ahead_behind,
            tokens: &token_values,
            suppress_git_details: false,
        },
    );
    // The repo's create command reports first on the worktree's last row,
    // ahead of the branch, which is what yields to a narrow sidebar.
    if let (Some(setup), Some(row)) = (workspace.setup.as_ref(), rows.last_mut()) {
        let (text, color) = if setup.running {
            (
                "setup…",
                crate::config::SidebarTokenColor::rgb(0xc7, 0x8a, 0x1f),
            )
        } else {
            (
                "setup ✗",
                crate::config::SidebarTokenColor::rgb(0xc0, 0x4a, 0x4a),
            )
        };
        row.insert(
            0,
            crate::ui::ResolvedToken {
                kind: crate::ui::ResolvedTokenKind::Custom(text.to_owned()),
                style: crate::config::SidebarTokenStyle {
                    fg: Some(color),
                    bold: None,
                    dim: Some(false),
                },
            },
        );
    }
    rows
}

/// Secondary sidebar text keeps its space hue but recedes toward the sidebar
/// background instead of stacking terminal faint on a muted color.
pub(in crate::client::shell) fn muted_space_color(
    color: ratatui::style::Color,
    palette: &Palette,
) -> ratatui::style::Color {
    crate::ui::mute_color(color, palette.sidebar_bg, 60)
}

/// Theme palette plus the workspace's space color for one sidebar row.
#[derive(Clone, Copy)]
pub(in crate::client::shell) struct WorkspaceRowColors<'a> {
    pub(in crate::client::shell) palette: &'a Palette,
    pub(in crate::client::shell) space: ratatui::style::Color,
}

pub(in crate::client::shell) fn render_workspace_rows(
    buffer: &mut Buffer,
    area: Rect,
    status: crate::api::schema::AgentStatus,
    indicators: crate::config::StatusIndicatorStyle,
    entry: &WorkspaceEntry,
    rows: Vec<Vec<crate::ui::ResolvedToken>>,
    focused: bool,
    selected: bool,
    navigating: bool,
    dragged: bool,
    colors: WorkspaceRowColors<'_>,
) {
    let WorkspaceRowColors {
        palette,
        space: space_color,
    } = colors;
    for (row_index, row) in rows.iter().enumerate() {
        let y = area.y + row_index as u16;
        if y >= area.bottom() {
            break;
        }
        let mut x = area.x;
        if entry.indented {
            let prefix = if row_index == 0 {
                if entry.last_child {
                    " └─ "
                } else {
                    " ├─ "
                }
            } else if entry.last_child {
                "      "
            } else {
                " │    "
            };
            x = put_segment(
                buffer,
                x,
                y,
                area.right(),
                prefix,
                Style::default().fg(space_color),
            );
        } else if row_index == 0 {
            x = x.saturating_add(1);
        } else {
            x = x.saturating_add(3);
        }
        let highlighted = focused || selected || dragged;
        let workspace_style = Style::default()
            .fg(space_color)
            .add_modifier(if highlighted {
                Modifier::BOLD
            } else {
                Modifier::empty()
            });
        let secondary_style = Style::default().fg(if highlighted {
            space_color
        } else {
            muted_space_color(space_color, palette)
        });
        let spans = crate::ui::resolved_token_spans(
            row,
            (
                status_icon(status, indicators),
                Style::default().fg(status_color(status, palette)),
            ),
            Style::default().fg(status_color(status, palette)),
            workspace_style,
            secondary_style,
            Style::default().fg(palette.overlay1),
            palette,
            area.right().saturating_sub(2).saturating_sub(x) as usize,
        );
        Paragraph::new(Line::from(spans)).render(
            Rect::new(x, y, area.right().saturating_sub(2).saturating_sub(x), 1),
            buffer,
        );
    }

    let background = if selected {
        Some(workspace_selection_background(palette))
    } else if dragged {
        Some(palette.surface1)
    } else if focused {
        Some(workspace_active_background(palette, navigating))
    } else {
        None
    };
    if let Some(background) = background {
        for y in area.y..area.bottom() {
            for x in area.x..area.right() {
                buffer[(x, y)].set_bg(background);
            }
        }
    }
}
