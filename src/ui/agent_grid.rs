use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::Line,
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::protocol::CursorState;
use crate::{
    app::state::AgentGridPaneInfo,
    app::{AppState, Mode},
    layout::PaneInfo,
    terminal::{TerminalId, TerminalRuntimeRegistry},
};

struct LiveAgentTarget {
    ws_idx: usize,
    pane_id: crate::layout::PaneId,
    terminal_id: TerminalId,
    label: String,
    state: crate::detect::AgentState,
    space_color: ratatui::style::Color,
}

/// Deterministically tile `count` surfaces along an edge-connected snake path.
///
/// Candidate row counts are scored against a roughly square physical terminal
/// tile (terminal cells are commonly about twice as tall as they are wide).
/// Rows differ by at most one tile and every returned rectangle stays inside
/// `area`. Reversing alternate rows keeps adjacent items physically adjacent
/// across row boundaries, which lets each space form one connected tile group.
/// Zero-sized rectangles are possible only when the area cannot hold one cell
/// per requested tile.
pub(crate) fn auto_tile_rects(area: Rect, count: usize) -> Vec<Rect> {
    if count == 0 {
        return Vec::new();
    }

    let rows = best_row_count(area, count);
    let row_lengths = split_lengths(area.height, rows);
    let base_columns = count / rows;
    let extra_columns = count % rows;
    let mut rects = Vec::with_capacity(count);
    let mut y = area.y;

    for (row_idx, row_height) in row_lengths.into_iter().enumerate() {
        let columns = base_columns + usize::from(row_idx < extra_columns);
        if columns == 0 {
            continue;
        }
        let column_widths = split_lengths(area.width, columns);
        let mut x = area.x;
        let mut row_rects = Vec::with_capacity(columns);
        for width in column_widths {
            row_rects.push(Rect::new(x, y, width, row_height));
            x = x.saturating_add(width);
        }
        if row_idx % 2 == 1 {
            row_rects.reverse();
        }
        rects.extend(row_rects);
        y = y.saturating_add(row_height);
    }

    rects.truncate(count);
    rects
}

fn best_row_count(area: Rect, count: usize) -> usize {
    let mut best = 1usize;
    let mut best_score = u128::MAX;
    let capacity_is_sufficient =
        usize::from(area.width).saturating_mul(usize::from(area.height)) >= count;

    for rows in 1..=count {
        let columns = count.div_ceil(rows);
        if capacity_is_sufficient
            && (rows > usize::from(area.height) || columns > usize::from(area.width))
        {
            continue;
        }

        let tile_width = usize::from(area.width) / columns.max(1);
        let tile_height = usize::from(area.height) / rows.max(1);
        let aspect_error = tile_width.abs_diff(tile_height.saturating_mul(2));
        let ragged = usize::from(!count.is_multiple_of(rows));
        let score = (aspect_error as u128)
            .saturating_mul(count as u128)
            .saturating_add(
                (ragged as u128).saturating_mul(tile_width.min(tile_height).max(1) as u128),
            );

        if score < best_score {
            best = rows;
            best_score = score;
        }
    }

    best
}

fn split_lengths(total: u16, parts: usize) -> Vec<u16> {
    if parts == 0 {
        return Vec::new();
    }
    let total = usize::from(total);
    let base = total / parts;
    let extra = total % parts;
    (0..parts)
        .map(|idx| (base + usize::from(idx < extra)).min(u16::MAX as usize) as u16)
        .collect()
}

fn live_agent_targets(
    app: &AppState,
    terminal_runtimes: &TerminalRuntimeRegistry,
    spaces: &super::space_colors::SpacePresentation,
) -> Vec<LiveAgentTarget> {
    let mut targets = Vec::new();
    for &ws_idx in spaces.workspace_order() {
        let Some(workspace) = app.workspaces.get(ws_idx) else {
            continue;
        };
        let workspace_label = workspace.display_name_from_terminals(&app.terminals);
        let multi_tab = workspace.tabs.len() > 1;
        for (tab_idx, tab) in workspace.tabs.iter().enumerate() {
            let tab_label = multi_tab
                .then(|| workspace.tab_display_name(tab_idx))
                .flatten();
            for pane_id in tab.layout.pane_ids() {
                let Some(pane) = tab.panes.get(&pane_id) else {
                    continue;
                };
                let Some(terminal) = app.terminals.get(&pane.attached_terminal_id) else {
                    continue;
                };
                if !terminal.is_agent_terminal()
                    || app
                        .runtime_for_pane_in_workspace(terminal_runtimes, ws_idx, pane_id)
                        .is_none()
                {
                    continue;
                }

                let agent_label = terminal
                    .effective_display_agent()
                    .or_else(|| terminal.effective_agent_label().map(str::to_string))
                    .unwrap_or_else(|| "agent".to_string());
                let context = tab_label
                    .as_ref()
                    .map(|tab| format!("{workspace_label}/{tab}"))
                    .unwrap_or_else(|| workspace_label.clone());
                targets.push(LiveAgentTarget {
                    ws_idx,
                    pane_id,
                    terminal_id: pane.attached_terminal_id.clone(),
                    label: format!("{agent_label} · {context}"),
                    state: terminal.state,
                    space_color: spaces.color(ws_idx),
                });
            }
        }
    }
    targets
}

pub(super) fn compute_agent_grid(
    app: &mut AppState,
    terminal_runtimes: &TerminalRuntimeRegistry,
    area: Rect,
    resize_panes: bool,
    cell_size: crate::kitty_graphics::HostCellSize,
) -> Vec<AgentGridPaneInfo> {
    let spaces = super::space_colors::SpacePresentation::new(app);
    let targets = live_agent_targets(app, terminal_runtimes, &spaces);
    let selected_is_live = app
        .agent_grid_selected_terminal
        .as_ref()
        .is_some_and(|selected| targets.iter().any(|target| &target.terminal_id == selected));
    if !selected_is_live {
        let focused_terminal = app.active.and_then(|ws_idx| {
            let workspace = app.workspaces.get(ws_idx)?;
            let pane_id = workspace.focused_pane_id()?;
            workspace.terminal_id(pane_id).cloned()
        });
        app.agent_grid_selected_terminal = focused_terminal
            .filter(|terminal_id| {
                targets
                    .iter()
                    .any(|target| &target.terminal_id == terminal_id)
            })
            .or_else(|| targets.first().map(|target| target.terminal_id.clone()));
    }

    let selected = app.agent_grid_selected_terminal.as_ref();
    let rects = auto_tile_rects(area, targets.len());
    let raw_infos = targets
        .iter()
        .zip(rects)
        .map(|(target, rect)| PaneInfo {
            id: target.pane_id,
            rect,
            inner_rect: rect,
            scrollbar_rect: None,
            borders: Borders::NONE,
            is_focused: selected == Some(&target.terminal_id),
        })
        .collect::<Vec<_>>();
    let pane_infos = super::panes::apply_pane_chrome(raw_infos, true, false, true);

    targets
        .into_iter()
        .zip(pane_infos)
        .map(|(target, mut pane_info)| {
            let pane_inner = super::panes::pane_inner_rect(pane_info.rect, pane_info.borders);
            pane_info.inner_rect = pane_inner;
            if let Some(runtime) =
                app.runtime_for_pane_in_workspace(terminal_runtimes, target.ws_idx, target.pane_id)
            {
                (pane_info.inner_rect, pane_info.scrollbar_rect) =
                    super::panes::stable_scrollbar_gutter(runtime, pane_inner, app.pane_scrollbars);
                if resize_panes
                    && pane_info.inner_rect.width > 0
                    && pane_info.inner_rect.height > 0
                    && !app.direct_attach_resize_locks.contains(&target.terminal_id)
                {
                    runtime.resize(
                        pane_info.inner_rect.height,
                        pane_info.inner_rect.width,
                        cell_size.width_px,
                        cell_size.height_px,
                    );
                }
            }
            AgentGridPaneInfo {
                ws_idx: target.ws_idx,
                terminal_id: target.terminal_id,
                label: target.label,
                state: target.state,
                space_color: target.space_color,
                pane_info,
            }
        })
        .collect()
}

pub(super) fn render_agent_grid(
    app: &AppState,
    terminal_runtimes: &TerminalRuntimeRegistry,
    frame: &mut Frame,
    area: Rect,
) {
    if app.view.agent_grid_panes.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from("no live agents"))
                .alignment(ratatui::layout::Alignment::Center)
                .style(Style::default().fg(app.palette.overlay0)),
            area,
        );
        return;
    }

    let terminal_active = app.mode == Mode::Terminal;
    for grid in &app.view.agent_grid_panes {
        let border_color = grid.space_color;
        let max_title_width = grid.pane_info.rect.width.saturating_sub(4) as usize;
        let title = (max_title_width > 0).then(|| {
            Line::styled(
                format!(
                    " {} ",
                    super::text::truncate_end(&grid.label, max_title_width)
                ),
                Style::default().fg(super::status::state_label_color(
                    grid.state,
                    true,
                    &app.palette,
                )),
            )
        });
        let mut block = Block::default()
            .borders(grid.pane_info.borders)
            .border_style(Style::default().fg(border_color));
        if grid.pane_info.is_focused {
            block = block.border_style(
                Style::default()
                    .fg(border_color)
                    .add_modifier(Modifier::BOLD),
            );
        }
        if let Some(title) = title {
            block = block.title(title);
        }
        frame.render_widget(block, grid.pane_info.rect);
        super::panes::render_projected_pane(
            app,
            terminal_runtimes,
            frame,
            grid.ws_idx,
            &grid.pane_info,
            true,
            terminal_active,
        );
    }
}

pub(crate) fn agent_grid_hyperlinks(
    app: &AppState,
    terminal_runtimes: &TerminalRuntimeRegistry,
) -> Vec<((u16, u16), String, String)> {
    let mut links = Vec::new();
    for grid in &app.view.agent_grid_panes {
        if let Some(runtime) =
            app.runtime_for_pane_in_workspace(terminal_runtimes, grid.ws_idx, grid.pane_info.id)
        {
            links.extend(runtime.visible_hyperlinks(grid.pane_info.inner_rect));
        }
    }
    links
}

pub(crate) fn agent_grid_cursor(
    app: &AppState,
    terminal_runtimes: &TerminalRuntimeRegistry,
) -> Option<CursorState> {
    if app.mode != Mode::Terminal {
        return None;
    }
    let grid = app.agent_grid_selected_pane()?;
    if !app.pane_exposes_host_cursor(grid.ws_idx, grid.pane_info.id) {
        return None;
    }
    let runtime =
        app.runtime_for_pane_in_workspace(terminal_runtimes, grid.ws_idx, grid.pane_info.id)?;
    if runtime.synchronized_output_active() {
        return None;
    }
    let scrolled_back = super::panes::pane_is_scrolled_back(runtime);
    let reveal = app.reveal_hidden_cursor_for_cjk_ime
        && (!app.cjk_ime_agent_filter_configured
            || app
                .terminals
                .get(&grid.terminal_id)
                .and_then(|terminal| terminal.detected_agent)
                .is_some_and(|agent| app.cjk_ime_agents.contains(&agent)));

    if let Some(cursor) = runtime.cursor_state(grid.pane_info.inner_rect, true) {
        let visible = if reveal {
            !scrolled_back
        } else {
            cursor.visible && !scrolled_back
        };
        Some(CursorState {
            x: cursor.x,
            y: cursor.y,
            visible,
            shape: if reveal && visible {
                app.cjk_ime_cursor_shape
            } else {
                cursor.shape
            },
        })
    } else if reveal && !scrolled_back {
        Some(CursorState {
            x: grid.pane_info.inner_rect.x,
            y: grid.pane_info.inner_rect.y,
            visible: true,
            shape: app.cjk_ime_cursor_shape,
        })
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{app::state::MainSurface, detect::Agent, workspace::Workspace};
    use ratatui::{backend::TestBackend, Terminal};

    fn overlap(left: Rect, right: Rect) -> bool {
        left.x < right.x.saturating_add(right.width)
            && right.x < left.x.saturating_add(left.width)
            && left.y < right.y.saturating_add(right.height)
            && right.y < left.y.saturating_add(left.height)
    }

    fn shares_edge(left: Rect, right: Rect) -> bool {
        let horizontal_overlap = left.x < right.x.saturating_add(right.width)
            && right.x < left.x.saturating_add(left.width);
        let vertical_overlap = left.y < right.y.saturating_add(right.height)
            && right.y < left.y.saturating_add(left.height);
        (horizontal_overlap
            && (left.y.saturating_add(left.height) == right.y
                || right.y.saturating_add(right.height) == left.y))
            || (vertical_overlap
                && (left.x.saturating_add(left.width) == right.x
                    || right.x.saturating_add(right.width) == left.x))
    }

    #[test]
    fn auto_tile_geometry_is_deterministic_contained_and_disjoint() {
        let area = Rect::new(7, 3, 120, 40);
        for count in [0, 1, 2, 3, 4, 5, 15, 50] {
            let first = auto_tile_rects(area, count);
            let second = auto_tile_rects(area, count);
            assert_eq!(first, second, "count={count}");
            assert_eq!(first.len(), count, "count={count}");

            for (idx, rect) in first.iter().enumerate() {
                assert!(rect.width > 0 && rect.height > 0, "count={count} idx={idx}");
                assert!(
                    rect.x >= area.x && rect.y >= area.y,
                    "count={count} idx={idx}"
                );
                assert!(
                    rect.x.saturating_add(rect.width) <= area.x.saturating_add(area.width),
                    "count={count} idx={idx}"
                );
                assert!(
                    rect.y.saturating_add(rect.height) <= area.y.saturating_add(area.height),
                    "count={count} idx={idx}"
                );
            }

            for left in 0..first.len() {
                for right in left + 1..first.len() {
                    assert!(
                        !overlap(first[left], first[right]),
                        "count={count} left={left} right={right}"
                    );
                }
            }
        }
    }

    #[test]
    fn auto_tile_geometry_uses_balanced_rows_for_common_counts() {
        let area = Rect::new(0, 0, 120, 40);
        let rows = |count| {
            let rects = auto_tile_rects(area, count);
            let mut ys = rects.iter().map(|rect| rect.y).collect::<Vec<_>>();
            ys.sort_unstable();
            ys.dedup();
            ys.len()
        };

        assert_eq!(rows(1), 1);
        assert_eq!(rows(2), 1);
        assert_eq!(rows(3), 2);
        assert_eq!(rows(4), 2);
        assert_eq!(rows(5), 2);
        assert_eq!(rows(15), 3);
    }

    #[test]
    fn auto_tile_sequence_stays_edge_connected_across_rows() {
        let area = Rect::new(0, 0, 120, 40);
        for count in 2..=50 {
            let rects = auto_tile_rects(area, count);
            assert!(
                rects.windows(2).all(|pair| shares_edge(pair[0], pair[1])),
                "count={count} rects={rects:?}"
            );
        }
    }

    #[test]
    fn auto_tile_geometry_is_safe_for_impossibly_tiny_areas() {
        for area in [
            Rect::new(2, 4, 0, 0),
            Rect::new(2, 4, 1, 1),
            Rect::new(2, 4, 2, 1),
        ] {
            let rects = auto_tile_rects(area, 5);
            assert_eq!(rects.len(), 5);
            assert!(rects.iter().all(|rect| {
                rect.x >= area.x
                    && rect.y >= area.y
                    && rect.x.saturating_add(rect.width) <= area.x.saturating_add(area.width)
                    && rect.y.saturating_add(rect.height) <= area.y.saturating_add(area.height)
            }));
        }
    }

    fn cross_workspace_agent_app() -> (
        AppState,
        crate::layout::PaneId,
        crate::layout::PaneId,
        crate::layout::PaneId,
    ) {
        let mut first = Workspace::test_new("one");
        let shell = first.tabs[0].root_pane;
        first.insert_test_runtime(
            shell,
            crate::terminal::TerminalRuntime::test_with_screen_bytes(20, 5, b"PLAIN-SHELL"),
        );
        let agent_tab = first.test_add_tab(Some("review"));
        let first_agent = first.tabs[agent_tab].root_pane;
        first.insert_test_runtime(
            first_agent,
            crate::terminal::TerminalRuntime::test_with_screen_bytes(20, 5, b"PI-LIVE"),
        );

        let mut second = Workspace::test_new("two");
        let second_agent = second.tabs[0].root_pane;
        second.insert_test_runtime(
            second_agent,
            crate::terminal::TerminalRuntime::test_with_screen_bytes(20, 5, b"CLAUDE-LIVE"),
        );

        let mut app = AppState::test_new();
        app.workspaces = vec![first, second];
        app.ensure_test_terminals();
        for (ws_idx, pane_id, agent) in [
            (0, first_agent, Agent::Pi),
            (1, second_agent, Agent::Claude),
        ] {
            let terminal_id = app.workspaces[ws_idx]
                .terminal_id(pane_id)
                .cloned()
                .expect("terminal id");
            app.terminals
                .get_mut(&terminal_id)
                .expect("terminal state")
                .detected_agent = Some(agent);
        }
        app.active = Some(0);
        app.selected = 0;
        app.mode = Mode::Terminal;
        app.main_surface = MainSurface::LiveAgents;
        app.session_dirty = false;
        (app, shell, first_agent, second_agent)
    }

    #[tokio::test]
    async fn agent_grid_keeps_same_space_agents_adjacent_with_matching_colors() {
        fn agent_workspace(
            name: &str,
            space: Option<(&str, bool)>,
        ) -> (Workspace, crate::layout::PaneId) {
            let mut workspace = Workspace::test_new(name);
            if let Some((key, linked)) = space {
                workspace.worktree_space = Some(crate::workspace::WorktreeSpaceMembership {
                    key: key.into(),
                    label: key.into(),
                    repo_root: format!("/repo/{key}").into(),
                    checkout_path: format!("/repo/{name}").into(),
                    is_linked_worktree: linked,
                });
            }
            let pane_id = workspace.tabs[0].root_pane;
            workspace.insert_test_runtime(
                pane_id,
                crate::terminal::TerminalRuntime::test_with_screen_bytes(20, 5, name.as_bytes()),
            );
            (workspace, pane_id)
        }

        let (issue, issue_pane) = agent_workspace("issue", Some(("repo", true)));
        let (notes, notes_pane) = agent_workspace("notes", None);
        let (main, main_pane) = agent_workspace("main", Some(("repo", false)));
        let mut app = AppState::test_new();
        app.workspaces = vec![issue, notes, main];
        app.ensure_test_terminals();
        for (ws_idx, pane_id) in [(0, issue_pane), (1, notes_pane), (2, main_pane)] {
            let terminal_id = app.workspaces[ws_idx]
                .terminal_id(pane_id)
                .cloned()
                .expect("terminal id");
            app.terminals
                .get_mut(&terminal_id)
                .expect("terminal")
                .detected_agent = Some(Agent::Pi);
        }
        app.active = Some(0);
        app.main_surface = MainSurface::LiveAgents;

        crate::ui::compute_view(&mut app, Rect::new(0, 0, 120, 40));

        let panes = &app.view.agent_grid_panes;
        assert_eq!(
            panes
                .iter()
                .map(|pane| pane.pane_info.id)
                .collect::<Vec<_>>(),
            vec![main_pane, issue_pane, notes_pane]
        );
        assert_eq!(panes[0].space_color, panes[1].space_color);
        assert_ne!(panes[1].space_color, panes[2].space_color);
        assert!(shares_edge(
            panes[0].pane_info.rect,
            panes[1].pane_info.rect
        ));
    }

    #[tokio::test]
    async fn agent_grid_projects_live_agents_without_mutating_workspace_layouts() {
        let (mut app, shell, first_agent, second_agent) = cross_workspace_agent_app();
        let active_tabs = app
            .workspaces
            .iter()
            .map(|workspace| workspace.active_tab)
            .collect::<Vec<_>>();
        let pane_orders = app
            .workspaces
            .iter()
            .flat_map(|workspace| &workspace.tabs)
            .map(|tab| tab.layout.pane_ids())
            .collect::<Vec<_>>();

        crate::ui::compute_view(&mut app, Rect::new(0, 0, 120, 40));

        assert_eq!(
            app.view
                .agent_grid_panes
                .iter()
                .map(|pane| pane.pane_info.id)
                .collect::<Vec<_>>(),
            vec![first_agent, second_agent]
        );
        assert!(!app
            .view
            .agent_grid_panes
            .iter()
            .any(|pane| pane.pane_info.id == shell));
        assert!(app.view.pane_infos.is_empty());
        assert_eq!(
            app.workspaces
                .iter()
                .map(|workspace| workspace.active_tab)
                .collect::<Vec<_>>(),
            active_tabs
        );
        assert_eq!(
            app.workspaces
                .iter()
                .flat_map(|workspace| &workspace.tabs)
                .map(|tab| tab.layout.pane_ids())
                .collect::<Vec<_>>(),
            pane_orders
        );
        assert!(!app.session_dirty);
        app.assert_invariants_for_test();
    }

    #[tokio::test]
    async fn agent_grid_renders_terminals_from_inactive_tabs_and_workspaces() {
        let (mut app, _shell, _first_agent, _second_agent) = cross_workspace_agent_app();
        let area = Rect::new(0, 0, 120, 40);
        crate::ui::compute_view(&mut app, area);
        let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
        terminal
            .draw(|frame| crate::ui::render(&app, frame))
            .unwrap();
        let rendered = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();

        assert!(rendered.contains("PI-LIVE"), "{rendered}");
        assert!(rendered.contains("CLAUDE-LIVE"), "{rendered}");
        assert!(!rendered.contains("PLAIN-SHELL"), "{rendered}");
    }

    #[tokio::test]
    async fn agent_grid_order_does_not_change_with_agent_status() {
        let (mut app, _shell, first_agent, second_agent) = cross_workspace_agent_app();
        crate::ui::compute_view(&mut app, Rect::new(0, 0, 120, 40));
        let before = app
            .view
            .agent_grid_panes
            .iter()
            .map(|pane| pane.pane_info.id)
            .collect::<Vec<_>>();
        let terminal_id = app.workspaces[1]
            .terminal_id(second_agent)
            .cloned()
            .expect("second agent terminal");
        app.terminals.get_mut(&terminal_id).expect("terminal").state =
            crate::detect::AgentState::Blocked;

        crate::ui::compute_view(&mut app, Rect::new(0, 0, 120, 40));
        let after = app
            .view
            .agent_grid_panes
            .iter()
            .map(|pane| pane.pane_info.id)
            .collect::<Vec<_>>();
        assert_eq!(before, vec![first_agent, second_agent]);
        assert_eq!(after, before);
    }

    #[tokio::test]
    async fn app_surface_visibility_contains_grid_agents_but_not_shells() {
        let (app, shell, first_agent, second_agent) = cross_workspace_agent_app();
        let visible = app.app_surface_pane_ids();
        assert_eq!(visible.len(), 2);
        assert!(visible.contains(&first_agent));
        assert!(visible.contains(&second_agent));
        assert!(!visible.contains(&shell));
    }
}
