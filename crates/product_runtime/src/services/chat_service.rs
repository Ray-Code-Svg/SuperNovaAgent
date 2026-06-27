use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use axum::response::sse::Event;
use local_runtime_protocol::{
    ChatStreamPayload, ChatThreadRecord, ChatTurnStreamRequest, ContainerMessage, ForceCloseResult,
    MessageLane, MessageRole, MessageType,
};
use serde_json::{json, Value};
use supernova_process_kernel::{
    ChatEvent, ChatTurnStatus, ModelStreamDelta, ModelStreamDeltaKind, ModelStreamSink,
};

use crate::http::sse;
use crate::kernel::event_projection::{chat_event_to_message, chat_thread_to_record};
use crate::kernel::KernelBridge;
use crate::services::context_pack_service::ContextPackService;
use crate::services::run_manager::{ChatWorkerRequest, ProcessWorkerEvent, RunManager};
use crate::services::settings_service::SettingsService;
use crate::state::message_feed::{advance_message_cursor, message_cursor_event_id, new_message};
use crate::state::product_db::ProductDb;
use crate::state::projection_shards::ProjectionShardDb;

#[derive(Clone)]
pub struct ChatService {
    db: ProductDb,
    kernel: KernelBridge,
    context_pack: ContextPackService,
    run_manager: RunManager,
    settings: SettingsService,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChatProjectionRepairReport {
    pub threads_scanned: usize,
    pub threads_with_truth: usize,
    pub messages_projected: usize,
    pub runs_repaired: usize,
    pub database_locked_messages_downgraded: usize,
}

impl ChatService {
    pub fn new(
        db: ProductDb,
        kernel: KernelBridge,
        context_pack: ContextPackService,
        run_manager: RunManager,
        settings: SettingsService,
    ) -> Self {
        Self {
            db,
            kernel,
            context_pack,
            run_manager,
            settings,
        }
    }

    pub fn list_threads(&self, container_id: &str) -> rusqlite::Result<Vec<ChatThreadRecord>> {
        if let Ok(threads) = self.kernel.list_chat_threads(container_id) {
            for thread in threads {
                let record = chat_thread_to_record(thread);
                let _ = self.db.upsert_chat_thread(&record)?;
            }
        }
        self.db.list_chat_threads(container_id)
    }

    pub fn create_thread(
        &self,
        container_id: &str,
        title: Option<String>,
    ) -> rusqlite::Result<ChatThreadRecord> {
        let thread = self
            .kernel
            .create_chat_thread(container_id, title.clone())
            .map(chat_thread_to_record)
            .map_err(|err| rusqlite::Error::ToSqlConversionFailure(Box::new(err)))?;
        self.db.upsert_chat_thread(&thread)
    }

    pub fn messages(&self, chat_thread_id: &str) -> rusqlite::Result<Vec<ContainerMessage>> {
        self.hydrate_thread(chat_thread_id)?;
        self.db
            .list_projected_chat_messages_page(chat_thread_id, None, None)
    }

    pub fn messages_page(
        &self,
        chat_thread_id: &str,
        after_event_id: Option<i64>,
        limit: Option<usize>,
    ) -> rusqlite::Result<Vec<ContainerMessage>> {
        self.db
            .list_projected_chat_messages_page(chat_thread_id, after_event_id, limit)
    }

    pub fn force_close(
        &self,
        chat_thread_id: &str,
        reason: Option<String>,
    ) -> rusqlite::Result<ForceCloseResult> {
        let reason = reason
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "User forced this chat turn closed from Workbench v2.".to_string());
        let event = self
            .kernel
            .force_close_chat_turn(chat_thread_id, &reason)
            .map_err(|err| rusqlite::Error::ToSqlConversionFailure(Box::new(err)))?;
        if let Some(message) = self.project_chat_event(&event, false)? {
            let _ = self.append_chat_projection_message(chat_thread_id, message)?;
        }
        let _ = self.run_manager.request_cancel_chat_run(chat_thread_id)?;
        self.complete_chat_runtime_phase(chat_thread_id, "cancelled")?;
        Ok(ForceCloseResult {
            action: "force_close".into(),
            status: "cancelled".into(),
            messages: self
                .db
                .list_projected_chat_messages_page(chat_thread_id, None, None)?,
        })
    }

    pub fn hydrate_container(&self, container_id: &str) -> rusqlite::Result<()> {
        for thread in self.list_threads(container_id)? {
            self.hydrate_thread(&thread.chat_thread_id)?;
        }
        Ok(())
    }

    pub fn repair_workspace_projection(&self) -> rusqlite::Result<ChatProjectionRepairReport> {
        let mut report = ChatProjectionRepairReport::default();
        for thread in self.db.list_all_chat_threads()? {
            report.threads_scanned += 1;
            let Ok(chat_events) = self.kernel.read_chat_events(&thread.chat_thread_id) else {
                continue;
            };
            if chat_events.is_empty() {
                continue;
            }
            report.threads_with_truth += 1;
            let before_count = self
                .db
                .list_projected_chat_messages_page(&thread.chat_thread_id, None, None)?
                .len();
            for chat_event in &chat_events {
                if let Some(message) = self.project_chat_event(chat_event, false)? {
                    let _ = self.append_chat_projection_message(&thread.chat_thread_id, message)?;
                }
            }
            let after_count = self
                .db
                .list_projected_chat_messages_page(&thread.chat_thread_id, None, None)?
                .len();
            report.messages_projected += after_count.saturating_sub(before_count);
            if let Some((terminal_status, terminal_error_message)) =
                chat_terminal_status_from_events(&chat_events)
            {
                self.complete_chat_runtime_phase(&thread.chat_thread_id, &terminal_status)?;
                report.runs_repaired += self.db.repair_database_locked_chat_runs(
                    &thread.chat_thread_id,
                    &terminal_status,
                    terminal_error_message.as_deref(),
                )?;
                if terminal_status != "running" {
                    report.database_locked_messages_downgraded += self
                        .db
                        .downgrade_database_locked_chat_errors(&thread.container_id)?;
                }
            }
        }
        Ok(report)
    }

    pub fn record_turn(
        &self,
        chat_thread_id: &str,
        request: ChatTurnStreamRequest,
    ) -> rusqlite::Result<Vec<Event>> {
        let response_language = self
            .settings
            .appearance_settings()
            .map_err(io_to_sqlite)?
            .language;
        let thread = self.db.get_chat_thread(chat_thread_id)?;
        let context_pack = self
            .context_pack
            .materialize_for_request(&thread.container_id, request.context_pack.clone())?;
        let mut events = Vec::new();
        let mut user = new_message(
            &self.db.workspace_uid,
            &thread.container_id,
            MessageLane::Chat,
            MessageRole::User,
            MessageType::Text,
            Some(request.message.clone()),
            Some(chat_thread_id.to_string()),
        );
        user.source_kind = "product_runtime_request".into();
        user.source_ref = "chat_turn_pending".into();
        let user = self.db.append_message(user)?;
        events.push(sse_message(
            "chat.user.message",
            &self.db.workspace_uid,
            Some(user),
        ));

        let mut running = new_message(
            &self.db.workspace_uid,
            &thread.container_id,
            MessageLane::Chat,
            MessageRole::System,
            MessageType::Phase,
            Some("Kernel ChatRuntime is processing this turn.".into()),
            Some(chat_thread_id.to_string()),
        );
        running.status = "streaming".into();
        running.title = Some("ChatRuntime running".into());
        running.source_kind = "product_runtime_runtime".into();
        running.source_ref = "chat_turn_running".into();
        let running = self.db.append_message(running)?;
        let mut last_cursor_event_id = Some(message_cursor_event_id(&running));
        events.push(sse_message(
            "chat.phase",
            &self.db.workspace_uid,
            Some(running),
        ));
        let run = self
            .run_manager
            .start_chat_run(&thread.container_id, chat_thread_id)?;
        let run_id = run.run_id.clone();
        let chat_projection_shard = self
            .db
            .ensure_chat_projection_shard(&thread.container_id, chat_thread_id)?;

        let stream_sink = Arc::new(ProductChatStreamSink::new(
            self.db.workspace_uid.clone(),
            thread.container_id.clone(),
            chat_thread_id.to_string(),
            chat_projection_shard,
        ));
        let chat_result = if self.run_manager.process_worker_enabled() {
            let stream_sink_for_worker = stream_sink.clone();
            self.run_manager.run_chat_in_process_worker(
                &run_id,
                ChatWorkerRequest {
                    container_id: thread.container_id.clone(),
                    chat_thread_id: Some(chat_thread_id.to_string()),
                    message: request.message.clone(),
                    context_pack,
                    source_guidance: request.source_guidance.clone(),
                    model_config: request.model_config,
                    response_language,
                },
                move |event| {
                    if let ProcessWorkerEvent::ModelStreamDelta(delta) = event {
                        stream_sink_for_worker.on_model_stream_delta(delta);
                    }
                    Ok(())
                },
            )
        } else {
            self.kernel
                .start_chat_turn_with_stream_sink_and_response_language(
                    &thread.container_id,
                    Some(chat_thread_id.to_string()),
                    request.message.clone(),
                    context_pack,
                    request.source_guidance.clone(),
                    request.model_config,
                    stream_sink,
                    response_language,
                )
        };
        match chat_result {
            Ok(result) => {
                if self.chat_thread_was_force_closed(chat_thread_id)? {
                    self.complete_chat_runtime_phase(chat_thread_id, "cancelled")?;
                    let _ = self.run_manager.complete_run(&run_id, "cancelled")?;
                    last_cursor_event_id = self
                        .db
                        .list_projected_chat_messages_page(chat_thread_id, None, None)?
                        .iter()
                        .map(message_cursor_event_id)
                        .max();
                    events.push(sse_payload(
                        "chat.complete",
                        &self.db.workspace_uid,
                        last_cursor_event_id,
                        json!({"status": "cancelled"}),
                    ));
                    return Ok(events);
                }
                let suppress_final_answer = self.complete_streamed_assistant_answer(
                    chat_thread_id,
                    result.assistant_content.as_deref(),
                )?;
                self.complete_streamed_chat_reasoning(chat_thread_id)?;
                let runtime_status = chat_runtime_phase_status(&result.status);
                self.complete_chat_runtime_phase(chat_thread_id, runtime_status)?;
                let _ = self.run_manager.complete_run(&run_id, runtime_status)?;
                for chat_event in &result.events {
                    if let Some(message) =
                        self.project_chat_event(chat_event, suppress_final_answer)?
                    {
                        let message =
                            self.append_chat_projection_message(chat_thread_id, message)?;
                        last_cursor_event_id = Some(message_cursor_event_id(&message));
                        let event_type = event_type_for_message(&message);
                        events.push(sse_message(
                            event_type,
                            &self.db.workspace_uid,
                            Some(message),
                        ));
                    }
                }
                events.push(sse_payload(
                    if runtime_status == "failed" {
                        "chat.error"
                    } else {
                        "chat.complete"
                    },
                    &self.db.workspace_uid,
                    last_cursor_event_id,
                    json!({"status": runtime_status}),
                ));
            }
            Err(err) => {
                self.complete_chat_runtime_phase(chat_thread_id, "failed")?;
                let _ = self.run_manager.fail_run(&run_id, &err.to_string())?;
                let mut error = new_message(
                    &self.db.workspace_uid,
                    &thread.container_id,
                    MessageLane::Chat,
                    MessageRole::System,
                    MessageType::Error,
                    Some(err.to_string()),
                    Some(chat_thread_id.to_string()),
                );
                error.source_kind = "kernel_bridge".into();
                error.source_ref = "chat_turn".into();
                let error = self.append_chat_projection_message(chat_thread_id, error)?;
                last_cursor_event_id = Some(message_cursor_event_id(&error));
                events.push(sse_message(
                    "chat.error",
                    &self.db.workspace_uid,
                    Some(error),
                ));
                events.push(sse_payload(
                    "chat.complete",
                    &self.db.workspace_uid,
                    last_cursor_event_id,
                    json!({"status": "failed"}),
                ));
            }
        }
        Ok(events)
    }

    fn chat_thread_was_force_closed(&self, chat_thread_id: &str) -> rusqlite::Result<bool> {
        Ok(self
            .kernel
            .read_chat_events(chat_thread_id)
            .map_err(|err| rusqlite::Error::ToSqlConversionFailure(Box::new(err)))?
            .iter()
            .any(|event| event.event_type == "chat_turn_user_forced_closed"))
    }

    fn hydrate_thread(&self, chat_thread_id: &str) -> rusqlite::Result<()> {
        if let Ok(chat_events) = self.kernel.read_chat_events(chat_thread_id) {
            for chat_event in &chat_events {
                if let Some(message) = self.project_chat_event(chat_event, false)? {
                    let _ = self.append_chat_projection_message(chat_thread_id, message)?;
                }
            }
        }
        Ok(())
    }

    fn project_chat_event(
        &self,
        chat_event: &supernova_process_kernel::ChatEvent,
        suppress_final_answer: bool,
    ) -> rusqlite::Result<Option<ContainerMessage>> {
        let Some(mut message) = chat_event_to_message(&self.db.workspace_uid, chat_event) else {
            return Ok(None);
        };
        if message.body_text.is_none() {
            let blob_ref = match chat_event.event_type.as_str() {
                "chat_user_message_recorded" => chat_event
                    .payload
                    .get("message_ref")
                    .and_then(Value::as_str),
                "chat_assistant_answered" => chat_event
                    .payload
                    .get("assistant_content_ref")
                    .and_then(Value::as_str),
                _ => None,
            };
            if let Some(blob_ref) = blob_ref {
                if let Ok(text) = self.kernel.read_chat_blob_text(blob_ref) {
                    message.body_text = Some(text);
                }
            }
        }
        if chat_event.event_type == "chat_user_message_recorded"
            && self.has_pending_chat_user_message(
                &chat_event.chat_thread_id,
                message.body_text.as_deref(),
            )?
        {
            return Ok(None);
        }
        if chat_event.event_type == "chat_assistant_answered"
            && (suppress_final_answer
                || self.has_completed_stream_answer(
                    &chat_event.chat_thread_id,
                    message.body_text.as_deref(),
                )?)
        {
            return Ok(None);
        }
        Ok(Some(message))
    }

    fn has_pending_chat_user_message(
        &self,
        chat_thread_id: &str,
        body_text: Option<&str>,
    ) -> rusqlite::Result<bool> {
        let Some(body_text) = body_text else {
            return Ok(false);
        };
        Ok(self
            .db
            .list_projected_chat_messages_page(chat_thread_id, None, None)?
            .iter()
            .any(|message| {
                message.source_kind == "product_runtime_request"
                    && message.source_ref == "chat_turn_pending"
                    && message.body_text.as_deref() == Some(body_text)
            }))
    }

    fn complete_streamed_assistant_answer(
        &self,
        chat_thread_id: &str,
        final_content: Option<&str>,
    ) -> rusqlite::Result<bool> {
        let mut suppress_final_answer = false;
        for mut message in self
            .db
            .list_projected_chat_messages_page(chat_thread_id, None, None)?
        {
            if message.source_kind != "model_stream"
                || message.role != MessageRole::Assistant
                || message.message_type != MessageType::Text
            {
                continue;
            }
            if message_text_matches(message.body_text.as_deref(), final_content) {
                suppress_final_answer = true;
            }
            if message.status == "completed" {
                continue;
            }
            message.status = "completed".into();
            message.title = Some("DeepSeek answer".into());
            message.body_json = json!({
                "schema": "supernova_model_stream.v1",
                "completed": true,
                "source_ref": message.source_ref,
            });
            advance_message_cursor(&mut message);
            let _ = self.append_chat_projection_message(chat_thread_id, message)?;
        }
        Ok(suppress_final_answer)
    }

    fn has_completed_stream_answer(
        &self,
        chat_thread_id: &str,
        final_content: Option<&str>,
    ) -> rusqlite::Result<bool> {
        let Some(final_content) = final_content else {
            return Ok(false);
        };
        Ok(self
            .db
            .list_projected_chat_messages_page(chat_thread_id, None, None)?
            .iter()
            .any(|message| {
                message.source_kind == "model_stream"
                    && message.role == MessageRole::Assistant
                    && message.message_type == MessageType::Text
                    && message_text_matches(message.body_text.as_deref(), Some(final_content))
            }))
    }

    fn complete_streamed_chat_reasoning(&self, chat_thread_id: &str) -> rusqlite::Result<()> {
        for mut message in self
            .db
            .list_projected_chat_messages_page(chat_thread_id, None, None)?
        {
            if message.source_kind != "model_stream"
                || message.role != MessageRole::Agent
                || message.message_type != MessageType::Reasoning
                || message.status == "completed"
            {
                continue;
            }
            message.status = "completed".into();
            message.title = Some("DeepSeek reasoning".into());
            message.body_json = json!({
                "schema": "supernova_model_stream.v1",
                "completed": true,
                "stream_kind": "reasoning",
                "source_ref": message.source_ref,
            });
            advance_message_cursor(&mut message);
            let _ = self.append_chat_projection_message(chat_thread_id, message)?;
        }
        Ok(())
    }

    fn complete_chat_runtime_phase(
        &self,
        chat_thread_id: &str,
        status: &str,
    ) -> rusqlite::Result<()> {
        for mut message in self.db.list_chat_messages(chat_thread_id)? {
            if message.source_kind != "product_runtime_runtime"
                || message.source_ref != "chat_turn_running"
            {
                continue;
            }
            message.status = status.to_string();
            message.title = Some(
                match status {
                    "completed" => "ChatRuntime completed",
                    "cancelled" | "canceled" => "ChatRuntime forced closed",
                    "failed" => "ChatRuntime failed",
                    _ => "ChatRuntime status updated",
                }
                .into(),
            );
            message.body_json = json!({
                "schema": "supernova_chat_runtime_phase.v1",
                "status": status,
                "source_ref": message.source_ref,
            });
            advance_message_cursor(&mut message);
            let _ = self.db.append_message(message)?;
        }
        Ok(())
    }

    fn append_chat_projection_message(
        &self,
        chat_thread_id: &str,
        message: ContainerMessage,
    ) -> rusqlite::Result<ContainerMessage> {
        match message.source_kind.as_str() {
            "chat_truth" | "model_stream" | "kernel_bridge" => self
                .db
                .append_chat_projection_message(chat_thread_id, message),
            _ => self.db.append_message(message),
        }
    }
}

fn message_text_matches(left: Option<&str>, right: Option<&str>) -> bool {
    let (Some(left), Some(right)) = (left, right) else {
        return false;
    };
    normalize_message_text(left) == normalize_message_text(right)
}

fn normalize_message_text(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn chat_runtime_phase_status(status: &ChatTurnStatus) -> &'static str {
    match status {
        ChatTurnStatus::Answered | ChatTurnStatus::Clarifying | ChatTurnStatus::NeedsTask => {
            "completed"
        }
        ChatTurnStatus::Blocked | ChatTurnStatus::Failed => "failed",
    }
}

fn chat_terminal_status_from_events(events: &[ChatEvent]) -> Option<(String, Option<String>)> {
    let mut terminal: Option<(String, Option<String>)> = None;
    for event in events {
        let next = match event.event_type.as_str() {
            "chat_assistant_answered"
            | "chat_clarification_requested"
            | "chat_needs_task_suggested" => Some(("completed".to_string(), None)),
            "chat_turn_user_forced_closed" => Some(("cancelled".to_string(), None)),
            "chat_turn_failed" | "chat_turn_blocked" => Some((
                "failed".to_string(),
                projection_error_message(&event.payload),
            )),
            _ => None,
        };
        if let Some(next) = next {
            terminal = Some(next);
        }
    }
    terminal
}

fn projection_error_message(value: &Value) -> Option<String> {
    value
        .get("message")
        .and_then(Value::as_str)
        .or_else(|| value.get("reason").and_then(Value::as_str))
        .or_else(|| value.pointer("/error/message").and_then(Value::as_str))
        .or_else(|| value.get("error").and_then(Value::as_str))
        .map(str::trim)
        .filter(|message| !message.is_empty())
        .map(ToString::to_string)
}

#[derive(Debug)]
struct ProductChatStreamSink {
    workspace_uid: String,
    container_id: String,
    chat_thread_id: String,
    projection_shard: ProjectionShardDb,
    states: Mutex<BTreeMap<String, ProductChatStreamState>>,
}

#[derive(Debug)]
struct ProductChatStreamState {
    answer_message_id: String,
    answer_created_at_ms: u128,
    answer_text: String,
    reasoning_message_id: String,
    reasoning_created_at_ms: u128,
    reasoning_text: String,
}

impl ProductChatStreamSink {
    fn new(
        workspace_uid: String,
        container_id: String,
        chat_thread_id: String,
        projection_shard: ProjectionShardDb,
    ) -> Self {
        Self {
            workspace_uid,
            container_id,
            chat_thread_id,
            projection_shard,
            states: Mutex::new(BTreeMap::new()),
        }
    }
}

impl ModelStreamSink for ProductChatStreamSink {
    fn on_model_stream_delta(&self, delta: ModelStreamDelta) {
        if delta.delta.is_empty() {
            return;
        }
        let mut states = self.states.lock().unwrap_or_else(|err| err.into_inner());
        let state = states
            .entry(delta.model_call_id.clone())
            .or_insert_with(|| ProductChatStreamState {
                answer_message_id: format!(
                    "chat_stream_answer_{}_{}",
                    safe_message_component(&self.chat_thread_id),
                    safe_message_component(&delta.model_call_id)
                ),
                answer_created_at_ms: crate::state::workspace_registry::now_ms() as u128,
                answer_text: String::new(),
                reasoning_message_id: format!(
                    "chat_stream_reasoning_{}_{}",
                    safe_message_component(&self.chat_thread_id),
                    safe_message_component(&delta.model_call_id)
                ),
                reasoning_created_at_ms: crate::state::workspace_registry::now_ms() as u128,
                reasoning_text: String::new(),
            });
        let (message_id, created_at_ms, body_text, role, message_type, title, stream_kind) =
            match delta.kind {
                ModelStreamDeltaKind::Answer => {
                    state.answer_text.push_str(&delta.delta);
                    (
                        state.answer_message_id.clone(),
                        state.answer_created_at_ms,
                        state.answer_text.clone(),
                        MessageRole::Assistant,
                        MessageType::Text,
                        "DeepSeek answer streaming",
                        "answer",
                    )
                }
                ModelStreamDeltaKind::Reasoning => {
                    state.reasoning_text.push_str(&delta.delta);
                    (
                        state.reasoning_message_id.clone(),
                        state.reasoning_created_at_ms,
                        state.reasoning_text.clone(),
                        MessageRole::Agent,
                        MessageType::Reasoning,
                        "DeepSeek reasoning streaming",
                        "reasoning",
                    )
                }
            };
        let mut message = new_message(
            &self.workspace_uid,
            &self.container_id,
            MessageLane::Chat,
            role,
            message_type,
            Some(body_text),
            Some(self.chat_thread_id.clone()),
        );
        message.message_id = message_id;
        message.created_at_ms = created_at_ms;
        message.status = "streaming".into();
        message.title = Some(title.into());
        message.source_kind = "model_stream".into();
        message.source_ref = delta.model_call_id.clone();
        message.source_seq = Some(i64::from(delta.sequence));
        message.body_json = json!({
            "schema": "supernova_model_stream.v1",
            "model_call_id": delta.model_call_id,
            "operation": delta.operation.as_str(),
            "stream_kind": stream_kind,
            "delta_sequence": delta.sequence,
        });
        let _ = self.projection_shard.append_message(message);
    }
}

fn safe_message_component(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn event_type_for_message(message: &ContainerMessage) -> &'static str {
    if message.role == MessageRole::User && message.message_type == MessageType::Text {
        return "chat.user.message";
    }
    if message.role == MessageRole::Assistant
        && message.message_type == MessageType::Text
        && message.source_kind == "model_stream"
        && message.status == "streaming"
    {
        return "chat.answer.delta";
    }
    if message.role == MessageRole::Assistant && message.message_type == MessageType::Text {
        return "chat.answer.final";
    }
    match message.message_type {
        MessageType::Reasoning => "chat.reasoning.delta",
        MessageType::ToolCall => "chat.tool.call",
        MessageType::ToolResult => "chat.tool.result",
        MessageType::Approval => "chat.needs_task",
        MessageType::Error => "chat.error",
        MessageType::Phase => "chat.phase",
        _ => "chat.event",
    }
}

fn sse_message(
    event_type: &'static str,
    workspace_uid: &str,
    message: Option<ContainerMessage>,
) -> Event {
    let payload = serde_json::to_value(ChatStreamPayload {
        delta: message.as_ref().and_then(|value| value.body_text.clone()),
        message: message.clone(),
        tool_call: None,
        suggested_task: None,
    })
    .unwrap_or_else(|_| json!({}));
    sse::protocol_message_event(event_type, workspace_uid, payload, message.as_ref())
}

fn sse_payload(
    event_type: &'static str,
    workspace_uid: &str,
    after_event_id: Option<i64>,
    payload: Value,
) -> Event {
    sse::protocol_payload_event(
        event_type,
        workspace_uid,
        local_runtime_protocol::Cursor {
            kind: "message_feed".into(),
            after: None,
            after_event_id,
        },
        payload,
    )
}

fn io_to_sqlite(err: std::io::Error) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(Box::new(err))
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::PathBuf;

    use local_runtime_protocol::ChatTurnStreamRequest;
    use supernova_process_kernel::{ChatTruthStore, ModelOperation};

    use crate::app_paths::AppPaths;
    use crate::services::settings_service::SettingsService;
    use crate::state::workspace_registry::now_ms;

    fn temp_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("supernova_chat_service_{name}_{}", now_ms()));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn test_service(name: &str) -> ChatService {
        let workspace_root = temp_root(&format!("{name}_workspace"));
        let state_root = temp_root(&format!("{name}_state"));
        let provider_root = temp_root(&format!("{name}_provider"));
        let db = ProductDb::open(&state_root, format!("workspace_{name}")).unwrap();
        let kernel = KernelBridge::new(workspace_root.clone(), state_root.clone(), provider_root);
        let context_pack =
            ContextPackService::new(db.clone(), kernel.clone(), workspace_root.clone());
        let run_manager = RunManager::new(db.clone());
        let settings = test_settings(name, &kernel);
        ChatService::new(db, kernel, context_pack, run_manager, settings)
    }

    fn test_settings(name: &str, kernel: &KernelBridge) -> SettingsService {
        SettingsService::new(
            AppPaths {
                app_config_root: temp_root(&format!("{name}_config")),
                app_state_root: temp_root(&format!("{name}_app_state")),
            },
            kernel.clone(),
        )
    }

    #[test]
    fn record_turn_persists_frontend_visible_messages_before_kernel_completion() {
        let service = test_service("preflight_messages");
        let container = service
            .kernel
            .create_container(Some("Chat Container".into()), None, None)
            .unwrap();
        let thread = service
            .create_thread(&container.container_id, Some("Chat".into()))
            .unwrap();

        let _ = service.record_turn(
            &thread.chat_thread_id,
            ChatTurnStreamRequest {
                message: "hello streaming".into(),
                session_id: None,
                context_pack_id: None,
                context_pack: None,
                source_guidance: None,
                model_config: None,
            },
        );

        let messages = service
            .db
            .list_projected_chat_messages_page(&thread.chat_thread_id, None, Some(20))
            .unwrap();
        assert!(messages.iter().any(|message| {
            message.source_kind == "product_runtime_request"
                && message.source_ref == "chat_turn_pending"
                && message.body_text.as_deref() == Some("hello streaming")
        }));
        assert!(messages.iter().any(|message| {
            message.source_kind == "product_runtime_runtime"
                && message.source_ref == "chat_turn_running"
                && message.status != "streaming"
        }));
    }

    #[test]
    fn messages_page_reads_product_db_projection_without_hydrating_chat_truth() {
        let service = test_service("messages_page_projection_only");
        let container = service
            .kernel
            .create_container(Some("Chat Container".into()), None, None)
            .unwrap();
        let thread = service
            .create_thread(&container.container_id, Some("Chat".into()))
            .unwrap();
        service
            .kernel
            .force_close_chat_turn(&thread.chat_thread_id, "projection-only check")
            .unwrap();

        let page = service
            .messages_page(&thread.chat_thread_id, None, Some(20))
            .unwrap();

        assert!(
            page.is_empty(),
            "page reads must not hydrate ChatTruth on the UI hot path"
        );

        let hydrated = service.messages(&thread.chat_thread_id).unwrap();
        assert!(hydrated
            .iter()
            .any(|message| message.source_kind == "chat_truth"));
    }

    #[test]
    fn repair_workspace_projection_recovers_database_locked_chat_failure_error() {
        let workspace_root = temp_root("repair_chat_failure_workspace");
        let state_root = temp_root("repair_chat_failure_state");
        let provider_root = temp_root("repair_chat_failure_provider");
        let db = ProductDb::open(&state_root, "workspace_repair_chat_failure".into()).unwrap();
        let kernel = KernelBridge::new(workspace_root.clone(), state_root.clone(), provider_root);
        let context_pack =
            ContextPackService::new(db.clone(), kernel.clone(), workspace_root.clone());
        let run_manager = RunManager::new(db.clone());
        let settings = test_settings("repair_chat_failure", &kernel);
        let service = ChatService::new(db, kernel, context_pack, run_manager, settings);
        let container = service
            .kernel
            .create_container(Some("Chat Container".into()), None, None)
            .unwrap();
        let thread = service
            .create_thread(&container.container_id, Some("Chat".into()))
            .unwrap();
        let run = service
            .run_manager
            .start_chat_run(&container.container_id, &thread.chat_thread_id)
            .unwrap();
        service
            .run_manager
            .fail_run(&run.run_id, "database is locked")
            .unwrap();
        let mut running = new_message(
            &service.db.workspace_uid,
            &container.container_id,
            MessageLane::Chat,
            MessageRole::System,
            MessageType::Phase,
            Some("Kernel ChatRuntime is processing this turn.".into()),
            Some(thread.chat_thread_id.clone()),
        );
        running.status = "streaming".into();
        running.source_kind = "product_runtime_runtime".into();
        running.source_ref = "chat_turn_running".into();
        service.db.append_message(running).unwrap();
        let mut lock_error = new_message(
            &service.db.workspace_uid,
            &container.container_id,
            MessageLane::Chat,
            MessageRole::System,
            MessageType::Error,
            Some("database is locked".into()),
            Some(thread.chat_thread_id.clone()),
        );
        lock_error.source_kind = "kernel_bridge".into();
        lock_error.source_ref = "chat_turn".into();
        service.db.append_message(lock_error).unwrap();
        let chat_truth = ChatTruthStore::new_with_state_root(&workspace_root, &state_root).unwrap();
        chat_truth
            .append_event(
                &thread.chat_thread_id,
                &container.container_id,
                "chat_turn_failed",
                json!({
                    "schema_version": "supernova_chat_truth.v1",
                    "turn_id": "turn_test",
                    "error": {
                        "code": "DEEPSEEK_HTTP_400",
                        "message": "assistant tool_calls must be followed by tool messages"
                    }
                }),
                None,
            )
            .unwrap();

        let report = service.repair_workspace_projection().unwrap();

        assert_eq!(report.threads_scanned, 1);
        assert_eq!(report.threads_with_truth, 1);
        assert_eq!(report.runs_repaired, 1);
        assert_eq!(report.database_locked_messages_downgraded, 1);
        let repaired_run = service.db.get_run(&run.run_id).unwrap();
        assert_eq!(repaired_run.status, "failed");
        assert_eq!(
            repaired_run.error_message.as_deref(),
            Some("assistant tool_calls must be followed by tool messages")
        );
        let messages = service
            .db
            .list_projected_chat_messages_page(&thread.chat_thread_id, None, Some(20))
            .unwrap();
        assert!(messages.iter().any(|message| {
            message.source_kind == "chat_truth"
                && message.message_type == MessageType::Error
                && message.body_text.as_deref()
                    == Some("assistant tool_calls must be followed by tool messages")
        }));
        let recovered = messages
            .iter()
            .find(|message| {
                message.source_kind == "kernel_bridge" && message.source_ref == "chat_turn"
            })
            .unwrap();
        assert_eq!(recovered.message_type, MessageType::Phase);
        assert_eq!(
            recovered.body_json["status"].as_str(),
            Some("projection_error_recovered")
        );
    }

    #[test]
    fn model_stream_sink_projects_answer_chunks_as_single_live_message() {
        let service = test_service("model_stream_sink");
        let container = service
            .kernel
            .create_container(Some("Chat Container".into()), None, None)
            .unwrap();
        let thread = service
            .create_thread(&container.container_id, Some("Chat".into()))
            .unwrap();
        let sink = ProductChatStreamSink::new(
            service.db.workspace_uid.clone(),
            container.container_id.clone(),
            thread.chat_thread_id.clone(),
            service
                .db
                .ensure_chat_projection_shard(&container.container_id, &thread.chat_thread_id)
                .unwrap(),
        );

        sink.on_model_stream_delta(ModelStreamDelta {
            model_call_id: "call/stream-1".into(),
            operation: ModelOperation::ChatTurn,
            kind: ModelStreamDeltaKind::Answer,
            sequence: 1,
            delta: "Hel".into(),
        });
        sink.on_model_stream_delta(ModelStreamDelta {
            model_call_id: "call/stream-1".into(),
            operation: ModelOperation::ChatTurn,
            kind: ModelStreamDeltaKind::Answer,
            sequence: 2,
            delta: "lo".into(),
        });
        sink.on_model_stream_delta(ModelStreamDelta {
            model_call_id: "call/stream-1".into(),
            operation: ModelOperation::ChatTurn,
            kind: ModelStreamDeltaKind::Reasoning,
            sequence: 1,
            delta: "Thinking".into(),
        });

        let messages = service
            .db
            .list_projected_chat_messages_page(&thread.chat_thread_id, None, Some(20))
            .unwrap();
        let stream_messages: Vec<_> = messages
            .iter()
            .filter(|message| message.source_kind == "model_stream")
            .collect();
        assert_eq!(stream_messages.len(), 2);
        assert!(service
            .db
            .list_chat_messages(&thread.chat_thread_id)
            .unwrap()
            .iter()
            .all(|message| message.source_kind != "model_stream"));
        let stream_message = stream_messages
            .iter()
            .copied()
            .find(|message| message.role == MessageRole::Assistant)
            .unwrap();
        assert_eq!(stream_message.body_text.as_deref(), Some("Hello"));
        assert_eq!(stream_message.status, "streaming");
        assert_eq!(stream_message.source_seq, Some(2));
        assert_eq!(event_type_for_message(stream_message), "chat.answer.delta");
        let reasoning_message = stream_messages
            .iter()
            .copied()
            .find(|message| message.message_type == MessageType::Reasoning)
            .unwrap();
        assert_eq!(reasoning_message.body_text.as_deref(), Some("Thinking"));
        assert_eq!(
            event_type_for_message(reasoning_message),
            "chat.reasoning.delta"
        );
        let streaming_cursor = message_cursor_event_id(stream_message);

        let suppress_final = service
            .complete_streamed_assistant_answer(&thread.chat_thread_id, Some("Hello"))
            .unwrap();
        assert!(suppress_final);
        service
            .complete_streamed_chat_reasoning(&thread.chat_thread_id)
            .unwrap();

        let completed_messages = service
            .db
            .list_projected_chat_messages_page(&thread.chat_thread_id, None, Some(20))
            .unwrap();
        let completed_stream_messages: Vec<_> = completed_messages
            .iter()
            .filter(|message| message.source_kind == "model_stream")
            .collect();
        assert_eq!(completed_stream_messages.len(), 2);
        let completed = completed_stream_messages
            .iter()
            .copied()
            .find(|message| message.role == MessageRole::Assistant)
            .unwrap();
        assert_eq!(completed.body_text.as_deref(), Some("Hello"));
        assert_eq!(completed.status, "completed");
        assert_eq!(completed.title.as_deref(), Some("DeepSeek answer"));
        assert_eq!(event_type_for_message(completed), "chat.answer.final");
        assert!(
            message_cursor_event_id(completed) > streaming_cursor,
            "completed stream replacement must advance the message-feed cursor so SSE clients receive it"
        );
        assert!(service
            .has_completed_stream_answer(&thread.chat_thread_id, Some("Hello"))
            .unwrap());
        let completed_reasoning = completed_stream_messages
            .iter()
            .copied()
            .find(|message| message.message_type == MessageType::Reasoning)
            .unwrap();
        assert_eq!(completed_reasoning.status, "completed");
        assert_eq!(
            completed_reasoning.title.as_deref(),
            Some("DeepSeek reasoning")
        );
    }
}
