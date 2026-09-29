use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use crate::workspace::WorktreeSpaceMembership;

/// Built-in space holding every workspace the user has not organized. It is
/// always last, cannot be renamed or deleted, and is hidden while empty.
pub const OTHER_SPACE_ID: &str = "other";
pub const OTHER_SPACE_NAME: &str = "other";

static NEXT_SPACE_SEQUENCE: AtomicU64 = AtomicU64::new(1);

fn generate_id(prefix: &str) -> String {
    let sequence = NEXT_SPACE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let created_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    format!("{prefix}_{created_at}_{sequence}")
}

/// A user-named container for workspaces, usually the worktrees of one
/// feature across several repositories. Live members are the workspaces whose
/// `space_id` names this space; `closed` keeps members whose terminals were
/// closed so they can be reopened in place.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Space {
    pub id: String,
    pub name: String,
    /// Color slot. It stays with the space when spaces are reordered.
    #[serde(default)]
    pub color: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub closed: Vec<ClosedMember>,
}

impl Space {
    pub fn new(name: String, color: usize) -> Self {
        Self {
            id: generate_id("space"),
            name,
            color,
            closed: Vec::new(),
        }
    }

    pub fn other(color: usize) -> Self {
        Self {
            id: OTHER_SPACE_ID.to_owned(),
            name: OTHER_SPACE_NAME.to_owned(),
            color,
            closed: Vec::new(),
        }
    }

    pub fn is_other(&self) -> bool {
        self.id == OTHER_SPACE_ID
    }
}

/// A space member with no live workspace. Reopening creates a workspace at
/// `cwd` in the same space.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClosedMember {
    pub id: String,
    pub label: String,
    pub cwd: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// Herdr-managed worktree provenance, kept so a reopened checkout can
    /// still be deleted through the worktree flow.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_space: Option<WorktreeSpaceMembership>,
}

impl ClosedMember {
    pub fn new(
        label: String,
        cwd: PathBuf,
        custom_name: Option<String>,
        branch: Option<String>,
        worktree_space: Option<WorktreeSpaceMembership>,
    ) -> Self {
        Self {
            id: generate_id("member"),
            label,
            cwd,
            custom_name,
            branch,
            worktree_space,
        }
    }
}

/// The smallest color slot no space uses, so new spaces avoid their
/// neighbors' colors until every slot is taken.
pub fn next_color_slot(spaces: &[Space]) -> usize {
    (0..)
        .find(|slot| spaces.iter().all(|space| space.color != *slot))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_prefixed() {
        let first = Space::new("a".into(), 0);
        let second = Space::new("a".into(), 0);
        assert_ne!(first.id, second.id);
        assert!(first.id.starts_with("space_"));
        let member = ClosedMember::new("repo".into(), "/repo".into(), None, None, None);
        assert!(member.id.starts_with("member_"));
    }

    #[test]
    fn next_color_slot_fills_gaps_first() {
        let spaces = vec![
            Space::new("a".into(), 0),
            Space::new("b".into(), 2),
            Space::other(1),
        ];
        assert_eq!(next_color_slot(&spaces), 3);
        let spaces = vec![Space::new("a".into(), 0), Space::new("b".into(), 2)];
        assert_eq!(next_color_slot(&spaces), 1);
        assert_eq!(next_color_slot(&[]), 0);
    }
}
