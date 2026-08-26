use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use crate::workspace::{Workspace, WorktreeSpaceMembership};

static NEXT_SPACE_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PinnedSpaceKey {
    Workspace { workspace_id: String },
    Worktree { key: String },
}

impl PinnedSpaceKey {
    pub fn from_workspace(workspace: &Workspace) -> Self {
        workspace
            .worktree_space()
            .map(|space| Self::Worktree {
                key: space.key.clone(),
            })
            .unwrap_or_else(|| Self::Workspace {
                workspace_id: workspace.id.clone(),
            })
    }

    pub fn matches_workspace(&self, workspace: &Workspace) -> bool {
        match self {
            Self::Workspace { workspace_id } => workspace.id == *workspace_id,
            Self::Worktree { key } => workspace
                .worktree_space()
                .is_some_and(|space| space.key == *key),
        }
    }
}

/// Durable identity and launch metadata for a space that may have no live PTY.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PinnedSpace {
    pub id: String,
    pub key: PinnedSpaceKey,
    pub label: String,
    pub cwd: PathBuf,
    #[serde(default)]
    pub order: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_space: Option<WorktreeSpaceMembership>,
}

impl PinnedSpace {
    pub fn new(
        key: PinnedSpaceKey,
        label: String,
        cwd: PathBuf,
        order: usize,
        worktree_space: Option<WorktreeSpaceMembership>,
    ) -> Self {
        let sequence = NEXT_SPACE_ID.fetch_add(1, Ordering::Relaxed);
        let created_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |duration| duration.as_nanos());
        Self {
            id: format!("space_{created_at}_{sequence}"),
            key,
            label,
            cwd,
            order,
            worktree_space,
        }
    }

    pub fn matches_workspace(&self, workspace: &Workspace) -> bool {
        self.key.matches_workspace(workspace)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_and_worktree_keys_match_only_their_logical_space() {
        let ordinary = Workspace::test_new("ordinary");
        let ordinary_key = PinnedSpaceKey::from_workspace(&ordinary);
        assert!(ordinary_key.matches_workspace(&ordinary));
        assert!(!ordinary_key.matches_workspace(&Workspace::test_new("other")));

        let mut parent = Workspace::test_new("parent");
        parent.worktree_space = Some(WorktreeSpaceMembership {
            key: "repo-key".into(),
            label: "repo".into(),
            repo_root: "/repo".into(),
            checkout_path: "/repo".into(),
            is_linked_worktree: false,
        });
        let mut child = Workspace::test_new("child");
        child.worktree_space = Some(WorktreeSpaceMembership {
            checkout_path: "/repo/child".into(),
            is_linked_worktree: true,
            ..parent.worktree_space.clone().expect("membership")
        });
        let group_key = PinnedSpaceKey::from_workspace(&child);
        assert!(group_key.matches_workspace(&parent));
        assert!(group_key.matches_workspace(&child));
    }
}
