use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{ERROR_SCHEMA_VERSION, PROTOCOL_VERSION};

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct ProtocolErrorEnvelope {
    pub protocol_version: String,
    pub schema_version: String,
    pub request_id: String,
    pub workspace_id: String,
    pub error: ProtocolError,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct ProtocolError {
    pub code: String,
    pub message: String,
    pub status: u16,
    pub scope: String,
    pub retryable: bool,
    pub detail: Value,
}

impl ProtocolErrorEnvelope {
    pub fn new(
        request_id: impl Into<String>,
        workspace_id: impl Into<String>,
        code: impl Into<String>,
        message: impl Into<String>,
        status: u16,
        scope: impl Into<String>,
    ) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION.to_string(),
            schema_version: ERROR_SCHEMA_VERSION.to_string(),
            request_id: request_id.into(),
            workspace_id: workspace_id.into(),
            error: ProtocolError {
                code: code.into(),
                message: message.into(),
                status,
                scope: scope.into(),
                retryable: false,
                detail: Value::Object(Default::default()),
            },
        }
    }
}
