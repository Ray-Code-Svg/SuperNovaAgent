use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct DiagnosticsSnapshot {
    pub runtime_status: String,
    pub protocol_version: String,
    pub runtime_layer: String,
    pub kernel_layer: String,
    pub app_config_root: String,
    pub app_state_root: String,
    pub workspace_id: String,
    pub last_error: Option<String>,
}
