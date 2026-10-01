use super::*;

impl ClientContextMenuOverlay {
    pub(super) fn items(&self) -> Vec<ClientContextMenuItem> {
        let mut items = self.target_items();
        let pull_request = match &self.target {
            ClientContextMenuTarget::Workspace { pull_request, .. }
            | ClientContextMenuTarget::Pane { pull_request, .. }
            | ClientContextMenuTarget::Agent { pull_request, .. } => *pull_request,
            _ => false,
        };
        if pull_request {
            items.insert(
                0,
                ClientContextMenuItem {
                    label: "Open PR",
                    action: ClientContextMenuAction::OpenPullRequest,
                },
            );
        }
        items
    }

    fn target_items(&self) -> Vec<ClientContextMenuItem> {
        use ClientContextMenuAction as Action;

        let item = |label, action| ClientContextMenuItem { label, action };
        match &self.target {
            ClientContextMenuTarget::Space {
                built_in,
                collapsed,
                editable,
                worktrees,
                agent_context,
                ..
            } => {
                let mut items = Vec::new();
                if *worktrees {
                    items.push(item("New worktree…", Action::NewSpaceWorktree));
                    items.push(item("Add existing worktree…", Action::AddExistingWorktree));
                }
                if *editable && !*built_in {
                    items.push(item("Rename", Action::RenameSpace));
                }
                if let Some(enabled) = agent_context {
                    items.push(item(
                        if *enabled {
                            "[x] Enable agent context"
                        } else {
                            "[ ] Enable agent context"
                        },
                        Action::ToggleSpaceAgentContext,
                    ));
                }
                items.push(item(
                    if *collapsed { "Expand" } else { "Collapse" },
                    Action::ToggleSpace,
                ));
                if *editable && !*built_in {
                    items.push(item("Delete space", Action::DeleteSpace));
                }
                items
            }
            ClientContextMenuTarget::AddWorktree { .. } => vec![
                item("New worktree…", Action::NewSpaceWorktree),
                item("Add existing worktree…", Action::AddExistingWorktree),
            ],
            ClientContextMenuTarget::ClosedMember { .. } => vec![
                item("Open", Action::OpenClosedMember),
                item("Remove from space", Action::RemoveClosedMember),
            ],
            ClientContextMenuTarget::Workspace {
                is_linked_worktree: true,
                space_worktrees: true,
                ..
            } => vec![
                item("Rename", Action::Rename),
                item("Close", Action::Close),
                item("Delete worktree checkout...", Action::RemoveWorktree),
            ],
            ClientContextMenuTarget::Workspace { is_git: false, .. }
            | ClientContextMenuTarget::Workspace {
                space_worktrees: true,
                ..
            } => {
                vec![item("Rename", Action::Rename), item("Close", Action::Close)]
            }
            ClientContextMenuTarget::Workspace {
                is_linked_worktree: false,
                ..
            } => vec![
                item("Rename", Action::Rename),
                item("Close", Action::Close),
                item("New worktree", Action::NewWorktree),
                item("Open worktree...", Action::OpenWorktree),
            ],
            ClientContextMenuTarget::Workspace {
                is_linked_worktree: true,
                ..
            } => vec![
                item("Rename", Action::Rename),
                item("Close", Action::Close),
                item("Delete worktree checkout...", Action::RemoveWorktree),
            ],
            ClientContextMenuTarget::Tab { .. } => vec![
                item("New tab", Action::NewTab),
                item("Rename", Action::Rename),
                item("Close", Action::Close),
            ],
            ClientContextMenuTarget::Agent {
                agent_grid_excluded,
                ..
            } => match agent_grid_excluded {
                Some(true) => vec![item("Include in grid", Action::IncludeInAgentGrid)],
                Some(false) => vec![item("Exclude from grid", Action::ExcludeFromAgentGrid)],
                None => Vec::new(),
            },
            ClientContextMenuTarget::Pane {
                source_pane_id,
                has_manual_label,
                right_click_passthrough,
                agent_grid_tile,
                ..
            } => {
                let mut items = Vec::new();
                if *agent_grid_tile {
                    items.push(item("Exclude from grid", Action::ExcludeFromAgentGrid));
                }
                items.push(item("Rename pane", Action::RenamePane));
                if *has_manual_label {
                    items.push(item("Clear pane name", Action::ClearPaneName));
                }
                if source_pane_id.is_some() {
                    items.push(item("Swap with focused pane", Action::SwapWithFocusedPane));
                }
                items.extend([
                    item("Split right", Action::SplitRight),
                    item("Split down", Action::SplitDown),
                    item("Zoom", Action::Zoom),
                    item(
                        if *right_click_passthrough {
                            "Use Herdr right-click menu"
                        } else {
                            "Send right-clicks to pane"
                        },
                        Action::ToggleRightClickPassthrough,
                    ),
                    item("Close pane", Action::ClosePane),
                ]);
                items
            }
        }
    }
}

impl ClientShellState {
    pub(super) fn open_workspace_context_menu(&mut self, workspace_id: String, x: u16, y: u16) {
        let Some(snapshot) = self.snapshot.as_deref() else {
            return;
        };
        let Some(workspace) = snapshot
            .workspaces
            .iter()
            .find(|workspace| workspace.workspace_id == workspace_id)
        else {
            return;
        };
        let worktree = workspace.worktree.as_ref();
        let is_git = worktree.is_some() || workspace.branch.is_some();
        let is_linked_worktree = worktree.is_some_and(|worktree| worktree.is_linked_worktree);
        let pull_request = workspace.pull_request.is_some();
        let space_worktrees = self.endpoint_supports_space_worktrees();
        self.overlay = Some(ClientShellOverlay::ContextMenu(ClientContextMenuOverlay {
            target: ClientContextMenuTarget::Workspace {
                workspace_id,
                is_git,
                is_linked_worktree,
                space_worktrees,
                pull_request,
            },
            x,
            y,
            highlighted: 0,
        }));
    }

    pub(super) fn open_space_context_menu(&mut self, space_id: String, x: u16, y: u16) {
        let Some(space) = self.snapshot.as_deref().and_then(|snapshot| {
            snapshot
                .spaces
                .iter()
                .find(|space| space.space_id == space_id)
        }) else {
            return;
        };
        let built_in = space.built_in;
        let collapsed = self.group_is_collapsed(&self.active_endpoint_id, &space_id);
        let editable = self.active_endpoint_supports_spaces();
        let worktrees = editable && self.endpoint_supports_space_worktrees();
        // `other` groups nothing, so it has no agent context to offer.
        let agent_context = space
            .agent_context
            .filter(|_| !built_in && self.space_agent_context_supported());
        self.overlay = Some(ClientShellOverlay::ContextMenu(ClientContextMenuOverlay {
            target: ClientContextMenuTarget::Space {
                space_id,
                built_in,
                collapsed,
                editable,
                worktrees,
                agent_context,
            },
            x,
            y,
            highlighted: 0,
        }));
    }

    pub(super) fn open_add_worktree_menu(&mut self, space_id: String, x: u16, y: u16) {
        if !self.endpoint_supports_space_worktrees() {
            return;
        }
        self.overlay = Some(ClientShellOverlay::ContextMenu(ClientContextMenuOverlay {
            target: ClientContextMenuTarget::AddWorktree { space_id },
            x,
            y,
            highlighted: 0,
        }));
    }

    pub(super) fn open_closed_member_context_menu(
        &mut self,
        space_id: String,
        member_id: String,
        x: u16,
        y: u16,
    ) {
        if !self.active_endpoint_supports_spaces() {
            return;
        }
        self.overlay = Some(ClientShellOverlay::ContextMenu(ClientContextMenuOverlay {
            target: ClientContextMenuTarget::ClosedMember {
                space_id,
                member_id,
            },
            x,
            y,
            highlighted: 0,
        }));
    }

    /// Spaces are fork endpoint methods; hide their actions from endpoints
    /// that do not advertise them instead of surfacing rejections.
    pub(super) fn active_endpoint_supports_spaces(&self) -> bool {
        use crate::api::schema::{Method, SpaceMemberTarget, SpaceRenameParams};

        [
            Method::SpaceRename(SpaceRenameParams {
                space_id: String::new(),
                name: String::new(),
            }),
            Method::SpaceMemberOpen(SpaceMemberTarget {
                space_id: String::new(),
                member_id: String::new(),
                focus: true,
            }),
        ]
        .iter()
        .all(|method| self.supports_endpoint_method(method))
    }

    /// Ask for a new space's name; `workspace_id` is filed under it.
    pub(super) fn begin_new_space(&mut self, workspace_id: Option<String>) {
        self.overlay = Some(ClientShellOverlay::Rename(ClientRenameOverlay {
            title: "new space",
            input: TextEditor::new("", true),
            target: ClientRenameTarget::NewSpace { workspace_id },
        }));
    }

    pub(super) fn open_closed_member(
        &mut self,
        space_id: String,
        member_id: String,
        outcome: &mut ClientShellInput,
    ) {
        self.push_endpoint_method(
            crate::api::schema::Method::SpaceMemberOpen(crate::api::schema::SpaceMemberTarget {
                space_id,
                member_id,
                focus: true,
            }),
            outcome,
        );
    }

    pub(super) fn open_tab_context_menu(&mut self, tab_id: String, x: u16, y: u16) {
        let Some(tab) = self
            .snapshot
            .as_deref()
            .and_then(|snapshot| snapshot.tabs.iter().find(|tab| tab.tab_id == tab_id))
        else {
            return;
        };
        self.overlay = Some(ClientShellOverlay::ContextMenu(ClientContextMenuOverlay {
            target: ClientContextMenuTarget::Tab {
                tab_id,
                workspace_id: tab.workspace_id.clone(),
            },
            x,
            y,
            highlighted: 0,
        }));
    }

    pub(super) fn open_pane_context_menu(&mut self, pane_id: String, x: u16, y: u16) {
        let Some(snapshot) = self.snapshot.as_deref() else {
            return;
        };
        let Some(pane) = snapshot.panes.iter().find(|pane| pane.pane_id == pane_id) else {
            return;
        };
        let source_pane_id = snapshot
            .focused_pane_id
            .clone()
            .filter(|focused| focused != &pane_id);
        let agent_grid_tile = self.agent_grid_active() && self.agent_grid_exclusion_supported();
        let pull_request = workspace_pull_request_url(snapshot, &pane.workspace_id).is_some();
        self.overlay = Some(ClientShellOverlay::ContextMenu(ClientContextMenuOverlay {
            target: ClientContextMenuTarget::Pane {
                pane_id,
                workspace_id: pane.workspace_id.clone(),
                source_pane_id,
                has_manual_label: pane.label.is_some(),
                right_click_passthrough: pane.right_click_passthrough,
                agent_grid_tile,
                pull_request,
            },
            x,
            y,
            highlighted: 0,
        }));
    }

    /// Opens the menu for an agent row: leaving the agent out of the live
    /// agent grid, and opening its workspace's pull request. With neither,
    /// nothing opens.
    pub(super) fn open_agent_context_menu(&mut self, pane_id: String, x: u16, y: u16) -> bool {
        let exclusion = self.agent_grid_exclusion_supported();
        let Some(snapshot) = self.snapshot.as_deref() else {
            return false;
        };
        let Some(agent) = snapshot
            .agents
            .iter()
            .find(|agent| agent.pane_id == pane_id)
        else {
            return false;
        };
        let pull_request = workspace_pull_request_url(snapshot, &agent.workspace_id).is_some();
        if !exclusion && !pull_request {
            return false;
        }
        let agent_grid_excluded =
            exclusion.then(|| super::agent_grid::agent_grid_excludes(snapshot, &pane_id));
        self.overlay = Some(ClientShellOverlay::ContextMenu(ClientContextMenuOverlay {
            target: ClientContextMenuTarget::Agent {
                pane_id,
                agent_grid_excluded,
                pull_request,
            },
            x,
            y,
            highlighted: 0,
        }));
        true
    }

    pub(super) fn move_context_menu_selection(&mut self, delta: isize) {
        let Some(ClientShellOverlay::ContextMenu(menu)) = self.overlay.as_mut() else {
            return;
        };
        let item_count = menu.items().len();
        if item_count == 0 {
            return;
        }
        menu.highlighted = (menu.highlighted as isize + delta)
            .clamp(0, item_count.saturating_sub(1) as isize) as usize;
    }

    pub(super) fn activate_context_menu_item(
        &mut self,
        index: usize,
        outcome: &mut ClientShellInput,
    ) {
        let Some(ClientShellOverlay::ContextMenu(menu)) = self.overlay.take() else {
            return;
        };
        let Some(action) = menu.items().get(index).map(|item| item.action) else {
            outcome.repaint = true;
            return;
        };
        if action == ClientContextMenuAction::OpenPullRequest {
            self.open_target_pull_request(&menu.target, outcome);
            return;
        }
        match menu.target {
            ClientContextMenuTarget::Workspace { workspace_id, .. } => {
                self.activate_workspace_context_action(workspace_id, action, outcome)
            }
            ClientContextMenuTarget::Space { space_id, .. }
            | ClientContextMenuTarget::AddWorktree { space_id } => {
                self.activate_space_context_action(space_id, action, outcome)
            }
            ClientContextMenuTarget::ClosedMember {
                space_id,
                member_id,
            } => match action {
                ClientContextMenuAction::OpenClosedMember => {
                    self.open_closed_member(space_id, member_id, outcome)
                }
                ClientContextMenuAction::RemoveClosedMember => self.push_endpoint_method(
                    crate::api::schema::Method::SpaceMemberRemove(
                        crate::api::schema::SpaceMemberTarget {
                            space_id,
                            member_id,
                            focus: false,
                        },
                    ),
                    outcome,
                ),
                _ => {}
            },
            ClientContextMenuTarget::Tab {
                tab_id,
                workspace_id,
            } => self.activate_tab_context_action(tab_id, workspace_id, action, outcome),
            ClientContextMenuTarget::Agent { pane_id, .. } => match action {
                ClientContextMenuAction::ExcludeFromAgentGrid => {
                    self.set_agent_grid_excluded(pane_id, true, outcome)
                }
                ClientContextMenuAction::IncludeInAgentGrid => {
                    self.set_agent_grid_excluded(pane_id, false, outcome)
                }
                _ => {}
            },
            ClientContextMenuTarget::Pane {
                pane_id,
                workspace_id,
                source_pane_id,
                right_click_passthrough,
                ..
            } => self.activate_pane_context_action(
                pane_id,
                workspace_id,
                source_pane_id,
                right_click_passthrough,
                action,
                outcome,
            ),
        }
        outcome.repaint = true;
    }

    fn activate_workspace_context_action(
        &mut self,
        workspace_id: String,
        action: ClientContextMenuAction,
        outcome: &mut ClientShellInput,
    ) {
        use crate::input::KeybindAction;

        match action {
            ClientContextMenuAction::Rename => {
                let label = self
                    .snapshot
                    .as_deref()
                    .and_then(|snapshot| {
                        snapshot
                            .workspaces
                            .iter()
                            .find(|workspace| workspace.workspace_id == workspace_id)
                    })
                    .map(|workspace| workspace.label.clone());
                if let Some(label) = label {
                    self.overlay = Some(ClientShellOverlay::Rename(ClientRenameOverlay {
                        title: "rename workspace",
                        input: TextEditor::new(&label, false),
                        target: ClientRenameTarget::Workspace { workspace_id },
                    }));
                }
            }
            ClientContextMenuAction::Close => {
                if self.config.confirm_close {
                    self.open_confirm_close_overlay(workspace_id);
                } else {
                    self.push_endpoint_method(
                        crate::api::schema::Method::WorkspaceClose(
                            crate::api::schema::WorkspaceCloseParams {
                                workspace_id,
                                close_group: true,
                            },
                        ),
                        outcome,
                    );
                }
            }
            ClientContextMenuAction::NewWorktree => {
                self.begin_worktree_action_for(KeybindAction::NewWorktree, workspace_id, outcome)
            }
            ClientContextMenuAction::OpenWorktree => {
                self.begin_worktree_action_for(KeybindAction::OpenWorktree, workspace_id, outcome)
            }
            ClientContextMenuAction::RemoveWorktree => {
                self.begin_worktree_action_for(KeybindAction::RemoveWorktree, workspace_id, outcome)
            }
            _ => {}
        }
    }

    fn activate_space_context_action(
        &mut self,
        space_id: String,
        action: ClientContextMenuAction,
        outcome: &mut ClientShellInput,
    ) {
        match action {
            ClientContextMenuAction::NewSpaceWorktree => {
                self.open_space_worktree_dialog(&space_id);
            }
            ClientContextMenuAction::AddExistingWorktree => {
                self.open_existing_worktree_picker(&space_id, outcome);
            }
            ClientContextMenuAction::RenameSpace => {
                let name = self.snapshot.as_deref().and_then(|snapshot| {
                    snapshot
                        .spaces
                        .iter()
                        .find(|space| space.space_id == space_id)
                        .map(|space| space.name.clone())
                });
                if let Some(name) = name {
                    self.overlay = Some(ClientShellOverlay::Rename(ClientRenameOverlay {
                        title: "rename space",
                        input: TextEditor::new(&name, false),
                        target: ClientRenameTarget::Space { space_id },
                    }));
                }
            }
            ClientContextMenuAction::ToggleSpace => {
                let endpoint_id = self.active_endpoint_id.clone();
                self.toggle_collapsed_group(&endpoint_id, space_id);
                self.persist_chrome_preferences(outcome);
            }
            ClientContextMenuAction::DeleteSpace => self.push_endpoint_method(
                crate::api::schema::Method::SpaceDelete(crate::api::schema::SpaceTarget {
                    space_id,
                }),
                outcome,
            ),
            ClientContextMenuAction::ToggleSpaceAgentContext => {
                let enabled = self
                    .snapshot
                    .as_deref()
                    .and_then(|snapshot| {
                        snapshot
                            .spaces
                            .iter()
                            .find(|space| space.space_id == space_id)
                    })
                    .and_then(|space| space.agent_context);
                if let Some(enabled) = enabled {
                    self.push_endpoint_method(
                        crate::api::schema::Method::SpaceAgentContextSet(
                            crate::api::schema::SpaceAgentContextSetParams {
                                space_id,
                                enabled: !enabled,
                            },
                        ),
                        outcome,
                    );
                }
            }
            _ => {}
        }
    }

    pub(super) fn space_agent_context_supported(&self) -> bool {
        self.supports_endpoint_method(&crate::api::schema::Method::SpaceAgentContextSet(
            crate::api::schema::SpaceAgentContextSetParams {
                space_id: String::new(),
                enabled: true,
            },
        ))
    }

    fn activate_tab_context_action(
        &mut self,
        tab_id: String,
        workspace_id: String,
        action: ClientContextMenuAction,
        outcome: &mut ClientShellInput,
    ) {
        use crate::api::schema::{Method, TabTarget};

        self.push_endpoint_method(
            Method::TabFocus(TabTarget {
                tab_id: tab_id.clone(),
            }),
            outcome,
        );
        match action {
            ClientContextMenuAction::NewTab => {
                if self.config.prompt_new_tab_name {
                    let default_name = (self
                        .snapshot
                        .as_deref()
                        .map(|snapshot| {
                            snapshot
                                .tabs
                                .iter()
                                .filter(|tab| tab.workspace_id == workspace_id)
                                .count()
                        })
                        .unwrap_or(0)
                        + 1)
                    .to_string();
                    self.overlay = Some(ClientShellOverlay::Rename(ClientRenameOverlay {
                        title: "new tab",
                        input: TextEditor::new(&default_name, true),
                        target: ClientRenameTarget::NewTab {
                            workspace_id,
                            default_name,
                        },
                    }));
                } else {
                    self.push_endpoint_method(
                        Method::TabCreate(crate::api::schema::TabCreateParams {
                            workspace_id: Some(workspace_id),
                            cwd: None,
                            focus: true,
                            label: None,
                            env: Default::default(),
                        }),
                        outcome,
                    );
                }
            }
            ClientContextMenuAction::Rename => {
                let tab = self
                    .snapshot
                    .as_deref()
                    .and_then(|snapshot| snapshot.tabs.iter().find(|tab| tab.tab_id == tab_id));
                if let Some(tab) = tab {
                    self.overlay = Some(ClientShellOverlay::Rename(ClientRenameOverlay {
                        title: "rename tab",
                        input: TextEditor::new(&tab.label, false),
                        target: ClientRenameTarget::Tab {
                            tab_id,
                            auto_name: !tab.custom_label,
                            original_name: tab.label.clone(),
                        },
                    }));
                }
            }
            ClientContextMenuAction::Close => {
                self.request_tab_close(tab_id, outcome);
            }
            _ => {}
        }
    }

    fn activate_pane_context_action(
        &mut self,
        pane_id: String,
        workspace_id: String,
        source_pane_id: Option<String>,
        right_click_passthrough: bool,
        action: ClientContextMenuAction,
        outcome: &mut ClientShellInput,
    ) {
        use crate::api::schema::{
            Method, PaneInputSetParams, PaneRenameParams, PaneRightClickTarget, PaneSplitParams,
            PaneSwapParams, PaneTarget, PaneZoomMode, PaneZoomParams, SplitDirection,
        };

        match action {
            ClientContextMenuAction::RenamePane => {
                let label = self.snapshot.as_deref().and_then(|snapshot| {
                    snapshot
                        .panes
                        .iter()
                        .find(|pane| pane.pane_id == pane_id)
                        .and_then(|pane| pane.label.clone())
                });
                self.overlay = Some(ClientShellOverlay::Rename(ClientRenameOverlay {
                    title: "rename pane",
                    input: TextEditor::new(label.as_deref().unwrap_or_default(), label.is_none()),
                    target: ClientRenameTarget::Pane { pane_id },
                }));
            }
            ClientContextMenuAction::ClearPaneName => self.push_endpoint_method(
                Method::PaneRename(PaneRenameParams {
                    pane_id,
                    label: None,
                }),
                outcome,
            ),
            ClientContextMenuAction::SwapWithFocusedPane => {
                if let Some(source_pane_id) = source_pane_id {
                    self.push_endpoint_method(
                        Method::PaneSwap(PaneSwapParams {
                            pane_id: None,
                            direction: None,
                            source_pane_id: Some(source_pane_id.clone()),
                            target_pane_id: Some(pane_id),
                        }),
                        outcome,
                    );
                    self.push_endpoint_method(
                        Method::PaneFocus(PaneTarget {
                            pane_id: source_pane_id,
                        }),
                        outcome,
                    );
                }
            }
            ClientContextMenuAction::SplitRight | ClientContextMenuAction::SplitDown => {
                self.push_endpoint_method(
                    Method::PaneSplit(PaneSplitParams {
                        workspace_id: Some(workspace_id),
                        target_pane_id: Some(pane_id),
                        direction: if action == ClientContextMenuAction::SplitRight {
                            SplitDirection::Right
                        } else {
                            SplitDirection::Down
                        },
                        ratio: None,
                        cwd: None,
                        focus: true,
                        right_click: Default::default(),
                        env: Default::default(),
                    }),
                    outcome,
                );
            }
            ClientContextMenuAction::Zoom => self.push_endpoint_method(
                Method::PaneZoom(PaneZoomParams {
                    pane_id: Some(pane_id),
                    mode: PaneZoomMode::Toggle,
                }),
                outcome,
            ),
            ClientContextMenuAction::ToggleRightClickPassthrough => self.push_endpoint_method(
                Method::PaneInputSet(PaneInputSetParams {
                    pane_id,
                    right_click: if right_click_passthrough {
                        PaneRightClickTarget::Herdr
                    } else {
                        PaneRightClickTarget::Pane
                    },
                }),
                outcome,
            ),
            ClientContextMenuAction::ClosePane => {
                self.push_endpoint_method(Method::PaneClose(PaneTarget { pane_id }), outcome)
            }
            ClientContextMenuAction::ExcludeFromAgentGrid => {
                self.set_agent_grid_excluded(pane_id, true, outcome)
            }
            _ => {}
        }
    }
}

/// The pull request URL of a workspace, when the endpoint found one.
pub(super) fn workspace_pull_request_url<'a>(
    snapshot: &'a ClientShellSnapshot,
    workspace_id: &str,
) -> Option<&'a str> {
    snapshot
        .workspaces
        .iter()
        .find(|workspace| workspace.workspace_id == workspace_id)
        .and_then(|workspace| workspace.pull_request.as_ref())
        .map(|pull_request| pull_request.url.as_str())
}

impl ClientShellState {
    /// Opens the pull request of the menu target's workspace in the local
    /// browser, so it works the same for a remote endpoint.
    fn open_target_pull_request(
        &mut self,
        target: &ClientContextMenuTarget,
        outcome: &mut ClientShellInput,
    ) {
        let Some(snapshot) = self.snapshot.as_deref() else {
            return;
        };
        let workspace_id = match target {
            ClientContextMenuTarget::Workspace { workspace_id, .. }
            | ClientContextMenuTarget::Pane { workspace_id, .. } => Some(workspace_id.as_str()),
            ClientContextMenuTarget::Agent { pane_id, .. } => snapshot
                .agents
                .iter()
                .find(|agent| &agent.pane_id == pane_id)
                .map(|agent| agent.workspace_id.as_str()),
            _ => None,
        };
        if let Some(url) =
            workspace_id.and_then(|workspace_id| workspace_pull_request_url(snapshot, workspace_id))
        {
            outcome
                .actions
                .push(ClientShellAction::OpenSafeWebUrl(url.to_owned()));
        }
    }
}
