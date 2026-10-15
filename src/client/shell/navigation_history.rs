//! Client-local destinations, independent of terminal contents and render revisions.
use super::*;
use crate::api::schema::{AgentGridFilter, Method, PaneTarget, PaneZoomMode, PaneZoomParams};
use std::time::{Duration, Instant};

const HISTORY_LIMIT: usize = 256;

#[derive(Clone, Debug, PartialEq, Eq)]
struct ViewLocation {
    endpoint: ClientEndpointId,
    boot: String,
    generation: Option<u64>,
    workspace: String,
    tab: String,
    pane: Option<String>,
    tab_zoom: bool,
    grid: bool,
    filter: AgentGridFilter,
    grid_zoom: bool,
}

struct PendingNavigation {
    requests: HashSet<String>,
    deadline: Instant,
    /// Creation/directional focus gets its destination from the server.
    resolve_after_revision: Option<u64>,
    resolve_from: Option<ViewLocation>,
}

#[derive(Default)]
pub(super) struct NavigationHistory {
    entries: VecDeque<ViewLocation>,
    entry_ids: VecDeque<u64>,
    next_entry_id: u64,
    request_visits: HashMap<String, u64>,
    cursor: usize,
    pending: Option<PendingNavigation>,
    restore_after_activation: Option<ViewLocation>,
}

impl NavigationHistory {
    fn current(&self) -> Option<&ViewLocation> {
        self.entries.get(self.cursor)
    }

    fn visit(&mut self, location: ViewLocation) -> bool {
        if self.current() == Some(&location) {
            return false;
        }
        self.entries.truncate(self.cursor + 1);
        self.entry_ids.truncate(self.cursor + 1);
        self.entries.push_back(location);
        self.next_entry_id = self.next_entry_id.saturating_add(1);
        self.entry_ids.push_back(self.next_entry_id);
        if self.entries.len() > HISTORY_LIMIT {
            self.entries.pop_front();
            self.entry_ids.pop_front();
        }
        self.cursor = self.entries.len() - 1;
        self.request_visits
            .retain(|_, id| self.entry_ids.contains(id));
        true
    }

    fn track_visit_requests(&mut self, requests: &HashSet<String>) {
        if let Some(id) = self.entry_ids.get(self.cursor) {
            self.request_visits
                .extend(requests.iter().map(|request| (request.clone(), *id)));
        }
    }

    fn complete_visit_request(&mut self, request: &str, success: bool) {
        let Some(id) = self.request_visits.remove(request) else {
            return;
        };
        if success {
            return;
        }
        let Some(index) = self.entry_ids.iter().position(|entry| *entry == id) else {
            return;
        };
        if index != self.cursor {
            self.entries.remove(index);
            self.entry_ids.remove(index);
            if index < self.cursor {
                self.cursor -= 1;
            }
            self.request_visits.retain(|_, entry| *entry != id);
        }
    }
}

impl ViewLocation {
    fn from_snapshot(
        endpoint: ClientEndpointId,
        generation: Option<u64>,
        snapshot: &ClientShellSnapshot,
    ) -> Option<Self> {
        let tab = snapshot.focused_tab_id.as_ref()?;
        Some(Self {
            endpoint,
            boot: snapshot.boot_id.clone(),
            generation,
            workspace: snapshot.focused_workspace_id.clone()?,
            tab: tab.clone(),
            pane: snapshot.focused_pane_id.clone(),
            tab_zoom: snapshot
                .tabs
                .iter()
                .find(|entry| &entry.tab_id == tab)?
                .zoomed,
            grid: false,
            filter: AgentGridFilter::default(),
            grid_zoom: false,
        })
    }

    fn focus_tab(&mut self, snapshot: &ClientShellSnapshot, tab_id: &str) -> bool {
        if self.tab == tab_id {
            return true;
        }
        let Some(tab) = snapshot.tabs.iter().find(|tab| tab.tab_id == tab_id) else {
            return false;
        };
        self.workspace = tab.workspace_id.clone();
        self.tab = tab.tab_id.clone();
        self.tab_zoom = tab.zoomed;
        self.pane = snapshot
            .panes
            .iter()
            .filter(|pane| pane.tab_id == tab_id)
            .find(|pane| pane.focused)
            .or_else(|| snapshot.panes.iter().find(|pane| pane.tab_id == tab_id))
            .map(|pane| pane.pane_id.clone());
        true
    }

    fn focus_pane(&mut self, snapshot: &ClientShellSnapshot, pane_id: &str) -> bool {
        let Some(pane) = snapshot.panes.iter().find(|pane| pane.pane_id == pane_id) else {
            return false;
        };
        if self.tab != pane.tab_id {
            self.focus_tab(snapshot, &pane.tab_id);
        }
        self.pane = Some(pane_id.to_owned());
        true
    }

    fn project(&mut self, snapshot: &ClientShellSnapshot, method: &Method) -> bool {
        match method {
            Method::WorkspaceFocus(target) => snapshot
                .workspaces
                .iter()
                .find(|workspace| workspace.workspace_id == target.workspace_id)
                .is_some_and(|workspace| {
                    self.workspace == workspace.workspace_id
                        || self.focus_tab(snapshot, &workspace.active_tab_id)
                }),
            Method::TabFocus(target) => self.focus_tab(snapshot, &target.tab_id),
            Method::PaneFocus(target) => self.focus_pane(snapshot, &target.pane_id),
            Method::PaneZoom(params) => {
                if let Some(pane) = &params.pane_id {
                    self.focus_pane(snapshot, pane);
                }
                self.tab_zoom = snapshot
                    .panes
                    .iter()
                    .filter(|pane| pane.tab_id == self.tab)
                    .take(2)
                    .count()
                    > 1
                    && match params.mode {
                        PaneZoomMode::On => true,
                        PaneZoomMode::Off => false,
                        PaneZoomMode::Toggle => !self.tab_zoom,
                    };
                true
            }
            Method::ClientShellAgentGridSet(params) => {
                self.grid = params.active;
                self.grid_zoom = false;
                true
            }
            Method::ClientShellAgentGridFilterSet(params) => {
                self.filter = params.filter;
                true
            }
            Method::ClientShellAgentGridZoomSet(params) => {
                self.grid_zoom = params.zoomed;
                true
            }
            _ => false,
        }
    }

    // Hidden tab zoom and a remembered filter aren't changes to the displayed view.
    fn normalize(&mut self) {
        if self.grid {
            self.tab_zoom = false;
        } else {
            self.filter = AgentGridFilter::default();
            self.grid_zoom = false;
        }
    }
}

impl ClientShellState {
    fn current_view_location(&self) -> Option<ViewLocation> {
        let mut location = ViewLocation::from_snapshot(
            self.active_endpoint_id.clone(),
            self.active_snapshot_generation,
            self.snapshot.as_deref()?,
        )?;
        location.grid = self.agent_grid_active();
        location.filter = self.effective_agent_grid_filter();
        location.grid_zoom = self.agent_grid_is_zoomed();
        location.normalize();
        Some(location)
    }

    /// Observe only at input boundaries, never per frame or per pane. Pending
    /// commands own the logical destination until their snapshots catch up.
    pub(super) fn observe_navigation_location(&mut self) {
        let Some(actual) = self.current_view_location() else {
            return;
        };
        if self.navigation_history.current().is_some_and(|current| {
            current.endpoint == actual.endpoint
                && (current.boot != actual.boot || current.generation != actual.generation)
        }) {
            self.navigation_history.pending = None;
            self.navigation_history.restore_after_activation = None;
        }
        if let Some(pending) = &self.navigation_history.pending {
            if pending.resolve_after_revision.is_some_and(|revision| {
                (pending.requests.is_empty()
                    || (pending.resolve_from.as_ref() != Some(&actual)
                        && self.navigation_history.current() != Some(&actual)))
                    && self
                        .snapshot
                        .as_deref()
                        .is_some_and(|snapshot| snapshot.revision > revision)
            }) {
                let requests = pending.requests.clone();
                self.navigation_history.pending = None;
                if self.navigation_history.visit(actual) {
                    self.navigation_history.track_visit_requests(&requests);
                }
                return;
            }
            let settled = pending.requests.is_empty()
                && self.navigation_history.restore_after_activation.is_none()
                && self.navigation_history.current() == Some(&actual);
            if !settled && Instant::now() < pending.deadline {
                return;
            }
            self.navigation_history.pending = None;
            self.navigation_history.restore_after_activation = None;
            // A rejection or changed destination replaces the speculative entry,
            // rather than making the server's intermediate state another visit.
            let cursor = self.navigation_history.cursor;
            if let Some(entry) = self.navigation_history.entries.get_mut(cursor) {
                *entry = actual;
            }
        } else {
            self.navigation_history.visit(actual);
        }
    }

    /// Fold every request from one input event into a single destination. This
    /// includes implicit focus when opening the grid or zooming another tile.
    pub(super) fn record_navigation_actions(&mut self, actions: &[ClientShellAction]) {
        let Some(mut next) = self.navigation_history.current().cloned() else {
            return;
        };
        let mut requests = HashSet::new();
        let mut navigated = false;
        let mut resolve_after_revision = None;
        for action in actions {
            match action {
                ClientShellAction::Endpoint {
                    endpoint_id,
                    request,
                    ..
                } => {
                    if endpoint_id != &next.endpoint {
                        continue;
                    }
                    if let Some(snapshot) = self
                        .endpoints
                        .iter()
                        .find(|endpoint| &endpoint.endpoint_id == endpoint_id)
                        .and_then(|endpoint| endpoint.snapshot.as_deref())
                    {
                        if next.project(snapshot, &request.method) {
                            navigated = true;
                            requests.insert(request.id.clone());
                        } else if matches!(
                            request.method,
                            Method::PaneFocusDirection(_)
                                | Method::WorkspaceCreate(_)
                                | Method::TabCreate(_)
                                | Method::PaneSplit(_)
                                | Method::SpaceMemberOpen(_)
                                | Method::PaneClose(_)
                                | Method::TabClose(_)
                                | Method::WorkspaceClose(_)
                        ) {
                            navigated = true;
                            requests.insert(request.id.clone());
                            resolve_after_revision = Some(snapshot.revision);
                        }
                    }
                }
                ClientShellAction::ActivateEndpoint {
                    endpoint_id,
                    target,
                } => {
                    let Some(endpoint) = self
                        .endpoints
                        .iter()
                        .find(|endpoint| &endpoint.endpoint_id == endpoint_id)
                    else {
                        continue;
                    };
                    let Some(snapshot) = endpoint.snapshot.as_deref() else {
                        continue;
                    };
                    let Some(mut location) = ViewLocation::from_snapshot(
                        endpoint_id.clone(),
                        endpoint.snapshot_generation,
                        snapshot,
                    ) else {
                        continue;
                    };
                    if let Some(target) = target {
                        match target {
                            ClientEndpointFocusTarget::Pane(pane) => {
                                location.focus_pane(snapshot, pane);
                            }
                            ClientEndpointFocusTarget::Tab(tab) => {
                                location.focus_tab(snapshot, tab);
                            }
                            ClientEndpointFocusTarget::Workspace(workspace) => {
                                location.project(
                                    snapshot,
                                    &Method::WorkspaceFocus(crate::api::schema::WorkspaceTarget {
                                        workspace_id: workspace.clone(),
                                    }),
                                );
                            }
                        }
                    }
                    next = location;
                    navigated = true;
                }
                _ => {}
            }
        }
        if navigated {
            next.normalize();
            if self.navigation_history.visit(next) {
                self.navigation_history.track_visit_requests(&requests);
            }
            self.navigation_history.restore_after_activation = None;
            self.navigation_history.pending = Some(PendingNavigation {
                requests,
                deadline: Instant::now() + Duration::from_secs(60),
                resolve_after_revision,
                resolve_from: resolve_after_revision.and_then(|_| self.current_view_location()),
            });
        }
    }

    pub(super) fn navigation_request_finished(&mut self, request_id: &str, success: bool) {
        self.navigation_history
            .complete_visit_request(request_id, success);
        if let Some(pending) = &mut self.navigation_history.pending {
            if pending.requests.remove(request_id) {
                if !success {
                    pending.resolve_after_revision = None;
                    pending.deadline = Instant::now();
                } else if pending.requests.is_empty() {
                    pending.deadline = Instant::now() + Duration::from_secs(2);
                }
            }
        }
    }

    fn history_location_available(&self, location: &ViewLocation) -> bool {
        self.endpoints.iter().any(|endpoint| {
            endpoint.endpoint_id == location.endpoint
                && endpoint.status == ClientEndpointStatus::Online
                && endpoint.snapshot_generation == location.generation
                && endpoint.snapshot.as_deref().is_some_and(|snapshot| {
                    snapshot.boot_id == location.boot
                        && snapshot
                            .workspaces
                            .iter()
                            .any(|workspace| workspace.workspace_id == location.workspace)
                        && snapshot.tabs.iter().any(|tab| {
                            tab.tab_id == location.tab && tab.workspace_id == location.workspace
                        })
                        && location.pane.as_ref().is_none_or(|id| {
                            snapshot.panes.iter().any(|pane| {
                                &pane.pane_id == id
                                    && pane.tab_id == location.tab
                                    && (!location.grid
                                        || !pane.agent_grid_excluded
                                            && snapshot
                                                .agents
                                                .iter()
                                                .any(|agent| &agent.pane_id == id))
                            })
                        })
                })
                && endpoint.methods.as_ref().is_none_or(|methods| {
                    methods.contains(if location.pane.is_some() {
                        "pane.focus"
                    } else {
                        "tab.focus"
                    }) && if location.grid {
                        methods.contains("client_shell.agent_grid.set")
                            && (location.filter == AgentGridFilter::default()
                                || methods.contains("client_shell.agent_grid.filter.set"))
                            && (!location.grid_zoom
                                || methods.contains("client_shell.agent_grid.zoom.set"))
                    } else {
                        location.pane.is_none() || methods.contains("pane.zoom")
                    }
                })
        })
    }

    pub(super) fn navigate_history(&mut self, forward: bool, outcome: &mut ClientShellInput) {
        self.observe_navigation_location();
        let activation_pending = self.navigation_history.restore_after_activation.is_some()
            || self
                .navigation_history
                .current()
                .is_some_and(|location| location.endpoint != self.active_endpoint_id);
        let mut index = self.navigation_history.cursor;
        let target = loop {
            let next = if forward {
                index.checked_add(1)
            } else {
                index.checked_sub(1)
            };
            let Some(next) = next.filter(|index| *index < self.navigation_history.entries.len())
            else {
                return;
            };
            index = next;
            let candidate = &self.navigation_history.entries[index];
            if self.history_location_available(candidate)
                && self.navigation_history.current() != Some(candidate)
            {
                break candidate.clone();
            }
        };
        self.navigation_history.cursor = index;
        self.release_input_leases(outcome);
        self.overlay = None;
        self.mode = ClientShellMode::Terminal;
        self.navigate_workspace_id = None;
        self.selection = None;
        self.copy_mode = None;
        self.reset_copy_pipeline();
        self.stop_selection_autoscroll();
        self.word_selection_gesture = None;
        self.chrome_drag = None;
        self.workspace_press = None;
        self.tab_press = None;
        self.space_press = None;
        self.clear_link_hover();
        self.reconcile_input_source();
        self.navigation_history.pending = Some(PendingNavigation {
            requests: HashSet::new(),
            deadline: Instant::now() + Duration::from_secs(60),
            resolve_after_revision: None,
            resolve_from: None,
        });
        if target.endpoint != self.active_endpoint_id || activation_pending {
            self.navigation_history.restore_after_activation = Some(target.clone());
            outcome.actions.push(ClientShellAction::ActivateEndpoint {
                endpoint_id: target.endpoint,
                target: None,
            });
        } else {
            self.navigation_history.restore_after_activation = None;
            self.restore_history_location(&target, outcome);
        }
        outcome.repaint = true;
    }

    /// Called only when the runtime has completed its endpoint handoff.
    pub(crate) fn resume_navigation_history(&mut self, outcome: &mut ClientShellInput) {
        let Some(target) = self.navigation_history.restore_after_activation.clone() else {
            return;
        };
        if !self.history_location_available(&target) {
            self.navigation_history.restore_after_activation = None;
            if let Some(pending) = &mut self.navigation_history.pending {
                pending.deadline = Instant::now();
            }
        } else if target.endpoint == self.active_endpoint_id {
            self.navigation_history.restore_after_activation = None;
            self.restore_history_location(&target, outcome);
        }
    }

    fn restore_history_location(&mut self, target: &ViewLocation, outcome: &mut ClientShellInput) {
        let start = outcome.actions.len();
        self.set_agent_grid(false, outcome);
        if let Some(pane) = &target.pane {
            self.push_endpoint_method(
                Method::PaneFocus(PaneTarget {
                    pane_id: pane.clone(),
                }),
                outcome,
            );
        } else {
            self.push_endpoint_method(
                Method::TabFocus(crate::api::schema::TabTarget {
                    tab_id: target.tab.clone(),
                }),
                outcome,
            );
        }
        if target.grid {
            self.sidebar_collapsed = false;
            self.agent_grid_filter = target.filter;
            self.set_agent_grid(true, outcome);
            // Focus above already selects the destination. Do not refocus from
            // the old snapshot in set_agent_grid_zoom().
            self.agent_grid_zoomed = target.grid_zoom;
            if self.agent_grid_zoom_supported() {
                self.push_endpoint_method(
                    Method::ClientShellAgentGridZoomSet(
                        crate::api::schema::ClientShellAgentGridZoomSetParams {
                            zoomed: target.grid_zoom,
                        },
                    ),
                    outcome,
                );
            }
        } else if let Some(pane) = &target.pane {
            self.push_endpoint_method(
                Method::PaneZoom(PaneZoomParams {
                    pane_id: Some(pane.clone()),
                    mode: if target.tab_zoom {
                        PaneZoomMode::On
                    } else {
                        PaneZoomMode::Off
                    },
                }),
                outcome,
            );
        }
        if let Some(pending) = &mut self.navigation_history.pending {
            pending.requests = outcome.actions[start..]
                .iter()
                .filter_map(|action| match action {
                    ClientShellAction::Endpoint { request, .. } => Some(request.id.clone()),
                    _ => None,
                })
                .collect();
            pending.resolve_after_revision = None;
            pending.resolve_from = None;
            pending.deadline = Instant::now() + Duration::from_secs(60);
        }
        outcome.repaint = true;
    }
}

#[cfg(test)]
#[path = "tests/navigation_history.rs"]
mod tests;
