use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::app::AppState;
use crate::layout::{PaneId, PaneInfo};
use crate::protocol::CursorState;
use crate::terminal::{TerminalId, TerminalRuntimeRegistry};

/// Percentage of the original color kept on unselected grid tiles. The
/// remainder blends toward the panel background so only the selected tile
/// renders at full strength.
const UNSELECTED_TILE_COLOR_PERCENT: u8 = 25;

/// One live agent terminal shown in the grid, in grid order.
pub(crate) struct AgentGridTarget {
    pub(crate) workspace_index: usize,
    pub(crate) pane_id: PaneId,
    pub(crate) terminal_id: TerminalId,
    label: String,
    state: crate::detect::AgentState,
    space_color: Color,
}

/// One positioned grid tile. `info.is_focused` marks the selected tile.
pub(crate) struct AgentGridTile {
    pub(crate) workspace_index: usize,
    pub(crate) terminal_id: TerminalId,
    pub(crate) info: PaneInfo,
    label: String,
    state: crate::detect::AgentState,
    space_color: Color,
}

fn is_live_agent(
    app: &AppState,
    terminal_runtimes: &TerminalRuntimeRegistry,
    workspace_index: usize,
    pane_id: PaneId,
    terminal_id: &TerminalId,
) -> bool {
    app.terminals
        .get(terminal_id)
        .is_some_and(crate::terminal::TerminalState::is_agent_terminal)
        && app
            .runtime_for_pane_in_workspace(terminal_runtimes, workspace_index, pane_id)
            .is_some()
}

/// Whether a pane belongs in the live agent grid: a detected agent terminal
/// with a running runtime that the user has not left out. Ordinary shell
/// panes never appear in the grid.
pub(crate) fn pane_in_agent_grid(
    app: &AppState,
    terminal_runtimes: &TerminalRuntimeRegistry,
    workspace_index: usize,
    pane_id: PaneId,
) -> bool {
    app.workspaces
        .get(workspace_index)
        .and_then(|workspace| workspace.pane_state(pane_id))
        .is_some_and(|pane| {
            !pane.agent_grid_excluded
                && is_live_agent(
                    app,
                    terminal_runtimes,
                    workspace_index,
                    pane_id,
                    &pane.attached_terminal_id,
                )
        })
}

fn space_presentation(app: &AppState) -> super::SpacePresentation {
    super::SpacePresentation::new(&app.palette, space_workspaces(app))
}

fn space_workspaces(app: &AppState) -> impl Iterator<Item = super::SpaceWorkspace> + '_ {
    app.workspaces
        .iter()
        .map(|workspace| super::SpaceWorkspace {
            color_slot: app
                .space(&workspace.space_id)
                .filter(|space| !space.is_other())
                .map(|space| space.color),
        })
}

/// Workspace indices in sidebar order: each space's members in its order,
/// then any workspace without a known space.
fn sidebar_workspace_order(app: &AppState) -> Vec<usize> {
    let mut listed = vec![false; app.workspaces.len()];
    let mut order = Vec::with_capacity(app.workspaces.len());
    for space in &app.spaces {
        for (index, workspace) in app.workspaces.iter().enumerate() {
            if !listed[index] && workspace.space_id == space.id {
                listed[index] = true;
                order.push(index);
            }
        }
    }
    order.extend((0..app.workspaces.len()).filter(|&index| !listed[index]));
    order
}

/// Whether some live agent is left out of the grid, which then may be empty
/// only because of the user's choice.
fn any_excluded_live_agent(app: &AppState, terminal_runtimes: &TerminalRuntimeRegistry) -> bool {
    app.workspaces
        .iter()
        .enumerate()
        .any(|(workspace_index, workspace)| {
            workspace.tabs.iter().any(|tab| {
                tab.panes.iter().any(|(&pane_id, pane)| {
                    pane.agent_grid_excluded
                        && is_live_agent(
                            app,
                            terminal_runtimes,
                            workspace_index,
                            pane_id,
                            &pane.attached_terminal_id,
                        )
                })
            })
        })
}

/// Every live agent the user has not left out, in space, workspace, tab, and
/// pane order. The order does not depend on agent status, so tiles never jump
/// when an agent changes state.
pub(crate) fn live_agent_targets(
    app: &AppState,
    terminal_runtimes: &TerminalRuntimeRegistry,
) -> Vec<AgentGridTarget> {
    let spaces = space_presentation(app);
    let mut targets = Vec::new();
    for workspace_index in sidebar_workspace_order(app) {
        let Some(workspace) = app.workspaces.get(workspace_index) else {
            continue;
        };
        let multi_tab = workspace.tabs.len() > 1;
        for (tab_index, tab) in workspace.tabs.iter().enumerate() {
            let tab_label = multi_tab
                .then(|| workspace.tab_display_name(tab_index))
                .flatten();
            for pane_id in tab.layout.pane_ids() {
                let Some(pane) = tab.panes.get(&pane_id) else {
                    continue;
                };
                if pane.agent_grid_excluded {
                    continue;
                }
                let terminal_id = &pane.attached_terminal_id;
                if !is_live_agent(
                    app,
                    terminal_runtimes,
                    workspace_index,
                    pane_id,
                    terminal_id,
                ) {
                    continue;
                }
                let Some(terminal) = app.terminals.get(terminal_id) else {
                    continue;
                };
                let agent_label = terminal
                    .effective_display_agent()
                    .or_else(|| terminal.agent_name.clone())
                    .or_else(|| terminal.effective_agent_label().map(str::to_string))
                    .unwrap_or_else(|| "agent".to_string());
                // The workspace label matches the sidebar; a pane's cwd can
                // point into another checkout and would mislabel the tile.
                let context = workspace.cached_display_name();
                let context = tab_label
                    .as_ref()
                    .map(|tab| format!("{context}/{tab}"))
                    .unwrap_or(context);
                targets.push(AgentGridTarget {
                    workspace_index,
                    pane_id,
                    terminal_id: terminal_id.clone(),
                    label: format!("{agent_label} · {context}"),
                    state: terminal.state,
                    space_color: spaces.color(workspace_index),
                });
            }
        }
    }
    targets
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

/// Lay out every live agent over `area`. `focused` selects the tile drawn at
/// full strength and given the host cursor. With `resize` set, each agent's
/// PTY is resized to its tile so the program redraws for the visible size.
pub(crate) fn compute_agent_grid(
    app: &AppState,
    terminal_runtimes: &TerminalRuntimeRegistry,
    area: Rect,
    focused: Option<(usize, PaneId)>,
    resize: Option<crate::kitty_graphics::HostCellSize>,
) -> Vec<AgentGridTile> {
    let targets = live_agent_targets(app, terminal_runtimes);
    let raw_infos = targets
        .iter()
        .zip(auto_tile_rects(area, targets.len()))
        .map(|(target, rect)| PaneInfo {
            id: target.pane_id,
            rect,
            inner_rect: rect,
            scrollbar_rect: None,
            borders: Borders::NONE,
            is_focused: focused == Some((target.workspace_index, target.pane_id)),
        })
        .collect::<Vec<_>>();
    let infos = super::apply_pane_chrome(
        raw_infos,
        crate::config::PaneBordersConfig::Always,
        false,
        true,
    );

    targets
        .into_iter()
        .zip(infos)
        .map(|(target, mut info)| {
            let pane_inner = super::pane_inner_rect(info.rect, info.borders);
            info.inner_rect = pane_inner;
            if let Some(runtime) = app.runtime_for_pane_in_workspace(
                terminal_runtimes,
                target.workspace_index,
                target.pane_id,
            ) {
                (info.inner_rect, info.scrollbar_rect) =
                    super::panes::stable_scrollbar_gutter(runtime, pane_inner, app.pane_scrollbars);
                if let Some(cell_size) = resize {
                    if info.inner_rect.width > 0
                        && info.inner_rect.height > 0
                        && !app.direct_attach_resize_locks.contains(&target.terminal_id)
                    {
                        runtime.resize(
                            info.inner_rect.height,
                            info.inner_rect.width,
                            cell_size.width_px,
                            cell_size.height_px,
                        );
                    }
                }
            }
            AgentGridTile {
                workspace_index: target.workspace_index,
                terminal_id: target.terminal_id,
                info,
                label: target.label,
                state: target.state,
                space_color: target.space_color,
            }
        })
        .collect()
}

fn state_color(state: crate::detect::AgentState, palette: &crate::app::state::Palette) -> Color {
    match state {
        crate::detect::AgentState::Working => palette.yellow,
        crate::detect::AgentState::Blocked => palette.red,
        crate::detect::AgentState::Idle => palette.green,
        crate::detect::AgentState::Unknown => palette.overlay0,
    }
}

pub(crate) fn render_agent_grid(
    app: &AppState,
    terminal_runtimes: &TerminalRuntimeRegistry,
    tiles: &[AgentGridTile],
    frame: &mut Frame,
    area: Rect,
) {
    if tiles.is_empty() {
        let message = if any_excluded_live_agent(app, terminal_runtimes) {
            "every agent is excluded from the grid"
        } else {
            "no live agents"
        };
        frame.render_widget(
            Paragraph::new(Line::from(message))
                .alignment(ratatui::layout::Alignment::Center)
                .style(Style::default().fg(app.palette.overlay0)),
            area,
        );
        return;
    }

    for tile in tiles {
        let selected = tile.info.is_focused;
        let tile_color = |color: Color| {
            if selected {
                color
            } else {
                super::mute_color(color, app.palette.panel_bg, UNSELECTED_TILE_COLOR_PERCENT)
            }
        };
        let border_style = Style::default().fg(tile_color(tile.space_color));
        let mut block = Block::default()
            .borders(tile.info.borders)
            .border_style(if selected {
                border_style.add_modifier(Modifier::BOLD)
            } else {
                border_style
            });
        let max_title_width = tile.info.rect.width.saturating_sub(4) as usize;
        if max_title_width > 0 {
            block = block.title(Line::styled(
                format!(" {} ", super::truncate_end(&tile.label, max_title_width)),
                Style::default().fg(tile_color(state_color(tile.state, &app.palette))),
            ));
        }
        frame.render_widget(block, tile.info.rect);

        if let Some(runtime) =
            app.runtime_for_pane_in_workspace(terminal_runtimes, tile.workspace_index, tile.info.id)
        {
            let show_cursor = selected
                && !super::pane_is_scrolled_back(runtime)
                && app.pane_exposes_host_cursor(tile.workspace_index, tile.info.id);
            runtime.render(frame, tile.info.inner_rect, show_cursor);
            super::scrollbar::render_pane_scrollbar(app, frame, &tile.info, runtime);
        }
    }
}

pub(crate) fn agent_grid_hyperlinks(
    app: &AppState,
    terminal_runtimes: &TerminalRuntimeRegistry,
    tiles: &[AgentGridTile],
) -> Vec<((u16, u16), String, String)> {
    let mut links = Vec::new();
    for tile in tiles {
        if let Some(runtime) =
            app.runtime_for_pane_in_workspace(terminal_runtimes, tile.workspace_index, tile.info.id)
        {
            links.extend(runtime.visible_hyperlinks(tile.info.inner_rect));
        }
    }
    links
}

pub(crate) fn agent_grid_cursor(
    app: &AppState,
    terminal_runtimes: &TerminalRuntimeRegistry,
    tiles: &[AgentGridTile],
) -> Option<CursorState> {
    let tile = tiles.iter().find(|tile| tile.info.is_focused)?;
    if !app.pane_exposes_host_cursor(tile.workspace_index, tile.info.id) {
        return None;
    }
    let runtime =
        app.runtime_for_pane_in_workspace(terminal_runtimes, tile.workspace_index, tile.info.id)?;
    if runtime.synchronized_output_active() {
        return None;
    }
    let scrolled_back = super::pane_is_scrolled_back(runtime);
    let reveal = app.reveal_hidden_cursor_for_cjk_ime
        && (!app.cjk_ime_agent_filter_configured
            || app
                .terminals
                .get(&tile.terminal_id)
                .and_then(|terminal| terminal.detected_agent)
                .is_some_and(|agent| app.cjk_ime_agents.contains(&agent)));

    if let Some(cursor) = runtime.cursor_state(tile.info.inner_rect, true) {
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
            x: tile.info.inner_rect.x,
            y: tile.info.inner_rect.y,
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
    use crate::{detect::Agent, workspace::Workspace};

    fn overlap(left: Rect, right: Rect) -> bool {
        left.x < right.x.saturating_add(right.width)
            && right.x < left.x.saturating_add(left.width)
            && left.y < right.y.saturating_add(right.height)
            && right.y < left.y.saturating_add(left.height)
    }

    pub(crate) fn shares_edge(left: Rect, right: Rect) -> bool {
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

    pub(crate) fn cross_workspace_agent_app() -> (AppState, PaneId, PaneId, PaneId) {
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
        (app, shell, first_agent, second_agent)
    }

    fn grid(app: &AppState, focused: Option<(usize, PaneId)>) -> Vec<AgentGridTile> {
        compute_agent_grid(
            app,
            &TerminalRuntimeRegistry::new(),
            Rect::new(0, 0, 120, 40),
            focused,
            None,
        )
    }

    #[tokio::test]
    async fn grid_projects_live_agents_across_workspaces_without_shells() {
        let (app, shell, first_agent, second_agent) = cross_workspace_agent_app();
        let tiles = grid(&app, None);
        assert_eq!(
            tiles.iter().map(|tile| tile.info.id).collect::<Vec<_>>(),
            vec![first_agent, second_agent]
        );
        assert!(!tiles.iter().any(|tile| tile.info.id == shell));
        assert!(pane_in_agent_grid(
            &app,
            &TerminalRuntimeRegistry::new(),
            1,
            second_agent
        ));
        assert!(!pane_in_agent_grid(
            &app,
            &TerminalRuntimeRegistry::new(),
            0,
            shell
        ));
    }

    #[tokio::test]
    async fn grid_leaves_out_excluded_agents_and_keeps_the_rest_in_order() {
        let (mut app, _, first_agent, second_agent) = cross_workspace_agent_app();
        app.workspaces[0]
            .pane_state_mut(first_agent)
            .expect("first agent pane")
            .agent_grid_excluded = true;

        let tiles = grid(&app, None);
        assert_eq!(
            tiles.iter().map(|tile| tile.info.id).collect::<Vec<_>>(),
            vec![second_agent]
        );
        assert_eq!(tiles[0].info.rect, Rect::new(0, 0, 120, 40));
        assert!(!pane_in_agent_grid(
            &app,
            &TerminalRuntimeRegistry::new(),
            0,
            first_agent
        ));
        assert!(pane_in_agent_grid(
            &app,
            &TerminalRuntimeRegistry::new(),
            1,
            second_agent
        ));
    }

    #[tokio::test]
    async fn grid_keeps_same_space_agents_adjacent_with_matching_colors() {
        fn agent_workspace(name: &str) -> (Workspace, PaneId) {
            let mut workspace = Workspace::test_new(name);
            workspace.id = name.into();
            let pane_id = workspace.tabs[0].root_pane;
            workspace.insert_test_runtime(
                pane_id,
                crate::terminal::TerminalRuntime::test_with_screen_bytes(20, 5, name.as_bytes()),
            );
            (workspace, pane_id)
        }

        let (issue, issue_pane) = agent_workspace("issue");
        let (notes, notes_pane) = agent_workspace("notes");
        let (main, main_pane) = agent_workspace("main");
        let mut app = AppState::test_new();
        app.workspaces = vec![issue, notes, main];
        app.ensure_test_terminals();
        app.normalize_spaces();
        let space = app.create_space("feature").expect("create space");
        app.assign_workspace_to_space("main", &space, None)
            .expect("assign main");
        app.assign_workspace_to_space("issue", &space, None)
            .expect("assign issue");
        for (workspace_id, pane_id) in [
            ("issue", issue_pane),
            ("notes", notes_pane),
            ("main", main_pane),
        ] {
            let ws_idx = app
                .workspaces
                .iter()
                .position(|workspace| workspace.id == workspace_id)
                .expect("workspace");
            let terminal_id = app.workspaces[ws_idx]
                .terminal_id(pane_id)
                .cloned()
                .expect("terminal id");
            app.terminals
                .get_mut(&terminal_id)
                .expect("terminal")
                .detected_agent = Some(Agent::Pi);
        }

        let tiles = grid(&app, None);
        assert_eq!(
            tiles.iter().map(|tile| tile.info.id).collect::<Vec<_>>(),
            vec![main_pane, issue_pane, notes_pane]
        );
        assert_eq!(tiles[0].space_color, tiles[1].space_color);
        assert_ne!(tiles[1].space_color, tiles[2].space_color);
        assert!(shares_edge(tiles[0].info.rect, tiles[1].info.rect));
    }

    #[tokio::test]
    async fn grid_order_does_not_change_with_agent_status() {
        let (mut app, _shell, first_agent, second_agent) = cross_workspace_agent_app();
        let before = grid(&app, None)
            .iter()
            .map(|tile| tile.info.id)
            .collect::<Vec<_>>();
        let terminal_id = app.workspaces[1]
            .terminal_id(second_agent)
            .cloned()
            .expect("second agent terminal");
        app.terminals.get_mut(&terminal_id).expect("terminal").state =
            crate::detect::AgentState::Blocked;
        let after = grid(&app, None)
            .iter()
            .map(|tile| tile.info.id)
            .collect::<Vec<_>>();
        assert_eq!(before, vec![first_agent, second_agent]);
        assert_eq!(after, before);
    }

    #[tokio::test]
    async fn grid_titles_name_the_agent_and_its_workspace_not_the_pane_cwd() {
        let (mut app, _shell, first_agent, _second_agent) = cross_workspace_agent_app();
        let terminal_id = app.workspaces[0]
            .terminal_id(first_agent)
            .cloned()
            .expect("first agent terminal");
        let terminal = app.terminals.get_mut(&terminal_id).expect("terminal");
        terminal.set_agent_name("planner".into());
        // An agent that wandered into another checkout still belongs to its workspace.
        terminal.cwd = "/projects/pyshiftup".into();
        app.workspaces[0].custom_name = Some("alpha".into());

        let labels = grid(&app, None)
            .into_iter()
            .map(|tile| tile.label)
            .collect::<Vec<_>>();
        assert_eq!(labels[0], "planner · alpha/review");
        assert!(labels[1].starts_with("claude · "), "{labels:?}");
    }

    #[tokio::test]
    async fn grid_renders_inactive_terminals_and_mutes_unselected_tiles() {
        let (app, _shell, first_agent, _second_agent) = cross_workspace_agent_app();
        let area = Rect::new(0, 0, 120, 40);
        let runtimes = TerminalRuntimeRegistry::new();
        let tiles = compute_agent_grid(&app, &runtimes, area, Some((0, first_agent)), None);
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(area.width, area.height))
                .unwrap();
        terminal
            .draw(|frame| render_agent_grid(&app, &runtimes, &tiles, frame, area))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let rendered = buffer
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(rendered.contains("PI-LIVE"), "{rendered}");
        assert!(rendered.contains("CLAUDE-LIVE"), "{rendered}");
        assert!(!rendered.contains("PLAIN-SHELL"), "{rendered}");

        assert_eq!(tiles.iter().filter(|tile| tile.info.is_focused).count(), 1);
        for tile in &tiles {
            let expected = if tile.info.is_focused {
                tile.space_color
            } else {
                crate::ui::mute_color(
                    tile.space_color,
                    app.palette.panel_bg,
                    UNSELECTED_TILE_COLOR_PERCENT,
                )
            };
            assert_eq!(buffer[(tile.info.rect.x, tile.info.rect.y)].fg, expected);
        }
    }

    #[tokio::test]
    async fn empty_grid_renders_an_empty_state() {
        let mut app = AppState::test_new();
        app.workspaces = vec![Workspace::test_new("shell")];
        app.ensure_test_terminals();
        let area = Rect::new(0, 0, 60, 10);
        let runtimes = TerminalRuntimeRegistry::new();
        let tiles = compute_agent_grid(&app, &runtimes, area, None, None);
        assert!(tiles.is_empty());
        assert!(agent_grid_cursor(&app, &runtimes, &tiles).is_none());
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(area.width, area.height))
                .unwrap();
        terminal
            .draw(|frame| render_agent_grid(&app, &runtimes, &tiles, frame, area))
            .unwrap();
        let rendered = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(rendered.contains("no live agents"));
    }
}
