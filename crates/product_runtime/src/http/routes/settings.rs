use axum::extract::State;
use axum::Json;
use local_runtime_protocol::{
    AppSettings, ProtocolResponse, ProviderApiSettings, ProviderApiTestRequest,
    ProviderApiTestResult, ProviderApiUpdateRequest,
};
use std::io;

use crate::app_state::ProductRuntimeState;
use crate::error::{RuntimeError, RuntimeResult};

pub async fn get(
    State(state): State<ProductRuntimeState>,
) -> RuntimeResult<Json<ProtocolResponse<AppSettings>>> {
    let workspace_uid = state.workspace_uid();
    let services = state.services();
    let settings = services
        .settings
        .get()
        .map_err(|err| RuntimeError::internal(workspace_uid.clone(), err.to_string()))?;
    Ok(Json(ProtocolResponse::new(
        "req_settings",
        workspace_uid,
        "settings",
        settings,
    )))
}

pub async fn update(
    State(state): State<ProductRuntimeState>,
    Json(request): Json<AppSettings>,
) -> RuntimeResult<Json<ProtocolResponse<AppSettings>>> {
    let workspace_uid = state.workspace_uid();
    let services = state.services();
    let settings = services
        .settings
        .update(request)
        .map_err(|err| RuntimeError::internal(workspace_uid.clone(), err.to_string()))?;
    Ok(Json(ProtocolResponse::new(
        "req_settings_update",
        workspace_uid,
        "settings.update",
        settings,
    )))
}

pub async fn provider(
    State(state): State<ProductRuntimeState>,
) -> RuntimeResult<Json<ProtocolResponse<ProviderApiSettings>>> {
    let workspace_uid = state.workspace_uid();
    let services = state.services();
    let settings = services
        .settings
        .provider_settings()
        .map_err(|err| RuntimeError::internal(workspace_uid.clone(), err.to_string()))?;
    Ok(Json(ProtocolResponse::new(
        "req_settings_provider",
        workspace_uid,
        "settings.provider",
        settings,
    )))
}

pub async fn update_provider(
    State(state): State<ProductRuntimeState>,
    Json(request): Json<ProviderApiUpdateRequest>,
) -> RuntimeResult<Json<ProtocolResponse<ProviderApiSettings>>> {
    let workspace_uid = state.workspace_uid();
    let services = state.services();
    let settings = services
        .settings
        .update_provider(request)
        .map_err(|err| settings_error(workspace_uid.clone(), err))?;
    Ok(Json(ProtocolResponse::new(
        "req_settings_provider_update",
        workspace_uid,
        "settings.provider.update",
        settings,
    )))
}

pub async fn test_provider(
    State(state): State<ProductRuntimeState>,
    Json(request): Json<ProviderApiTestRequest>,
) -> RuntimeResult<Json<ProtocolResponse<ProviderApiTestResult>>> {
    let workspace_uid = state.workspace_uid();
    let services = state.services();
    let result = services
        .settings
        .test_provider(request)
        .map_err(|err| RuntimeError::internal(workspace_uid.clone(), err.to_string()))?;
    Ok(Json(ProtocolResponse::new(
        "req_settings_provider_test",
        workspace_uid,
        "settings.provider.test",
        result,
    )))
}

fn settings_error(workspace_uid: String, err: io::Error) -> RuntimeError {
    if err.kind() == io::ErrorKind::InvalidInput {
        return RuntimeError::bad_request(workspace_uid, err.to_string());
    }
    RuntimeError::internal(workspace_uid, err.to_string())
}
