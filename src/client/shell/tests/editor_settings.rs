use super::*;
use crate::editor::KnownEditor;

/// Points the config file at a fresh directory for the test, and back.
struct TempConfig {
    dir: std::path::PathBuf,
    path: std::path::PathBuf,
    previous: Option<std::ffi::OsString>,
}

impl TempConfig {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("herdr-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        let previous = std::env::var_os(crate::config::CONFIG_PATH_ENV_VAR);
        std::env::set_var(crate::config::CONFIG_PATH_ENV_VAR, &path);
        Self {
            dir,
            path,
            previous,
        }
    }

    fn editor(&self) -> toml::Value {
        let content = std::fs::read_to_string(&self.path).unwrap();
        toml::from_str::<toml::Value>(&content).unwrap()["ui"]["editor"].clone()
    }
}

impl Drop for TempConfig {
    fn drop(&mut self) {
        match self.previous.take() {
            Some(value) => std::env::set_var(crate::config::CONFIG_PATH_ENV_VAR, value),
            None => std::env::remove_var(crate::config::CONFIG_PATH_ENV_VAR),
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn editor_state(installed: Vec<KnownEditor>) -> ClientShellState {
    let mut state = ClientShellState::new(
        ClientShellConfig::from_config(&Config::default()).with_installed_editors(installed),
    );
    state.set_snapshot(Box::new(snapshot()));
    state.set_pane_surface(surface());
    state.open_editor_settings(&mut ClientShellInput::default());
    state
}

fn choose(state: &mut ClientShellState, index: usize) -> ClientShellInput {
    let mut outcome = ClientShellInput::default();
    state.select_settings_choice(index);
    state.apply_settings_choice(&mut outcome);
    outcome
}

fn editor_tab_selection(state: &ClientShellState) -> Option<usize> {
    match &state.overlay {
        Some(ClientShellOverlay::Settings(settings))
            if settings.section == ClientSettingsSection::Editor =>
        {
            Some(settings.selected)
        }
        _ => None,
    }
}

fn type_command(state: &mut ClientShellState, command: &str) -> ClientShellInput {
    let Some(ClientShellOverlay::Rename(rename)) = state.overlay.as_mut() else {
        panic!("expected the editor command prompt");
    };
    rename.input = TextEditor::new(command, false);
    let mut outcome = ClientShellInput::default();
    state.save_rename_overlay(&mut outcome);
    outcome
}

#[test]
fn the_editor_tab_shows_which_editors_are_on_path() {
    let mut state = editor_state(vec![KnownEditor::VsCode, KnownEditor::Cursor]);
    let frame = state.compose(106, 30).expect("frame");
    let text = frame_rows(&frame).join("\n");
    for expected in [
        "automatic        uses VS Code ✓",
        "VS Code          code on PATH",
        "Zed              zed not found",
        "Cursor           cursor on PATH",
        "custom command…",
    ] {
        assert!(text.contains(expected), "no {expected:?} in\n{text}");
    }

    let mut state = editor_state(Vec::new());
    let frame = state.compose(106, 30).expect("frame");
    let text = frame_rows(&frame).join("\n");
    assert!(
        text.contains("automatic        no editor found on PATH"),
        "{text}"
    );
}

#[test]
fn choosing_an_editor_or_a_custom_command_saves_it_to_the_config() {
    let config = TempConfig::new("editor-settings");
    let mut state = editor_state(vec![KnownEditor::VsCode]);
    assert_eq!(
        editor_tab_selection(&state),
        Some(0),
        "automatic by default"
    );

    choose(&mut state, 2);
    assert_eq!(config.editor()["app"].as_str(), Some("zed"));
    assert_eq!(state.config.editor.app, crate::config::EditorAppConfig::Zed);
    assert_eq!(editor_tab_selection(&state), Some(2), "the tab stays open");

    // The custom row asks for the command and comes back to the tab.
    choose(&mut state, 4);
    assert!(matches!(
        state.overlay,
        Some(ClientShellOverlay::Rename(ClientRenameOverlay {
            target: ClientRenameTarget::EditorCommand,
            ..
        }))
    ));
    type_command(&mut state, "code \"unclosed");
    assert!(
        matches!(state.overlay, Some(ClientShellOverlay::Rename(_))),
        "an unmatched quote keeps the prompt open"
    );
    assert!(state.endpoint_error.is_some());
    type_command(&mut state, "subl --new-window");
    assert_eq!(config.editor()["app"].as_str(), Some("custom"));
    assert_eq!(
        config.editor()["command"].as_str(),
        Some("subl --new-window")
    );
    assert_eq!(state.config.editor.command, "subl --new-window");
    assert_eq!(editor_tab_selection(&state), Some(4));

    // Cancelling the prompt also returns to the tab and changes nothing.
    choose(&mut state, 4);
    let mut outcome = ClientShellInput::default();
    state.cancel_rename_overlay(&mut outcome);
    assert_eq!(editor_tab_selection(&state), Some(4));
    assert_eq!(state.config.editor.command, "subl --new-window");
}
