use std::time::{Duration, Instant};

use super::{App, SESSION_SAVE_DEBOUNCE};

enum SessionSaveJob {
    Clear,
    Save {
        snapshot: crate::persist::SessionSnapshot,
        history: Option<crate::persist::SessionHistorySnapshot>,
    },
}

impl App {
    pub(super) fn schedule_session_save(&mut self) {
        if !self.no_session {
            self.session_save_deadline = Some(Instant::now() + SESSION_SAVE_DEBOUNCE);
        }
    }

    pub(crate) fn sync_session_save_schedule(&mut self) {
        if self.state.session_dirty {
            self.state.session_dirty = false;
            self.schedule_session_save();
        }
    }

    fn reap_finished_session_save(&mut self) {
        if self
            .session_save_thread
            .as_ref()
            .is_some_and(std::thread::JoinHandle::is_finished)
        {
            if let Some(thread) = self.session_save_thread.take() {
                let _ = thread.join();
            }
        }
    }

    fn capture_session_save_job(&self) -> SessionSaveJob {
        if self.state.workspaces.is_empty() && self.state.pinned_spaces.is_empty() {
            SessionSaveJob::Clear
        } else {
            let snapshot = crate::persist::capture(
                &self.state.workspaces,
                &self.state.pinned_spaces,
                &self.state.terminals,
                &self.terminal_runtimes,
                self.state.active,
                self.state.selected,
                self.state.sidebar_width,
                self.state.sidebar_section_split,
                self.state.collapsed_space_keys.clone(),
            );
            let history = self.persist_pane_history.then(|| {
                crate::persist::capture_history(&self.state.workspaces, &self.terminal_runtimes)
            });
            SessionSaveJob::Save { snapshot, history }
        }
    }

    pub(crate) fn start_background_session_save(&mut self) {
        if self.no_session {
            self.session_save_deadline = None;
            return;
        }

        self.reap_finished_session_save();
        if self.session_save_thread.is_some() {
            self.session_save_deadline = Some(Instant::now() + Duration::from_millis(250));
            return;
        }

        let job = self.capture_session_save_job();
        self.session_save_deadline = None;
        match std::thread::Builder::new()
            .name("herdr-session-save".into())
            .spawn(move || run_session_save_job(job))
        {
            Ok(thread) => self.session_save_thread = Some(thread),
            Err(err) => {
                tracing::warn!(err = %err, "failed to spawn session save thread; saving inline");
                run_session_save_job(self.capture_session_save_job());
            }
        }
    }

    pub(crate) fn save_session_now(&mut self) {
        if let Some(thread) = self.session_save_thread.take() {
            let _ = thread.join();
        }

        if self.no_session {
            self.session_save_deadline = None;
            return;
        }

        run_session_save_job(self.capture_session_save_job());
        self.session_save_deadline = None;
    }
}

fn run_session_save_job(job: SessionSaveJob) {
    match job {
        SessionSaveJob::Clear => crate::persist::clear(),
        SessionSaveJob::Save { snapshot, history } => {
            crate::persist::save(&snapshot, history.as_ref());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pinned_only_session_is_saved_instead_of_cleared() {
        let (_api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(
            &crate::config::Config::default(),
            true,
            None,
            api_rx,
            crate::api::EventHub::default(),
        );
        app.state.workspaces.clear();
        app.state.active = None;
        app.state.selected = 0;
        app.state.pinned_spaces.push(crate::space::PinnedSpace::new(
            crate::space::PinnedSpaceKey::Workspace {
                workspace_id: "w_saved".into(),
            },
            "saved".into(),
            "/tmp/saved".into(),
            0,
            None,
        ));

        match app.capture_session_save_job() {
            SessionSaveJob::Save { snapshot, history } => {
                assert!(snapshot.workspaces.is_empty());
                assert_eq!(snapshot.pinned_spaces, app.state.pinned_spaces);
                assert!(history.is_none());
            }
            SessionSaveJob::Clear => panic!("pinned-only session must not be cleared"),
        }
    }
}
