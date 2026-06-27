use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use local_runtime_protocol::ProtocolErrorEnvelope;

#[derive(Debug)]
pub struct RuntimeError {
    pub status: StatusCode,
    pub code: String,
    pub message: String,
    pub scope: String,
    pub workspace_id: String,
}

impl RuntimeError {
    pub fn internal(workspace_id: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "RUNTIME_INTERNAL_ERROR".into(),
            message: message.into(),
            scope: "runtime".into(),
            workspace_id: workspace_id.into(),
        }
    }

    pub fn not_found(workspace_id: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            code: "NOT_FOUND".into(),
            message: message.into(),
            scope: "runtime".into(),
            workspace_id: workspace_id.into(),
        }
    }

    pub fn bad_request(workspace_id: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code: "BAD_REQUEST".into(),
            message: message.into(),
            scope: "runtime".into(),
            workspace_id: workspace_id.into(),
        }
    }
}

impl IntoResponse for RuntimeError {
    fn into_response(self) -> Response {
        let envelope = ProtocolErrorEnvelope::new(
            "req_runtime_error",
            self.workspace_id,
            self.code,
            self.message,
            self.status.as_u16(),
            self.scope,
        );
        (self.status, Json(envelope)).into_response()
    }
}

pub type RuntimeResult<T> = Result<T, RuntimeError>;
