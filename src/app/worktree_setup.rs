//! Repo commands around space worktrees: the create command runs in the
//! background once Herdr creates a worktree, then the start command is typed
//! into the worktree's first pane.

use bytes::Bytes;

use super::App;
use crate::events::{AppEvent, WorktreeSetupPlan, WorktreeSetupResult};

/// A repo create command for one workspace, running or failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorktreeSetup {
    operation: u64,
    pub(crate) running: bool,
    /// Last line the command printed when it failed.
    pub(crate) failure: Option<String>,
    pub(crate) log_path: std::path::PathBuf,
    start_command: Option<String>,
}

impl App {
    /// Starts `plan` for the new worktree open in `ws_idx`.
    pub(crate) fn start_worktree_setup(&mut self, ws_idx: usize, plan: WorktreeSetupPlan) {
        let Some(workspace_id) = self
            .state
            .workspaces
            .get(ws_idx)
            .map(|workspace| workspace.id.clone())
        else {
            return;
        };
        let Some(hook) = plan.on_create else {
            if let Some(command) = plan.start_command {
                self.type_start_command(&workspace_id, &command);
            }
            return;
        };
        let operation = self.next_api_worktree_operation_id();
        self.worktree_setups.insert(
            workspace_id.clone(),
            WorktreeSetup {
                operation,
                running: true,
                failure: None,
                log_path: hook.log_path.clone(),
                start_command: plan.start_command,
            },
        );
        let event_tx = self.event_tx.clone();
        std::thread::spawn(move || {
            let result = hook.run();
            let _ = event_tx.blocking_send(AppEvent::WorktreeSetupFinished(Box::new(
                WorktreeSetupResult {
                    workspace_id,
                    operation,
                    result,
                },
            )));
        });
    }

    pub(crate) fn handle_worktree_setup_finished(&mut self, result: WorktreeSetupResult) {
        let Some(setup) = self
            .worktree_setups
            .get_mut(&result.workspace_id)
            .filter(|setup| setup.operation == result.operation)
        else {
            return;
        };
        let workspace_open = self
            .state
            .workspaces
            .iter()
            .any(|workspace| workspace.id == result.workspace_id);
        match result.result {
            Ok(()) => {
                let start_command = setup.start_command.take();
                self.worktree_setups.remove(&result.workspace_id);
                if let Some(command) = start_command.filter(|_| workspace_open) {
                    self.type_start_command(&result.workspace_id, &command);
                }
            }
            Err(message) if workspace_open => {
                tracing::warn!(
                    workspace = %result.workspace_id,
                    log = %setup.log_path.display(),
                    %message,
                    "worktree create command failed"
                );
                setup.running = false;
                setup.failure = Some(message);
                setup.start_command = None;
            }
            Err(_) => {
                self.worktree_setups.remove(&result.workspace_id);
            }
        }
    }

    /// The setup state shown for a workspace, if it has one.
    pub(crate) fn worktree_setup(&self, workspace_id: &str) -> Option<&WorktreeSetup> {
        self.worktree_setups.get(workspace_id)
    }

    /// Types `command` and Enter into the workspace's first pane.
    fn type_start_command(&mut self, workspace_id: &str, command: &str) {
        let Some(ws_idx) = self
            .state
            .workspaces
            .iter()
            .position(|workspace| workspace.id == workspace_id)
        else {
            return;
        };
        let Some(pane_id) = self.state.workspaces[ws_idx]
            .tabs
            .first()
            .map(|tab| tab.root_pane)
        else {
            return;
        };
        let Some(runtime) = self.lookup_runtime_sender(ws_idx, pane_id) else {
            return;
        };
        if let Err(err) = runtime.try_send_bytes(Bytes::from(format!("{command}\r"))) {
            tracing::warn!(%workspace_id, %err, "couldn't type the repo start command");
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::worktree::WorktreeHook;

    fn app_with_workspace() -> App {
        let (_api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(
            &Config::default(),
            crate::app::AppPolicy::TEST,
            None,
            api_rx,
            crate::api::EventHub::default(),
        );
        app.state.workspaces = vec![crate::workspace::Workspace::test_new("alpha")];
        app
    }

    fn finish(app: &mut App) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            match app.event_rx.try_recv() {
                Ok(AppEvent::WorktreeSetupFinished(result)) => {
                    app.handle_worktree_setup_finished(*result);
                    return;
                }
                Ok(_) => {}
                Err(_) => {
                    assert!(std::time::Instant::now() < deadline, "setup never finished");
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
            }
        }
    }

    fn hook(command: &str) -> WorktreeHook {
        let dir = std::env::temp_dir().join(format!(
            "herdr-setup-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        WorktreeHook {
            command: command.into(),
            cwd: dir.clone(),
            env: Vec::new(),
            log_path: dir.join("create.log"),
        }
    }

    #[tokio::test]
    async fn a_failed_create_command_stays_visible_and_cancels_the_start_command() {
        let mut app = app_with_workspace();
        let workspace_id = app.state.workspaces[0].id.clone();
        app.start_worktree_setup(
            0,
            WorktreeSetupPlan {
                on_create: Some(hook("echo 'boom: missing tool'; exit 1")),
                start_command: Some("claude".into()),
            },
        );
        let running = app.worktree_setup(&workspace_id).expect("setup tracked");
        assert!(running.running);

        finish(&mut app);

        let failed = app.worktree_setup(&workspace_id).expect("failure kept");
        assert!(!failed.running);
        assert_eq!(failed.failure.as_deref(), Some("boom: missing tool"));
        assert_eq!(failed.start_command, None);
    }

    #[tokio::test]
    async fn a_successful_create_command_clears_its_status() {
        let mut app = app_with_workspace();
        let workspace_id = app.state.workspaces[0].id.clone();
        app.start_worktree_setup(
            0,
            WorktreeSetupPlan {
                on_create: Some(hook("true")),
                start_command: None,
            },
        );
        finish(&mut app);
        assert_eq!(app.worktree_setup(&workspace_id), None);
    }
}
