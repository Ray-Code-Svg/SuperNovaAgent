use std::collections::{BTreeMap, BTreeSet};
use std::io;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::context_window::{ContextScope, ContextWindowEstimate};
use crate::provider_transcript::ProviderTranscriptMessage;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ContextCompactionInput {
    pub schema: String,
    pub scope: ContextScope,
    pub estimate: ContextWindowEstimate,
    pub visible_context_ref: Option<String>,
    pub selected_refs: Vec<String>,
    pub live_suffix_refs: Vec<String>,
    pub target_summary_tokens: u64,
    pub payload: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextCheckpointReceipt {
    pub checkpoint_id: String,
    pub scope: ContextScope,
    pub checkpoint_ref: String,
    pub created_at_ms: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ContextCompactionReceipt {
    pub compaction_id: String,
    pub scope: ContextScope,
    pub status: String,
    pub summary_ref: Option<String>,
    pub live_suffix_ref: Option<String>,
    pub model_call_receipt_ref: Option<String>,
    pub compacted_until_message_index: Option<usize>,
    pub errors: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderTranscriptReplacement {
    pub old_transcript_ref: String,
    pub new_transcript_ref: String,
    pub summary_ref: Option<String>,
    pub live_suffix_ref: Option<String>,
    pub compacted_until_message_index: Option<usize>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderTranscriptValidationReceipt {
    pub provider: String,
    pub protocol: String,
    pub valid: bool,
    pub message_count: usize,
    pub pending_tool_call_ids: Vec<String>,
    pub errors: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct ProviderTranscriptProtocolValidator;

impl ProviderTranscriptProtocolValidator {
    pub fn validate_deepseek_native_messages(
        messages: &[ProviderTranscriptMessage],
    ) -> io::Result<ProviderTranscriptValidationReceipt> {
        let mut errors = Vec::new();
        let mut pending: BTreeMap<String, usize> = BTreeMap::new();
        let mut completed: BTreeSet<String> = BTreeSet::new();
        for (index, message) in messages.iter().enumerate() {
            match message.role.as_str() {
                "assistant" => {
                    if message.tool_call_id.is_some() {
                        errors.push(format!(
                            "assistant message at index {index} must not carry tool_call_id"
                        ));
                    }
                    for tool_call in &message.tool_calls {
                        if tool_call.id.trim().is_empty() {
                            errors
                                .push(format!("assistant tool_call at index {index} has empty id"));
                            continue;
                        }
                        if pending.contains_key(&tool_call.id) || completed.contains(&tool_call.id)
                        {
                            errors.push(format!("duplicate tool_call_id {}", tool_call.id));
                        } else {
                            pending.insert(tool_call.id.clone(), index);
                        }
                    }
                }
                "tool" => {
                    let Some(tool_call_id) = message.tool_call_id.as_deref() else {
                        errors.push(format!(
                            "tool message at index {index} missing tool_call_id"
                        ));
                        continue;
                    };
                    if !pending.contains_key(tool_call_id) {
                        errors.push(format!("orphan tool message for {tool_call_id}"));
                    } else {
                        pending.remove(tool_call_id);
                        completed.insert(tool_call_id.to_string());
                    }
                    if !message.tool_calls.is_empty() {
                        errors.push(format!(
                            "tool message at index {index} must not contain nested tool_calls"
                        ));
                    }
                }
                "system" | "user" => {
                    if message.tool_call_id.is_some() || !message.tool_calls.is_empty() {
                        errors.push(format!(
                            "{} message at index {index} must not carry tool-call fields",
                            message.role
                        ));
                    }
                }
                other => errors.push(format!("unsupported provider transcript role {other}")),
            }
        }
        Ok(ProviderTranscriptValidationReceipt {
            provider: "deepseek".to_string(),
            protocol: "deepseek_chat_completions".to_string(),
            valid: errors.is_empty() && pending.is_empty(),
            message_count: messages.len(),
            pending_tool_call_ids: pending.keys().cloned().collect(),
            errors,
        })
    }
}

pub fn validate_container_context_summary(value: &Value, container_id: &str) -> io::Result<()> {
    let schema = value
        .get("schema")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if schema != "supernova_container_context_summary.v1" {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "container context summary schema mismatch",
        ));
    }
    if value
        .get("container_id")
        .and_then(Value::as_str)
        .is_some_and(|actual| actual == container_id)
    {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "container context summary container_id mismatch",
        ))
    }
}

pub fn validate_chat_context_summary(value: &Value) -> io::Result<()> {
    validate_summary_schema(
        value,
        "supernova_chat_context_summary.v1",
        "chat context summary",
    )
}

pub fn validate_task_context_summary(value: &Value) -> io::Result<()> {
    validate_summary_schema(
        value,
        "supernova_task_context_summary.v1",
        "task context summary",
    )
}

pub fn container_context_summary_output_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "required": ["schema", "container_id", "summary"],
        "schema_name": "supernova_container_context_summary.v1",
        "properties": {
            "schema": {"type": "string"},
            "container_id": {"type": "string"},
            "summary": {"type": "string"},
            "important_decisions": {"type": "array"},
            "active_goals": {"type": "array"},
            "artifact_index": {"type": "array"},
            "source_refs": {"type": "array"},
            "task_refs": {"type": "array"},
            "chat_refs": {"type": "array"},
            "memory_refs": {"type": "array"},
            "known_constraints": {"type": "array"},
            "open_questions": {"type": "array"}
        }
    })
}

pub fn chat_context_summary_output_schema() -> Value {
    summary_output_schema("supernova_chat_context_summary.v1")
}

pub fn task_context_summary_output_schema() -> Value {
    summary_output_schema("supernova_task_context_summary.v1")
}

fn summary_output_schema(schema_name: &str) -> Value {
    serde_json::json!({
        "type": "object",
        "required": ["schema", "summary"],
        "schema_name": schema_name,
        "properties": {
            "schema": {"type": "string"},
            "summary": {"type": "string"},
            "important_decisions": {"type": "array"},
            "active_goals": {"type": "array"},
            "artifact_index": {"type": "array"},
            "source_refs": {"type": "array"},
            "task_refs": {"type": "array"},
            "chat_refs": {"type": "array"},
            "memory_refs": {"type": "array"},
            "known_constraints": {"type": "array"},
            "open_questions": {"type": "array"}
        }
    })
}

fn validate_summary_schema(value: &Value, expected: &str, label: &str) -> io::Result<()> {
    let schema = value
        .get("schema")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if schema != expected {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{label} schema mismatch"),
        ));
    }
    let summary = value
        .get("summary")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if summary.trim().is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{label} summary is empty"),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model_runtime::ProviderToolCall;
    use serde_json::json;

    fn tool_call(id: &str) -> ProviderToolCall {
        ProviderToolCall {
            id: id.to_string(),
            r#type: "function".to_string(),
            function: json!({"name": "os.read_file", "arguments": "{}"}),
        }
    }

    #[test]
    fn deepseek_protocol_validator_requires_tool_messages_for_retained_calls() {
        let messages = vec![ProviderTranscriptMessage {
            role: "assistant".to_string(),
            content: None,
            reasoning_content: Some("reasoning".to_string()),
            tool_calls: vec![tool_call("call_1")],
            tool_call_id: None,
        }];
        let receipt =
            ProviderTranscriptProtocolValidator::validate_deepseek_native_messages(&messages)
                .unwrap();
        assert!(!receipt.valid);
        assert_eq!(receipt.pending_tool_call_ids, vec!["call_1".to_string()]);
    }

    #[test]
    fn deepseek_protocol_validator_accepts_closed_tool_call_pair() {
        let messages = vec![
            ProviderTranscriptMessage {
                role: "assistant".to_string(),
                content: None,
                reasoning_content: Some("reasoning".to_string()),
                tool_calls: vec![tool_call("call_1")],
                tool_call_id: None,
            },
            ProviderTranscriptMessage {
                role: "tool".to_string(),
                content: Some("{\"status\":\"success\"}".to_string()),
                reasoning_content: None,
                tool_calls: vec![],
                tool_call_id: Some("call_1".to_string()),
            },
        ];
        let receipt =
            ProviderTranscriptProtocolValidator::validate_deepseek_native_messages(&messages)
                .unwrap();
        assert!(receipt.valid, "{receipt:#?}");
        assert!(receipt.pending_tool_call_ids.is_empty());
    }
}
