use super::*;
use crate::raw_input::RawInputEvent;

fn state() -> ClientShellState {
    let mut snapshot = super::super::tests::snapshot();
    for number in 2..=3 {
        let mut tab = snapshot.tabs[0].clone();
        tab.tab_id = format!("tab_{number}");
        tab.number = number;
        tab.focused = false;
        snapshot.tabs.push(tab);
        let mut pane = snapshot.panes[0].clone();
        pane.pane_id = format!("pane_{number}");
        pane.tab_id = format!("tab_{number}");
        pane.focused = true;
        snapshot.panes.push(pane);
    }
    snapshot.agents = vec![crate::protocol::ClientShellAgent {
        pane_id: "pane_3".into(),
        tab_id: "tab_3".into(),
        workspace_id: "ws_1".into(),
        name: None,
        display_agent: None,
        agent: Some("claude".into()),
        title: None,
        terminal_title: None,
        terminal_title_stripped: None,
        agent_status: crate::api::schema::AgentStatus::Working,
        state_change_seq: 1,
        state_labels: Vec::new(),
        tokens: Vec::new(),
        focused: false,
        state_changed_at_ms: None,
    }];
    snapshot.focused_tab_id = Some("tab_2".into());
    snapshot.focused_pane_id = Some("pane_2".into());
    let mut split = snapshot.panes[1].clone();
    split.pane_id = "pane_4".into();
    split.focused = false;
    snapshot.panes.push(split);
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot));
    state
}

fn perform(
    state: &mut ClientShellState,
    action: impl FnOnce(&mut ClientShellState, &mut ClientShellInput),
) -> ClientShellInput {
    state.observe_navigation_location();
    let mut outcome = ClientShellInput::default();
    action(state, &mut outcome);
    state.record_navigation_actions(&outcome.actions);
    outcome
}

fn current(state: &ClientShellState) -> ViewLocation {
    state.navigation_history.current().unwrap().clone()
}

fn focus(state: &mut ClientShellState, pane: &str) -> ClientShellInput {
    perform(state, |state, outcome| {
        state.push_endpoint_method(
            Method::PaneFocus(PaneTarget {
                pane_id: pane.into(),
            }),
            outcome,
        )
    })
}

fn back(state: &mut ClientShellState) -> ClientShellInput {
    state.handle_input_bytes(b"\x1b[<128;10;10M")
}

fn forward(state: &mut ClientShellState) -> ClientShellInput {
    state.handle_input_bytes(b"\x1b[<129;10;10M")
}

#[test]
fn navigation_history_tab_grid_zoom_round_trip_without_waiting_for_snapshots() {
    let mut state = state();
    // Real input dispatch records the grid's implicit agent focus as one visit.
    state.hits.agent_grid_toggle = Rect::new(2, 2, 8, 1);
    state.handle_input_bytes(b"\x1b[<0;3;3M");
    assert!(current(&state).grid);
    assert_eq!(current(&state).pane.as_deref(), Some("pane_3"));
    perform(&mut state, |state, outcome| {
        state.set_agent_grid_zoom("pane_3".into(), true, outcome)
    });
    assert!(current(&state).grid_zoom);
    assert_eq!(state.navigation_history.entries.len(), 3);

    back(&mut state);
    assert!(current(&state).grid);
    assert!(!current(&state).grid_zoom);
    let tab = back(&mut state);
    assert!(!current(&state).grid);
    assert_eq!(current(&state).tab, "tab_2");
    assert!(tab.actions.iter().any(|action| matches!(action,
        ClientShellAction::Endpoint { request, .. }
            if matches!(&request.method, Method::PaneFocus(target) if target.pane_id == "pane_2")
    )));
    forward(&mut state);
    assert!(current(&state).grid);
    assert!(!current(&state).grid_zoom);
    forward(&mut state);
    assert!(current(&state).grid_zoom);
    assert_eq!(state.navigation_history.entries.len(), 3);
    assert!(forward(&mut state).actions.is_empty());
}

#[test]
fn navigation_history_branching_and_duplicate_focus() {
    let mut state = state();
    focus(&mut state, "pane_1");
    focus(&mut state, "pane_1");
    assert_eq!(state.navigation_history.entries.len(), 2);
    back(&mut state);
    focus(&mut state, "pane_3");
    assert_eq!(state.navigation_history.entries.len(), 2);
    assert!(forward(&mut state).actions.is_empty());
    assert_eq!(current(&state).pane.as_deref(), Some("pane_3"));
}

#[test]
fn navigation_history_batched_back_and_release_do_not_repeat() {
    let mut state = state();
    focus(&mut state, "pane_1");
    focus(&mut state, "pane_3");
    state.handle_input_bytes(b"\x1b[<128;1;1M\x1b[<128;1;1m\x1b[<128;1;1M");
    assert_eq!(current(&state).tab, "tab_2");
    assert_eq!(state.navigation_history.cursor, 0);
}

#[test]
fn navigation_history_skips_closed_targets_and_reused_boot_ids() {
    let mut state = state();
    focus(&mut state, "pane_1");
    focus(&mut state, "pane_3");
    let mut snapshot = state.snapshot.clone().unwrap();
    snapshot.panes.retain(|pane| pane.pane_id != "pane_1");
    snapshot.tabs.retain(|tab| tab.tab_id != "tab_1");
    snapshot.revision += 1;
    state.set_snapshot(snapshot);
    back(&mut state);
    assert_eq!(current(&state).tab, "tab_2");
    let mut snapshot = state.snapshot.clone().unwrap();
    snapshot.boot_id = "new-boot".into();
    state.set_snapshot(snapshot);
    assert!(forward(&mut state).actions.is_empty());
}

#[test]
fn navigation_history_restores_explicit_tab_zoom() {
    let mut state = state();
    perform(&mut state, |state, outcome| {
        state.push_endpoint_method(
            Method::PaneZoom(PaneZoomParams {
                pane_id: Some("pane_2".into()),
                mode: PaneZoomMode::On,
            }),
            outcome,
        )
    });
    focus(&mut state, "pane_1");
    let outcome = back(&mut state);
    assert!(current(&state).tab_zoom);
    assert!(outcome.actions.iter().any(|action| matches!(action,
        ClientShellAction::Endpoint { request, .. }
            if matches!(&request.method, Method::PaneZoom(params) if params.mode == PaneZoomMode::On)
    )));
    back(&mut state);
    assert!(!current(&state).tab_zoom);
}

#[test]
fn navigation_history_ignores_render_changes_and_recovers_from_rejection() {
    let mut state = state();
    state.observe_navigation_location();
    let outcome = focus(&mut state, "pane_1");
    for action in outcome.actions {
        if let ClientShellAction::Endpoint { request, .. } = action {
            state.navigation_request_finished(&request.id, false);
        }
    }
    state.observe_navigation_location();
    assert_eq!(current(&state).tab, "tab_2");
    let count = state.navigation_history.entries.len();
    for _ in 0..3 {
        let mut snapshot = state.snapshot.clone().unwrap();
        snapshot.revision += 1;
        state.set_snapshot(snapshot);
        state.handle_raw_events(vec![RawInputEvent::Unsupported]);
    }
    assert_eq!(state.navigation_history.entries.len(), count);
}

#[test]
fn navigation_history_shortcut_works_over_overlay_without_pane_input() {
    let mut state = state();
    focus(&mut state, "pane_1");
    state.open_navigator_overlay();
    let outcome = state.handle_input_bytes(b"\x1b[1;7D");
    assert_eq!(current(&state).tab, "tab_2");
    assert!(state.overlay.is_none());
    assert!(outcome.requests.is_empty());
}

#[test]
fn navigation_history_pixel_side_button() {
    let mut state = state();
    focus(&mut state, "pane_1");
    state.handle_pixel_mouse(
        b"\x1b[<128;200;100M",
        crate::input::mouse::HostGeometry {
            cols: 100,
            rows: 30,
            width_px: 1000,
            height_px: 600,
        },
    );
    assert_eq!(current(&state).tab, "tab_2");
}

#[test]
fn navigation_history_is_bounded() {
    let mut state = state();
    for index in 0..300 {
        focus(&mut state, if index % 2 == 0 { "pane_1" } else { "pane_2" });
    }
    assert_eq!(state.navigation_history.entries.len(), HISTORY_LIMIT);
    assert_eq!(state.navigation_history.cursor, HISTORY_LIMIT - 1);
}

#[test]
fn navigation_history_single_pane_zoom_is_not_a_visit() {
    let mut state = state();
    focus(&mut state, "pane_1");
    let count = state.navigation_history.entries.len();
    perform(&mut state, |state, outcome| {
        state.push_endpoint_method(
            Method::PaneZoom(PaneZoomParams {
                pane_id: Some("pane_1".into()),
                mode: PaneZoomMode::On,
            }),
            outcome,
        )
    });
    assert!(!current(&state).tab_zoom);
    assert_eq!(state.navigation_history.entries.len(), count);
}

#[test]
fn navigation_history_late_failure_removes_superseded_visit() {
    let mut state = state();
    let failed = focus(&mut state, "pane_1");
    focus(&mut state, "pane_3");
    for action in failed.actions {
        if let ClientShellAction::Endpoint { request, .. } = action {
            state.navigation_request_finished(&request.id, false);
        }
    }
    assert_eq!(state.navigation_history.entries.len(), 2);
    back(&mut state);
    assert_eq!(current(&state).tab, "tab_2");
}

#[test]
fn navigation_history_visible_directional_target_survives_delayed_response() {
    let mut state = state();
    perform(&mut state, |state, outcome| {
        state.push_endpoint_method(
            Method::PaneFocusDirection(crate::api::schema::PaneFocusDirectionParams {
                pane_id: None,
                direction: crate::api::schema::PaneDirection::Right,
            }),
            outcome,
        )
    });
    let mut snapshot = state.snapshot.clone().unwrap();
    snapshot.revision += 1;
    snapshot.focused_pane_id = Some("pane_4".into());
    state.set_snapshot(snapshot);
    focus(&mut state, "pane_1");
    assert_eq!(state.navigation_history.entries.len(), 3);
    back(&mut state);
    assert_eq!(current(&state).pane.as_deref(), Some("pane_4"));
}

fn add_remote(state: &mut ClientShellState) -> ClientEndpointId {
    let remote = ClientEndpointId::Ssh(
        crate::client::endpoint::ProfileId::parse("0123456789abcdef0123456789abcdef").unwrap(),
    );
    let mut endpoint = state.endpoints[0].clone();
    endpoint.endpoint_id = remote.clone();
    endpoint.label = "Remote".into();
    state.endpoints.push(endpoint);
    remote
}

#[test]
fn navigation_history_waits_for_endpoint_then_restores_grid() {
    let mut state = state();
    let remote = add_remote(&mut state);
    perform(&mut state, |state, outcome| {
        state.toggle_agent_grid(outcome)
    });
    perform(&mut state, |state, outcome| {
        state.set_agent_grid_zoom("pane_3".into(), true, outcome)
    });
    let mut remote_location = current(&state);
    remote_location.endpoint = remote.clone();
    state.navigation_history.visit(remote_location);
    state.navigation_history.pending = None;
    state.activate_endpoint_projection(&remote);
    // The actual remote opens in its tab, not the source connection's grid.
    state.observe_navigation_location();
    back(&mut state); // remembered remote grid
    back(&mut state); // local zoomed grid; handoff has not completed
    assert!(state.navigation_history.restore_after_activation.is_some());
    let mut outcome = ClientShellInput::default();
    state.resume_navigation_history(&mut outcome);
    assert!(outcome.actions.is_empty());
    state.activate_endpoint_projection(&ClientEndpointId::Local);
    state.resume_navigation_history(&mut outcome);
    assert!(state.agent_grid_is_zoomed());
    assert!(!outcome.actions.is_empty());
}

#[test]
fn navigation_history_supersedes_handoff_when_returning_to_displayed_endpoint() {
    let mut state = state();
    let remote = add_remote(&mut state);
    state.observe_navigation_location();
    let local = current(&state);
    let mut other = local.clone();
    other.endpoint = remote.clone();
    state.navigation_history.visit(other);
    state.navigation_history.visit(local);
    back(&mut state);
    let cancelled = forward(&mut state);
    assert!(cancelled.actions.iter().any(|action| matches!(
        action,
        ClientShellAction::ActivateEndpoint {
            endpoint_id: ClientEndpointId::Local,
            ..
        }
    )));
    assert_eq!(
        state
            .navigation_history
            .restore_after_activation
            .as_ref()
            .unwrap()
            .endpoint,
        ClientEndpointId::Local
    );
}
