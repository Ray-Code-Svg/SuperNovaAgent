use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use axum::Json;
use local_runtime_protocol::{
    ChatThreadRecord, ChatTurnStreamRequest, ContainerMessage, CreateChatThreadRequest,
    ForceCloseRequest, ForceCloseResult, Page, ProtocolResponse, StreamOpenRequest,
};

use crate::app_state::ProductRuntimeState;
use crate::error::{RuntimeError, RuntimeResult};
use crate::http::sse;
use crate::state::message_feed::{message_cursor_event_id, page_cursor_for_messages};

pub async fn threads(
    State(state): State<ProductRuntimeState>,
    Path(container_id): Path<String>,
) -> RuntimeResult<Json<ProtocolResponse<Page<ChatThreadRecord>>>> {
    let workspace_uid = state.workspace_uid();
    let services = state.services();
    let threads = services
        .chat
        .list_threads(&container_id)
        .map_err(|err| RuntimeError::internal(workspace_uid.clone(), err.to_string()))?;
    Ok(Json(ProtocolResponse::new(
        "req_chat_threads",
        workspace_uid,
        "chat.threads",
        Page::new(threads, None),
    )))
}

pub async fn create_thread(
    State(state): State<ProductRuntimeState>,
    Path(container_id): Path<String>,
    Json(request): Json<CreateChatThreadRequest>,
) -> RuntimeResult<Json<ProtocolResponse<ChatThreadRecord>>> {
    let workspace_uid = state.workspace_uid();
    let services = state.services();
    let thread = services
        .chat
        .create_thread(&container_id, request.title)
        .map_err(|err| RuntimeError::internal(workspace_uid.clone(), err.to_string()))?;
    Ok(Json(ProtocolResponse::new(
        "req_chat_thread_create",
        workspace_uid,
        "chat.thread.create",
        thread,
    )))
}

pub async fn messages(
    State(state): State<ProductRuntimeState>,
    Path(chat_thread_id): Path<String>,
    Query(query): Query<StreamOpenRequest>,
) -> RuntimeResult<Json<ProtocolResponse<Page<ContainerMessage>>>> {
    let workspace_uid = state.workspace_uid();
    let services = state.services();
    let messages = services
        .chat
        .messages_page(&chat_thread_id, query.after_event_id, query.limit)
        .map_err(|err| RuntimeError::internal(workspace_uid.clone(), err.to_string()))?;
    let cursor = page_cursor_for_messages("message_feed", &messages);
    Ok(Json(ProtocolResponse::new(
        "req_chat_messages",
        workspace_uid,
        "chat.messages",
        Page::new(messages, cursor),
    )))
}

pub async fn turn_stream(
    State(state): State<ProductRuntimeState>,
    Path(chat_thread_id): Path<String>,
    Json(request): Json<ChatTurnStreamRequest>,
) -> RuntimeResult<impl IntoResponse> {
    let workspace_uid = state.workspace_uid();
    let services = state.services();
    let before = services
        .chat
        .messages_page(&chat_thread_id, None, Some(1))
        .map_err(|err| RuntimeError::internal(workspace_uid.clone(), err.to_string()))?;
    let initial_after_event_id = before.last().map(message_cursor_event_id);
    let run_services = services.clone();
    let run_thread_id = chat_thread_id.clone();
    let poll_services = services.clone();
    let poll_thread_id = chat_thread_id.clone();
    Ok(sse::live_message_stream(
        workspace_uid,
        initial_after_event_id,
        move || {
            run_services
                .chat
                .record_turn(&run_thread_id, request)
                .map(|_| ())
                .map_err(|err| err.to_string())
        },
        move |after_event_id| {
            poll_services
                .chat
                .messages_page(&poll_thread_id, after_event_id, Some(200))
                .map_err(|err| err.to_string())
        },
        "chat.complete",
        "chat.error",
        "chat.heartbeat",
    ))
}

pub async fn force_close(
    State(state): State<ProductRuntimeState>,
    Path(chat_thread_id): Path<String>,
    Json(request): Json<ForceCloseRequest>,
) -> RuntimeResult<Json<ProtocolResponse<ForceCloseResult>>> {
    let workspace_uid = state.workspace_uid();
    let services = state.services();
    let result = services
        .chat
        .force_close(&chat_thread_id, request.reason)
        .map_err(|err| RuntimeError::internal(workspace_uid.clone(), err.to_string()))?;
    Ok(Json(ProtocolResponse::new(
        "req_chat_force_close",
        workspace_uid,
        "chat.force_close",
        result,
    )))
}
