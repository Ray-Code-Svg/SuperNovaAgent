use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct WorkspaceRecord {
    pub workspace_uid: String,
    pub workspace_root: String,
    pub display_name: String,
    pub created_at_ms: u128,
    pub last_opened_at_ms: u128,
    pub archived: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct CreateWorkspaceRequest {
    pub workspace_root: String,
    pub display_name: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct ActivateWorkspaceRequest {
    pub workspace_uid: Option<String>,
    pub workspace_root: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct WorkspaceActivation {
    pub workspace: WorkspaceRecord,
    pub recent_active_container_id: Option<String>,
}
