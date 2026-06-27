use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{PROTOCOL_VERSION, RESPONSE_SCHEMA_VERSION};

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct ProtocolResponse<T> {
    pub protocol_version: String,
    pub schema_version: String,
    pub request_id: String,
    pub workspace_id: String,
    pub resource: String,
    pub data: T,
}

impl<T> ProtocolResponse<T> {
    pub fn new(
        request_id: impl Into<String>,
        workspace_id: impl Into<String>,
        resource: impl Into<String>,
        data: T,
    ) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION.to_string(),
            schema_version: RESPONSE_SCHEMA_VERSION.to_string(),
            request_id: request_id.into(),
            workspace_id: workspace_id.into(),
            resource: resource.into(),
            data,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub count: usize,
    pub cursor: Option<Cursor>,
}

impl<T> Page<T> {
    pub fn new(items: Vec<T>, cursor: Option<Cursor>) -> Self {
        let count = items.len();
        Self {
            items,
            count,
            cursor,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct Cursor {
    pub kind: String,
    pub after: Option<String>,
    pub after_event_id: Option<i64>,
}
