use axum::extract::{Path, State};
use axum::Json;
use local_runtime_protocol::{ArtifactTargetOption, Page, ProtocolResponse};

use crate::app_state::ProductRuntimeState;

pub async fn target_options(
    State(state): State<ProductRuntimeState>,
    Path(container_id): Path<String>,
) -> Json<ProtocolResponse<Page<ArtifactTargetOption>>> {
    let workspace_uid = state.workspace_uid();
    let services = state.services();
    let items = services.artifact.target_options(&container_id);
    Json(ProtocolResponse::new(
        "req_artifact_targets",
        workspace_uid,
        "artifact.targets",
        Page::new(items, None),
    ))
}
