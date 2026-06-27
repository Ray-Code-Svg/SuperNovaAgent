use axum::extract::State;
use axum::Json;
use local_runtime_protocol::{DiagnosticsSnapshot, ProtocolResponse};

use crate::app_state::ProductRuntimeState;

pub async fn get(
    State(state): State<ProductRuntimeState>,
) -> Json<ProtocolResponse<DiagnosticsSnapshot>> {
    let workspace_uid = state.workspace_uid();
    let services = state.services();
    Json(ProtocolResponse::new(
        "req_diagnostics",
        workspace_uid,
        "diagnostics",
        services.diagnostics.snapshot(),
    ))
}
