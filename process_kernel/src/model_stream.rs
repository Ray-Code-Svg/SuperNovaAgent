use std::io::{self, BufRead, BufReader};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::model_runtime::{
    ModelOperation, ModelProviderFailure, ModelProviderRequest, ModelProviderResponse,
    ModelStreamDelta, ModelStreamDeltaKind, ModelStreamSink, ProviderAssistantMessage,
    ProviderToolCall,
};

#[derive(Clone, Debug)]
pub struct DeepSeekStreamConfig {
    pub first_byte_timeout_ms: u64,
    pub idle_timeout_ms: u64,
    pub max_wall_time_ms: u64,
}

pub fn read_deepseek_streaming_response(
    response: ureq::Response,
    request: &ModelProviderRequest,
    config: &DeepSeekStreamConfig,
    stream_sink: Option<&dyn ModelStreamSink>,
) -> Result<ModelProviderResponse, ModelProviderFailure> {
    let started_at = Instant::now();
    let mut first_token_ms: Option<u128> = None;
    let mut chunks_count: u32 = 0;
    let mut stream_event_count: u32 = 0;
    let mut chunks: Vec<String> = Vec::new();
    let mut reasoning_chunks: Vec<String> = Vec::new();
    let mut reasoning_chunks_count: u32 = 0;
    let mut tool_calls: Vec<PartialToolCall> = Vec::new();
    let mut usage = json!({});
    let mut finish_reason: Option<String> = None;
    let mut done = false;
    let mut reader = BufReader::new(response.into_reader());
    let mut line = String::new();
    loop {
        if config.max_wall_time_ms > 0
            && started_at.elapsed() > Duration::from_millis(config.max_wall_time_ms)
        {
            return Err(stream_failure(
                "DEEPSEEK_STREAM_WALL_TIMEOUT",
                format!(
                    "DeepSeek stream exceeded max wall time after {} events, {} chunks, {} partial chars; partial output discarded",
                    stream_event_count,
                    chunks_count,
                    chunks.iter().map(String::len).sum::<usize>()
                ),
                true,
            ));
        }
        line.clear();
        let read = reader.read_line(&mut line).map_err(|err| {
            let code = if err.kind() == io::ErrorKind::TimedOut {
                if first_token_ms.is_none() {
                    "DEEPSEEK_STREAM_FIRST_BYTE_TIMEOUT"
                } else {
                    "DEEPSEEK_STREAM_IDLE_TIMEOUT"
                }
            } else {
                "DEEPSEEK_STREAM_READ_ERROR"
            };
            stream_failure(
                code,
                format!(
                    "{}; {} partial chunks and {} partial chars discarded",
                    err,
                    chunks_count,
                    chunks.iter().map(String::len).sum::<usize>()
                ),
                true,
            )
        })?;
        if read == 0 {
            break;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with(':') {
            continue;
        }
        let Some(data) = trimmed.strip_prefix("data:") else {
            continue;
        };
        let payload = data.trim();
        if payload == "[DONE]" {
            done = true;
            break;
        }
        stream_event_count = stream_event_count.saturating_add(1);
        let event: Value = serde_json::from_str(payload).map_err(|err| {
            stream_failure(
                "DEEPSEEK_STREAM_JSON_INVALID",
                format!("{err}: {}", payload.chars().take(400).collect::<String>()),
                false,
            )
        })?;
        if let Some(provider_error) = event.get("error") {
            return Err(stream_failure(
                "DEEPSEEK_STREAM_PROVIDER_ERROR",
                provider_error.to_string(),
                true,
            ));
        }
        if let Some(event_usage) = event.get("usage") {
            if !event_usage.is_null() {
                usage = event_usage.clone();
            }
        }
        let Some(choices) = event.get("choices").and_then(Value::as_array) else {
            continue;
        };
        for choice in choices {
            if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
                if !reason.is_empty() {
                    finish_reason = Some(reason.to_string());
                }
            }
            let Some(delta) = choice.get("delta") else {
                continue;
            };
            if let Some(content) = extract_response_text(delta.get("content")) {
                if !content.is_empty() {
                    if first_token_ms.is_none() {
                        first_token_ms = Some(started_at.elapsed().as_millis());
                    }
                    chunks_count = chunks_count.saturating_add(1);
                    if let Some(sink) = stream_sink {
                        sink.on_model_stream_delta(ModelStreamDelta {
                            model_call_id: request.model_call_id.clone(),
                            operation: request.action.operation.clone(),
                            kind: ModelStreamDeltaKind::Answer,
                            sequence: chunks_count,
                            delta: content.clone(),
                        });
                    }
                    chunks.push(content);
                }
            }
            if let Some(reasoning_content) = extract_response_text(delta.get("reasoning_content")) {
                if !reasoning_content.is_empty() {
                    if first_token_ms.is_none() {
                        first_token_ms = Some(started_at.elapsed().as_millis());
                    }
                    reasoning_chunks_count = reasoning_chunks_count.saturating_add(1);
                    if let Some(sink) = stream_sink {
                        sink.on_model_stream_delta(ModelStreamDelta {
                            model_call_id: request.model_call_id.clone(),
                            operation: request.action.operation.clone(),
                            kind: ModelStreamDeltaKind::Reasoning,
                            sequence: reasoning_chunks_count,
                            delta: reasoning_content.clone(),
                        });
                    }
                    reasoning_chunks.push(reasoning_content);
                }
            }
            accumulate_tool_call_deltas(delta.get("tool_calls"), &mut tool_calls);
        }
    }
    if let Some(reason) = finish_reason.as_deref() {
        validate_finish_reason(reason)?;
    }
    let output_text = chunks.join("");
    let provider_tool_calls = materialize_tool_calls(tool_calls);
    if output_text.trim().is_empty() && provider_tool_calls.is_empty() {
        return Err(stream_failure(
            "DEEPSEEK_STREAM_EMPTY_RESPONSE",
            "DeepSeek stream ended without text content".to_string(),
            !done,
        ));
    }
    let reasoning_content = if reasoning_chunks.is_empty() {
        None
    } else {
        Some(reasoning_chunks.join(""))
    };
    Ok(ModelProviderResponse {
        output_text: output_text.clone(),
        assistant_message: Some(ProviderAssistantMessage {
            role: "assistant".to_string(),
            content: if output_text.is_empty() {
                None
            } else {
                Some(output_text.clone())
            },
            reasoning_content: reasoning_content.clone(),
            tool_calls: provider_tool_calls.clone(),
        }),
        reasoning_content,
        tool_calls: provider_tool_calls,
        usage: usage.clone(),
        finish_reason,
        raw: json!({
            "stream": true,
            "done": done,
            "chunks_count": chunks_count,
            "reasoning_chunks_count": reasoning_chunks_count,
            "stream_event_count": stream_event_count,
            "usage": usage,
            "model_call_id": request.model_call_id,
        }),
        sampling_ignored_by_provider: request.model_config.sampling_ignored_by_provider(),
        streaming: true,
        first_token_ms,
        chunks_count,
        stream_event_count,
        first_byte_timeout_ms: Some(config.first_byte_timeout_ms),
        idle_timeout_ms: Some(config.idle_timeout_ms),
        max_wall_time_ms: Some(config.max_wall_time_ms),
    })
}

pub fn operation_supports_streaming(operation: &ModelOperation) -> bool {
    matches!(
        operation,
        ModelOperation::ChatTurn
            | ModelOperation::Summarize
            | ModelOperation::Rewrite
            | ModelOperation::GenerateArtifact
            | ModelOperation::RenderEntityReply
    )
}

#[derive(Clone, Debug, Default)]
struct PartialToolCall {
    id: Option<String>,
    call_type: Option<String>,
    function_name: String,
    function_arguments: String,
}

fn accumulate_tool_call_deltas(value: Option<&Value>, tool_calls: &mut Vec<PartialToolCall>) {
    let Some(items) = value.and_then(Value::as_array) else {
        return;
    };
    for (fallback_index, item) in items.iter().enumerate() {
        let index = item
            .get("index")
            .and_then(Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .unwrap_or(fallback_index);
        while tool_calls.len() <= index {
            tool_calls.push(PartialToolCall::default());
        }
        let call = &mut tool_calls[index];
        if let Some(id) = item.get("id").and_then(Value::as_str) {
            if !id.is_empty() {
                call.id = Some(id.to_string());
            }
        }
        if let Some(call_type) = item.get("type").and_then(Value::as_str) {
            if !call_type.is_empty() {
                call.call_type = Some(call_type.to_string());
            }
        }
        let Some(function) = item.get("function") else {
            continue;
        };
        if let Some(name) = function.get("name").and_then(Value::as_str) {
            if !name.is_empty() {
                call.function_name.push_str(name);
            }
        }
        if let Some(arguments) = function.get("arguments").and_then(Value::as_str) {
            if !arguments.is_empty() {
                call.function_arguments.push_str(arguments);
            }
        }
    }
}

fn materialize_tool_calls(tool_calls: Vec<PartialToolCall>) -> Vec<ProviderToolCall> {
    tool_calls
        .into_iter()
        .enumerate()
        .filter_map(|(index, call)| {
            if call.id.is_none()
                && call.function_name.trim().is_empty()
                && call.function_arguments.trim().is_empty()
            {
                return None;
            }
            Some(ProviderToolCall {
                id: call.id.unwrap_or_else(|| format!("tool_call_{index}")),
                r#type: call.call_type.unwrap_or_else(|| "function".to_string()),
                function: json!({
                    "name": call.function_name,
                    "arguments": call.function_arguments,
                }),
            })
        })
        .collect()
}

pub fn validate_finish_reason(finish_reason: &str) -> Result<(), ModelProviderFailure> {
    match finish_reason {
        "length" => Err(ModelProviderFailure {
            error_code: "MODEL_OUTPUT_TRUNCATED".to_string(),
            message: "model output was truncated by provider finish_reason=length".to_string(),
            retryable: false,
        }),
        "content_filter" => Err(ModelProviderFailure {
            error_code: "MODEL_CONTENT_FILTERED".to_string(),
            message: "model output was filtered by provider".to_string(),
            retryable: false,
        }),
        "insufficient_system_resource" => Err(ModelProviderFailure {
            error_code: "MODEL_UNAVAILABLE".to_string(),
            message: "provider reported insufficient system resource".to_string(),
            retryable: true,
        }),
        _ => Ok(()),
    }
}

fn extract_response_text(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) => Some(text.clone()),
        Value::Array(items) => {
            let mut parts = Vec::new();
            for item in items {
                if let Some(text) = item.get("text").and_then(Value::as_str) {
                    parts.push(text.to_string());
                } else if let Some(text) = item.as_str() {
                    parts.push(text.to_string());
                }
            }
            Some(parts.join("\n"))
        }
        other if !other.is_null() => Some(other.to_string()),
        _ => None,
    }
}

fn stream_failure(error_code: &str, message: String, retryable: bool) -> ModelProviderFailure {
    ModelProviderFailure {
        error_code: error_code.to_string(),
        message,
        retryable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::BTreeMap;
    use std::sync::Mutex;

    use crate::model_config::ModelInvocationConfig;
    use crate::model_runtime::{ModelAction, ModelBudget, ModelFailurePolicy};

    #[derive(Debug, Default)]
    struct RecordingSink {
        deltas: Mutex<Vec<ModelStreamDelta>>,
    }

    impl RecordingSink {
        fn deltas(&self) -> Vec<ModelStreamDelta> {
            self.deltas
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .clone()
        }
    }

    impl ModelStreamSink for RecordingSink {
        fn on_model_stream_delta(&self, delta: ModelStreamDelta) {
            self.deltas
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .push(delta);
        }
    }

    fn provider_request(operation: ModelOperation) -> ModelProviderRequest {
        ModelProviderRequest {
            model_call_id: "model_call_stream_parser".into(),
            action: ModelAction {
                action_id: "act_stream_parser".into(),
                job_id: "job_stream_parser".into(),
                pid: "pid_stream_parser".into(),
                reasoning_step_id: "reason_stream_parser".into(),
                operation,
                instruction_ref: "blob://job_stream_parser/instruction".into(),
                input_refs: vec!["blob://job_stream_parser/input".into()],
                preference_snapshot_ref: None,
                output_schema: json!({"type": "string"}),
                provider: "deepseek".into(),
                model: "deepseek-v4-flash".into(),
                budget: ModelBudget::default(),
                failure_policy: ModelFailurePolicy::FailClosed,
                required: true,
            },
            input_payloads: BTreeMap::new(),
            capability_snapshot: json!({}),
            model_config: ModelInvocationConfig::default(),
            client_locale_context_ref: None,
            client_locale_context: None,
            provider_tools: Vec::new(),
            provider_tool_choice: None,
            provider_transcript_messages: Vec::new(),
            provider_toolset_ref: None,
            current_user_message_required: false,
        }
    }

    fn stream_config() -> DeepSeekStreamConfig {
        DeepSeekStreamConfig {
            first_byte_timeout_ms: 1_000,
            idle_timeout_ms: 1_000,
            max_wall_time_ms: 10_000,
        }
    }

    #[test]
    fn deepseek_stream_reader_forwards_answer_chunks_to_sink() {
        let body = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"Hel\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"think\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"lo\"},\"finish_reason\":\"stop\"}],\"usage\":{\"completion_tokens\":2}}\n\n",
            "data: [DONE]\n\n",
        );
        let response = ureq::Response::new(200, "OK", body).unwrap();
        let request = provider_request(ModelOperation::ChatTurn);
        let sink = RecordingSink::default();

        let parsed =
            read_deepseek_streaming_response(response, &request, &stream_config(), Some(&sink))
                .unwrap();

        assert_eq!(parsed.output_text, "Hello");
        assert_eq!(parsed.reasoning_content.as_deref(), Some("think"));
        assert!(parsed.streaming);
        assert_eq!(parsed.chunks_count, 2);
        let deltas = sink.deltas();
        assert_eq!(deltas.len(), 3);
        assert_eq!(deltas[0].kind, ModelStreamDeltaKind::Answer);
        assert_eq!(deltas[0].sequence, 1);
        assert_eq!(deltas[0].delta, "Hel");
        assert_eq!(deltas[1].kind, ModelStreamDeltaKind::Reasoning);
        assert_eq!(deltas[2].kind, ModelStreamDeltaKind::Answer);
        assert_eq!(deltas[2].sequence, 2);
        assert_eq!(deltas[2].delta, "lo");
    }

    #[test]
    fn deepseek_stream_reader_materializes_tool_call_only_streams() {
        let body = concat!(
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"type\":\"function\",\"function\":{\"name\":\"workspace.read_file\",\"arguments\":\"{\\\"path\\\":\"}}]}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"\\\"README.md\\\"}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\n",
            "data: [DONE]\n\n",
        );
        let response = ureq::Response::new(200, "OK", body).unwrap();
        let request = provider_request(ModelOperation::ChatTurn);

        let parsed =
            read_deepseek_streaming_response(response, &request, &stream_config(), None).unwrap();

        assert_eq!(parsed.output_text, "");
        assert_eq!(parsed.tool_calls.len(), 1);
        assert_eq!(parsed.tool_calls[0].id, "call_1");
        assert_eq!(
            parsed.tool_calls[0]
                .function
                .get("name")
                .and_then(Value::as_str),
            Some("workspace.read_file")
        );
        assert_eq!(
            parsed.tool_calls[0]
                .function
                .get("arguments")
                .and_then(Value::as_str),
            Some("{\"path\":\"README.md\"}")
        );
    }

    #[test]
    fn chat_turn_operations_support_streaming() {
        assert!(operation_supports_streaming(&ModelOperation::ChatTurn));
        assert!(
            crate::model_runtime::operation_supports_task_reasoning_stream(
                &ModelOperation::DecideNextAction
            )
        );
    }
}
