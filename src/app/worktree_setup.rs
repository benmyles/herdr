//! Repo commands around space worktrees: once Herdr creates a worktree, the
//! repo's create command and then its start command are typed into the
//! worktree's first pane, so the user watches them run.

use bytes::Bytes;

use super::App;
use crate::events::WorktreeSetupPlan;

/// The line typed into a new worktree's first pane: the create command, then
/// the start command only if the create command succeeded.
fn setup_command_line(plan: &WorktreeSetupPlan, powershell: bool) -> Option<String> {
    let on_create = plan.on_create.as_deref().map(str::trim);
    let start = plan.start_command.as_deref().map(str::trim);
    match (
        on_create.filter(|command| !command.is_empty()),
        start.filter(|command| !command.is_empty()),
    ) {
        // `&&` needs PowerShell 7; `$?` works in 5.1 too.
        (Some(on_create), Some(start)) if powershell => {
            Some(format!("{on_create}; if ($?) {{ {start} }}"))
        }
        (Some(on_create), Some(start)) => Some(format!("{on_create} && {start}")),
        (Some(command), None) | (None, Some(command)) => Some(command.to_owned()),
        (None, None) => None,
    }
}

impl App {
    /// Types `plan`'s commands into the first pane of the new worktree open
    /// in `ws_idx`.
    pub(crate) fn start_worktree_setup(&mut self, ws_idx: usize, plan: WorktreeSetupPlan) {
        let powershell = crate::pane::pane_shell_is_powershell(&self.state.default_shell);
        let Some(line) = setup_command_line(&plan, powershell) else {
            return;
        };
        let Some(pane_id) = self
            .state
            .workspaces
            .get(ws_idx)
            .and_then(|workspace| workspace.tabs.first())
            .map(|tab| tab.root_pane)
        else {
            return;
        };
        let Some(runtime) = self.lookup_runtime_sender(ws_idx, pane_id) else {
            return;
        };
        // The pty holds the line until the shell is ready to read it.
        if let Err(err) = runtime.try_send_bytes(Bytes::from(format!("{line}\r"))) {
            let workspace_id = &self.state.workspaces[ws_idx].id;
            tracing::warn!(%workspace_id, %err, "couldn't type the repo setup commands");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(on_create: &str, start: &str) -> WorktreeSetupPlan {
        WorktreeSetupPlan {
            on_create: Some(on_create.into()),
            start_command: Some(start.into()),
            env: Vec::new(),
        }
    }

    #[test]
    fn start_command_waits_for_a_successful_create_command() {
        assert_eq!(
            setup_command_line(&plan("just setup", "claude"), false).as_deref(),
            Some("just setup && claude")
        );
        assert_eq!(
            setup_command_line(&plan("just setup", "claude"), true).as_deref(),
            Some("just setup; if ($?) { claude }")
        );
    }

    #[test]
    fn a_lone_or_blank_command_is_typed_as_is() {
        assert_eq!(
            setup_command_line(&plan(" npm install ", ""), false).as_deref(),
            Some("npm install")
        );
        assert_eq!(
            setup_command_line(&plan("", "claude"), true).as_deref(),
            Some("claude")
        );
        assert_eq!(setup_command_line(&plan(" ", ""), false), None);
        assert_eq!(
            setup_command_line(&WorktreeSetupPlan::default(), false),
            None
        );
    }
}
