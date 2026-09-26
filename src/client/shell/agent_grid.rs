use super::*;

/// Endpoint connection that showed the live agent grid. The server keeps grid
/// state per connection, so a new boot or connection generation starts closed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ClientAgentGridOwner {
    endpoint_id: ClientEndpointId,
    boot_id: String,
    generation: Option<u64>,
}

impl ClientShellState {
    fn current_agent_grid_owner(&self) -> Option<ClientAgentGridOwner> {
        Some(ClientAgentGridOwner {
            endpoint_id: self.active_endpoint_id.clone(),
            boot_id: self.snapshot.as_deref()?.boot_id.clone(),
            generation: self.active_snapshot_generation,
        })
    }

    /// Whether the selected endpoint connection shows the live agent grid.
    pub(super) fn agent_grid_active(&self) -> bool {
        self.agent_grid_owner.is_some() && self.agent_grid_owner == self.current_agent_grid_owner()
    }

    pub(super) fn agent_grid_supported(&self) -> bool {
        self.supports_endpoint_method(&crate::api::schema::Method::ClientShellAgentGridSet(
            crate::api::schema::ClientShellAgentGridSetParams { active: true },
        ))
    }

    /// Toggle state for the agents heading: `None` when the endpoint cannot
    /// show a grid, otherwise whether the grid is shown.
    pub(super) fn agent_grid_toggle_state(&self) -> Option<bool> {
        self.agent_grid_supported()
            .then(|| self.agent_grid_active())
    }

    pub(super) fn toggle_agent_grid(&mut self, outcome: &mut ClientShellInput) {
        let active = !self.agent_grid_active();
        self.set_agent_grid(active, outcome);
        if active {
            self.focus_agent_for_grid(outcome);
        }
    }

    /// The grid shows only agents, so keyboard input must target one of them.
    fn focus_agent_for_grid(&mut self, outcome: &mut ClientShellInput) {
        let Some(snapshot) = self.snapshot.as_deref() else {
            return;
        };
        let focused = snapshot.focused_pane_id.as_deref();
        if snapshot
            .agents
            .iter()
            .any(|agent| Some(agent.pane_id.as_str()) == focused)
        {
            return;
        }
        let Some(pane_id) = super::agent_sidebar::ordered_agent_pane_ids(
            snapshot,
            crate::config::AgentPanelSortConfig::Spaces,
        )
        .into_iter()
        .next() else {
            return;
        };
        self.push_endpoint_method(
            crate::api::schema::Method::PaneFocus(crate::api::schema::PaneTarget { pane_id }),
            outcome,
        );
    }

    pub(super) fn set_agent_grid(&mut self, active: bool, outcome: &mut ClientShellInput) {
        if self.agent_grid_active() == active {
            return;
        }
        self.agent_grid_owner = if active {
            self.current_agent_grid_owner()
        } else {
            None
        };
        self.push_endpoint_method(
            crate::api::schema::Method::ClientShellAgentGridSet(
                crate::api::schema::ClientShellAgentGridSetParams { active },
            ),
            outcome,
        );
        // The grid replaces the tab bar, so the pane surface changes size.
        self.invalidate_pane_surface();
        outcome.repaint = true;
        outcome.resize = true;
    }

    #[cfg(test)]
    pub(crate) fn show_test_agent_grid(&mut self) {
        self.set_agent_grid(true, &mut ClientShellInput::default());
    }

    /// Leaves the grid before navigation that selects a workspace or tab view.
    pub(super) fn close_agent_grid_for_navigation(
        &mut self,
        method: &crate::api::schema::Method,
        outcome: &mut ClientShellInput,
    ) {
        use crate::api::schema::Method;

        let navigates = match method {
            Method::WorkspaceFocus(_) | Method::TabFocus(_) | Method::SpaceOpen(_) => true,
            Method::WorkspaceCreate(params) => params.focus,
            Method::TabCreate(params) => params.focus,
            _ => false,
        };
        if navigates && self.agent_grid_active() {
            self.set_agent_grid(false, outcome);
        }
    }

    /// Closes the grid when its toggle is no longer reachable: the sidebar is
    /// collapsed or the terminal is narrow enough for the mobile layout.
    pub(crate) fn close_hidden_agent_grid(&mut self, cols: u16) -> ClientShellInput {
        let mut outcome = ClientShellInput::default();
        if self.agent_grid_active()
            && (self.sidebar_collapsed || cols <= self.config.mobile_width_threshold)
        {
            self.set_agent_grid(false, &mut outcome);
        }
        outcome
    }
}
