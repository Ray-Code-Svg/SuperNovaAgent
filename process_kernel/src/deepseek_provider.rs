use std::env;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use crate::model_retry::{classify_io_transport_retryable, retryable_transport_message};
use crate::model_runtime::{
    operation_supports_task_reasoning_stream, DeterministicModelProvider, MissingModelProvider,
    ModelOperation, ModelProvider, ModelProviderFailure, ModelProviderRequest,
    ModelProviderResponse, ModelStreamSink, ProviderAssistantMessage, ProviderToolCall,
};
use crate::model_stream::{
    operation_supports_streaming, read_deepseek_streaming_response, validate_finish_reason,
    DeepSeekStreamConfig,
};

pub const DEEPSEEK_OFFICIAL_BASE_URL: &str = "https://api.deepseek.com";

#[derive(Clone, Debug)]
pub struct DeepSeekModelProvider {
    api_key: String,
    base_url: String,
    flash_model: String,
    pro_model: String,
    timeout_ms: u64,
    streaming_enabled: bool,
    stream_first_byte_timeout_ms: u64,
    stream_idle_timeout_ms: u64,
    stream_max_wall_time_ms: u64,
}

impl DeepSeekModelProvider {
    pub fn from_env() -> Result<Self, ModelProviderFailure> {
        let api_key = env::var("SUPERNOVA_DEEPSEEK_API_KEY")
            .or_else(|_| env::var("DEEPSEEK_API_KEY"))
            .map_err(|_| ModelProviderFailure {
                error_code: "DEEPSEEK_API_KEY_MISSING".to_string(),
                message: "SUPERNOVA_DEEPSEEK_API_KEY or DEEPSEEK_API_KEY is required for live V2 ModelRuntime calls".to_string(),
                retryable: false,
            })?;
        let flash_model = deepseek_flash_model_from_env();
        let pro_model = deepseek_pro_model_from_env();
        Ok(Self::new(
            api_key,
            deepseek_base_url_default(),
            flash_model.clone(),
            deepseek_timeout_ms_from_env(),
        )
        .with_route_models(flash_model, pro_model))
    }

    pub fn from_resolved_credential(
        api_key: impl Into<String>,
        base_url: impl Into<String>,
    ) -> Self {
        let flash_model = deepseek_flash_model_from_env();
        let pro_model = deepseek_pro_model_from_env();
        Self::new(
            api_key,
            base_url,
            flash_model.clone(),
            deepseek_timeout_ms_from_env(),
        )
        .with_route_models(flash_model, pro_model)
    }

    pub fn new(
        api_key: impl Into<String>,
        base_url: impl Into<String>,
        model: impl Into<String>,
        timeout_ms: u64,
    ) -> Self {
        let model = model.into();
        Self {
            api_key: api_key.into(),
            base_url: base_url.into(),
            flash_model: model.clone(),
            pro_model: model,
            timeout_ms,
            streaming_enabled: deepseek_streaming_enabled_from_env(),
            stream_first_byte_timeout_ms: deepseek_stream_first_byte_timeout_ms_from_env(),
            stream_idle_timeout_ms: deepseek_stream_idle_timeout_ms_from_env(),
            stream_max_wall_time_ms: deepseek_stream_max_wall_time_ms_from_env(),
        }
    }

    pub fn with_route_models(
        mut self,
        flash_model: impl Into<String>,
        pro_model: impl Into<String>,
    ) -> Self {
        self.flash_model = flash_model.into();
        self.pro_model = pro_model.into();
        self
    }

    pub fn with_streaming(mut self, enabled: bool) -> Self {
        self.streaming_enabled = enabled;
        self
    }

    pub fn with_stream_timeouts(
        mut self,
        first_byte_timeout_ms: u64,
        idle_timeout_ms: u64,
        max_wall_time_ms: u64,
    ) -> Self {
        self.stream_first_byte_timeout_ms = first_byte_timeout_ms;
        self.stream_idle_timeout_ms = idle_timeout_ms;
        self.stream_max_wall_time_ms = max_wall_time_ms;
        self
    }

    fn chat_completions_url(&self) -> String {
        let trimmed = self.base_url.trim().trim_end_matches('/');
        if trimmed.ends_with("/chat/completions") {
            trimmed.to_string()
        } else {
            format!("{trimmed}/chat/completions")
        }
    }
}

impl ModelProvider for DeepSeekModelProvider {
    fn provider_name(&self) -> &str {
        "deepseek"
    }

    fn model_name(&self) -> &str {
        &self.flash_model
    }

    fn model_name_for_operation(&self, operation: &ModelOperation) -> String {
        match deepseek_route_for_operation(operation) {
            DeepSeekModelRoute::Flash => self.flash_model.clone(),
            DeepSeekModelRoute::Pro => self.pro_model.clone(),
        }
    }

    fn capability_snapshot(&self) -> Value {
        json!({
            "provider": "deepseek",
            "model": self.flash_model,
            "protocol": "openai_compatible_chat_completions",
            "base_url": self.base_url,
            "chat_completions_url": self.chat_completions_url(),
            "routing": {
                "simple_model": self.flash_model,
                "complex_model": self.pro_model,
                "flash_operations": ["decide_next_action", "extract_json", "summarize", "render_entity_reply"],
                "pro_operations": ["rewrite", "generate_artifact", "audit"]
            },
            "supports_operations": [
                "decide_next_action",
                "extract_json",
                "summarize",
                "rewrite",
                "generate_artifact",
                "audit",
                "render_entity_reply"
            ],
            "supports_schema_validation": true,
            "supports_ledger": true,
            "supports_thinking": true,
            "supports_reasoning_content": true,
            "reasoning_effort_values": ["high", "max"],
            "supports_streaming": true,
            "streaming_enabled": self.streaming_enabled,
            "streaming_policy": "chat_text_and_product_task_reasoning_operations",
            "stream_first_byte_timeout_ms": self.stream_first_byte_timeout_ms,
            "stream_idle_timeout_ms": self.stream_idle_timeout_ms,
            "stream_max_wall_time_ms": self.stream_max_wall_time_ms,
            "live_api": true,
        })
    }

    fn invoke(
        &self,
        request: &ModelProviderRequest,
    ) -> Result<ModelProviderResponse, ModelProviderFailure> {
        self.invoke_deepseek(request, None)
    }

    fn invoke_with_stream_sink(
        &self,
        request: &ModelProviderRequest,
        stream_sink: Option<Arc<dyn ModelStreamSink>>,
    ) -> Result<ModelProviderResponse, ModelProviderFailure> {
        self.invoke_deepseek(request, stream_sink)
    }
}

impl DeepSeekModelProvider {
    fn invoke_deepseek(
        &self,
        request: &ModelProviderRequest,
        stream_sink: Option<Arc<dyn ModelStreamSink>>,
    ) -> Result<ModelProviderResponse, ModelProviderFailure> {
        let native_tool_request = !request.provider_tools.is_empty();
        let streaming = self.streaming_enabled
            && (operation_supports_streaming(&request.action.operation)
                || (stream_sink.is_some()
                    && operation_supports_task_reasoning_stream(&request.action.operation)));
        let selected_model = request.action.model.clone();
        let timeout_ms = request.action.budget.timeout_ms.min(self.timeout_ms);
        let agent = if streaming {
            ureq::AgentBuilder::new()
                .timeout_connect(Duration::from_millis(
                    self.stream_first_byte_timeout_ms.min(timeout_ms),
                ))
                .timeout_write(Duration::from_millis(
                    self.stream_first_byte_timeout_ms.min(timeout_ms),
                ))
                .timeout_read(Duration::from_millis(self.stream_idle_timeout_ms))
                .build()
        } else {
            ureq::AgentBuilder::new()
                .timeout(Duration::from_millis(timeout_ms))
                .build()
        };
        let mut payload = json!({
            "model": selected_model,
            "messages": render_deepseek_messages(request),
            "max_tokens": request.action.budget.max_output_tokens,
            "stream": streaming
        });
        let thinking_type = request.model_config.thinking.mode.deepseek_type();
        payload["thinking"] = json!({"type": thinking_type});
        if request.model_config.thinking.mode.is_effectively_enabled() {
            payload["reasoning_effort"] = json!(request
                .model_config
                .thinking
                .reasoning_effort
                .as_deepseek_value());
        } else {
            payload["temperature"] = json!(0.2);
        }
        if streaming {
            payload["stream_options"] = json!({"include_usage": true});
        }
        if !native_tool_request
            && output_schema_requests_json(&request.action.operation, &request.action.output_schema)
        {
            payload["response_format"] = json!({"type": "json_object"});
        }
        if native_tool_request {
            payload["tools"] = serde_json::to_value(&request.provider_tools)
                .map_err(deepseek_request_json_error)?;
            if let Some(tool_choice) = request.provider_tool_choice.clone() {
                payload["tool_choice"] = tool_choice;
            }
        }
        let response = agent
            .post(&self.chat_completions_url())
            .set("Authorization", &format!("Bearer {}", self.api_key))
            .set("Content-Type", "application/json")
            .set(
                "Accept",
                if streaming {
                    "text/event-stream"
                } else {
                    "application/json"
                },
            )
            .send_json(payload)
            .map_err(deepseek_http_error)?;
        if streaming {
            return read_deepseek_streaming_response(
                response,
                request,
                &DeepSeekStreamConfig {
                    first_byte_timeout_ms: self.stream_first_byte_timeout_ms,
                    idle_timeout_ms: self.stream_idle_timeout_ms,
                    max_wall_time_ms: self.stream_max_wall_time_ms,
                },
                stream_sink.as_deref(),
            );
        }
        let raw: Value = response.into_json().map_err(deepseek_response_json_error)?;
        let message = raw
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first())
            .and_then(|choice| choice.get("message"))
            .cloned()
            .unwrap_or_else(|| json!({}));
        let output_text = message
            .get("content")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_default();
        let reasoning_content = message
            .get("reasoning_content")
            .and_then(Value::as_str)
            .map(str::to_string);
        let tool_calls = parse_provider_tool_calls(message.get("tool_calls"));
        if output_text.trim().is_empty() && tool_calls.is_empty() {
            return Err(ModelProviderFailure {
                error_code: "DEEPSEEK_RESPONSE_CONTENT_MISSING".to_string(),
                message: "DeepSeek response did not contain choices[0].message.content".to_string(),
                retryable: false,
            });
        }
        let finish_reason = raw
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first())
            .and_then(|choice| choice.get("finish_reason"))
            .and_then(Value::as_str)
            .map(str::to_string);
        if let Some(reason) = finish_reason.as_deref() {
            validate_finish_reason(reason)?;
        }
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
                tool_calls: tool_calls.clone(),
            }),
            reasoning_content,
            tool_calls,
            usage: raw.get("usage").cloned().unwrap_or_else(|| json!({})),
            finish_reason,
            raw,
            sampling_ignored_by_provider: request.model_config.sampling_ignored_by_provider(),
            streaming: false,
            first_token_ms: None,
            chunks_count: 0,
            stream_event_count: 0,
            first_byte_timeout_ms: None,
            idle_timeout_ms: None,
            max_wall_time_ms: None,
        })
    }
}

pub fn default_model_provider_from_env() -> Arc<dyn ModelProvider> {
    if let Ok(raw_fixture) = env::var("SUPERNOVA_DETERMINISTIC_PROVIDER_JSON") {
        return deterministic_provider_from_fixture_json(&raw_fixture).unwrap_or_else(|err| {
            Arc::new(MissingModelProvider::new(
                "deterministic",
                "fixture-invalid",
                format!("invalid SUPERNOVA_DETERMINISTIC_PROVIDER_JSON: {err}"),
            ))
        });
    }
    match DeepSeekModelProvider::from_env() {
        Ok(provider) => Arc::new(provider),
        Err(err) => Arc::new(MissingModelProvider::new(
            "deepseek",
            deepseek_flash_model_from_env(),
            err.message,
        )),
    }
}

pub fn deepseek_provider_from_resolved_credential(
    api_key: impl Into<String>,
    base_url: impl Into<String>,
) -> DeepSeekModelProvider {
    DeepSeekModelProvider::from_resolved_credential(api_key, base_url)
}

pub fn deepseek_base_url_default() -> String {
    DEEPSEEK_OFFICIAL_BASE_URL.to_string()
}

fn deterministic_provider_from_fixture_json(
    raw_fixture: &str,
) -> Result<Arc<dyn ModelProvider>, String> {
    let value = serde_json::from_str::<Value>(raw_fixture).map_err(|err| err.to_string())?;
    let provider_name = value
        .get("provider")
        .and_then(Value::as_str)
        .unwrap_or("deterministic");
    let model_name = value
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("fixture-model");
    let mut provider = DeterministicModelProvider::new(provider_name, model_name);
    if let Some(outputs) = value.get("outputs").and_then(Value::as_object) {
        for (operation, output) in outputs {
            provider = provider.with_output_for_operation(
                operation.clone(),
                output.as_str().unwrap_or("").to_string(),
            );
        }
    }
    if let Some(tool_calls_by_operation) = value.get("tool_calls").and_then(Value::as_object) {
        for (operation, tool_calls) in tool_calls_by_operation {
            let calls = serde_json::from_value::<Vec<ProviderToolCall>>(tool_calls.clone())
                .map_err(|err| format!("invalid tool_calls for {operation}: {err}"))?;
            provider = provider.with_tool_calls_for_operation(operation.clone(), calls);
        }
    }
    Ok(Arc::new(provider))
}

fn deepseek_fixed_model_from_env() -> Option<String> {
    env::var("SUPERNOVA_DEEPSEEK_MODEL")
        .or_else(|_| env::var("DEEPSEEK_MODEL"))
        .ok()
        .filter(|value| !value.trim().is_empty())
}

fn deepseek_flash_model_from_env() -> String {
    env::var("SUPERNOVA_DEEPSEEK_FLASH_MODEL")
        .or_else(|_| env::var("DEEPSEEK_FLASH_MODEL"))
        .ok()
        .or_else(deepseek_fixed_model_from_env)
        .unwrap_or_else(|| "deepseek-v4-flash".to_string())
}

fn deepseek_pro_model_from_env() -> String {
    env::var("SUPERNOVA_DEEPSEEK_PRO_MODEL")
        .or_else(|_| env::var("DEEPSEEK_PRO_MODEL"))
        .ok()
        .or_else(deepseek_fixed_model_from_env)
        .unwrap_or_else(|| "deepseek-v4-pro".to_string())
}

fn deepseek_timeout_ms_from_env() -> u64 {
    env_ms_or_seconds(
        &["SUPERNOVA_DEEPSEEK_TIMEOUT_MS"],
        &["LLM_REQUEST_TIMEOUT_SEC"],
        120_000,
    )
}

fn deepseek_streaming_enabled_from_env() -> bool {
    env_bool(
        &[
            "SUPERNOVA_DEEPSEEK_STREAMING",
            "SUPERNOVA_MODEL_STREAMING",
            "LLM_ARTIFACT_CONTENT_STREAMING",
        ],
        true,
    )
}

fn deepseek_stream_first_byte_timeout_ms_from_env() -> u64 {
    env_ms_or_seconds(
        &["SUPERNOVA_DEEPSEEK_STREAM_FIRST_BYTE_TIMEOUT_MS"],
        &["LLM_STREAM_FIRST_BYTE_TIMEOUT_SEC"],
        30_000,
    )
}

fn deepseek_stream_idle_timeout_ms_from_env() -> u64 {
    env_ms_or_seconds(
        &["SUPERNOVA_DEEPSEEK_STREAM_IDLE_TIMEOUT_MS"],
        &["LLM_STREAM_IDLE_TIMEOUT_SEC"],
        60_000,
    )
}

fn deepseek_stream_max_wall_time_ms_from_env() -> u64 {
    env_ms_or_seconds(
        &["SUPERNOVA_DEEPSEEK_STREAM_MAX_WALL_TIME_MS"],
        &["LLM_STREAM_MAX_WALL_TIME_SEC"],
        0,
    )
}

fn env_bool(names: &[&str], default: bool) -> bool {
    for name in names {
        if let Ok(value) = env::var(name) {
            return matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            );
        }
    }
    default
}

fn env_ms_or_seconds(ms_names: &[&str], sec_names: &[&str], default_ms: u64) -> u64 {
    for name in ms_names {
        if let Ok(value) = env::var(name) {
            if let Ok(parsed) = value.trim().parse::<u64>() {
                return parsed;
            }
        }
    }
    for name in sec_names {
        if let Ok(value) = env::var(name) {
            if let Ok(parsed) = value.trim().parse::<f64>() {
                if parsed > 0.0 {
                    return (parsed * 1000.0).round() as u64;
                }
            }
        }
    }
    default_ms
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DeepSeekModelRoute {
    Flash,
    Pro,
}

fn deepseek_route_for_operation(operation: &ModelOperation) -> DeepSeekModelRoute {
    match operation {
        ModelOperation::ChatTurn
        | ModelOperation::CompactContainerContext
        | ModelOperation::CompactChatContext
        | ModelOperation::CompactTaskContext
        | ModelOperation::ExtractJson
        | ModelOperation::Summarize
        | ModelOperation::RenderEntityReply
        | ModelOperation::DecideNextAction => DeepSeekModelRoute::Flash,
        ModelOperation::Rewrite | ModelOperation::GenerateArtifact | ModelOperation::Audit => {
            DeepSeekModelRoute::Pro
        }
    }
}

fn output_schema_requests_json(operation: &ModelOperation, output_schema: &Value) -> bool {
    matches!(
        operation,
        ModelOperation::DecideNextAction
            | ModelOperation::CompactContainerContext
            | ModelOperation::CompactChatContext
            | ModelOperation::CompactTaskContext
            | ModelOperation::ExtractJson
            | ModelOperation::Audit
    ) || output_schema
        .get("type")
        .and_then(Value::as_str)
        .is_some_and(|schema_type| matches!(schema_type, "object"))
}

fn render_deepseek_prompt(request: &ModelProviderRequest) -> String {
    let mut lines = vec![
        format!("operation: {}", request.action.operation.as_str()),
        format!("model_call_id: {}", request.model_call_id),
        format!("required: {}", request.action.required),
        format!("failure_policy: {}", request.action.failure_policy.as_str()),
        "output_schema:".to_string(),
        serde_json::to_string_pretty(&request.action.output_schema)
            .unwrap_or_else(|_| "{}".to_string()),
        "inputs:".to_string(),
    ];
    for (input_ref, payload) in &request.input_payloads {
        lines.push(format!("--- {input_ref} ---"));
        lines.push(payload.clone());
    }
    lines.join("\n")
}

fn render_deepseek_messages(request: &ModelProviderRequest) -> Vec<Value> {
    let base_system_content = format!(
        "{}\n\n{}",
        "You are the SuperNova V2 Model Runtime. Return only the requested output. Obey the output schema. Never produce placeholder, template fallback, or unverifiable content.",
        request.model_config.response_language.prompt_instruction()
    );
    let system_content = match request.client_locale_context.as_ref() {
        Some(context) => format!(
            "{}\n\n{}",
            base_system_content,
            render_client_locale_context_block(context)
        ),
        None => base_system_content,
    };
    let current_user_message = json!({
        "role": "user",
        "content": render_deepseek_prompt(request)
    });
    let mut transcript_messages = request
        .provider_transcript_messages
        .iter()
        .filter_map(|message| serde_json::to_value(message).ok())
        .collect::<Vec<_>>();
    let transcript_has_user = request
        .provider_transcript_messages
        .iter()
        .any(|message| message.role == "user");
    let transcript_ends_with_tool = request
        .provider_transcript_messages
        .last()
        .is_some_and(|message| message.role == "tool");
    let mut messages = vec![json!({
        "role": "system",
        "content": system_content
    })];
    if transcript_has_user {
        messages.append(&mut transcript_messages);
        if !transcript_ends_with_tool || request.current_user_message_required {
            messages.push(current_user_message);
        }
    } else {
        messages.push(current_user_message);
        messages.append(&mut transcript_messages);
    }
    messages
}

fn render_client_locale_context_block(context: &crate::ClientLocaleContext) -> String {
    format!(
        "[Client Context]\n- origin: {}\n- os_family: {}\n- timezone_id: {}\n- utc_offset_minutes: {}\n- locale: {}\n- current_local_datetime: {}\n\nUse this context for date/time interpretation, locale-sensitive wording, filesystem/platform assumptions, and scheduling reasoning. Do not infer sensitive identity from it.",
        context.origin,
        context.os_family,
        context
            .timezone_id
            .as_deref()
            .unwrap_or("unknown"),
        context
            .utc_offset_minutes
            .map(|value| value.to_string())
            .unwrap_or_else(|| "unknown".to_string()),
        context.locale.as_deref().unwrap_or("unknown"),
        context.current_local_datetime,
    )
}

fn parse_provider_tool_calls(value: Option<&Value>) -> Vec<ProviderToolCall> {
    let Some(items) = value.and_then(Value::as_array) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let id = item.get("id").and_then(Value::as_str)?.to_string();
            let r#type = item
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("function")
                .to_string();
            let function = item.get("function").cloned().unwrap_or_else(|| json!({}));
            Some(ProviderToolCall {
                id,
                r#type,
                function,
            })
        })
        .collect()
}

fn deepseek_request_json_error(err: serde_json::Error) -> ModelProviderFailure {
    ModelProviderFailure {
        error_code: "DEEPSEEK_REQUEST_JSON_INVALID".to_string(),
        message: err.to_string(),
        retryable: false,
    }
}

fn deepseek_http_error(err: ureq::Error) -> ModelProviderFailure {
    match err {
        ureq::Error::Status(code, response) => {
            let body = response
                .into_string()
                .unwrap_or_else(|_| "<unreadable response body>".to_string());
            ModelProviderFailure {
                error_code: format!("DEEPSEEK_HTTP_{code}"),
                message: body,
                retryable: code == 429 || code >= 500,
            }
        }
        ureq::Error::Transport(err) => ModelProviderFailure {
            error_code: "DEEPSEEK_TRANSPORT_ERROR".to_string(),
            message: err.to_string(),
            retryable: true,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ClientLocaleContext, ModelAction, ModelBudget, ModelFailurePolicy, ModelInvocationConfig,
        ProviderTranscriptMessage, ResponseLanguage,
    };
    use std::collections::BTreeMap;

    fn base_request(
        input: &str,
        provider_transcript_messages: Vec<ProviderTranscriptMessage>,
    ) -> ModelProviderRequest {
        ModelProviderRequest {
            model_call_id: "mcall_test".to_string(),
            action: ModelAction {
                action_id: "act_test".to_string(),
                job_id: "job_test".to_string(),
                pid: "pid_test".to_string(),
                reasoning_step_id: "reason_test".to_string(),
                operation: ModelOperation::ChatTurn,
                instruction_ref: "blob://job_test/instruction.txt".to_string(),
                input_refs: vec!["blob://job_test/input.txt".to_string()],
                preference_snapshot_ref: None,
                output_schema: json!({"type": "object"}),
                provider: "deepseek".to_string(),
                model: "deepseek-chat".to_string(),
                budget: ModelBudget::default(),
                failure_policy: ModelFailurePolicy::FailClosed,
                required: true,
            },
            input_payloads: BTreeMap::from([(
                "blob://job_test/input.txt".to_string(),
                input.to_string(),
            )]),
            capability_snapshot: json!({}),
            model_config: ModelInvocationConfig::default(),
            client_locale_context_ref: None,
            client_locale_context: None,
            provider_tools: Vec::new(),
            provider_tool_choice: None,
            provider_transcript_messages,
            provider_toolset_ref: None,
            current_user_message_required: false,
        }
    }

    #[test]
    fn deepseek_system_message_includes_low_sensitive_client_context() {
        let request = ModelProviderRequest {
            model_call_id: "mcall_locale".to_string(),
            client_locale_context_ref: Some(
                "blob://job_locale/client_locale_contexts/mcall_locale.json".to_string(),
            ),
            client_locale_context: Some(ClientLocaleContext {
                schema_version: crate::CLIENT_LOCALE_CONTEXT_SCHEMA_VERSION.to_string(),
                captured_at_unix_ms: 1,
                origin: crate::CLIENT_ENV_ORIGIN.to_string(),
                os_family: "windows".to_string(),
                timezone_id: Some("China Standard Time".to_string()),
                utc_offset_minutes: Some(480),
                locale: Some("zh-CN".to_string()),
                current_local_datetime: "2026-05-31T12:00:00+08:00".to_string(),
                sensitivity: "low".to_string(),
            }),
            ..base_request("", Vec::new())
        };
        let messages = render_deepseek_messages(&request);
        let system = messages[0]["content"].as_str().unwrap();
        assert!(system.contains("[Client Context]"));
        assert!(system.contains("timezone_id: China Standard Time"));
        assert!(system.contains("locale: zh-CN"));
        assert!(system.contains("current_local_datetime: 2026-05-31T12:00:00+08:00"));
        assert!(!system.contains("local_ip"));
        assert!(!system.contains("mac"));
    }

    #[test]
    fn deepseek_system_message_uses_response_language() {
        let mut zh_request = base_request("return a final answer", Vec::new());
        zh_request.model_config.response_language = ResponseLanguage::ZhCn;
        let zh_messages = render_deepseek_messages(&zh_request);
        let zh_system = zh_messages[0]["content"].as_str().unwrap();

        let mut en_request = base_request("return a final answer", Vec::new());
        en_request.model_config.response_language = ResponseLanguage::EnUs;
        let en_messages = render_deepseek_messages(&en_request);
        let en_system = en_messages[0]["content"].as_str().unwrap();

        assert!(zh_system.contains("Use Simplified Chinese"));
        assert!(en_system.contains("Use English"));
        assert!(zh_system.contains("Obey the output schema"));
        assert!(en_system.contains("JSON keys"));
    }

    #[test]
    fn deepseek_places_current_user_after_user_backed_transcript() {
        let request = base_request(
            "second turn",
            vec![
                ProviderTranscriptMessage {
                    role: "user".to_string(),
                    content: Some("first turn".to_string()),
                    reasoning_content: None,
                    tool_calls: Vec::new(),
                    tool_call_id: None,
                },
                ProviderTranscriptMessage {
                    role: "assistant".to_string(),
                    content: Some("first answer".to_string()),
                    reasoning_content: None,
                    tool_calls: Vec::new(),
                    tool_call_id: None,
                },
            ],
        );

        let messages = render_deepseek_messages(&request);

        assert_eq!(messages[0]["role"], "system");
        assert_eq!(messages[1]["role"], "user");
        assert_eq!(messages[1]["content"], "first turn");
        assert_eq!(messages[2]["role"], "assistant");
        assert_eq!(messages[2]["content"], "first answer");
        assert_eq!(messages[3]["role"], "user");
        assert!(messages[3]["content"]
            .as_str()
            .unwrap()
            .contains("second turn"));
    }

    #[test]
    fn deepseek_does_not_repeat_current_user_after_tool_result() {
        let request = base_request(
            "same turn",
            vec![
                ProviderTranscriptMessage {
                    role: "user".to_string(),
                    content: Some("same turn".to_string()),
                    reasoning_content: None,
                    tool_calls: Vec::new(),
                    tool_call_id: None,
                },
                ProviderTranscriptMessage {
                    role: "assistant".to_string(),
                    content: None,
                    reasoning_content: None,
                    tool_calls: vec![ProviderToolCall {
                        id: "call_1".to_string(),
                        r#type: "function".to_string(),
                        function: json!({
                            "name": "os.read_file",
                            "arguments": "{\"path\":\"README.md\"}",
                        }),
                    }],
                    tool_call_id: None,
                },
                ProviderTranscriptMessage {
                    role: "tool".to_string(),
                    content: Some("file content".to_string()),
                    reasoning_content: None,
                    tool_calls: Vec::new(),
                    tool_call_id: Some("call_1".to_string()),
                },
            ],
        );

        let messages = render_deepseek_messages(&request);
        let user_count = messages
            .iter()
            .filter(|message| message["role"] == "user")
            .count();

        assert_eq!(messages.last().unwrap()["role"], "tool");
        assert_eq!(user_count, 1);
    }

    #[test]
    fn deepseek_appends_new_user_turn_after_dangling_tool_result() {
        let mut request = base_request(
            "new turn",
            vec![
                ProviderTranscriptMessage {
                    role: "user".to_string(),
                    content: Some("previous turn".to_string()),
                    reasoning_content: None,
                    tool_calls: Vec::new(),
                    tool_call_id: None,
                },
                ProviderTranscriptMessage {
                    role: "assistant".to_string(),
                    content: None,
                    reasoning_content: None,
                    tool_calls: vec![ProviderToolCall {
                        id: "call_1".to_string(),
                        r#type: "function".to_string(),
                        function: json!({
                            "name": "os.read_file",
                            "arguments": "{\"path\":\"README.md\"}",
                        }),
                    }],
                    tool_call_id: None,
                },
                ProviderTranscriptMessage {
                    role: "tool".to_string(),
                    content: Some("file content".to_string()),
                    reasoning_content: None,
                    tool_calls: Vec::new(),
                    tool_call_id: Some("call_1".to_string()),
                },
            ],
        );
        request.current_user_message_required = true;

        let messages = render_deepseek_messages(&request);

        assert_eq!(messages.last().unwrap()["role"], "user");
        assert!(messages.last().unwrap()["content"]
            .as_str()
            .unwrap()
            .contains("new turn"));
    }
}

fn deepseek_response_json_error(err: std::io::Error) -> ModelProviderFailure {
    let message = err.to_string();
    let retryable = classify_io_transport_retryable(err.kind(), &message);
    ModelProviderFailure {
        error_code: if retryable {
            "DEEPSEEK_RESPONSE_TRANSPORT_ERROR".to_string()
        } else {
            "DEEPSEEK_RESPONSE_JSON_INVALID".to_string()
        },
        message,
        retryable,
    }
}

#[allow(dead_code)]
fn deepseek_transport_message_is_retryable(message: &str) -> bool {
    retryable_transport_message(message)
}
