use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct RunRecord {
    pub run_id: String,
    pub workspace_uid: String,
    pub container_id: String,
    pub run_kind: String,
    pub chat_thread_id: Option<String>,
    pub task_id: Option<String>,
    pub job_id: Option<String>,
    pub worker_id: String,
    pub status: String,
    pub cancel_requested: bool,
    pub heartbeat_at_ms: Option<u128>,
    pub started_at_ms: u128,
    pub updated_at_ms: u128,
    pub error_message: Option<String>,
}
