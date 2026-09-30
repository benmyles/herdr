use super::*;

fn grid_snapshot() -> ClientShellSnapshot {
    let mut projected = snapshot();
    let mut second_tab = projected.tabs[0].clone();
    second_tab.tab_id = "tab_2".into();
    second_tab.number = 2;
    second_tab.label = "2".into();
    second_tab.focused = false;
    projected.tabs.push(second_tab);
    projected.panes.push(ClientShellPane {
        pane_id: "pane_2".into(),
        workspace_id: "ws_1".into(),
        tab_id: "tab_2".into(),
        label: None,
        cwd: Some("/repo".into()),
        foreground_cwd: Some("/repo".into()),
        focused: false,
        right_click_passthrough: false,
        agent_grid_excluded: false,
    });
    projected.agents = vec![ClientShellAgent {
        pane_id: "pane_2".into(),
        workspace_id: "ws_1".into(),
        tab_id: "tab_2".into(),
        name: None,
        display_agent: None,
        agent: Some("claude".into()),
        title: None,
        terminal_title: None,
        terminal_title_stripped: None,
        agent_status: AgentStatus::Working,
        state_change_seq: 1,
        state_labels: Vec::new(),
        tokens: Vec::new(),
        focused: false,
        state_changed_at_ms: None,
    }];
    projected
}

fn grid_state() -> ClientShellState {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(grid_snapshot()));
    state.set_pane_surface(surface());
    state.compose(106, 30).expect("shell frame");
    state
}

fn press(state: &mut ClientShellState, hit: impl Fn(&ShellHitMap) -> Rect) -> ClientShellInput {
    let rect = hit(&state.hits);
    state.handle_raw_events(vec![RawInputEvent::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: rect.x,
        row: rect.y,
        modifiers: KeyModifiers::empty(),
    })])
}

fn methods(outcome: &ClientShellInput) -> Vec<crate::api::schema::Method> {
    outcome
        .actions
        .iter()
        .filter_map(|action| match action {
            ClientShellAction::Endpoint { request, .. } => Some(request.method.clone()),
            _ => None,
        })
        .collect()
}

fn grid_set(active: bool) -> crate::api::schema::Method {
    crate::api::schema::Method::ClientShellAgentGridSet(
        crate::api::schema::ClientShellAgentGridSetParams { active },
    )
}

#[test]
fn agents_heading_toggles_the_grid_and_gives_it_the_tab_bar_row() {
    let mut state = grid_state();
    let tab_view = state.layout(106, 30);
    assert!(tab_view.tab_bar.height > 0);
    let toggle = state.hits.agent_grid_toggle;
    assert!(toggle.width > 0, "agents heading is clickable");
    assert!(
        toggle.right() <= state.hits.agent_sort_toggle.x,
        "heading never overlaps the sort control"
    );

    let open = press(&mut state, |hits| hits.agent_grid_toggle);
    assert_eq!(
        methods(&open),
        vec![
            grid_set(true),
            crate::api::schema::Method::PaneFocus(crate::api::schema::PaneTarget {
                pane_id: "pane_2".into(),
            }),
        ],
        "a focused shell hands keyboard focus to an agent in the grid"
    );
    assert!(open.resize);
    let grid_view = state.layout(106, 30);
    assert_eq!(grid_view.tab_bar, Rect::default());
    assert_eq!(
        grid_view.pane_surface.height,
        tab_view.pane_surface.height + tab_view.tab_bar.height
    );

    state.set_pane_surface(surface());
    let frame = state.compose(106, 30).expect("grid frame");
    let buffer = frame.to_ratatui_buffer().expect("grid buffer");
    let title = cell_symbol_position(&frame, state.hits.agent_grid_toggle, "agents");
    assert_eq!(buffer[title].fg, state.config.palette.accent);

    let close = press(&mut state, |hits| hits.agent_grid_toggle);
    assert_eq!(methods(&close), vec![grid_set(false)]);
    assert!(state.layout(106, 30).tab_bar.height > 0);
}

#[test]
fn workspace_navigation_and_collapsing_the_sidebar_leave_the_grid() {
    let mut state = grid_state();
    press(&mut state, |hits| hits.agent_grid_toggle);
    assert!(state.agent_grid_active());

    let mut outcome = ClientShellInput::default();
    state.push_endpoint_method(
        crate::api::schema::Method::WorkspaceFocus(crate::api::schema::WorkspaceTarget {
            workspace_id: "ws_1".into(),
        }),
        &mut outcome,
    );
    assert_eq!(
        methods(&outcome)[..1],
        [grid_set(false)],
        "the grid closes before the workspace view is selected"
    );
    assert!(!state.agent_grid_active());

    state.set_pane_surface(surface());
    state.compose(106, 30).expect("tab frame");
    press(&mut state, |hits| hits.agent_grid_toggle);
    assert!(state.agent_grid_active());
    let mut outcome = ClientShellInput::default();
    state.push_endpoint_method(
        crate::api::schema::Method::PaneFocus(crate::api::schema::PaneTarget {
            pane_id: "pane_2".into(),
        }),
        &mut outcome,
    );
    assert!(
        state.agent_grid_active(),
        "selecting an agent keeps the grid"
    );

    state.set_pane_surface(surface());
    state.compose(106, 30).expect("grid frame");
    let collapse = press(&mut state, |hits| hits.sidebar_toggle);
    assert_eq!(methods(&collapse), vec![grid_set(false)]);
    assert!(state.sidebar_collapsed);
    assert!(!state.agent_grid_active());
}

#[test]
fn mobile_width_closes_the_grid() {
    let mut state = grid_state();
    press(&mut state, |hits| hits.agent_grid_toggle);
    assert!(methods(&state.close_hidden_agent_grid(106)).is_empty());
    let narrow = state.config.mobile_width_threshold;
    assert_eq!(
        methods(&state.close_hidden_agent_grid(narrow)),
        vec![grid_set(false)]
    );
}

#[test]
fn grid_toggle_is_hidden_from_endpoints_without_the_method() {
    let mut state = grid_state();
    state.set_endpoint_methods(Some(vec!["pane.focus".into()]));
    state.set_pane_surface(surface());
    state.compose(106, 30).expect("frame");
    assert_eq!(state.hits.agent_grid_toggle, Rect::default());
}

#[test]
fn a_new_server_boot_starts_with_the_grid_closed() {
    let mut state = grid_state();
    press(&mut state, |hits| hits.agent_grid_toggle);
    assert!(state.agent_grid_active());

    let mut restarted = grid_snapshot();
    restarted.boot_id = "boot-2".into();
    state.set_snapshot(Box::new(restarted));
    assert!(!state.agent_grid_active());
    assert!(state.layout(106, 30).tab_bar.height > 0);
}

#[test]
fn agents_visible_in_the_grid_do_not_raise_attention_toasts() {
    let blocked_state = || {
        let mut projected = grid_snapshot();
        projected.agents[0].agent_status = AgentStatus::Blocked;
        let mut config = ClientShellConfig::from_config(&Config::default());
        config.toast_delivery = crate::config::ToastDelivery::Herdr;
        config.toast_delay_seconds = 0;
        let mut state = ClientShellState::new(config);
        state.set_snapshot(Box::new(projected));
        state.set_pane_surface(surface());
        state.compose(106, 30).expect("shell frame");
        state
    };
    let notify = |state: &mut ClientShellState| {
        state.receive_notification(
            &ClientEndpointId::Local,
            SemanticNotification {
                kind: SemanticNotificationKind::NeedsAttention,
                title: "claude needs attention".into(),
                body: None,
                sound: None,
                agent: Some("claude".into()),
                workspace_id: Some("ws_1".into()),
                tab_id: Some("tab_2".into()),
                pane_id: Some("pane_2".into()),
                position: None,
            },
            std::time::Instant::now(),
        );
        state.visible_notification.is_some()
    };

    let mut state = blocked_state();
    assert!(notify(&mut state), "a background tab agent raises a toast");

    let mut state = blocked_state();
    press(&mut state, |hits| hits.agent_grid_toggle);
    assert!(
        !notify(&mut state),
        "the grid already shows the blocked agent"
    );
}

#[test]
fn grid_tiles_are_titled_from_their_own_agent() {
    use ratatui::widgets::{Block, Widget};

    let mut projected = grid_snapshot();
    projected.panes.push(ClientShellPane {
        pane_id: "pane_3".into(),
        workspace_id: "ws_1".into(),
        tab_id: "tab_2".into(),
        label: None,
        cwd: Some("/repo".into()),
        foreground_cwd: Some("/repo".into()),
        focused: false,
        right_click_passthrough: false,
        agent_grid_excluded: false,
    });
    projected.agents[0].terminal_title_stripped = Some("fix the grid".into());
    let mut blocked = projected.agents[0].clone();
    blocked.pane_id = "pane_3".into();
    blocked.agent = Some("codex".into());
    blocked.agent_status = AgentStatus::Blocked;
    // Codex titles a waiting pane with its attention bracket.
    blocked.terminal_title_stripped = Some("[ ! ] Action needed".into());
    projected.agents.push(blocked);

    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(projected));
    state.set_pane_surface(surface());
    state.compose(106, 30).expect("shell frame");
    state.show_test_agent_grid();

    // The server draws bordered tiles; the codex tile comes first.
    let mut tiles = Buffer::empty(Rect::new(0, 0, 60, 6));
    Block::bordered()
        .title(" stale ")
        .render(Rect::new(0, 0, 30, 6), &mut tiles);
    Block::bordered().render(Rect::new(30, 0, 30, 6), &mut tiles);
    let tile = |pane_id: &str, x: u16, focused: bool| PaneSurfacePane {
        pane_id: pane_id.into(),
        content_revision: 0,
        rect: SurfaceRect {
            x,
            y: 0,
            width: 30,
            height: 6,
        },
        inner_rect: SurfaceRect {
            x: x + 1,
            y: 1,
            width: 28,
            height: 4,
        },
        scrollbar_rect: None,
        scroll: None,
        focused,
        mouse_reporting: false,
        sgr_pixel_mouse: false,
        alternate_screen_active: false,
        pixel_width: 0,
        pixel_height: 0,
    };
    let mut grid = surface();
    grid.frame = FrameData::from_ratatui_buffer_with_hyperlinks(&tiles, None, &[]);
    grid.panes = vec![tile("pane_3", 0, false), tile("pane_2", 30, true)];
    state.set_pane_surface(grid);

    let frame = state.compose(106, 30).expect("grid frame");
    let origin = state.layout(106, 30).pane_surface;
    let rows = frame_rows(&frame);
    let title_row = rows[origin.y as usize].chars().collect::<Vec<_>>();
    let tile_title = |x: u16| {
        title_row[(origin.x + x) as usize..(origin.x + x + 30) as usize]
            .iter()
            .collect::<String>()
    };
    let codex = tile_title(0);
    let claude = tile_title(30);
    assert!(codex.starts_with("┌ Λ ? Action needed"), "{codex}");
    assert!(!codex.contains("stale"), "{codex}");
    assert!(claude.starts_with("┌ § "), "{claude}");
    assert!(claude.contains("fix the grid"), "{claude}");
    assert!(!claude.contains("Action"), "{claude}");
    assert!(claude.trim_end().ends_with("─┐"), "{claude}");
}

/// Two agents in the second tab; `pane_2` is focused.
fn two_agent_state(excluded: &[&str]) -> ClientShellState {
    let mut projected = grid_snapshot();
    let mut second = projected.panes[1].clone();
    second.pane_id = "pane_3".into();
    projected.panes.push(second);
    let mut agent = projected.agents[0].clone();
    agent.pane_id = "pane_3".into();
    agent.agent = Some("codex".into());
    projected.agents.push(agent);
    projected.focused_tab_id = Some("tab_2".into());
    projected.focused_pane_id = Some("pane_2".into());
    for pane in &mut projected.panes {
        pane.focused = pane.pane_id == "pane_2";
        pane.agent_grid_excluded = excluded.contains(&pane.pane_id.as_str());
    }
    projected.agents[0].focused = true;
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(projected));
    state.set_pane_surface(surface());
    state.compose(106, 30).expect("shell frame");
    state
}

fn agent_row(state: &ClientShellState, pane_id: &str) -> Rect {
    state
        .hits
        .agents
        .iter()
        .map(|(rect, id)| (rect, id))
        .chain(
            state
                .hits
                .endpoint_agents
                .iter()
                .map(|(rect, _, id)| (rect, id)),
        )
        .find(|(_, id)| id.as_str() == pane_id)
        .map(|(rect, _)| *rect)
        .expect("agent row")
}

fn right_click(state: &mut ClientShellState, rect: Rect) -> ClientShellInput {
    state.handle_raw_events(vec![RawInputEvent::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Right),
        column: rect.x + rect.width / 2,
        row: rect.y,
        modifiers: KeyModifiers::empty(),
    })])
}

fn menu_labels(state: &ClientShellState) -> Vec<&'static str> {
    match &state.overlay {
        Some(ClientShellOverlay::ContextMenu(menu)) => {
            menu.items().iter().map(|item| item.label).collect()
        }
        _ => Vec::new(),
    }
}

fn activate_menu_item(state: &mut ClientShellState, label: &str) -> ClientShellInput {
    let index = menu_labels(state)
        .iter()
        .position(|item| *item == label)
        .unwrap_or_else(|| panic!("menu offers {label:?}: {:?}", menu_labels(state)));
    let mut outcome = ClientShellInput::default();
    state.activate_context_menu_item(index, &mut outcome);
    outcome
}

fn grid_excluded(pane_id: &str, excluded: bool) -> crate::api::schema::Method {
    crate::api::schema::Method::PaneAgentGridSet(crate::api::schema::PaneAgentGridSetParams {
        pane_id: pane_id.into(),
        excluded,
    })
}

fn pane_focus(pane_id: &str) -> crate::api::schema::Method {
    crate::api::schema::Method::PaneFocus(crate::api::schema::PaneTarget {
        pane_id: pane_id.into(),
    })
}

/// Whether the agent row starts with the grid rail in the accent color.
fn has_grid_rail(state: &mut ClientShellState, pane_id: &str) -> bool {
    state.set_pane_surface(surface());
    let frame = state.compose(106, 30).expect("frame");
    let buffer = frame.to_ratatui_buffer().expect("buffer");
    let row = agent_row(state, pane_id);
    let cell = &buffer[(row.x, row.y)];
    cell.symbol() == "▎" && cell.fg == state.config.palette.accent
}

#[test]
fn right_clicking_an_agent_row_excludes_it_and_moves_the_grid_selection() {
    let mut state = two_agent_state(&[]);
    state.show_test_agent_grid();
    state.set_pane_surface(surface());
    state.compose(106, 30).expect("grid frame");

    let row = agent_row(&state, "pane_2");
    assert!(right_click(&mut state, row).repaint);
    assert_eq!(menu_labels(&state), vec!["Exclude from grid"]);
    assert_eq!(
        methods(&activate_menu_item(&mut state, "Exclude from grid")),
        vec![grid_excluded("pane_2", true), pane_focus("pane_3")],
        "the selected tile left, so keyboard input moves to a tile still shown"
    );
    assert!(state.agent_grid_active());

    let mut state = two_agent_state(&["pane_2"]);
    state.show_test_agent_grid();
    state.set_pane_surface(surface());
    state.compose(106, 30).expect("grid frame");
    let row = agent_row(&state, "pane_2");
    right_click(&mut state, row);
    assert_eq!(menu_labels(&state), vec!["Include in grid"]);
    assert_eq!(
        methods(&activate_menu_item(&mut state, "Include in grid")),
        vec![grid_excluded("pane_2", false)]
    );
}

#[test]
fn excluding_an_agent_outside_the_grid_leaves_focus_alone() {
    let mut state = two_agent_state(&[]);
    let row = agent_row(&state, "pane_2");
    right_click(&mut state, row);
    assert_eq!(
        methods(&activate_menu_item(&mut state, "Exclude from grid")),
        vec![grid_excluded("pane_2", true)]
    );
}

#[test]
fn only_agents_shown_in_the_grid_carry_the_rail() {
    let mut state = two_agent_state(&["pane_2"]);
    assert!(
        !has_grid_rail(&mut state, "pane_3"),
        "no rail without a grid"
    );

    state.show_test_agent_grid();
    assert!(has_grid_rail(&mut state, "pane_3"));
    assert!(!has_grid_rail(&mut state, "pane_2"));
}

#[test]
fn opening_the_grid_skips_an_excluded_focused_agent() {
    let mut state = two_agent_state(&["pane_2"]);
    let open = press(&mut state, |hits| hits.agent_grid_toggle);
    assert_eq!(methods(&open), vec![grid_set(true), pane_focus("pane_3")]);
}

#[test]
fn selecting_an_excluded_agent_leaves_the_grid_for_its_tab() {
    let mut state = two_agent_state(&["pane_3"]);
    state.show_test_agent_grid();
    state.set_pane_surface(surface());
    state.compose(106, 30).expect("grid frame");

    let row = agent_row(&state, "pane_3");
    let select = press(&mut state, move |_| row);
    assert_eq!(
        methods(&select),
        vec![grid_set(false), pane_focus("pane_3")]
    );
    assert!(!state.agent_grid_active());
}

#[test]
fn agent_cycling_in_the_grid_skips_excluded_agents() {
    use crate::input::KeybindAction;

    let mut state = two_agent_state(&["pane_3"]);
    assert_eq!(
        state.endpoint_method_for_action(KeybindAction::NextAgent),
        Some(pane_focus("pane_3")),
        "without a grid every agent takes a turn"
    );
    state.show_test_agent_grid();
    assert_eq!(
        state.endpoint_method_for_action(KeybindAction::NextAgent),
        Some(pane_focus("pane_2"))
    );
}

#[test]
fn grid_tiles_offer_exclusion_first_in_their_pane_menu() {
    let mut state = two_agent_state(&[]);
    state.open_pane_context_menu("pane_3".into(), 40, 10);
    assert!(!menu_labels(&state).contains(&"Exclude from grid"));

    state.show_test_agent_grid();
    state.open_pane_context_menu("pane_3".into(), 40, 10);
    assert_eq!(menu_labels(&state)[0], "Exclude from grid");
    assert_eq!(
        methods(&activate_menu_item(&mut state, "Exclude from grid")),
        vec![grid_excluded("pane_3", true)],
        "the selected tile stays when another tile leaves"
    );
}

#[test]
fn endpoints_without_grid_exclusion_open_no_agent_menu() {
    let mut state = two_agent_state(&[]);
    state.set_endpoint_methods(Some(vec![
        "pane.focus".into(),
        "client_shell.agent_grid.set".into(),
    ]));
    state.set_pane_surface(surface());
    state.compose(106, 30).expect("frame");
    let row = agent_row(&state, "pane_2");
    right_click(&mut state, row);
    assert!(state.overlay.is_none());

    state.show_test_agent_grid();
    state.open_pane_context_menu("pane_2".into(), 40, 10);
    assert!(!menu_labels(&state).contains(&"Exclude from grid"));
}
