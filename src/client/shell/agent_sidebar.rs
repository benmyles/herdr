use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
};

use super::*;

use super::agent_marks::{AgentClock, MarkState};
use crate::protocol::ClientShellAgent;

/// Agents in panel order. Grouped: the sidebar's space and workspace order,
/// then urgency within each workspace. Priority: urgency across everything.
pub(super) fn ordered_agent_pane_ids(
    snapshot: &ClientShellSnapshot,
    sort: crate::config::AgentPanelSortConfig,
) -> Vec<String> {
    if snapshot.agent_view_label.is_some() {
        return snapshot
            .agent_order
            .iter()
            .filter(|pane_id| {
                snapshot
                    .agents
                    .iter()
                    .any(|agent| agent.pane_id == pane_id.as_str())
            })
            .cloned()
            .collect();
    }
    let mut agents = grouped_workspaces(snapshot)
        .into_iter()
        .flat_map(|group| group.workspaces)
        .flat_map(|index| workspace_agents(snapshot, index))
        .collect::<Vec<_>>();
    // Agents whose workspace the snapshot does not list still get a place.
    agents.extend(snapshot.agents.iter().filter(|agent| {
        !snapshot
            .workspaces
            .iter()
            .any(|workspace| workspace.workspace_id == agent.workspace_id)
    }));
    if sort == crate::config::AgentPanelSortConfig::Priority {
        agents.sort_by_key(|agent| urgency(agent));
    }
    agents
        .into_iter()
        .map(|agent| agent.pane_id.clone())
        .collect()
}

/// Most urgent first; the latest change first among equals.
fn urgency(agent: &ClientShellAgent) -> (std::cmp::Reverse<u8>, std::cmp::Reverse<u64>) {
    (
        std::cmp::Reverse(status_priority(agent.agent_status)),
        std::cmp::Reverse(agent.state_change_seq),
    )
}

/// One workspace's agents, most urgent first.
fn workspace_agents(snapshot: &ClientShellSnapshot, index: usize) -> Vec<&ClientShellAgent> {
    let Some(workspace) = snapshot.workspaces.get(index) else {
        return Vec::new();
    };
    let mut agents = snapshot
        .agents
        .iter()
        .filter(|agent| agent.workspace_id == workspace.workspace_id)
        .collect::<Vec<_>>();
    agents.sort_by_key(|agent| urgency(agent));
    agents
}

/// A space and its workspaces in sidebar order. `space` is `None` for
/// workspaces from servers without spaces.
pub(super) struct WorkspaceGroup {
    pub(super) space: Option<usize>,
    pub(super) workspaces: Vec<usize>,
}

pub(super) fn grouped_workspaces(snapshot: &ClientShellSnapshot) -> Vec<WorkspaceGroup> {
    let mut listed = vec![false; snapshot.workspaces.len()];
    let mut groups = snapshot
        .spaces
        .iter()
        .enumerate()
        .map(|(space_index, space)| {
            let workspaces = snapshot
                .workspaces
                .iter()
                .enumerate()
                .filter(|(_, workspace)| workspace.space_id.as_deref() == Some(&space.space_id))
                .map(|(index, _)| index)
                .collect::<Vec<_>>();
            for &index in &workspaces {
                listed[index] = true;
            }
            WorkspaceGroup {
                space: Some(space_index),
                workspaces,
            }
        })
        .collect::<Vec<_>>();
    groups.extend(
        listed
            .into_iter()
            .enumerate()
            .filter(|(_, listed)| !listed)
            .map(|(index, _)| WorkspaceGroup {
                space: None,
                workspaces: vec![index],
            }),
    );
    groups
}

/// The branch worth showing beside a worktree's label: not when it repeats
/// the label, or the space name worktrees are named after by default.
pub(super) fn distinct_branch<'a>(
    workspace: &'a crate::protocol::ClientShellWorkspace,
    space: Option<&crate::protocol::ClientShellSpace>,
) -> Option<&'a str> {
    let branch = workspace.branch.as_deref()?;
    let space_name = space
        .filter(|space| !space.built_in)
        .map(|space| space.name.split_whitespace().collect::<Vec<_>>().join("-"));
    (branch != workspace.label && !space_name.is_some_and(|name| name.eq_ignore_ascii_case(branch)))
        .then_some(branch)
}

/// One endpoint whose agents the panel lists.
pub(super) struct AgentPanelSource<'a> {
    /// Set when agent hits must name their endpoint.
    pub(super) endpoint_id: Option<&'a ClientEndpointId>,
    /// Set when several machines share the panel.
    pub(super) machine: Option<&'a str>,
    pub(super) stale: bool,
    /// Focus marks count only on the active endpoint.
    pub(super) active: bool,
    /// The view of this endpoint's live agent grid while it is shown; rows
    /// then mark the agents it shows.
    pub(super) agent_grid: Option<crate::api::schema::AgentGridFilter>,
    pub(super) snapshot: &'a ClientShellSnapshot,
}

pub(super) enum PanelLine {
    Machine {
        label: String,
        stale: bool,
    },
    Space {
        name: String,
        color: ratatui::style::Color,
        stale: bool,
        indent: u16,
    },
    Worktree {
        label: String,
        branch: Option<String>,
        color: ratatui::style::Color,
        stale: bool,
        indent: u16,
        pull_request: Option<crate::api::schema::WorkspacePullRequest>,
    },
    Agent(PanelAgent),
}

impl PanelLine {
    fn agent(&self) -> Option<&PanelAgent> {
        match self {
            Self::Agent(agent) => Some(agent),
            _ => None,
        }
    }
}

pub(super) struct PanelAgent {
    pub(super) endpoint_id: Option<ClientEndpointId>,
    pub(super) pane_id: String,
    focused: bool,
    stale: bool,
    state: MarkState,
    icon: Option<(char, Style)>,
    lead_style: Style,
    title: String,
    title_style: Style,
    context: Option<String>,
    age: Option<String>,
    indent: u16,
    /// Whether the shown live agent grid has this agent's tile; `None` while
    /// no grid is shown.
    in_agent_grid: Option<bool>,
}

impl PanelAgent {
    fn new(
        source: &AgentPanelSource<'_>,
        agent: &ClientShellAgent,
        config: &ClientShellConfig,
        clock: AgentClock,
        indent: u16,
        context: Option<String>,
    ) -> Self {
        let marks = &config.agent_marks;
        let palette = &config.palette;
        let state = marks.agent_state(agent, clock.now);
        let cwd = source
            .snapshot
            .panes
            .iter()
            .find(|pane| pane.pane_id == agent.pane_id)
            .and_then(|pane| pane.cwd.as_deref());
        Self {
            endpoint_id: source.endpoint_id.cloned(),
            pane_id: agent.pane_id.clone(),
            focused: agent.focused && source.active,
            stale: source.stale,
            state,
            icon: marks
                .icon(agent)
                .map(|icon| (icon, state.icon_style(agent, palette))),
            lead_style: state.lead_style(agent, palette),
            title: super::agent_marks::session_title(agent, cwd),
            title_style: state.title_style(agent, palette),
            context,
            age: state
                .shows_age()
                .then(|| super::agent_marks::age_label(agent.state_changed_at_ms, clock.now))
                .flatten(),
            indent,
            // The grid keeps its selected agent under every view.
            in_agent_grid: source.agent_grid.map(|filter| {
                super::agent_grid::agent_grid_filter_matches(source.snapshot, agent, filter)
                    || (agent.focused
                        && source.active
                        && !super::agent_grid::agent_grid_excludes(source.snapshot, &agent.pane_id))
            }),
        }
    }
}

/// Panel lines for `sources`: grouped by machine, space, and workspace, or
/// one flat urgency-ordered list with each agent's workspace alongside.
pub(super) fn panel_lines(
    sources: &[AgentPanelSource<'_>],
    order: Option<&[(usize, String)]>,
    config: &ClientShellConfig,
    clock: AgentClock,
) -> Vec<PanelLine> {
    if let Some(order) = order {
        return order
            .iter()
            .filter_map(|(source_index, pane_id)| {
                let source = sources.get(*source_index)?;
                let agent = source
                    .snapshot
                    .agents
                    .iter()
                    .find(|agent| &agent.pane_id == pane_id)?;
                let workspace = source
                    .snapshot
                    .workspaces
                    .iter()
                    .find(|workspace| workspace.workspace_id == agent.workspace_id)
                    .map(|workspace| workspace.label.as_str());
                let context = match (source.machine, workspace) {
                    (Some(machine), Some(workspace)) => Some(format!("{machine} · {workspace}")),
                    (Some(machine), None) => Some(machine.to_owned()),
                    (None, workspace) => workspace.map(str::to_owned),
                };
                Some(PanelLine::Agent(PanelAgent::new(
                    source, agent, config, clock, 1, context,
                )))
            })
            .collect();
    }
    let mut lines = Vec::new();
    for source in sources {
        let snapshot = source.snapshot;
        if snapshot.agents.is_empty() {
            continue;
        }
        if let Some(machine) = source.machine {
            lines.push(PanelLine::Machine {
                label: machine.to_owned(),
                stale: source.stale,
            });
        }
        let spaces = super::sidebar::space_presentation(snapshot, &config.palette);
        // Each level indents one column under the one above it.
        let base = u16::from(source.machine.is_some());
        for group in grouped_workspaces(snapshot) {
            let members = group
                .workspaces
                .iter()
                .map(|&index| (index, workspace_agents(snapshot, index)))
                .filter(|(_, agents)| !agents.is_empty())
                .collect::<Vec<_>>();
            if members.is_empty() {
                continue;
            }
            let space = group.space.and_then(|index| snapshot.spaces.get(index));
            if let Some(space) = space {
                lines.push(PanelLine::Space {
                    name: space.name.clone(),
                    color: super::sidebar::space_header_color(space, &config.palette),
                    stale: source.stale,
                    indent: 1 + base,
                });
            }
            let depth = base + u16::from(space.is_some());
            for (index, agents) in members {
                let workspace = &snapshot.workspaces[index];
                lines.push(PanelLine::Worktree {
                    label: workspace.label.clone(),
                    branch: distinct_branch(workspace, space).map(str::to_owned),
                    color: spaces.color(index),
                    stale: source.stale,
                    indent: 1 + depth,
                    pull_request: workspace.pull_request.clone(),
                });
                lines.extend(agents.into_iter().map(|agent| {
                    PanelLine::Agent(PanelAgent::new(
                        source,
                        agent,
                        config,
                        clock,
                        2 + depth,
                        None,
                    ))
                }));
            }
        }
        lines.extend(
            snapshot
                .agents
                .iter()
                .filter(|agent| {
                    !snapshot
                        .workspaces
                        .iter()
                        .any(|workspace| workspace.workspace_id == agent.workspace_id)
                })
                .map(|agent| {
                    PanelLine::Agent(PanelAgent::new(source, agent, config, clock, 1, None))
                }),
        );
    }
    lines
}

pub(super) fn render_agent_panel(
    buffer: &mut Buffer,
    area: Rect,
    snapshot: &ClientShellSnapshot,
    config: &ClientShellConfig,
    agent_grid: Option<super::agent_grid::AgentGridHeading>,
    clock: AgentClock,
    agent_scroll: &mut usize,
    hits: &mut ShellHitMap,
) -> bool {
    if !render_agent_panel_header(
        buffer,
        area,
        snapshot.agent_view_label.as_deref(),
        config,
        agent_grid,
        hits,
    ) {
        return false;
    }

    let sources = [AgentPanelSource {
        endpoint_id: None,
        machine: None,
        stale: false,
        active: true,
        agent_grid: super::agent_grid::AgentGridHeading::shown_filter(agent_grid),
        snapshot,
    }];
    let flat = (snapshot.agent_view_label.is_some()
        || config.agent_panel_sort == crate::config::AgentPanelSortConfig::Priority)
        .then(|| {
            ordered_agent_pane_ids(snapshot, config.agent_panel_sort)
                .into_iter()
                .map(|pane_id| (0, pane_id))
                .collect::<Vec<_>>()
        });
    let lines = panel_lines(&sources, flat.as_deref(), config, clock);
    render_panel_lines(
        buffer,
        area,
        &lines,
        snapshot
            .agent_view_label
            .as_ref()
            .map(|_| " no matching agents"),
        config,
        clock,
        agent_scroll,
        hits,
    );
    panel_lines_animate(&lines)
}

/// Scroll start that keeps the agent line for `pane_id` in view.
pub(super) fn reveal_panel_agent(
    lines: &[PanelLine],
    endpoint_id: Option<&ClientEndpointId>,
    pane_id: &str,
    body_height: u16,
    agent_scroll: usize,
) -> Option<usize> {
    let target = lines.iter().position(|line| {
        line.agent().is_some_and(|agent| {
            agent.pane_id == pane_id && agent.endpoint_id.as_ref() == endpoint_id
        })
    })?;
    let heights = vec![1; lines.len()];
    let gaps = vec![0; lines.len()];
    Some(super::scroll::list_scroll_start_to_reveal(
        &heights,
        &gaps,
        body_height,
        agent_scroll,
        target,
    ))
}

/// Whether any line animates at `clock`'s rate.
pub(super) fn panel_lines_animate(lines: &[PanelLine]) -> bool {
    lines
        .iter()
        .any(|line| line.agent().is_some_and(|agent| agent.state.animated()))
}

pub(super) fn render_agent_panel_header(
    buffer: &mut Buffer,
    area: Rect,
    agent_view_label: Option<&str>,
    config: &ClientShellConfig,
    agent_grid: Option<super::agent_grid::AgentGridHeading>,
    hits: &mut ShellHitMap,
) -> bool {
    if area.height == 0 {
        return false;
    }
    put_text(
        buffer,
        area.x,
        area.y,
        area.width,
        &"─".repeat(area.width as usize),
        Style::default().fg(config.palette.surface_dim),
    );
    if area.height < 2 {
        return false;
    }
    let sort_label = agent_view_label.unwrap_or(match config.agent_panel_sort {
        crate::config::AgentPanelSortConfig::Spaces => "grouped",
        crate::config::AgentPanelSortConfig::Priority => "priority",
    });
    let sort_width = display_width(sort_label).min(area.width as usize) as u16;
    let sort_rect = Rect::new(
        area.right().saturating_sub(sort_width),
        area.y + 1,
        sort_width,
        1,
    );
    // The heading toggles the live agent grid and never overlaps the sort control.
    let shown = agent_grid.is_some_and(|heading| heading.shown);
    let title = " agents";
    let title_rect = Rect::new(
        area.x,
        area.y + 1,
        (display_width(title) as u16).min(sort_rect.x.saturating_sub(area.x)),
        1,
    );
    put_text(
        buffer,
        title_rect.x,
        title_rect.y,
        title_rect.width,
        title,
        Style::default()
            .fg(if shown {
                config.palette.accent
            } else {
                config.palette.overlay0
            })
            .add_modifier(Modifier::BOLD),
    );
    hits.agent_grid_toggle = if config.mouse_capture && agent_grid.is_some() {
        title_rect
    } else {
        Rect::default()
    };
    // The grid's view follows the heading and switches it; it shows the
    // remembered view while the grid is closed.
    hits.agent_grid_filter_toggle = Rect::default();
    if let Some(filter) = agent_grid.and_then(|heading| heading.filter) {
        let separator = " | ";
        let label = match filter {
            crate::api::schema::AgentGridFilter::All => "all",
            crate::api::schema::AgentGridFilter::Active => "active",
        };
        let separator_x = title_rect.right();
        let label_x = separator_x + display_width(separator) as u16;
        let label_width = display_width(label) as u16;
        // Keep a space before the sort control.
        if label_x + label_width < sort_rect.x {
            put_text(
                buffer,
                separator_x,
                title_rect.y,
                label_x - separator_x,
                separator,
                Style::default().fg(config.palette.surface_dim),
            );
            let label_rect = Rect::new(label_x, title_rect.y, label_width, 1);
            put_text(
                buffer,
                label_rect.x,
                label_rect.y,
                label_rect.width,
                label,
                Style::default()
                    .fg(if shown {
                        config.palette.accent
                    } else {
                        config.palette.overlay0
                    })
                    .add_modifier(Modifier::BOLD),
            );
            if config.mouse_capture {
                hits.agent_grid_filter_toggle = label_rect;
            }
        }
    }
    hits.agent_sort_toggle = if config.mouse_capture && agent_view_label.is_none() {
        sort_rect
    } else {
        Rect::default()
    };
    put_text(
        buffer,
        sort_rect.x,
        sort_rect.y,
        sort_rect.width,
        sort_label,
        Style::default()
            .fg(if agent_view_label.is_some() {
                config.palette.accent
            } else {
                config.palette.overlay0
            })
            .add_modifier(Modifier::BOLD),
    );
    true
}

pub(super) fn render_panel_lines(
    buffer: &mut Buffer,
    area: Rect,
    lines: &[PanelLine],
    empty_message: Option<&str>,
    config: &ClientShellConfig,
    clock: AgentClock,
    agent_scroll: &mut usize,
    hits: &mut ShellHitMap,
) {
    let body = Rect::new(
        area.x,
        area.y.saturating_add(3),
        area.width,
        area.height.saturating_sub(3),
    );
    hits.agent_body = body;
    if body.is_empty() || lines.is_empty() {
        *agent_scroll = 0;
        if let Some(message) = empty_message.filter(|_| !body.is_empty()) {
            put_text(
                buffer,
                body.x,
                body.y,
                body.width,
                message,
                Style::default()
                    .fg(config.palette.overlay0)
                    .add_modifier(Modifier::DIM),
            );
        }
        return;
    }

    let heights = vec![1; lines.len()];
    let gaps = vec![0; lines.len()];
    let metrics = super::scroll::list_scroll_metrics(&heights, &gaps, body.height, *agent_scroll);
    hits.agent_max_scroll = metrics.max_offset_from_bottom;
    hits.agent_scroll_metrics = Some(metrics);
    *agent_scroll = metrics
        .max_offset_from_bottom
        .saturating_sub(metrics.offset_from_bottom);
    let show_scrollbar = metrics.max_offset_from_bottom > 0 && body.width > 1;
    let content_width = body.width.saturating_sub(u16::from(show_scrollbar));
    for (offset, line) in lines
        .iter()
        .skip(*agent_scroll)
        .take(body.height as usize)
        .enumerate()
    {
        let rect = Rect::new(body.x, body.y + offset as u16, content_width, 1);
        render_panel_line(buffer, rect, line, config, clock);
        if let PanelLine::Agent(agent) = line {
            match &agent.endpoint_id {
                Some(endpoint_id) => {
                    hits.endpoint_agents
                        .push((rect, endpoint_id.clone(), agent.pane_id.clone()))
                }
                None => hits.agents.push((rect, agent.pane_id.clone())),
            }
        }
    }

    if show_scrollbar {
        let track = Rect::new(body.right().saturating_sub(1), body.y, 1, body.height);
        hits.agent_scrollbar = track;
        super::scroll::render_list_scrollbar(buffer, track, metrics, &config.palette);
    }
}

fn render_panel_line(
    buffer: &mut Buffer,
    rect: Rect,
    line: &PanelLine,
    config: &ClientShellConfig,
    clock: AgentClock,
) {
    let palette = &config.palette;
    let stale_style = Style::default()
        .fg(palette.overlay0)
        .add_modifier(Modifier::DIM);
    let pick = |style: Style, stale: bool| if stale { stale_style } else { style };
    match line {
        PanelLine::Machine { label, stale } => {
            put_str(
                buffer,
                rect,
                rect.x + 1,
                label,
                pick(
                    Style::default()
                        .fg(palette.overlay1)
                        .add_modifier(Modifier::BOLD),
                    *stale,
                ),
            );
        }
        PanelLine::Space {
            name,
            color,
            stale,
            indent,
        } => {
            put_str(
                buffer,
                rect,
                rect.x + indent,
                name,
                pick(
                    Style::default().fg(*color).add_modifier(Modifier::BOLD),
                    *stale,
                ),
            );
        }
        PanelLine::Worktree {
            label,
            branch,
            color,
            stale,
            indent,
            pull_request,
        } => {
            let badge = pull_request
                .as_ref()
                .filter(|_| !*stale)
                .map(|pull_request| super::sidebar::pull_request_badge(pull_request, palette))
                .unwrap_or_default();
            let rect = if badge.is_empty() {
                rect
            } else {
                super::sidebar::put_right_badge(
                    buffer,
                    Rect::new(rect.x, rect.y, rect.width.saturating_sub(1), rect.height),
                    &badge,
                    indent + 8,
                )
            };
            let x = put_str(
                buffer,
                rect,
                rect.x + indent,
                label,
                pick(
                    Style::default().fg(super::sidebar::muted_space_color(*color, palette)),
                    *stale,
                ),
            );
            if let Some(branch) = branch {
                put_str(
                    buffer,
                    rect,
                    x,
                    &format!(" · {branch}"),
                    pick(Style::default().fg(palette.overlay0), *stale),
                );
            }
        }
        PanelLine::Agent(agent) => render_panel_agent(buffer, rect, agent, config, clock),
    }
}

fn render_panel_agent(
    buffer: &mut Buffer,
    rect: Rect,
    agent: &PanelAgent,
    config: &ClientShellConfig,
    clock: AgentClock,
) {
    let palette = &config.palette;
    if agent.focused {
        buffer.set_style(rect, Style::default().bg(palette.active_row_bg));
    }
    // Agents shown in the live grid carry the lit agents heading's accent.
    if agent.in_agent_grid == Some(true) {
        put_str(
            buffer,
            rect,
            rect.x,
            AGENT_GRID_RAIL,
            Style::default().fg(palette.accent),
        );
    }
    let stale = |style: Style| {
        if agent.stale {
            Style::default()
                .fg(palette.overlay0)
                .add_modifier(Modifier::DIM)
        } else {
            style
        }
    };
    let age = agent
        .age
        .as_deref()
        .filter(|age| rect.width as usize >= display_width(age) + agent.indent as usize + 8);
    let right = age.map_or(rect.right(), |age| {
        rect.right().saturating_sub(display_width(age) as u16 + 1)
    });
    let text_rect = Rect::new(rect.x, rect.y, right.saturating_sub(rect.x), 1);
    let mut x = rect.x + agent.indent;
    if let Some((icon, style)) = agent.icon {
        x = put_str(buffer, text_rect, x, &format!("{icon} "), stale(style));
    }
    if let Some(lead) = config.agent_marks.lead(agent.state, clock.frame) {
        x = put_str(
            buffer,
            text_rect,
            x,
            &format!("{lead} "),
            stale(agent.lead_style),
        );
    }
    let title_style = if agent.focused {
        agent.title_style.add_modifier(Modifier::BOLD)
    } else {
        agent.title_style
    };
    x = put_str(buffer, text_rect, x, &agent.title, stale(title_style));
    if let Some(context) = &agent.context {
        put_str(
            buffer,
            text_rect,
            x,
            &format!(" · {context}"),
            stale(Style::default().fg(palette.overlay0)),
        );
    }
    if let Some(age) = age {
        put_str(
            buffer,
            rect,
            right + 1,
            age,
            stale(Style::default().fg(palette.overlay0)),
        );
    }
}

/// Left-edge mark on agent rows the shown live grid has a tile for. Agent
/// rows always indent at least one column, so the rail never covers text.
const AGENT_GRID_RAIL: &str = "▎";

/// Draws `text` from `x`, clipped to `rect`, ending with `…` when cut.
/// Returns the column after the text.
fn put_str(buffer: &mut Buffer, rect: Rect, x: u16, text: &str, style: Style) -> u16 {
    let available = rect.right().saturating_sub(x) as usize;
    if available == 0 || rect.height == 0 {
        return x;
    }
    let text = crate::ui::truncate_end(text, available);
    let (end, _) = buffer.set_stringn(x, rect.y, &text, available, style);
    end
}

fn put_text(buffer: &mut Buffer, x: u16, y: u16, width: u16, text: &str, style: Style) {
    for (offset, character) in text.chars().take(width as usize).enumerate() {
        if let Some(cell) = buffer.cell_mut((x + offset as u16, y)) {
            cell.set_char(character).set_style(style);
        }
    }
}

fn display_width(text: &str) -> usize {
    unicode_width::UnicodeWidthStr::width(text)
}
