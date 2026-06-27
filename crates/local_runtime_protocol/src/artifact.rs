use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct ArtifactRecord {
    pub artifact_id: String,
    pub container_id: String,
    pub task_id: Option<String>,
    pub title: String,
    pub artifact_type: String,
    pub path: Option<String>,
    pub status: String,
    pub capability_id: Option<String>,
    pub receipt_ref: Option<String>,
    pub verified: bool,
    pub kind: Option<String>,
    pub created_at_ms: u128,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct ArtifactTargetRequest {
    pub container_id: String,
    pub artifact_type: String,
    pub target_dir: Option<String>,
    pub save_strategy: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct ArtifactTargetOption {
    pub target_id: String,
    pub label: String,
    pub target_dir: String,
    pub artifact_types: Vec<String>,
    pub save_strategies: Vec<String>,
    pub user_visible: bool,
}
