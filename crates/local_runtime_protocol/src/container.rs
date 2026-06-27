use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContainerStatus {
    Active,
    Running,
    Approval,
    Blocked,
    Archived,
    Deleted,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct ContainerRecord {
    pub container_id: String,
    pub workspace_uid: String,
    pub title: String,
    pub status: ContainerStatus,
    pub badges: ContainerBadges,
    pub created_at_ms: u128,
    pub updated_at_ms: u128,
    pub last_active_at_ms: u128,
    pub default_model_config: Option<Value>,
    pub context_policy: Option<Value>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct ContainerBadges {
    pub running: u32,
    pub approval: u32,
    pub blocked: u32,
    pub unread: u32,
    pub artifact_ready: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct CreateContainerRequest {
    pub workspace_uid: Option<String>,
    pub title: Option<String>,
    pub model_config: Option<Value>,
    pub context_policy: Option<Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct UpdateContainerRequest {
    pub title: Option<String>,
    pub status: Option<ContainerStatus>,
    pub model_config: Option<Value>,
    pub context_policy: Option<Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct ContainerSnapshot {
    pub container: ContainerRecord,
    pub messages: Vec<ContainerMessage>,
    pub chat_threads: Vec<crate::ChatThreadRecord>,
    pub tasks: Vec<crate::TaskRecord>,
    pub context_pack: Option<crate::ContextPack>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct ContainerMessage {
    pub message_id: String,
    pub workspace_uid: String,
    pub container_id: String,
    pub lane: MessageLane,
    pub role: MessageRole,
    pub message_type: MessageType,
    pub status: String,
    pub title: Option<String>,
    pub body_text: Option<String>,
    pub body_json: Value,
    pub card_json: Value,
    pub chat_thread_id: Option<String>,
    pub task_id: Option<String>,
    pub job_id: Option<String>,
    pub source_kind: String,
    pub source_ref: String,
    pub source_seq: Option<i64>,
    pub created_at_ms: u128,
    pub updated_at_ms: u128,
    pub sort_key: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MessageLane {
    Chat,
    Task,
    Runtime,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    User,
    Assistant,
    Agent,
    Tool,
    System,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MessageType {
    Text,
    Reasoning,
    ToolCall,
    ToolResult,
    Approval,
    Artifact,
    Phase,
    Error,
}
