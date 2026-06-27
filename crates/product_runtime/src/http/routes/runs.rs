use axum::extract::{Query, State};
use axum::Json;
use local_runtime_protocol::{Page, ProtocolResponse, RunRecord};
use serde::Deserialize;

use crate::app_state::ProductRuntimeState;
use crate::error::{RuntimeError, RuntimeResult};

#[derive(Debug, Default, Deserialize)]
pub struct RunsQuery {
    pub container_id: Option<String>,
}

pub async fn list(
    State(state): State<ProductRuntimeState>,
    Query(query): Query<RunsQuery>,
) -> RuntimeResult<Json<ProtocolResponse<Page<RunRecord>>>> {
    let workspace_uid = state.workspace_uid();
    let services = state.services();
    if let Some(container_id) = query.container_id.as_deref() {
        services
            .task
            .repair_container_stale_task_runs(container_id)
            .map_err(|err| RuntimeError::internal(workspace_uid.clone(), err.to_string()))?;
    }
    let runs = services
        .run_manager
        .list_runs(query.container_id.as_deref())
        .map_err(|err| RuntimeError::internal(workspace_uid.clone(), err.to_string()))?;
    Ok(Json(ProtocolResponse::new(
        "req_runs",
        workspace_uid,
        "runs",
        Page::new(runs, None),
    )))
}
