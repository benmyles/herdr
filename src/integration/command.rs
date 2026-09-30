use std::path::Path;

#[cfg(any(windows, test))]
use base64::Engine;

pub(crate) fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

/// The session-start hook command that gives Claude Code and Codex the
/// context of their Herdr space. It runs the Herdr binary that started the
/// pane, found through the pane's environment, so it does nothing in other
/// terminals and needs no path that could go stale.
pub(crate) fn agent_context_hook_command() -> String {
    let var = super::HERDR_BIN_PATH_ENV_VAR;
    #[cfg(windows)]
    {
        format!(
            "powershell -NoProfile -Command \"if ($env:{var}) {{ & $env:{var} agent-context session-start }}\""
        )
    }

    #[cfg(not(windows))]
    {
        format!("[ -n \"${{{var}:-}}\" ] && \"${var}\" agent-context session-start || true")
    }
}

pub(crate) fn hook_command(hook_path: &Path, action: Option<&str>) -> String {
    let path = hook_path.display().to_string();
    #[cfg(windows)]
    {
        let mut command = format!(
            "powershell -NoProfile -ExecutionPolicy Bypass -File {}",
            windows_command_quote(&path)
        );
        if let Some(action) = action {
            command.push(' ');
            command.push_str(action);
        }
        command
    }

    #[cfg(not(windows))]
    {
        let mut command = format!("bash {}", shell_single_quote(&path));
        if let Some(action) = action {
            command.push(' ');
            command.push_str(action);
        }
        command
    }
}

#[cfg(any(windows, test))]
pub(crate) fn powershell_encoded_hook_command(hook_path: &Path, action: &str) -> String {
    let path = hook_path.display().to_string().replace('\'', "''");
    let script = format!("& '{path}' {action}");
    let encoded_script = script
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect::<Vec<_>>();
    let encoded = base64::engine::general_purpose::STANDARD.encode(encoded_script);
    format!("powershell -NoProfile -ExecutionPolicy Bypass -EncodedCommand {encoded}")
}

pub(crate) fn legacy_bash_hook_command(hook_path: &Path, action: Option<&str>) -> String {
    let mut command = format!(
        "bash {}",
        shell_single_quote(&hook_path.display().to_string())
    );
    if let Some(action) = action {
        command.push(' ');
        command.push_str(action);
    }
    command
}

#[cfg(windows)]
fn windows_command_quote(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\\\""))
}
