use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{Cursor, MessageLane, EVENT_SCHEMA_VERSION, PROTOCOL_VERSION};

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct ProtocolEvent<T> {
    pub protocol_version: String,
    pub schema_version: String,
    pub event_id: String,
    pub event_type: String,
    pub cursor: Cursor,
    pub workspace_id: String,
    pub container_id: Option<String>,
    pub chat_thread_id: Option<String>,
    pub task_id: Option<String>,
    pub job_id: Option<String>,
    pub payload: T,
}

impl<T> ProtocolEvent<T> {
    pub fn new(
        event_id: impl Into<String>,
        event_type: impl Into<String>,
        cursor: Cursor,
        workspace_id: impl Into<String>,
        payload: T,
    ) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION.to_string(),
            schema_version: EVENT_SCHEMA_VERSION.to_string(),
            event_id: event_id.into(),
            event_type: event_type.into(),
            cursor,
            workspace_id: workspace_id.into(),
            container_id: None,
            chat_thread_id: None,
            task_id: None,
            job_id: None,
            payload,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct StreamOpenRequest {
    pub request_id: Option<String>,
    pub after_event_id: Option<i64>,
    pub limit: Option<usize>,
    pub lane: Option<MessageLane>,
}
