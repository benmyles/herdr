//! The command palette: one fuzzy-searchable list of every place and command
//! the client can reach. Places come from each connected endpoint's snapshot
//! (agents, terminals, workspaces, tabs, spaces, machines); commands come
//! from the keybinding table, settings sections, and endpoint commands, and
//! run through the same paths their keybindings and menus use.

use super::*;

use crate::api::schema::AgentStatus;
use crate::input::KeybindAction;

#[derive(Debug)]
pub(super) struct ClientCommandPaletteOverlay {
    pub(super) query: TextEditor,
    /// Index into the ranked results.
    pub(super) selected: usize,
    /// First result shown.
    pub(super) scroll: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PaletteKind {
    Agent,
    Terminal,
    Workspace,
    Tab,
    Space,
    Machine,
    View,
    Settings,
    Command,
}

impl PaletteKind {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Agent => "agent",
            Self::Terminal => "terminal",
            Self::Workspace => "workspace",
            Self::Tab => "tab",
            Self::Space => "space",
            Self::Machine => "machine",
            Self::View => "view",
            Self::Settings => "settings",
            Self::Command => "command",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum PaletteAction {
    Focus {
        endpoint_id: ClientEndpointId,
        target: ClientEndpointFocusTarget,
    },
    Machine(ClientEndpointId),
    Binding(KeybindAction),
    EndpointCommand {
        command_id: String,
        action: crate::protocol::ClientShellCommandAction,
    },
    Settings(ClientSettingsSection),
    AgentGrid,
    /// Switches the grid between every agent and the active ones.
    AgentGridFilter,
    NewSpace,
    WhatsNew,
}

#[derive(Clone, Debug)]
pub(super) struct PaletteEntry {
    pub(super) kind: PaletteKind,
    pub(super) title: String,
    /// Where the entry lives or what it does, shown muted after the title.
    pub(super) detail: String,
    /// Keybinding that runs the entry directly.
    pub(super) hint: Option<String>,
    pub(super) status: Option<AgentStatus>,
    /// The place the client is on now.
    pub(super) current: bool,
    /// From an endpoint that is not online.
    pub(super) stale: bool,
    pub(super) action: PaletteAction,
    /// Names the item in the usage stats across restarts: kind, machine, and
    /// a stable name, not the endpoint's ids.
    pub(super) usage_key: String,
    /// Ranking points from how often and how recently it was chosen.
    pub(super) boost: i32,
}

pub(super) struct PaletteMatch<'a> {
    pub(super) entry: &'a PaletteEntry,
    /// Char indices of the title the query matched.
    pub(super) highlights: Vec<usize>,
}

/// The letter that opens result `index` with ctrl: a to z for the first 26.
pub(super) fn pick_letter(index: usize) -> Option<char> {
    u8::try_from(index)
        .ok()
        .filter(|index| *index < 26)
        .map(|index| char::from(b'a' + index))
}

/// The result index a pick letter opens.
fn pick_index(letter: char) -> Option<usize> {
    letter
        .is_ascii_lowercase()
        .then(|| usize::from(letter as u8 - b'a'))
}

/// Entries matching `query`, best first; entries keep their listed order
/// among equal scores and when the query is empty. Spaces are optional: an
/// entry matches when every space-separated term matches its title, detail,
/// kind, or keybinding (title matches count double, so "settings theme"
/// finds the theme tab), or when the query without spaces matches across its
/// title and location together, so "a1" finds tab 1 of workspace "api".
/// Past picks boost an entry, and with no query the most chosen come first.
/// Matches scoring under a third of the best one are dropped as noise.
pub(super) fn rank<'a>(entries: &'a [PaletteEntry], query: &str) -> Vec<PaletteMatch<'a>> {
    let terms = query.split_whitespace().collect::<Vec<_>>();
    if terms.is_empty() {
        let mut listed = entries
            .iter()
            .map(|entry| PaletteMatch {
                entry,
                highlights: Vec::new(),
            })
            .collect::<Vec<_>>();
        // The most chosen come first; the rest keep their listed order.
        listed.sort_by_key(|found| std::cmp::Reverse(found.entry.boost));
        return listed;
    }
    let whole = query.trim().to_lowercase();
    let compact = query.split_whitespace().collect::<String>();
    let mut scored = entries
        .iter()
        .filter_map(|entry| {
            let (mut score, highlights) = [
                score_terms(entry, &terms),
                score_across_fields(entry, &compact),
            ]
            .into_iter()
            .flatten()
            .max_by_key(|(score, _)| *score)?;
            if entry.title.to_lowercase().starts_with(&whole) {
                score += 50;
            }
            score += entry.boost;
            Some((score, PaletteMatch { entry, highlights }))
        })
        .collect::<Vec<_>>();
    scored.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
    let floor = scored.first().map_or(0, |(best, _)| best / 3);
    scored
        .into_iter()
        .take_while(|(score, _)| *score >= floor)
        .map(|(_, found)| found)
        .collect()
}

/// The query matched in order through the entry's fields joined, title first
/// or location first, highlighting the characters that land in the title.
fn score_across_fields(entry: &PaletteEntry, compact: &str) -> Option<(i32, Vec<usize>)> {
    let title_len = entry.title.chars().count();
    let tail = format!(
        " {} {}",
        entry.kind.label(),
        entry.hint.as_deref().unwrap_or_default()
    );
    [
        (format!("{} {}{tail}", entry.title, entry.detail), 0),
        (
            format!("{} {}{tail}", entry.detail, entry.title),
            entry.detail.chars().count() + 1,
        ),
    ]
    .into_iter()
    .filter_map(|(joined, title_start)| {
        let (score, positions) = super::fuzzy::fuzzy_match(compact, &joined)?;
        let title = title_start..title_start + title_len;
        let highlights = positions
            .into_iter()
            .filter(|position| title.contains(position))
            .map(|position| position - title_start)
            .collect();
        Some((score, highlights))
    })
    .max_by_key(|(score, _)| *score)
}

fn score_terms(entry: &PaletteEntry, terms: &[&str]) -> Option<(i32, Vec<usize>)> {
    let mut total = 0;
    let mut highlights = Vec::new();
    for term in terms {
        let title = super::fuzzy::fuzzy_match(term, &entry.title)
            .map(|(score, positions)| (score * 2, Some(positions)));
        let elsewhere = [
            entry.detail.as_str(),
            entry.kind.label(),
            entry.hint.as_deref().unwrap_or_default(),
        ]
        .into_iter()
        .filter_map(|field| super::fuzzy::fuzzy_match(term, field))
        .map(|(score, _)| (score, None))
        .max_by_key(|(score, _)| *score);
        let (score, positions) = [title, elsewhere]
            .into_iter()
            .flatten()
            .max_by_key(|(score, _)| *score)?;
        total += score;
        highlights.extend(positions.into_iter().flatten());
    }
    highlights.sort_unstable();
    highlights.dedup();
    Some((total, highlights))
}

fn status_word(status: AgentStatus) -> Option<&'static str> {
    match status {
        AgentStatus::Blocked => Some("blocked"),
        AgentStatus::Working => Some("working"),
        AgentStatus::Idle => Some("idle"),
        AgentStatus::Done => Some("done"),
        AgentStatus::Unknown => None,
    }
}

/// "space › workspace/tab", naming the tab only when the workspace has more
/// than one or it was renamed, as grid tile titles do.
fn place(snapshot: &ClientShellSnapshot, workspace_id: &str, tab_id: &str) -> String {
    let Some(workspace) = snapshot
        .workspaces
        .iter()
        .find(|workspace| workspace.workspace_id == workspace_id)
    else {
        return String::new();
    };
    let tab_count = snapshot
        .tabs
        .iter()
        .filter(|tab| tab.workspace_id == workspace_id)
        .count();
    let mut place = workspace.label.clone();
    if let Some(tab) = snapshot
        .tabs
        .iter()
        .find(|tab| tab.tab_id == tab_id)
        .filter(|tab| tab_count > 1 || tab.custom_label)
    {
        place = format!("{place}/{}", tab.label);
    }
    match space_name(snapshot, workspace) {
        Some(space) => format!("{space} › {place}"),
        None => place,
    }
}

fn space_name<'a>(
    snapshot: &'a ClientShellSnapshot,
    workspace: &crate::protocol::ClientShellWorkspace,
) -> Option<&'a str> {
    let space_id = workspace.space_id.as_deref()?;
    snapshot
        .spaces
        .iter()
        .find(|space| space.space_id == space_id && !space.built_in)
        .map(|space| space.name.as_str())
}

fn joined<'a>(parts: impl IntoIterator<Item = &'a str>) -> String {
    parts
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" · ")
}

/// Places on one endpoint, in sidebar order: agents, other terminals,
/// workspaces, renamed or extra tabs, then spaces.
fn endpoint_places(
    endpoint: &super::aggregate_navigation::CachedEndpointSnapshot<'_>,
    machine: Option<&str>,
    active: bool,
    entries: &mut Vec<PaletteEntry>,
) {
    let snapshot = endpoint.snapshot;
    let stale = endpoint.stale();
    let focus = |target| PaletteAction::Focus {
        endpoint_id: endpoint.endpoint_id.clone(),
        target,
    };
    let machine = machine.unwrap_or_default();
    let endpoint_label = endpoint.label;

    for pane_id in super::agent_sidebar::ordered_agent_pane_ids(
        snapshot,
        crate::config::AgentPanelSortConfig::Spaces,
    ) {
        let Some(agent) = snapshot
            .agents
            .iter()
            .find(|agent| agent.pane_id == pane_id)
        else {
            continue;
        };
        let cwd = snapshot
            .panes
            .iter()
            .find(|pane| pane.pane_id == agent.pane_id)
            .and_then(|pane| pane.cwd.as_deref());
        let vendor = agent
            .display_agent
            .as_deref()
            .or(agent.agent.as_deref())
            .unwrap_or_default();
        let location = place(snapshot, &agent.workspace_id, &agent.tab_id);
        entries.push(PaletteEntry {
            kind: PaletteKind::Agent,
            title: super::agent_marks::session_title(agent, cwd),
            detail: joined([
                vendor,
                machine,
                &location,
                status_word(agent.agent_status).unwrap_or_default(),
            ]),
            hint: None,
            status: Some(agent.agent_status),
            current: active && agent.focused,
            stale,
            action: focus(ClientEndpointFocusTarget::Pane(agent.pane_id.clone())),
            // Session titles change as an agent works; its seat does not.
            usage_key: format!("agent:{endpoint_label}:{location}:{vendor}"),
            boost: 0,
        });
    }

    for pane in &snapshot.panes {
        if snapshot
            .agents
            .iter()
            .any(|agent| agent.pane_id == pane.pane_id)
        {
            continue;
        }
        let cwd = pane
            .foreground_cwd
            .as_deref()
            .or(pane.cwd.as_deref())
            .unwrap_or_default();
        let folder = cwd
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .filter(|folder| !folder.is_empty())
            .unwrap_or("terminal");
        let location = place(snapshot, &pane.workspace_id, &pane.tab_id);
        let title = pane.label.clone().unwrap_or_else(|| folder.to_owned());
        entries.push(PaletteEntry {
            kind: PaletteKind::Terminal,
            usage_key: format!("terminal:{endpoint_label}:{location}:{title}"),
            title,
            detail: joined([machine, &location, cwd]),
            hint: None,
            status: None,
            current: active && pane.focused,
            stale,
            action: focus(ClientEndpointFocusTarget::Pane(pane.pane_id.clone())),
            boost: 0,
        });
    }

    let groups = super::agent_sidebar::grouped_workspaces(snapshot);
    for &index in groups.iter().flat_map(|group| &group.workspaces) {
        let workspace = &snapshot.workspaces[index];
        let space = workspace.space_id.as_deref().and_then(|space_id| {
            snapshot
                .spaces
                .iter()
                .find(|space| space.space_id == space_id)
        });
        entries.push(PaletteEntry {
            kind: PaletteKind::Workspace,
            title: workspace.label.clone(),
            detail: joined([
                machine,
                space_name(snapshot, workspace).unwrap_or_default(),
                super::agent_sidebar::distinct_branch(workspace, space).unwrap_or_default(),
            ]),
            hint: None,
            status: None,
            current: active
                && snapshot.focused_workspace_id.as_deref() == Some(&workspace.workspace_id),
            stale,
            action: focus(ClientEndpointFocusTarget::Workspace(
                workspace.workspace_id.clone(),
            )),
            usage_key: format!("workspace:{endpoint_label}:{}", workspace.label),
            boost: 0,
        });
        let tabs = snapshot
            .tabs
            .iter()
            .filter(|tab| tab.workspace_id == workspace.workspace_id)
            .collect::<Vec<_>>();
        for tab in tabs.iter().filter(|tab| tabs.len() > 1 || tab.custom_label) {
            entries.push(PaletteEntry {
                kind: PaletteKind::Tab,
                title: tab.label.clone(),
                detail: joined([machine, &workspace.label]),
                hint: None,
                status: None,
                current: active && tab.focused,
                stale,
                action: focus(ClientEndpointFocusTarget::Tab(tab.tab_id.clone())),
                usage_key: format!("tab:{endpoint_label}:{}/{}", workspace.label, tab.label),
                boost: 0,
            });
        }
    }

    for group in &groups {
        let (Some(space), Some(&first)) = (
            group.space.and_then(|index| snapshot.spaces.get(index)),
            group.workspaces.first(),
        ) else {
            continue;
        };
        if space.built_in {
            continue;
        }
        let count = group.workspaces.len();
        entries.push(PaletteEntry {
            kind: PaletteKind::Space,
            title: space.name.clone(),
            detail: joined([
                machine,
                &format!("{count} worktree{}", if count == 1 { "" } else { "s" }),
            ]),
            hint: None,
            status: None,
            current: false,
            stale,
            action: focus(ClientEndpointFocusTarget::Workspace(
                snapshot.workspaces[first].workspace_id.clone(),
            )),
            usage_key: format!("space:{endpoint_label}:{}", space.name),
            boost: 0,
        });
    }
}

impl ClientShellState {
    /// Everything the palette offers right now, boosted by past picks.
    pub(super) fn command_palette_entries(&self) -> Vec<PaletteEntry> {
        let mut entries = self.unboosted_command_palette_entries();
        let now = super::palette_usage::now();
        for entry in &mut entries {
            entry.boost = self.palette_usage.boost(&entry.usage_key, now);
        }
        entries
    }

    fn unboosted_command_palette_entries(&self) -> Vec<PaletteEntry> {
        let mut entries = Vec::new();
        let many = self.endpoints.len() > 1;
        let endpoints = super::aggregate_navigation::cached_endpoint_snapshots(&self.endpoints)
            .collect::<Vec<_>>();
        for endpoint in &endpoints {
            let active = *endpoint.endpoint_id == self.active_endpoint_id;
            endpoint_places(
                endpoint,
                many.then_some(endpoint.label),
                active,
                &mut entries,
            );
        }
        if many {
            entries.extend(endpoints.iter().map(|endpoint| {
                PaletteEntry {
                    kind: PaletteKind::Machine,
                    title: endpoint.label.to_owned(),
                    detail: if endpoint.stale() {
                        "offline"
                    } else {
                        "online"
                    }
                    .to_owned(),
                    hint: None,
                    status: None,
                    current: *endpoint.endpoint_id == self.active_endpoint_id,
                    stale: endpoint.stale(),
                    action: PaletteAction::Machine(endpoint.endpoint_id.clone()),
                    usage_key: format!("machine:{}", endpoint.label),
                    boost: 0,
                }
            }));
        }

        // The grid needs its sidebar toggle, which a collapsed sidebar hides.
        if let Some(heading) = self
            .agent_grid_toggle_state()
            .filter(|_| !self.sidebar_collapsed)
        {
            entries.push(PaletteEntry {
                kind: PaletteKind::View,
                title: if heading.shown {
                    "hide agent grid"
                } else {
                    "show agent grid"
                }
                .to_owned(),
                detail: match heading.filter {
                    Some(crate::api::schema::AgentGridFilter::Active) => {
                        "tile agents that are working or waiting on you"
                    }
                    _ => "tile every live agent",
                }
                .to_owned(),
                hint: None,
                status: None,
                current: false,
                stale: false,
                action: PaletteAction::AgentGrid,
                usage_key: "view:agent grid".to_owned(),
                boost: 0,
            });
            if let Some(filter) = heading.filter {
                let (title, detail) = match filter {
                    crate::api::schema::AgentGridFilter::All => (
                        "agent grid: active agents",
                        "only agents working or waiting on you",
                    ),
                    crate::api::schema::AgentGridFilter::Active => {
                        ("agent grid: all agents", "every live agent")
                    }
                };
                entries.push(PaletteEntry {
                    kind: PaletteKind::View,
                    title: title.to_owned(),
                    detail: detail.to_owned(),
                    hint: None,
                    status: None,
                    current: false,
                    stale: false,
                    action: PaletteAction::AgentGridFilter,
                    usage_key: "view:agent grid filter".to_owned(),
                    boost: 0,
                });
            }
        }

        entries.extend(
            ClientSettingsSection::ALL
                .iter()
                .map(|&section| PaletteEntry {
                    kind: PaletteKind::Settings,
                    title: section.label().to_owned(),
                    detail: "open settings".to_owned(),
                    hint: None,
                    status: None,
                    current: false,
                    stale: false,
                    action: PaletteAction::Settings(section),
                    usage_key: format!("settings:{}", section.label()),
                    boost: 0,
                }),
        );

        let command = |title: &str, detail: &str, hint: Option<String>, action| PaletteEntry {
            kind: PaletteKind::Command,
            title: title.to_owned(),
            detail: detail.to_owned(),
            hint,
            status: None,
            current: false,
            stale: false,
            action,
            usage_key: format!("command:{title}"),
            boost: 0,
        };
        for (bindings, action) in crate::input::action_bindings(&self.config.keybinds.keybinds) {
            if action != KeybindAction::CommandPalette {
                entries.push(command(
                    action.label(),
                    "",
                    bindings.label(),
                    PaletteAction::Binding(action),
                ));
            }
        }
        if self.active_endpoint_supports_spaces() {
            entries.push(command(
                "new space",
                "group worktrees in the sidebar",
                None,
                PaletteAction::NewSpace,
            ));
        }
        if let Some(snapshot) = self.snapshot.as_deref() {
            // Release notes open only once their body has arrived.
            if snapshot.release_notes.is_some() {
                entries.push(command(
                    if snapshot.update_available.is_some() {
                        "update ready"
                    } else {
                        "what's new"
                    },
                    "release notes",
                    None,
                    PaletteAction::WhatsNew,
                ));
            }
            entries.extend(snapshot.commands.iter().map(|endpoint_command| {
                command(
                    endpoint_command
                        .description
                        .as_deref()
                        .unwrap_or(&endpoint_command.command_id),
                    "custom command",
                    (!endpoint_command.binding_label.is_empty())
                        .then(|| endpoint_command.binding_label.clone()),
                    PaletteAction::EndpointCommand {
                        command_id: endpoint_command.command_id.clone(),
                        action: endpoint_command.action,
                    },
                )
            }));
        }
        entries
    }

    pub(super) fn open_command_palette(&mut self) {
        self.overlay = Some(ClientShellOverlay::CommandPalette(
            ClientCommandPaletteOverlay {
                query: TextEditor::default(),
                selected: 0,
                scroll: 0,
            },
        ));
    }

    fn command_palette_result_count(&self) -> usize {
        let Some(ClientShellOverlay::CommandPalette(palette)) = self.overlay.as_ref() else {
            return 0;
        };
        rank(&self.command_palette_entries(), palette.query.as_str()).len()
    }

    /// Moves the selection, scrolling so it stays among the rows last drawn.
    pub(super) fn move_command_palette_selection(&mut self, delta: isize) {
        let count = self.command_palette_result_count();
        let rows = self.hits.command_palette_rows.len().max(1);
        let Some(ClientShellOverlay::CommandPalette(palette)) = self.overlay.as_mut() else {
            return;
        };
        if count == 0 {
            return;
        }
        palette.selected =
            (palette.selected as isize + delta).clamp(0, count as isize - 1) as usize;
        if palette.selected < palette.scroll {
            palette.scroll = palette.selected;
        } else if palette.selected >= palette.scroll + rows {
            palette.scroll = palette.selected + 1 - rows;
        }
    }

    fn reset_command_palette_results(&mut self) {
        if let Some(ClientShellOverlay::CommandPalette(palette)) = self.overlay.as_mut() {
            palette.selected = 0;
            palette.scroll = 0;
        }
    }

    pub(super) fn insert_command_palette_text(&mut self, text: &str) -> bool {
        let Some(ClientShellOverlay::CommandPalette(palette)) = self.overlay.as_mut() else {
            return false;
        };
        if palette.query.insert(text) {
            self.reset_command_palette_results();
        }
        true
    }

    /// Runs the selected result and closes the palette.
    pub(super) fn accept_command_palette(&mut self, outcome: &mut ClientShellInput) {
        let entries = self.command_palette_entries();
        let Some(ClientShellOverlay::CommandPalette(palette)) = self.overlay.as_ref() else {
            return;
        };
        let Some((action, usage_key)) = rank(&entries, palette.query.as_str())
            .get(palette.selected)
            .map(|found| (found.entry.action.clone(), found.entry.usage_key.clone()))
        else {
            return;
        };
        self.overlay = None;
        outcome.repaint = true;
        self.record_palette_usage(&usage_key);
        match action {
            PaletteAction::Focus {
                endpoint_id,
                target,
            } => {
                self.focus_or_activate(endpoint_id, target, outcome);
            }
            PaletteAction::Machine(endpoint_id) => {
                self.activate_endpoint(endpoint_id, outcome);
            }
            PaletteAction::Binding(action) => {
                self.record_binding(crate::input::KeybindMatch::Action(action), outcome)
            }
            PaletteAction::EndpointCommand { command_id, action } => {
                self.invoke_endpoint_command(command_id, action, outcome)
            }
            PaletteAction::Settings(section) => {
                self.open_settings_overlay();
                self.select_settings_section(section, outcome);
            }
            PaletteAction::AgentGrid => self.toggle_agent_grid(outcome),
            PaletteAction::AgentGridFilter => self.cycle_agent_grid_filter(outcome),
            PaletteAction::NewSpace => self.begin_new_space(None),
            PaletteAction::WhatsNew => self.open_release_notes(),
        }
    }

    fn record_palette_usage(&mut self, usage_key: &str) {
        self.palette_usage
            .record(usage_key, super::palette_usage::now());
        if let Some(path) = self.config.palette_usage_path.as_deref() {
            if let Err(error) = super::palette_usage::store(path, &self.palette_usage) {
                tracing::warn!(%error, "failed to save command palette usage");
            }
        }
    }

    /// Opens the result labelled `letter`, when that many results match.
    fn pick_command_palette_result(&mut self, letter: char, outcome: &mut ClientShellInput) {
        let Some(index) = pick_index(letter) else {
            return;
        };
        if index >= self.command_palette_result_count() {
            return;
        }
        if let Some(ClientShellOverlay::CommandPalette(palette)) = self.overlay.as_mut() {
            palette.selected = index;
        }
        self.accept_command_palette(outcome);
    }

    /// Fills the query with the selected result's title.
    fn complete_command_palette(&mut self) {
        let entries = self.command_palette_entries();
        let Some(ClientShellOverlay::CommandPalette(palette)) = self.overlay.as_ref() else {
            return;
        };
        let Some(title) = rank(&entries, palette.query.as_str())
            .get(palette.selected)
            .map(|found| found.entry.title.clone())
        else {
            return;
        };
        if let Some(ClientShellOverlay::CommandPalette(palette)) = self.overlay.as_mut() {
            palette.query = TextEditor::new(&title, false);
        }
        self.reset_command_palette_results();
    }

    /// Handles a key while the palette is open. Returns false when it is not.
    pub(super) fn route_command_palette_key(
        &mut self,
        key: &crate::input::TerminalKey,
        outcome: &mut ClientShellInput,
    ) -> bool {
        use crossterm::event::KeyModifiers;

        if !matches!(self.overlay, Some(ClientShellOverlay::CommandPalette(_))) {
            return false;
        }
        outcome.repaint = true;
        let (code, modifiers) = crate::config::normalize_key_combo((key.code, key.modifiers));
        let page = self.hits.command_palette_rows.len().max(1) as isize;
        let control = modifiers == KeyModifiers::CONTROL;
        match code {
            KeyCode::Esc => self.overlay = None,
            KeyCode::Enter => self.accept_command_palette(outcome),
            KeyCode::Tab => self.complete_command_palette(),
            KeyCode::Up => self.move_command_palette_selection(-1),
            KeyCode::Down => self.move_command_palette_selection(1),
            // Ctrl with a result's letter opens it, ahead of editing shortcuts.
            KeyCode::Char(letter) if control && letter.is_ascii_lowercase() => {
                self.pick_command_palette_result(letter, outcome)
            }
            KeyCode::PageUp => self.move_command_palette_selection(-page),
            KeyCode::PageDown => self.move_command_palette_selection(page),
            _ => {
                let Some(ClientShellOverlay::CommandPalette(palette)) = self.overlay.as_mut()
                else {
                    return true;
                };
                if palette.query.handle_key(key) == Some(true) {
                    self.reset_command_palette_results();
                }
            }
        }
        true
    }

    /// Handles the mouse while the palette is open. Returns false when it is not.
    pub(super) fn route_command_palette_mouse(
        &mut self,
        mouse: crossterm::event::MouseEvent,
        outcome: &mut ClientShellInput,
    ) -> bool {
        use crossterm::event::{MouseButton, MouseEventKind};

        if !matches!(self.overlay, Some(ClientShellOverlay::CommandPalette(_))) {
            return false;
        }
        let point = (mouse.column, mouse.row);
        let row = self
            .hits
            .command_palette_rows
            .iter()
            .find(|(rect, _)| super::contains(*rect, point))
            .map(|(_, index)| *index);
        let select = |state: &mut Self, index: usize| {
            if let Some(ClientShellOverlay::CommandPalette(palette)) = state.overlay.as_mut() {
                palette.selected = index;
            }
        };
        match mouse.kind {
            MouseEventKind::Moved => {
                if let Some(index) = row {
                    select(self, index);
                    outcome.repaint = true;
                }
            }
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some(index) = row {
                    select(self, index);
                    self.accept_command_palette(outcome);
                } else if !super::contains(self.hits.command_palette_popup, point) {
                    self.overlay = None;
                    outcome.repaint = true;
                }
            }
            MouseEventKind::ScrollUp => {
                self.move_command_palette_selection(-3);
                outcome.repaint = true;
            }
            MouseEventKind::ScrollDown => {
                self.move_command_palette_selection(3);
                outcome.repaint = true;
            }
            _ => {}
        }
        true
    }
}
