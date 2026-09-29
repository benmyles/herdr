use super::*;

#[tokio::test]
async fn client_shell_reopens_closed_space_member_and_follows_it() {
    let mut server = test_headless_server();
    server.app.state.shell_mode = crate::config::ShellModeConfig::NonLogin;
    server.app.state.workspaces = vec![crate::workspace::Workspace::test_new("live")];
    server.app.state.ensure_test_terminals();
    server.app.state.active = Some(0);
    server.app.state.selected = 0;
    server.app.state.mode = crate::app::Mode::Terminal;
    server.app.state.normalize_spaces();
    let space_id = server
        .app
        .state
        .create_space("knowledge")
        .expect("create space");
    let member = crate::space::ClosedMember::new(
        "repo".into(),
        std::env::current_dir().expect("current dir"),
        None,
        Some("knowledge".into()),
        None,
    );
    let member_id = member.id.clone();
    server.app.state.spaces[0].closed.push(member);

    let (control, _render) = connect_matching_test_shell(&mut server, 61);
    let initial = client_shell_snapshot(&control);
    assert!(initial
        .spaces
        .iter()
        .any(|space| space.space_id == space_id && space.closed.len() == 1));

    assert!(
        server.handle_server_event(ServerEvent::ClientShellEndpointRequest {
            client_id: 61,
            boot_id: server.client_shell_boot_id.clone(),
            request: Box::new(api::schema::Request {
                id: "open-member".into(),
                method: api::schema::Method::SpaceMemberOpen(api::schema::SpaceMemberTarget {
                    space_id: space_id.clone(),
                    member_id,
                    focus: true,
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
        .find(|workspace| workspace.space_id.as_deref() == Some(space_id.as_str()))
        .expect("reopened workspace is filed under its space");
    assert!(
        reopened.focused,
        "the requesting shell follows the reopened member"
    );
    assert!(replacement
        .spaces
        .iter()
        .all(|space| space.closed.is_empty()));
    server.app.state.assert_invariants_for_test();
    shutdown_test_runtimes(&mut server);
}
