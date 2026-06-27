use std::io;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::model_config::estimate_text_tokens_conservative;
use crate::model_runtime::{ModelProviderResponse, ProviderAssistantMessage, ProviderToolCall};
use crate::provider_debug::append_provider_native_debug;
use crate::{json_err, now_ms, safe_blob_name, ProcessEvent, ProcessTruthStore};

const COMPACT_MESSAGE_THRESHOLD: usize = 64;
const COMPACT_TOKEN_THRESHOLD: u64 = 160_000;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderToolCallState {
    pub provider_tool_call_id: String,
    pub provider_name: Option<String>,
    pub status: String,
    pub assistant_message_ref: Option<String>,
    pub tool_result_ref: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderTranscriptTokenEstimate {
    pub input_tokens_estimated: u64,
    pub output_tokens_reserved: u64,
    pub reasoning_tokens_reserved: u64,
    pub total_context_limit: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ProviderTranscriptState {
    pub provider: String,
    pub protocol: String,
    pub transcript_id: String,
    pub messages_ref: String,
    pub summary_ref: Option<String>,
    pub latest_assistant_message_ref: Option<String>,
    pub pending_tool_calls: Vec<ProviderToolCallState>,
    pub reasoning_content_refs: Vec<String>,
    pub token_estimate: ProviderTranscriptTokenEstimate,
    pub updated_event_id: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ProviderTranscriptMessage {
    pub role: String,
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ProviderToolCall>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ProviderTranscriptSummary {
    pub provider: String,
    pub protocol: String,
    pub transcript_id: String,
    pub messages_ref: String,
    pub message_count: usize,
    pub latest_assistant_message_ref: Option<String>,
    pub pending_tool_call_count: usize,
    pub reasoning_content_refs: Vec<String>,
    pub token_estimate: ProviderTranscriptTokenEstimate,
    pub compacted: bool,
    pub updated_at_ms: u128,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderTranscriptRecord {
    pub transcript_id: String,
    pub messages_ref: String,
    pub summary_ref: String,
    pub assistant_message_ref: Option<String>,
    pub reasoning_content_ref: Option<String>,
    pub message_index: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderToolResultRecord {
    pub transcript_id: String,
    pub messages_ref: String,
    pub summary_ref: String,
    pub tool_message_ref: String,
    pub provider_tool_call_id: String,
    pub provider_tool_call_index: Option<usize>,
    pub provider_tool_batch_id: Option<String>,
    pub message_index: usize,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderToolResultMetadata {
    pub provider_tool_call_index: Option<usize>,
    pub provider_tool_batch_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderUserControlMessageRecord {
    pub transcript_id: String,
    pub messages_ref: String,
    pub summary_ref: String,
    pub control_message_ref: String,
    pub message_index: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderUserMessageRecord {
    pub transcript_id: String,
    pub messages_ref: String,
    pub summary_ref: String,
    pub user_message_ref: String,
    pub message_index: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderTranscriptReplacementRecord {
    pub transcript_id: String,
    pub old_transcript_ref: String,
    pub new_transcript_ref: String,
    pub summary_ref: String,
    pub live_suffix_ref: String,
    pub compacted_until_message_index: usize,
    pub message_count: usize,
    pub pending_tool_call_count: usize,
}

pub fn record_provider_assistant_response(
    truth: &ProcessTruthStore,
    pid: &str,
    provider: &str,
    protocol: &str,
    model_call_id: &str,
    response: &ModelProviderResponse,
    store_reasoning_content: bool,
) -> io::Result<Option<ProviderTranscriptRecord>> {
    let Some(assistant) = response.assistant_message.clone().or_else(|| {
        if response.output_text.is_empty()
            && response.reasoning_content.is_none()
            && response.tool_calls.is_empty()
        {
            None
        } else {
            Some(ProviderAssistantMessage {
                role: "assistant".to_string(),
                content: if response.output_text.is_empty() {
                    None
                } else {
                    Some(response.output_text.clone())
                },
                reasoning_content: response.reasoning_content.clone(),
                tool_calls: response.tool_calls.clone(),
            })
        }
    }) else {
        return Ok(None);
    };

    let transcript_id = transcript_id_for(truth.job_id(), provider);
    let previous = replay_provider_transcript_state_from_events(
        truth.job_id(),
        provider,
        protocol,
        &transcript_id,
        &truth.read_events()?,
    );
    if previous.is_none() {
        truth.append_event(
            Some(pid),
            "provider_transcript_created",
            json!({
                "provider": provider,
                "protocol": protocol,
                "transcript_id": transcript_id,
            }),
        )?;
    }

    let mut messages = read_messages_from_state(truth, previous.as_ref())?;
    let message_index = messages.len();
    let mandatory_reasoning_replay = !assistant.tool_calls.is_empty();
    let reasoning_content_ref = if let Some(reasoning) = assistant.reasoning_content.as_deref() {
        if reasoning.is_empty() || (!store_reasoning_content && !mandatory_reasoning_replay) {
            None
        } else {
            let reasoning_ref = truth.write_blob(
                &format!(
                    "provider_transcripts/{}/reasoning_{}.txt",
                    safe_blob_name(&transcript_id),
                    message_index
                ),
                reasoning.as_bytes(),
            )?;
            truth.append_event(
                Some(pid),
                "provider_reasoning_content_recorded",
                json!({
                    "provider": provider,
                    "protocol": protocol,
                    "transcript_id": transcript_id,
                    "model_call_id": model_call_id,
                    "reasoning_content_ref": reasoning_ref.clone(),
                    "reasoning_content_tokens_estimated": estimate_text_tokens_conservative(reasoning),
                    "mandatory_provider_replay": mandatory_reasoning_replay,
                    "retention_policy": "job_local_provider_protocol_evidence",
                    "default_export_contains_raw_reasoning": false,
                    "task_portfolio_contains_raw_reasoning": false,
                    "long_term_memory_promotion_allowed": false,
                }),
            )?;
            Some(reasoning_ref)
        }
    } else {
        None
    };

    let assistant_message = ProviderTranscriptMessage {
        role: "assistant".to_string(),
        content: assistant.content.clone(),
        reasoning_content: assistant.reasoning_content.clone(),
        tool_calls: assistant.tool_calls.clone(),
        tool_call_id: None,
    };
    let assistant_message_ref = truth.write_blob(
        &format!(
            "provider_transcripts/{}/assistant_{}.json",
            safe_blob_name(&transcript_id),
            message_index
        ),
        &serde_json::to_vec_pretty(&assistant_message).map_err(json_err)?,
    )?;
    messages.push(assistant_message);

    let mut compacted = false;
    let estimated_tokens = estimate_messages_tokens(&messages);
    if messages.len() > COMPACT_MESSAGE_THRESHOLD || estimated_tokens > COMPACT_TOKEN_THRESHOLD {
        compact_non_mandatory_reasoning(&mut messages);
        compacted = true;
    }

    let messages_ref = truth.write_blob(
        &format!(
            "provider_transcripts/{}/messages.json",
            safe_blob_name(&transcript_id)
        ),
        &serde_json::to_vec_pretty(&messages).map_err(json_err)?,
    )?;
    if compacted {
        truth.append_event(
            Some(pid),
            "provider_transcript_compacted",
            json!({
                "provider": provider,
                "protocol": protocol,
                "transcript_id": transcript_id,
                "messages_ref": messages_ref.clone(),
                "compaction_policy": "drop_non_tool_call_reasoning_content",
                "message_count": messages.len(),
            }),
        )?;
    }

    let mut reasoning_refs = previous
        .as_ref()
        .map(|state| state.reasoning_content_refs.clone())
        .unwrap_or_default();
    if let Some(reasoning_ref) = reasoning_content_ref.clone() {
        reasoning_refs.push(reasoning_ref);
    }
    let mut pending_tool_calls = previous
        .as_ref()
        .map(|state| state.pending_tool_calls.clone())
        .unwrap_or_default();
    pending_tool_calls.extend(assistant.tool_calls.iter().map(|tool_call| {
        ProviderToolCallState {
            provider_tool_call_id: tool_call.id.clone(),
            provider_name: tool_call
                .function
                .get("name")
                .and_then(Value::as_str)
                .map(ToString::to_string),
            status: "requested".to_string(),
            assistant_message_ref: Some(assistant_message_ref.clone()),
            tool_result_ref: None,
        }
    }));
    let summary = ProviderTranscriptSummary {
        provider: provider.to_string(),
        protocol: protocol.to_string(),
        transcript_id: transcript_id.clone(),
        messages_ref: messages_ref.clone(),
        message_count: messages.len(),
        latest_assistant_message_ref: Some(assistant_message_ref.clone()),
        pending_tool_call_count: pending_tool_calls.len(),
        reasoning_content_refs: reasoning_refs.clone(),
        token_estimate: ProviderTranscriptTokenEstimate {
            input_tokens_estimated: estimate_messages_tokens(&messages),
            output_tokens_reserved: 0,
            reasoning_tokens_reserved: reasoning_refs.len() as u64,
            total_context_limit: 1_000_000,
        },
        compacted,
        updated_at_ms: now_ms(),
    };
    let summary_ref = truth.write_blob(
        &format!(
            "provider_transcripts/{}/summary.json",
            safe_blob_name(&transcript_id)
        ),
        &serde_json::to_vec_pretty(&summary).map_err(json_err)?,
    )?;
    let append_event = truth.append_event(
        Some(pid),
        "provider_transcript_appended",
        json!({
            "provider": provider,
            "protocol": protocol,
            "transcript_id": transcript_id,
            "model_call_id": model_call_id,
            "messages_ref": messages_ref.clone(),
            "summary_ref": summary_ref.clone(),
            "latest_assistant_message_ref": assistant_message_ref.clone(),
            "reasoning_content_ref": reasoning_content_ref.clone(),
            "message_index": message_index,
            "message_count": messages.len(),
            "pending_tool_call_count": pending_tool_calls.len(),
            "compacted": compacted,
        }),
    )?;
    truth.append_event(
        Some(pid),
        "provider_assistant_message_recorded",
        json!({
            "provider": provider,
            "protocol": protocol,
            "transcript_id": transcript_id,
            "model_call_id": model_call_id,
            "assistant_message_ref": assistant_message_ref.clone(),
            "messages_ref": messages_ref.clone(),
            "reasoning_content_ref": reasoning_content_ref.clone(),
            "tool_call_count": assistant.tool_calls.len(),
            "message_index": message_index,
        }),
    )?;
    for tool_call in &assistant.tool_calls {
        truth.append_event(
            Some(pid),
            "provider_tool_call_requested",
            json!({
                "provider": provider,
                "protocol": protocol,
                "transcript_id": transcript_id,
                "model_call_id": model_call_id,
                "provider_tool_call_id": tool_call.id,
                "provider_tool_name": tool_call.function.get("name").and_then(Value::as_str),
                "assistant_message_ref": assistant_message_ref.clone(),
                "messages_ref": messages_ref.clone(),
            }),
        )?;
    }
    if !assistant.tool_calls.is_empty() {
        let _ = append_provider_native_debug(
            truth,
            "transcript_check",
            json!({
                "model_call_id": model_call_id,
                "decision_protocol": "provider_native_tool_calls",
                "diagnostic": {
                    "append_kind": "assistant_tool_calls",
                    "transcript_id": transcript_id.clone(),
                    "assistant_message_ref": assistant_message_ref.clone(),
                    "message_index": message_index,
                    "assistant_tool_call_count": assistant.tool_calls.len(),
                    "pending_tool_call_count": pending_tool_calls.len(),
                    "pending_tool_calls": pending_tool_calls.iter().map(|item| {
                        json!({
                            "provider_tool_call_id": item.provider_tool_call_id.clone(),
                            "provider_name": item.provider_name.clone(),
                            "status": item.status.clone(),
                        })
                    }).collect::<Vec<_>>(),
                    "deepseek_requires_matching_tool_messages": true,
                }
            }),
        );
    }

    let _state = ProviderTranscriptState {
        provider: provider.to_string(),
        protocol: protocol.to_string(),
        transcript_id: transcript_id.clone(),
        messages_ref: messages_ref.clone(),
        summary_ref: Some(summary_ref.clone()),
        latest_assistant_message_ref: Some(assistant_message_ref.clone()),
        pending_tool_calls,
        reasoning_content_refs: reasoning_refs,
        token_estimate: summary.token_estimate,
        updated_event_id: append_event.event_id,
    };
    Ok(Some(ProviderTranscriptRecord {
        transcript_id,
        messages_ref,
        summary_ref,
        assistant_message_ref: Some(assistant_message_ref),
        reasoning_content_ref,
        message_index,
    }))
}

pub fn record_provider_tool_result(
    truth: &ProcessTruthStore,
    pid: &str,
    provider: &str,
    protocol: &str,
    provider_tool_call_id: &str,
    tool_result: &Value,
) -> io::Result<ProviderToolResultRecord> {
    record_provider_tool_result_with_metadata(
        truth,
        pid,
        provider,
        protocol,
        provider_tool_call_id,
        tool_result,
        ProviderToolResultMetadata::default(),
    )
}

pub fn record_provider_tool_result_with_metadata(
    truth: &ProcessTruthStore,
    pid: &str,
    provider: &str,
    protocol: &str,
    provider_tool_call_id: &str,
    tool_result: &Value,
    metadata: ProviderToolResultMetadata,
) -> io::Result<ProviderToolResultRecord> {
    let transcript_id = transcript_id_for(truth.job_id(), provider);
    let previous = replay_provider_transcript_state_from_events(
        truth.job_id(),
        provider,
        protocol,
        &transcript_id,
        &truth.read_events()?,
    )
    .ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "provider transcript does not exist for tool result",
        )
    })?;
    let mut pending_tool_calls = previous.pending_tool_calls.clone();
    let Some(pending) = pending_tool_calls
        .iter_mut()
        .find(|item| item.provider_tool_call_id == provider_tool_call_id)
    else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "provider tool result does not match a pending tool call",
        ));
    };

    let mut messages = read_provider_messages(truth, &previous)?;
    let message_index = messages.len();
    let tool_message = ProviderTranscriptMessage {
        role: "tool".to_string(),
        content: Some(provider_tool_result_content(tool_result)?),
        reasoning_content: None,
        tool_calls: Vec::new(),
        tool_call_id: Some(provider_tool_call_id.to_string()),
    };
    let tool_message_ref = truth.write_blob(
        &format!(
            "provider_transcripts/{}/tool_{}.json",
            safe_blob_name(&transcript_id),
            safe_blob_name(provider_tool_call_id)
        ),
        &serde_json::to_vec_pretty(&tool_message).map_err(json_err)?,
    )?;
    pending.status = "completed".to_string();
    pending.tool_result_ref = Some(tool_message_ref.clone());
    messages.push(tool_message);

    let mut compacted = false;
    let estimated_tokens = estimate_messages_tokens(&messages);
    if messages.len() > COMPACT_MESSAGE_THRESHOLD || estimated_tokens > COMPACT_TOKEN_THRESHOLD {
        compact_non_mandatory_reasoning(&mut messages);
        compacted = true;
    }

    let messages_ref = truth.write_blob(
        &format!(
            "provider_transcripts/{}/messages.json",
            safe_blob_name(&transcript_id)
        ),
        &serde_json::to_vec_pretty(&messages).map_err(json_err)?,
    )?;
    if compacted {
        truth.append_event(
            Some(pid),
            "provider_transcript_compacted",
            json!({
                "provider": provider,
                "protocol": protocol,
                "transcript_id": transcript_id,
                "messages_ref": messages_ref.clone(),
                "compaction_policy": "drop_non_tool_call_reasoning_content",
                "message_count": messages.len(),
            }),
        )?;
    }

    let active_pending_tool_calls = pending_tool_calls
        .iter()
        .filter(|item| item.status != "completed")
        .cloned()
        .collect::<Vec<_>>();
    let summary = ProviderTranscriptSummary {
        provider: provider.to_string(),
        protocol: protocol.to_string(),
        transcript_id: transcript_id.clone(),
        messages_ref: messages_ref.clone(),
        message_count: messages.len(),
        latest_assistant_message_ref: previous.latest_assistant_message_ref.clone(),
        pending_tool_call_count: active_pending_tool_calls.len(),
        reasoning_content_refs: previous.reasoning_content_refs.clone(),
        token_estimate: ProviderTranscriptTokenEstimate {
            input_tokens_estimated: estimate_messages_tokens(&messages),
            output_tokens_reserved: 0,
            reasoning_tokens_reserved: previous.reasoning_content_refs.len() as u64,
            total_context_limit: 1_000_000,
        },
        compacted,
        updated_at_ms: now_ms(),
    };
    let summary_ref = truth.write_blob(
        &format!(
            "provider_transcripts/{}/summary.json",
            safe_blob_name(&transcript_id)
        ),
        &serde_json::to_vec_pretty(&summary).map_err(json_err)?,
    )?;
    truth.append_event(
        Some(pid),
        "provider_tool_result_recorded",
        json!({
            "provider": provider,
            "protocol": protocol,
            "transcript_id": transcript_id,
            "provider_tool_call_id": provider_tool_call_id,
            "provider_tool_call_index": metadata.provider_tool_call_index,
            "provider_tool_batch_id": metadata.provider_tool_batch_id.clone(),
            "tool_result_ref": tool_message_ref.clone(),
            "messages_ref": messages_ref.clone(),
            "summary_ref": summary_ref.clone(),
            "message_index": message_index,
        }),
    )?;
    truth.append_event(
        Some(pid),
        "provider_transcript_appended",
        json!({
            "provider": provider,
            "protocol": protocol,
            "transcript_id": transcript_id,
            "append_kind": "tool_result",
            "messages_ref": messages_ref.clone(),
            "summary_ref": summary_ref.clone(),
            "latest_assistant_message_ref": previous.latest_assistant_message_ref,
            "latest_tool_message_ref": tool_message_ref.clone(),
            "provider_tool_call_id": provider_tool_call_id,
            "provider_tool_call_index": metadata.provider_tool_call_index,
            "provider_tool_batch_id": metadata.provider_tool_batch_id.clone(),
            "message_index": message_index,
            "message_count": messages.len(),
            "pending_tool_call_count": active_pending_tool_calls.len(),
            "compacted": compacted,
        }),
    )?;
    let _ = append_provider_native_debug(
        truth,
        "transcript_check",
        json!({
            "provider_tool_call_id": provider_tool_call_id,
            "provider_tool_call_index": metadata.provider_tool_call_index,
            "provider_tool_batch_id": metadata.provider_tool_batch_id.clone(),
            "decision_protocol": "provider_native_tool_calls",
            "diagnostic": {
                "append_kind": "tool_result",
                "transcript_id": transcript_id.clone(),
                "tool_message_ref": tool_message_ref.clone(),
                "messages_ref": messages_ref.clone(),
                "message_index": message_index,
                "pending_tool_call_count": active_pending_tool_calls.len(),
                "pending_tool_calls": active_pending_tool_calls.iter().map(|item| {
                    json!({
                        "provider_tool_call_id": item.provider_tool_call_id.clone(),
                        "provider_name": item.provider_name.clone(),
                        "status": item.status.clone(),
                    })
                }).collect::<Vec<_>>(),
                "deepseek_ready_for_next_request": active_pending_tool_calls.is_empty(),
            }
        }),
    );

    Ok(ProviderToolResultRecord {
        transcript_id,
        messages_ref,
        summary_ref,
        tool_message_ref,
        provider_tool_call_id: provider_tool_call_id.to_string(),
        provider_tool_call_index: metadata.provider_tool_call_index,
        provider_tool_batch_id: metadata.provider_tool_batch_id,
        message_index,
    })
}

pub fn record_provider_user_message(
    truth: &ProcessTruthStore,
    pid: &str,
    provider: &str,
    protocol: &str,
    model_call_id: &str,
    content: &str,
) -> io::Result<ProviderUserMessageRecord> {
    let transcript_id = transcript_id_for(truth.job_id(), provider);
    let previous = replay_provider_transcript_state_from_events(
        truth.job_id(),
        provider,
        protocol,
        &transcript_id,
        &truth.read_events()?,
    );
    if previous.is_none() {
        truth.append_event(
            Some(pid),
            "provider_transcript_created",
            json!({
                "provider": provider,
                "protocol": protocol,
                "transcript_id": transcript_id,
            }),
        )?;
    }

    let mut messages = read_messages_from_state(truth, previous.as_ref())?;
    let message_index = messages.len();
    let user_message = ProviderTranscriptMessage {
        role: "user".to_string(),
        content: Some(content.to_string()),
        reasoning_content: None,
        tool_calls: Vec::new(),
        tool_call_id: None,
    };
    let user_message_ref = truth.write_blob(
        &format!(
            "provider_transcripts/{}/user_{}.json",
            safe_blob_name(&transcript_id),
            message_index
        ),
        &serde_json::to_vec_pretty(&user_message).map_err(json_err)?,
    )?;
    messages.push(user_message);

    let mut compacted = false;
    let estimated_tokens = estimate_messages_tokens(&messages);
    if messages.len() > COMPACT_MESSAGE_THRESHOLD || estimated_tokens > COMPACT_TOKEN_THRESHOLD {
        compact_non_mandatory_reasoning(&mut messages);
        compacted = true;
    }

    let messages_ref = truth.write_blob(
        &format!(
            "provider_transcripts/{}/messages.json",
            safe_blob_name(&transcript_id)
        ),
        &serde_json::to_vec_pretty(&messages).map_err(json_err)?,
    )?;
    if compacted {
        truth.append_event(
            Some(pid),
            "provider_transcript_compacted",
            json!({
                "provider": provider,
                "protocol": protocol,
                "transcript_id": transcript_id,
                "messages_ref": messages_ref.clone(),
                "compaction_policy": "drop_non_tool_call_reasoning_content",
                "message_count": messages.len(),
            }),
        )?;
    }

    let latest_assistant_message_ref = previous
        .as_ref()
        .and_then(|state| state.latest_assistant_message_ref.clone());
    let pending_tool_calls = previous
        .as_ref()
        .map(|state| state.pending_tool_calls.clone())
        .unwrap_or_default();
    let reasoning_refs = previous
        .as_ref()
        .map(|state| state.reasoning_content_refs.clone())
        .unwrap_or_default();
    let summary = ProviderTranscriptSummary {
        provider: provider.to_string(),
        protocol: protocol.to_string(),
        transcript_id: transcript_id.clone(),
        messages_ref: messages_ref.clone(),
        message_count: messages.len(),
        latest_assistant_message_ref: latest_assistant_message_ref.clone(),
        pending_tool_call_count: pending_tool_calls.len(),
        reasoning_content_refs: reasoning_refs.clone(),
        token_estimate: ProviderTranscriptTokenEstimate {
            input_tokens_estimated: estimate_messages_tokens(&messages),
            output_tokens_reserved: 0,
            reasoning_tokens_reserved: reasoning_refs.len() as u64,
            total_context_limit: 1_000_000,
        },
        compacted,
        updated_at_ms: now_ms(),
    };
    let summary_ref = truth.write_blob(
        &format!(
            "provider_transcripts/{}/summary.json",
            safe_blob_name(&transcript_id)
        ),
        &serde_json::to_vec_pretty(&summary).map_err(json_err)?,
    )?;
    truth.append_event(
        Some(pid),
        "provider_user_message_recorded",
        json!({
            "provider": provider,
            "protocol": protocol,
            "transcript_id": transcript_id,
            "model_call_id": model_call_id,
            "user_message_ref": user_message_ref.clone(),
            "messages_ref": messages_ref.clone(),
            "summary_ref": summary_ref.clone(),
            "message_index": message_index,
        }),
    )?;
    truth.append_event(
        Some(pid),
        "provider_transcript_appended",
        json!({
            "provider": provider,
            "protocol": protocol,
            "transcript_id": transcript_id,
            "append_kind": "user_message",
            "model_call_id": model_call_id,
            "messages_ref": messages_ref.clone(),
            "summary_ref": summary_ref.clone(),
            "latest_assistant_message_ref": latest_assistant_message_ref,
            "latest_user_message_ref": user_message_ref.clone(),
            "message_index": message_index,
            "message_count": messages.len(),
            "pending_tool_call_count": pending_tool_calls.len(),
            "compacted": compacted,
        }),
    )?;

    Ok(ProviderUserMessageRecord {
        transcript_id,
        messages_ref,
        summary_ref,
        user_message_ref,
        message_index,
    })
}

pub fn record_provider_user_control_message(
    truth: &ProcessTruthStore,
    pid: &str,
    provider: &str,
    protocol: &str,
    control_kind: &str,
    control_message: &Value,
) -> io::Result<Option<ProviderUserControlMessageRecord>> {
    let transcript_id = transcript_id_for(truth.job_id(), provider);
    let Some(previous) = replay_provider_transcript_state_from_events(
        truth.job_id(),
        provider,
        protocol,
        &transcript_id,
        &truth.read_events()?,
    ) else {
        return Ok(None);
    };

    let mut messages = read_provider_messages(truth, &previous)?;
    let message_index = messages.len();
    let user_message = ProviderTranscriptMessage {
        role: "user".to_string(),
        content: Some(provider_tool_result_content(control_message)?),
        reasoning_content: None,
        tool_calls: Vec::new(),
        tool_call_id: None,
    };
    let control_message_ref = truth.write_blob(
        &format!(
            "provider_transcripts/{}/user_control_{}.json",
            safe_blob_name(&transcript_id),
            message_index
        ),
        &serde_json::to_vec_pretty(&user_message).map_err(json_err)?,
    )?;
    messages.push(user_message);

    let mut compacted = false;
    let estimated_tokens = estimate_messages_tokens(&messages);
    if messages.len() > COMPACT_MESSAGE_THRESHOLD || estimated_tokens > COMPACT_TOKEN_THRESHOLD {
        compact_non_mandatory_reasoning(&mut messages);
        compacted = true;
    }

    let messages_ref = truth.write_blob(
        &format!(
            "provider_transcripts/{}/messages.json",
            safe_blob_name(&transcript_id)
        ),
        &serde_json::to_vec_pretty(&messages).map_err(json_err)?,
    )?;
    if compacted {
        truth.append_event(
            Some(pid),
            "provider_transcript_compacted",
            json!({
                "provider": provider,
                "protocol": protocol,
                "transcript_id": transcript_id,
                "messages_ref": messages_ref.clone(),
                "compaction_policy": "drop_non_tool_call_reasoning_content",
                "message_count": messages.len(),
            }),
        )?;
    }

    let summary = ProviderTranscriptSummary {
        provider: provider.to_string(),
        protocol: protocol.to_string(),
        transcript_id: transcript_id.clone(),
        messages_ref: messages_ref.clone(),
        message_count: messages.len(),
        latest_assistant_message_ref: previous.latest_assistant_message_ref.clone(),
        pending_tool_call_count: previous.pending_tool_calls.len(),
        reasoning_content_refs: previous.reasoning_content_refs.clone(),
        token_estimate: ProviderTranscriptTokenEstimate {
            input_tokens_estimated: estimate_messages_tokens(&messages),
            output_tokens_reserved: 0,
            reasoning_tokens_reserved: previous.reasoning_content_refs.len() as u64,
            total_context_limit: 1_000_000,
        },
        compacted,
        updated_at_ms: now_ms(),
    };
    let summary_ref = truth.write_blob(
        &format!(
            "provider_transcripts/{}/summary.json",
            safe_blob_name(&transcript_id)
        ),
        &serde_json::to_vec_pretty(&summary).map_err(json_err)?,
    )?;
    truth.append_event(
        Some(pid),
        "provider_user_control_message_recorded",
        json!({
            "provider": provider,
            "protocol": protocol,
            "transcript_id": transcript_id,
            "control_kind": control_kind,
            "control_message_ref": control_message_ref.clone(),
            "messages_ref": messages_ref.clone(),
            "summary_ref": summary_ref.clone(),
            "preview_id": control_message.get("preview_id").and_then(Value::as_str),
            "approval_token_id": control_message.get("approval_token_id").and_then(Value::as_str),
            "message_index": message_index,
        }),
    )?;
    truth.append_event(
        Some(pid),
        "provider_transcript_appended",
        json!({
            "provider": provider,
            "protocol": protocol,
            "transcript_id": transcript_id,
            "append_kind": "user_control",
            "control_kind": control_kind,
            "messages_ref": messages_ref.clone(),
            "summary_ref": summary_ref.clone(),
            "latest_assistant_message_ref": previous.latest_assistant_message_ref,
            "latest_user_control_message_ref": control_message_ref.clone(),
            "message_index": message_index,
            "message_count": messages.len(),
            "pending_tool_call_count": previous.pending_tool_calls.len(),
            "compacted": compacted,
        }),
    )?;

    Ok(Some(ProviderUserControlMessageRecord {
        transcript_id,
        messages_ref,
        summary_ref,
        control_message_ref,
        message_index,
    }))
}

pub fn replace_provider_visible_transcript_with_summary(
    truth: &ProcessTruthStore,
    pid: &str,
    provider: &str,
    protocol: &str,
    summary_text: &str,
    min_live_suffix_turns: usize,
    compaction_reason: &str,
) -> io::Result<Option<ProviderTranscriptReplacementRecord>> {
    let transcript_id = transcript_id_for(truth.job_id(), provider);
    let Some(previous) = replay_provider_transcript_state_from_events(
        truth.job_id(),
        provider,
        protocol,
        &transcript_id,
        &truth.read_events()?,
    ) else {
        return Ok(None);
    };
    let messages = read_provider_messages(truth, &previous)?;
    if messages.is_empty() {
        return Ok(None);
    }

    let live_suffix_start = live_suffix_start_index(
        &messages,
        &previous.pending_tool_calls,
        min_live_suffix_turns,
    );
    let live_suffix = messages
        .iter()
        .skip(live_suffix_start)
        .cloned()
        .collect::<Vec<_>>();
    let summary_message = ProviderTranscriptMessage {
        role: "user".to_string(),
        content: Some(summary_text.to_string()),
        reasoning_content: None,
        tool_calls: Vec::new(),
        tool_call_id: None,
    };
    let mut replacement_messages = Vec::with_capacity(live_suffix.len() + 1);
    replacement_messages.push(summary_message);
    replacement_messages.extend(live_suffix.iter().cloned());

    if provider == "deepseek" && protocol == "deepseek_chat_completions" {
        let validation =
            crate::context_compaction::ProviderTranscriptProtocolValidator::validate_deepseek_native_messages(
                &replacement_messages,
            )?;
        if !validation.valid {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "provider transcript replacement rejected before commit: {}",
                    validation.errors.join("; ")
                ),
            ));
        }
    }

    let replacement_stamp = now_ms();
    let live_suffix_ref = truth.write_blob(
        &format!(
            "provider_transcripts/{}/live_suffix_{}.json",
            safe_blob_name(&transcript_id),
            replacement_stamp
        ),
        &serde_json::to_vec_pretty(&live_suffix).map_err(json_err)?,
    )?;
    let messages_ref = truth.write_blob(
        &format!(
            "provider_transcripts/{}/messages_compacted_{}.json",
            safe_blob_name(&transcript_id),
            replacement_stamp
        ),
        &serde_json::to_vec_pretty(&replacement_messages).map_err(json_err)?,
    )?;
    let summary = ProviderTranscriptSummary {
        provider: provider.to_string(),
        protocol: protocol.to_string(),
        transcript_id: transcript_id.clone(),
        messages_ref: messages_ref.clone(),
        message_count: replacement_messages.len(),
        latest_assistant_message_ref: previous.latest_assistant_message_ref.clone(),
        pending_tool_call_count: previous.pending_tool_calls.len(),
        reasoning_content_refs: previous.reasoning_content_refs.clone(),
        token_estimate: ProviderTranscriptTokenEstimate {
            input_tokens_estimated: estimate_messages_tokens(&replacement_messages),
            output_tokens_reserved: 0,
            reasoning_tokens_reserved: previous.reasoning_content_refs.len() as u64,
            total_context_limit: previous.token_estimate.total_context_limit,
        },
        compacted: true,
        updated_at_ms: now_ms(),
    };
    let summary_ref = truth.write_blob(
        &format!(
            "provider_transcripts/{}/summary_compacted_{}.json",
            safe_blob_name(&transcript_id),
            replacement_stamp
        ),
        &serde_json::to_vec_pretty(&summary).map_err(json_err)?,
    )?;
    let compacted_until_message_index = live_suffix_start.saturating_sub(1);
    truth.append_event(
        Some(pid),
        "provider_transcript_compacted",
        json!({
            "provider": provider,
            "protocol": protocol,
            "transcript_id": transcript_id,
            "old_transcript_ref": previous.messages_ref,
            "messages_ref": messages_ref.clone(),
            "summary_ref": summary_ref.clone(),
            "live_suffix_ref": live_suffix_ref.clone(),
            "compacted_until_message_index": compacted_until_message_index,
            "message_count": replacement_messages.len(),
            "pending_tool_call_count": previous.pending_tool_calls.len(),
            "compaction_policy": "context_window_summary_live_suffix",
            "compaction_reason": compaction_reason,
        }),
    )?;
    Ok(Some(ProviderTranscriptReplacementRecord {
        transcript_id,
        old_transcript_ref: previous.messages_ref,
        new_transcript_ref: messages_ref,
        summary_ref,
        live_suffix_ref,
        compacted_until_message_index,
        message_count: replacement_messages.len(),
        pending_tool_call_count: previous.pending_tool_calls.len(),
    }))
}

pub fn replay_provider_transcript_state(
    truth: &ProcessTruthStore,
    provider: &str,
    protocol: &str,
) -> io::Result<Option<ProviderTranscriptState>> {
    let transcript_id = transcript_id_for(truth.job_id(), provider);
    Ok(replay_provider_transcript_state_from_events(
        truth.job_id(),
        provider,
        protocol,
        &transcript_id,
        &truth.read_events()?,
    ))
}

pub fn replay_provider_transcript_state_from_events(
    _job_id: &str,
    provider: &str,
    protocol: &str,
    transcript_id: &str,
    events: &[ProcessEvent],
) -> Option<ProviderTranscriptState> {
    let mut messages_ref = None;
    let mut summary_ref = None;
    let mut latest_assistant_message_ref = None;
    let mut reasoning_content_refs = Vec::new();
    let mut pending_tool_calls = Vec::new();
    let mut updated_event_id = 0;

    for event in events {
        let event_provider = event.data.get("provider").and_then(Value::as_str);
        let event_protocol = event.data.get("protocol").and_then(Value::as_str);
        let event_transcript_id = event.data.get("transcript_id").and_then(Value::as_str);
        if event_provider != Some(provider)
            || event_protocol != Some(protocol)
            || event_transcript_id != Some(transcript_id)
        {
            continue;
        }
        match event.event_type.as_str() {
            "provider_transcript_appended" | "provider_transcript_compacted" => {
                messages_ref = event
                    .data
                    .get("messages_ref")
                    .and_then(Value::as_str)
                    .map(ToString::to_string)
                    .or(messages_ref);
                summary_ref = event
                    .data
                    .get("summary_ref")
                    .and_then(Value::as_str)
                    .map(ToString::to_string)
                    .or(summary_ref);
                latest_assistant_message_ref = event
                    .data
                    .get("latest_assistant_message_ref")
                    .and_then(Value::as_str)
                    .map(ToString::to_string)
                    .or(latest_assistant_message_ref);
                updated_event_id = event.event_id;
            }
            "provider_reasoning_content_recorded" => {
                if let Some(reasoning_ref) = event
                    .data
                    .get("reasoning_content_ref")
                    .and_then(Value::as_str)
                {
                    reasoning_content_refs.push(reasoning_ref.to_string());
                }
            }
            "provider_tool_call_requested" => {
                if let Some(tool_call_id) = event
                    .data
                    .get("provider_tool_call_id")
                    .and_then(Value::as_str)
                {
                    pending_tool_calls.push(ProviderToolCallState {
                        provider_tool_call_id: tool_call_id.to_string(),
                        provider_name: event
                            .data
                            .get("provider_tool_name")
                            .and_then(Value::as_str)
                            .map(ToString::to_string),
                        status: "requested".to_string(),
                        assistant_message_ref: event
                            .data
                            .get("assistant_message_ref")
                            .and_then(Value::as_str)
                            .map(ToString::to_string),
                        tool_result_ref: None,
                    });
                }
            }
            "provider_tool_result_recorded" => {
                if let Some(tool_call_id) = event
                    .data
                    .get("provider_tool_call_id")
                    .and_then(Value::as_str)
                {
                    for item in pending_tool_calls
                        .iter_mut()
                        .filter(|item| item.provider_tool_call_id == tool_call_id)
                    {
                        item.status = "completed".to_string();
                        item.tool_result_ref = event
                            .data
                            .get("tool_result_ref")
                            .and_then(Value::as_str)
                            .map(ToString::to_string);
                    }
                }
            }
            _ => {}
        }
    }

    let messages_ref = messages_ref?;
    Some(ProviderTranscriptState {
        provider: provider.to_string(),
        protocol: protocol.to_string(),
        transcript_id: transcript_id.to_string(),
        messages_ref,
        summary_ref,
        latest_assistant_message_ref,
        pending_tool_calls: pending_tool_calls
            .into_iter()
            .filter(|item| item.status != "completed")
            .collect(),
        reasoning_content_refs,
        token_estimate: ProviderTranscriptTokenEstimate {
            input_tokens_estimated: 0,
            output_tokens_reserved: 0,
            reasoning_tokens_reserved: 0,
            total_context_limit: 1_000_000,
        },
        updated_event_id,
    })
}

fn live_suffix_start_index(
    messages: &[ProviderTranscriptMessage],
    pending_tool_calls: &[ProviderToolCallState],
    min_live_suffix_turns: usize,
) -> usize {
    if messages.is_empty() {
        return 0;
    }
    let keep_messages = min_live_suffix_turns.saturating_mul(2).max(1);
    let mut start = messages.len().saturating_sub(keep_messages);

    for pending in pending_tool_calls {
        if let Some(index) = messages.iter().position(|message| {
            message.role == "assistant"
                && message
                    .tool_calls
                    .iter()
                    .any(|tool_call| tool_call.id == pending.provider_tool_call_id)
        }) {
            start = start.min(index);
        }
    }

    let mut changed = true;
    while changed {
        changed = false;
        for index in start..messages.len() {
            let message = &messages[index];
            if message.role == "tool" {
                let Some(tool_call_id) = message.tool_call_id.as_deref() else {
                    continue;
                };
                if let Some(assistant_index) = messages[..index].iter().rposition(|candidate| {
                    candidate.role == "assistant"
                        && candidate
                            .tool_calls
                            .iter()
                            .any(|tool_call| tool_call.id == tool_call_id)
                }) {
                    if assistant_index < start {
                        start = assistant_index;
                        changed = true;
                    }
                }
            }
            if message.role == "assistant" && !message.tool_calls.is_empty() {
                for tool_call in &message.tool_calls {
                    if let Some(tool_index) = messages[index + 1..].iter().position(|candidate| {
                        candidate.role == "tool"
                            && candidate.tool_call_id.as_deref() == Some(tool_call.id.as_str())
                    }) {
                        let absolute_tool_index = index + 1 + tool_index;
                        if absolute_tool_index < start {
                            start = index;
                            changed = true;
                        }
                    }
                }
            }
        }
    }
    start
}

pub fn read_provider_messages(
    truth: &ProcessTruthStore,
    state: &ProviderTranscriptState,
) -> io::Result<Vec<ProviderTranscriptMessage>> {
    let path = truth.resolve_blob_ref(&state.messages_ref)?;
    let bytes = std::fs::read(path)?;
    serde_json::from_slice::<Vec<ProviderTranscriptMessage>>(&bytes).map_err(json_err)
}

fn read_messages_from_state(
    truth: &ProcessTruthStore,
    state: Option<&ProviderTranscriptState>,
) -> io::Result<Vec<ProviderTranscriptMessage>> {
    match state {
        Some(state) => read_provider_messages(truth, state),
        None => Ok(Vec::new()),
    }
}

fn compact_non_mandatory_reasoning(messages: &mut [ProviderTranscriptMessage]) {
    for message in messages {
        if message.role == "assistant" && message.tool_calls.is_empty() {
            if message.reasoning_content.is_some() {
                message.reasoning_content =
                    Some("[compacted: non-tool-call reasoning omitted]".to_string());
            }
        }
    }
}

fn estimate_messages_tokens(messages: &[ProviderTranscriptMessage]) -> u64 {
    messages
        .iter()
        .map(|message| {
            serde_json::to_string(message)
                .map(|text| estimate_text_tokens_conservative(&text))
                .unwrap_or(0)
        })
        .sum()
}

fn provider_tool_result_content(tool_result: &Value) -> io::Result<String> {
    match tool_result {
        Value::String(text) => Ok(text.clone()),
        _ => serde_json::to_string(tool_result).map_err(json_err),
    }
}

fn transcript_id_for(job_id: &str, provider: &str) -> String {
    format!(
        "ptr_{}_{}",
        safe_blob_name(job_id),
        safe_blob_name(provider)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context_compaction::ProviderTranscriptProtocolValidator;
    use crate::model_runtime::{ModelProviderResponse, ProviderAssistantMessage};
    use crate::{create_agent_job, now_ms};
    use serde_json::json;
    use std::fs;

    fn workspace(name: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("supernova_{name}_{}", now_ms()));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn response(content: Option<&str>, tool_calls: Vec<ProviderToolCall>) -> ModelProviderResponse {
        ModelProviderResponse {
            output_text: content.unwrap_or_default().to_string(),
            assistant_message: Some(ProviderAssistantMessage {
                role: "assistant".to_string(),
                content: content.map(ToString::to_string),
                reasoning_content: None,
                tool_calls: tool_calls.clone(),
            }),
            reasoning_content: None,
            tool_calls,
            usage: json!({}),
            finish_reason: Some("stop".to_string()),
            raw: json!({}),
            sampling_ignored_by_provider: false,
            streaming: false,
            first_token_ms: None,
            chunks_count: 0,
            stream_event_count: 0,
            first_byte_timeout_ms: None,
            idle_timeout_ms: None,
            max_wall_time_ms: None,
        }
    }

    fn tool_call(id: &str) -> ProviderToolCall {
        ProviderToolCall {
            id: id.to_string(),
            r#type: "function".to_string(),
            function: json!({"name": "os.read_file", "arguments": "{}"}),
        }
    }

    #[test]
    fn provider_visible_replacement_preserves_closed_tool_group_and_replays_new_ref() {
        let root = workspace("provider_visible_replacement");
        let (job, process, truth) = create_agent_job(&root, "compact transcript").unwrap();
        let provider = "deepseek";
        let protocol = "deepseek_chat_completions";
        record_provider_assistant_response(
            &truth,
            &process.pid,
            provider,
            protocol,
            "mcall_old",
            &response(Some("old context that should be summarized"), Vec::new()),
            true,
        )
        .unwrap();
        record_provider_assistant_response(
            &truth,
            &process.pid,
            provider,
            protocol,
            "mcall_tool",
            &response(None, vec![tool_call("call_read")]),
            true,
        )
        .unwrap();
        record_provider_tool_result(
            &truth,
            &process.pid,
            provider,
            protocol,
            "call_read",
            &json!({"status": "success", "receipt_ref": "receipt://read"}),
        )
        .unwrap();
        record_provider_assistant_response(
            &truth,
            &process.pid,
            provider,
            protocol,
            "mcall_latest",
            &response(Some("latest answer"), Vec::new()),
            true,
        )
        .unwrap();
        let before = replay_provider_transcript_state(&truth, provider, protocol)
            .unwrap()
            .unwrap();

        let replacement = replace_provider_visible_transcript_with_summary(
            &truth,
            &process.pid,
            provider,
            protocol,
            r#"{"schema":"supernova_task_context_summary.v1","summary":"old context summarized"}"#,
            1,
            "unit_test",
        )
        .unwrap()
        .unwrap();
        let after = replay_provider_transcript_state(&truth, provider, protocol)
            .unwrap()
            .unwrap();
        assert_eq!(after.messages_ref, replacement.new_transcript_ref);
        assert_ne!(after.messages_ref, before.messages_ref);

        let messages = read_provider_messages(&truth, &after).unwrap();
        assert_eq!(messages.first().unwrap().role, "user");
        assert!(messages.iter().any(|message| {
            message.role == "assistant"
                && message
                    .tool_calls
                    .iter()
                    .any(|tool_call| tool_call.id == "call_read")
        }));
        assert!(messages.iter().any(|message| {
            message.role == "tool" && message.tool_call_id.as_deref() == Some("call_read")
        }));
        let validation =
            ProviderTranscriptProtocolValidator::validate_deepseek_native_messages(&messages)
                .unwrap();
        assert!(validation.valid, "{:?}", validation.errors);
        assert!(replacement.live_suffix_ref.contains("live_suffix_"));
        assert_eq!(
            replacement.transcript_id,
            transcript_id_for(&job.job_id, provider)
        );
    }

    #[test]
    fn provider_visible_replacement_rejects_invalid_pending_tool_boundary_before_commit() {
        let root = workspace("provider_visible_replacement_invalid");
        let (_job, process, truth) = create_agent_job(&root, "compact invalid transcript").unwrap();
        let provider = "deepseek";
        let protocol = "deepseek_chat_completions";
        record_provider_assistant_response(
            &truth,
            &process.pid,
            provider,
            protocol,
            "mcall_tool",
            &response(None, vec![tool_call("call_pending")]),
            true,
        )
        .unwrap();
        let before = replay_provider_transcript_state(&truth, provider, protocol)
            .unwrap()
            .unwrap();

        let err = replace_provider_visible_transcript_with_summary(
            &truth,
            &process.pid,
            provider,
            protocol,
            r#"{"schema":"supernova_task_context_summary.v1","summary":"summary"}"#,
            1,
            "unit_test",
        )
        .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);

        let after = replay_provider_transcript_state(&truth, provider, protocol)
            .unwrap()
            .unwrap();
        assert_eq!(after.messages_ref, before.messages_ref);
        assert!(!truth
            .read_events()
            .unwrap()
            .iter()
            .any(|event| event.event_type == "provider_transcript_compacted"));
    }
}
