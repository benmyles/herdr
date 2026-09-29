//! New worktree for a space, and the endpoint's repo list it draws from.

use super::*;
use crossterm::event::{KeyCode, KeyModifiers};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SpaceWorktreePreview {
    pub(super) branch: String,
    pub(super) checkout: String,
    pub(super) valid: bool,
}

/// The name a new worktree starts with: the space name, with whitespace
/// turned into `-` so it is a usable branch name.
pub(super) fn default_worktree_name(space_name: &str) -> String {
    space_name.split_whitespace().collect::<Vec<_>>().join("-")
}

pub(super) fn space_worktree_preview(
    template: &str,
    space_name: &str,
    repo: &crate::protocol::ClientShellRepo,
    name: &str,
    sync: bool,
) -> SpaceWorktreePreview {
    if name.is_empty() {
        return SpaceWorktreePreview {
            branch: "enter a name".to_owned(),
            checkout: "—".to_owned(),
            valid: false,
        };
    }
    if crate::repos::validated_branch(name, "branch name").is_err() {
        return SpaceWorktreePreview {
            branch: "not a valid branch name (no spaces or ~^:?*[\\)".to_owned(),
            checkout: "—".to_owned(),
            valid: false,
        };
    }
    let start = match (sync, repo.remote.as_deref()) {
        (true, Some(remote)) => format!("{remote}/{}", repo.base_branch),
        _ => repo.base_branch.clone(),
    };
    SpaceWorktreePreview {
        branch: format!("{name}  (new from {start} unless it already exists)"),
        checkout: if template.is_empty() {
            "chosen by the server".to_owned()
        } else {
            crate::worktree::expand_space_path_template(template, space_name, &repo.name, name)
        },
        valid: true,
    }
}

pub(super) fn selected_repo_index(
    dialog: &ClientSpaceWorktreeOverlay,
    repos: &[crate::protocol::ClientShellRepo],
) -> usize {
    dialog
        .selected_repo
        .as_deref()
        .and_then(|name| repos.iter().position(|repo| repo.name == name))
        .unwrap_or(0)
}

fn optional(text: &str) -> Option<String> {
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

impl ClientShellState {
    fn endpoint_repos(&self) -> &[crate::protocol::ClientShellRepo] {
        self.snapshot
            .as_deref()
            .map(|snapshot| snapshot.repos.as_slice())
            .unwrap_or_default()
    }

    pub(super) fn endpoint_supports_space_worktrees(&self) -> bool {
        use crate::api::schema::{Method, RepoAddParams, SpaceWorktreeCreateParams};

        // Servers with space worktrees always send their path template.
        let has_template = self
            .snapshot
            .as_deref()
            .is_some_and(|snapshot| !snapshot.worktree_path_template.is_empty());
        has_template
            && [
                Method::SpaceWorktreeCreate(SpaceWorktreeCreateParams {
                    space_id: String::new(),
                    repo: String::new(),
                    name: String::new(),
                    sync: true,
                    focus: false,
                }),
                Method::RepoAdd(RepoAddParams::default()),
            ]
            .iter()
            .all(|method| self.supports_endpoint_method(method))
    }

    pub(super) fn open_space_worktree_dialog(&mut self, space_id: &str) {
        let Some(space_name) = self.snapshot.as_deref().and_then(|snapshot| {
            snapshot
                .spaces
                .iter()
                .find(|space| space.space_id == space_id)
                .map(|space| space.name.clone())
        }) else {
            return;
        };
        self.open_space_worktree_dialog_named(space_id.to_owned(), space_name, None);
    }

    /// `space_name` is passed in because a just-created space may not be in
    /// the snapshot yet.
    pub(super) fn open_space_worktree_dialog_named(
        &mut self,
        space_id: String,
        space_name: String,
        selected_repo: Option<String>,
    ) {
        let built_in = self
            .snapshot
            .as_deref()
            .and_then(|snapshot| {
                snapshot
                    .spaces
                    .iter()
                    .find(|space| space.space_id == space_id)
            })
            .is_some_and(|space| space.built_in);
        let selected_repo = selected_repo.or_else(|| self.default_repo_for_space(&space_id));
        self.overlay = Some(ClientShellOverlay::SpaceWorktree(
            ClientSpaceWorktreeOverlay {
                name: TextEditor::new(
                    &if built_in {
                        String::new()
                    } else {
                        default_worktree_name(&space_name)
                    },
                    true,
                ),
                space_id,
                space_name,
                selected_repo,
                sync: true,
                field: SpaceWorktreeField::Name,
                error: None,
                offer_without_sync: false,
                creating: false,
            },
        ));
    }

    /// The first repo that has no checkout in the space yet.
    fn default_repo_for_space(&self, space_id: &str) -> Option<String> {
        let snapshot = self.snapshot.as_deref()?;
        let used = snapshot
            .workspaces
            .iter()
            .filter(|workspace| workspace.space_id.as_deref() == Some(space_id))
            .filter_map(|workspace| workspace.worktree.as_ref())
            .map(|worktree| worktree.label.as_str())
            .collect::<Vec<_>>();
        snapshot
            .repos
            .iter()
            .find(|repo| !used.contains(&repo.name.as_str()))
            .or_else(|| snapshot.repos.first())
            .map(|repo| repo.name.clone())
    }

    pub(super) fn move_space_worktree_repo(&mut self, delta: isize) {
        let names = self
            .endpoint_repos()
            .iter()
            .map(|repo| repo.name.clone())
            .collect::<Vec<_>>();
        let Some(ClientShellOverlay::SpaceWorktree(dialog)) = self.overlay.as_mut() else {
            return;
        };
        if names.is_empty() || dialog.creating {
            return;
        }
        let current = dialog
            .selected_repo
            .as_deref()
            .and_then(|name| names.iter().position(|candidate| candidate == name))
            .unwrap_or(0);
        let next = (current as isize + delta).clamp(0, names.len() as isize - 1) as usize;
        dialog.selected_repo = Some(names[next].clone());
        dialog.error = None;
        dialog.offer_without_sync = false;
    }

    fn select_space_worktree_repo(&mut self, index: usize) {
        let name = self
            .endpoint_repos()
            .get(index)
            .map(|repo| repo.name.clone());
        if let (Some(name), Some(ClientShellOverlay::SpaceWorktree(dialog))) =
            (name, self.overlay.as_mut())
        {
            if !dialog.creating {
                dialog.selected_repo = Some(name);
                dialog.error = None;
                dialog.offer_without_sync = false;
            }
        }
    }

    pub(super) fn submit_space_worktree(&mut self, outcome: &mut ClientShellInput) {
        outcome.repaint = true;
        let Some(ClientShellOverlay::SpaceWorktree(dialog)) = self.overlay.as_ref() else {
            return;
        };
        if dialog.creating {
            return;
        }
        let repos = self.endpoint_repos();
        let Some(repo) = repos.get(selected_repo_index(dialog, repos)).cloned() else {
            let space_id = dialog.space_id.clone();
            self.open_repo_editor(None, ClientRepoEditReturn::SpaceWorktree { space_id });
            return;
        };
        let Some(ClientShellOverlay::SpaceWorktree(dialog)) = self.overlay.as_mut() else {
            return;
        };
        let name = dialog.name.trim().to_owned();
        if name.is_empty() {
            dialog.error = Some("name is required".to_owned());
            dialog.field = SpaceWorktreeField::Name;
            return;
        }
        if let Err(error) = crate::repos::validated_branch(&name, "branch name") {
            dialog.error = Some(error.message);
            dialog.field = SpaceWorktreeField::Name;
            return;
        }
        dialog.name.trim_and_accept();
        dialog.selected_repo = Some(repo.name.clone());
        dialog.creating = true;
        dialog.error = None;
        let method = crate::api::schema::Method::SpaceWorktreeCreate(
            crate::api::schema::SpaceWorktreeCreateParams {
                space_id: dialog.space_id.clone(),
                repo: repo.name,
                name,
                sync: dialog.sync && repo.remote.is_some(),
                focus: false,
            },
        );
        if !self.push_endpoint_method_with_kind(
            method,
            PendingEndpointKind::SpaceWorktreeCreate,
            outcome,
        ) {
            if let Some(ClientShellOverlay::SpaceWorktree(dialog)) = self.overlay.as_mut() {
                dialog.creating = false;
            }
        }
    }

    pub(super) fn open_repo_editor(
        &mut self,
        original_name: Option<&str>,
        return_to: ClientRepoEditReturn,
    ) {
        let repo = original_name.and_then(|name| {
            self.endpoint_repos()
                .iter()
                .find(|repo| repo.name == name)
                .cloned()
        });
        let fields = match repo.as_ref() {
            Some(repo) => [
                TextEditor::new(&repo.root, false),
                TextEditor::new(&repo.name, false),
                TextEditor::new(&repo.base_branch, false),
                TextEditor::new(repo.remote.as_deref().unwrap_or_default(), false),
            ],
            None => Default::default(),
        };
        self.overlay = Some(ClientShellOverlay::RepoEdit(ClientRepoEditOverlay {
            original_name: repo.map(|repo| repo.name),
            fields,
            field: 0,
            error: None,
            saving: false,
            return_to,
        }));
    }

    fn close_repo_editor(&mut self, saved_name: Option<String>, outcome: &mut ClientShellInput) {
        let Some(ClientShellOverlay::RepoEdit(edit)) = self.overlay.take() else {
            return;
        };
        match edit.return_to {
            ClientRepoEditReturn::Settings => {
                self.open_settings_overlay();
                self.select_settings_section(ClientSettingsSection::Repos, outcome);
                if let Some(index) = saved_name.and_then(|name| {
                    self.endpoint_repos()
                        .iter()
                        .position(|repo| repo.name == name)
                }) {
                    self.select_settings_choice(index);
                }
            }
            ClientRepoEditReturn::SpaceWorktree { space_id } => {
                let space_name = self.snapshot.as_deref().and_then(|snapshot| {
                    snapshot
                        .spaces
                        .iter()
                        .find(|space| space.space_id == space_id)
                        .map(|space| space.name.clone())
                });
                if let Some(space_name) = space_name {
                    self.open_space_worktree_dialog_named(space_id, space_name, saved_name);
                }
            }
        }
        outcome.repaint = true;
    }

    fn submit_repo_edit(&mut self, outcome: &mut ClientShellInput) {
        use crate::api::schema::{Method, RepoAddParams, RepoUpdateParams};

        outcome.repaint = true;
        let current = self.endpoint_repos().to_vec();
        let Some(ClientShellOverlay::RepoEdit(edit)) = self.overlay.as_mut() else {
            return;
        };
        if edit.saving {
            return;
        }
        let [root, name, base, remote] = &edit.fields;
        let method = match edit.original_name.as_deref() {
            None => {
                let Some(root) = optional(root) else {
                    edit.error = Some("root is required".to_owned());
                    edit.field = 0;
                    return;
                };
                Method::RepoAdd(RepoAddParams {
                    root,
                    name: optional(name),
                    base_branch: optional(base),
                    remote: optional(remote),
                })
            }
            Some(original) => {
                let Some(repo) = current.iter().find(|repo| repo.name == original) else {
                    edit.error = Some(format!("repo {original} no longer exists"));
                    return;
                };
                let changed = |value: &str, current: &str| {
                    (value.trim() != current).then(|| value.trim().to_owned())
                };
                Method::RepoUpdate(RepoUpdateParams {
                    repo: original.to_owned(),
                    name: changed(name, &repo.name).filter(|name| !name.is_empty()),
                    root: changed(root, &repo.root).filter(|root| !root.is_empty()),
                    base_branch: changed(base, &repo.base_branch).filter(|base| !base.is_empty()),
                    remote: changed(remote, repo.remote.as_deref().unwrap_or_default()),
                })
            }
        };
        edit.saving = true;
        edit.error = None;
        if !self.push_endpoint_method_with_kind(method, PendingEndpointKind::RepoSave, outcome) {
            if let Some(ClientShellOverlay::RepoEdit(edit)) = self.overlay.as_mut() {
                edit.saving = false;
            }
        }
    }

    pub(super) fn remove_selected_settings_repo(&mut self, outcome: &mut ClientShellInput) {
        let Some(ClientShellOverlay::Settings(settings)) = self.overlay.as_ref() else {
            return;
        };
        let Some(repo) = self.endpoint_repos().get(settings.selected) else {
            return;
        };
        let repo = repo.name.clone();
        self.push_endpoint_method_with_kind(
            crate::api::schema::Method::RepoRemove(crate::api::schema::RepoTarget { repo }),
            PendingEndpointKind::RepoRemove,
            outcome,
        );
        outcome.repaint = true;
    }

    pub(super) fn edit_selected_settings_repo(&mut self) {
        let Some(ClientShellOverlay::Settings(settings)) = self.overlay.as_ref() else {
            return;
        };
        let name = self
            .endpoint_repos()
            .get(settings.selected)
            .map(|repo| repo.name.clone());
        self.cancel_settings_overlay();
        self.open_repo_editor(name.as_deref(), ClientRepoEditReturn::Settings);
    }

    pub(super) fn insert_repo_overlay_text(&mut self, text: &str) -> bool {
        match self.overlay.as_mut() {
            Some(ClientShellOverlay::SpaceWorktree(dialog)) if !dialog.creating => {
                dialog.field = SpaceWorktreeField::Name;
                if dialog.name.insert(text) {
                    dialog.error = None;
                    dialog.offer_without_sync = false;
                }
                true
            }
            Some(ClientShellOverlay::RepoEdit(edit)) if !edit.saving => {
                edit.fields[edit.field].insert(text);
                true
            }
            _ => false,
        }
    }

    pub(super) fn route_repo_overlay_key(
        &mut self,
        key: &crate::input::TerminalKey,
        outcome: &mut ClientShellInput,
    ) -> bool {
        let (code, modifiers) = crate::config::normalize_key_combo((key.code, key.modifiers));
        let ctrl = modifiers == KeyModifiers::CONTROL;
        match self.overlay.as_ref() {
            Some(ClientShellOverlay::SpaceWorktree(dialog)) => {
                outcome.repaint = true;
                if dialog.creating {
                    return true;
                }
                let field = dialog.field;
                match code {
                    KeyCode::Esc => self.overlay = None,
                    KeyCode::Enter => self.submit_space_worktree(outcome),
                    KeyCode::Up => self.move_space_worktree_repo(-1),
                    KeyCode::Down => self.move_space_worktree_repo(1),
                    KeyCode::Char('p') if ctrl => self.move_space_worktree_repo(-1),
                    KeyCode::Char('n') if ctrl => self.move_space_worktree_repo(1),
                    _ => {
                        let Some(ClientShellOverlay::SpaceWorktree(dialog)) = self.overlay.as_mut()
                        else {
                            return true;
                        };
                        match code {
                            KeyCode::Tab | KeyCode::BackTab => {
                                dialog.field = match field {
                                    SpaceWorktreeField::Name => SpaceWorktreeField::Sync,
                                    SpaceWorktreeField::Sync => SpaceWorktreeField::Name,
                                };
                            }
                            KeyCode::Char(' ') if field == SpaceWorktreeField::Sync => {
                                dialog.sync = !dialog.sync;
                                dialog.offer_without_sync = false;
                                dialog.error = None;
                            }
                            // Typing on the checkbox goes to the name.
                            _ if dialog.name.handle_key(key) == Some(true) => {
                                dialog.field = SpaceWorktreeField::Name;
                                dialog.error = None;
                                dialog.offer_without_sync = false;
                            }
                            _ => {}
                        }
                    }
                }
                true
            }
            Some(ClientShellOverlay::RepoEdit(edit)) => {
                outcome.repaint = true;
                if edit.saving {
                    return true;
                }
                match code {
                    KeyCode::Esc => self.close_repo_editor(None, outcome),
                    KeyCode::Enter => self.submit_repo_edit(outcome),
                    _ => {
                        let Some(ClientShellOverlay::RepoEdit(edit)) = self.overlay.as_mut() else {
                            return true;
                        };
                        let count = edit.fields.len();
                        match code {
                            KeyCode::Tab | KeyCode::Down => edit.field = (edit.field + 1) % count,
                            KeyCode::BackTab | KeyCode::Up => {
                                edit.field = (edit.field + count - 1) % count;
                            }
                            _ => {
                                if edit.fields[edit.field].handle_key(key) == Some(true) {
                                    edit.error = None;
                                }
                            }
                        }
                    }
                }
                true
            }
            _ => false,
        }
    }

    pub(super) fn click_overlay_hit(
        &mut self,
        hit: ClientOverlayHit,
        outcome: &mut ClientShellInput,
    ) {
        outcome.repaint = true;
        match hit {
            ClientOverlayHit::SpaceWorktreeRepo(index) => self.select_space_worktree_repo(index),
            ClientOverlayHit::SpaceWorktreeName => {
                if let Some(ClientShellOverlay::SpaceWorktree(dialog)) = self.overlay.as_mut() {
                    dialog.field = SpaceWorktreeField::Name;
                }
            }
            ClientOverlayHit::SpaceWorktreeSync => {
                if let Some(ClientShellOverlay::SpaceWorktree(dialog)) = self.overlay.as_mut() {
                    if !dialog.creating {
                        dialog.field = SpaceWorktreeField::Sync;
                        dialog.sync = !dialog.sync;
                        dialog.offer_without_sync = false;
                        dialog.error = None;
                    }
                }
            }
            ClientOverlayHit::SpaceWorktreeAddRepo => {
                if let Some(ClientShellOverlay::SpaceWorktree(dialog)) = self.overlay.as_ref() {
                    if !dialog.creating {
                        let space_id = dialog.space_id.clone();
                        self.open_repo_editor(
                            None,
                            ClientRepoEditReturn::SpaceWorktree { space_id },
                        );
                    }
                }
            }
            ClientOverlayHit::ExistingWorktree(index) => {
                if let Some(ClientShellOverlay::SpaceWorktreeOpen(picker)) = self.overlay.as_mut() {
                    picker.selected = index;
                }
                self.submit_existing_worktree(outcome);
            }
            ClientOverlayHit::RepoEditField(index) => {
                if let Some(ClientShellOverlay::RepoEdit(edit)) = self.overlay.as_mut() {
                    edit.field = index.min(edit.fields.len() - 1);
                }
            }
            ClientOverlayHit::SettingsAddRepo => {
                self.cancel_settings_overlay();
                self.open_repo_editor(None, ClientRepoEditReturn::Settings);
            }
            ClientOverlayHit::SettingsRemoveRepo => self.remove_selected_settings_repo(outcome),
        }
    }

    /// Cancel/escape for the space worktree and repo overlays.
    pub(super) fn dismiss_repo_overlay(&mut self, outcome: &mut ClientShellInput) {
        match self.overlay.as_ref() {
            Some(ClientShellOverlay::SpaceWorktree(dialog)) if !dialog.creating => {
                self.overlay = None;
            }
            Some(ClientShellOverlay::RepoEdit(edit)) if !edit.saving => {
                self.close_repo_editor(None, outcome);
            }
            Some(ClientShellOverlay::SpaceWorktreeOpen(picker)) if !picker.opening => {
                self.overlay = None;
            }
            _ => {}
        }
        outcome.repaint = true;
    }

    pub(super) fn scroll_repo_overlay(&mut self, delta: isize) {
        match self.overlay.as_ref() {
            Some(ClientShellOverlay::SpaceWorktree(_)) => self.move_space_worktree_repo(delta),
            Some(ClientShellOverlay::SpaceWorktreeOpen(_)) => {
                self.move_existing_worktree_selection(delta)
            }
            _ => {}
        }
    }

    pub(super) fn submit_repo_overlay(&mut self, outcome: &mut ClientShellInput) {
        match self.overlay.as_ref() {
            Some(ClientShellOverlay::SpaceWorktree(_)) => self.submit_space_worktree(outcome),
            Some(ClientShellOverlay::RepoEdit(_)) => self.submit_repo_edit(outcome),
            Some(ClientShellOverlay::SpaceWorktreeOpen(_)) => {
                self.submit_existing_worktree(outcome)
            }
            _ => {}
        }
    }

    pub(super) fn handle_repo_endpoint_result(
        &mut self,
        kind: PendingEndpointKind,
        result: Result<crate::api::schema::ResponseResult, ClientShellEndpointError>,
        outcome: &mut ClientShellInput,
    ) -> bool {
        use crate::api::schema::ResponseResult;

        match (kind, result) {
            (PendingEndpointKind::SpaceCreate, Ok(ResponseResult::SpaceInfo { space })) => {
                if self.overlay.is_none() && self.endpoint_supports_space_worktrees() {
                    self.open_space_worktree_dialog_named(space.space_id, space.name, None);
                }
            }
            (
                PendingEndpointKind::SpaceWorktreeCreate,
                Ok(ResponseResult::SpaceWorktreeCreated(info)),
            ) => {
                if matches!(self.overlay, Some(ClientShellOverlay::SpaceWorktree(_))) {
                    self.overlay = None;
                }
                if !info.sync.warnings.is_empty() {
                    self.push_endpoint_notice(
                        ClientEndpointNoticeKind::Warning,
                        "space.worktree.create",
                        "Worktree created",
                        info.sync.warnings.join("\n"),
                    );
                }
                self.push_endpoint_method(
                    crate::api::schema::Method::TabFocus(crate::api::schema::TabTarget {
                        tab_id: info.tab.tab_id,
                    }),
                    outcome,
                );
            }
            (PendingEndpointKind::SpaceWorktreeCreate, Err(error)) => {
                let base = self.snapshot.as_deref().and_then(|snapshot| {
                    let Some(ClientShellOverlay::SpaceWorktree(dialog)) = self.overlay.as_ref()
                    else {
                        return None;
                    };
                    snapshot
                        .repos
                        .get(selected_repo_index(dialog, &snapshot.repos))
                        .map(|repo| repo.base_branch.clone())
                });
                if let Some(ClientShellOverlay::SpaceWorktree(dialog)) = self.overlay.as_mut() {
                    dialog.creating = false;
                    if error.code.as_deref() == Some("sync_fetch_failed") {
                        dialog.sync = false;
                        dialog.offer_without_sync = true;
                        dialog.error = Some(format!(
                            "{}. Sync is now off; ↵ creates the worktree from local {}.",
                            error.message.trim_end_matches('.'),
                            base.unwrap_or_else(|| "base".to_owned())
                        ));
                    } else {
                        dialog.error = Some(error.message);
                    }
                }
            }
            (PendingEndpointKind::RepoSave, Ok(ResponseResult::RepoInfo { repo })) => {
                if matches!(
                    self.overlay,
                    Some(ClientShellOverlay::RepoEdit(ClientRepoEditOverlay {
                        saving: true,
                        ..
                    }))
                ) {
                    self.close_repo_editor(Some(repo.name), outcome);
                }
            }
            (PendingEndpointKind::RepoSave, Err(error)) => {
                if let Some(ClientShellOverlay::RepoEdit(edit)) = self.overlay.as_mut() {
                    edit.saving = false;
                    edit.error = Some(error.message);
                }
            }
            (PendingEndpointKind::RepoRemove, Ok(ResponseResult::RepoList { repos })) => {
                if let Some(ClientShellOverlay::Settings(settings)) = self.overlay.as_mut() {
                    settings.selected = settings.selected.min(repos.len().saturating_sub(1));
                }
            }
            (
                PendingEndpointKind::SpaceCreate
                | PendingEndpointKind::SpaceWorktreeCreate
                | PendingEndpointKind::RepoSave
                | PendingEndpointKind::RepoRemove,
                Ok(_),
            ) => self.set_endpoint_error("endpoint returned an unexpected result"),
            (_, Err(_)) => {}
            (_, Ok(_)) => return false,
        }
        true
    }
}

impl ClientShellState {
    /// Lists every configured repo's checkouts, then lets the user file one
    /// under the space.
    pub(super) fn open_existing_worktree_picker(
        &mut self,
        space_id: &str,
        outcome: &mut ClientShellInput,
    ) {
        let Some(space_name) = self.snapshot.as_deref().and_then(|snapshot| {
            snapshot
                .spaces
                .iter()
                .find(|space| space.space_id == space_id)
                .map(|space| space.name.clone())
        }) else {
            return;
        };
        let repos = self.endpoint_repos().to_vec();
        if repos.is_empty() {
            self.open_repo_editor(
                None,
                ClientRepoEditReturn::SpaceWorktree {
                    space_id: space_id.to_owned(),
                },
            );
            outcome.repaint = true;
            return;
        }
        self.overlay = Some(ClientShellOverlay::SpaceWorktreeOpen(
            ClientSpaceWorktreeOpenOverlay {
                space_id: space_id.to_owned(),
                space_name,
                entries: Vec::new(),
                loading: 0,
                query: TextEditor::default(),
                selected: 0,
                error: None,
                opening: false,
            },
        ));
        for repo in repos {
            let queued = self.push_endpoint_method_with_kind(
                crate::api::schema::Method::WorktreeList(crate::api::schema::WorktreeListParams {
                    workspace_id: None,
                    cwd: Some(repo.root.clone()),
                    trust_repository: false,
                }),
                PendingEndpointKind::SpaceWorktreeList {
                    space_id: space_id.to_owned(),
                    repo: repo.name,
                },
                outcome,
            );
            if queued {
                if let Some(ClientShellOverlay::SpaceWorktreeOpen(picker)) = self.overlay.as_mut() {
                    picker.loading += 1;
                }
            }
        }
        outcome.repaint = true;
    }

    fn move_existing_worktree_selection(&mut self, delta: isize) {
        let Some(ClientShellOverlay::SpaceWorktreeOpen(picker)) = self.overlay.as_mut() else {
            return;
        };
        let filtered = picker.filtered_indices();
        if filtered.is_empty() {
            return;
        }
        let current = filtered
            .iter()
            .position(|index| *index == picker.selected)
            .unwrap_or(0);
        let next = (current as isize + delta).clamp(0, filtered.len() as isize - 1) as usize;
        picker.selected = filtered[next];
    }

    fn submit_existing_worktree(&mut self, outcome: &mut ClientShellInput) {
        outcome.repaint = true;
        let Some(ClientShellOverlay::SpaceWorktreeOpen(picker)) = self.overlay.as_mut() else {
            return;
        };
        if picker.opening {
            return;
        }
        let filtered = picker.filtered_indices();
        let index = if filtered.contains(&picker.selected) {
            picker.selected
        } else if let Some(first) = filtered.first() {
            *first
        } else {
            return;
        };
        let Some(entry) = picker.entries.get(index) else {
            return;
        };
        let method = crate::api::schema::Method::SpaceWorktreeOpen(
            crate::api::schema::SpaceWorktreeOpenParams {
                space_id: picker.space_id.clone(),
                path: entry.path.clone(),
                focus: true,
            },
        );
        picker.selected = index;
        picker.opening = true;
        picker.error = None;
        if !self.push_endpoint_method_with_kind(
            method,
            PendingEndpointKind::SpaceWorktreeOpen,
            outcome,
        ) {
            if let Some(ClientShellOverlay::SpaceWorktreeOpen(picker)) = self.overlay.as_mut() {
                picker.opening = false;
            }
        }
    }

    pub(super) fn route_existing_worktree_key(
        &mut self,
        key: &crate::input::TerminalKey,
        outcome: &mut ClientShellInput,
    ) -> bool {
        let Some(ClientShellOverlay::SpaceWorktreeOpen(picker)) = self.overlay.as_mut() else {
            return false;
        };
        outcome.repaint = true;
        if picker.opening {
            return true;
        }
        let (code, modifiers) = crate::config::normalize_key_combo((key.code, key.modifiers));
        let ctrl = modifiers == KeyModifiers::CONTROL;
        match code {
            KeyCode::Esc => self.overlay = None,
            KeyCode::Enter => self.submit_existing_worktree(outcome),
            KeyCode::Up => self.move_existing_worktree_selection(-1),
            KeyCode::Down => self.move_existing_worktree_selection(1),
            KeyCode::Char('p') if ctrl => self.move_existing_worktree_selection(-1),
            KeyCode::Char('n') if ctrl => self.move_existing_worktree_selection(1),
            _ => {
                if picker.query.handle_key(key) == Some(true) {
                    if let Some(first) = picker.filtered_indices().first().copied() {
                        picker.selected = first;
                    }
                }
            }
        }
        true
    }

    pub(super) fn insert_existing_worktree_text(&mut self, text: &str) -> bool {
        let Some(ClientShellOverlay::SpaceWorktreeOpen(picker)) = self.overlay.as_mut() else {
            return false;
        };
        if !picker.opening && picker.query.insert(text) {
            if let Some(first) = picker.filtered_indices().first().copied() {
                picker.selected = first;
            }
        }
        true
    }

    pub(super) fn handle_existing_worktree_result(
        &mut self,
        kind: PendingEndpointKind,
        result: Result<crate::api::schema::ResponseResult, ClientShellEndpointError>,
    ) -> bool {
        use crate::api::schema::ResponseResult;

        match (kind, result) {
            (PendingEndpointKind::SpaceWorktreeList { space_id, repo }, result) => {
                let Some(ClientShellOverlay::SpaceWorktreeOpen(picker)) = self.overlay.as_mut()
                else {
                    return false;
                };
                if picker.space_id != space_id {
                    return false;
                }
                picker.loading = picker.loading.saturating_sub(1);
                match result {
                    Ok(ResponseResult::WorktreeList { worktrees, .. }) => {
                        picker.entries.extend(
                            worktrees
                                .into_iter()
                                .filter(|entry| !entry.is_bare && !entry.is_prunable)
                                .map(|entry| ClientExistingWorktree {
                                    repo: repo.clone(),
                                    path: entry.path,
                                    branch: entry.branch,
                                    is_linked_worktree: entry.is_linked_worktree,
                                    open_workspace_id: entry.open_workspace_id,
                                }),
                        );
                    }
                    Ok(_) => picker.error = Some(format!("{repo}: unexpected result")),
                    Err(error) => picker.error = Some(format!("{repo}: {}", error.message)),
                }
                true
            }
            (PendingEndpointKind::SpaceWorktreeOpen, Ok(_)) => {
                if matches!(self.overlay, Some(ClientShellOverlay::SpaceWorktreeOpen(_))) {
                    self.overlay = None;
                }
                true
            }
            (PendingEndpointKind::SpaceWorktreeOpen, Err(error)) => {
                if let Some(ClientShellOverlay::SpaceWorktreeOpen(picker)) = self.overlay.as_mut() {
                    picker.opening = false;
                    picker.error = Some(error.message);
                }
                true
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo(remote: Option<&str>) -> crate::protocol::ClientShellRepo {
        crate::protocol::ClientShellRepo {
            name: "pyshiftup".into(),
            root: "~/code/pyshiftup".into(),
            base_branch: "main".into(),
            remote: remote.map(str::to_owned),
        }
    }

    #[test]
    fn preview_follows_the_endpoint_template_and_sync_choice() {
        let template = "/home/ben/worktrees/{space}/{repo}/{name}";
        let synced = space_worktree_preview(
            template,
            "knowledge",
            &repo(Some("origin")),
            "knowledge",
            true,
        );
        assert_eq!(
            synced.checkout,
            "/home/ben/worktrees/knowledge/pyshiftup/knowledge"
        );
        assert!(synced.branch.contains("new from origin/main"), "{synced:?}");

        let local =
            space_worktree_preview(template, "knowledge", &repo(Some("origin")), "x", false);
        assert!(local.branch.contains("new from main"), "{local:?}");

        let no_remote = space_worktree_preview(template, "knowledge", &repo(None), "x", true);
        assert!(no_remote.branch.contains("new from main"), "{no_remote:?}");

        let old_server = space_worktree_preview("", "knowledge", &repo(None), "x", true);
        assert_eq!(old_server.checkout, "chosen by the server");

        let invalid = space_worktree_preview(template, "k", &repo(None), "has space", true);
        assert!(!invalid.valid);
        assert_eq!(default_worktree_name(" billing  launch "), "billing-launch");
    }
}
