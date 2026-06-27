use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub use crate::deepseek_provider::{default_model_provider_from_env, DeepSeekModelProvider};
use crate::model_config::{estimate_text_tokens_conservative, ModelInvocationConfig};
use crate::model_retry::RetryPolicy;
use crate::provider_debug::append_provider_native_debug;
use crate::provider_tool::{
    provider_native_tool_request_enabled, provider_tool_choice_value, ProviderToolDefinition,
};
use crate::provider_toolset::{ProviderToolsetPlan, ProviderToolsetPlanner};
use crate::provider_transcript::{
    read_provider_messages, record_provider_assistant_response, record_provider_user_message,
    replay_provider_transcript_state, ProviderTranscriptMessage,
};
use crate::{
    default_capability_registry, CapabilityReceipt, CapabilityToken, ClientEnvRuntime,
    ClientLocaleContext, ModelContextProfile, ProcessTruthStore,
};

#[derive(Clone, Debug)]
pub struct ModelRuntime {
    truth: ProcessTruthStore,
    token: CapabilityToken,
    provider: Arc<dyn ModelProvider>,
    model_config: ModelInvocationConfig,
    model_invocation_config_ref: Option<String>,
    preplanned_provider_toolset: Option<ProviderToolsetPlan>,
    model_call_id_override: Option<String>,
    stream_sink: Option<Arc<dyn ModelStreamSink>>,
    record_provider_user_message: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModelOperation {
    ChatTurn,
    DecideNextAction,
    CompactContainerContext,
    CompactChatContext,
    CompactTaskContext,
    ExtractJson,
    Summarize,
    Rewrite,
    GenerateArtifact,
    Audit,
    RenderEntityReply,
}

impl ModelOperation {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ChatTurn => "chat_turn",
            Self::DecideNextAction => "decide_next_action",
            Self::CompactContainerContext => "compact_container_context",
            Self::CompactChatContext => "compact_chat_context",
            Self::CompactTaskContext => "compact_task_context",
            Self::ExtractJson => "extract_json",
            Self::Summarize => "summarize",
            Self::Rewrite => "rewrite",
            Self::GenerateArtifact => "generate_artifact",
            Self::Audit => "audit",
            Self::RenderEntityReply => "render_entity_reply",
        }
    }

    pub fn capability_id(&self) -> &'static str {
        match self {
            Self::ChatTurn => "model.chat_turn",
            Self::DecideNextAction => "model.decide_next_action",
            Self::CompactContainerContext => "model.compact_container_context",
            Self::CompactChatContext => "model.compact_chat_context",
            Self::CompactTaskContext => "model.compact_task_context",
            Self::ExtractJson => "model.extract_json",
            Self::Summarize => "model.summarize",
            Self::Rewrite => "model.rewrite",
            Self::GenerateArtifact => "model.generate_artifact",
            Self::Audit => "model.audit",
            Self::RenderEntityReply => "model.render_entity_reply",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModelFailurePolicy {
    FailClosed,
    OptionalVisible,
}

impl ModelFailurePolicy {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::FailClosed => "fail_closed",
            Self::OptionalVisible => "optional_visible",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelBudget {
    pub max_input_bytes: u64,
    pub max_output_tokens: u32,
    pub timeout_ms: u64,
    pub max_retries: u32,
}

impl Default for ModelBudget {
    fn default() -> Self {
        Self {
            max_input_bytes: 1024 * 1024,
            max_output_tokens: 65_536,
            timeout_ms: 120_000,
            max_retries: 1,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ModelAction {
    pub action_id: String,
    pub job_id: String,
    pub pid: String,
    pub reasoning_step_id: String,
    pub operation: ModelOperation,
    pub instruction_ref: String,
    pub input_refs: Vec<String>,
    pub preference_snapshot_ref: Option<String>,
    pub output_schema: Value,
    pub provider: String,
    pub model: String,
    pub budget: ModelBudget,
    pub failure_policy: ModelFailurePolicy,
    pub required: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ModelProviderRequest {
    pub model_call_id: String,
    pub action: ModelAction,
    pub input_payloads: BTreeMap<String, String>,
    pub capability_snapshot: Value,
    pub model_config: ModelInvocationConfig,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_locale_context_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_locale_context: Option<ClientLocaleContext>,
    #[serde(default)]
    pub provider_tools: Vec<ProviderToolDefinition>,
    #[serde(default)]
    pub provider_tool_choice: Option<Value>,
    #[serde(default)]
    pub provider_transcript_messages: Vec<ProviderTranscriptMessage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_toolset_ref: Option<String>,
    #[serde(default)]
    pub current_user_message_required: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ProviderToolCall {
    pub id: String,
    pub r#type: String,
    pub function: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ProviderAssistantMessage {
    pub role: String,
    pub content: Option<String>,
    pub reasoning_content: Option<String>,
    pub tool_calls: Vec<ProviderToolCall>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ModelProviderResponse {
    pub output_text: String,
    pub assistant_message: Option<ProviderAssistantMessage>,
    pub reasoning_content: Option<String>,
    pub tool_calls: Vec<ProviderToolCall>,
    pub usage: Value,
    pub finish_reason: Option<String>,
    pub raw: Value,
    pub sampling_ignored_by_provider: bool,
    pub streaming: bool,
    pub first_token_ms: Option<u128>,
    pub chunks_count: u32,
    pub stream_event_count: u32,
    pub first_byte_timeout_ms: Option<u64>,
    pub idle_timeout_ms: Option<u64>,
    pub max_wall_time_ms: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModelStreamDeltaKind {
    Answer,
    Reasoning,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelStreamDelta {
    pub model_call_id: String,
    pub operation: ModelOperation,
    pub kind: ModelStreamDeltaKind,
    pub sequence: u32,
    pub delta: String,
}

pub trait ModelStreamSink: Send + Sync + std::fmt::Debug {
    fn on_model_stream_delta(&self, delta: ModelStreamDelta);
}

pub fn operation_supports_task_reasoning_stream(operation: &ModelOperation) -> bool {
    matches!(operation, ModelOperation::DecideNextAction)
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelProviderFailure {
    pub error_code: String,
    pub message: String,
    pub retryable: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelSchemaValidation {
    pub schema_valid: bool,
    pub errors: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ModelCallReceipt {
    pub model_call_id: String,
    pub capability_id: String,
    pub job_id: String,
    pub pid: String,
    pub reasoning_step_id: String,
    pub operation: ModelOperation,
    pub status: String,
    pub provider: String,
    pub model: String,
    pub input_refs: Vec<String>,
    pub instruction_ref: String,
    pub preference_snapshot_ref: Option<String>,
    pub output_schema: Value,
    pub output_ref: Option<String>,
    pub request_ref: Option<String>,
    pub ledger_ref: Option<String>,
    pub client_locale_context_ref: Option<String>,
    pub required: bool,
    pub failure_policy: ModelFailurePolicy,
    pub budget: ModelBudget,
    pub model_invocation_config: ModelInvocationConfig,
    pub model_invocation_config_ref: Option<String>,
    pub schema_validation: ModelSchemaValidation,
    pub no_silent_fallback_pass: bool,
    pub fallback_risks: Vec<String>,
    pub provider_capability_snapshot: Value,
    pub reasoning_content_ref: Option<String>,
    pub reasoning_content_tokens_estimated: u64,
    pub provider_transcript_id: Option<String>,
    pub provider_transcript_ref: Option<String>,
    pub provider_transcript_summary_ref: Option<String>,
    pub provider_assistant_message_ref: Option<String>,
    pub provider_tool_calls: Vec<ProviderToolCall>,
    pub provider_toolset_ref: Option<String>,
    pub sampling_ignored_by_provider: bool,
    pub usage: Value,
    pub finish_reason: Option<String>,
    pub streaming: bool,
    pub first_token_ms: Option<u128>,
    pub chunks_count: u32,
    pub stream_event_count: u32,
    pub first_byte_timeout_ms: Option<u64>,
    pub idle_timeout_ms: Option<u64>,
    pub max_wall_time_ms: Option<u64>,
    pub wall_time_ms: u128,
    pub attempts: u32,
    pub retry_count: u32,
    pub error: Option<ModelProviderFailure>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ModelCallLedger {
    pub job_id: String,
    pub ledger_ref: String,
    pub entries: Vec<ModelCallReceipt>,
}

pub trait ModelProvider: Send + Sync + std::fmt::Debug {
    fn provider_name(&self) -> &str;
    fn model_name(&self) -> &str;
    fn model_name_for_operation(&self, _operation: &ModelOperation) -> String {
        self.model_name().to_string()
    }
    fn capability_snapshot(&self) -> Value;
    fn invoke(
        &self,
        request: &ModelProviderRequest,
    ) -> Result<ModelProviderResponse, ModelProviderFailure>;
    fn invoke_with_stream_sink(
        &self,
        request: &ModelProviderRequest,
        stream_sink: Option<Arc<dyn ModelStreamSink>>,
    ) -> Result<ModelProviderResponse, ModelProviderFailure> {
        let _ = stream_sink;
        self.invoke(request)
    }
}

#[derive(Clone, Debug)]
pub struct MissingModelProvider {
    provider: String,
    model: String,
    reason: String,
}

impl MissingModelProvider {
    pub fn new(
        provider: impl Into<String>,
        model: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            provider: provider.into(),
            model: model.into(),
            reason: reason.into(),
        }
    }
}

impl ModelProvider for MissingModelProvider {
    fn provider_name(&self) -> &str {
        &self.provider
    }

    fn model_name(&self) -> &str {
        &self.model
    }

    fn capability_snapshot(&self) -> Value {
        json!({
            "provider": self.provider,
            "model": self.model,
            "protocol": "missing_provider",
            "available": false,
            "reason": self.reason,
            "supports_schema_validation": true,
            "supports_ledger": true,
        })
    }

    fn invoke(
        &self,
        _request: &ModelProviderRequest,
    ) -> Result<ModelProviderResponse, ModelProviderFailure> {
        Err(ModelProviderFailure {
            error_code: "MODEL_PROVIDER_NOT_CONFIGURED".to_string(),
            message: self.reason.clone(),
            retryable: false,
        })
    }
}

#[derive(Clone, Debug)]
pub struct DeterministicModelProvider {
    provider: String,
    model: String,
    outputs: BTreeMap<String, String>,
    tool_calls: BTreeMap<String, Vec<ProviderToolCall>>,
    failing_operations: BTreeSet<String>,
}

impl DeterministicModelProvider {
    pub fn new(provider: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            model: model.into(),
            outputs: BTreeMap::new(),
            tool_calls: BTreeMap::new(),
            failing_operations: BTreeSet::new(),
        }
    }

    pub fn with_output(mut self, operation: ModelOperation, output: impl Into<String>) -> Self {
        self = self.with_output_for_operation(operation.as_str(), output);
        self
    }

    pub fn with_output_for_operation(
        mut self,
        operation: impl Into<String>,
        output: impl Into<String>,
    ) -> Self {
        self.outputs.insert(operation.into(), output.into());
        self
    }

    pub fn with_tool_calls(
        mut self,
        operation: ModelOperation,
        tool_calls: Vec<ProviderToolCall>,
    ) -> Self {
        self = self.with_tool_calls_for_operation(operation.as_str(), tool_calls);
        self
    }

    pub fn with_tool_calls_for_operation(
        mut self,
        operation: impl Into<String>,
        tool_calls: Vec<ProviderToolCall>,
    ) -> Self {
        self.tool_calls.insert(operation.into(), tool_calls);
        self
    }

    pub fn fail_operation(mut self, operation: ModelOperation) -> Self {
        self.failing_operations
            .insert(operation.as_str().to_string());
        self
    }
}

impl ModelProvider for DeterministicModelProvider {
    fn provider_name(&self) -> &str {
        &self.provider
    }

    fn model_name(&self) -> &str {
        &self.model
    }

    fn capability_snapshot(&self) -> Value {
        json!({
            "provider": self.provider,
            "model": self.model,
            "protocol": "deterministic_provider",
            "supports_operations": [
                "decide_next_action",
                "chat_turn",
                "compact_container_context",
                "compact_chat_context",
                "compact_task_context",
                "extract_json",
                "summarize",
                "rewrite",
                "generate_artifact",
                "audit",
                "render_entity_reply"
            ],
            "supports_schema_validation": true,
            "supports_ledger": true,
        })
    }

    fn invoke(
        &self,
        request: &ModelProviderRequest,
    ) -> Result<ModelProviderResponse, ModelProviderFailure> {
        let operation = request.action.operation.as_str();
        if self.failing_operations.contains(operation) {
            return Err(ModelProviderFailure {
                error_code: "DETERMINISTIC_PROVIDER_FAILURE".to_string(),
                message: format!("deterministic provider failed operation {operation}"),
                retryable: false,
            });
        }
        let output = self
            .outputs
            .get(operation)
            .cloned()
            .unwrap_or_else(|| format!("{{\"operation\":\"{operation}\",\"status\":\"ok\"}}"));
        let tool_calls = self.tool_calls.get(operation).cloned().unwrap_or_default();
        Ok(ModelProviderResponse {
            output_text: output,
            assistant_message: None,
            reasoning_content: None,
            tool_calls,
            usage: json!({
                "input_ref_count": request.action.input_refs.len(),
                "output_chars": self.outputs.get(operation).map(|value| value.len()).unwrap_or(0),
            }),
            finish_reason: Some("stop".to_string()),
            raw: json!({"provider": self.provider, "model": self.model}),
            sampling_ignored_by_provider: false,
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

impl ModelRuntime {
    pub fn new(
        truth: ProcessTruthStore,
        token: CapabilityToken,
        provider: Arc<dyn ModelProvider>,
    ) -> Self {
        Self {
            truth,
            token,
            provider,
            model_config: ModelInvocationConfig::default(),
            model_invocation_config_ref: None,
            preplanned_provider_toolset: None,
            model_call_id_override: None,
            stream_sink: None,
            record_provider_user_message: false,
        }
    }

    pub fn with_model_invocation_config(
        mut self,
        model_config: ModelInvocationConfig,
        model_invocation_config_ref: Option<String>,
    ) -> Self {
        self.model_config = model_config;
        self.model_invocation_config_ref = model_invocation_config_ref;
        self
    }

    pub fn with_preplanned_provider_toolset(
        mut self,
        provider_toolset: Option<ProviderToolsetPlan>,
    ) -> Self {
        self.preplanned_provider_toolset = provider_toolset;
        self
    }

    pub fn with_model_call_id_override(mut self, model_call_id: Option<String>) -> Self {
        self.model_call_id_override = model_call_id;
        self
    }

    pub fn with_stream_sink(mut self, stream_sink: Option<Arc<dyn ModelStreamSink>>) -> Self {
        self.stream_sink = stream_sink;
        self
    }

    pub fn with_provider_user_message_recording(mut self, enabled: bool) -> Self {
        self.record_provider_user_message = enabled;
        self
    }

    pub fn invoke(&self, mut action: ModelAction) -> io::Result<ModelCallReceipt> {
        let started_at = Instant::now();
        let model_call_id = self.model_call_id_override.clone().unwrap_or_else(|| {
            format!(
                "mcall_{}_{}",
                safe_blob_name(&action.reasoning_step_id),
                now_ms()
            )
        });
        let capability_id = action.operation.capability_id().to_string();
        let provider_snapshot = self.provider.capability_snapshot();
        let context_profile =
            ModelContextProfile::for_provider(self.provider.as_ref(), &action.operation);
        self.model_config.apply_budget_overrides(&mut action.budget);
        context_profile.clamp_budget_to_context_window(&mut action.budget);
        action.model = self.model_config.effective_model_for_operation(
            self.provider.as_ref(),
            &action.operation,
            &provider_snapshot,
        );
        action.provider = self.provider.provider_name().to_string();
        let client_locale_context = ClientEnvRuntime::capture_locale_context();
        let client_locale_context_ref = self.truth.write_blob(
            &format!("client_locale_contexts/{model_call_id}.json"),
            &serde_json::to_vec_pretty(&client_locale_context).map_err(json_err)?,
        )?;
        let started_ref = self.truth.write_blob(
            &format!("model_requests/{model_call_id}_action.json"),
            &serde_json::to_vec_pretty(&action).map_err(json_err)?,
        )?;
        self.truth.append_event(
            Some(&self.token.pid),
            "model_call_started",
            json!({
                "model_call_id": model_call_id.clone(),
                "capability_id": capability_id.clone(),
                "reasoning_step_id": action.reasoning_step_id.clone(),
                "operation": action.operation.as_str(),
                "input_refs": action.input_refs.clone(),
                "instruction_ref": action.instruction_ref.clone(),
                "preference_snapshot_ref": action.preference_snapshot_ref.clone(),
                "provider": action.provider.clone(),
                "model": action.model.clone(),
                "budget": action.budget.clone(),
                "model_invocation_config_ref": self.model_invocation_config_ref.clone(),
                "model_invocation_config": self.model_config.clone(),
                "failure_policy": action.failure_policy.as_str(),
                "required": action.required,
                "request_ref": started_ref.clone(),
                "client_locale_context_ref": client_locale_context_ref.clone(),
            }),
        )?;

        if let Some(mut receipt) = self.blocked_receipt_for_action(
            &action,
            &model_call_id,
            &capability_id,
            &provider_snapshot,
        ) {
            receipt.request_ref = Some(started_ref);
            receipt.client_locale_context_ref = Some(client_locale_context_ref);
            receipt.wall_time_ms = started_at.elapsed().as_millis();
            self.finish_model_call(receipt)?;
            return self.last_model_receipt(&model_call_id);
        }

        let input_payloads = match self.resolve_model_inputs(&action) {
            Ok(payloads) => payloads,
            Err(err) => {
                let mut receipt = self.failed_receipt(
                    &action,
                    &model_call_id,
                    &capability_id,
                    provider_snapshot,
                    "MODEL_INPUT_REF_INVALID",
                    &err.to_string(),
                    false,
                    started_at.elapsed().as_millis(),
                    None,
                );
                receipt.request_ref = Some(started_ref);
                receipt.client_locale_context_ref = Some(client_locale_context_ref);
                self.finish_model_call(receipt)?;
                return self.last_model_receipt(&model_call_id);
            }
        };
        let provider_tool_plan =
            if provider_native_tool_request_enabled(&self.model_config, &action.operation) {
                if let Some(plan) = self.preplanned_provider_toolset.clone() {
                    Some(plan)
                } else {
                    match ProviderToolsetPlanner::new(
                        default_capability_registry(),
                        self.model_config.clone(),
                    )
                    .plan_and_record(
                        &self.truth,
                        &self.token.pid,
                        &model_call_id,
                        &action.operation,
                    ) {
                        Ok(plan) => Some(plan),
                        Err(err) => {
                            let mut receipt = self.failed_receipt(
                                &action,
                                &model_call_id,
                                &capability_id,
                                provider_snapshot,
                                &err.error_code,
                                &err.message,
                                false,
                                started_at.elapsed().as_millis(),
                                None,
                            );
                            receipt.request_ref = Some(started_ref);
                            receipt.client_locale_context_ref =
                                Some(client_locale_context_ref.clone());
                            self.finish_model_call(receipt)?;
                            return self.last_model_receipt(&model_call_id);
                        }
                    }
                }
            } else {
                None
            };
        let provider_toolset_ref = provider_tool_plan
            .as_ref()
            .map(|plan| plan.provider_toolset_ref.clone());
        let provider_tools = provider_tool_plan
            .as_ref()
            .map(|plan| plan.registry.tools.clone())
            .unwrap_or_default();
        let provider_tool_choice = if provider_tools.is_empty() {
            None
        } else {
            provider_tool_choice_value(&self.model_config)
        };
        let provider_transcript_messages = if provider_tools.is_empty() {
            Vec::new()
        } else {
            self.provider_transcript_messages(&action.provider)?
        };
        let request = ModelProviderRequest {
            model_call_id: model_call_id.clone(),
            action: action.clone(),
            input_payloads,
            capability_snapshot: provider_snapshot.clone(),
            model_config: self.model_config.clone(),
            client_locale_context_ref: Some(client_locale_context_ref.clone()),
            client_locale_context: Some(client_locale_context),
            provider_tools,
            provider_tool_choice,
            provider_transcript_messages,
            provider_toolset_ref: provider_toolset_ref.clone(),
            current_user_message_required: self.record_provider_user_message,
        };
        let provider_user_message = if self.record_provider_user_message
            && model_operation_updates_provider_transcript(&action.operation)
        {
            Some(render_provider_user_prompt(&request))
        } else {
            None
        };
        let request_ref = self.truth.write_blob(
            &format!("model_requests/{model_call_id}.json"),
            &serde_json::to_vec_pretty(&request).map_err(json_err)?,
        )?;
        let _ = append_provider_native_debug(
            &self.truth,
            "request_built",
            json!({
                "model_call_id": model_call_id,
                "turn_id": action.reasoning_step_id.clone(),
                "capability_id": capability_id,
                "decision_protocol": "provider_native_tool_calls",
                "provider_toolset_ref": request.provider_toolset_ref.clone(),
                "toolset_contains": request.provider_tools.iter()
                    .map(|tool| tool.function.name.clone())
                    .collect::<Vec<_>>(),
                "diagnostic": {
                    "provider": action.provider.clone(),
                    "model": action.model.clone(),
                    "operation": action.operation.as_str(),
                    "request_ref": request_ref.clone(),
                    "provider_tool_count": request.provider_tools.len(),
                    "provider_tool_choice": request.provider_tool_choice.clone(),
                    "provider_transcript_message_count": request.provider_transcript_messages.len(),
                    "provider_transcript_roles": request.provider_transcript_messages.iter()
                        .map(|message| json!({
                            "role": message.role.clone(),
                            "tool_call_id": message.tool_call_id.clone(),
                            "tool_call_count": message.tool_calls.len(),
                            "content_present": message.content.as_ref().is_some_and(|value| !value.is_empty()),
                            "reasoning_content_present": message.reasoning_content.as_ref().is_some_and(|value| !value.is_empty()),
                        }))
                        .collect::<Vec<_>>(),
                    "thinking": request.model_config.thinking.clone(),
                    "tool_calling": request.model_config.tool_calling.clone(),
                }
            }),
        );
        let retry_policy = RetryPolicy::from_budget(&action.budget);
        let mut attempts = 0;
        let mut last_error: Option<ModelProviderFailure> = None;
        let mut provider_response: Option<ModelProviderResponse> = None;
        while attempts < retry_policy.max_attempts {
            attempts += 1;
            let attempt_started_at = Instant::now();
            self.append_model_attempt_started(
                &action,
                &model_call_id,
                &capability_id,
                attempts,
                &retry_policy,
                &request_ref,
            )?;
            let provider_result =
                match self.acquire_provider_api_permit(&action, &provider_snapshot, &model_call_id)
                {
                    Ok(Some(_permit)) => self
                        .provider
                        .invoke_with_stream_sink(&request, self.stream_sink.clone()),
                    Ok(None) => self
                        .provider
                        .invoke_with_stream_sink(&request, self.stream_sink.clone()),
                    Err(err) => Err(err),
                };
            match provider_result {
                Ok(response) => {
                    self.append_model_attempt_succeeded(
                        &action,
                        &model_call_id,
                        &capability_id,
                        attempts,
                        attempt_started_at.elapsed().as_millis(),
                        &response,
                    )?;
                    provider_response = Some(response);
                    break;
                }
                Err(err) => {
                    let should_retry = retry_policy.should_retry(attempts, &err);
                    self.append_model_attempt_failed(
                        &action,
                        &model_call_id,
                        &capability_id,
                        attempts,
                        attempt_started_at.elapsed().as_millis(),
                        &err,
                        should_retry,
                    )?;
                    last_error = Some(err.clone());
                    if !should_retry {
                        break;
                    }
                    let backoff_ms = retry_policy.backoff_ms(attempts);
                    self.append_model_attempt_backoff(
                        &action,
                        &model_call_id,
                        &capability_id,
                        attempts,
                        backoff_ms,
                    )?;
                    retry_policy.sleep_before_retry(attempts);
                }
            }
        }

        let receipt = if let Some(response) = provider_response {
            self.success_or_validation_failure_receipt(
                action,
                model_call_id.clone(),
                capability_id,
                request_ref,
                provider_snapshot,
                response,
                started_at.elapsed().as_millis(),
                attempts,
                provider_toolset_ref,
                provider_user_message,
            )?
        } else {
            let err = last_error.unwrap_or(ModelProviderFailure {
                error_code: "MODEL_PROVIDER_FAILED".to_string(),
                message: "model provider failed without detail".to_string(),
                retryable: false,
            });
            let mut receipt = self.failed_receipt(
                &action,
                &model_call_id,
                &capability_id,
                provider_snapshot,
                &err.error_code,
                &err.message,
                err.retryable,
                started_at.elapsed().as_millis(),
                provider_toolset_ref,
            );
            receipt.request_ref = Some(request_ref);
            receipt.client_locale_context_ref = Some(client_locale_context_ref.clone());
            receipt.attempts = attempts;
            receipt.retry_count = attempts.saturating_sub(1);
            receipt.error = Some(err);
            receipt
        };
        let mut receipt = receipt;
        if receipt.client_locale_context_ref.is_none() {
            receipt.client_locale_context_ref = Some(client_locale_context_ref);
        }
        self.finish_model_call(receipt)?;
        self.last_model_receipt(&model_call_id)
    }

    fn acquire_provider_api_permit(
        &self,
        action: &ModelAction,
        provider_snapshot: &Value,
        model_call_id: &str,
    ) -> Result<Option<ProviderApiPermit>, ModelProviderFailure> {
        if !provider_global_limiter_enabled(provider_snapshot, &action.provider) {
            return Ok(None);
        }
        let permit = ProviderApiLimiter::new(self.truth.state_root(), &action.provider)
            .acquire(model_call_id, action.budget.timeout_ms)?;
        let _ = self.truth.append_event(
            Some(&self.token.pid),
            "provider_rate_limiter_acquired",
            json!({
                "model_call_id": model_call_id,
                "provider": action.provider.clone(),
                "wait_ms": permit.wait_ms,
                "min_interval_ms": permit.min_interval_ms,
                "state_path": permit.state_path.display().to_string(),
            }),
        );
        Ok(Some(permit))
    }

    pub fn decide_next_action(&self, mut action: ModelAction) -> io::Result<ModelCallReceipt> {
        action.operation = ModelOperation::DecideNextAction;
        self.invoke(action)
    }

    pub fn chat_turn(&self, mut action: ModelAction) -> io::Result<ModelCallReceipt> {
        action.operation = ModelOperation::ChatTurn;
        self.invoke(action)
    }

    pub fn compact_container_context(
        &self,
        mut action: ModelAction,
    ) -> io::Result<ModelCallReceipt> {
        action.operation = ModelOperation::CompactContainerContext;
        self.invoke(action)
    }

    pub fn compact_chat_context(&self, mut action: ModelAction) -> io::Result<ModelCallReceipt> {
        action.operation = ModelOperation::CompactChatContext;
        self.invoke(action)
    }

    pub fn compact_task_context(&self, mut action: ModelAction) -> io::Result<ModelCallReceipt> {
        action.operation = ModelOperation::CompactTaskContext;
        self.invoke(action)
    }

    pub fn extract_json(&self, mut action: ModelAction) -> io::Result<ModelCallReceipt> {
        action.operation = ModelOperation::ExtractJson;
        self.invoke(action)
    }

    pub fn summarize(&self, mut action: ModelAction) -> io::Result<ModelCallReceipt> {
        action.operation = ModelOperation::Summarize;
        self.invoke(action)
    }

    pub fn rewrite(&self, mut action: ModelAction) -> io::Result<ModelCallReceipt> {
        action.operation = ModelOperation::Rewrite;
        self.invoke(action)
    }

    pub fn generate_artifact(&self, mut action: ModelAction) -> io::Result<ModelCallReceipt> {
        action.operation = ModelOperation::GenerateArtifact;
        self.invoke(action)
    }

    pub fn audit(&self, mut action: ModelAction) -> io::Result<ModelCallReceipt> {
        action.operation = ModelOperation::Audit;
        self.invoke(action)
    }

    pub fn render_entity_reply(&self, mut action: ModelAction) -> io::Result<ModelCallReceipt> {
        action.operation = ModelOperation::RenderEntityReply;
        self.invoke(action)
    }

    fn provider_transcript_messages(
        &self,
        provider: &str,
    ) -> io::Result<Vec<ProviderTranscriptMessage>> {
        let protocol = provider_protocol_for(provider);
        let Some(state) = replay_provider_transcript_state(&self.truth, provider, protocol)? else {
            return Ok(Vec::new());
        };
        read_provider_messages(&self.truth, &state)
    }

    fn append_model_attempt_started(
        &self,
        action: &ModelAction,
        model_call_id: &str,
        capability_id: &str,
        attempt: u32,
        retry_policy: &RetryPolicy,
        request_ref: &str,
    ) -> io::Result<()> {
        self.truth.append_event(
            Some(&self.token.pid),
            "model_call_attempt_started",
            json!({
                "model_call_id": model_call_id,
                "capability_id": capability_id,
                "reasoning_step_id": action.reasoning_step_id.clone(),
                "operation": action.operation.as_str(),
                "provider": action.provider.clone(),
                "model": action.model.clone(),
                "attempt": attempt,
                "max_attempts": retry_policy.max_attempts,
                "retry_policy": retry_policy,
                "request_ref": request_ref,
            }),
        )?;
        Ok(())
    }

    fn append_model_attempt_succeeded(
        &self,
        action: &ModelAction,
        model_call_id: &str,
        capability_id: &str,
        attempt: u32,
        wall_time_ms: u128,
        response: &ModelProviderResponse,
    ) -> io::Result<()> {
        self.truth.append_event(
            Some(&self.token.pid),
            "model_call_attempt_succeeded",
            json!({
                "model_call_id": model_call_id,
                "capability_id": capability_id,
                "reasoning_step_id": action.reasoning_step_id.clone(),
                "operation": action.operation.as_str(),
                "provider": action.provider.clone(),
                "model": action.model.clone(),
                "attempt": attempt,
                "wall_time_ms": wall_time_ms,
                "streaming": response.streaming,
                "chunks_count": response.chunks_count,
                "stream_event_count": response.stream_event_count,
                "finish_reason": response.finish_reason.clone(),
                "reasoning_content_captured": response
                    .reasoning_content
                    .as_ref()
                    .is_some_and(|value| !value.is_empty()),
                "reasoning_content_tokens_estimated": response
                    .reasoning_content
                    .as_deref()
                    .map(estimate_text_tokens_conservative)
                    .unwrap_or(0),
                "sampling_ignored_by_provider": response.sampling_ignored_by_provider,
                "tool_call_count": response.tool_calls.len(),
                "usage": response.usage.clone(),
            }),
        )?;
        let _ = append_provider_native_debug(
            &self.truth,
            "response_parsed",
            json!({
                "model_call_id": model_call_id,
                "turn_id": action.reasoning_step_id.clone(),
                "capability_id": capability_id,
                "decision_protocol": "provider_native_tool_calls",
                "diagnostic": {
                    "provider": action.provider.clone(),
                    "model": action.model.clone(),
                    "operation": action.operation.as_str(),
                    "attempt": attempt,
                    "wall_time_ms": wall_time_ms,
                    "finish_reason": response.finish_reason.clone(),
                    "status": "success",
                    "tool_call_count": response.tool_calls.len(),
                    "tool_calls": response.tool_calls.iter().map(|call| {
                        json!({
                            "id": call.id.clone(),
                            "type": call.r#type.clone(),
                            "name": call.function.get("name").and_then(Value::as_str),
                            "arguments_shape": call.function.get("arguments")
                                .map(crate::provider_debug::argument_shape)
                                .unwrap_or_else(|| json!({"type": "missing"})),
                        })
                    }).collect::<Vec<_>>(),
                    "content_present": !response.output_text.trim().is_empty(),
                    "reasoning_content_present": response.reasoning_content.as_ref().is_some_and(|value| !value.is_empty()),
                    "reasoning_content_tokens_estimated": response
                        .reasoning_content
                        .as_deref()
                        .map(estimate_text_tokens_conservative)
                        .unwrap_or(0),
                    "usage": response.usage.clone(),
                }
            }),
        );
        Ok(())
    }

    fn append_model_attempt_failed(
        &self,
        action: &ModelAction,
        model_call_id: &str,
        capability_id: &str,
        attempt: u32,
        wall_time_ms: u128,
        error: &ModelProviderFailure,
        will_retry: bool,
    ) -> io::Result<()> {
        self.truth.append_event(
            Some(&self.token.pid),
            "model_call_attempt_failed",
            json!({
                "model_call_id": model_call_id,
                "capability_id": capability_id,
                "reasoning_step_id": action.reasoning_step_id.clone(),
                "operation": action.operation.as_str(),
                "provider": action.provider.clone(),
                "model": action.model.clone(),
                "attempt": attempt,
                "wall_time_ms": wall_time_ms,
                "error": error,
                "will_retry": will_retry,
            }),
        )?;
        let _ = append_provider_native_debug(
            &self.truth,
            "response_parsed",
            json!({
                "model_call_id": model_call_id,
                "turn_id": action.reasoning_step_id.clone(),
                "capability_id": capability_id,
                "decision_protocol": "provider_native_tool_calls",
                "diagnostic": {
                    "provider": action.provider.clone(),
                    "model": action.model.clone(),
                    "operation": action.operation.as_str(),
                    "attempt": attempt,
                    "wall_time_ms": wall_time_ms,
                    "status": "failed",
                    "error": error,
                    "will_retry": will_retry,
                }
            }),
        );
        Ok(())
    }

    fn append_model_attempt_backoff(
        &self,
        action: &ModelAction,
        model_call_id: &str,
        capability_id: &str,
        attempt: u32,
        backoff_ms: u64,
    ) -> io::Result<()> {
        self.truth.append_event(
            Some(&self.token.pid),
            "model_call_attempt_backoff",
            json!({
                "model_call_id": model_call_id,
                "capability_id": capability_id,
                "reasoning_step_id": action.reasoning_step_id.clone(),
                "operation": action.operation.as_str(),
                "provider": action.provider.clone(),
                "model": action.model.clone(),
                "completed_attempt": attempt,
                "next_attempt": attempt.saturating_add(1),
                "backoff_ms": backoff_ms,
            }),
        )?;
        Ok(())
    }

    fn blocked_receipt_for_action(
        &self,
        action: &ModelAction,
        model_call_id: &str,
        capability_id: &str,
        provider_snapshot: &Value,
    ) -> Option<ModelCallReceipt> {
        let token_allows_capability = self
            .token
            .capabilities
            .iter()
            .any(|item| item == "model.invoke" || item == capability_id);
        let token_allows_permission = self
            .token
            .permissions
            .iter()
            .any(|item| item == "model:invoke");
        let reason = if !token_allows_capability {
            Some(format!("{capability_id} not granted"))
        } else if !token_allows_permission {
            Some("model:invoke permission not granted".to_string())
        } else if action.job_id != self.token.job_id || action.pid != self.token.pid {
            Some("model action job_id/pid does not match capability token".to_string())
        } else if action.provider != self.provider.provider_name()
            || action.model
                != self.model_config.effective_model_for_operation(
                    self.provider.as_ref(),
                    &action.operation,
                    provider_snapshot,
                )
        {
            Some("model action provider/model does not match selected ModelProvider".to_string())
        } else if !is_valid_model_ref(&action.instruction_ref, &self.token.job_id) {
            Some("instruction_ref must be a typed ref owned by this job".to_string())
        } else {
            None
        };
        reason.map(|message| ModelCallReceipt {
            model_call_id: model_call_id.to_string(),
            capability_id: capability_id.to_string(),
            job_id: self.token.job_id.clone(),
            pid: self.token.pid.clone(),
            reasoning_step_id: action.reasoning_step_id.clone(),
            operation: action.operation.clone(),
            status: "blocked".to_string(),
            provider: action.provider.clone(),
            model: action.model.clone(),
            input_refs: action.input_refs.clone(),
            instruction_ref: action.instruction_ref.clone(),
            preference_snapshot_ref: action.preference_snapshot_ref.clone(),
            output_schema: action.output_schema.clone(),
            output_ref: None,
            request_ref: None,
            ledger_ref: None,
            client_locale_context_ref: None,
            required: action.required,
            failure_policy: action.failure_policy.clone(),
            budget: action.budget.clone(),
            model_invocation_config: self.model_config.clone(),
            model_invocation_config_ref: self.model_invocation_config_ref.clone(),
            schema_validation: ModelSchemaValidation {
                schema_valid: false,
                errors: vec![message.clone()],
            },
            no_silent_fallback_pass: true,
            fallback_risks: Vec::new(),
            provider_capability_snapshot: provider_snapshot.clone(),
            reasoning_content_ref: None,
            reasoning_content_tokens_estimated: 0,
            provider_transcript_id: None,
            provider_transcript_ref: None,
            provider_transcript_summary_ref: None,
            provider_assistant_message_ref: None,
            provider_tool_calls: Vec::new(),
            provider_toolset_ref: None,
            sampling_ignored_by_provider: self.model_config.sampling_ignored_by_provider(),
            usage: json!({}),
            finish_reason: None,
            streaming: false,
            first_token_ms: None,
            chunks_count: 0,
            stream_event_count: 0,
            first_byte_timeout_ms: None,
            idle_timeout_ms: None,
            max_wall_time_ms: None,
            wall_time_ms: 0,
            attempts: 0,
            retry_count: 0,
            error: Some(ModelProviderFailure {
                error_code: "MODEL_ACTION_BLOCKED".to_string(),
                message,
                retryable: false,
            }),
        })
    }

    fn resolve_model_inputs(&self, action: &ModelAction) -> io::Result<BTreeMap<String, String>> {
        let mut refs = vec![action.instruction_ref.clone()];
        refs.extend(action.input_refs.iter().cloned());
        if let Some(preference_ref) = &action.preference_snapshot_ref {
            refs.push(preference_ref.clone());
        }
        for input_ref in &refs {
            if !is_valid_model_ref(input_ref, &self.token.job_id) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("model input ref is not a typed ref owned by this job: {input_ref}"),
                ));
            }
        }
        if action.input_refs.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "model operation requires source input_refs",
            ));
        }
        let mut total_bytes: u64 = 0;
        let mut payloads = BTreeMap::new();
        for input_ref in refs {
            if input_ref.starts_with("blob://") {
                let path = self.truth.resolve_blob_ref(&input_ref)?;
                let bytes = fs::read(&path)?;
                total_bytes = total_bytes.saturating_add(bytes.len() as u64);
                if total_bytes > action.budget.max_input_bytes {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "model action exceeds max_input_bytes budget",
                    ));
                }
                payloads.insert(input_ref, String::from_utf8_lossy(&bytes).to_string());
            } else {
                payloads.insert(input_ref, "<typed-ref-deferred-to-runtime>".to_string());
            }
        }
        Ok(payloads)
    }

    fn success_or_validation_failure_receipt(
        &self,
        action: ModelAction,
        model_call_id: String,
        capability_id: String,
        request_ref: String,
        provider_snapshot: Value,
        response: ModelProviderResponse,
        wall_time_ms: u128,
        attempts: u32,
        provider_toolset_ref: Option<String>,
        provider_user_message: Option<String>,
    ) -> io::Result<ModelCallReceipt> {
        let output_ref = self.truth.write_blob(
            &format!("model_outputs/{model_call_id}.txt"),
            response.output_text.as_bytes(),
        )?;
        let reasoning_content_tokens_estimated = response
            .reasoning_content
            .as_deref()
            .map(estimate_text_tokens_conservative)
            .unwrap_or(0);
        let transcript_record = if model_operation_updates_provider_transcript(&action.operation) {
            let protocol = provider_protocol_for(&action.provider);
            if let Some(user_message) = provider_user_message.as_deref() {
                record_provider_user_message(
                    &self.truth,
                    &self.token.pid,
                    &action.provider,
                    protocol,
                    &model_call_id,
                    user_message,
                )?;
            }
            record_provider_assistant_response(
                &self.truth,
                &self.token.pid,
                &action.provider,
                protocol,
                &model_call_id,
                &response,
                self.model_config.thinking.store_reasoning_content,
            )?
        } else {
            None
        };
        let reasoning_content_ref = transcript_record
            .as_ref()
            .and_then(|record| record.reasoning_content_ref.clone());
        let provider_native_tool_request =
            provider_native_tool_request_enabled(&self.model_config, &action.operation);
        let schema_validation = if provider_native_tool_request {
            ModelSchemaValidation {
                schema_valid: true,
                errors: Vec::new(),
            }
        } else {
            validate_model_output(
                &action.operation,
                &response.output_text,
                &action.output_schema,
            )
        };
        let mut status = "success".to_string();
        let mut error = None;
        if !schema_validation.schema_valid {
            status = "failed".to_string();
            error = Some(ModelProviderFailure {
                error_code: "MODEL_OUTPUT_SCHEMA_INVALID".to_string(),
                message: schema_validation.errors.join("; "),
                retryable: false,
            });
        }
        Ok(ModelCallReceipt {
            model_call_id,
            capability_id,
            job_id: self.token.job_id.clone(),
            pid: self.token.pid.clone(),
            reasoning_step_id: action.reasoning_step_id,
            operation: action.operation,
            status,
            provider: action.provider,
            model: action.model,
            input_refs: action.input_refs,
            instruction_ref: action.instruction_ref,
            preference_snapshot_ref: action.preference_snapshot_ref,
            output_schema: action.output_schema,
            output_ref: Some(output_ref),
            request_ref: Some(request_ref),
            ledger_ref: None,
            client_locale_context_ref: None,
            required: action.required,
            failure_policy: action.failure_policy,
            budget: action.budget,
            model_invocation_config: self.model_config.clone(),
            model_invocation_config_ref: self.model_invocation_config_ref.clone(),
            schema_validation,
            no_silent_fallback_pass: true,
            fallback_risks: Vec::new(),
            provider_capability_snapshot: provider_snapshot,
            reasoning_content_ref,
            reasoning_content_tokens_estimated,
            provider_transcript_id: transcript_record
                .as_ref()
                .map(|record| record.transcript_id.clone()),
            provider_transcript_ref: transcript_record
                .as_ref()
                .map(|record| record.messages_ref.clone()),
            provider_transcript_summary_ref: transcript_record
                .as_ref()
                .map(|record| record.summary_ref.clone()),
            provider_assistant_message_ref: transcript_record
                .as_ref()
                .and_then(|record| record.assistant_message_ref.clone()),
            provider_tool_calls: response.tool_calls,
            provider_toolset_ref,
            sampling_ignored_by_provider: response.sampling_ignored_by_provider,
            usage: response.usage,
            finish_reason: response.finish_reason,
            streaming: response.streaming,
            first_token_ms: response.first_token_ms,
            chunks_count: response.chunks_count,
            stream_event_count: response.stream_event_count,
            first_byte_timeout_ms: response.first_byte_timeout_ms,
            idle_timeout_ms: response.idle_timeout_ms,
            max_wall_time_ms: response.max_wall_time_ms,
            wall_time_ms,
            attempts,
            retry_count: attempts.saturating_sub(1),
            error,
        })
    }

    fn failed_receipt(
        &self,
        action: &ModelAction,
        model_call_id: &str,
        capability_id: &str,
        provider_snapshot: Value,
        error_code: &str,
        message: &str,
        retryable: bool,
        wall_time_ms: u128,
        provider_toolset_ref: Option<String>,
    ) -> ModelCallReceipt {
        ModelCallReceipt {
            model_call_id: model_call_id.to_string(),
            capability_id: capability_id.to_string(),
            job_id: self.token.job_id.clone(),
            pid: self.token.pid.clone(),
            reasoning_step_id: action.reasoning_step_id.clone(),
            operation: action.operation.clone(),
            status: "failed".to_string(),
            provider: action.provider.clone(),
            model: action.model.clone(),
            input_refs: action.input_refs.clone(),
            instruction_ref: action.instruction_ref.clone(),
            preference_snapshot_ref: action.preference_snapshot_ref.clone(),
            output_schema: action.output_schema.clone(),
            output_ref: None,
            request_ref: None,
            ledger_ref: None,
            client_locale_context_ref: None,
            required: action.required,
            failure_policy: action.failure_policy.clone(),
            budget: action.budget.clone(),
            model_invocation_config: self.model_config.clone(),
            model_invocation_config_ref: self.model_invocation_config_ref.clone(),
            schema_validation: ModelSchemaValidation {
                schema_valid: false,
                errors: vec![message.to_string()],
            },
            no_silent_fallback_pass: true,
            fallback_risks: Vec::new(),
            provider_capability_snapshot: provider_snapshot,
            reasoning_content_ref: None,
            reasoning_content_tokens_estimated: 0,
            provider_transcript_id: None,
            provider_transcript_ref: None,
            provider_transcript_summary_ref: None,
            provider_assistant_message_ref: None,
            provider_tool_calls: Vec::new(),
            provider_toolset_ref,
            sampling_ignored_by_provider: self.model_config.sampling_ignored_by_provider(),
            usage: json!({}),
            finish_reason: None,
            streaming: false,
            first_token_ms: None,
            chunks_count: 0,
            stream_event_count: 0,
            first_byte_timeout_ms: None,
            idle_timeout_ms: None,
            max_wall_time_ms: None,
            wall_time_ms,
            attempts: 0,
            retry_count: 0,
            error: Some(ModelProviderFailure {
                error_code: error_code.to_string(),
                message: message.to_string(),
                retryable,
            }),
        }
    }

    fn finish_model_call(&self, mut receipt: ModelCallReceipt) -> io::Result<()> {
        let ledger_ref = format!(
            "model_ledger://{}/model_call_ledger.json",
            self.token.job_id
        );
        receipt.ledger_ref = Some(ledger_ref.clone());
        self.write_model_call_ledger(&receipt)?;
        let event_type = match receipt.status.as_str() {
            "success" => "model_call_completed",
            "blocked" => "model_call_blocked",
            _ => "model_call_failed",
        };
        self.truth
            .append_event(Some(&self.token.pid), event_type, to_json_value(&receipt)?)?;
        self.truth.append_event(
            Some(&self.token.pid),
            "model_call_ledger",
            json!({
                "model_call_id": receipt.model_call_id.clone(),
                "ledger_ref": ledger_ref,
                "provider": receipt.provider.clone(),
                "model": receipt.model.clone(),
                "operation": receipt.operation.as_str(),
                "status": receipt.status.clone(),
                "model_invocation_config_ref": receipt.model_invocation_config_ref.clone(),
                "client_locale_context_ref": receipt.client_locale_context_ref.clone(),
                "reasoning_content_ref": receipt.reasoning_content_ref.clone(),
                "provider_transcript_ref": receipt.provider_transcript_ref.clone(),
                "provider_transcript_summary_ref": receipt.provider_transcript_summary_ref.clone(),
                "provider_toolset_ref": receipt.provider_toolset_ref.clone(),
                "provider_tool_call_count": receipt.provider_tool_calls.len(),
            }),
        )?;
        self.truth.append_event(
            Some(&self.token.pid),
            "model_call_receipt",
            to_json_value(&receipt)?,
        )?;
        let capability_receipt = CapabilityReceipt {
            capability_id: receipt.capability_id.clone(),
            job_id: receipt.job_id.clone(),
            pid: receipt.pid.clone(),
            status: receipt.status.clone(),
            data: to_json_value(&receipt)?,
        };
        self.truth.append_event(
            Some(&self.token.pid),
            "capability_receipt",
            to_json_value(&capability_receipt)?,
        )?;
        if receipt.status != "success"
            && (receipt.required || receipt.failure_policy == ModelFailurePolicy::FailClosed)
        {
            self.truth.append_event(
                Some(&self.token.pid),
                "required_model_operation_failed",
                json!({
                    "runtime_note": "model runtime records the failed syscall; TaskAgent decides whether to retry, revise inputs, fail, or produce an explicit boundary artifact",
                    "model_call_id": receipt.model_call_id,
                    "operation": receipt.operation.as_str(),
                    "status": receipt.status,
                    "error": receipt.error,
                }),
            )?;
        }
        Ok(())
    }

    fn write_model_call_ledger(&self, receipt: &ModelCallReceipt) -> io::Result<()> {
        let ledger_path = self.model_call_ledger_path()?;
        let ledger_ref = format!(
            "model_ledger://{}/model_call_ledger.json",
            self.token.job_id
        );
        let mut ledger = if ledger_path.is_file() {
            let raw = fs::read_to_string(&ledger_path)?;
            serde_json::from_str::<ModelCallLedger>(&raw).unwrap_or(ModelCallLedger {
                job_id: self.token.job_id.clone(),
                ledger_ref: ledger_ref.clone(),
                entries: serde_json::from_str::<Vec<ModelCallReceipt>>(&raw).unwrap_or_default(),
            })
        } else {
            ModelCallLedger {
                job_id: self.token.job_id.clone(),
                ledger_ref: ledger_ref.clone(),
                entries: Vec::new(),
            }
        };
        ledger.job_id = self.token.job_id.clone();
        ledger.ledger_ref = ledger_ref;
        ledger.entries.push(receipt.clone());
        fs::write(
            &ledger_path,
            serde_json::to_vec_pretty(&ledger).map_err(json_err)?,
        )?;
        Ok(())
    }

    fn last_model_receipt(&self, model_call_id: &str) -> io::Result<ModelCallReceipt> {
        let ledger_path = self.model_call_ledger_path()?;
        let ledger: ModelCallLedger =
            serde_json::from_str(&fs::read_to_string(ledger_path)?).map_err(json_err)?;
        ledger
            .entries
            .into_iter()
            .rev()
            .find(|receipt| receipt.model_call_id == model_call_id)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "model receipt not found"))
    }

    fn model_call_ledger_path(&self) -> io::Result<std::path::PathBuf> {
        let ledger_dir = self
            .truth
            .state_root()
            .join("model_call_ledger")
            .join(&self.token.job_id);
        fs::create_dir_all(&ledger_dir)?;
        Ok(ledger_dir.join("model_call_ledger.json"))
    }
}

fn provider_global_limiter_enabled(provider_snapshot: &Value, provider: &str) -> bool {
    provider == "deepseek"
        || provider_snapshot
            .get("live_api")
            .and_then(Value::as_bool)
            .unwrap_or(false)
}

#[derive(Debug)]
struct ProviderApiLimiter {
    provider: String,
    lock_path: PathBuf,
    state_path: PathBuf,
    min_interval_ms: u64,
    stale_lock_ms: u64,
}

#[derive(Debug)]
struct ProviderApiPermit {
    lock_path: PathBuf,
    state_path: PathBuf,
    wait_ms: u128,
    min_interval_ms: u64,
}

#[derive(Debug, Serialize, Deserialize)]
struct ProviderApiLimiterState {
    schema_version: String,
    provider: String,
    next_allowed_at_ms: u128,
    updated_by_model_call_id: String,
}

impl ProviderApiLimiter {
    fn new(state_root: &Path, provider: &str) -> Self {
        let provider_key = safe_blob_name(provider);
        let limiter_dir = state_root.join("provider_api_limiter");
        Self {
            provider: provider.to_string(),
            lock_path: limiter_dir.join(format!("{provider_key}.lock")),
            state_path: limiter_dir.join(format!("{provider_key}.json")),
            min_interval_ms: provider_global_min_interval_ms(),
            stale_lock_ms: provider_global_stale_lock_ms(),
        }
    }

    fn acquire(
        &self,
        model_call_id: &str,
        timeout_ms: u64,
    ) -> Result<ProviderApiPermit, ModelProviderFailure> {
        let started_at = Instant::now();
        let timeout = Duration::from_millis(timeout_ms.max(1));
        if let Some(parent) = self.lock_path.parent() {
            fs::create_dir_all(parent).map_err(provider_limiter_io_error)?;
        }
        loop {
            self.remove_stale_lock_if_needed();
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&self.lock_path)
            {
                Ok(mut file) => {
                    writeln!(
                        file,
                        "{}",
                        serde_json::to_string(&json!({
                            "schema_version": "supernova.provider_api_limiter.lock.v1",
                            "provider": self.provider.clone(),
                            "model_call_id": model_call_id,
                            "owner_pid": std::process::id(),
                            "created_at_ms": now_ms(),
                        }))
                        .map_err(provider_limiter_json_error)?
                    )
                    .map_err(provider_limiter_io_error)?;
                    let mut permit = ProviderApiPermit {
                        lock_path: self.lock_path.clone(),
                        state_path: self.state_path.clone(),
                        wait_ms: started_at.elapsed().as_millis(),
                        min_interval_ms: self.min_interval_ms,
                    };
                    self.enforce_min_interval(&mut permit, model_call_id, started_at, timeout)?;
                    permit.wait_ms = started_at.elapsed().as_millis();
                    return Ok(permit);
                }
                Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
                    if started_at.elapsed() >= timeout {
                        return Err(ModelProviderFailure {
                            error_code: "MODEL_PROVIDER_RATE_LIMITER_TIMEOUT".to_string(),
                            message: format!(
                                "Timed out waiting for provider API limiter lock for provider {}",
                                self.provider
                            ),
                            retryable: true,
                        });
                    }
                    thread::sleep(Duration::from_millis(25));
                }
                Err(err) => return Err(provider_limiter_io_error(err)),
            }
        }
    }

    fn enforce_min_interval(
        &self,
        _permit: &mut ProviderApiPermit,
        model_call_id: &str,
        started_at: Instant,
        timeout: Duration,
    ) -> Result<(), ModelProviderFailure> {
        let next_allowed_at_ms = self.read_next_allowed_at_ms()?;
        let now = now_ms();
        if next_allowed_at_ms > now {
            let wait_ms = next_allowed_at_ms - now;
            let remaining_ms = timeout
                .checked_sub(started_at.elapsed())
                .map(|duration| duration.as_millis())
                .unwrap_or(0);
            if wait_ms > remaining_ms {
                return Err(ModelProviderFailure {
                    error_code: "MODEL_PROVIDER_RATE_LIMITER_TIMEOUT".to_string(),
                    message: format!(
                        "Provider API limiter interval for provider {} exceeds remaining model attempt budget",
                        self.provider
                    ),
                    retryable: true,
                });
            }
            thread::sleep(Duration::from_millis(wait_ms as u64));
        }
        let granted_at_ms = now_ms();
        let state = ProviderApiLimiterState {
            schema_version: "supernova.provider_api_limiter.state.v1".to_string(),
            provider: self.provider.clone(),
            next_allowed_at_ms: granted_at_ms + self.min_interval_ms as u128,
            updated_by_model_call_id: model_call_id.to_string(),
        };
        fs::write(
            &self.state_path,
            serde_json::to_vec_pretty(&state).map_err(provider_limiter_json_error)?,
        )
        .map_err(provider_limiter_io_error)?;
        Ok(())
    }

    fn read_next_allowed_at_ms(&self) -> Result<u128, ModelProviderFailure> {
        if !self.state_path.exists() {
            return Ok(0);
        }
        let raw = fs::read(&self.state_path).map_err(provider_limiter_io_error)?;
        let state: ProviderApiLimiterState =
            serde_json::from_slice(&raw).map_err(provider_limiter_json_error)?;
        Ok(state.next_allowed_at_ms)
    }

    fn remove_stale_lock_if_needed(&self) {
        let Ok(metadata) = fs::metadata(&self.lock_path) else {
            return;
        };
        let Ok(modified) = metadata.modified() else {
            return;
        };
        let Ok(age) = modified.elapsed() else {
            return;
        };
        if age.as_millis() >= self.stale_lock_ms as u128 {
            let _ = fs::remove_file(&self.lock_path);
        }
    }
}

impl Drop for ProviderApiPermit {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.lock_path);
    }
}

fn provider_global_min_interval_ms() -> u64 {
    std::env::var("SUPERNOVA_PROVIDER_GLOBAL_MIN_INTERVAL_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0)
}

fn provider_global_stale_lock_ms() -> u64 {
    std::env::var("SUPERNOVA_PROVIDER_GLOBAL_STALE_LOCK_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(600_000)
}

fn provider_limiter_io_error(err: io::Error) -> ModelProviderFailure {
    ModelProviderFailure {
        error_code: "MODEL_PROVIDER_RATE_LIMITER_IO".to_string(),
        message: err.to_string(),
        retryable: true,
    }
}

fn provider_limiter_json_error(err: serde_json::Error) -> ModelProviderFailure {
    ModelProviderFailure {
        error_code: "MODEL_PROVIDER_RATE_LIMITER_STATE_INVALID".to_string(),
        message: err.to_string(),
        retryable: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{create_agent_job, WorkspaceGuard};
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Barrier, Mutex,
    };
    use std::thread;

    fn temp_workspace(name: &str) -> std::path::PathBuf {
        let path =
            std::env::temp_dir().join(format!("supernova_model_runtime_{}_{}", name, now_ms()));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn model_call_records_client_locale_context_ref_in_request_receipt_and_ledger() {
        let workspace = temp_workspace("client_locale_context");
        let (job, process, truth) = create_agent_job(&workspace, "model locale context").unwrap();
        let _guard = WorkspaceGuard::new(&workspace).unwrap();
        let instruction_ref = truth
            .write_blob("model_inputs/instruction.txt", b"Return JSON.")
            .unwrap();
        let input_ref = truth
            .write_blob("model_inputs/input.txt", b"input")
            .unwrap();
        let token = CapabilityToken {
            token_id: "token_model_locale".to_string(),
            job_id: job.job_id.clone(),
            pid: process.pid.clone(),
            workspace_root: workspace.display().to_string(),
            capabilities: vec!["model.invoke".to_string()],
            permissions: vec!["model:invoke".to_string()],
        };
        let provider = Arc::new(
            DeterministicModelProvider::new("deterministic", "deterministic-model")
                .with_output(ModelOperation::Summarize, "{\"summary\":\"ok\"}"),
        );
        let runtime = ModelRuntime::new(truth.clone(), token, provider.clone());
        let mut action = ModelAction {
            action_id: "act_locale".to_string(),
            job_id: job.job_id.clone(),
            pid: process.pid.clone(),
            reasoning_step_id: "reason_locale".to_string(),
            operation: ModelOperation::Summarize,
            instruction_ref,
            input_refs: vec![input_ref],
            preference_snapshot_ref: None,
            output_schema: json!({"type": "object"}),
            provider: provider.provider_name().to_string(),
            model: provider.model_name().to_string(),
            budget: ModelBudget::default(),
            failure_policy: ModelFailurePolicy::FailClosed,
            required: true,
        };
        action.budget.max_input_bytes = 1024 * 1024;
        let receipt = runtime.summarize(action).unwrap();
        let context_ref = receipt
            .client_locale_context_ref
            .clone()
            .expect("receipt records locale context ref");
        assert!(context_ref.starts_with(&format!("blob://{}/", job.job_id)));

        let request_ref = receipt.request_ref.as_ref().unwrap();
        let request: ModelProviderRequest = serde_json::from_slice(
            &std::fs::read(truth.resolve_blob_ref(request_ref).unwrap()).unwrap(),
        )
        .unwrap();
        assert_eq!(
            request.client_locale_context_ref.as_deref(),
            Some(context_ref.as_str())
        );
        assert_eq!(
            request
                .client_locale_context
                .as_ref()
                .map(|context| context.schema_version.as_str()),
            Some(crate::CLIENT_LOCALE_CONTEXT_SCHEMA_VERSION)
        );

        let events = truth.read_events().unwrap();
        assert!(events.iter().any(|event| {
            event.event_type == "model_call_started"
                && event
                    .data
                    .get("client_locale_context_ref")
                    .and_then(Value::as_str)
                    == Some(context_ref.as_str())
        }));
        assert!(events.iter().any(|event| {
            event.event_type == "model_call_ledger"
                && event
                    .data
                    .get("client_locale_context_ref")
                    .and_then(Value::as_str)
                    == Some(context_ref.as_str())
        }));
        let ledger: ModelCallLedger = serde_json::from_str(
            &std::fs::read_to_string(
                workspace
                    .join(crate::RUNTIME_DIR_NAME)
                    .join("model_call_ledger")
                    .join(&job.job_id)
                    .join("model_call_ledger.json"),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            ledger.entries[0].client_locale_context_ref.as_deref(),
            Some(context_ref.as_str())
        );
    }

    #[derive(Debug, Default)]
    struct RecordingStreamSink {
        deltas: Mutex<Vec<ModelStreamDelta>>,
    }

    impl RecordingStreamSink {
        fn deltas(&self) -> Vec<ModelStreamDelta> {
            self.deltas
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .clone()
        }
    }

    impl ModelStreamSink for RecordingStreamSink {
        fn on_model_stream_delta(&self, delta: ModelStreamDelta) {
            self.deltas
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .push(delta);
        }
    }

    #[derive(Debug)]
    struct StreamingProvider;

    impl ModelProvider for StreamingProvider {
        fn provider_name(&self) -> &str {
            "streaming-provider"
        }

        fn model_name(&self) -> &str {
            "streaming-model"
        }

        fn capability_snapshot(&self) -> Value {
            json!({
                "provider": self.provider_name(),
                "model": self.model_name(),
                "protocol": "stream_sink_test",
                "supports_ledger": true,
            })
        }

        fn invoke(
            &self,
            request: &ModelProviderRequest,
        ) -> Result<ModelProviderResponse, ModelProviderFailure> {
            self.invoke_with_stream_sink(request, None)
        }

        fn invoke_with_stream_sink(
            &self,
            request: &ModelProviderRequest,
            stream_sink: Option<Arc<dyn ModelStreamSink>>,
        ) -> Result<ModelProviderResponse, ModelProviderFailure> {
            if let Some(sink) = stream_sink {
                for (sequence, delta) in [(1, "Hel"), (2, "lo")] {
                    sink.on_model_stream_delta(ModelStreamDelta {
                        model_call_id: request.model_call_id.clone(),
                        operation: request.action.operation.clone(),
                        kind: ModelStreamDeltaKind::Answer,
                        sequence,
                        delta: delta.into(),
                    });
                }
            }
            Ok(ModelProviderResponse {
                output_text: "Hello".into(),
                assistant_message: None,
                reasoning_content: None,
                tool_calls: Vec::new(),
                usage: json!({"output_chars": 5}),
                finish_reason: Some("stop".into()),
                raw: json!({"provider": self.provider_name()}),
                sampling_ignored_by_provider: false,
                streaming: true,
                first_token_ms: Some(1),
                chunks_count: 2,
                stream_event_count: 2,
                first_byte_timeout_ms: None,
                idle_timeout_ms: None,
                max_wall_time_ms: None,
            })
        }
    }

    #[derive(Debug)]
    struct LiveSerialProvider {
        active_calls: Arc<AtomicUsize>,
        max_active_calls: Arc<AtomicUsize>,
        sleep_ms: u64,
    }

    impl ModelProvider for LiveSerialProvider {
        fn provider_name(&self) -> &str {
            "live-serial-provider"
        }

        fn model_name(&self) -> &str {
            "live-serial-model"
        }

        fn capability_snapshot(&self) -> Value {
            json!({
                "provider": self.provider_name(),
                "model": self.model_name(),
                "protocol": "live_provider_limiter_test",
                "supports_ledger": true,
                "live_api": true,
            })
        }

        fn invoke(
            &self,
            request: &ModelProviderRequest,
        ) -> Result<ModelProviderResponse, ModelProviderFailure> {
            let active = self.active_calls.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_active_calls.fetch_max(active, Ordering::SeqCst);
            thread::sleep(std::time::Duration::from_millis(self.sleep_ms));
            self.active_calls.fetch_sub(1, Ordering::SeqCst);
            Ok(ModelProviderResponse {
                output_text: format!("ok:{}", request.model_call_id),
                assistant_message: None,
                reasoning_content: None,
                tool_calls: Vec::new(),
                usage: json!({"output_chars": 2}),
                finish_reason: Some("stop".into()),
                raw: json!({"provider": self.provider_name()}),
                sampling_ignored_by_provider: false,
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

    #[test]
    fn model_runtime_serializes_live_provider_attempts_across_jobs() {
        let workspace = temp_workspace("live_provider_limiter");
        let active_calls = Arc::new(AtomicUsize::new(0));
        let max_active_calls = Arc::new(AtomicUsize::new(0));
        let provider = Arc::new(LiveSerialProvider {
            active_calls,
            max_active_calls: max_active_calls.clone(),
            sleep_ms: 75,
        });
        let mut cases = Vec::new();
        for index in 0..2 {
            let (job, process, truth) =
                create_agent_job(&workspace, &format!("live provider limiter {index}")).unwrap();
            let instruction_ref = truth
                .write_blob(
                    &format!("model_inputs/instruction_{index}.txt"),
                    b"Return ok.",
                )
                .unwrap();
            let input_ref = truth
                .write_blob(&format!("model_inputs/input_{index}.txt"), b"input")
                .unwrap();
            let token = CapabilityToken {
                token_id: format!("token_live_provider_limiter_{index}"),
                job_id: job.job_id.clone(),
                pid: process.pid.clone(),
                workspace_root: workspace.display().to_string(),
                capabilities: vec!["model.invoke".to_string()],
                permissions: vec!["model:invoke".to_string()],
            };
            let runtime = ModelRuntime::new(truth.clone(), token, provider.clone())
                .with_model_call_id_override(Some(format!("live_provider_limiter_{index}")));
            let mut action = ModelAction {
                action_id: format!("act_live_provider_limiter_{index}"),
                job_id: job.job_id.clone(),
                pid: process.pid.clone(),
                reasoning_step_id: format!("reason_live_provider_limiter_{index}"),
                operation: ModelOperation::Summarize,
                instruction_ref,
                input_refs: vec![input_ref],
                preference_snapshot_ref: None,
                output_schema: json!({"type": "string"}),
                provider: provider.provider_name().to_string(),
                model: provider.model_name().to_string(),
                budget: ModelBudget::default(),
                failure_policy: ModelFailurePolicy::FailClosed,
                required: true,
            };
            action.budget.timeout_ms = 10_000;
            action.budget.max_input_bytes = 1024 * 1024;
            cases.push((runtime, action, truth));
        }
        let (runtime_a, action_a, truth_a) = cases.remove(0);
        let (runtime_b, action_b, truth_b) = cases.remove(0);
        let barrier = Arc::new(Barrier::new(2));
        let barrier_a = barrier.clone();
        let handle_a = thread::spawn(move || {
            barrier_a.wait();
            runtime_a.summarize(action_a).unwrap()
        });
        let barrier_b = barrier.clone();
        let handle_b = thread::spawn(move || {
            barrier_b.wait();
            runtime_b.summarize(action_b).unwrap()
        });

        let receipt_a = handle_a.join().unwrap();
        let receipt_b = handle_b.join().unwrap();

        assert_eq!(receipt_a.status, "success");
        assert_eq!(receipt_b.status, "success");
        assert_eq!(max_active_calls.load(Ordering::SeqCst), 1);
        let limiter_events = truth_a
            .read_events()
            .unwrap()
            .into_iter()
            .chain(truth_b.read_events().unwrap())
            .filter(|event| event.event_type == "provider_rate_limiter_acquired")
            .count();
        assert_eq!(limiter_events, 2);
    }

    #[test]
    fn model_runtime_forwards_provider_answer_chunks_to_stream_sink() {
        let workspace = temp_workspace("stream_sink");
        let (job, process, truth) = create_agent_job(&workspace, "model stream sink").unwrap();
        let instruction_ref = truth
            .write_blob("model_inputs/instruction.txt", b"Answer directly.")
            .unwrap();
        let input_ref = truth
            .write_blob("model_inputs/input.txt", b"input")
            .unwrap();
        let token = CapabilityToken {
            token_id: "token_model_stream_sink".to_string(),
            job_id: job.job_id.clone(),
            pid: process.pid.clone(),
            workspace_root: workspace.display().to_string(),
            capabilities: vec![
                "model.invoke".to_string(),
                ModelOperation::ChatTurn.capability_id().to_string(),
            ],
            permissions: vec!["model:invoke".to_string()],
        };
        let provider = Arc::new(StreamingProvider);
        let sink = Arc::new(RecordingStreamSink::default());
        let sink_trait: Arc<dyn ModelStreamSink> = sink.clone();
        let runtime = ModelRuntime::new(truth.clone(), token, provider.clone())
            .with_model_call_id_override(Some("model_call_stream_sink".into()))
            .with_stream_sink(Some(sink_trait));
        let mut action = ModelAction {
            action_id: "act_stream_sink".to_string(),
            job_id: job.job_id.clone(),
            pid: process.pid.clone(),
            reasoning_step_id: "reason_stream_sink".to_string(),
            operation: ModelOperation::ChatTurn,
            instruction_ref,
            input_refs: vec![input_ref],
            preference_snapshot_ref: None,
            output_schema: json!({"type": "string"}),
            provider: provider.provider_name().to_string(),
            model: provider.model_name().to_string(),
            budget: ModelBudget::default(),
            failure_policy: ModelFailurePolicy::FailClosed,
            required: true,
        };
        action.budget.max_input_bytes = 1024 * 1024;

        let receipt = runtime.chat_turn(action).unwrap();

        assert_eq!(receipt.status, "success");
        assert_eq!(receipt.model_call_id, "model_call_stream_sink");
        assert_eq!(receipt.streaming, true);
        assert_eq!(receipt.chunks_count, 2);
        let deltas = sink.deltas();
        assert_eq!(deltas.len(), 2);
        assert_eq!(deltas[0].model_call_id, "model_call_stream_sink");
        assert_eq!(deltas[0].operation, ModelOperation::ChatTurn);
        assert_eq!(deltas[0].kind, ModelStreamDeltaKind::Answer);
        assert_eq!(deltas[0].sequence, 1);
        assert_eq!(deltas[0].delta, "Hel");
        assert_eq!(deltas[1].delta, "lo");
    }

    #[test]
    fn named_model_entrypoints_record_client_locale_context_ref() {
        for operation in [
            ModelOperation::ChatTurn,
            ModelOperation::CompactContainerContext,
            ModelOperation::CompactChatContext,
            ModelOperation::CompactTaskContext,
            ModelOperation::RenderEntityReply,
        ] {
            assert_named_entrypoint_records_client_locale_context(operation);
        }
    }

    fn assert_named_entrypoint_records_client_locale_context(operation: ModelOperation) {
        let operation_name = operation.as_str().to_string();
        let workspace = temp_workspace(&format!("client_locale_{operation_name}"));
        let goal = format!("model locale {operation_name}");
        let (job, process, truth) = create_agent_job(&workspace, &goal).unwrap();
        let instruction_ref = truth
            .write_blob("model_inputs/instruction.txt", b"Return a valid response.")
            .unwrap();
        let input_ref = truth
            .write_blob("model_inputs/input.txt", b"input")
            .unwrap();
        let token = CapabilityToken {
            token_id: format!("token_model_locale_{operation_name}"),
            job_id: job.job_id.clone(),
            pid: process.pid.clone(),
            workspace_root: workspace.display().to_string(),
            capabilities: vec![
                "model.invoke".to_string(),
                operation.capability_id().to_string(),
            ],
            permissions: vec!["model:invoke".to_string()],
        };
        let (output_text, output_schema) = if matches!(
            operation,
            ModelOperation::CompactContainerContext
                | ModelOperation::CompactChatContext
                | ModelOperation::CompactTaskContext
        ) {
            ("{\"summary\":\"ok\"}", json!({"type": "object"}))
        } else {
            ("ok", json!({"type": "string"}))
        };
        let provider = Arc::new(
            DeterministicModelProvider::new("deterministic", "deterministic-model")
                .with_output(operation.clone(), output_text),
        );
        let runtime = ModelRuntime::new(truth.clone(), token, provider.clone());
        let mut action = ModelAction {
            action_id: format!("act_locale_{operation_name}"),
            job_id: job.job_id.clone(),
            pid: process.pid.clone(),
            reasoning_step_id: format!("reason_locale_{operation_name}"),
            operation: operation.clone(),
            instruction_ref,
            input_refs: vec![input_ref],
            preference_snapshot_ref: None,
            output_schema,
            provider: provider.provider_name().to_string(),
            model: provider.model_name().to_string(),
            budget: ModelBudget::default(),
            failure_policy: ModelFailurePolicy::FailClosed,
            required: true,
        };
        action.budget.max_input_bytes = 1024 * 1024;
        let receipt = match operation {
            ModelOperation::ChatTurn => runtime.chat_turn(action),
            ModelOperation::CompactContainerContext => runtime.compact_container_context(action),
            ModelOperation::CompactChatContext => runtime.compact_chat_context(action),
            ModelOperation::CompactTaskContext => runtime.compact_task_context(action),
            ModelOperation::RenderEntityReply => runtime.render_entity_reply(action),
            other => panic!("unexpected operation in entrypoint locale test: {other:?}"),
        }
        .unwrap();

        assert_eq!(receipt.operation.as_str(), operation_name);
        assert_eq!(receipt.status, "success");
        let context_ref = receipt
            .client_locale_context_ref
            .clone()
            .expect("receipt records locale context ref");
        let request_ref = receipt.request_ref.as_ref().unwrap();
        let request: ModelProviderRequest = serde_json::from_slice(
            &std::fs::read(truth.resolve_blob_ref(request_ref).unwrap()).unwrap(),
        )
        .unwrap();
        assert_eq!(
            request.client_locale_context_ref.as_deref(),
            Some(context_ref.as_str())
        );
        assert_eq!(
            request
                .client_locale_context
                .as_ref()
                .map(|context| context.schema_version.as_str()),
            Some(crate::CLIENT_LOCALE_CONTEXT_SCHEMA_VERSION)
        );
        let events = truth.read_events().unwrap();
        assert!(events.iter().any(|event| {
            event.event_type == "model_call_started"
                && event
                    .data
                    .get("client_locale_context_ref")
                    .and_then(Value::as_str)
                    == Some(context_ref.as_str())
        }));
        let ledger: ModelCallLedger = serde_json::from_str(
            &std::fs::read_to_string(
                workspace
                    .join(crate::RUNTIME_DIR_NAME)
                    .join("model_call_ledger")
                    .join(&job.job_id)
                    .join("model_call_ledger.json"),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            ledger.entries[0].client_locale_context_ref.as_deref(),
            Some(context_ref.as_str())
        );
    }
}

fn is_valid_model_ref(input_ref: &str, job_id: &str) -> bool {
    let owned_blob_prefix = format!("blob://{job_id}/");
    input_ref.starts_with(&owned_blob_prefix)
        || input_ref.starts_with("source_set_ref://")
        || input_ref.starts_with("dataset_ref://")
        || input_ref.starts_with("artifact_ref://")
        || input_ref.starts_with("memory_snapshot_ref://")
}

fn validate_model_output(
    operation: &ModelOperation,
    output_text: &str,
    output_schema: &Value,
) -> ModelSchemaValidation {
    let mut errors = Vec::new();
    let schema_type = output_schema
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("");
    if schema_type.is_empty() && *operation != ModelOperation::ExtractJson {
        return ModelSchemaValidation {
            schema_valid: true,
            errors,
        };
    }
    if schema_type == "string" {
        if output_text.trim().is_empty() {
            errors.push("string output is empty".to_string());
        }
        return ModelSchemaValidation {
            schema_valid: errors.is_empty(),
            errors,
        };
    }
    let parsed = match serde_json::from_str::<Value>(output_text) {
        Ok(value) => value,
        Err(err) => {
            return ModelSchemaValidation {
                schema_valid: false,
                errors: vec![format!("model output is not valid JSON: {err}")],
            };
        }
    };
    match schema_type {
        "object" | "" => {
            let Some(object) = parsed.as_object() else {
                return ModelSchemaValidation {
                    schema_valid: false,
                    errors: vec!["model output is not a JSON object".to_string()],
                };
            };
            if let Some(required) = output_schema.get("required").and_then(Value::as_array) {
                for key in required {
                    if let Some(key) = key.as_str() {
                        if !object.contains_key(key) {
                            errors.push(format!("required key missing: {key}"));
                        }
                    }
                }
            }
        }
        "array" => {
            if !parsed.is_array() {
                errors.push("model output is not a JSON array".to_string());
            }
        }
        other => errors.push(format!("unsupported output schema type: {other}")),
    }
    ModelSchemaValidation {
        schema_valid: errors.is_empty(),
        errors,
    }
}

fn model_operation_updates_provider_transcript(operation: &ModelOperation) -> bool {
    !matches!(
        operation,
        ModelOperation::CompactContainerContext
            | ModelOperation::CompactChatContext
            | ModelOperation::CompactTaskContext
    )
}

fn provider_protocol_for(provider: &str) -> &str {
    if provider == "deepseek" {
        "deepseek_chat_completions"
    } else {
        "model_provider_transcript"
    }
}

fn render_provider_user_prompt(request: &ModelProviderRequest) -> String {
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

fn safe_blob_name(value: &str) -> String {
    value
        .replace('\\', "/")
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn to_json_value<T: Serialize>(value: &T) -> io::Result<Value> {
    serde_json::to_value(value).map_err(json_err)
}

fn json_err(err: serde_json::Error) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, err)
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_millis(0))
        .as_millis()
}
