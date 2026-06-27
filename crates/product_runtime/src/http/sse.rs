use axum::response::sse::{Event, Sse};
use axum::response::IntoResponse;
use local_runtime_protocol::{
    ContainerMessage, Cursor, MessageLane, MessageRole, MessageType, ProtocolEvent,
};
use serde_json::json;
use serde_json::Value;
use tokio::sync::mpsc;
use tokio::time::{interval, interval_at, Duration, Instant};

use crate::state::message_feed::message_cursor_event_id;

pub type RuntimeSse =
    Sse<tokio_stream::wrappers::ReceiverStream<Result<Event, std::convert::Infallible>>>;

pub fn single_event(event: Event) -> impl IntoResponse {
    let (tx, rx) = mpsc::channel::<Result<Event, std::convert::Infallible>>(1);
    let _ = tx.try_send(Ok(event));
    Sse::new(tokio_stream::wrappers::ReceiverStream::new(rx))
}

pub fn event_batch(events: Vec<Event>) -> impl IntoResponse {
    let (tx, rx) = mpsc::channel::<Result<Event, std::convert::Infallible>>(events.len().max(1));
    for event in events {
        let _ = tx.try_send(Ok(event));
    }
    Sse::new(tokio_stream::wrappers::ReceiverStream::new(rx))
}

pub fn live_message_stream<Run, Load>(
    workspace_uid: String,
    initial_after_event_id: Option<i64>,
    run: Run,
    mut load_messages: Load,
    complete_event_type: &'static str,
    failed_event_type: &'static str,
    heartbeat_event_type: &'static str,
) -> RuntimeSse
where
    Run: FnOnce() -> Result<(), String> + Send + 'static,
    Load: FnMut(Option<i64>) -> Result<Vec<ContainerMessage>, String> + Send + 'static,
{
    let (tx, rx) = mpsc::channel::<Result<Event, std::convert::Infallible>>(64);
    tokio::spawn(async move {
        let (done_tx, mut done_rx) = tokio::sync::oneshot::channel();
        tokio::task::spawn_blocking(move || {
            let _ = done_tx.send(run());
        });

        let mut after_event_id = initial_after_event_id;
        let mut worker_done = false;
        let mut worker_result: Option<Result<(), String>> = None;
        let mut ticker = interval(Duration::from_millis(200));
        let mut heartbeat = interval_at(
            Instant::now() + Duration::from_secs(2),
            Duration::from_secs(2),
        );

        loop {
            let mut heartbeat_due = false;
            tokio::select! {
                result = &mut done_rx, if !worker_done => {
                    worker_done = true;
                    worker_result = Some(result.unwrap_or_else(|err| Err(format!("runtime worker aborted: {err}"))));
                }
                _ = ticker.tick() => {}
                _ = heartbeat.tick() => {
                    heartbeat_due = true;
                }
            }

            let mut emitted_messages = false;
            match load_messages(after_event_id) {
                Ok(messages) => {
                    for message in messages {
                        emitted_messages = true;
                        after_event_id = Some(message_cursor_event_id(&message));
                        let payload = json!({
                            "delta": message.body_text.clone(),
                            "message": message.clone(),
                            "tool_call": null,
                            "suggested_task": null
                        });
                        if !send_event(
                            &tx,
                            protocol_message_event(
                                event_type_for_message(&message),
                                &workspace_uid,
                                payload,
                                Some(&message),
                            ),
                        )
                        .await
                        {
                            return;
                        }
                    }
                }
                Err(err) => {
                    let cursor = Cursor {
                        kind: "message_feed".into(),
                        after: None,
                        after_event_id,
                    };
                    let event = protocol_payload_event(
                        failed_event_type,
                        &workspace_uid,
                        cursor,
                        json!({"status": "failed", "message": err}),
                    );
                    let _ = send_event(&tx, event).await;
                    return;
                }
            }

            if heartbeat_due && !emitted_messages && !worker_done {
                let cursor = Cursor {
                    kind: "message_feed".into(),
                    after: None,
                    after_event_id,
                };
                let event = protocol_payload_event(
                    heartbeat_event_type,
                    &workspace_uid,
                    cursor,
                    json!({"status": "running", "message": null}),
                );
                if !send_event(&tx, event).await {
                    return;
                }
            }

            if let Some(result) = worker_result.take() {
                let (event_type, payload) = match result {
                    Ok(()) => (complete_event_type, json!({"status": "closed"})),
                    Err(err) => (
                        failed_event_type,
                        json!({"status": "failed", "message": err}),
                    ),
                };
                let cursor = Cursor {
                    kind: "message_feed".into(),
                    after: None,
                    after_event_id,
                };
                let event = protocol_payload_event(event_type, &workspace_uid, cursor, payload);
                let _ = send_event(&tx, event).await;
                return;
            }
        }
    });
    Sse::new(tokio_stream::wrappers::ReceiverStream::new(rx))
}

async fn send_event(
    tx: &mpsc::Sender<Result<Event, std::convert::Infallible>>,
    event: Event,
) -> bool {
    tx.send(Ok(event)).await.is_ok()
}

pub fn protocol_payload_event(
    event_type: impl Into<String>,
    workspace_uid: &str,
    cursor: Cursor,
    payload: Value,
) -> Event {
    let event_type = event_type.into();
    let event_id = crate::state::product_db::next_id("evt");
    let protocol_event = ProtocolEvent::new(
        event_id,
        event_type.clone(),
        cursor,
        workspace_uid.to_string(),
        payload,
    );
    Event::default()
        .event(event_type)
        .json_data(protocol_event)
        .unwrap_or_else(|_| Event::default().event("protocol.error").data("{}"))
}

pub fn protocol_message_event(
    event_type: impl Into<String>,
    workspace_uid: &str,
    payload: Value,
    message: Option<&ContainerMessage>,
) -> Event {
    let event_type = event_type.into();
    let cursor = message.map(message_cursor).unwrap_or_else(|| Cursor {
        kind: "message_feed".into(),
        after: None,
        after_event_id: None,
    });
    let event_id = cursor
        .after_event_id
        .map(|id| format!("msg_evt_{id}"))
        .unwrap_or_else(|| crate::state::product_db::next_id("evt"));
    let mut protocol_event = ProtocolEvent::new(
        event_id,
        event_type.clone(),
        cursor,
        workspace_uid.to_string(),
        payload,
    );
    if let Some(message) = message {
        protocol_event.container_id = Some(message.container_id.clone());
        protocol_event.chat_thread_id = message.chat_thread_id.clone();
        protocol_event.task_id = message.task_id.clone();
        protocol_event.job_id = message.job_id.clone();
    }
    Event::default()
        .event(event_type)
        .json_data(protocol_event)
        .unwrap_or_else(|_| Event::default().event("protocol.error").data("{}"))
}

pub fn message_cursor(message: &ContainerMessage) -> Cursor {
    Cursor {
        kind: "message_feed".into(),
        after: Some(message.sort_key.clone()),
        after_event_id: Some(message_cursor_event_id(message)),
    }
}

pub fn event_type_for_message(message: &ContainerMessage) -> &'static str {
    match &message.lane {
        MessageLane::Chat => {
            if message.role == MessageRole::User && message.message_type == MessageType::Text {
                "chat.user.message"
            } else if message.role == MessageRole::Assistant
                && message.message_type == MessageType::Text
                && message.source_kind == "model_stream"
                && message.status == "streaming"
            {
                "chat.answer.delta"
            } else if message.role == MessageRole::Assistant
                && message.message_type == MessageType::Text
            {
                "chat.answer.final"
            } else if message.message_type == MessageType::Reasoning {
                "chat.reasoning.delta"
            } else if message.message_type == MessageType::ToolCall {
                "chat.tool.call"
            } else if message.message_type == MessageType::ToolResult {
                "chat.tool.result"
            } else if message.message_type == MessageType::Approval {
                "chat.needs_task"
            } else if message.message_type == MessageType::Error {
                "chat.error"
            } else if message.message_type == MessageType::Phase {
                "chat.phase"
            } else {
                "chat.answer.final"
            }
        }
        MessageLane::Task => {
            if message.role == MessageRole::User && message.message_type == MessageType::Text {
                "task.started"
            } else if message.role == MessageRole::Assistant
                && message.message_type == MessageType::Text
                && message.title.as_deref() == Some("Model message")
            {
                "task.message"
            } else {
                match &message.message_type {
                    MessageType::Reasoning => "task.reasoning.delta",
                    MessageType::ToolCall => "task.tool.call",
                    MessageType::ToolResult => "task.tool.result",
                    MessageType::Approval => "task.approval.required",
                    MessageType::Artifact => "task.artifact.ready",
                    MessageType::Error => "task.error",
                    MessageType::Text => "task.complete",
                    _ => "task.phase",
                }
            }
        }
        MessageLane::Runtime => "runtime.message",
    }
}
