use axum::extract::State;
use axum::Json;
use local_runtime_protocol::{ModelConfig, ModelConfigDescriptor, ProtocolResponse};

use crate::app_state::ProductRuntimeState;
use crate::error::{RuntimeError, RuntimeResult};

pub async fn get(
    State(state): State<ProductRuntimeState>,
) -> Json<ProtocolResponse<ModelConfigDescriptor>> {
    let workspace_uid = state.workspace_uid();
    let services = state.services();
    Json(ProtocolResponse::new(
        "req_model_config",
        workspace_uid,
        "model-config",
        services.model_config.descriptor(),
    ))
}

pub async fn update(
    State(state): State<ProductRuntimeState>,
    Json(request): Json<ModelConfig>,
) -> RuntimeResult<Json<ProtocolResponse<ModelConfigDescriptor>>> {
    let workspace_uid = state.workspace_uid();
    let services = state.services();
    let descriptor = services
        .model_config
        .update(request)
        .map_err(|err| RuntimeError::internal(workspace_uid.clone(), err.to_string()))?;
    Ok(Json(ProtocolResponse::new(
        "req_model_config_update",
        workspace_uid,
        "model-config.update",
        descriptor,
    )))
}
