use local_runtime_protocol::{
    ChatThreadRecord, ContainerMessage, MessageLane, MessageRole, MessageType, TaskRecord,
};
use serde_json::{json, Value};
use supernova_process_kernel::{
    ChatEvent, ChatThread, ContainerTimelineItem, ProcessEvent, TaskAgentRunResult,
};

use crate::state::workspace_registry::now_ms;

pub fn chat_thread_to_record(thread: ChatThread) -> ChatThreadRecord {
    ChatThreadRecord {
        chat_thread_id: thread.chat_thread_id,
        container_id: thread.container_id,
        title: thread.title.unwrap_or_else(|| "Chat".to_string()),
        created_at_ms: thread.created_at_ms.max(0) as u128,
        updated_at_ms: thread.updated_at_ms.max(0) as u128,
    }
}
pub fn timeline_task_to_record(container_id: &str, item: ContainerTimelineItem) -> TaskRecord {
    TaskRecord {
        task_id: item.ref_id.clone(),
        container_id: container_id.to_string(),
        job_id: Some(item.ref_id),
        title: item.title.unwrap_or_else(|| "Task".to_string()),
        goal: String::new(),
        status: item.status,
        badges: Default::default(),
        created_at_ms: item.created_at_ms.max(0) as u128,
        updated_at_ms: item.updated_at_ms.max(0) as u128,
    }
}

pub fn task_result_to_record(
    container_id: &str,
    goal: &str,
    result: &TaskAgentRunResult,
) -> TaskRecord {
    let now = now_ms() as u128;
    TaskRecord {
        task_id: result.job_id.clone(),
        container_id: container_id.to_string(),
        job_id: Some(result.job_id.clone()),
        title: goal.chars().take(80).collect(),
        goal: goal.to_string(),
        status: result.status.clone(),
        badges: Default::default(),
        created_at_ms: now,
        updated_at_ms: now,
    }
}

pub fn chat_event_to_message(workspace_uid: &str, event: &ChatEvent) -> Option<ContainerMessage> {
    let (role, message_type, title, body_text, body_json) = match event.event_type.as_str() {
        "chat_turn_started" => (
            MessageRole::System,
            MessageType::Phase,
            Some("Chat turn started".to_string()),
            Some("Chat turn started".to_string()),
            event.payload.clone(),
        ),
        "chat_user_message_recorded" => (
            MessageRole::User,
            MessageType::Text,
            Some("User message".to_string()),
            None,
            event.payload.clone(),
        ),
        "chat_reference_sources_attached" => (
            MessageRole::System,
            MessageType::Phase,
            Some("Reference sources selected".to_string()),
            Some("Reference source guidance attached. Contents are not preloaded.".to_string()),
            event.payload.clone(),
        ),
        "chat_model_config_bound" => (
            MessageRole::System,
            MessageType::Phase,
            Some("Model config bound".to_string()),
            Some(
                model_config_summary(&event.payload)
                    .unwrap_or_else(|| "Model config bound".to_string()),
            ),
            event.payload.clone(),
        ),
        "chat_context_pack_loaded" | "chat_context_window_checked" => (
            MessageRole::System,
            MessageType::Phase,
            Some(humanize_event(&event.event_type)),
            Some(humanize_event(&event.event_type)),
            event.payload.clone(),
        ),
        "chat_model_call_started" | "chat_model_call_completed" => return None,
        "chat_provider_tool_call_decoded" => (
            MessageRole::Tool,
            MessageType::ToolCall,
            Some("Tool call".to_string()),
            Some(tool_name(&event.payload).unwrap_or_else(|| "Tool call".to_string())),
            event.payload.clone(),
        ),
        "chat_provider_tool_result_appended" | "chat_readonly_capability_receipt" => (
            MessageRole::Tool,
            MessageType::ToolResult,
            Some("Tool result".to_string()),
            Some("Tool result received".to_string()),
            event.payload.clone(),
        ),
        "chat_assistant_answered" => (
            MessageRole::Assistant,
            MessageType::Text,
            Some("Answer".to_string()),
            None,
            event.payload.clone(),
        ),
        "chat_needs_task_suggested" => (
            MessageRole::Assistant,
            MessageType::Approval,
            Some("Needs task".to_string()),
            suggested_task_text(&event.payload),
            event.payload.clone(),
        ),
        "chat_clarification_requested" => (
            MessageRole::Assistant,
            MessageType::Text,
            Some("Clarification".to_string()),
            event
                .payload
                .get("question")
                .and_then(Value::as_str)
                .map(ToString::to_string),
            event.payload.clone(),
        ),
        "chat_turn_blocked" | "chat_turn_failed" => (
            MessageRole::System,
            MessageType::Error,
            Some(humanize_event(&event.event_type)),
            reason_value(&event.payload),
            event.payload.clone(),
        ),
        "chat_turn_user_forced_closed" => (
            MessageRole::System,
            MessageType::Phase,
            Some("用户强制关闭".to_string()),
            event
                .payload
                .get("reason")
                .and_then(Value::as_str)
                .map(|reason| format!("用户已强制关闭该 CHAT。{reason}"))
                .or_else(|| Some("用户已强制关闭该 CHAT。".to_string())),
            event.payload.clone(),
        ),
        _ => return None,
    };
    Some(ContainerMessage {
        message_id: format!("chat_evt_{}_{}", event.chat_thread_id, event.event_seq),
        workspace_uid: workspace_uid.to_string(),
        container_id: event.container_id.clone(),
        lane: MessageLane::Chat,
        role,
        message_type,
        status: "completed".into(),
        title,
        body_text,
        body_json,
        card_json: json!({}),
        chat_thread_id: Some(event.chat_thread_id.clone()),
        task_id: None,
        job_id: None,
        source_kind: "chat_truth".into(),
        source_ref: event.event_id.clone(),
        source_seq: Some(event.event_seq as i64),
        created_at_ms: event.created_at_ms.max(0) as u128,
        updated_at_ms: event.created_at_ms.max(0) as u128,
        sort_key: format!("{:020}_{:010}", event.created_at_ms.max(0), event.event_seq),
    })
}

pub fn process_event_to_message(
    workspace_uid: &str,
    container_id: &str,
    job_id: &str,
    event: &ProcessEvent,
) -> Option<ContainerMessage> {
    let (role, message_type, title, body_text) = match event.event_type.as_str() {
        "process_started"
        | "model_config_bound"
        | "task_reference_sources_attached"
        | "task_artifact_destination_guidance_attached"
        | "context_window_checked"
        | "context_window_advisory"
        | "context_window_compaction_required"
        | "context_window_checkpoint_created"
        | "context_window_reestimate_completed"
        | "task_agent_session_started"
        | "task_agent_observation_recorded" => (
            MessageRole::Agent,
            MessageType::Phase,
            Some(humanize_event(&event.event_type)),
            Some(humanize_event(&event.event_type)),
        ),
        "model_call_started" | "model_call_completed" => (
            MessageRole::Agent,
            MessageType::Phase,
            Some(humanize_event(&event.event_type)),
            Some(model_call_event_body(&event.data, &event.event_type)),
        ),
        "provider_native_assistant_content_yielded" => (
            MessageRole::Assistant,
            MessageType::Text,
            Some("Model message".to_string()),
            provider_native_assistant_content_body(&event.data),
        ),
        "provider_tool_call_requested" | "provider_tool_call_decoded" | "capability_dispatched" => {
            (
                MessageRole::Tool,
                MessageType::ToolCall,
                Some("Tool call".to_string()),
                Some(tool_event_body(&event.data, "requested")),
            )
        }
        "provider_tool_result_recorded"
        | "capability_completed"
        | "capability_receipt"
        | "capability_receipt_recorded" => (
            MessageRole::Tool,
            MessageType::ToolResult,
            Some(tool_result_title(&event.data)),
            Some(tool_event_body(&event.data, "completed")),
        ),
        "approval_requested"
        | "preview_ready"
        | "preview_tx_created"
        | "preview_created"
        | "provider_tool_call_waiting_approval" => return None,
        "artifact_ready" | "artifact_verified" => (
            MessageRole::Agent,
            MessageType::Artifact,
            Some(humanize_event(&event.event_type)),
            Some(humanize_event(&event.event_type)),
        ),
        "process_completed" | "task_completed" => (
            MessageRole::Agent,
            MessageType::Text,
            Some("Task complete".to_string()),
            Some("Task completed".to_string()),
        ),
        "completion_statement_recorded" => (
            MessageRole::Agent,
            MessageType::Text,
            Some("Task final answer".to_string()),
            Some(completion_message_body(&event.data)),
        ),
        "job_completed" => (
            MessageRole::Agent,
            MessageType::Phase,
            Some("Task completed".to_string()),
            Some("Task completed.".to_string()),
        ),
        "job_waiting_user" => (
            MessageRole::Assistant,
            MessageType::Text,
            Some("Clarification requested".to_string()),
            clarification_message_body(&event.data),
        ),
        "job_cancelled" => (
            MessageRole::System,
            MessageType::Phase,
            Some("用户强制关闭".to_string()),
            Some(user_forced_close_body(&event.data)),
        ),
        "model_call_attempt_failed"
        | "model_call_failed"
        | "model_call_blocked"
        | "provider_tool_call_recoverable_error"
        | "provider_tool_protocol_error"
        | "capability_blocked"
        | "process_action_blocked"
        | "completion_blocked"
        | "closure_gate_blocked"
        | "task_agent_session_failed"
        | "task_agent_session_blocked" => return None,
        "job_failed" | "job_blocked" => (
            MessageRole::System,
            MessageType::Error,
            Some(humanize_event(&event.event_type)),
            Some(error_event_body(&event.event_type, &event.data)),
        ),
        value if value.contains("failed") || value.contains("blocked") => (
            MessageRole::System,
            MessageType::Phase,
            Some(humanize_event(value)),
            Some(error_event_body(value, &event.data)),
        ),
        _ => return None,
    };
    let body_json = event.data.clone();
    let card_json = json!({});

    Some(ContainerMessage {
        message_id: format!("task_evt_{}_{}", job_id, event.event_id),
        workspace_uid: workspace_uid.to_string(),
        container_id: container_id.to_string(),
        lane: MessageLane::Task,
        role,
        message_type,
        status: process_event_message_status(&event.event_type).into(),
        title,
        body_text,
        body_json,
        card_json,
        chat_thread_id: None,
        task_id: Some(job_id.to_string()),
        job_id: Some(job_id.to_string()),
        source_kind: "process_truth".into(),
        source_ref: event.event_id.to_string(),
        source_seq: Some(event.event_id as i64),
        created_at_ms: event.timestamp_ms,
        updated_at_ms: event.timestamp_ms,
        sort_key: format!("{:020}_{:010}", event.timestamp_ms, event.event_id),
    })
}

fn process_event_message_status(event_type: &str) -> &'static str {
    match event_type {
        "model_call_started"
        | "provider_tool_call_requested"
        | "provider_tool_call_decoded"
        | "capability_dispatched" => "streaming",
        "model_call_blocked" => "blocked",
        "job_failed" | "job_blocked" => "failed",
        "job_waiting_user" => "waiting_user",
        "job_cancelled" => "cancelled",
        value if value.contains("failed") || value.contains("blocked") => "completed",
        _ => "completed",
    }
}

fn clarification_message_body(payload: &Value) -> Option<String> {
    payload
        .get("question")
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .or_else(|| {
            payload
                .get("reason")
                .and_then(Value::as_str)
                .map(ToString::to_string)
        })
}

fn user_forced_close_body(payload: &Value) -> String {
    payload
        .get("reason")
        .and_then(Value::as_str)
        .map(|reason| format!("用户已强制关闭该 TASK。{reason}"))
        .unwrap_or_else(|| "用户已强制关闭该 TASK。".to_string())
}

fn humanize_event(value: &str) -> String {
    value.replace('_', " ")
}

fn model_call_event_body(payload: &Value, event_type: &str) -> String {
    let operation = string_at(payload, &["operation"])
        .or_else(|| string_at(payload, &["data", "operation"]))
        .or_else(|| string_at(payload, &["capability_id"]))
        .or_else(|| string_at(payload, &["data", "capability_id"]))
        .unwrap_or_else(|| "model operation".to_string());
    let model = string_at(payload, &["model"])
        .or_else(|| string_at(payload, &["data", "model"]))
        .unwrap_or_else(|| "model".to_string());
    let provider = string_at(payload, &["provider"])
        .or_else(|| string_at(payload, &["data", "provider"]))
        .unwrap_or_else(|| "provider".to_string());
    let status = status_value(payload).unwrap_or_else(|| humanize_event(event_type));
    format!("{operation}: {status}\nProvider: {provider}\nModel: {model}")
}

fn completion_message_body(payload: &Value) -> String {
    let mut lines = Vec::new();
    if let Some(statement) = payload.get("completion_statement").and_then(Value::as_str) {
        lines.push(statement.to_string());
    } else if let Some(status) = payload.get("status").and_then(Value::as_str) {
        lines.push(format!("Task {status}."));
    } else {
        lines.push("Task completed.".to_string());
    }

    let claimed_artifacts = payload
        .get("claimed_artifacts")
        .or_else(|| payload.get("artifacts"))
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_str).collect::<Vec<_>>())
        .unwrap_or_default();
    if !claimed_artifacts.is_empty() {
        lines.push(String::new());
        lines.push("Artifacts:".to_string());
        for artifact in claimed_artifacts {
            lines.push(format!("- {artifact}"));
        }
    }

    let limitations = payload
        .get("known_limitations")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_str).collect::<Vec<_>>())
        .unwrap_or_default();
    if !limitations.is_empty() {
        lines.push(String::new());
        lines.push("Known limitations:".to_string());
        for limitation in limitations {
            lines.push(format!("- {limitation}"));
        }
    }

    lines.join("\n")
}

fn provider_native_assistant_content_body(payload: &Value) -> Option<String> {
    payload
        .get("assistant_content")
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            payload
                .get("output_ref")
                .and_then(Value::as_str)
                .map(|output_ref| format!("Model message content: {output_ref}"))
        })
}

fn tool_result_title(payload: &Value) -> String {
    let capability = capability_name(payload).unwrap_or_else(|| "Tool result".to_string());
    let status = status_value(payload).unwrap_or_else(|| "recorded".to_string());
    format!("{capability}: {status}")
}

fn tool_event_body(payload: &Value, fallback_status: &str) -> String {
    let capability = capability_name(payload).unwrap_or_else(|| "Tool".to_string());
    let status = status_value(payload).unwrap_or_else(|| fallback_status.to_string());
    let mut lines = vec![format!("{capability}: {status}")];
    if let Some(reason) = reason_value(payload) {
        lines.push(format!("Reason: {reason}"));
    }
    if let Some(preview_id) =
        string_at(payload, &["preview_id"]).or_else(|| string_at(payload, &["data", "preview_id"]))
    {
        lines.push(format!("Preview: {preview_id}"));
    }
    if let Some(receipt_ref) = string_at(payload, &["receipt_ref"])
        .or_else(|| string_at(payload, &["data", "receipt_ref"]))
    {
        lines.push(format!("Receipt: {receipt_ref}"));
    }
    lines.join("\n")
}

fn error_event_body(event_type: &str, payload: &Value) -> String {
    let mut lines = vec![humanize_event(event_type)];
    if let Some(capability) = capability_name(payload) {
        lines.push(format!("Capability: {capability}"));
    }
    if let Some(reason) = reason_value(payload) {
        lines.push(format!("Reason: {reason}"));
    }
    if let Some(status) = status_value(payload) {
        lines.push(format!("Status: {status}"));
    }
    lines.join("\n")
}

fn capability_name(payload: &Value) -> Option<String> {
    string_at(payload, &["capability_id"])
        .or_else(|| string_at(payload, &["provider_tool_name"]))
        .or_else(|| string_at(payload, &["tool_name"]))
        .or_else(|| string_at(payload, &["data", "capability_id"]))
        .or_else(|| string_at(payload, &["data", "provider_tool_name"]))
        .or_else(|| string_at(payload, &["data", "tool_name"]))
}

fn status_value(payload: &Value) -> Option<String> {
    string_at(payload, &["status"]).or_else(|| string_at(payload, &["data", "status"]))
}

fn reason_value(payload: &Value) -> Option<String> {
    string_at(payload, &["reason"])
        .or_else(|| string_at(payload, &["message"]))
        .or_else(|| string_at(payload, &["error"]))
        .or_else(|| string_at(payload, &["error", "message"]))
        .or_else(|| string_at(payload, &["data", "reason"]))
        .or_else(|| string_at(payload, &["data", "message"]))
        .or_else(|| string_at(payload, &["data", "error"]))
        .or_else(|| string_at(payload, &["data", "error", "message"]))
        .or_else(|| {
            payload
                .get("data")
                .and_then(|data| data.get("invalid_fields"))
                .map(|value| format!("invalid fields: {value}"))
        })
}

fn string_at(payload: &Value, path: &[&str]) -> Option<String> {
    let mut current = payload;
    for segment in path {
        current = current.get(*segment)?;
    }
    current.as_str().map(ToString::to_string)
}

fn model_config_summary(payload: &Value) -> Option<String> {
    let config = payload.get("effective_config")?;
    let model = config
        .get("model_id")
        .and_then(Value::as_str)
        .unwrap_or("model");
    let reasoning = config
        .pointer("/thinking/reasoning_effort")
        .and_then(Value::as_str)
        .unwrap_or("reasoning");
    let max_output = config
        .get("max_output_tokens")
        .and_then(Value::as_u64)
        .map(|value| value.to_string())
        .unwrap_or_else(|| "default".to_string());
    Some(format!(
        "Model config bound: {model}, {reasoning}, max output {max_output}"
    ))
}

fn tool_name(payload: &Value) -> Option<String> {
    payload
        .get("tool_name")
        .or_else(|| payload.get("capability_id"))
        .and_then(Value::as_str)
        .map(ToString::to_string)
}

fn suggested_task_text(payload: &Value) -> Option<String> {
    payload
        .get("suggested_task")
        .and_then(|value| value.get("goal"))
        .and_then(Value::as_str)
        .map(|goal| format!("Suggested Task: {goal}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waiting_approval_process_event_is_not_projected_as_task_message() {
        let event = ProcessEvent {
            schema_version: "supernova.process_truth.v1".into(),
            event_id: 131,
            timestamp_ms: 1781030504365,
            job_id: "job_test".into(),
            pid: Some("pid_test".into()),
            event_type: "provider_tool_call_waiting_approval".into(),
            data: json!({
                "capability_id": "os.write_artifact",
                "preview_id": "preview_1",
            }),
        };

        let message = process_event_to_message("workspace_1", "container_1", "job_test", &event);

        assert!(message.is_none());
    }

    #[test]
    fn chat_model_call_lifecycle_events_do_not_project_to_main_message_feed() {
        let event = ChatEvent {
            event_id: "chat_evt_1".into(),
            event_seq: 7,
            created_at_ms: 1781030504365,
            container_id: "container_1".into(),
            chat_thread_id: "chat_1".into(),
            event_type: "chat_model_call_started".into(),
            payload: json!({
                "operation": "model.chat_turn",
                "provider": "deepseek",
            }),
            blob_ref: None,
        };

        let message = chat_event_to_message("workspace_1", &event);

        assert!(message.is_none());
    }

    #[test]
    fn legacy_chat_task_suggested_event_does_not_project_to_main_message_feed() {
        let event = ChatEvent {
            event_id: "chat_evt_legacy_task_suggested".into(),
            event_seq: 7,
            created_at_ms: 1781030504365,
            container_id: "container_1".into(),
            chat_thread_id: "chat_1".into(),
            event_type: "chat_task_suggested".into(),
            payload: json!({
                "schema_version": "supernova_chat_truth.v1",
                "suggested_task": {
                    "goal": "create project"
                }
            }),
            blob_ref: None,
        };

        let message = chat_event_to_message("workspace_1", &event);

        assert!(message.is_none());
    }

    #[test]
    fn canonical_chat_needs_task_event_projects_once() {
        let event = ChatEvent {
            event_id: "chat_evt_needs_task".into(),
            event_seq: 8,
            created_at_ms: 1781030504366,
            container_id: "container_1".into(),
            chat_thread_id: "chat_1".into(),
            event_type: "chat_needs_task_suggested".into(),
            payload: json!({
                "schema_version": "supernova_chat_truth.v1",
                "suggested_task": {
                    "goal": "create project"
                }
            }),
            blob_ref: None,
        };

        let message = chat_event_to_message("workspace_1", &event).unwrap();

        assert_eq!(message.message_id, "chat_evt_chat_1_8");
        assert_eq!(message.message_type, MessageType::Approval);
        assert_eq!(
            message.body_text.as_deref(),
            Some("Suggested Task: create project")
        );
    }

    #[test]
    fn chat_failed_projects_nested_provider_error_message() {
        let event = ChatEvent {
            event_id: "chat_evt_failed".into(),
            event_seq: 8,
            created_at_ms: 1781030504366,
            container_id: "container_1".into(),
            chat_thread_id: "chat_1".into(),
            event_type: "chat_turn_failed".into(),
            payload: json!({
                "schema_version": "supernova_chat_truth.v1",
                "turn_id": "turn_1",
                "error": {
                    "code": "DEEPSEEK_HTTP_400",
                    "message": "assistant tool_calls must be followed by tool messages"
                }
            }),
            blob_ref: None,
        };

        let message = chat_event_to_message("workspace_1", &event).unwrap();

        assert_eq!(message.message_type, MessageType::Error);
        assert_eq!(
            message.body_text.as_deref(),
            Some("assistant tool_calls must be followed by tool messages")
        );
    }

    #[test]
    fn model_call_started_projects_running_task_phase() {
        let event = ProcessEvent {
            schema_version: "supernova.process_truth.v1".into(),
            event_id: 132,
            timestamp_ms: 1781030504366,
            job_id: "job_test".into(),
            pid: Some("pid_test".into()),
            event_type: "model_call_started".into(),
            data: json!({
                "operation": "model.decide_next_action",
                "provider": "deepseek",
                "model": "deepseek-chat",
            }),
        };

        let message =
            process_event_to_message("workspace_1", "container_1", "job_test", &event).unwrap();

        assert_eq!(message.message_type, MessageType::Phase);
        assert_eq!(message.status, "streaming");
        assert_eq!(message.title.as_deref(), Some("model call started"));
        assert!(message
            .body_text
            .unwrap_or_default()
            .contains("model.decide_next_action"));
    }

    #[test]
    fn capability_dispatched_projects_tool_call_message() {
        let event = ProcessEvent {
            schema_version: "supernova.process_truth.v1".into(),
            event_id: 133,
            timestamp_ms: 1781030504367,
            job_id: "job_test".into(),
            pid: Some("pid_test".into()),
            event_type: "capability_dispatched".into(),
            data: json!({
                "capability_id": "os.write_artifact",
                "status": "running",
            }),
        };

        let message =
            process_event_to_message("workspace_1", "container_1", "job_test", &event).unwrap();

        assert_eq!(message.message_type, MessageType::ToolCall);
        assert_eq!(message.status, "streaming");
        assert!(message
            .body_text
            .unwrap_or_default()
            .contains("os.write_artifact"));
    }

    #[test]
    fn capability_receipt_projects_task_tool_result_message() {
        let event = ProcessEvent {
            schema_version: "supernova.process_truth.v1".into(),
            event_id: 221,
            timestamp_ms: 1781030505000,
            job_id: "job_test".into(),
            pid: Some("pid_test".into()),
            event_type: "capability_receipt".into(),
            data: json!({
                "capability_id": "os.list_tree",
                "status": "success",
            }),
        };

        let message =
            process_event_to_message("workspace_1", "container_1", "job_test", &event).unwrap();

        assert_eq!(message.message_type, MessageType::ToolResult);
        assert_eq!(message.title.as_deref(), Some("os.list_tree: success"));
    }

    #[test]
    fn recoverable_or_intermediate_blocked_event_is_not_projected_to_main_feed() {
        let event = ProcessEvent {
            schema_version: "supernova.process_truth.v1".into(),
            event_id: 222,
            timestamp_ms: 1781030505001,
            job_id: "job_test".into(),
            pid: Some("pid_test".into()),
            event_type: "capability_blocked".into(),
            data: json!({
                "capability_id": "os.read_file",
                "status": "blocked",
                "data": {"reason": "read target is not a file"},
            }),
        };

        let message = process_event_to_message("workspace_1", "container_1", "job_test", &event);

        assert!(message.is_none());
    }

    #[test]
    fn unknown_recoverable_blocked_event_projects_phase_diagnostic_not_error() {
        let event = ProcessEvent {
            schema_version: "supernova.process_truth.v1".into(),
            event_id: 224,
            timestamp_ms: 1781030505002,
            job_id: "job_test".into(),
            pid: Some("pid_test".into()),
            event_type: "terminal_service_blocked".into(),
            data: json!({
                "capability_id": "terminal.service_status",
                "status": "blocked",
                "error": {
                    "code": "SERVICE_NOT_FOUND",
                    "message": "service not found"
                },
                "recoverable": true,
            }),
        };

        let message =
            process_event_to_message("workspace_1", "container_1", "job_test", &event).unwrap();

        assert_eq!(message.message_type, MessageType::Phase);
        assert_eq!(message.status, "completed");
        assert_eq!(message.title.as_deref(), Some("terminal service blocked"));
        assert!(message
            .body_text
            .as_deref()
            .unwrap_or_default()
            .contains("service not found"));
    }

    #[test]
    fn final_job_blocked_projects_single_user_visible_error() {
        let event = ProcessEvent {
            schema_version: "supernova.process_truth.v1".into(),
            event_id: 222,
            timestamp_ms: 1781030505001,
            job_id: "job_test".into(),
            pid: Some("pid_test".into()),
            event_type: "job_blocked".into(),
            data: json!({
                "error": {
                    "code": "RUNTIME_CAPABILITY_BLOCKED",
                    "message": "source mutation requires approval"
                },
                "runtime_id": "runtime_1",
            }),
        };

        let message =
            process_event_to_message("workspace_1", "container_1", "job_test", &event).unwrap();

        assert_eq!(message.message_type, MessageType::Error);
        let body = message.body_text.unwrap_or_default();
        assert!(body.contains("job blocked"));
        assert!(body.contains("source mutation requires approval"));
    }

    #[test]
    fn job_completed_projects_phase_not_duplicate_final_answer() {
        let event = ProcessEvent {
            schema_version: "supernova.process_truth.v1".into(),
            event_id: 223,
            timestamp_ms: 1781030505002,
            job_id: "job_test".into(),
            pid: Some("pid_test".into()),
            event_type: "job_completed".into(),
            data: json!({"completion_statement": "Task finished."}),
        };

        let message =
            process_event_to_message("workspace_1", "container_1", "job_test", &event).unwrap();

        assert_eq!(message.message_type, MessageType::Phase);
        assert_eq!(message.title.as_deref(), Some("Task completed"));
    }

    #[test]
    fn completion_statement_projects_task_final_answer_message() {
        let event = ProcessEvent {
            schema_version: "supernova.process_truth.v1".into(),
            event_id: 220,
            timestamp_ms: 1781030504999,
            job_id: "job_test".into(),
            pid: Some("pid_test".into()),
            event_type: "completion_statement_recorded".into(),
            data: json!({
                "completion_statement": "Task summary completed.",
                "claimed_artifacts": ["out.md"],
                "known_limitations": ["Manual review is still recommended."],
            }),
        };

        let message =
            process_event_to_message("workspace_1", "container_1", "job_test", &event).unwrap();

        assert_eq!(message.message_type, MessageType::Text);
        assert_eq!(message.title.as_deref(), Some("Task final answer"));
        let body = message.body_text.unwrap_or_default();
        assert!(body.contains("Task summary completed."));
        assert!(body.contains("- out.md"));
        assert!(body.contains("Manual review is still recommended."));
    }

    #[test]
    fn provider_native_assistant_content_projects_model_text_message() {
        let event = ProcessEvent {
            schema_version: "supernova.process_truth.v1".into(),
            event_id: 224,
            timestamp_ms: 1781030505003,
            job_id: "job_test".into(),
            pid: Some("pid_test".into()),
            event_type: "provider_native_assistant_content_yielded".into(),
            data: json!({
                "assistant_content": "I need one more observation before choosing a tool.",
                "task_status": "running",
                "closure_allowed": false,
                "required_closure_tool": "process.complete",
            }),
        };

        let message =
            process_event_to_message("workspace_1", "container_1", "job_test", &event).unwrap();

        assert_eq!(message.role, MessageRole::Assistant);
        assert_eq!(message.message_type, MessageType::Text);
        assert_eq!(message.title.as_deref(), Some("Model message"));
        assert_eq!(
            message.body_text.as_deref(),
            Some("I need one more observation before choosing a tool.")
        );
        assert_ne!(message.message_type, MessageType::Reasoning);
    }
}
