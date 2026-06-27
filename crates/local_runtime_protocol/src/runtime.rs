use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct RuntimeMeta {
    pub workspace_root: String,
    pub workspace_id: String,
    pub runtime_layer: String,
    pub kernel_layer: String,
    pub transport: String,
    pub python_main_path: bool,
    pub supports: RuntimeSupports,
    pub capability_manifest_ref: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct RuntimeSupports {
    pub workspace_switch: bool,
    pub sse: bool,
    pub containers: bool,
    pub chat_truth: bool,
    pub process_truth: bool,
    pub appdata_state: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct RuntimeHealth {
    pub status: String,
    pub runtime_layer: String,
    pub workspace_id: String,
    pub uptime_ms: u128,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct RuntimeEventPayload {
    pub summary: Option<String>,
    pub message: Option<crate::ContainerMessage>,
    pub record: Option<Value>,
}

impl RuntimeMeta {
    pub fn rust_product_runtime(workspace_root: String, workspace_id: String) -> Self {
        Self {
            workspace_root,
            workspace_id,
            runtime_layer: "rust_product_runtime".into(),
            kernel_layer: "rust_process_kernel".into(),
            transport: "loopback_http_sse".into(),
            python_main_path: false,
            supports: RuntimeSupports {
                workspace_switch: true,
                sse: true,
                containers: true,
                chat_truth: true,
                process_truth: true,
                appdata_state: true,
            },
            capability_manifest_ref: Some("runtime.capabilities".into()),
        }
    }
}
