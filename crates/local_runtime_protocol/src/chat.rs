use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct ChatThreadRecord {
    pub chat_thread_id: String,
    pub container_id: String,
    pub title: String,
    pub created_at_ms: u128,
    pub updated_at_ms: u128,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct CreateChatThreadRequest {
    pub title: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct ChatTurnStreamRequest {
    pub message: String,
    pub session_id: Option<String>,
    pub context_pack_id: Option<String>,
    pub context_pack: Option<crate::ContextPack>,
    pub source_guidance: Option<crate::SourceGuidance>,
    pub model_config: Option<crate::ModelConfig>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct ChatStreamPayload {
    pub delta: Option<String>,
    pub message: Option<crate::ContainerMessage>,
    pub tool_call: Option<Value>,
    pub suggested_task: Option<Value>,
}
