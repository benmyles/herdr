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
            Method::WorkspaceFocus(_) | Method::TabFocus(_) | Method::SpaceMemberOpen(_) => true,
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

/// Draws each grid tile's title over the top border the server drew: status
/// mark, vendor icon, session title, and where the agent works. Titles come
/// from the same agent projection as the agents panel, keyed by pane, so a
/// tile can never show another agent's title. Returns whether a mark animates.
pub(super) fn render_tile_titles(
    frame: &mut FrameData,
    snapshot: &ClientShellSnapshot,
    surface: &PaneSurfaceFrame,
    origin: Rect,
    config: &ClientShellConfig,
    clock: super::agent_marks::AgentClock,
) -> bool {
    let mut animating = false;
    let spaces = super::sidebar::space_presentation(snapshot, &config.palette);
    for pane in &surface.panes {
        // Only a tile with a top border row has room for a title.
        if pane.inner_rect.y <= pane.rect.y || pane.rect.width < 8 {
            continue;
        }
        let Some(agent) = snapshot
            .agents
            .iter()
            .find(|agent| agent.pane_id == pane.pane_id)
        else {
            continue;
        };
        let x = origin.x.saturating_add(pane.rect.x);
        let y = origin.y.saturating_add(pane.rect.y);
        let width = pane.rect.width.min(frame.width.saturating_sub(x));
        if y >= frame.height || width < 8 {
            continue;
        }
        let row_start = usize::from(y) * usize::from(frame.width);
        let Some(corner) = frame.cells.get(row_start + usize::from(x)).cloned() else {
            continue;
        };
        let workspace_index = snapshot
            .workspaces
            .iter()
            .position(|workspace| workspace.workspace_id == agent.workspace_id);
        let inner = width - 2;
        let title = tile_title(
            snapshot,
            agent,
            workspace_index,
            &spaces,
            pane.focused,
            inner.saturating_sub(1),
            config,
            clock,
        );
        animating |= title.animating;

        // Redraw the border between the corners, then the title over it.
        let mut border = corner.clone();
        border.symbol = horizontal_border(&corner.symbol).to_owned();
        border.hyperlink = None;
        for column in 1..=inner {
            if let Some(cell) = frame.cells.get_mut(row_start + usize::from(x + column)) {
                *cell = border.clone();
            }
        }
        let mut row = Buffer::empty(Rect::new(0, 0, inner, 1));
        let mut drawn = Vec::new();
        // Keep at least one border cell before the far corner.
        let (left_end, _) = row.set_line(0, 0, &title.left, inner.saturating_sub(1));
        drawn.push(0..left_end.min(inner));
        // The widest context that still leaves border on both sides.
        if let Some(right) = title
            .right
            .iter()
            .find(|right| left_end + 2 + right.width() as u16 <= inner.saturating_sub(1))
        {
            let right_width = right.width() as u16;
            let right_x = inner - 1 - right_width;
            row.set_line(right_x, 0, right, right_width);
            drawn.push(right_x..inner - 1);
        }
        let cells = FrameData::from_ratatui_buffer_with_hyperlinks(&row, None, &[]).cells;
        // The last column is never drawn, so it carries the unset background.
        let unset_bg = cells.last().map(|cell| cell.bg);
        for range in drawn {
            for column in range {
                let (Some(source), Some(target)) = (
                    cells.get(usize::from(column)),
                    frame.cells.get_mut(row_start + usize::from(x + 1 + column)),
                ) else {
                    continue;
                };
                let mut cell = source.clone();
                if Some(cell.bg) == unset_bg {
                    cell.bg = border.bg;
                }
                *target = cell;
            }
        }
    }
    animating
}

struct TileTitle {
    left: ratatui::text::Line<'static>,
    /// Where the agent works, widest first; the first that fits is drawn.
    right: Vec<ratatui::text::Line<'static>>,
    animating: bool,
}

fn tile_title(
    snapshot: &ClientShellSnapshot,
    agent: &crate::protocol::ClientShellAgent,
    workspace_index: Option<usize>,
    spaces: &crate::ui::SpacePresentation,
    selected: bool,
    max_left: u16,
    config: &ClientShellConfig,
    clock: super::agent_marks::AgentClock,
) -> TileTitle {
    use ratatui::text::{Line, Span};

    let marks = &config.agent_marks;
    let palette = &config.palette;
    let state = marks.agent_state(agent, clock.now);
    let cwd = snapshot
        .panes
        .iter()
        .find(|pane| pane.pane_id == agent.pane_id)
        .and_then(|pane| pane.cwd.as_deref());
    let mut left = vec![Span::raw(" ")];
    if let Some(icon) = marks.icon(agent) {
        left.push(Span::styled(
            format!("{icon} "),
            state.icon_style(agent, palette),
        ));
    }
    if let Some(lead) = marks.lead(state, clock.frame) {
        left.push(Span::styled(
            format!("{lead} "),
            state.lead_style(agent, palette),
        ));
    }
    let title_style = state.title_style(agent, palette);
    let marks_width = left.iter().map(Span::width).sum::<usize>();
    let title_width = usize::from(max_left).saturating_sub(marks_width + 1);
    left.push(Span::styled(
        crate::ui::truncate_end(&super::agent_marks::session_title(agent, cwd), title_width),
        if selected {
            title_style.add_modifier(Modifier::BOLD)
        } else {
            title_style.remove_modifier(Modifier::BOLD)
        },
    ));
    left.push(Span::raw(" "));

    let mut right = Vec::new();
    if let Some(index) = workspace_index {
        let workspace = &snapshot.workspaces[index];
        let space = workspace.space_id.as_deref().and_then(|space_id| {
            snapshot
                .spaces
                .iter()
                .find(|space| space.space_id == space_id && !space.built_in)
        });
        let space_color = if selected {
            spaces.color(index)
        } else {
            super::sidebar::muted_space_color(spaces.color(index), palette)
        };
        let tab_count = snapshot
            .tabs
            .iter()
            .filter(|tab| tab.workspace_id == workspace.workspace_id)
            .count();
        let tab = snapshot
            .tabs
            .iter()
            .find(|tab| tab.tab_id == agent.tab_id)
            .filter(|tab| tab_count > 1 || tab.custom_label);
        let label = match super::agent_sidebar::distinct_branch(workspace, space) {
            Some(branch) => format!("{} · {branch}", workspace.label),
            None => workspace.label.clone(),
        };
        let place = Span::styled(
            match tab {
                Some(tab) => format!("{label}/{}", tab.label),
                None => label,
            },
            Style::default().fg(if selected {
                palette.overlay1
            } else {
                palette.overlay0
            }),
        );
        let age = state
            .shows_age()
            .then(|| super::agent_marks::age_label(agent.state_changed_at_ms, clock.now))
            .flatten()
            .map(|age| Span::styled(format!(" · {age}"), Style::default().fg(palette.overlay0)));
        let spaced = |spans: Vec<Span<'static>>| {
            let mut line = vec![Span::raw(" ")];
            line.extend(spans);
            line.push(Span::raw(" "));
            Line::from(line)
        };
        let mut full = Vec::new();
        if let Some(space) = space {
            full.push(Span::styled(
                space.name.clone(),
                Style::default().fg(space_color),
            ));
            full.push(Span::styled(" › ", Style::default().fg(palette.overlay0)));
        }
        full.push(place.clone());
        if let Some(age) = age {
            let mut with_age = full.clone();
            with_age.push(age);
            right.push(spaced(with_age));
        }
        right.push(spaced(full));
        right.push(spaced(vec![place]));
    }
    TileTitle {
        left: Line::from(left),
        right,
        animating: state.animated(),
    }
}

fn horizontal_border(corner: &str) -> &'static str {
    match corner {
        "┏" | "┓" | "┗" | "┛" | "┣" | "┫" | "┳" | "┻" | "╋" | "━" => "━",
        "╔" | "╗" | "╚" | "╝" | "╠" | "╣" | "╦" | "╩" | "╬" | "═" => "═",
        _ => "─",
    }
}
