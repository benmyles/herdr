use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SpaceTarget {
    pub space_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SpaceCreateParams {
    pub name: String,
    /// Workspace to file under the new space right away.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SpaceRenameParams {
    pub space_id: String,
    pub name: String,
}

/// Turns on or off the context agents in a space's checkouts get about the
/// space's other checkouts. The built-in `other` space has none.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SpaceAgentContextSetParams {
    pub space_id: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SpaceMoveParams {
    pub space_id: String,
    /// Place before this space; omitted means last among user spaces.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before_space_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SpaceAssignParams {
    pub workspace_id: String,
    pub space_id: String,
    /// Place before this workspace of the target space; omitted means last.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before_workspace_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SpaceMemberTarget {
    pub space_id: String,
    pub member_id: String,
    #[serde(default)]
    pub focus: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ClosedSpaceMemberInfo {
    pub member_id: String,
    pub label: String,
    pub cwd: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SpaceInfo {
    pub space_id: String,
    pub name: String,
    /// Color slot, stable across reorders.
    pub color: usize,
    /// The built-in `other` space, which cannot be renamed, moved, or deleted.
    #[serde(default, skip_serializing_if = "super::is_false")]
    pub built_in: bool,
    /// Live workspaces in sidebar order.
    pub workspace_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub closed: Vec<ClosedSpaceMemberInfo>,
    /// Whether agents in this space are told about its other checkouts.
    /// Always false for the built-in `other` space.
    #[serde(default = "super::default_true")]
    pub agent_context: bool,
}
