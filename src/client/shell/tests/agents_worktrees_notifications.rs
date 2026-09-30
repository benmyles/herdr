use super::*;

#[test]
fn mouse_hits_use_stable_workspace_tab_and_pane_ids() {
    let config = ClientShellConfig::from_config(&Config::default());
    let mut state = ClientShellState::new(config);
    state.set_snapshot(Box::new(snapshot()));
    state.set_pane_surface(surface());
    state.compose(106, 20).expect("composed frame");

    let workspace_down =
        state.handle_raw_events(vec![RawInputEvent::Mouse(crossterm::event::MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 2,
            row: 2,
            modifiers: KeyModifiers::empty(),
        })]);
    assert!(workspace_down.actions.is_empty());
    let workspace =
        state.handle_raw_events(vec![RawInputEvent::Mouse(crossterm::event::MouseEvent {
            kind: MouseEventKind::Up(MouseButton::Left),
            column: 2,
            row: 2,
            modifiers: KeyModifiers::empty(),
        })]);
    assert!(workspace.requests.is_empty());
    let [ClientShellAction::Endpoint { request, .. }] = &workspace.actions[..] else {
        panic!("workspace click should use the endpoint API");
    };
    assert!(matches!(
        &request.method,
        crate::api::schema::Method::WorkspaceFocus(target)
            if target.workspace_id == "ws_1"
    ));

    let pane = state.handle_raw_events(vec![RawInputEvent::Mouse(crossterm::event::MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 27,
        row: 1,
        modifiers: KeyModifiers::empty(),
    })]);
    assert!(pane.requests.is_empty());
    let [ClientShellAction::Endpoint { request, .. }] = &pane.actions[..] else {
        panic!("pane click should use the endpoint API");
    };
    assert!(matches!(
        &request.method,
        crate::api::schema::Method::PaneFocus(target) if target.pane_id == "pane_1"
    ));
}

#[test]
fn collapsed_workspace_jitter_remains_a_click() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.sidebar_collapsed = true;
    state.set_snapshot(Box::new(snapshot()));
    state.set_pane_surface(surface());
    state.compose(106, 20).expect("collapsed sidebar");
    let workspace = state.hits.workspaces[0].rect;
    state.handle_raw_events(vec![RawInputEvent::Mouse(crossterm::event::MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: workspace.x,
        row: workspace.y,
        modifiers: KeyModifiers::empty(),
    })]);
    state.handle_raw_events(vec![RawInputEvent::Mouse(crossterm::event::MouseEvent {
        kind: MouseEventKind::Drag(MouseButton::Left),
        column: workspace.x + 1,
        row: workspace.y,
        modifiers: KeyModifiers::empty(),
    })]);
    assert!(state.chrome_drag.is_none());
    assert!(state.workspace_press.is_some());
    let release =
        state.handle_raw_events(vec![RawInputEvent::Mouse(crossterm::event::MouseEvent {
            kind: MouseEventKind::Up(MouseButton::Left),
            column: workspace.x + 1,
            row: workspace.y,
            modifiers: KeyModifiers::empty(),
        })]);
    assert!(matches!(
        &release.actions[..],
        [ClientShellAction::Endpoint { request, .. }]
            if matches!(
                &request.method,
                crate::api::schema::Method::WorkspaceFocus(target)
                    if target.workspace_id == "ws_1"
            )
    ));
}

#[test]
fn space_members_render_labels_branches_and_collapsed_status() {
    let config = ClientShellConfig::from_config(&Config::default());
    let mut state = ClientShellState::new(config);
    let mut snapshot = snapshot();
    snapshot.workspaces[0].space_id = Some("space_repo".into());
    snapshot.workspaces.push(ClientShellWorkspace {
        workspace_id: "ws_2".into(),
        active_tab_id: "tab_ws2".into(),
        new_workspace_cwd: "/repo/feature".into(),
        number: 2,
        label: "repo-feature".into(),
        custom_label: false,
        branch: Some("feature".into()),
        git_ahead_behind: None,
        tokens: Vec::new(),
        worktree: None,
        focused: false,
        agent_status: AgentStatus::Idle,
        space_id: Some("space_repo".into()),
        setup: None,
    });
    snapshot.spaces = vec![crate::protocol::ClientShellSpace {
        space_id: "space_repo".into(),
        name: "repo".into(),
        color: 0,
        built_in: false,
        closed: Vec::new(),
    }];
    state.set_snapshot(Box::new(snapshot));
    state.set_pane_surface(surface());
    let frame = state.compose(106, 20).expect("composed frame");
    let text = frame
        .cells
        .chunks(frame.width as usize)
        .map(|row| {
            row.iter()
                .map(|cell| cell.symbol.as_str())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("├─"));
    assert!(text.contains("└─"));
    assert!(text.contains("repo-feature"));
    assert!(text.contains("feature"), "members show their branch");

    let mut replacement = (**state.snapshot.as_ref().expect("snapshot")).clone();
    replacement.revision = 2;
    replacement.focused_workspace_id = Some("ws_2".into());
    replacement.workspaces[0].focused = false;
    replacement.workspaces[1].focused = true;
    replacement.workspaces[1].agent_status = AgentStatus::Blocked;
    let mut replacement_surface = surface();
    replacement_surface.projection_revision = 2;
    state.collapsed_groups.insert("space_repo".into());
    state.set_snapshot(Box::new(replacement));
    state.set_pane_surface(replacement_surface);
    let collapsed = state.compose(106, 20).expect("collapsed space");
    let header = state.hits.space_headers[0].rect;
    let status_cell = usize::from(header.y) * usize::from(collapsed.width)
        + usize::from(header.x.saturating_add(1));
    assert_eq!(
        collapsed.cells[status_cell].fg,
        crate::protocol::color_to_u32(state.config.palette.red),
        "a collapsed space shows its most urgent member status"
    );
    assert_eq!(
        state
            .hits
            .workspaces
            .iter()
            .map(|hit| hit.workspace_id.as_str())
            .collect::<Vec<_>>(),
        ["ws_2"],
        "a collapsed space keeps its focused member visible"
    );
}

#[test]
fn workspace_click_waits_for_release_and_drag_reorders_by_stable_id() {
    let mut projected = snapshot();
    for index in 2..=3 {
        let mut workspace = projected.workspaces[0].clone();
        workspace.workspace_id = format!("ws_{index}");
        workspace.number = index;
        workspace.label = format!("workspace-{index}");
        workspace.focused = false;
        projected.workspaces.push(workspace);
    }
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(projected));
    state.set_pane_surface(surface());
    state.compose(106, 24).expect("three workspaces");
    let first = state.hits.workspaces[0].rect;
    let third = state.hits.workspaces[2].rect;

    let down = state.handle_raw_events(vec![RawInputEvent::Mouse(crossterm::event::MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: first.x + 2,
        row: first.y,
        modifiers: KeyModifiers::empty(),
    })]);
    assert!(down.actions.is_empty());
    let drag = state.handle_raw_events(vec![RawInputEvent::Mouse(crossterm::event::MouseEvent {
        kind: MouseEventKind::Drag(MouseButton::Left),
        column: third.x + 2,
        row: third.bottom(),
        modifiers: KeyModifiers::empty(),
    })]);
    assert!(drag.repaint);
    assert!(matches!(
        state.chrome_drag,
        Some(ClientChromeDrag::Workspace {
            ref source_workspace_id,
            target: Some(WorkspaceDropTarget {
                space_id: None,
                before_workspace_id: None,
                ..
            }),
        }) if source_workspace_id == "ws_1"
    ));
    let frame = state.compose(106, 24).expect("workspace drop indicator");
    assert!(frame
        .cells
        .chunks(frame.width as usize)
        .any(|row| row.iter().take(20).any(|cell| cell.symbol == "─")));

    let release =
        state.handle_raw_events(vec![RawInputEvent::Mouse(crossterm::event::MouseEvent {
            kind: MouseEventKind::Up(MouseButton::Left),
            column: third.x + 2,
            row: third.bottom(),
            modifiers: KeyModifiers::empty(),
        })]);
    assert!(matches!(
        &release.actions[..],
        [ClientShellAction::Endpoint { request, .. }]
            if matches!(
                &request.method,
                crate::api::schema::Method::WorkspaceMove(params)
                    if params.workspace_id == "ws_1" && params.insert_index == 3
            )
    ));

    state.compose(106, 24).expect("workspaces after drag");
    let second = state.hits.workspaces[1].rect;
    state.handle_raw_events(vec![RawInputEvent::Mouse(crossterm::event::MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: second.x + 2,
        row: second.y,
        modifiers: KeyModifiers::empty(),
    })]);
    let click = state.handle_raw_events(vec![RawInputEvent::Mouse(crossterm::event::MouseEvent {
        kind: MouseEventKind::Up(MouseButton::Left),
        column: second.x + 2,
        row: second.y,
        modifiers: KeyModifiers::empty(),
    })]);
    assert!(matches!(
        &click.actions[..],
        [ClientShellAction::Endpoint { request, .. }]
            if matches!(
                &request.method,
                crate::api::schema::Method::WorkspaceFocus(target)
                    if target.workspace_id == "ws_2"
            )
    ));
}

#[test]
fn pane_cycle_last_and_agent_actions_resolve_to_stable_pane_ids() {
    let mut initial = snapshot();
    let mut second = initial.panes[0].clone();
    second.pane_id = "pane_2".into();
    second.focused = false;
    initial.panes.push(second);
    initial.agents = vec![
        ClientShellAgent {
            pane_id: "pane_1".into(),
            workspace_id: "ws_1".into(),
            tab_id: "tab_1".into(),
            name: Some("first".into()),
            display_agent: None,
            agent: None,
            title: None,
            terminal_title: None,
            terminal_title_stripped: None,
            agent_status: AgentStatus::Idle,
            state_change_seq: 1,
            state_labels: Vec::new(),
            tokens: Vec::new(),
            focused: true,
            state_changed_at_ms: None,
        },
        ClientShellAgent {
            pane_id: "pane_2".into(),
            workspace_id: "ws_1".into(),
            tab_id: "tab_1".into(),
            name: Some("second".into()),
            display_agent: None,
            agent: None,
            title: None,
            terminal_title: None,
            terminal_title_stripped: None,
            agent_status: AgentStatus::Idle,
            state_change_seq: 2,
            state_labels: Vec::new(),
            tokens: Vec::new(),
            focused: false,
            state_changed_at_ms: None,
        },
    ];
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(initial.clone()));

    let mut cycle = ClientShellInput::default();
    state.record_binding(
        crate::input::KeybindMatch::Action(crate::input::KeybindAction::CyclePaneNext),
        &mut cycle,
    );
    let [ClientShellAction::Endpoint { request, .. }] = &cycle.actions[..] else {
        panic!("pane cycle should use endpoint API");
    };
    assert!(matches!(
        &request.method,
        crate::api::schema::Method::PaneFocus(target) if target.pane_id == "pane_2"
    ));

    let mut replacement = initial;
    replacement.revision = 2;
    replacement.focused_pane_id = Some("pane_2".into());
    replacement.panes[0].focused = false;
    replacement.panes[1].focused = true;
    state.set_snapshot(Box::new(replacement));
    let mut last = ClientShellInput::default();
    state.record_binding(
        crate::input::KeybindMatch::Action(crate::input::KeybindAction::LastPane),
        &mut last,
    );
    let [ClientShellAction::Endpoint { request, .. }] = &last.actions[..] else {
        panic!("last pane should use endpoint API");
    };
    assert!(matches!(
        &request.method,
        crate::api::schema::Method::PaneFocus(target) if target.pane_id == "pane_1"
    ));

    let mut agent = ClientShellInput::default();
    state.record_binding(
        crate::input::KeybindMatch::Action(crate::input::KeybindAction::FocusAgent(1)),
        &mut agent,
    );
    let [ClientShellAction::Endpoint { request, .. }] = &agent.actions[..] else {
        panic!("agent focus should use endpoint API");
    };
    // Equally urgent agents list the latest change first: pane_2, then pane_1.
    assert!(matches!(
        &request.method,
        crate::api::schema::Method::PaneFocus(target) if target.pane_id == "pane_1"
    ));
}

#[test]
fn agent_panel_priority_orders_by_urgency_with_marks_and_stable_hits() {
    let mut projected = snapshot();
    let mut second_pane = projected.panes[0].clone();
    second_pane.pane_id = "pane_2".into();
    second_pane.focused = false;
    projected.panes.push(second_pane);
    projected.agents = vec![
        ClientShellAgent {
            pane_id: "pane_1".into(),
            workspace_id: "ws_1".into(),
            tab_id: "tab_1".into(),
            name: Some("pi one".into()),
            display_agent: None,
            agent: Some("pi".into()),
            title: None,
            terminal_title: Some("first title".into()),
            terminal_title_stripped: Some("first".into()),
            agent_status: AgentStatus::Done,
            state_change_seq: 10,
            state_labels: Vec::new(),
            tokens: vec![("summary".into(), "review complete".into())],
            focused: true,
            state_changed_at_ms: None,
        },
        ClientShellAgent {
            pane_id: "pane_2".into(),
            workspace_id: "ws_1".into(),
            tab_id: "tab_1".into(),
            name: Some("pi two".into()),
            display_agent: None,
            agent: Some("pi".into()),
            title: None,
            terminal_title: Some("second title".into()),
            terminal_title_stripped: Some("second".into()),
            agent_status: AgentStatus::Blocked,
            state_change_seq: 20,
            state_labels: vec![("blocked".into(), "needs input".into())],
            tokens: vec![("summary".into(), "waiting for Can".into())],
            focused: false,
            state_changed_at_ms: None,
        },
    ];
    let mut config = Config::default();
    config.ui.agent_panel_sort = crate::config::AgentPanelSortConfig::Priority;
    config.ui.status_indicators = crate::config::StatusIndicatorStyle::Symbols;
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&config));
    state.set_snapshot(Box::new(projected));
    state.set_pane_surface(surface());

    let frame = state.compose(106, 30).expect("agent sidebar frame");
    let text = frame
        .cells
        .chunks(frame.width as usize)
        .map(|row| {
            row.iter()
                .map(|cell| cell.symbol.as_str())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    // Vendor icon, pulsing question mark, then the session title; the most
    // urgent agent leads, each with its workspace alongside.
    assert!(text.contains("π ? second · "), "frame: {text}");
    assert!(text.contains("π first · "), "frame: {text}");
    assert_eq!(
        state
            .hits
            .agents
            .first()
            .map(|(_, pane_id)| pane_id.as_str()),
        Some("pane_2")
    );

    let first = state.hits.agents[0].0;
    let click = state.handle_raw_events(vec![RawInputEvent::Mouse(crossterm::event::MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: first.x,
        row: first.y,
        modifiers: KeyModifiers::empty(),
    })]);
    let [ClientShellAction::Endpoint { request, .. }] = &click.actions[..] else {
        panic!("agent row should focus through endpoint API");
    };
    assert!(matches!(
        &request.method,
        crate::api::schema::Method::PaneFocus(target) if target.pane_id == "pane_2"
    ));

    state.compose(106, 8).expect("short agent sidebar frame");
    assert_eq!(
        state
            .hits
            .agents
            .first()
            .map(|(_, pane_id)| pane_id.as_str()),
        Some("pane_2")
    );
    let body = state.hits.agent_body;
    state.handle_raw_events(vec![RawInputEvent::Mouse(crossterm::event::MouseEvent {
        kind: MouseEventKind::ScrollDown,
        column: body.x,
        row: body.y,
        modifiers: KeyModifiers::empty(),
    })]);
    state.compose(106, 8).expect("scrolled agent sidebar frame");
    assert_eq!(
        state
            .hits
            .agents
            .first()
            .map(|(_, pane_id)| pane_id.as_str()),
        Some("pane_1")
    );

    state.sidebar_collapsed = true;
    let compact = state.compose(106, 30).expect("compact agent sidebar frame");
    let blocked = state
        .hits
        .agents
        .iter()
        .find(|(_, pane_id)| pane_id == "pane_2")
        .expect("blocked compact agent")
        .0;
    let row_start = blocked.y as usize * compact.width as usize + blocked.x as usize;
    assert_ne!(compact.cells[row_start].fg, compact.cells[row_start + 2].fg);
    assert_eq!(compact.cells[row_start].bg, compact.cells[row_start + 2].bg);
}

#[test]
fn muted_agent_sidebar_rows_do_not_stack_terminal_faint() {
    let mut projected = snapshot();
    projected.tabs[0].label = "second".into();
    projected.tabs[0].custom_label = true;
    projected.agents = vec![ClientShellAgent {
        pane_id: "pane_1".into(),
        workspace_id: "ws_1".into(),
        tab_id: "tab_1".into(),
        name: Some("reviewer".into()),
        display_agent: None,
        agent: Some("pi".into()),
        title: None,
        terminal_title: None,
        terminal_title_stripped: None,
        agent_status: AgentStatus::Working,
        state_change_seq: 1,
        state_labels: Vec::new(),
        tokens: Vec::new(),
        focused: true,
        state_changed_at_ms: None,
    }];
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(projected));
    state.set_pane_surface(surface());
    let frame = state.compose(106, 30).expect("agent sidebar frame");
    let row = state.hits.agents.first().expect("agent row hit").0;
    let buffer = frame.to_ratatui_buffer().expect("agent sidebar buffer");

    for (label, needle) in [("icon", "π"), ("agent", "reviewer")] {
        let (x, y) = cell_symbol_position(&frame, row, needle);
        let cell = buffer.cell((x, y)).expect("muted sidebar cell");
        assert!(
            !cell.modifier.contains(Modifier::DIM),
            "{label} cell at ({x},{y}) should not stack terminal faint: {cell:?}"
        );
    }
}

#[test]
fn workspace_state_text_does_not_stack_terminal_faint() {
    use crate::config::SpaceSidebarToken;

    let mut config = Config::default();
    config.ui.sidebar.spaces.rows = vec![
        vec![SpaceSidebarToken::StateIcon, SpaceSidebarToken::Workspace],
        vec![SpaceSidebarToken::StateText],
    ];
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&config));
    state.set_snapshot(Box::new(snapshot()));
    state.set_pane_surface(surface());
    let frame = state.compose(106, 30).expect("workspace sidebar frame");
    let rect = state.hits.workspaces.first().expect("workspace hit").rect;
    let buffer = frame.to_ratatui_buffer().expect("workspace sidebar buffer");
    let (x, y) = cell_symbol_position(&frame, rect, "idle");
    let cell = buffer.cell((x, y)).expect("workspace state text cell");
    assert!(
        !cell.modifier.contains(Modifier::DIM),
        "workspace state text at ({x},{y}) should not stack terminal faint: {cell:?}"
    );
}

#[test]
fn active_agent_view_controls_sidebar_order_and_focus_indices() {
    let mut projected = snapshot();
    let mut second_pane = projected.panes[0].clone();
    second_pane.pane_id = "pane_2".into();
    second_pane.focused = false;
    projected.panes.push(second_pane.clone());
    let mut third_pane = second_pane;
    third_pane.pane_id = "pane_3".into();
    projected.panes.push(third_pane);
    projected.agents = vec![
        ClientShellAgent {
            pane_id: "pane_1".into(),
            workspace_id: "ws_1".into(),
            tab_id: "tab_1".into(),
            name: Some("first".into()),
            display_agent: None,
            agent: Some("pi".into()),
            title: None,
            terminal_title: None,
            terminal_title_stripped: None,
            agent_status: AgentStatus::Idle,
            state_change_seq: 1,
            state_labels: Vec::new(),
            tokens: Vec::new(),
            focused: true,
            state_changed_at_ms: None,
        },
        ClientShellAgent {
            pane_id: "pane_2".into(),
            workspace_id: "ws_1".into(),
            tab_id: "tab_1".into(),
            name: Some("second".into()),
            display_agent: None,
            agent: Some("pi".into()),
            title: None,
            terminal_title: None,
            terminal_title_stripped: None,
            agent_status: AgentStatus::Blocked,
            state_change_seq: 2,
            state_labels: Vec::new(),
            tokens: Vec::new(),
            focused: false,
            state_changed_at_ms: None,
        },
        ClientShellAgent {
            pane_id: "pane_3".into(),
            workspace_id: "ws_1".into(),
            tab_id: "tab_1".into(),
            name: Some("third".into()),
            display_agent: None,
            agent: Some("pi".into()),
            title: None,
            terminal_title: None,
            terminal_title_stripped: None,
            agent_status: AgentStatus::Idle,
            state_change_seq: 3,
            state_labels: Vec::new(),
            tokens: Vec::new(),
            focused: false,
            state_changed_at_ms: None,
        },
    ];
    projected.agent_view_label = Some("review".into());
    projected.agent_order = vec!["pane_2".into(), "pane_3".into()];
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(projected));
    state.set_pane_surface(surface());
    state.compose(106, 30).expect("filtered agent sidebar");
    assert_eq!(
        state
            .hits
            .agents
            .iter()
            .map(|(_, pane_id)| pane_id.as_str())
            .collect::<Vec<_>>(),
        vec!["pane_2", "pane_3"]
    );
    assert_eq!(state.hits.agent_sort_toggle, Rect::default());

    let mut focus = ClientShellInput::default();
    state.record_binding(
        crate::input::KeybindMatch::Action(crate::input::KeybindAction::FocusAgent(0)),
        &mut focus,
    );
    assert!(matches!(
        &focus.actions[..],
        [ClientShellAction::Endpoint { request, .. }]
            if matches!(
                &request.method,
                crate::api::schema::Method::PaneFocus(target) if target.pane_id == "pane_2"
            )
    ));

    let mut next = ClientShellInput::default();
    state.record_binding(
        crate::input::KeybindMatch::Action(crate::input::KeybindAction::NextAgent),
        &mut next,
    );
    assert!(matches!(
        &next.actions[..],
        [ClientShellAction::Endpoint { request, .. }]
            if matches!(
                &request.method,
                crate::api::schema::Method::PaneFocus(target) if target.pane_id == "pane_2"
            )
    ));
}

#[test]
fn agent_sort_toggle_is_client_local_and_persists_per_endpoint() {
    let path = std::env::temp_dir().join(format!(
        "herdr-shell-agent-sort-{}-{}.json",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos()
    ));
    let mut projected = snapshot();
    projected.agents.push(ClientShellAgent {
        pane_id: "pane_1".into(),
        workspace_id: "ws_1".into(),
        tab_id: "tab_1".into(),
        name: Some("pi".into()),
        display_agent: None,
        agent: Some("pi".into()),
        title: None,
        terminal_title: None,
        terminal_title_stripped: None,
        agent_status: AgentStatus::Working,
        state_change_seq: 1,
        state_labels: Vec::new(),
        tokens: Vec::new(),
        focused: true,
        state_changed_at_ms: None,
    });
    let config =
        ClientShellConfig::from_config(&Config::default()).with_preferences_path(path.clone());
    let mut state = ClientShellState::new(config);
    state.set_snapshot(Box::new(projected));
    state.set_pane_surface(surface());
    state.compose(106, 30).expect("agent sidebar frame");
    let toggle = state.hits.agent_sort_toggle;

    let click = state.handle_raw_events(vec![RawInputEvent::Mouse(crossterm::event::MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: toggle.x,
        row: toggle.y,
        modifiers: KeyModifiers::empty(),
    })]);

    assert_eq!(
        state.config.agent_panel_sort,
        crate::config::AgentPanelSortConfig::Priority
    );
    assert!(click.actions.is_empty());
    let reloaded_config =
        ClientShellConfig::from_config(&Config::default()).with_preferences_path(path.clone());
    let reloaded = ClientShellState::new(reloaded_config);
    assert_eq!(
        reloaded.config.agent_panel_sort,
        crate::config::AgentPanelSortConfig::Priority
    );
    assert!(reloaded.agent_panel_sort_manual);
    std::fs::remove_file(path).expect("remove agent sort preferences");
}

#[test]
fn workspace_actions_preserve_selected_target_and_client_confirmation() {
    let mut snapshot = snapshot();
    let mut second = snapshot.workspaces[0].clone();
    second.workspace_id = "ws_2".into();
    second.number = 2;
    second.label = "second".into();
    second.focused = false;
    snapshot.workspaces.push(second);
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot));
    state.mode = ClientShellMode::Navigate;
    state.navigate_workspace_id = state.navigation_target(&ClientEndpointId::Local, "ws_2");

    let rename = state.handle_raw_events(vec![RawInputEvent::Key(crate::input::TerminalKey::new(
        KeyCode::Char('w'),
        KeyModifiers::SHIFT,
    ))]);
    assert!(rename.actions.is_empty());
    assert!(matches!(
        state.overlay.as_ref(),
        Some(ClientShellOverlay::Rename(ClientRenameOverlay {
            target: ClientRenameTarget::Workspace { workspace_id },
            ..
        })) if workspace_id == "ws_2"
    ));
    assert!(state.handle_input_bytes(&[0x15]).actions.is_empty());
    assert!(state.handle_input_bytes(b"renamed").actions.is_empty());
    let save = state.handle_input_bytes(b"\r");
    let [ClientShellAction::Endpoint { request, .. }] = &save.actions[..] else {
        panic!("workspace rename should use endpoint API");
    };
    assert!(matches!(
        &request.method,
        crate::api::schema::Method::WorkspaceRename(params)
            if params.workspace_id == "ws_2" && params.label == "renamed"
    ));

    state.navigate_workspace_id = state.navigation_target(&ClientEndpointId::Local, "ws_2");
    let mut close = ClientShellInput::default();
    state.record_binding(
        crate::input::KeybindMatch::Action(crate::input::KeybindAction::CloseWorkspace),
        &mut close,
    );
    assert!(close.actions.is_empty());
    assert!(matches!(
        state.overlay.as_ref(),
        Some(ClientShellOverlay::ConfirmClose(ClientConfirmCloseOverlay {
            workspace_id,
            ..
        })) if workspace_id == "ws_2"
    ));
    let confirm = state.handle_input_bytes(b"\r");
    let [ClientShellAction::Endpoint { request, .. }] = &confirm.actions[..] else {
        panic!("workspace confirmation should use endpoint API");
    };
    assert!(matches!(
        &request.method,
        crate::api::schema::Method::WorkspaceClose(params)
            if params.workspace_id == "ws_2" && params.close_group
    ));
}

#[test]
fn desktop_workspace_navigation_reveals_overflowing_selection() {
    let mut projected = snapshot();
    for index in 2..=8 {
        let mut workspace = projected.workspaces[0].clone();
        workspace.workspace_id = format!("ws_{index}");
        workspace.number = index;
        workspace.label = format!("workspace-{index}");
        workspace.focused = false;
        projected.workspaces.push(workspace);
    }
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(projected));
    state.set_pane_surface(surface());
    state.mode = ClientShellMode::Navigate;
    state.navigate_workspace_id = state.navigation_target(&ClientEndpointId::Local, "ws_1");
    state.compose(106, 12).expect("overflowing sidebar");

    for _ in 0..6 {
        state.handle_input_bytes(b"\x1b[B");
        state.compose(106, 12).expect("revealed workspace");
        let selected = state
            .navigate_workspace_id
            .as_ref()
            .expect("selection")
            .workspace_id
            .as_str();
        assert!(
            state
                .hits
                .workspaces
                .iter()
                .any(|hit| hit.workspace_id == selected),
            "{selected} should remain visible"
        );
    }
}

#[test]
fn named_workspace_overlay_targets_projected_source_workspace() {
    let mut config = Config::default();
    config.ui.prompt_new_workspace_name = true;
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&config));
    state.set_snapshot(Box::new(snapshot()));
    let mut open = ClientShellInput::default();
    state.record_binding(
        crate::input::KeybindMatch::Action(crate::input::KeybindAction::NewWorkspace),
        &mut open,
    );
    assert!(matches!(
        state.overlay.as_ref(),
        Some(ClientShellOverlay::Rename(ClientRenameOverlay {
            input: value,
            target: ClientRenameTarget::NewWorkspace {
                source_workspace_id,
                ..
            },
            ..
        })) if value.as_str() == "repo" && source_workspace_id.as_deref() == Some("ws_1")
    ));
    let create = state.handle_input_bytes(b"\r");
    let [ClientShellAction::Endpoint { request, .. }] = &create.actions[..] else {
        panic!("named workspace should use endpoint API");
    };
    assert!(matches!(
        &request.method,
        crate::api::schema::Method::WorkspaceCreate(params)
            if params.source_workspace_id.as_deref() == Some("ws_1")
                && params.cwd.as_deref() == Some("/repo")
                && params.label.is_none()
    ));
}

#[test]
fn navigate_mode_selects_workspace_locally_then_focuses_by_stable_id() {
    let mut snapshot = snapshot();
    let mut second = snapshot.workspaces[0].clone();
    second.workspace_id = "ws_2".into();
    second.number = 2;
    second.label = "second".into();
    second.focused = false;
    snapshot.workspaces.push(second);
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot));
    state.set_pane_surface(surface());

    assert!(state.handle_input_bytes(&[0x02]).actions.is_empty());
    let enter_navigate = state.handle_input_bytes(b"w");
    assert!(enter_navigate.repaint);
    assert_eq!(state.mode, ClientShellMode::Navigate);
    assert_eq!(
        state.navigate_workspace_id,
        state.navigation_target(&ClientEndpointId::Local, "ws_1")
    );

    let invalid = state.handle_input_bytes(b"9");
    assert!(invalid.actions.is_empty());
    assert_eq!(state.mode, ClientShellMode::Navigate);
    assert_eq!(
        state.navigate_workspace_id,
        state.navigation_target(&ClientEndpointId::Local, "ws_1")
    );

    let move_selection = state.handle_input_bytes(b"\x1b[B");
    assert!(move_selection.actions.is_empty());
    assert_eq!(
        state.navigate_workspace_id,
        state.navigation_target(&ClientEndpointId::Local, "ws_2")
    );
    let frame = state.compose(106, 20).expect("navigate frame");
    let text = frame
        .cells
        .chunks(frame.width as usize)
        .map(|row| {
            row.iter()
                .map(|cell| cell.symbol.as_str())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("second"));
    assert!(text.contains("NAVIGATE"));

    let focus = state.handle_input_bytes(b"\r");
    let [ClientShellAction::Endpoint { request, .. }] = &focus.actions[..] else {
        panic!("selected workspace should use endpoint API");
    };
    assert!(matches!(
        &request.method,
        crate::api::schema::Method::WorkspaceFocus(target)
            if target.workspace_id == "ws_2"
    ));
    assert_eq!(state.mode, ClientShellMode::Terminal);
}

#[test]
fn worktree_create_previews_the_endpoint_owned_checkout_path() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    state.set_pane_surface(surface());
    let mut prepare = ClientShellInput::default();
    state.record_binding(
        crate::input::KeybindMatch::Action(crate::input::KeybindAction::NewWorktree),
        &mut prepare,
    );
    let [ClientShellAction::Endpoint { request, .. }] = &prepare.actions[..] else {
        panic!("new worktree should prepare through worktree.list");
    };
    assert!(matches!(
        &request.method,
        crate::api::schema::Method::WorktreeList(params)
            if params.workspace_id.as_deref() == Some("ws_1")
    ));
    let request_id = request.id.clone();
    assert!(
        state
            .handle_endpoint_result("boot-1", &request_id, Ok(worktree_list_result(None)))
            .0
    );
    let frame = state.compose(106, 30).expect("new worktree modal");
    let text = frame
        .cells
        .chunks(frame.width as usize)
        .map(|row| {
            row.iter()
                .map(|cell| cell.symbol.as_str())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("new worktree"));
    assert!(text.contains("create and open"));
    assert!(frame.cursor.as_ref().is_some_and(|cursor| cursor.visible));

    assert!(state
        .handle_input_bytes(b"feature/client-shell")
        .actions
        .is_empty());
    assert!(matches!(
        &state.overlay,
        Some(ClientShellOverlay::WorktreeCreate(create))
            if create.checkout_path
                == "/tmp/herdr-worktrees/repo/feature-client-shell"
    ));
    let submit = state.handle_input_bytes(b"\r");
    let [ClientShellAction::Endpoint { request, .. }] = &submit.actions[..] else {
        panic!("worktree create should use endpoint API");
    };
    assert!(matches!(
        &request.method,
        crate::api::schema::Method::WorktreeCreate(params)
            if params.workspace_id.as_deref() == Some("ws_1")
                && params.branch.as_deref() == Some("feature/client-shell")
                && params.path.is_none()
                && !params.focus
    ));
}

#[test]
fn unavailable_worktree_create_does_not_wedge_the_overlay() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    state.set_pane_surface(surface());
    let mut prepare = ClientShellInput::default();
    state.record_binding(
        crate::input::KeybindMatch::Action(crate::input::KeybindAction::NewWorktree),
        &mut prepare,
    );
    let [ClientShellAction::Endpoint { request, .. }] = &prepare.actions[..] else {
        panic!("new worktree should prepare through worktree.list");
    };
    state.handle_endpoint_result("boot-1", &request.id, Ok(worktree_list_result(None)));
    state.set_endpoint_methods(Some(vec!["worktree.list".into()]));
    state.handle_input_bytes(b"feature/unavailable");

    let submit = state.handle_input_bytes(b"\r");

    assert!(submit.actions.is_empty());
    assert!(matches!(
        &state.overlay,
        Some(ClientShellOverlay::WorktreeCreate(create)) if !create.creating
    ));
    assert!(state
        .visible_endpoint_notice
        .as_ref()
        .is_some_and(|notice| notice.key.code == "worktree.create"));
}

#[test]
fn worktree_action_errors_expire_without_more_input() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    state.set_pane_surface(surface());

    let mut guard = ClientShellInput::default();
    state.record_binding(
        crate::input::KeybindMatch::Action(crate::input::KeybindAction::RemoveWorktree),
        &mut guard,
    );
    let message = "This workspace is not a Herdr-managed worktree checkout.";
    assert_eq!(state.endpoint_error.as_deref(), Some(message));

    let deadline = state.endpoint_error_deadline.expect("deadline");
    assert!(!state.tick_endpoint_error(deadline - std::time::Duration::from_secs(1)));
    assert_eq!(state.endpoint_error.as_deref(), Some(message));

    assert!(state.tick_endpoint_error(deadline + std::time::Duration::from_millis(1)));
    assert!(state.endpoint_error.is_none());

    // A repeated identical message must start a fresh lifetime instead of
    // inheriting the earlier deadline.
    let before_repeat = std::time::Instant::now();
    state.set_endpoint_error(message);
    assert!(
        state.endpoint_error_deadline.expect("deadline")
            >= before_repeat + std::time::Duration::from_secs(5)
    );
}

#[test]
fn worktree_prepare_rejection_notice_expires() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    state.set_pane_surface(surface());
    let mut prepare = ClientShellInput::default();
    state.record_binding(
        crate::input::KeybindMatch::Action(crate::input::KeybindAction::NewWorktree),
        &mut prepare,
    );
    let [ClientShellAction::Endpoint { request, .. }] = &prepare.actions[..] else {
        panic!("new worktree should prepare through worktree.list");
    };
    let request_id = request.id.clone();
    state.handle_endpoint_result(
        "boot-1",
        &request_id,
        Err(ClientShellEndpointError {
            code: Some("not_git_worktree".into()),
            message: "Herdr worktree actions require a workspace inside a Git work tree".into(),
        }),
    );
    let notice = state
        .visible_endpoint_notice
        .as_ref()
        .expect("rejection notice");
    assert!(notice.key.code.contains("not_git_worktree"));
    let deadline = notice.deadline;

    let (_, repaint) = state.tick_notifications(deadline + std::time::Duration::from_millis(1));
    assert!(repaint);
    assert!(state.visible_endpoint_notice.is_none());
}

#[test]
fn worktree_open_filters_and_clicks_a_stable_public_entry() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    state.set_pane_surface(surface());
    let mut prepare = ClientShellInput::default();
    state.record_binding(
        crate::input::KeybindMatch::Action(crate::input::KeybindAction::OpenWorktree),
        &mut prepare,
    );
    let [ClientShellAction::Endpoint { request, .. }] = &prepare.actions[..] else {
        panic!("open worktree should prepare through worktree.list");
    };
    let request_id = request.id.clone();
    state.handle_endpoint_result("boot-1", &request_id, Ok(worktree_list_result(None)));
    let frame = state.compose(106, 30).expect("open worktree modal");
    let text = frame
        .cells
        .chunks(frame.width as usize)
        .map(|row| {
            row.iter()
                .map(|cell| cell.symbol.as_str())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("feature"));
    let row = state.hits.worktree_rows[0].0;
    let open = state.handle_raw_events(vec![RawInputEvent::Mouse(crossterm::event::MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: row.x + 2,
        row: row.y,
        modifiers: KeyModifiers::empty(),
    })]);
    let [ClientShellAction::Endpoint { request, .. }] = &open.actions[..] else {
        panic!("worktree row should open through endpoint API");
    };
    assert!(matches!(
        &request.method,
        crate::api::schema::Method::WorktreeOpen(params)
            if params.workspace_id.as_deref() == Some("ws_1")
                && params.path.as_deref() == Some("/repo-feature")
                && params.focus
    ));
}

#[test]
fn worktree_remove_escalates_recoverable_failure_to_force_confirmation() {
    for (code, message, expect_force) in [
        ("dirty_worktree_requires_force", "dirty worktree", true),
        (
            "worktree_remove_failed",
            "fatal: '/repo-feature' is not a working tree",
            true,
        ),
        ("worktree_remove_failed", "Permission denied", false),
        ("server_unavailable", "is not a working tree", false),
    ] {
        let mut snapshot = snapshot();
        snapshot.workspaces[0].worktree = Some(ClientShellWorktree {
            key: "repo-key".into(),
            label: "repo".into(),
            is_linked_worktree: true,
        });
        let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
        state.set_snapshot(Box::new(snapshot));
        state.set_pane_surface(surface());
        let mut prepare = ClientShellInput::default();
        state.record_binding(
            crate::input::KeybindMatch::Action(crate::input::KeybindAction::RemoveWorktree),
            &mut prepare,
        );
        let [ClientShellAction::Endpoint { request, .. }] = &prepare.actions[..] else {
            panic!("remove worktree should prepare through worktree.list");
        };
        let request_id = request.id.clone();
        state.handle_endpoint_result(
            "boot-1",
            &request_id,
            Ok(worktree_list_result(Some("ws_1"))),
        );
        let remove = state.handle_input_bytes(b"\r");
        let [ClientShellAction::Endpoint { request, .. }] = &remove.actions[..] else {
            panic!("worktree remove should use endpoint API");
        };
        assert!(matches!(
            &request.method,
            crate::api::schema::Method::WorktreeRemove(params)
                if params.workspace_id == "ws_1" && !params.force
        ));
        let request_id = request.id.clone();
        let (_, actions) = state.handle_endpoint_result(
            "boot-1",
            &request_id,
            Err(ClientShellEndpointError {
                code: Some(code.into()),
                message: message.into(),
            }),
        );
        assert!(actions.is_empty(), "failure must not retry automatically");
        let Some(ClientShellOverlay::WorktreeRemove(remove)) = &state.overlay else {
            panic!("failed remove should keep its confirmation");
        };
        assert!(!remove.removing);
        assert_eq!(remove.force_confirmation, expect_force);
        if !expect_force {
            assert_eq!(remove.error.as_deref(), Some(message));
            continue;
        }
        assert!(remove.error.is_none());
        let frame = state.compose(106, 30).expect("force remove modal");
        let text = frame
            .cells
            .chunks(frame.width as usize)
            .map(|row| {
                row.iter()
                    .map(|cell| cell.symbol.as_str())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("delete anyway"));
        assert!(text.contains("permanently deleted"));
        let force = state.handle_input_bytes(b"\r");
        let [ClientShellAction::Endpoint { request, .. }] = &force.actions[..] else {
            panic!("forced worktree remove should use endpoint API");
        };
        assert!(matches!(
            &request.method,
            crate::api::schema::Method::WorktreeRemove(params)
                if params.workspace_id == "ws_1" && params.force
        ));
        let request_id = request.id.clone();
        let (_, actions) = state.handle_endpoint_result(
            "boot-1",
            &request_id,
            Err(ClientShellEndpointError {
                code: Some("worktree_remove_failed".into()),
                message: "fatal: '/repo-feature' is not a working tree".into(),
            }),
        );
        assert!(actions.is_empty());
        let Some(ClientShellOverlay::WorktreeRemove(remove)) = &state.overlay else {
            panic!("forced failure should keep its confirmation");
        };
        assert!(!remove.removing);
        assert_eq!(
            remove.error.as_deref(),
            Some("fatal: '/repo-feature' is not a working tree")
        );
        state.handle_input_bytes(b"\x1b");
        assert!(state.overlay.is_none());
    }
}

#[test]
fn semantic_notifications_use_client_policy_and_stable_navigation_targets() {
    let mut config = ClientShellConfig::from_config(&Config::default());
    config.toast_delivery = crate::config::ToastDelivery::Herdr;
    config.toast_delay_seconds = 0;
    let mut state = ClientShellState::new(config);
    let mut projected = snapshot();
    projected.agents.push(ClientShellAgent {
        pane_id: "pane_2".into(),
        workspace_id: "ws_2".into(),
        tab_id: "tab_2".into(),
        name: None,
        display_agent: Some("codex".into()),
        agent: Some("codex".into()),
        title: None,
        terminal_title: None,
        terminal_title_stripped: None,
        agent_status: AgentStatus::Blocked,
        state_change_seq: 1,
        state_labels: Vec::new(),
        tokens: Vec::new(),
        focused: false,
        state_changed_at_ms: None,
    });
    state.set_snapshot(Box::new(projected));
    state.set_pane_surface(surface());
    let now = std::time::Instant::now();
    let (effects, repaint) = state.receive_notification(
        &ClientEndpointId::Local,
        SemanticNotification {
            kind: SemanticNotificationKind::NeedsAttention,
            title: "codex needs attention".into(),
            body: Some("other · 2".into()),
            sound: Some(SemanticNotificationSound::Request),
            agent: Some("codex".into()),
            workspace_id: Some("ws_2".into()),
            tab_id: Some("tab_2".into()),
            pane_id: Some("pane_2".into()),
            position: None,
        },
        now,
    );
    assert!(repaint);
    assert!(matches!(
        effects.as_slice(),
        [ClientShellNotificationEffect::Sound {
            sound: crate::sound::Sound::Request,
            ..
        }]
    ));
    let frame = state.compose(100, 28).expect("notification frame");
    let rendered = frame
        .cells
        .chunks(frame.width as usize)
        .map(|row| {
            row.iter()
                .map(|cell| cell.symbol.as_str())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(rendered.contains("codex needs attention"));
    let hit = state.hits.notification_toast;
    let click = || {
        RawInputEvent::Mouse(crossterm::event::MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: hit.x,
            row: hit.y,
            modifiers: KeyModifiers::empty(),
        })
    };
    state.mode = ClientShellMode::Navigate;
    let ignored = state.handle_raw_events(vec![click()]);
    assert!(ignored.actions.is_empty());
    assert!(state.visible_notification.is_some());

    state.mode = ClientShellMode::Terminal;
    let outcome = state.handle_raw_events(vec![click()]);
    assert!(outcome.actions.iter().any(|action| matches!(
        action,
        ClientShellAction::Endpoint { request, .. }
            if matches!(
                &request.method,
                crate::api::schema::Method::PaneFocus(params)
                    if params.pane_id == "pane_2"
            )
    )));
    assert!(state.visible_notification.is_none());

    state.receive_notification(
        &ClientEndpointId::Local,
        SemanticNotification {
            kind: SemanticNotificationKind::NeedsAttention,
            title: "codex needs attention".into(),
            body: None,
            sound: None,
            agent: Some("codex".into()),
            workspace_id: Some("ws_2".into()),
            tab_id: Some("tab_2".into()),
            pane_id: Some("pane_2".into()),
            position: None,
        },
        now,
    );
    let mut keybind = ClientShellInput::default();
    state.record_binding(
        crate::input::KeybindMatch::Action(crate::input::KeybindAction::OpenNotificationTarget),
        &mut keybind,
    );
    assert!(keybind.actions.iter().any(|action| matches!(
        action,
        ClientShellAction::Endpoint { request, .. }
            if matches!(
                &request.method,
                crate::api::schema::Method::PaneFocus(params)
                    if params.pane_id == "pane_2"
            )
    )));
    assert!(state.visible_notification.is_none());

    state.receive_notification(
        &ClientEndpointId::Local,
        SemanticNotification {
            kind: SemanticNotificationKind::NeedsAttention,
            title: "first".into(),
            body: None,
            sound: None,
            agent: Some("codex".into()),
            workspace_id: Some("ws_2".into()),
            tab_id: Some("tab_2".into()),
            pane_id: Some("pane_2".into()),
            position: None,
        },
        now,
    );
    assert!(state.visible_notification.is_some());
    state.config.toast_delay_seconds = 1;
    let (_, repaint) = state.receive_notification(
        &ClientEndpointId::Local,
        SemanticNotification {
            kind: SemanticNotificationKind::NeedsAttention,
            title: "replacement".into(),
            body: None,
            sound: None,
            agent: Some("codex".into()),
            workspace_id: Some("ws_2".into()),
            tab_id: Some("tab_2".into()),
            pane_id: Some("pane_2".into()),
            position: None,
        },
        now,
    );
    assert!(repaint);
    assert!(state.visible_notification.is_none());
    assert_eq!(state.pending_notifications.len(), 1);
}

fn space_color_snapshot() -> ClientShellSnapshot {
    fn workspace(id: &str, label: &str, space_id: &str) -> ClientShellWorkspace {
        let mut workspace = snapshot().workspaces.remove(0);
        workspace.workspace_id = id.into();
        workspace.active_tab_id = format!("{id}_tab");
        workspace.label = label.into();
        workspace.branch = None;
        workspace.focused = false;
        workspace.space_id = Some(space_id.into());
        workspace
    }
    fn agent(workspace_id: &str, agent: &str) -> ClientShellAgent {
        ClientShellAgent {
            pane_id: format!("{workspace_id}_pane"),
            workspace_id: workspace_id.into(),
            tab_id: format!("{workspace_id}_tab"),
            name: None,
            display_agent: None,
            agent: Some(agent.into()),
            title: None,
            terminal_title: None,
            terminal_title_stripped: None,
            agent_status: AgentStatus::Idle,
            state_change_seq: 1,
            state_labels: Vec::new(),
            tokens: Vec::new(),
            focused: false,
            state_changed_at_ms: None,
        }
    }

    let mut projected = snapshot();
    projected.workspaces = vec![
        workspace("ws_issue", "issue", "space_feature"),
        workspace("ws_main", "main", "space_feature"),
        workspace("ws_notes", "notes", "other"),
    ];
    projected.spaces = vec![
        crate::protocol::ClientShellSpace {
            space_id: "space_feature".into(),
            name: "feature".into(),
            color: 0,
            built_in: false,
            closed: Vec::new(),
        },
        crate::protocol::ClientShellSpace {
            space_id: "other".into(),
            name: "other".into(),
            color: 0,
            built_in: true,
            closed: Vec::new(),
        },
    ];
    projected.agents = vec![
        agent("ws_notes", "claude"),
        agent("ws_issue", "pi"),
        agent("ws_main", "codex"),
    ];
    projected
}

#[test]
fn expanded_sidebar_uses_matching_space_colors_and_grouped_agent_order() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(space_color_snapshot()));
    state.set_pane_surface(surface());
    let frame = state.compose(106, 30).expect("sidebar frame");
    let buffer = frame.to_ratatui_buffer().expect("sidebar buffer");
    let fg_at = |rect: Rect, needle: &str| {
        let position = cell_symbol_position(&frame, rect, needle);
        buffer[position].fg
    };

    let workspaces = state
        .hits
        .workspaces
        .iter()
        .map(|hit| (hit.workspace_id.as_str(), hit.rect))
        .collect::<Vec<_>>();
    assert_eq!(
        workspaces.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
        ["ws_issue", "ws_main", "ws_notes"]
    );
    let issue = fg_at(workspaces[0].1, "issue");
    let main = fg_at(workspaces[1].1, "main");
    let notes = fg_at(workspaces[2].1, "notes");
    assert_eq!(main, issue, "space members share one space color");
    assert_ne!(main, notes, "other is neutral, not the feature color");
    assert_ne!(main, state.config.palette.subtext0);

    let agents = state
        .hits
        .agents
        .iter()
        .map(|(rect, pane_id)| (pane_id.as_str(), *rect))
        .collect::<Vec<_>>();
    assert_eq!(
        agents.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
        ["ws_issue_pane", "ws_main_pane", "ws_notes_pane"],
        "grouped agents follow space order"
    );
    // Each agent sits under its worktree header, drawn in its space color.
    let header_fg =
        |agent: Rect, needle: &str| fg_at(Rect::new(agent.x, agent.y - 1, agent.width, 1), needle);
    let muted =
        |color| crate::client::shell::sidebar::muted_space_color(color, &state.config.palette);
    assert_eq!(header_fg(agents[0].1, "issue"), muted(issue));
    assert_eq!(header_fg(agents[1].1, "main"), muted(main));
    assert_eq!(header_fg(agents[2].1, "notes"), muted(notes));
    let rows = frame_rows(&frame);
    let space_header = agents[0].1.y as usize - 2;
    assert!(
        rows[space_header].contains("feature"),
        "{}",
        rows[space_header]
    );
}

#[test]
fn collapsed_sidebar_numbers_use_space_colors() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(space_color_snapshot()));
    state.set_pane_surface(surface());
    state.sidebar_collapsed = true;
    let frame = state.compose(106, 30).expect("collapsed frame");
    let buffer = frame.to_ratatui_buffer().expect("collapsed buffer");
    let number_fg = |workspace_id: &str| {
        let hit = state
            .hits
            .workspaces
            .iter()
            .find(|hit| hit.workspace_id == workspace_id)
            .expect("collapsed workspace hit");
        buffer[(hit.rect.x, hit.rect.y)].fg
    };

    assert_eq!(number_fg("ws_issue"), number_fg("ws_main"));
    assert_ne!(number_fg("ws_notes"), number_fg("ws_main"));
}

fn spaced_snapshot() -> ClientShellSnapshot {
    let mut projected = snapshot();
    projected.workspaces[0].space_id = Some("space_knowledge".into());
    projected.spaces = vec![
        crate::protocol::ClientShellSpace {
            space_id: "space_knowledge".into(),
            name: "knowledge".into(),
            color: 2,
            built_in: false,
            closed: vec![crate::protocol::ClientShellClosedMember {
                member_id: "member_old".into(),
                label: "old-project".into(),
                cwd: "/old-project".into(),
                branch: Some("knowledge".into()),
            }],
        },
        crate::protocol::ClientShellSpace {
            space_id: "other".into(),
            name: "other".into(),
            color: 0,
            built_in: true,
            closed: Vec::new(),
        },
    ];
    projected
}

fn click(state: &mut ClientShellState, button: MouseButton, rect: Rect) -> ClientShellInput {
    state.handle_raw_events(vec![RawInputEvent::Mouse(crossterm::event::MouseEvent {
        kind: MouseEventKind::Down(button),
        column: rect.x + 1,
        row: rect.y,
        modifiers: KeyModifiers::empty(),
    })])
}

/// A full left click: press and release.
fn press_release(state: &mut ClientShellState, rect: Rect) -> ClientShellInput {
    let at = |kind| {
        RawInputEvent::Mouse(crossterm::event::MouseEvent {
            kind,
            column: rect.x + 1,
            row: rect.y,
            modifiers: KeyModifiers::empty(),
        })
    };
    state.handle_raw_events(vec![
        at(MouseEventKind::Down(MouseButton::Left)),
        at(MouseEventKind::Up(MouseButton::Left)),
    ])
}

fn single_endpoint_method(outcome: &ClientShellInput) -> crate::api::schema::Method {
    let [ClientShellAction::Endpoint { request, .. }] = &outcome.actions[..] else {
        panic!("expected one endpoint request, got {:?}", outcome.actions);
    };
    request.method.clone()
}

#[test]
fn spaces_render_headers_members_and_closed_rows() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(spaced_snapshot()));
    state.set_pane_surface(surface());
    let frame = state.compose(106, 30).expect("sidebar frame");
    let buffer = frame.to_ratatui_buffer().expect("sidebar buffer");

    // Empty `other` is hidden, so only the knowledge header renders.
    let [header] = &state.hits.space_headers[..] else {
        panic!("one space header: {:?}", state.hits.space_headers);
    };
    assert_eq!(header.space_id, "space_knowledge");
    let header_name = cell_symbol_position(&frame, header.rect, "knowledge");
    assert_eq!(
        buffer[header_name].fg,
        crate::ui::space_color(&state.config.palette, 2),
        "headers use their space's color slot"
    );

    let workspace = state.hits.workspaces[0].rect;
    assert!(workspace.y > header.rect.y, "members follow their header");
    let live = cell_symbol_position(&frame, workspace, "client-shell");
    assert_eq!(
        buffer[live].fg,
        crate::ui::space_color(&state.config.palette, 2),
        "members share the space color"
    );

    let [closed] = &state.hits.closed_members[..] else {
        panic!("one closed member: {:?}", state.hits.closed_members);
    };
    assert!(
        closed.rect.y > workspace.y,
        "closed members follow live ones"
    );
    let closed_rect = closed.rect;
    cell_symbol_position(&frame, closed_rect, "old-project");
    cell_symbol_position(&frame, closed_rect, "closed");

    let outcome = click(&mut state, MouseButton::Left, closed_rect);
    assert!(matches!(
        single_endpoint_method(&outcome),
        crate::api::schema::Method::SpaceMemberOpen(target)
            if target.space_id == "space_knowledge" && target.member_id == "member_old"
    ));
}

#[test]
fn clicking_a_space_header_collapses_it_to_the_focused_member() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(spaced_snapshot()));
    state.set_pane_surface(surface());
    state.compose(106, 30).expect("sidebar frame");
    let header = state.hits.space_headers[0].rect;

    click(&mut state, MouseButton::Left, header);
    assert!(
        !state.group_is_collapsed(&ClientEndpointId::Local, "space_knowledge"),
        "a header collapses on release so it can be dragged"
    );
    press_release(&mut state, header);
    let frame = state.compose(106, 30).expect("collapsed frame");
    assert!(state.group_is_collapsed(&ClientEndpointId::Local, "space_knowledge"));
    assert!(state.hits.closed_members.is_empty());
    assert_eq!(
        state.hits.workspaces.len(),
        1,
        "the focused member stays visible"
    );
    cell_symbol_position(&frame, state.hits.space_headers[0].rect, "▸");
}

#[test]
fn closed_member_context_menu_opens_or_removes() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(spaced_snapshot()));
    state.set_pane_surface(surface());
    state.compose(106, 30).expect("sidebar frame");
    let closed = state.hits.closed_members[0].rect;

    assert!(click(&mut state, MouseButton::Right, closed)
        .actions
        .is_empty());
    let labels = match state.overlay.as_ref() {
        Some(ClientShellOverlay::ContextMenu(menu)) => menu
            .items()
            .iter()
            .map(|item| item.label)
            .collect::<Vec<_>>(),
        _ => panic!("closed member context menu"),
    };
    assert_eq!(labels, ["Open", "Remove from space"]);

    let mut outcome = ClientShellInput::default();
    state.activate_context_menu_item(1, &mut outcome);
    assert!(matches!(
        single_endpoint_method(&outcome),
        crate::api::schema::Method::SpaceMemberRemove(target)
            if target.member_id == "member_old"
    ));
}

#[test]
fn space_header_menu_edits_supported_endpoints_only() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(spaced_snapshot()));
    state.set_pane_surface(surface());
    state.compose(106, 30).expect("sidebar frame");
    let header = state.hits.space_headers[0].rect;

    click(&mut state, MouseButton::Right, header);
    let labels = |state: &ClientShellState| match state.overlay.as_ref() {
        Some(ClientShellOverlay::ContextMenu(menu)) => menu
            .items()
            .iter()
            .map(|item| item.label)
            .collect::<Vec<_>>(),
        _ => panic!("space context menu"),
    };
    assert_eq!(labels(&state), ["Rename", "Collapse", "Delete space"]);
    let mut outcome = ClientShellInput::default();
    state.activate_context_menu_item(2, &mut outcome);
    assert!(matches!(
        single_endpoint_method(&outcome),
        crate::api::schema::Method::SpaceDelete(target) if target.space_id == "space_knowledge"
    ));

    state.set_endpoint_methods(Some(vec!["workspace.rename".into()]));
    click(&mut state, MouseButton::Right, header);
    assert_eq!(labels(&state), ["Collapse"]);
}

fn shell_repo(name: &str) -> crate::protocol::ClientShellRepo {
    crate::protocol::ClientShellRepo {
        name: name.into(),
        root: format!("~/code/{name}"),
        base_branch: "main".into(),
        remote: Some("origin".into()),
        settings: Default::default(),
    }
}

fn repo_snapshot() -> ClientShellSnapshot {
    let mut projected = spaced_snapshot();
    projected.workspaces[0].worktree = Some(crate::protocol::ClientShellWorktree {
        key: "repo-key".into(),
        label: "pyshiftup".into(),
        is_linked_worktree: true,
    });
    projected.repos = vec![shell_repo("pyshiftup"), shell_repo("guided")];
    projected.worktree_path_template = "/tmp/herdr-worktrees/{space}/{repo}/{name}".into();
    projected
}

fn repo_state() -> ClientShellState {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(repo_snapshot()));
    state.set_pane_surface(surface());
    state.compose(120, 34).expect("sidebar frame");
    state
}

fn screen_text(state: &mut ClientShellState) -> String {
    frame_rows(&state.compose(120, 34).expect("frame")).join("\n")
}

fn space_worktree_created(warnings: &[&str]) -> crate::api::schema::ResponseResult {
    serde_json::from_value(serde_json::json!({
        "type": "space_worktree_created",
        "workspace": {
            "workspace_id": "ws_9", "number": 2, "label": "pyshiftup", "focused": false,
            "space_id": "space_knowledge", "pane_count": 1, "tab_count": 1,
            "active_tab_id": "ws_9:t1", "agent_status": "unknown"
        },
        "tab": {
            "tab_id": "ws_9:t1", "workspace_id": "ws_9", "number": 1, "label": "1",
            "focused": false, "pane_count": 1, "agent_status": "unknown"
        },
        "root_pane": {
            "pane_id": "ws_9:p1", "terminal_id": "term_9", "workspace_id": "ws_9",
            "tab_id": "ws_9:t1", "focused": false, "agent_status": "unknown", "revision": 0
        },
        "worktree": {
            "path": "/tmp/herdr-worktrees/knowledge/pyshiftup/knowledge",
            "branch": "knowledge", "is_bare": false, "is_detached": false,
            "is_prunable": false, "is_linked_worktree": true, "label": "pyshiftup"
        },
        "sync": {
            "fetched": true, "root_updated": false, "branch_source": "new",
            "start_point": "origin/main", "warnings": warnings
        }
    }))
    .expect("space worktree response")
}

#[test]
fn space_worktree_dialog_creates_from_the_chosen_repo() {
    let mut state = repo_state();
    let header = state.hits.space_headers[0].rect;
    click(&mut state, MouseButton::Right, header);
    let mut outcome = ClientShellInput::default();
    state.activate_context_menu_item(0, &mut outcome);
    assert!(
        matches!(
            &state.overlay,
            Some(ClientShellOverlay::SpaceWorktree(dialog))
                if dialog.name.as_str() == "knowledge"
                    && dialog.selected_repo.as_deref() == Some("guided")
        ),
        "defaults to the space name and a repo not in the space yet"
    );
    let text = screen_text(&mut state);
    assert!(text.contains("new worktree in knowledge"), "{text}");
    assert!(text.contains("sync main with origin first"), "{text}");
    assert!(
        text.contains("/tmp/herdr-worktrees/knowledge/guided/knowledge"),
        "{text}"
    );

    state.handle_input_bytes(b"\x1b[A");
    let submit = state.handle_input_bytes(b"\r");
    let [ClientShellAction::Endpoint { request, .. }] = &submit.actions[..] else {
        panic!("expected space.worktree.create, got {:?}", submit.actions);
    };
    assert!(matches!(
        &request.method,
        crate::api::schema::Method::SpaceWorktreeCreate(params)
            if params.space_id == "space_knowledge"
                && params.repo == "pyshiftup"
                && params.name == "knowledge"
                && params.sync
                && !params.focus
    ));
    let (_, actions) = state.handle_endpoint_result(
        "boot-1",
        &request.id,
        Err(ClientShellEndpointError {
            code: Some("sync_fetch_failed".into()),
            message: "couldn't fetch origin: offline".into(),
        }),
    );
    assert!(actions.is_empty());
    assert!(
        state.visible_endpoint_notice.is_none(),
        "the dialog shows the failure itself"
    );
    assert!(matches!(
        &state.overlay,
        Some(ClientShellOverlay::SpaceWorktree(dialog))
            if !dialog.sync && dialog.offer_without_sync && !dialog.creating
                && dialog.error.as_deref().is_some_and(|error| error.contains("local main"))
    ));
    assert!(screen_text(&mut state).contains("create from local main"));

    let retry = state.handle_input_bytes(b"\r");
    let [ClientShellAction::Endpoint { request, .. }] = &retry.actions[..] else {
        panic!("expected a retry, got {:?}", retry.actions);
    };
    assert!(matches!(
        &request.method,
        crate::api::schema::Method::SpaceWorktreeCreate(params) if !params.sync
    ));
    let (_, actions) = state.handle_endpoint_result(
        "boot-1",
        &request.id,
        Ok(space_worktree_created(&[
            "main is checked out at /x; didn't update it",
        ])),
    );
    assert!(state.overlay.is_none());
    assert!(matches!(
        &actions[..],
        [ClientShellAction::Endpoint { request, .. }]
            if matches!(&request.method, crate::api::schema::Method::TabFocus(target)
                if target.tab_id == "ws_9:t1")
    ));
    assert!(state
        .visible_endpoint_notice
        .as_ref()
        .is_some_and(
            |notice| notice.key.kind == ClientEndpointNoticeKind::Warning
                && notice.body.contains("didn't update")
        ));
}

#[test]
fn space_worktree_dialog_without_repos_leads_to_adding_one() {
    let mut state = repo_state();
    let mut snapshot = repo_snapshot();
    snapshot.repos.clear();
    state.set_snapshot(Box::new(snapshot));
    state.open_space_worktree_dialog("space_knowledge");
    assert!(screen_text(&mut state).contains("no repos yet"));

    state.handle_input_bytes(b"\r");
    assert!(matches!(
        &state.overlay,
        Some(ClientShellOverlay::RepoEdit(edit))
            if edit.original_name.is_none()
                && edit.return_to == ClientRepoEditReturn::SpaceWorktree {
                    space_id: "space_knowledge".into()
                }
    ));
    state.handle_input_bytes(b"~/code/new-repo");
    let save = state.handle_input_bytes(b"\r");
    let [ClientShellAction::Endpoint { request, .. }] = &save.actions[..] else {
        panic!("expected repo.add, got {:?}", save.actions);
    };
    assert!(matches!(
        &request.method,
        crate::api::schema::Method::RepoAdd(params)
            if params.root == "~/code/new-repo"
                && params.name.is_none()
                && params.base_branch.is_none()
                && params.remote.is_none()
    ));
    state.handle_endpoint_result(
        "boot-1",
        &request.id,
        Ok(crate::api::schema::ResponseResult::RepoInfo {
            repo: crate::api::schema::RepoInfo {
                name: "new-repo".into(),
                root: "~/code/new-repo".into(),
                root_path: "/home/me/code/new-repo".into(),
                base_branch: "main".into(),
                remote: None,
                settings: Default::default(),
            },
        }),
    );
    assert!(
        matches!(
            &state.overlay,
            Some(ClientShellOverlay::SpaceWorktree(dialog))
                if dialog.selected_repo.as_deref() == Some("new-repo")
        ),
        "returns to the worktree dialog with the new repo selected"
    );
}

#[test]
fn settings_repos_tab_lists_adds_edits_and_removes() {
    let mut state = repo_state();
    state.open_settings_overlay();
    let mut outcome = ClientShellInput::default();
    state.select_settings_section(ClientSettingsSection::Repos, &mut outcome);
    let text = screen_text(&mut state);
    assert!(text.contains("pyshiftup"), "{text}");
    assert!(text.contains("~/code/guided"), "{text}");
    assert!(text.contains("main · origin"), "{text}");

    state.handle_input_bytes(b"\x1b[B");
    let remove = state.handle_input_bytes(b"x");
    let [ClientShellAction::Endpoint { request, .. }] = &remove.actions[..] else {
        panic!("expected repo.remove, got {:?}", remove.actions);
    };
    assert!(matches!(
        &request.method,
        crate::api::schema::Method::RepoRemove(target) if target.repo == "guided"
    ));

    state.handle_input_bytes(b"\r");
    let Some(ClientShellOverlay::RepoEdit(edit)) = &state.overlay else {
        panic!("enter edits the selected repo");
    };
    assert_eq!(edit.original_name.as_deref(), Some("guided"));
    assert_eq!(edit.fields[0].as_str(), "~/code/guided");
    // Move to the base branch field and change it.
    state.handle_input_bytes(b"\t\t");
    state.handle_input_bytes(b"\x15develop");
    let save = state.handle_input_bytes(b"\r");
    let [ClientShellAction::Endpoint { request, .. }] = &save.actions[..] else {
        panic!("expected repo.update, got {:?}", save.actions);
    };
    assert!(matches!(
        &request.method,
        crate::api::schema::Method::RepoUpdate(params)
            if params.repo == "guided"
                && params.base_branch.as_deref() == Some("develop")
                && params.name.is_none()
                && params.root.is_none()
                && params.remote.is_none()
    ));

    state.handle_input_bytes(b"\x1b");
    state.handle_endpoint_result(
        "boot-1",
        &request.id,
        Err(ClientShellEndpointError {
            code: Some("invalid_branch".into()),
            message: "'develop' is not a valid base branch".into(),
        }),
    );
    assert!(
        matches!(
            &state.overlay,
            Some(ClientShellOverlay::RepoEdit(edit))
                if !edit.saving && edit.error.as_deref().is_some_and(|e| e.contains("develop"))
        ),
        "saving ignores escape and shows the rejection"
    );
    state.handle_input_bytes(b"\x1b");
    assert!(
        matches!(
            &state.overlay,
            Some(ClientShellOverlay::Settings(settings))
                if settings.section == ClientSettingsSection::Repos
        ),
        "escape returns to settings"
    );
}

#[test]
fn repo_editor_saves_worktree_settings_after_the_repo() {
    let mut state = repo_state();
    let mut snapshot = repo_snapshot();
    snapshot.repos[0].settings.on_create = "npm ci".into();
    state.set_snapshot(Box::new(snapshot));
    state.open_repo_editor(Some("pyshiftup"), ClientRepoEditReturn::Settings);
    let text = screen_text(&mut state);
    assert!(text.contains("new worktrees"), "{text}");
    assert!(text.contains("npm ci"), "{text}");
    let Some(ClientShellOverlay::RepoEdit(edit)) = &state.overlay else {
        panic!("editor open");
    };
    assert_eq!(edit.field_count(), 9);

    // Only settings changed: one repo.settings.set.
    for _ in 0..8 {
        state.handle_input_bytes(b"\t");
    }
    state.handle_input_bytes(b"claude");
    let save = state.handle_input_bytes(b"\r");
    let [ClientShellAction::Endpoint { request, .. }] = &save.actions[..] else {
        panic!("expected repo.settings.set, got {:?}", save.actions);
    };
    assert!(matches!(
        &request.method,
        crate::api::schema::Method::RepoSettingsSet(params)
            if params.repo == "pyshiftup"
                && params.settings.on_create == "npm ci"
                && params.settings.start_command == "claude"
    ));

    // Adding a repo with settings: repo.add first, then its settings.
    state.overlay = None;
    state.open_repo_editor(None, ClientRepoEditReturn::Settings);
    state.handle_input_bytes(b"~/code/new-repo");
    for _ in 0..4 {
        state.handle_input_bytes(b"\t");
    }
    state.handle_input_bytes(b"ben/");
    let save = state.handle_input_bytes(b"\r");
    let [ClientShellAction::Endpoint { request, .. }] = &save.actions[..] else {
        panic!("expected repo.add, got {:?}", save.actions);
    };
    assert!(matches!(
        &request.method,
        crate::api::schema::Method::RepoAdd(_)
    ));
    let (_, actions) = state.handle_endpoint_result(
        "boot-1",
        &request.id,
        Ok(crate::api::schema::ResponseResult::RepoInfo {
            repo: crate::api::schema::RepoInfo {
                name: "new-repo".into(),
                root: "~/code/new-repo".into(),
                root_path: "/home/me/code/new-repo".into(),
                base_branch: "main".into(),
                remote: None,
                settings: Default::default(),
            },
        }),
    );
    let [ClientShellAction::Endpoint { request, .. }] = &actions[..] else {
        panic!("expected repo.settings.set, got {actions:?}");
    };
    assert!(matches!(
        &request.method,
        crate::api::schema::Method::RepoSettingsSet(params)
            if params.repo == "new-repo" && params.settings.branch_prefix == "ben/"
    ));
    assert!(matches!(
        &state.overlay,
        Some(ClientShellOverlay::RepoEdit(edit))
            if edit.saving && edit.original_name.as_deref() == Some("new-repo")
    ));

    // Servers without repo settings keep the four repo fields.
    state.overlay = None;
    state.set_endpoint_methods(Some(vec!["repo.update".into(), "repo.add".into()]));
    state.open_repo_editor(Some("pyshiftup"), ClientRepoEditReturn::Settings);
    let Some(ClientShellOverlay::RepoEdit(edit)) = &state.overlay else {
        panic!("editor open");
    };
    assert_eq!(edit.field_count(), 4);
    let text = screen_text(&mut state);
    assert!(!text.contains("branch prefix"), "{text}");
}

#[test]
fn creating_a_space_offers_its_first_worktree() {
    let mut state = repo_state();
    state.begin_new_space(None);
    state.handle_input_bytes(b"launch");
    let save = state.handle_input_bytes(b"\r");
    let [ClientShellAction::Endpoint { request, .. }] = &save.actions[..] else {
        panic!("expected space.create, got {:?}", save.actions);
    };
    assert!(matches!(
        &request.method,
        crate::api::schema::Method::SpaceCreate(params) if params.name == "launch"
    ));
    state.handle_endpoint_result(
        "boot-1",
        &request.id,
        Ok(crate::api::schema::ResponseResult::SpaceInfo {
            space: crate::api::schema::SpaceInfo {
                space_id: "space_launch".into(),
                name: "launch".into(),
                color: 3,
                built_in: false,
                workspace_ids: Vec::new(),
                closed: Vec::new(),
            },
        }),
    );
    assert!(matches!(
        &state.overlay,
        Some(ClientShellOverlay::SpaceWorktree(dialog))
            if dialog.space_id == "space_launch" && dialog.name.as_str() == "launch"
    ));
}

fn two_space_snapshot() -> ClientShellSnapshot {
    let mut projected = repo_snapshot();
    let mut second = projected.workspaces[0].clone();
    second.workspace_id = "ws_2".into();
    second.number = 2;
    second.label = "billing-api".into();
    second.focused = false;
    second.worktree = None;
    second.space_id = Some("space_billing".into());
    projected.workspaces.push(second);
    projected.spaces.insert(
        1,
        crate::protocol::ClientShellSpace {
            space_id: "space_billing".into(),
            name: "billing".into(),
            color: 3,
            built_in: false,
            closed: Vec::new(),
        },
    );
    projected
}

fn mouse(
    state: &mut ClientShellState,
    kind: MouseEventKind,
    column: u16,
    row: u16,
) -> ClientShellInput {
    state.handle_raw_events(vec![RawInputEvent::Mouse(crossterm::event::MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::empty(),
    })])
}

#[test]
fn add_worktree_row_adds_an_existing_checkout_to_the_space() {
    let mut state = repo_state();
    let text = screen_text(&mut state);
    assert!(text.contains("+ worktree"), "{text}");
    let add = state.hits.add_worktree[0].clone();
    assert_eq!(add.space_id, "space_knowledge");

    click(&mut state, MouseButton::Left, add.rect);
    let labels = match state.overlay.as_ref() {
        Some(ClientShellOverlay::ContextMenu(menu)) => menu
            .items()
            .iter()
            .map(|item| item.label)
            .collect::<Vec<_>>(),
        _ => panic!("add worktree menu"),
    };
    assert_eq!(labels, ["New worktree…", "Add existing worktree…"]);
    let mut outcome = ClientShellInput::default();
    state.activate_context_menu_item(1, &mut outcome);
    let requests = outcome
        .actions
        .iter()
        .map(|action| match action {
            ClientShellAction::Endpoint { request, .. } => request.clone(),
            other => panic!("unexpected action {other:?}"),
        })
        .collect::<Vec<_>>();
    assert_eq!(requests.len(), 2, "one listing per repo");
    assert!(matches!(
        &requests[1].method,
        crate::api::schema::Method::WorktreeList(params)
            if params.cwd.as_deref() == Some("~/code/guided") && params.workspace_id.is_none()
    ));
    let listing = |path: &str, branch: &str, linked: bool| crate::api::schema::WorktreeInfo {
        path: path.into(),
        branch: Some(branch.into()),
        is_bare: false,
        is_detached: false,
        is_prunable: false,
        is_linked_worktree: linked,
        open_workspace_id: None,
        label: "repo".into(),
    };
    let source = crate::api::schema::WorktreeSourceInfo {
        repo_key: "key".into(),
        repo_name: "repo".into(),
        repo_root: "/code/repo".into(),
        source_checkout_path: "/code/repo".into(),
        source_workspace_id: None,
    };
    state.handle_endpoint_result(
        "boot-1",
        &requests[0].id,
        Ok(crate::api::schema::ResponseResult::WorktreeList {
            source: source.clone(),
            worktrees: vec![listing("/code/pyshiftup", "main", false)],
        }),
    );
    state.handle_endpoint_result(
        "boot-1",
        &requests[1].id,
        Ok(crate::api::schema::ResponseResult::WorktreeList {
            source,
            worktrees: vec![
                listing("/code/guided", "main", false),
                listing("/wt/other/guided/spike", "spike", true),
            ],
        }),
    );
    let text = screen_text(&mut state);
    assert!(
        text.contains("add existing worktree to knowledge"),
        "{text}"
    );
    assert!(text.contains("guided · spike"), "{text}");
    assert!(text.contains("main checkout"), "{text}");

    state.handle_input_bytes(b"spike");
    let submit = state.handle_input_bytes(b"\r");
    let [ClientShellAction::Endpoint { request, .. }] = &submit.actions[..] else {
        panic!("expected space.worktree.open, got {:?}", submit.actions);
    };
    assert!(matches!(
        &request.method,
        crate::api::schema::Method::SpaceWorktreeOpen(params)
            if params.space_id == "space_knowledge"
                && params.path == "/wt/other/guided/spike"
                && params.focus
    ));
}

#[test]
fn dropping_a_workspace_on_another_space_files_it_there() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(two_space_snapshot()));
    state.set_pane_surface(surface());
    state.compose(120, 34).expect("sidebar");
    let source = state.hits.workspaces[0].rect;
    let billing = state
        .hits
        .space_headers
        .iter()
        .find(|hit| hit.space_id == "space_billing")
        .expect("billing header")
        .rect;

    mouse(
        &mut state,
        MouseEventKind::Down(MouseButton::Left),
        source.x + 2,
        source.y,
    );
    mouse(
        &mut state,
        MouseEventKind::Drag(MouseButton::Left),
        billing.x + 2,
        billing.y,
    );
    assert!(matches!(
        &state.chrome_drag,
        Some(ClientChromeDrag::Workspace {
            target: Some(WorkspaceDropTarget { space_id: Some(space), before_workspace_id: None, .. }),
            ..
        }) if space == "space_billing"
    ));
    let text = screen_text(&mut state);
    assert!(text.contains("─"), "drop indicator: {text}");
    let drop = mouse(
        &mut state,
        MouseEventKind::Up(MouseButton::Left),
        billing.x + 2,
        billing.y,
    );
    assert!(matches!(
        single_endpoint_method(&drop),
        crate::api::schema::Method::SpaceAssign(params)
            if params.workspace_id == "ws_1"
                && params.space_id == "space_billing"
                && params.before_workspace_id.is_none()
    ));

    // Releasing over the panes cancels.
    state.compose(120, 34).expect("sidebar");
    let source = state.hits.workspaces[0].rect;
    mouse(
        &mut state,
        MouseEventKind::Down(MouseButton::Left),
        source.x + 2,
        source.y,
    );
    mouse(
        &mut state,
        MouseEventKind::Drag(MouseButton::Left),
        80,
        source.y,
    );
    let cancelled = mouse(
        &mut state,
        MouseEventKind::Up(MouseButton::Left),
        80,
        source.y,
    );
    assert!(cancelled.actions.is_empty(), "{:?}", cancelled.actions);

    // Dropping back where it started sends nothing.
    state.compose(120, 34).expect("sidebar");
    let source = state.hits.workspaces[0].rect;
    mouse(
        &mut state,
        MouseEventKind::Down(MouseButton::Left),
        source.x + 2,
        source.y,
    );
    mouse(
        &mut state,
        MouseEventKind::Drag(MouseButton::Left),
        source.x + 3,
        source.y,
    );
    let none = mouse(
        &mut state,
        MouseEventKind::Up(MouseButton::Left),
        source.x + 3,
        source.y,
    );
    assert!(none.actions.is_empty(), "{:?}", none.actions);
}

#[test]
fn dragging_a_space_header_reorders_spaces() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(two_space_snapshot()));
    state.set_pane_surface(surface());
    state.compose(120, 34).expect("sidebar");
    let knowledge = state.hits.space_headers[0].rect;
    let billing_add = state
        .hits
        .add_worktree
        .iter()
        .find(|hit| hit.space_id == "space_billing")
        .expect("billing + worktree")
        .rect;

    mouse(
        &mut state,
        MouseEventKind::Down(MouseButton::Left),
        knowledge.x + 2,
        knowledge.y,
    );
    mouse(
        &mut state,
        MouseEventKind::Drag(MouseButton::Left),
        billing_add.x + 2,
        billing_add.bottom(),
    );
    assert!(matches!(
        &state.chrome_drag,
        Some(ClientChromeDrag::Space { space_id, target: Some((None, _)) })
            if space_id == "space_knowledge"
    ));
    let drop = mouse(
        &mut state,
        MouseEventKind::Up(MouseButton::Left),
        billing_add.x + 2,
        billing_add.bottom(),
    );
    assert!(matches!(
        single_endpoint_method(&drop),
        crate::api::schema::Method::SpaceMove(params)
            if params.space_id == "space_knowledge" && params.before_space_id.is_none()
    ));
    assert!(
        !state.group_is_collapsed(&ClientEndpointId::Local, "space_knowledge"),
        "dragging a header does not collapse it"
    );
}
