use super::agent_marks::AgentClock;
use super::render::put_text;
use super::*;

pub(super) fn render_collapsed(
    buffer: &mut Buffer,
    area: Rect,
    endpoints: &[ClientShellEndpoint],
    active_endpoint_id: &ClientEndpointId,
    config: &ClientShellConfig,
    hits: &mut ShellHitMap,
) {
    let rows = super::aggregate_navigation::aggregate_agent_rows(
        endpoints,
        active_endpoint_id,
        config.agent_panel_sort,
    );
    for (index, row) in rows.into_iter().take(area.height as usize).enumerate() {
        let rect = Rect::new(area.x, area.y + index as u16, area.width, 1);
        let stale = row.endpoint.stale();
        if row.agent.focused && row.endpoint.endpoint_id == active_endpoint_id {
            buffer.set_style(rect, Style::default().bg(config.palette.active_row_bg));
        }
        let initial = row.endpoint.label.chars().next().unwrap_or('?');
        put_text(
            buffer,
            rect.x,
            rect.y,
            rect.width,
            &format!(
                "{initial}{}",
                status_icon(row.agent.agent_status, config.status_indicators)
            ),
            Style::default()
                .fg(if stale {
                    config.palette.overlay0
                } else {
                    status_color(row.agent.agent_status, &config.palette)
                })
                .add_modifier(if stale {
                    Modifier::DIM
                } else {
                    Modifier::empty()
                }),
        );
        hits.endpoint_agents.push((
            rect,
            row.endpoint.endpoint_id.clone(),
            row.agent.pane_id.clone(),
        ));
    }
}

pub(super) fn render_expanded(
    buffer: &mut Buffer,
    area: Rect,
    agent_view_label: Option<&str>,
    endpoints: &[ClientShellEndpoint],
    active_endpoint_id: &ClientEndpointId,
    config: &ClientShellConfig,
    agent_grid: Option<bool>,
    clock: AgentClock,
    agent_scroll: &mut usize,
    hits: &mut ShellHitMap,
) -> bool {
    if !super::agent_sidebar::render_agent_panel_header(
        buffer,
        area,
        agent_view_label,
        config,
        agent_grid,
        hits,
    ) {
        return false;
    }
    let lines = expanded_lines(endpoints, active_endpoint_id, config, clock);
    super::agent_sidebar::render_panel_lines(
        buffer,
        area,
        &lines,
        agent_view_label.map(|_| " no matching agents"),
        config,
        clock,
        agent_scroll,
        hits,
    );
    super::agent_sidebar::panel_lines_animate(&lines)
}

/// Panel lines across every endpoint: grouped per machine, or one flat list
/// when an agent view or the priority sort orders agents across machines.
fn expanded_lines(
    endpoints: &[ClientShellEndpoint],
    active_endpoint_id: &ClientEndpointId,
    config: &ClientShellConfig,
    clock: AgentClock,
) -> Vec<super::agent_sidebar::PanelLine> {
    let many = endpoints.len() > 1;
    let cached =
        super::aggregate_navigation::cached_endpoint_snapshots(endpoints).collect::<Vec<_>>();
    let sources = cached
        .iter()
        .map(|endpoint| super::agent_sidebar::AgentPanelSource {
            endpoint_id: Some(endpoint.endpoint_id),
            machine: many.then_some(endpoint.label),
            stale: endpoint.stale(),
            active: endpoint.endpoint_id == active_endpoint_id,
            snapshot: endpoint.snapshot,
        })
        .collect::<Vec<_>>();
    let agent_view = endpoints
        .iter()
        .find(|endpoint| &endpoint.endpoint_id == active_endpoint_id)
        .and_then(|endpoint| endpoint.snapshot.as_deref())
        .is_some_and(|snapshot| snapshot.agent_view_label.is_some());
    let flat = (agent_view
        || config.agent_panel_sort == crate::config::AgentPanelSortConfig::Priority)
        .then(|| {
            super::aggregate_navigation::aggregate_agent_rows(
                endpoints,
                active_endpoint_id,
                config.agent_panel_sort,
            )
            .into_iter()
            .filter_map(|row| {
                let source = cached
                    .iter()
                    .position(|endpoint| endpoint.endpoint_index == row.endpoint.endpoint_index)?;
                Some((source, row.agent.pane_id.clone()))
            })
            .collect::<Vec<_>>()
        });
    super::agent_sidebar::panel_lines(&sources, flat.as_deref(), config, clock)
}

impl ClientShellState {
    pub(super) fn reveal_endpoint_agent(
        &mut self,
        endpoint_id: &ClientEndpointId,
        pane_id: &str,
        body_height: u16,
    ) {
        if body_height == 0 {
            return;
        }
        let lines = expanded_lines(
            &self.endpoints,
            &self.active_endpoint_id,
            &self.config,
            self.agent_clock(std::time::Instant::now()),
        );
        if let Some(start) = super::agent_sidebar::reveal_panel_agent(
            &lines,
            Some(endpoint_id),
            pane_id,
            body_height,
            self.agent_scroll,
        ) {
            self.agent_scroll = start;
        }
    }
}
