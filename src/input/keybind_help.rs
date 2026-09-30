use std::borrow::Cow;

use crossterm::event::{KeyCode, KeyModifiers};

use crate::{
    config::{ActionKeybinds, IndexedKeybind, Keybinds},
    input::{KeybindAction, TerminalKey},
};

pub(crate) type KeybindHelpEntry = (String, Cow<'static, str>);
pub(crate) type KeybindHelpGroup = (&'static str, Vec<KeybindHelpEntry>);

pub(crate) fn keybind_help_text_char(key: &TerminalKey) -> Option<char> {
    if !key.modifiers.difference(KeyModifiers::SHIFT).is_empty() {
        return None;
    }
    if let Some(character) = key.shifted_codepoint.and_then(char::from_u32) {
        return Some(character);
    }
    let KeyCode::Char(character) = key.code else {
        return None;
    };
    Some(character)
}

fn entry(key: impl Into<String>, label: &'static str) -> KeybindHelpEntry {
    (key.into(), Cow::Borrowed(label))
}

fn binding_label(bindings: &ActionKeybinds) -> String {
    bindings.label().unwrap_or_else(|| "unset".to_owned())
}

fn indexed_label(bindings: &[IndexedKeybind]) -> String {
    if bindings.is_empty() {
        return "unset".to_owned();
    }
    let mut parts = Vec::new();
    let mut index = 0;
    while index < bindings.len() {
        if let Some(prefix) = indexed_range_prefix(&bindings[index..]) {
            parts.push(format!("{prefix}1..9"));
            index += 9;
        } else {
            parts.push(bindings[index].label.clone());
            index += 1;
        }
    }
    parts.join(" / ")
}

fn indexed_range_prefix(bindings: &[IndexedKeybind]) -> Option<&str> {
    let run = bindings.get(..9)?;
    let prefix = run[0].label.strip_suffix('1')?;
    for (offset, binding) in run.iter().enumerate() {
        let digit = char::from(b'1' + offset as u8);
        if binding.label.strip_suffix(digit) != Some(prefix) {
            return None;
        }
    }
    Some(prefix)
}

pub(crate) fn keybind_help_groups(
    keybinds: &Keybinds,
    prefix: (crossterm::event::KeyCode, crossterm::event::KeyModifiers),
) -> Vec<KeybindHelpGroup> {
    let mut groups = vec![
        (
            "global",
            vec![
                entry(crate::config::format_key_combo(prefix), "prefix mode"),
                entry(binding_label(&keybinds.help), KeybindAction::Help.label()),
                entry(
                    binding_label(&keybinds.settings),
                    KeybindAction::Settings.label(),
                ),
                entry(
                    binding_label(&keybinds.detach),
                    KeybindAction::Detach.label(),
                ),
                entry(
                    binding_label(&keybinds.reload_config),
                    KeybindAction::ReloadConfig.label(),
                ),
                entry(
                    binding_label(&keybinds.open_notification_target),
                    KeybindAction::OpenNotificationTarget.label(),
                ),
            ],
        ),
        (
            "navigation",
            vec![
                entry("esc", "back"),
                entry(
                    format!(
                        "{} / {}",
                        binding_label(&keybinds.navigate.workspace_up),
                        binding_label(&keybinds.navigate.workspace_down)
                    ),
                    "workspace list",
                ),
                entry(
                    format!(
                        "{} / {} / {} / {} / left / right",
                        binding_label(&keybinds.navigate.pane_left),
                        binding_label(&keybinds.navigate.pane_down),
                        binding_label(&keybinds.navigate.pane_up),
                        binding_label(&keybinds.navigate.pane_right)
                    ),
                    "move focus",
                ),
                entry("tab / shift+tab", "cycle pane"),
                entry("enter", "open workspace"),
                entry("1..9", "switch workspace"),
            ],
        ),
        (
            "workspaces / tabs",
            vec![
                entry(
                    binding_label(&keybinds.workspace_picker),
                    KeybindAction::WorkspacePicker.label(),
                ),
                entry(
                    binding_label(&keybinds.goto),
                    KeybindAction::OpenNavigator.label(),
                ),
                entry(
                    binding_label(&keybinds.command_palette),
                    KeybindAction::CommandPalette.label(),
                ),
                entry(
                    binding_label(&keybinds.new_workspace),
                    KeybindAction::NewWorkspace.label(),
                ),
                entry(
                    binding_label(&keybinds.new_worktree),
                    KeybindAction::NewWorktree.label(),
                ),
                entry(
                    binding_label(&keybinds.open_worktree),
                    KeybindAction::OpenWorktree.label(),
                ),
                entry(
                    binding_label(&keybinds.remove_worktree),
                    KeybindAction::RemoveWorktree.label(),
                ),
                entry(
                    binding_label(&keybinds.rename_workspace),
                    KeybindAction::RenameWorkspace.label(),
                ),
                entry(
                    binding_label(&keybinds.close_workspace),
                    KeybindAction::CloseWorkspace.label(),
                ),
                entry(
                    binding_label(&keybinds.previous_workspace),
                    KeybindAction::PreviousWorkspace.label(),
                ),
                entry(
                    binding_label(&keybinds.next_workspace),
                    KeybindAction::NextWorkspace.label(),
                ),
                entry(
                    indexed_label(&keybinds.switch_workspace),
                    KeybindAction::SwitchWorkspace(0).label(),
                ),
                entry(
                    binding_label(&keybinds.previous_agent),
                    KeybindAction::PreviousAgent.label(),
                ),
                entry(
                    binding_label(&keybinds.next_agent),
                    KeybindAction::NextAgent.label(),
                ),
                entry(
                    indexed_label(&keybinds.focus_agent),
                    KeybindAction::FocusAgent(0).label(),
                ),
                entry(
                    binding_label(&keybinds.new_tab),
                    KeybindAction::NewTab.label(),
                ),
                entry(
                    binding_label(&keybinds.rename_tab),
                    KeybindAction::RenameTab.label(),
                ),
                entry(
                    binding_label(&keybinds.previous_tab),
                    KeybindAction::PreviousTab.label(),
                ),
                entry(
                    binding_label(&keybinds.next_tab),
                    KeybindAction::NextTab.label(),
                ),
                entry(
                    binding_label(&keybinds.move_tab_previous),
                    KeybindAction::MoveTabPrevious.label(),
                ),
                entry(
                    binding_label(&keybinds.move_tab_next),
                    KeybindAction::MoveTabNext.label(),
                ),
                entry(
                    indexed_label(&keybinds.switch_tab),
                    KeybindAction::SwitchTab(0).label(),
                ),
                entry(
                    binding_label(&keybinds.close_tab),
                    KeybindAction::CloseTab.label(),
                ),
            ],
        ),
        (
            "panes",
            vec![
                entry(
                    binding_label(&keybinds.split_vertical),
                    KeybindAction::SplitVertical.label(),
                ),
                entry(
                    binding_label(&keybinds.split_horizontal),
                    KeybindAction::SplitHorizontal.label(),
                ),
                entry(
                    binding_label(&keybinds.close_pane),
                    KeybindAction::ClosePane.label(),
                ),
                entry(
                    binding_label(&keybinds.rename_pane),
                    KeybindAction::RenamePane.label(),
                ),
                entry(
                    binding_label(&keybinds.edit_scrollback),
                    KeybindAction::EditScrollback.label(),
                ),
                entry(
                    binding_label(&keybinds.clear_pane),
                    KeybindAction::ClearPane.label(),
                ),
                entry(
                    binding_label(&keybinds.copy_mode),
                    KeybindAction::CopyMode.label(),
                ),
                entry(binding_label(&keybinds.zoom), KeybindAction::Zoom.label()),
                entry(
                    binding_label(&keybinds.resize_mode),
                    KeybindAction::EnterResizeMode.label(),
                ),
                entry(
                    binding_label(&keybinds.resize_pane_left),
                    KeybindAction::ResizePaneLeft.label(),
                ),
                entry(
                    binding_label(&keybinds.resize_pane_down),
                    KeybindAction::ResizePaneDown.label(),
                ),
                entry(
                    binding_label(&keybinds.resize_pane_up),
                    KeybindAction::ResizePaneUp.label(),
                ),
                entry(
                    binding_label(&keybinds.resize_pane_right),
                    KeybindAction::ResizePaneRight.label(),
                ),
                entry(
                    binding_label(&keybinds.toggle_sidebar),
                    KeybindAction::ToggleSidebar.label(),
                ),
                entry(
                    binding_label(&keybinds.focus_pane_left),
                    KeybindAction::FocusPaneLeft.label(),
                ),
                entry(
                    binding_label(&keybinds.focus_pane_down),
                    KeybindAction::FocusPaneDown.label(),
                ),
                entry(
                    binding_label(&keybinds.focus_pane_up),
                    KeybindAction::FocusPaneUp.label(),
                ),
                entry(
                    binding_label(&keybinds.focus_pane_right),
                    KeybindAction::FocusPaneRight.label(),
                ),
                entry(
                    binding_label(&keybinds.cycle_pane_next),
                    KeybindAction::CyclePaneNext.label(),
                ),
                entry(
                    binding_label(&keybinds.cycle_pane_previous),
                    KeybindAction::CyclePanePrevious.label(),
                ),
                entry(
                    binding_label(&keybinds.last_pane),
                    KeybindAction::LastPane.label(),
                ),
            ],
        ),
    ];

    if !keybinds.custom_commands.is_empty() {
        groups.push((
            "custom",
            keybinds
                .custom_commands
                .iter()
                .map(|binding| {
                    (
                        binding.label.clone(),
                        binding
                            .description
                            .clone()
                            .map(Cow::Owned)
                            .unwrap_or(Cow::Borrowed("custom command")),
                    )
                })
                .collect(),
        ));
    }
    groups
}

pub(crate) fn filter_keybind_help_groups(
    groups: Vec<KeybindHelpGroup>,
    query: &str,
) -> Vec<KeybindHelpGroup> {
    if query.is_empty() {
        return groups;
    }
    let query = query.to_lowercase();
    groups
        .into_iter()
        .filter_map(|(group, entries)| {
            let entries = entries
                .into_iter()
                .filter(|(key, label)| {
                    key.to_lowercase().contains(&query) || label.to_lowercase().contains(&query)
                })
                .collect::<Vec<_>>();
            (!entries.is_empty()).then_some((group, entries))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn groups() -> Vec<KeybindHelpGroup> {
        vec![
            (
                "workspaces / tabs",
                vec![entry("w", "workspace navigation"), entry("c", "new tab")],
            ),
            (
                "panes",
                vec![entry("v", "split vertical"), entry("x", "close pane")],
            ),
        ]
    }

    #[test]
    fn filter_matches_labels_and_shortcuts_case_insensitively() {
        let filtered = filter_keybind_help_groups(groups(), "WoRk");
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].1[0].1, "workspace navigation");

        let filtered = filter_keybind_help_groups(groups(), "x");
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].1[0].1, "close pane");
        assert!(filter_keybind_help_groups(groups(), "panes").is_empty());
    }
}
