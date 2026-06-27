use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct TaskRecord {
    pub task_id: String,
    pub container_id: String,
    pub job_id: Option<String>,
    pub title: String,
    pub goal: String,
    pub status: String,
    pub badges: crate::ContainerBadges,
    pub created_at_ms: u128,
    pub updated_at_ms: u128,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct TaskStreamRequest {
    pub goal: String,
    pub session_id: Option<String>,
    pub context_pack_id: Option<String>,
    pub source_guidance: Option<crate::SourceGuidance>,
    pub model_config: Option<crate::ModelConfig>,
    pub artifact_destination: Option<crate::ArtifactDestinationGuidance>,
    pub artifact_target: Option<crate::ArtifactTargetRequest>,
    pub auto_approve: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct TaskDetail {
    pub task: TaskRecord,
    pub messages: Vec<crate::ContainerMessage>,
    pub artifacts: Vec<crate::ArtifactRecord>,
    pub approvals: Vec<ApprovalRecord>,
    pub receipts: Vec<TaskReceiptRecord>,
    pub selected_output_dir: Option<String>,
    pub destination_fulfilled: Option<bool>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct TaskDraftArtifactRecord {
    pub draft_id: String,
    pub workspace_uid: String,
    pub container_id: String,
    pub task_id: String,
    pub approval_id: String,
    pub preview_ref: Option<String>,
    pub operation: Option<String>,
    pub status: String,
    pub content_format: String,
    pub content_text: String,
    pub created_at_ms: u128,
    pub updated_at_ms: u128,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct TaskUserInputRequest {
    pub input: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct ForceCloseRequest {
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct ForceCloseResult {
    pub action: String,
    pub status: String,
    pub messages: Vec<crate::ContainerMessage>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct TaskApprovalActionResult {
    pub action: String,
    pub task: TaskRecord,
    pub messages: Vec<crate::ContainerMessage>,
    pub status: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct ApprovalRecord {
    pub approval_id: String,
    pub task_id: String,
    pub operation: Option<String>,
    pub preview_ref: Option<String>,
    pub status: String,
    pub preview: Value,
    pub draft_artifact: Option<TaskDraftArtifactRecord>,
    pub created_at_ms: u128,
    pub resolved_at_ms: Option<u128>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct TaskReceiptRecord {
    pub receipt_id: String,
    pub task_id: String,
    pub capability_id: Option<String>,
    pub status: String,
    pub kind: String,
    pub receipt_ref: Option<String>,
    pub artifact_paths: Vec<String>,
    pub summary: Option<String>,
    pub created_at_ms: u128,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct TaskStreamPayload {
    pub phase: Option<String>,
    pub message: Option<crate::ContainerMessage>,
    pub approval: Option<ApprovalRecord>,
    pub artifact: Option<crate::ArtifactRecord>,
    pub receipt: Option<Value>,
}
