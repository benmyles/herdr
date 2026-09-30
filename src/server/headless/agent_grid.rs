use super::*;

impl HeadlessServer {
    /// Whether this client shows the live agent grid in place of its tab.
    pub(super) fn client_shows_agent_grid(&self, client_id: u64) -> bool {
        self.clients
            .get(&client_id)
            .is_some_and(|client| client.shell_agent_grid)
    }

    /// What this client's grid shows, when it shows one.
    pub(super) fn client_agent_grid_view(
        &self,
        client_id: u64,
    ) -> Option<crate::ui::AgentGridView> {
        let client = self.clients.get(&client_id)?;
        client.shell_agent_grid.then(|| crate::ui::AgentGridView {
            filter: client.shell_agent_grid_filter,
            selected: self
                .shell_focus_target(client_id)
                .map(|target| (target.workspace_index, target.pane_id)),
        })
    }

    /// The views of the grids that are shown to someone.
    fn shown_agent_grid_views(&self) -> Vec<crate::ui::AgentGridView> {
        self.clients
            .iter()
            .filter(|(_, client)| {
                client.shell_agent_grid
                    && client.is_active_shell_client()
                    && client.writer.is_some()
            })
            .filter_map(|(&client_id, _)| self.client_agent_grid_view(client_id))
            .collect()
    }

    /// The active grid client whose surface size owns agent PTY geometry.
    /// Choosing the lowest id keeps concurrent grids from fighting over sizes.
    fn agent_grid_geometry_client(&self) -> Option<u64> {
        self.clients
            .iter()
            .filter(|(_, client)| {
                client.shell_agent_grid
                    && client.is_active_shell_client()
                    && client.writer.is_some()
            })
            .map(|(&client_id, _)| client_id)
            .min()
    }

    /// Whether any active grid shows this pane.
    pub(super) fn any_agent_grid_contains_pane(&self, pane_id: crate::layout::PaneId) -> bool {
        let views = self.shown_agent_grid_views();
        !views.is_empty()
            && self
                .app
                .find_pane(pane_id)
                .is_some_and(|(workspace_index, _)| {
                    views.into_iter().any(|view| {
                        crate::ui::pane_in_agent_grid(
                            &self.app.state,
                            &self.app.terminal_runtimes,
                            workspace_index,
                            pane_id,
                            view,
                        )
                    })
                })
    }

    /// Panes shown by active grids, for immediate PTY presentation.
    pub(super) fn agent_grid_pane_ids(&self) -> Vec<crate::layout::PaneId> {
        let mut pane_ids = Vec::new();
        for view in self.shown_agent_grid_views() {
            for target in
                crate::ui::live_agent_targets(&self.app.state, &self.app.terminal_runtimes, view)
            {
                if !pane_ids.contains(&target.pane_id) {
                    pane_ids.push(target.pane_id);
                }
            }
        }
        pane_ids
    }

    /// Shows or hides one client's grid. Returns whether presentation changed.
    pub(super) fn set_client_shell_agent_grid(&mut self, client_id: u64, active: bool) -> bool {
        let Some(client) = self.clients.get_mut(&client_id) else {
            return false;
        };
        if !client.is_shell_client() || client.shell_agent_grid == active {
            return false;
        }
        client.shell_agent_grid = active;
        client.request_repaint();
        self.sync_agent_grid_geometry();
        true
    }

    /// Sets which agents one client's grid shows. Returns whether
    /// presentation changed.
    pub(super) fn set_client_shell_agent_grid_filter(
        &mut self,
        client_id: u64,
        filter: crate::api::schema::AgentGridFilter,
    ) -> bool {
        let Some(client) = self.clients.get_mut(&client_id) else {
            return false;
        };
        if !client.is_shell_client() || client.shell_agent_grid_filter == filter {
            return false;
        }
        client.shell_agent_grid_filter = filter;
        if !client.shell_agent_grid {
            return false;
        }
        client.request_repaint();
        self.sync_agent_grid_geometry();
        true
    }

    /// Keeps agent PTY sizes and resize locks in step with the grids that are
    /// shown. While a grid is visible its tiles own every live agent's size and
    /// tab geometry leaves those terminals alone; closing the last grid
    /// releases them and restores tab geometry. Returns whether tab geometry
    /// was reapplied.
    pub(super) fn sync_agent_grid_geometry(&mut self) -> bool {
        let owner = self.agent_grid_geometry_client();
        let owner_view = owner.and_then(|client_id| self.client_agent_grid_view(client_id));
        let locks = match owner_view {
            Some(view) => {
                crate::ui::live_agent_targets(&self.app.state, &self.app.terminal_runtimes, view)
                    .into_iter()
                    .map(|target| target.terminal_id)
                    .collect::<HashSet<_>>()
            }
            None => HashSet::new(),
        };
        let released = self
            .app
            .state
            .agent_grid_resize_locks
            .iter()
            .any(|terminal_id| !locks.contains(terminal_id));
        self.app.state.agent_grid_resize_locks = locks;
        if let Some(client) = owner.and_then(|client_id| self.clients.get(&client_id)) {
            let (cols, rows) = client.terminal_size;
            let cell_size = if client.cell_size.is_known() {
                client.cell_size
            } else {
                crate::kitty_graphics::HostCellSize::default()
            };
            crate::ui::compute_agent_grid(
                &self.app.state,
                &self.app.terminal_runtimes,
                Rect::new(0, 0, cols, rows),
                owner_view.unwrap_or_default(),
                Some(cell_size),
            );
        }
        released && self.reapply_controlled_shell_tab_geometry(false)
    }
}
