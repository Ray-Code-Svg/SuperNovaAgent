use axum::extract::{Path, Query, State};
use axum::Json;
use local_runtime_protocol::{
    ContextPack, ContextPackEstimate, Page, ProtocolResponse, SourceCandidate,
    SourceCandidateRequest,
};

use crate::app_state::ProductRuntimeState;
use crate::error::{RuntimeError, RuntimeResult};

pub async fn get_current(
    State(state): State<ProductRuntimeState>,
    Path(container_id): Path<String>,
) -> RuntimeResult<Json<ProtocolResponse<ContextPack>>> {
    let workspace_uid = state.workspace_uid();
    let services = state.services();
    let pack = services
        .context_pack
        .current(&container_id)
        .map_err(|err| RuntimeError::internal(workspace_uid.clone(), err.to_string()))?;
    Ok(Json(ProtocolResponse::new(
        "req_context_pack",
        workspace_uid,
        "context-pack.current",
        pack,
    )))
}

pub async fn save(
    State(state): State<ProductRuntimeState>,
    Path(container_id): Path<String>,
    Json(mut pack): Json<ContextPack>,
) -> RuntimeResult<Json<ProtocolResponse<ContextPack>>> {
    let workspace_uid = state.workspace_uid();
    let services = state.services();
    pack.container_id = container_id;
    let saved = services
        .context_pack
        .save(pack)
        .map_err(|err| RuntimeError::internal(workspace_uid.clone(), err.to_string()))?;
    Ok(Json(ProtocolResponse::new(
        "req_context_pack_save",
        workspace_uid,
        "context-pack.save",
        saved,
    )))
}

pub async fn estimate(
    State(state): State<ProductRuntimeState>,
    Path(container_id): Path<String>,
    Json(mut pack): Json<ContextPack>,
) -> Json<ProtocolResponse<ContextPackEstimate>> {
    let workspace_uid = state.workspace_uid();
    let services = state.services();
    pack.container_id = container_id;
    let estimate = services.context_pack.estimate(pack);
    Json(ProtocolResponse::new(
        "req_context_pack_estimate",
        workspace_uid,
        "context-pack.estimate",
        estimate,
    ))
}

pub async fn source_candidates(
    State(state): State<ProductRuntimeState>,
    Path(container_id): Path<String>,
    Query(query): Query<SourceCandidateRequest>,
) -> RuntimeResult<Json<ProtocolResponse<Page<SourceCandidate>>>> {
    let workspace_uid = state.workspace_uid();
    let services = state.services();
    let items = services
        .context_pack
        .source_candidates(&container_id, query)
        .map_err(|err| RuntimeError::internal(workspace_uid.clone(), err.to_string()))?;
    Ok(Json(ProtocolResponse::new(
        "req_context_source_candidates",
        workspace_uid,
        "context-pack.source-candidates",
        Page::new(items, None),
    )))
}
