use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct UiCapabilityManifest {
    pub commands: Vec<UiCommandDescriptor>,
    pub workspace_actions: Vec<UiActionDescriptor>,
    pub container_actions: Vec<UiActionDescriptor>,
    pub composer_tokens: Vec<ComposerTokenDescriptor>,
    pub model_config: crate::ModelConfigDescriptor,
    pub context_config: ContextConfigDescriptor,
    pub settings: Vec<UiActionDescriptor>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct UiCommandDescriptor {
    pub command_id: String,
    pub label: String,
    pub description: String,
    pub capability_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct UiActionDescriptor {
    pub action_id: String,
    pub label: String,
    pub capability_id: String,
    pub side_effect: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct ComposerTokenDescriptor {
    pub token: String,
    pub label: String,
    pub capability_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct ContextConfigDescriptor {
    pub supports_history_chat: bool,
    pub supports_history_task: bool,
    pub supports_container_default_pack: bool,
    pub supports_compaction: bool,
}
