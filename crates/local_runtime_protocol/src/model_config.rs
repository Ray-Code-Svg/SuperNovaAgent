use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct ModelConfig {
    pub provider: String,
    pub model: String,
    pub thinking: String,
    pub reasoning_effort: String,
    pub token_budget: Option<u64>,
    pub strict_tools: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct ModelConfigDescriptor {
    pub active: ModelConfig,
    pub providers: Vec<ModelProviderDescriptor>,
    pub thinking_options: Vec<ModelConfigOption>,
    pub reasoning_effort_options: Vec<ModelConfigOption>,
    pub token_budget_min: u64,
    pub token_budget_max: u64,
    pub token_budget_default: u64,
    pub strict_tools_label: String,
    pub strict_tools_description: String,
    pub advanced_defaults_collapsed: bool,
    pub user_summary: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct ModelProviderDescriptor {
    pub provider: String,
    pub display_name: String,
    pub models: Vec<String>,
    pub model_options: Vec<ModelConfigOption>,
    pub supports_thinking: bool,
    pub supports_strict_tools: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct ModelConfigOption {
    pub value: String,
    pub label: String,
    pub description: String,
}
