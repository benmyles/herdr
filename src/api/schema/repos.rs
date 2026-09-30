use serde::{Deserialize, Serialize};

use super::panes::PaneInfo;
use super::tabs::TabInfo;
use super::workspaces::WorkspaceInfo;
use super::worktrees::WorktreeInfo;

fn default_true() -> bool {
    true
}

/// A repo registered on this endpoint for creating worktrees from a space.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct RepoInfo {
    pub name: String,
    /// Main checkout as configured (may start with `~`).
    pub root: String,
    /// `root` resolved to an absolute path on the endpoint.
    pub root_path: String,
    pub base_branch: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote: Option<String>,
    /// Absent from servers without repo settings.
    #[serde(default, skip_serializing_if = "RepoSettings::is_empty")]
    pub settings: RepoSettings,
}

/// What Herdr does around each worktree it creates from a repo.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct RepoSettings {
    /// Prepended to a worktree's name to form its branch, e.g. `ben/`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub branch_prefix: String,
    /// Files copied from the main checkout into each new worktree, relative
    /// to the root. `*` and `?` match within one path component, e.g. `.env*`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub copy_files: Vec<String>,
    /// Shell command run in each new worktree after it is created.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub on_create: String,
    /// Shell command run in a worktree before Herdr removes it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub on_remove: String,
    /// Typed into a new worktree's first pane once `on_create` succeeds,
    /// e.g. `claude`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub start_command: String,
}

impl RepoSettings {
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }
}

/// Replaces every setting of a repo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct RepoSettingsSetParams {
    pub repo: String,
    pub settings: RepoSettings,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct RepoTarget {
    pub repo: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct RepoAddParams {
    /// Any path inside the repo's main checkout.
    pub root: String,
    /// Defaults to the repo directory name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Defaults to the remote's default branch, else the checked-out branch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_branch: Option<String>,
    /// Defaults to `origin` (or the only remote). Empty means no remote.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct RepoUpdateParams {
    pub repo: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_branch: Option<String>,
    /// Empty removes the remote.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SpaceWorktreeCreateParams {
    pub space_id: String,
    pub repo: String,
    /// Worktree name, used as the branch name as-is.
    pub name: String,
    /// Fetch the remote and fast-forward the repo's base branch first.
    #[serde(default = "default_true")]
    pub sync: bool,
    #[serde(default)]
    pub focus: bool,
}

/// Files an existing checkout (any Git work tree) under a space, opening a
/// workspace for it unless one is already open.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SpaceWorktreeOpenParams {
    pub space_id: String,
    /// Absolute path of the checkout (`~` allowed).
    pub path: String,
    #[serde(default)]
    pub focus: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WorktreeBranchSource {
    /// A new branch from `start_point`.
    New,
    /// An existing local branch.
    Local,
    /// A branch that existed only on the remote, now tracked.
    Remote,
    /// The checkout already existed and was reopened.
    Existing,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct WorktreeSyncInfo {
    pub fetched: bool,
    pub root_updated: bool,
    pub branch_source: WorktreeBranchSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_point: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SpaceWorktreeCreatedInfo {
    pub workspace: WorkspaceInfo,
    pub tab: TabInfo,
    pub root_pane: PaneInfo,
    pub worktree: WorktreeInfo,
    pub sync: WorktreeSyncInfo,
}
