use super::super::command_palette::{rank, PaletteKind};
use super::*;
use crate::protocol::ClientShellCommand;

/// A shell in the first tab, a "fix the grid" Claude agent in a second tab,
/// and one custom command.
fn palette_snapshot() -> ClientShellSnapshot {
    let mut projected = snapshot();
    let mut second_tab = projected.tabs[0].clone();
    second_tab.tab_id = "tab_2".into();
    second_tab.number = 2;
    second_tab.label = "review".into();
    second_tab.custom_label = true;
    second_tab.focused = false;
    projected.tabs.push(second_tab);
    let mut agent_pane = projected.panes[0].clone();
    agent_pane.pane_id = "pane_2".into();
    agent_pane.tab_id = "tab_2".into();
    agent_pane.focused = false;
    projected.panes.push(agent_pane);
    projected.agents = vec![ClientShellAgent {
        pane_id: "pane_2".into(),
        workspace_id: "ws_1".into(),
        tab_id: "tab_2".into(),
        name: None,
        display_agent: None,
        agent: Some("claude".into()),
        title: None,
        terminal_title: None,
        terminal_title_stripped: Some("fix the grid".into()),
        agent_status: AgentStatus::Blocked,
        state_change_seq: 1,
        state_labels: Vec::new(),
        tokens: Vec::new(),
        focused: false,
        state_changed_at_ms: None,
    }];
    projected.commands = vec![ClientShellCommand {
        command_id: "cmd_lazygit".into(),
        binding_label: "prefix+shift+l".into(),
        binding_labels: vec!["prefix+shift+l".into()],
        action: crate::protocol::ClientShellCommandAction::Pane,
        description: Some("open lazygit".into()),
    }];
    projected
}

fn palette_state() -> ClientShellState {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(palette_snapshot()));
    state.set_pane_surface(surface());
    state.compose(106, 30).expect("shell frame");
    state
}

/// Opens the palette with the default `prefix+space` and types `query`.
fn open_palette(state: &mut ClientShellState, query: &str) {
    state.handle_input_bytes(&[0x02]);
    state.handle_input_bytes(b" ");
    assert!(
        matches!(state.overlay, Some(ClientShellOverlay::CommandPalette(_))),
        "prefix+space opens the palette"
    );
    if !query.is_empty() {
        state.handle_input_bytes(query.as_bytes());
    }
    state.set_pane_surface(surface());
    state.compose(106, 30).expect("palette frame");
}

fn results(state: &ClientShellState) -> Vec<(PaletteKind, String)> {
    let entries = state.command_palette_entries();
    let Some(ClientShellOverlay::CommandPalette(palette)) = state.overlay.as_ref() else {
        return Vec::new();
    };
    rank(&entries, palette.query.as_str())
        .into_iter()
        .map(|found| (found.entry.kind, found.entry.title.clone()))
        .collect()
}

fn key(state: &mut ClientShellState, code: KeyCode) -> ClientShellInput {
    state.handle_raw_events(vec![RawInputEvent::Key(crate::input::TerminalKey::new(
        code,
        KeyModifiers::empty(),
    ))])
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

fn pane_focus(pane_id: &str) -> crate::api::schema::Method {
    crate::api::schema::Method::PaneFocus(crate::api::schema::PaneTarget {
        pane_id: pane_id.into(),
    })
}

#[test]
fn palette_lists_places_views_settings_and_every_command() {
    let mut state = palette_state();
    open_palette(&mut state, "");
    let entries = state.command_palette_entries();
    let has = |kind: PaletteKind, title: &str| {
        entries
            .iter()
            .any(|entry| entry.kind == kind && entry.title == title)
    };

    assert!(has(PaletteKind::Agent, "fix the grid"));
    assert!(
        has(PaletteKind::Terminal, "repo"),
        "shells are named by folder"
    );
    assert!(
        !has(PaletteKind::Terminal, "fix the grid"),
        "agents are listed once"
    );
    assert!(has(PaletteKind::Workspace, "client-shell"));
    assert!(has(PaletteKind::Tab, "review"));
    assert!(has(PaletteKind::View, "show agent grid"));
    for section in ClientSettingsSection::ALL {
        assert!(has(PaletteKind::Settings, section.label()));
    }
    for (_, action) in crate::input::action_bindings(&state.config.keybinds.keybinds) {
        assert_eq!(
            has(PaletteKind::Command, action.label()),
            action != crate::input::KeybindAction::CommandPalette,
            "{}",
            action.label()
        );
    }
    assert!(has(PaletteKind::Command, "open lazygit"));
    let new_tab = entries
        .iter()
        .find(|entry| entry.title == "new tab")
        .expect("new tab command");
    assert_eq!(new_tab.hint.as_deref(), Some("prefix+c"));
    let agent = entries
        .iter()
        .find(|entry| entry.kind == PaletteKind::Agent)
        .expect("agent entry");
    assert!(
        agent.detail.contains("client-shell/review"),
        "{}",
        agent.detail
    );
    assert!(agent.detail.contains("blocked"), "{}", agent.detail);
}

#[test]
fn fuzzy_query_ranks_the_closest_title_first_and_runs_it() {
    let mut state = palette_state();
    open_palette(&mut state, "tgl sdbr");
    assert_eq!(
        results(&state).first(),
        Some(&(PaletteKind::Command, "toggle sidebar".to_owned()))
    );

    let run = state.handle_input_bytes(b"\r");
    assert!(state.overlay.is_none());
    assert!(run.repaint);
    assert!(
        state.sidebar_collapsed,
        "the command ran like its keybinding"
    );
}

#[test]
fn jumping_to_an_agent_focuses_its_pane() {
    let mut state = palette_state();
    open_palette(&mut state, "fix grid");
    assert_eq!(
        results(&state).first(),
        Some(&(PaletteKind::Agent, "fix the grid".to_owned()))
    );
    let run = state.handle_input_bytes(b"\r");
    assert_eq!(methods(&run), vec![pane_focus("pane_2")]);
}

#[test]
fn terms_can_match_the_kind_so_settings_theme_opens_that_tab() {
    let mut state = palette_state();
    open_palette(&mut state, "settings sound");
    assert_eq!(
        results(&state).first(),
        Some(&(PaletteKind::Settings, "sound".to_owned()))
    );
    state.handle_input_bytes(b"\r");
    assert!(matches!(
        state.overlay,
        Some(ClientShellOverlay::Settings(ClientSettingsOverlay {
            section: ClientSettingsSection::Sound,
            ..
        }))
    ));
}

#[test]
fn custom_commands_run_through_the_endpoint() {
    let mut state = palette_state();
    open_palette(&mut state, "lazygit");
    let run = state.handle_input_bytes(b"\r");
    let [crate::api::schema::Method::CommandInvoke(params)] = &methods(&run)[..] else {
        panic!("expected a command invocation: {:?}", methods(&run));
    };
    assert_eq!(params.command_id, "cmd_lazygit");
    assert_eq!(params.pane_id.as_deref(), Some("pane_1"));
}

#[test]
fn jumping_to_a_shell_leaves_the_agent_grid() {
    let mut state = palette_state();
    state.show_test_agent_grid();
    open_palette(&mut state, "repo");
    assert_eq!(
        results(&state).first(),
        Some(&(PaletteKind::Terminal, "repo".to_owned()))
    );
    let run = state.handle_input_bytes(b"\r");
    assert_eq!(
        methods(&run),
        vec![
            crate::api::schema::Method::ClientShellAgentGridSet(
                crate::api::schema::ClientShellAgentGridSetParams { active: false },
            ),
            pane_focus("pane_1"),
        ],
        "a shell has no tile, so its tab replaces the grid"
    );
}

#[test]
fn arrows_move_the_selection_tab_completes_and_escape_closes() {
    let mut state = palette_state();
    open_palette(&mut state, "pane");
    let before = results(&state);
    key(&mut state, KeyCode::Down);
    let Some(ClientShellOverlay::CommandPalette(palette)) = state.overlay.as_ref() else {
        panic!("palette stays open");
    };
    assert_eq!(palette.selected, 1);

    key(&mut state, KeyCode::Tab);
    let Some(ClientShellOverlay::CommandPalette(palette)) = state.overlay.as_ref() else {
        panic!("palette stays open");
    };
    assert_eq!(palette.query.as_str(), before[1].1);
    assert_eq!(palette.selected, 0);

    key(&mut state, KeyCode::Esc);
    assert!(state.overlay.is_none());
}

#[test]
fn palette_draws_matches_with_their_keybindings_and_takes_clicks() {
    let mut state = palette_state();
    open_palette(&mut state, "new tab");
    let frame = state.compose(106, 30).expect("palette frame");
    let text = frame_rows(&frame).join("\n");
    assert!(text.contains("Command palette"), "{text}");
    assert!(text.contains("new tab"), "{text}");
    assert!(text.contains("prefix+c"), "{text}");
    assert!(text.contains("command"), "{text}");

    let (row, index) = state.hits.command_palette_rows[0];
    assert_eq!(index, 0);
    let outside = state.hits.command_palette_popup.bottom() + 1;
    state.handle_raw_events(vec![RawInputEvent::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: row.x,
        row: outside,
        modifiers: KeyModifiers::empty(),
    })]);
    assert!(
        state.overlay.is_none(),
        "a click outside closes the palette"
    );

    open_palette(&mut state, "toggle sidebar");
    let (row, _) = state.hits.command_palette_rows[0];
    state.handle_raw_events(vec![RawInputEvent::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: row.x + 2,
        row: row.y,
        modifiers: KeyModifiers::empty(),
    })]);
    assert!(state.overlay.is_none());
    assert!(state.sidebar_collapsed, "clicking a result runs it");
}

#[test]
fn results_are_lettered_and_ctrl_letter_opens_one() {
    let mut state = palette_state();
    open_palette(&mut state, "sidebar");
    let frame = state.compose(106, 30).expect("palette frame");
    let rows = frame_rows(&frame);
    let (first, _) = state.hits.command_palette_rows[0];
    let row = rows[first.y as usize]
        .chars()
        .skip(first.x as usize)
        .collect::<String>();
    assert!(row.starts_with(" a "), "{row}");
    let found = results(&state);
    assert_eq!(found[0].1, "toggle sidebar");
    // Ctrl with the letter after the last result opens nothing; the control
    // byte stays below ctrl+h, which terminals send as backspace.
    assert!(found.len() < 7, "{found:?}");
    state.handle_input_bytes(&[0x01 + found.len() as u8]);
    assert!(matches!(
        state.overlay,
        Some(ClientShellOverlay::CommandPalette(_))
    ));

    state.handle_input_bytes(&[0x01]);
    assert!(state.overlay.is_none());
    assert!(state.sidebar_collapsed, "ctrl+a opened the first result");
}

#[test]
fn spaces_are_optional_between_a_place_and_its_name() {
    let mut projected = palette_snapshot();
    projected.workspaces[0].label = "api".into();
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(projected));
    state.set_pane_surface(surface());
    state.compose(106, 30).expect("shell frame");

    for query in ["a1", "a 1"] {
        open_palette(&mut state, query);
        assert!(
            results(&state).contains(&(PaletteKind::Tab, "1".to_owned())),
            "{query:?} finds tab 1 of api: {:?}",
            results(&state)
        );
        key(&mut state, KeyCode::Esc);
    }
}

#[test]
fn chosen_items_rank_first_and_their_usage_is_saved() {
    let dir = std::env::temp_dir().join(format!(
        "herdr-palette-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let path = dir.join("command-palette.json");
    let stateful = || {
        let mut state = ClientShellState::new(
            ClientShellConfig::from_config(&Config::default())
                .with_palette_usage_path(path.clone()),
        );
        state.set_snapshot(Box::new(palette_snapshot()));
        state.set_pane_surface(surface());
        state.compose(106, 30).expect("shell frame");
        state
    };

    let mut state = stateful();
    open_palette(&mut state, "pane left");
    let before = results(&state);
    assert_ne!(before[0].1, "resize pane left");
    let resize = before
        .iter()
        .position(|(_, title)| title == "resize pane left")
        .expect("resize pane left matches");
    for _ in 0..resize {
        key(&mut state, KeyCode::Down);
    }
    state.handle_input_bytes(b"\r");
    assert!(path.exists(), "the pick is saved to the state directory");

    // A new client reads the saved usage back.
    let mut state = stateful();
    open_palette(&mut state, "pane left");
    assert_eq!(
        results(&state)[0].1,
        "resize pane left",
        "a past pick ranks first"
    );
    key(&mut state, KeyCode::Esc);
    open_palette(&mut state, "");
    assert_eq!(
        results(&state)[0].1,
        "resize pane left",
        "with no query the most chosen come first"
    );
    std::fs::remove_dir_all(dir).unwrap();
}
