use axum::body::Body;
use axum::extract::State;
use axum::http::{Method, Request, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use local_runtime_protocol::ProtocolErrorEnvelope;

use crate::app_state::ProductRuntimeState;

pub const RUNTIME_TOKEN_HEADER: &str = "x-supernova-runtime-token";

pub async fn require_runtime_token(
    State(state): State<ProductRuntimeState>,
    req: Request<Body>,
    next: Next,
) -> Response {
    if req.method() == Method::OPTIONS {
        return next.run(req).await;
    }

    let authorized = req
        .headers()
        .get(RUNTIME_TOKEN_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(|value| value == state.runtime_token())
        .unwrap_or(false);

    if authorized {
        return next.run(req).await;
    }

    let envelope = ProtocolErrorEnvelope::new(
        "req_runtime_auth",
        state.workspace_uid(),
        "RUNTIME_UNAUTHORIZED",
        "Missing or invalid SuperNova runtime token.",
        StatusCode::UNAUTHORIZED.as_u16(),
        "runtime.auth",
    );
    (StatusCode::UNAUTHORIZED, Json(envelope)).into_response()
}
