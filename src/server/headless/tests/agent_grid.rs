use super::*;

struct GridFixture {
    server: HeadlessServer,
    shell: crate::layout::PaneId,
    first_agent: crate::layout::PaneId,
    second_agent: crate::layout::PaneId,
    second_input: tokio::sync::mpsc::Receiver<bytes::Bytes>,
}

fn grid_fixture() -> GridFixture {
    let mut server = test_headless_server();
    let mut first = crate::workspace::Workspace::test_new("one");
    let shell = first.tabs[0].root_pane;
    first.insert_test_runtime(
        shell,
        crate::terminal::TerminalRuntime::test_with_screen_bytes(80, 23, b"PLAIN-SHELL"),
    );
    let agent_tab = first.test_add_tab(Some("review"));
    let first_agent = first.tabs[agent_tab].root_pane;
    first.insert_test_runtime(
        first_agent,
        crate::terminal::TerminalRuntime::test_with_screen_bytes(80, 23, b"FIRST-AGENT"),
    );
    let mut second = crate::workspace::Workspace::test_new("two");
    let second_agent = second.tabs[0].root_pane;
    let (second_runtime, second_input) =
        crate::terminal::TerminalRuntime::test_with_channel(80, 23);
    second.insert_test_runtime(second_agent, second_runtime);

    server.app.state.workspaces = vec![first, second];
    server.app.state.ensure_test_terminals();
    for (workspace_index, pane_id, agent) in [
        (0, first_agent, crate::detect::Agent::Pi),
        (1, second_agent, crate::detect::Agent::Claude),
    ] {
        let terminal_id = server.app.state.workspaces[workspace_index]
            .terminal_id(pane_id)
            .cloned()
            .expect("terminal id");
        server
            .app
            .state
            .terminals
            .get_mut(&terminal_id)
            .expect("terminal")
            .detected_agent = Some(agent);
    }
    server.app.state.active = Some(0);
    server.app.state.selected = 0;
    server.app.state.mode = crate::app::Mode::Terminal;
    GridFixture {
        server,
        shell,
        first_agent,
        second_agent,
        second_input,
    }
}

fn set_grid(server: &mut HeadlessServer, client_id: u64, active: bool) -> bool {
    server.handle_server_event(ServerEvent::ClientShellEndpointRequest {
        client_id,
        boot_id: server.client_shell_boot_id.clone(),
        request: Box::new(api::schema::Request {
            id: format!("grid-{active}"),
            method: api::schema::Method::ClientShellAgentGridSet(
                api::schema::ClientShellAgentGridSetParams { active },
            ),
        }),
    })
}

fn runtime_size(
    server: &HeadlessServer,
    workspace_index: usize,
    pane_id: crate::layout::PaneId,
) -> (u16, u16) {
    server
        .app
        .state
        .runtime_for_pane_in_workspace(&server.app.terminal_runtimes, workspace_index, pane_id)
        .expect("runtime")
        .current_size()
}

#[tokio::test]
async fn agent_grid_shows_live_agents_across_workspaces_without_shells() {
    let GridFixture {
        mut server,
        shell,
        first_agent,
        second_agent,
        mut second_input,
    } = grid_fixture();
    let (control, render) = connect_test_shell(&mut server, 7, 100, 30);
    let _ = client_shell_snapshot(&control);
    server.render_and_stream();
    let tab_surface = recv_pane_surface(&render, "tab surface");
    assert!(frame_text(&tab_surface.frame).contains("PLAIN-SHELL"));

    assert!(set_grid(&mut server, 7, true));
    let _ = control.recv().expect("grid response");
    server.render_and_stream();
    let grid = recv_pane_surface(&render, "grid surface");
    let text = frame_text(&grid.frame);
    assert!(text.contains("FIRST-AGENT"), "{text}");
    assert!(!text.contains("PLAIN-SHELL"), "{text}");
    assert_eq!(
        grid.panes
            .iter()
            .map(|pane| pane.pane_id.clone())
            .collect::<Vec<_>>(),
        vec![
            server.app.public_pane_id(0, first_agent).unwrap(),
            server.app.public_pane_id(1, second_agent).unwrap(),
        ]
    );
    assert!(grid.splits.is_empty());

    assert!(server.pty_sources_visible_to_any_render_target(&HashSet::from([second_agent])));
    assert!(!server.pty_sources_visible_to_any_render_target(&HashSet::from([shell])));

    server.app.state.workspaces[0].test_runtimes[&first_agent]
        .test_process_pty_bytes(b"\rUPDATED-AGENT");
    assert!(server.render_retained_pane_surface_and_stream(&HashSet::from([first_agent])));
    let patch = recv_pane_surface_patch(&render, "grid patch");
    assert_eq!(patch.panes.len(), 1);

    server.handle_server_event(ServerEvent::ClientShellPaneInput {
        client_id: 7,
        pane_id: server.app.public_pane_id(1, second_agent).unwrap(),
        events: vec![crate::protocol::ClientPaneInputEvent::Paste("x".into())],
    });
    assert!(
        second_input.try_recv().is_ok(),
        "grid routes input to an agent in another workspace"
    );
    assert!(
        !server.handle_server_event(ServerEvent::ClientShellPaneInput {
            client_id: 7,
            pane_id: server.app.public_pane_id(0, shell).unwrap(),
            events: vec![crate::protocol::ClientPaneInputEvent::Paste(
                "hidden".into()
            )],
        })
    );

    shutdown_test_runtimes(&mut server);
}

#[tokio::test]
async fn agent_grid_owns_agent_geometry_until_it_closes() {
    let GridFixture {
        mut server,
        shell,
        first_agent,
        second_agent,
        ..
    } = grid_fixture();
    let (control, render) = connect_test_shell(&mut server, 7, 100, 30);
    let _ = client_shell_snapshot(&control);
    server.render_and_stream();
    let _ = recv_pane_surface(&render, "tab surface");
    let tab_size = runtime_size(&server, 0, shell);
    assert_eq!(runtime_size(&server, 0, first_agent), tab_size);

    assert!(set_grid(&mut server, 7, true));
    let _ = control.recv().expect("grid response");
    server.render_and_stream();
    let grid = recv_pane_surface(&render, "grid surface");
    for (workspace_index, pane_id) in [(0, first_agent), (1, second_agent)] {
        let public_id = server.app.public_pane_id(workspace_index, pane_id).unwrap();
        let tile = grid
            .panes
            .iter()
            .find(|pane| pane.pane_id == public_id)
            .expect("grid tile");
        assert_eq!(
            runtime_size(&server, workspace_index, pane_id),
            (tile.inner_rect.height, tile.inner_rect.width),
            "agents are sized to their tile"
        );
    }
    assert_eq!(server.app.state.agent_grid_resize_locks.len(), 2);

    // Tab geometry must not pull a grid agent back to its tab size.
    server.resize_tabs_for_only_shell_client(false);
    assert_ne!(runtime_size(&server, 0, first_agent), tab_size);

    assert!(set_grid(&mut server, 7, false));
    let _ = control.recv().expect("grid close response");
    assert!(server.app.state.agent_grid_resize_locks.is_empty());
    assert_eq!(runtime_size(&server, 0, first_agent), tab_size);

    shutdown_test_runtimes(&mut server);
}

#[tokio::test]
async fn excluding_an_agent_returns_it_to_tab_geometry_and_regrids_the_rest() {
    let GridFixture {
        mut server,
        shell,
        first_agent,
        second_agent,
        ..
    } = grid_fixture();
    let (control, render) = connect_test_shell(&mut server, 7, 100, 30);
    let _ = client_shell_snapshot(&control);
    server.render_and_stream();
    let _ = recv_pane_surface(&render, "tab surface");
    let tab_size = runtime_size(&server, 0, shell);
    assert!(set_grid(&mut server, 7, true));
    let _ = control.recv().expect("grid response");
    server.render_and_stream();
    let _ = recv_pane_surface(&render, "grid surface");
    assert_ne!(runtime_size(&server, 0, first_agent), tab_size);

    let first_id = server.app.public_pane_id(0, first_agent).unwrap();
    assert!(
        server.handle_server_event(ServerEvent::ClientShellEndpointRequest {
            client_id: 7,
            boot_id: server.client_shell_boot_id.clone(),
            request: Box::new(api::schema::Request {
                id: "exclude".into(),
                method: api::schema::Method::PaneAgentGridSet(
                    api::schema::PaneAgentGridSetParams {
                        pane_id: first_id.clone(),
                        excluded: true,
                    },
                ),
            }),
        }),
        "leaving an agent out repaints"
    );
    server.render_and_stream();
    let grid = recv_pane_surface(&render, "regridded surface");
    let second_id = server.app.public_pane_id(1, second_agent).unwrap();
    assert_eq!(
        grid.panes
            .iter()
            .map(|pane| pane.pane_id.as_str())
            .collect::<Vec<_>>(),
        vec![second_id.as_str()],
        "the remaining agent takes the whole grid"
    );
    assert_eq!(server.app.state.agent_grid_resize_locks.len(), 1);
    assert_eq!(
        runtime_size(&server, 0, first_agent),
        tab_size,
        "the excluded agent returns to its tab size"
    );
    let snapshot = server.clients[&7]
        .shell_snapshot
        .as_ref()
        .expect("snapshot");
    assert!(snapshot
        .panes
        .iter()
        .any(|pane| pane.pane_id == first_id && pane.agent_grid_excluded));
    assert!(!snapshot
        .panes
        .iter()
        .any(|pane| pane.pane_id == second_id && pane.agent_grid_excluded));

    shutdown_test_runtimes(&mut server);
}
