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
