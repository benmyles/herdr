use super::*;

#[tokio::test]
async fn client_shell_reopens_dormant_pin_and_follows_it() {
    let mut server = test_headless_server();
    server.app.state.shell_mode = crate::config::ShellModeConfig::NonLogin;
    server.app.state.workspaces = vec![crate::workspace::Workspace::test_new("live")];
    server.app.state.ensure_test_terminals();
    server.app.state.active = Some(0);
    server.app.state.selected = 0;
    server.app.state.mode = crate::app::Mode::Terminal;
    let dormant_workspace_id = crate::workspace::Workspace::test_new("dormant").id;
    let pin = crate::space::PinnedSpace::new(
        crate::space::PinnedSpaceKey::Workspace {
            workspace_id: dormant_workspace_id.clone(),
        },
        "dormant".into(),
        std::env::current_dir().expect("current dir"),
        1,
        None,
    );
    let space_id = pin.id.clone();
    server.app.state.pinned_spaces.push(pin);

    let (control, _render) = connect_matching_test_shell(&mut server, 61);
    let initial = client_shell_snapshot(&control);
    assert!(initial
        .pinned_spaces
        .iter()
        .any(|pin| pin.space_id == space_id && !pin.live));

    assert!(
        server.handle_server_event(ServerEvent::ClientShellEndpointRequest {
            client_id: 61,
            boot_id: server.client_shell_boot_id.clone(),
            request: Box::new(api::schema::Request {
                id: "open-pin".into(),
                method: api::schema::Method::SpaceOpen(api::schema::SpaceTarget {
                    space_id: space_id.clone(),
                }),
            }),
        })
    );
    let response_ready = server
        .server_event_rx
        .recv()
        .await
        .expect("open response ready");
    server.handle_server_event(response_ready);
    let _ = control.recv().expect("open response");

    server.render_and_stream();
    let replacement = client_shell_snapshot(&control);
    let reopened = replacement
        .workspaces
        .iter()
        .find(|workspace| workspace.workspace_id == dormant_workspace_id)
        .expect("reopened workspace keeps the pinned identity");
    assert!(reopened.focused, "the requesting shell follows the pin");
    assert_eq!(reopened.pinned_space_id.as_deref(), Some(space_id.as_str()));
    assert!(replacement.pinned_spaces.iter().all(|pin| pin.live));
    server.app.state.assert_invariants_for_test();
    shutdown_test_runtimes(&mut server);
}
