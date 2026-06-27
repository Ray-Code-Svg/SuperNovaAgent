use axum::extract::{Path, Query, State};
use axum::Json;
use local_runtime_protocol::{
    ContainerMessage, ContainerRecord, ContainerSnapshot, CreateContainerRequest, Page,
    ProtocolResponse, StreamOpenRequest, UpdateContainerRequest,
};

use crate::app_state::ProductRuntimeState;
use crate::error::{RuntimeError, RuntimeResult};
use crate::state::message_feed::page_cursor_for_messages;

pub async fn list(
    State(state): State<ProductRuntimeState>,
) -> RuntimeResult<Json<ProtocolResponse<Page<ContainerRecord>>>> {
    let workspace_uid = state.workspace_uid();
    let services = state.services();
    let items = services
        .container
        .list()
        .map_err(|err| RuntimeError::internal(workspace_uid.clone(), err.to_string()))?;
    Ok(Json(ProtocolResponse::new(
        "req_containers_list",
        workspace_uid,
        "containers",
        Page::new(items, None),
    )))
}

pub async fn list_archived(
    State(state): State<ProductRuntimeState>,
) -> RuntimeResult<Json<ProtocolResponse<Page<ContainerRecord>>>> {
    let workspace_uid = state.workspace_uid();
    let services = state.services();
    let items = services
        .container
        .list_archived()
        .map_err(|err| RuntimeError::internal(workspace_uid.clone(), err.to_string()))?;
    Ok(Json(ProtocolResponse::new(
        "req_containers_archived",
        workspace_uid,
        "containers.archived",
        Page::new(items, None),
    )))
}

pub async fn create(
    State(state): State<ProductRuntimeState>,
    Json(request): Json<CreateContainerRequest>,
) -> RuntimeResult<Json<ProtocolResponse<ContainerRecord>>> {
    let workspace_uid = state.workspace_uid();
    let services = state.services();
    let container = services
        .container
        .create(request)
        .map_err(|err| RuntimeError::internal(workspace_uid.clone(), err.to_string()))?;
    Ok(Json(ProtocolResponse::new(
        "req_container_create",
        workspace_uid,
        "container.create",
        container,
    )))
}

pub async fn get_one(
    State(state): State<ProductRuntimeState>,
    Path(container_id): Path<String>,
) -> RuntimeResult<Json<ProtocolResponse<ContainerRecord>>> {
    let workspace_uid = state.workspace_uid();
    let services = state.services();
    let container = services
        .container
        .get(&container_id)
        .map_err(|err| RuntimeError::not_found(workspace_uid.clone(), err.to_string()))?;
    Ok(Json(ProtocolResponse::new(
        "req_container_get",
        workspace_uid,
        "container",
        container,
    )))
}

pub async fn update(
    State(state): State<ProductRuntimeState>,
    Path(container_id): Path<String>,
    Json(request): Json<UpdateContainerRequest>,
) -> RuntimeResult<Json<ProtocolResponse<ContainerRecord>>> {
    let workspace_uid = state.workspace_uid();
    let services = state.services();
    let container = services
        .container
        .update(&container_id, request)
        .map_err(|err| RuntimeError::internal(workspace_uid.clone(), err.to_string()))?;
    Ok(Json(ProtocolResponse::new(
        "req_container_update",
        workspace_uid,
        "container.update",
        container,
    )))
}

pub async fn activate(
    State(state): State<ProductRuntimeState>,
    Path(container_id): Path<String>,
) -> RuntimeResult<Json<ProtocolResponse<ContainerRecord>>> {
    let workspace_uid = state.workspace_uid();
    let services = state.services();
    let container = services
        .container
        .activate(&container_id)
        .map_err(|err| RuntimeError::internal(workspace_uid.clone(), err.to_string()))?;
    Ok(Json(ProtocolResponse::new(
        "req_container_activate",
        workspace_uid,
        "container.activate",
        container,
    )))
}

pub async fn archive(
    State(state): State<ProductRuntimeState>,
    Path(container_id): Path<String>,
) -> RuntimeResult<Json<ProtocolResponse<ContainerRecord>>> {
    let workspace_uid = state.workspace_uid();
    let services = state.services();
    let container = services
        .container
        .archive(&container_id)
        .map_err(|err| RuntimeError::internal(workspace_uid.clone(), err.to_string()))?;
    Ok(Json(ProtocolResponse::new(
        "req_container_archive",
        workspace_uid,
        "container.archive",
        container,
    )))
}

pub async fn restore(
    State(state): State<ProductRuntimeState>,
    Path(container_id): Path<String>,
) -> RuntimeResult<Json<ProtocolResponse<ContainerRecord>>> {
    let workspace_uid = state.workspace_uid();
    let services = state.services();
    let container = services
        .container
        .restore(&container_id)
        .map_err(|err| RuntimeError::internal(workspace_uid.clone(), err.to_string()))?;
    Ok(Json(ProtocolResponse::new(
        "req_container_restore",
        workspace_uid,
        "container.restore",
        container,
    )))
}

pub async fn delete_one(
    State(state): State<ProductRuntimeState>,
    Path(container_id): Path<String>,
) -> RuntimeResult<Json<ProtocolResponse<ContainerRecord>>> {
    let workspace_uid = state.workspace_uid();
    let services = state.services();
    let container = services
        .container
        .delete(&container_id)
        .map_err(|err| RuntimeError::internal(workspace_uid.clone(), err.to_string()))?;
    Ok(Json(ProtocolResponse::new(
        "req_container_delete",
        workspace_uid,
        "container.delete",
        container,
    )))
}

pub async fn snapshot(
    State(state): State<ProductRuntimeState>,
    Path(container_id): Path<String>,
) -> RuntimeResult<Json<ProtocolResponse<ContainerSnapshot>>> {
    let workspace_uid = state.workspace_uid();
    let services = state.services();
    services
        .chat
        .hydrate_container(&container_id)
        .map_err(|err| RuntimeError::internal(workspace_uid.clone(), err.to_string()))?;
    services
        .task
        .hydrate_container(&container_id)
        .map_err(|err| RuntimeError::internal(workspace_uid.clone(), err.to_string()))?;
    let snapshot = services
        .container
        .snapshot(&container_id)
        .map_err(|err| RuntimeError::internal(workspace_uid.clone(), err.to_string()))?;
    Ok(Json(ProtocolResponse::new(
        "req_container_snapshot",
        workspace_uid,
        "container.snapshot",
        snapshot,
    )))
}

pub async fn messages(
    State(state): State<ProductRuntimeState>,
    Path(container_id): Path<String>,
    Query(query): Query<StreamOpenRequest>,
) -> RuntimeResult<Json<ProtocolResponse<Page<ContainerMessage>>>> {
    let workspace_uid = state.workspace_uid();
    let services = state.services();
    let messages = services
        .container
        .messages_page(
            &container_id,
            query.lane.as_ref(),
            query.after_event_id,
            query.limit,
        )
        .map_err(|err| RuntimeError::internal(workspace_uid.clone(), err.to_string()))?;
    let cursor = page_cursor_for_messages("message_feed", &messages);
    Ok(Json(ProtocolResponse::new(
        "req_container_messages",
        workspace_uid,
        "container.messages",
        Page::new(messages, cursor),
    )))
}
