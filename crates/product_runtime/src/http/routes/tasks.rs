use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use axum::Json;
use local_runtime_protocol::{
    ContainerMessage, ForceCloseRequest, ForceCloseResult, MessageLane, Page, ProtocolResponse,
    StreamOpenRequest, TaskApprovalActionResult, TaskDetail, TaskRecord, TaskStreamRequest,
    TaskUserInputRequest,
};

use crate::app_state::ProductRuntimeState;
use crate::error::{RuntimeError, RuntimeResult};
use crate::http::sse;
use crate::state::message_feed::{message_cursor_event_id, page_cursor_for_messages};

pub async fn list(
    State(state): State<ProductRuntimeState>,
    Path(container_id): Path<String>,
) -> RuntimeResult<Json<ProtocolResponse<Page<TaskRecord>>>> {
    let workspace_uid = state.workspace_uid();
    let services = state.services();
    let tasks = services
        .task
        .list(&container_id)
        .map_err(|err| RuntimeError::internal(workspace_uid.clone(), err.to_string()))?;
    Ok(Json(ProtocolResponse::new(
        "req_tasks",
        workspace_uid,
        "tasks",
        Page::new(tasks, None),
    )))
}

pub async fn start_stream(
    State(state): State<ProductRuntimeState>,
    Path(container_id): Path<String>,
    Json(request): Json<TaskStreamRequest>,
) -> RuntimeResult<impl IntoResponse> {
    let workspace_uid = state.workspace_uid();
    let services = state.services();
    let before = services
        .container
        .messages_page(&container_id, Some(&MessageLane::Task), None, Some(1))
        .map_err(|err| RuntimeError::internal(workspace_uid.clone(), err.to_string()))?;
    let initial_after_event_id = before.last().map(message_cursor_event_id);
    let run_services = services.clone();
    let run_container_id = container_id.clone();
    let poll_services = services.clone();
    let poll_container_id = container_id.clone();
    Ok(sse::live_message_stream(
        workspace_uid,
        initial_after_event_id,
        move || {
            run_services
                .task
                .start_stream(&run_container_id, request)
                .map(|_| ())
                .map_err(|err| err.to_string())
        },
        move |after_event_id| {
            poll_services
                .container
                .messages_page(
                    &poll_container_id,
                    Some(&MessageLane::Task),
                    after_event_id,
                    Some(200),
                )
                .map_err(|err| err.to_string())
        },
        "task.complete",
        "task.error",
        "task.heartbeat",
    ))
}

pub async fn get_one(
    State(state): State<ProductRuntimeState>,
    Path(task_id): Path<String>,
) -> RuntimeResult<Json<ProtocolResponse<TaskDetail>>> {
    let workspace_uid = state.workspace_uid();
    let services = state.services();
    let detail = services
        .task
        .get(&task_id)
        .map_err(|err| RuntimeError::not_found(workspace_uid.clone(), err.to_string()))?;
    Ok(Json(ProtocolResponse::new(
        "req_task_detail",
        workspace_uid,
        "task.detail",
        detail,
    )))
}

pub async fn messages(
    State(state): State<ProductRuntimeState>,
    Path(task_id): Path<String>,
    Query(query): Query<StreamOpenRequest>,
) -> RuntimeResult<Json<ProtocolResponse<Page<ContainerMessage>>>> {
    let workspace_uid = state.workspace_uid();
    let services = state.services();
    let messages = services
        .task
        .messages_page(&task_id, query.after_event_id, query.limit)
        .map_err(|err| RuntimeError::internal(workspace_uid.clone(), err.to_string()))?;
    let cursor = page_cursor_for_messages("message_feed", &messages);
    Ok(Json(ProtocolResponse::new(
        "req_task_messages",
        workspace_uid,
        "task.messages",
        Page::new(messages, cursor),
    )))
}

pub async fn events_stream(
    State(state): State<ProductRuntimeState>,
    Path(task_id): Path<String>,
    Query(query): Query<StreamOpenRequest>,
) -> RuntimeResult<impl IntoResponse> {
    let workspace_uid = state.workspace_uid();
    let services = state.services();
    let events = services
        .task
        .events_stream(&task_id, query.after_event_id, query.limit)
        .map_err(|err| RuntimeError::internal(workspace_uid.clone(), err.to_string()))?;
    Ok(sse::event_batch(events))
}

pub async fn user_input(
    State(state): State<ProductRuntimeState>,
    Path(task_id): Path<String>,
    Json(request): Json<TaskUserInputRequest>,
) -> RuntimeResult<Json<ProtocolResponse<TaskApprovalActionResult>>> {
    let workspace_uid = state.workspace_uid();
    let services = state.services();
    let result = services
        .task
        .submit_user_input(&task_id, request)
        .map_err(|err| RuntimeError::internal(workspace_uid.clone(), err.to_string()))?;
    Ok(Json(ProtocolResponse::new(
        "req_task_user_input",
        workspace_uid,
        "task.user_input",
        result,
    )))
}

pub async fn force_close(
    State(state): State<ProductRuntimeState>,
    Path(task_id): Path<String>,
    Json(request): Json<ForceCloseRequest>,
) -> RuntimeResult<Json<ProtocolResponse<ForceCloseResult>>> {
    let workspace_uid = state.workspace_uid();
    let services = state.services();
    let result = services
        .task
        .force_close(&task_id, request.reason)
        .map_err(|err| RuntimeError::internal(workspace_uid.clone(), err.to_string()))?;
    Ok(Json(ProtocolResponse::new(
        "req_task_force_close",
        workspace_uid,
        "task.force_close",
        result,
    )))
}
