use axum::extract::{Path, State};
use axum::Json;
use std::path::PathBuf;

use local_runtime_protocol::{
    ActivateWorkspaceRequest, ContainerRecord, CreateWorkspaceRequest, Page, ProtocolResponse,
    WorkspaceActivation, WorkspaceRecord,
};

use crate::app_state::ProductRuntimeState;
use crate::error::{RuntimeError, RuntimeResult};
use crate::state::product_db::ProductDb;

pub async fn list(
    State(state): State<ProductRuntimeState>,
) -> RuntimeResult<Json<ProtocolResponse<Page<WorkspaceRecord>>>> {
    let workspace_uid = state.workspace_uid();
    let services = state.services();
    let items = services
        .workspace
        .list()
        .map_err(|err| RuntimeError::internal(workspace_uid.clone(), err.to_string()))?;
    Ok(Json(ProtocolResponse::new(
        "req_workspaces_list",
        workspace_uid,
        "workspaces",
        Page::new(items, None),
    )))
}

pub async fn create(
    State(state): State<ProductRuntimeState>,
    Json(request): Json<CreateWorkspaceRequest>,
) -> RuntimeResult<Json<ProtocolResponse<WorkspaceRecord>>> {
    let workspace_uid = state.workspace_uid();
    let services = state.services();
    let workspace = services
        .workspace
        .create(request)
        .map_err(|err| workspace_error(workspace_uid.clone(), err))?;
    Ok(Json(ProtocolResponse::new(
        "req_workspace_create",
        workspace_uid,
        "workspace.create",
        workspace,
    )))
}

pub async fn archive(
    State(state): State<ProductRuntimeState>,
    Path(workspace_uid): Path<String>,
) -> RuntimeResult<Json<ProtocolResponse<WorkspaceRecord>>> {
    let active_workspace_uid = state.workspace_uid();
    let services = state.services();
    let active_before_archive = services
        .workspace
        .list()
        .map_err(|err| RuntimeError::internal(active_workspace_uid.clone(), err.to_string()))?;
    if workspace_uid == active_workspace_uid && active_before_archive.len() <= 1 {
        return Err(RuntimeError::bad_request(
            active_workspace_uid,
            "Cannot archive the last workspace.",
        ));
    }
    let workspace = services
        .workspace
        .archive(&workspace_uid)
        .map_err(|err| RuntimeError::internal(active_workspace_uid.clone(), err.to_string()))?;
    if workspace_uid == active_workspace_uid {
        let next_workspace = services
            .workspace
            .list()
            .map_err(|err| RuntimeError::internal(workspace_uid.clone(), err.to_string()))?
            .into_iter()
            .next()
            .ok_or_else(|| {
                RuntimeError::internal(
                    workspace_uid.clone(),
                    "No workspace available after archive.",
                )
            })?;
        state
            .rebind_workspace(
                PathBuf::from(&next_workspace.workspace_root),
                next_workspace.workspace_uid,
            )
            .map_err(|err| RuntimeError::internal(workspace_uid.clone(), err.to_string()))?;
    }
    let response_workspace_uid = state.workspace_uid();
    Ok(Json(ProtocolResponse::new(
        "req_workspace_archive",
        response_workspace_uid,
        "workspace.archive",
        workspace,
    )))
}

pub async fn activate(
    State(state): State<ProductRuntimeState>,
    Json(request): Json<ActivateWorkspaceRequest>,
) -> RuntimeResult<Json<ProtocolResponse<WorkspaceActivation>>> {
    let workspace_uid = state.workspace_uid();
    let state_for_activation = state.clone();
    let error_workspace_uid = workspace_uid.clone();
    let activation = tokio::task::spawn_blocking(move || {
        let services = state_for_activation.services();
        let mut activation = services
            .workspace
            .activate(request)
            .map_err(|err| workspace_error(error_workspace_uid.clone(), err))?;
        activation.recent_active_container_id = state_for_activation
            .rebind_workspace(
                PathBuf::from(&activation.workspace.workspace_root),
                activation.workspace.workspace_uid.clone(),
            )
            .map_err(|err| RuntimeError::internal(error_workspace_uid.clone(), err.to_string()))?;
        Ok::<WorkspaceActivation, RuntimeError>(activation)
    })
    .await
    .map_err(|err| RuntimeError::internal(workspace_uid.clone(), err.to_string()))??;
    let active_workspace_uid = state.workspace_uid();
    Ok(Json(ProtocolResponse::new(
        "req_workspace_activate",
        active_workspace_uid,
        "workspace.activate",
        activation,
    )))
}

pub async fn containers(
    State(state): State<ProductRuntimeState>,
    Path(workspace_uid): Path<String>,
) -> RuntimeResult<Json<ProtocolResponse<Page<ContainerRecord>>>> {
    let active_workspace_uid = state.workspace_uid();
    let services = state.services();
    services
        .workspace
        .get(&workspace_uid)
        .map_err(|err| RuntimeError::not_found(active_workspace_uid.clone(), err.to_string()))?;

    let items = if workspace_uid == active_workspace_uid {
        services
            .container
            .list()
            .map_err(|err| RuntimeError::internal(active_workspace_uid.clone(), err.to_string()))?
    } else {
        let workspace_state_root = state.app_paths.workspace_state_root(&workspace_uid);
        match ProductDb::open_existing(&workspace_state_root, workspace_uid.clone())
            .map_err(|err| RuntimeError::internal(active_workspace_uid.clone(), err.to_string()))?
        {
            Some(db) => with_container_badges(
                &db,
                db.list_container_projections(false).map_err(|err| {
                    RuntimeError::internal(active_workspace_uid.clone(), err.to_string())
                })?,
            )
            .map_err(|err| RuntimeError::internal(active_workspace_uid.clone(), err.to_string()))?,
            None => Vec::new(),
        }
    };

    Ok(Json(ProtocolResponse::new(
        "req_workspace_containers",
        active_workspace_uid,
        "workspace.containers",
        Page::new(items, None),
    )))
}

fn with_container_badges(
    db: &ProductDb,
    mut records: Vec<ContainerRecord>,
) -> rusqlite::Result<Vec<ContainerRecord>> {
    for record in &mut records {
        let tasks = db.list_tasks(&record.container_id)?;
        for task in tasks {
            match task.status.as_str() {
                "running" => record.badges.running = record.badges.running.saturating_add(1),
                "blocked" | "failed" | "interrupted" => {
                    record.badges.blocked = record.badges.blocked.saturating_add(1)
                }
                _ => {}
            }
            record.badges.blocked = record.badges.blocked.saturating_add(task.badges.blocked);
            record.badges.artifact_ready = record
                .badges
                .artifact_ready
                .saturating_add(task.badges.artifact_ready);
        }
    }
    Ok(records)
}

fn workspace_error(workspace_uid: String, err: rusqlite::Error) -> RuntimeError {
    if matches!(err, rusqlite::Error::InvalidParameterName(_)) {
        return RuntimeError::bad_request(workspace_uid, err.to_string());
    }
    RuntimeError::internal(workspace_uid, err.to_string())
}
