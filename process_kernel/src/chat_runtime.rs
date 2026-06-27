use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::chat_truth::{
    ChatEvent, ChatProviderTranscript, ChatTruthStore, CHAT_TRUTH_SCHEMA_VERSION,
};
use crate::container_context::build_context_pack_visible_payload;
use crate::context_pack::ContextPack;
use crate::model_config::{
    ModelInvocationConfig, ProviderToolsetMode, ResponseLanguage, TaskAgentDecisionProtocol,
    ToolChoicePolicy,
};
use crate::model_runtime::{
    default_model_provider_from_env, ModelAction, ModelFailurePolicy, ModelOperation,
    ModelProvider, ModelRuntime, ModelStreamSink, ProviderToolCall,
};
use crate::provider_tool::{
    provider_tool_call_name, provider_tool_is_mutation_apply_capability, ProviderToolRegistry,
};
use crate::provider_tool_loop_executor::{
    ProviderToolExecution, ProviderToolLoopAdapter, ProviderToolLoopExecutor,
    ProviderToolLoopPolicy, ProviderToolLoopStatus,
};
use crate::provider_toolset::ProviderToolsetPlanner;
use crate::provider_transcript::{
    read_provider_messages, record_provider_tool_result_with_metadata,
    replace_provider_visible_transcript_with_summary, replay_provider_transcript_state,
    ProviderToolResultMetadata,
};
use crate::reasoning::NextActionDecision;
use crate::{
    context_window_tokens_for_budget, default_capability_registry, now_ms, safe_blob_name,
    CapabilityReceipt, CapabilityToken, ContextScope, ContextWindowController,
    ContextWindowRequestParts, ModelCallReceipt, ModelContextProfile, ProcessTruthStore,
    ProviderTranscriptProtocolValidator, ReadOnlyCapabilityExecutor, RuntimeKind, SourceGuidance,
    WorkspaceGuard,
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChatTurnStatus {
    Answered,
    Clarifying,
    NeedsTask,
    Blocked,
    Failed,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SuggestedTaskRequest {
    pub goal: String,
    pub reason: String,
    pub context_pack_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ChatTurnRequest {
    pub container_id: String,
    pub chat_thread_id: Option<String>,
    pub message: String,
    pub context_pack: Option<ContextPack>,
    pub source_guidance: Option<SourceGuidance>,
    pub model_config_override: Option<ModelInvocationConfig>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ChatTurnResult {
    pub chat_thread_id: String,
    pub turn_id: String,
    pub status: ChatTurnStatus,
    pub assistant_content: Option<String>,
    pub suggested_task: Option<SuggestedTaskRequest>,
    pub cited_refs: Vec<String>,
    pub tool_receipt_refs: Vec<String>,
    pub context_window_receipt_ref: Option<String>,
    pub events: Vec<ChatEvent>,
}

#[derive(Clone, Debug)]
pub struct ChatRuntime {
    workspace_root: PathBuf,
    state_root: PathBuf,
    model_provider: Arc<dyn ModelProvider>,
    model_config: ModelInvocationConfig,
}

#[derive(Clone, Debug)]
struct ChatProviderToolLoopAdapter {
    model_config: ModelInvocationConfig,
}

impl ChatProviderToolLoopAdapter {
    fn new(model_config: ModelInvocationConfig) -> Self {
        Self { model_config }
    }
}

impl ProviderToolLoopAdapter for ChatProviderToolLoopAdapter {
    fn runtime_kind(&self) -> RuntimeKind {
        RuntimeKind::Chat
    }

    fn loop_policy(&self) -> ProviderToolLoopPolicy {
        ProviderToolLoopPolicy {
            max_provider_subturns: self.model_config.tool_calling.max_provider_subturns,
            max_tool_calls_per_subturn: self.model_config.tool_calling.max_tool_calls_per_subturn,
            max_tool_calls_total: self.model_config.tool_calling.max_tool_calls_per_chat_turn,
            allow_parallel_readonly: self.model_config.tool_calling.allow_parallel_readonly,
            mutation_allowed: false,
        }
    }

    fn provider_tool_calls_are_limit_exempt(&self, _tool_calls: &[ProviderToolCall]) -> bool {
        true
    }
}

pub fn chat_model_syscall_truth_id(chat_thread_id: &str) -> String {
    format!("chat_model_syscalls_{}", safe_blob_name(chat_thread_id))
}

impl ChatRuntime {
    pub fn new(workspace_root: impl AsRef<Path>) -> io::Result<Self> {
        Self::with_model_provider(workspace_root, default_model_provider_from_env())
    }

    pub fn new_with_state_root(
        workspace_root: impl AsRef<Path>,
        state_root: impl AsRef<Path>,
    ) -> io::Result<Self> {
        Self::with_model_provider_and_state_root(
            workspace_root,
            state_root,
            default_model_provider_from_env(),
        )
    }

    pub fn with_model_provider(
        workspace_root: impl AsRef<Path>,
        model_provider: Arc<dyn ModelProvider>,
    ) -> io::Result<Self> {
        let guard = WorkspaceGuard::new(workspace_root)?;
        Ok(Self {
            workspace_root: guard.root().to_path_buf(),
            state_root: guard.root().join(crate::RUNTIME_DIR_NAME),
            model_provider,
            model_config: ModelInvocationConfig::from_env(),
        })
    }

    pub fn with_model_provider_and_state_root(
        workspace_root: impl AsRef<Path>,
        state_root: impl AsRef<Path>,
        model_provider: Arc<dyn ModelProvider>,
    ) -> io::Result<Self> {
        let guard = WorkspaceGuard::new(workspace_root)?;
        std::fs::create_dir_all(state_root.as_ref())?;
        Ok(Self {
            workspace_root: guard.root().to_path_buf(),
            state_root: state_root.as_ref().canonicalize()?,
            model_provider,
            model_config: ModelInvocationConfig::from_env(),
        })
    }

    pub fn with_model_config(mut self, model_config: ModelInvocationConfig) -> Self {
        self.model_config = model_config;
        self
    }

    pub fn start_turn(&self, request: ChatTurnRequest) -> io::Result<ChatTurnResult> {
        self.start_turn_with_stream_sink(request, None)
    }

    pub fn start_turn_with_stream_sink(
        &self,
        request: ChatTurnRequest,
        stream_sink: Option<Arc<dyn ModelStreamSink>>,
    ) -> io::Result<ChatTurnResult> {
        let chat_truth =
            ChatTruthStore::new_with_state_root(&self.workspace_root, &self.state_root)?;
        let thread = if let Some(thread_id) = request.chat_thread_id.as_deref() {
            chat_truth.get_thread(thread_id)?
        } else {
            chat_truth.create_thread(
                &request.container_id,
                Some(request.message.chars().take(80).collect::<String>()),
            )?
        };
        if thread.container_id != request.container_id {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "chat_thread_id does not belong to container_id",
            ));
        }
        let turn_id = format!("chat_turn_{}", crate::now_ms());
        let message_ref = chat_truth.write_chat_blob(
            &thread.chat_thread_id,
            &format!("turns/{turn_id}_user.txt"),
            request.message.as_bytes(),
        )?;
        chat_truth.append_event(
            &thread.chat_thread_id,
            &thread.container_id,
            "chat_turn_started",
            json!({
                "schema_version": CHAT_TRUTH_SCHEMA_VERSION,
                "turn_id": turn_id,
                "message_ref": message_ref.clone(),
                "context_pack_id": request.context_pack.as_ref().map(|pack| pack.context_pack_id.clone()),
            }),
            None,
        )?;
        chat_truth.append_event(
            &thread.chat_thread_id,
            &thread.container_id,
            "chat_user_message_recorded",
            json!({
                "schema_version": CHAT_TRUTH_SCHEMA_VERSION,
                "turn_id": turn_id,
                "message_ref": message_ref.clone(),
                "message_chars": request.message.chars().count(),
            }),
            None,
        )?;
        if let Some(pack) = request.context_pack.as_ref() {
            chat_truth.append_event(
                &thread.chat_thread_id,
                &thread.container_id,
                "chat_context_pack_loaded",
                json!({
                    "schema_version": CHAT_TRUTH_SCHEMA_VERSION,
                    "turn_id": turn_id,
                    "context_pack_id": pack.context_pack_id.clone(),
                    "selected_item_count": pack.selected_items.len(),
                    "excluded_item_count": pack.excluded_items.len(),
                    "summary_ref": pack.summary_ref.clone(),
                    "estimated_tokens": pack.estimated_tokens,
                }),
                None,
            )?;
        }

        let model_config = chat_runtime_model_config(
            request
                .model_config_override
                .clone()
                .unwrap_or_else(|| self.model_config.clone()),
        );
        let model_truth_id = chat_model_syscall_truth_id(&thread.chat_thread_id);
        let model_truth = ProcessTruthStore::new_with_state_root(
            &self.workspace_root,
            &self.state_root,
            &model_truth_id,
        )?;
        let pid = format!("chat_runtime_{}", thread.chat_thread_id);
        let model_config_ref = self.record_chat_model_config_bound(
            &chat_truth,
            &thread.chat_thread_id,
            &thread.container_id,
            &turn_id,
            &model_truth,
            &pid,
            &model_config,
        )?;
        let registry = ProviderToolRegistry::chat_runtime_readonly(
            &default_capability_registry(),
            &model_config,
        );
        let token = CapabilityToken {
            token_id: format!("token_{pid}"),
            job_id: model_truth_id.clone(),
            pid: pid.clone(),
            workspace_root: self.workspace_root.display().to_string(),
            capabilities: chat_runtime_capabilities(&registry),
            permissions: vec![
                "model:invoke".to_string(),
                "fs:read".to_string(),
                "process:control".to_string(),
                "chat:control".to_string(),
                "office:read".to_string(),
                "client_env:read".to_string(),
            ],
        };
        let prompt = chat_system_prompt_for_language(model_config.response_language);
        let instruction_ref = model_truth.write_blob(
            &format!("chat_model_inputs/{turn_id}_instruction.txt"),
            prompt.as_bytes(),
        )?;
        let input_ref = model_truth.write_blob(
            &format!("chat_model_inputs/{turn_id}_user.txt"),
            request.message.as_bytes(),
        )?;
        let source_guidance_ref = if let Some(guidance) = request
            .source_guidance
            .clone()
            .map(SourceGuidance::normalized)
            .filter(SourceGuidance::is_effective)
        {
            let guidance_text = guidance.provider_visible_text();
            let guidance_ref = model_truth.write_blob(
                &format!("chat_model_inputs/{turn_id}_reference_sources.txt"),
                guidance_text.as_bytes(),
            )?;
            chat_truth.append_event(
                &thread.chat_thread_id,
                &thread.container_id,
                "chat_reference_sources_attached",
                guidance.audit_payload(guidance_ref.clone()),
                Some(guidance_ref.clone()),
            )?;
            Some(guidance_ref)
        } else {
            None
        };
        let context_pack_payload = request
            .context_pack
            .as_ref()
            .map(|pack| {
                build_context_pack_visible_payload(&self.workspace_root, &self.state_root, pack)
            })
            .transpose()?
            .unwrap_or(Value::Null);
        let context_pack_ref = if context_pack_payload.is_null() {
            None
        } else {
            let context_pack_ref = model_truth.write_blob(
                &format!("chat_model_inputs/{turn_id}_context_pack.json"),
                &serde_json::to_vec_pretty(&context_pack_payload).map_err(crate::json_err)?,
            )?;
            chat_truth.append_event(
                &thread.chat_thread_id,
                &thread.container_id,
                "chat_context_pack_materialized",
                json!({
                    "schema_version": CHAT_TRUTH_SCHEMA_VERSION,
                    "turn_id": turn_id,
                    "context_pack_id": request.context_pack.as_ref().map(|pack| pack.context_pack_id.clone()),
                    "context_pack_ref": context_pack_ref.clone(),
                    "fact_boundary": "Context pack materialization is provider-visible input context only. It does not replace ChatTruth, ProcessTruth, receipts, or Kernel policy.",
                }),
                Some(context_pack_ref.clone()),
            )?;
            Some(context_pack_ref)
        };
        let provider = self.model_provider.clone();
        let operation = ModelOperation::ChatTurn;
        let context_profile = ModelContextProfile::for_provider(provider.as_ref(), &operation);
        let mut budget = context_profile.budget_for(&operation);
        model_config.apply_budget_overrides(&mut budget);
        context_profile.clamp_budget_to_context_window(&mut budget);
        let mut action = chat_model_action(
            &model_truth_id,
            &pid,
            &turn_id,
            operation.clone(),
            instruction_ref,
            append_optional_ref(
                append_optional_ref(vec![input_ref], source_guidance_ref.clone()),
                context_pack_ref.clone(),
            ),
            provider.as_ref(),
            budget.clone(),
        );
        action.model = effective_model_name(&model_config, provider.as_ref(), &operation);

        let parts = ContextWindowRequestParts {
            provider: action.provider.clone(),
            model: action.model.clone(),
            context_window_tokens: context_window_tokens_for_budget(&budget),
            system_prompt: prompt.to_string(),
            user_message: request.message.clone(),
            context_pack_payload,
            reserved_output_tokens: Some(budget.max_output_tokens as u64),
            reserved_reasoning_tokens: Some(model_config.context_window.reserve_reasoning_tokens),
            provider_options: json!({"operation": "chat_turn", "model_config": model_config.clone()}),
            ..ContextWindowRequestParts::default()
        };
        let context_preflight = ContextWindowController::preflight(
            ContextScope::Chat {
                container_id: thread.container_id.clone(),
                chat_thread_id: thread.chat_thread_id.clone(),
            },
            &model_config.context_window,
            &parts,
        )?;
        chat_truth.append_event(
            &thread.chat_thread_id,
            &thread.container_id,
            "context_window_checked",
            json!({
                "schema_version": crate::CONTEXT_WINDOW_EVENT_SCHEMA_VERSION,
                "turn_id": turn_id,
                "scope": context_preflight.scope,
                "estimate": context_preflight.estimate,
                "decision": context_preflight.decision,
            }),
            None,
        )?;
        chat_truth.append_event(
            &thread.chat_thread_id,
            &thread.container_id,
            "chat_context_window_checked",
            json!({
                "schema_version": CHAT_TRUTH_SCHEMA_VERSION,
                "turn_id": turn_id,
                "scope": context_preflight.scope,
                "estimate": context_preflight.estimate,
                "decision": context_preflight.decision,
            }),
            None,
        )?;
        if context_preflight.decision.compact_before_send {
            if let Some(summary_ref) = self.compact_chat_model_context(
                &chat_truth,
                &thread.chat_thread_id,
                &thread.container_id,
                &turn_id,
                &request.message,
                &request.context_pack,
                &model_truth,
                &token,
                provider.clone(),
                &model_config,
                Some(model_config_ref.clone()),
                &context_preflight,
            )? {
                action.input_refs = vec![summary_ref];
            }
        }

        self.run_provider_loop(
            &chat_truth,
            &thread.chat_thread_id,
            &thread.container_id,
            &turn_id,
            &request.message,
            model_truth,
            token,
            provider,
            model_config,
            Some(model_config_ref),
            registry,
            action,
            stream_sink,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn record_chat_model_config_bound(
        &self,
        chat_truth: &ChatTruthStore,
        chat_thread_id: &str,
        container_id: &str,
        turn_id: &str,
        model_truth: &ProcessTruthStore,
        pid: &str,
        model_config: &ModelInvocationConfig,
    ) -> io::Result<String> {
        let config_ref = model_truth.write_blob(
            &format!("chat_model_config/{turn_id}_model_invocation_config.json"),
            &serde_json::to_vec_pretty(model_config).map_err(crate::json_err)?,
        )?;
        let effective_config = model_config.redacted_binding_summary();
        model_truth.append_event(
            Some(pid),
            "model_config_bound",
            json!({
                "schema": "supernova_model_config_bound.v1",
                "turn_id": turn_id,
                "chat_thread_id": chat_thread_id,
                "container_id": container_id,
                "model_invocation_config_ref": config_ref.clone(),
                "effective_config": effective_config.clone(),
                "fact_boundary": "Chat model config is bound as provider-visible invocation configuration. Actual provider/model request facts are recorded by model_call_started and model_call_ledger.",
            }),
        )?;
        chat_truth.append_event(
            chat_thread_id,
            container_id,
            "chat_model_config_bound",
            json!({
                "schema_version": CHAT_TRUTH_SCHEMA_VERSION,
                "turn_id": turn_id,
                "model_invocation_config_ref": config_ref.clone(),
                "effective_config": effective_config,
            }),
            Some(config_ref.clone()),
        )?;
        Ok(config_ref)
    }

    #[allow(clippy::too_many_arguments)]
    fn run_provider_loop(
        &self,
        chat_truth: &ChatTruthStore,
        chat_thread_id: &str,
        container_id: &str,
        turn_id: &str,
        user_message: &str,
        model_truth: ProcessTruthStore,
        token: CapabilityToken,
        provider: Arc<dyn ModelProvider>,
        model_config: ModelInvocationConfig,
        model_config_ref: Option<String>,
        registry: ProviderToolRegistry,
        base_action: ModelAction,
        stream_sink: Option<Arc<dyn ModelStreamSink>>,
    ) -> io::Result<ChatTurnResult> {
        let loop_adapter = ChatProviderToolLoopAdapter::new(model_config.clone());
        let mut loop_exec = ProviderToolLoopExecutor::from_adapter(&loop_adapter);
        let guard = WorkspaceGuard::new(&self.workspace_root)?;
        let mut model_receipts = Vec::new();
        let mut executions = Vec::new();
        let mut cited_refs = Vec::new();
        let mut tool_receipt_refs = Vec::new();
        let mut read_bytes_used = 0_u64;
        let mut read_tokens_used = 0_u64;
        'provider_loop: for subturn_index in 0..loop_exec.policy().max_provider_subturns {
            let model_call_id = format!("mcall_{}_chat_{}_{}", turn_id, subturn_index, now_ms());
            let plan =
                ProviderToolsetPlanner::new(default_capability_registry(), model_config.clone())
                    .plan_chat_runtime_readonly(&model_truth, &token.pid, &model_call_id)
                    .map_err(|err| io::Error::new(io::ErrorKind::InvalidInput, err.message))?;
            chat_truth.append_event(
                chat_thread_id,
                container_id,
                "chat_provider_toolset_planned",
                json!({
                    "schema_version": CHAT_TRUTH_SCHEMA_VERSION,
                    "turn_id": turn_id,
                    "subturn_index": subturn_index,
                    "model_call_id": model_call_id.clone(),
                    "provider_toolset_ref": plan.provider_toolset_ref.clone(),
                    "provider_tool_count": plan.registry.tools.len(),
                    "omitted_count": plan.record.omitted_tools.len(),
                }),
                None,
            )?;
            let mut action = base_action.clone();
            action.action_id = format!("chat_model_action_{turn_id}_{subturn_index}");
            action.reasoning_step_id = format!("{turn_id}_{subturn_index}");
            chat_truth.append_event(
                chat_thread_id,
                container_id,
                "chat_model_call_started",
                json!({
                    "schema_version": CHAT_TRUTH_SCHEMA_VERSION,
                    "turn_id": turn_id,
                    "subturn_index": subturn_index,
                    "model_call_id": model_call_id.clone(),
                    "operation": "model.chat_turn",
                    "provider_toolset_ref": plan.provider_toolset_ref.clone(),
                }),
                None,
            )?;
            let receipt = ModelRuntime::new(model_truth.clone(), token.clone(), provider.clone())
                .with_model_invocation_config(model_config.clone(), model_config_ref.clone())
                .with_preplanned_provider_toolset(Some(plan))
                .with_model_call_id_override(Some(model_call_id.clone()))
                .with_stream_sink(stream_sink.clone())
                .with_provider_user_message_recording(subturn_index == 0)
                .chat_turn(action)?;
            self.sync_chat_provider_transcript(
                chat_truth,
                chat_thread_id,
                container_id,
                turn_id,
                &receipt,
                None,
                None,
            )?;
            chat_truth.append_event(
                chat_thread_id,
                container_id,
                "chat_model_call_completed",
                json!({
                    "schema_version": CHAT_TRUTH_SCHEMA_VERSION,
                    "turn_id": turn_id,
                    "subturn_index": subturn_index,
                    "model_call_id": receipt.model_call_id,
                    "status": receipt.status,
                    "provider_tool_call_count": receipt.provider_tool_calls.len(),
                    "provider_toolset_ref": receipt.provider_toolset_ref,
                    "provider_transcript_ref": receipt.provider_transcript_ref,
                    "provider_transcript_summary_ref": receipt.provider_transcript_summary_ref,
                }),
                None,
            )?;
            self.record_chat_model_receipt_projection(
                chat_truth,
                chat_thread_id,
                container_id,
                turn_id,
                subturn_index,
                &receipt,
            )?;
            model_receipts.push(receipt.clone());
            if receipt.status != "success" {
                return self.chat_failed_result(
                    chat_truth,
                    chat_thread_id,
                    container_id,
                    turn_id,
                    &receipt,
                );
            }
            if receipt.provider_tool_calls.is_empty() {
                let content = model_output_text(&model_truth, &receipt);
                return self.chat_answer_result(
                    chat_truth,
                    chat_thread_id,
                    container_id,
                    turn_id,
                    content,
                    cited_refs,
                    tool_receipt_refs,
                    model_receipts,
                    executions,
                );
            }
            let provider_tool_batch_id = loop_exec.batch_id(&receipt.model_call_id, subturn_index);
            let subturn = match loop_exec.begin_subturn(
                &loop_adapter,
                &receipt.model_call_id,
                subturn_index,
                &receipt.provider_tool_calls,
            ) {
                Ok(subturn) => subturn,
                Err(budget_error) => {
                    self.append_chat_provider_tool_skipped_results(
                        chat_truth,
                        chat_thread_id,
                        container_id,
                        turn_id,
                        &model_truth,
                        &token,
                        &receipt,
                        &loop_exec,
                        &provider_tool_batch_id,
                        &budget_error.message,
                    )?;
                    let budget_payload = json!({
                        "status": "recoverable_error",
                        "error_code": budget_error.error_code(),
                        "budget_kind": budget_error.budget_kind(),
                        "message": budget_error.message,
                        "requested_tool_calls": budget_error.requested_tool_calls,
                        "limit": budget_error.limit,
                        "executed_tool_calls_before": budget_error.executed_tool_calls_before,
                        "all_read_only_or_control": budget_error.all_read_only_or_control,
                        "provider_tool_batch_id": provider_tool_batch_id,
                        "tool_result_completeness": "all_provider_tool_calls_answered_with_skipped_results",
                    });
                    chat_truth.append_event(
                        chat_thread_id,
                        container_id,
                        "chat_provider_tool_loop_budget_exceeded",
                        json!({
                            "schema_version": CHAT_TRUTH_SCHEMA_VERSION,
                            "turn_id": turn_id,
                            "model_call_id": receipt.model_call_id,
                            "payload": budget_payload,
                        }),
                        None,
                    )?;
                    continue 'provider_loop;
                }
            };
            chat_truth.append_event(
                chat_thread_id,
                container_id,
                "chat_provider_tool_subturn_started",
                json!({
                    "schema_version": CHAT_TRUTH_SCHEMA_VERSION,
                    "turn_id": turn_id,
                    "subturn": subturn,
                }),
                None,
            )?;
            for (index, tool_call) in receipt.provider_tool_calls.iter().enumerate() {
                let decoded_tool_name = provider_tool_call_name(tool_call).ok();
                let decoded_capability = registry
                    .decision_for_tool_call(tool_call)
                    .ok()
                    .map(|decision| decision.capability_id);
                chat_truth.append_event(
                    chat_thread_id,
                    container_id,
                    "chat_provider_tool_call_decoded",
                    json!({
                        "schema_version": CHAT_TRUTH_SCHEMA_VERSION,
                        "turn_id": turn_id,
                        "model_call_id": receipt.model_call_id.clone(),
                        "provider_tool_batch_id": provider_tool_batch_id.clone(),
                        "provider_tool_call_id": tool_call.id.clone(),
                        "provider_tool_call_index": index,
                        "provider_tool_name": decoded_tool_name,
                        "capability_id": decoded_capability,
                    }),
                    None,
                )?;
                let execution = self.execute_chat_tool_call(
                    chat_truth,
                    chat_thread_id,
                    container_id,
                    turn_id,
                    user_message,
                    &model_truth,
                    &token,
                    &guard,
                    &registry,
                    &receipt,
                    tool_call,
                    &provider_tool_batch_id,
                    index,
                )?;
                chat_truth.append_event(
                    chat_thread_id,
                    container_id,
                    "chat_provider_tool_result_appended",
                    json!({
                        "schema_version": CHAT_TRUTH_SCHEMA_VERSION,
                        "turn_id": turn_id,
                        "model_call_id": receipt.model_call_id.clone(),
                        "provider_tool_batch_id": provider_tool_batch_id.clone(),
                        "provider_tool_call_id": tool_call.id.clone(),
                        "provider_tool_call_index": index,
                        "provider_tool_name": execution.provider_tool_name.clone(),
                        "capability_id": execution.capability_id.clone(),
                        "status": execution.status.clone(),
                        "tool_result_status": execution.tool_result.get("status").and_then(Value::as_str),
                    }),
                    None,
                )?;
                let read_usage = chat_tool_result_read_usage(&execution.tool_result);
                read_bytes_used = read_bytes_used.saturating_add(read_usage.0);
                read_tokens_used = read_tokens_used.saturating_add(read_usage.1);
                let read_budget_exceeded = read_bytes_used
                    > model_config.tool_calling.max_chat_read_bytes_per_turn
                    || read_tokens_used > model_config.tool_calling.max_chat_read_tokens_per_turn;
                if let Some(receipt_ref) = execution
                    .tool_result
                    .get("receipt_ref")
                    .and_then(Value::as_str)
                {
                    tool_receipt_refs.push(receipt_ref.to_string());
                }
                if let Some(refs) = execution
                    .tool_result
                    .get("cited_refs")
                    .and_then(Value::as_array)
                {
                    cited_refs.extend(
                        refs.iter()
                            .filter_map(Value::as_str)
                            .map(ToString::to_string),
                    );
                }
                let terminal_payload = execution.tool_result.clone();
                let status = execution.status.clone();
                executions.push(execution);
                loop_exec.mark_executed(1);
                if read_budget_exceeded {
                    let skipped_tool_call_count = self
                        .append_chat_provider_tool_skipped_results_from(
                            chat_truth,
                            chat_thread_id,
                            container_id,
                            turn_id,
                            &model_truth,
                            &token,
                            &receipt,
                            &loop_exec,
                            &provider_tool_batch_id,
                            index.saturating_add(1),
                            "ChatRuntime read-only provider tool loop exceeded per-turn read budget before executing the remaining provider tool calls.",
                        )?;
                    let budget_payload = json!({
                        "status": "recoverable_error",
                        "error_code": "CHAT_READONLY_BUDGET_EXCEEDED",
                        "message": "ChatRuntime read-only provider tool loop exceeded per-turn read budget.",
                        "provider_tool_batch_id": provider_tool_batch_id.clone(),
                        "executed_provider_tool_call_id": tool_call.id.clone(),
                        "executed_provider_tool_call_index": index,
                        "remaining_skipped_tool_call_count": skipped_tool_call_count,
                        "tool_result_completeness": if skipped_tool_call_count > 0 {
                            "remaining_provider_tool_calls_answered_with_skipped_results"
                        } else {
                            "all_provider_tool_calls_answered"
                        },
                        "read_bytes_used": read_bytes_used,
                        "read_tokens_used": read_tokens_used,
                        "max_read_bytes": model_config.tool_calling.max_chat_read_bytes_per_turn,
                        "max_read_tokens": model_config.tool_calling.max_chat_read_tokens_per_turn,
                    });
                    chat_truth.append_event(
                        chat_thread_id,
                        container_id,
                        "chat_readonly_budget_exceeded",
                        json!({
                            "schema_version": CHAT_TRUTH_SCHEMA_VERSION,
                            "turn_id": turn_id,
                            "payload": budget_payload,
                        }),
                        None,
                    )?;
                    continue 'provider_loop;
                }
                match status {
                    ProviderToolLoopStatus::Answered => {
                        let content = terminal_payload
                            .get("content")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string();
                        return self.chat_answer_result(
                            chat_truth,
                            chat_thread_id,
                            container_id,
                            turn_id,
                            content,
                            cited_refs,
                            tool_receipt_refs,
                            model_receipts,
                            executions,
                        );
                    }
                    ProviderToolLoopStatus::Clarifying => {
                        return self.chat_clarifying_result(
                            chat_truth,
                            chat_thread_id,
                            container_id,
                            turn_id,
                            terminal_payload,
                            cited_refs,
                            tool_receipt_refs,
                        );
                    }
                    ProviderToolLoopStatus::NeedsTask => {
                        return self.chat_needs_task_result(
                            chat_truth,
                            chat_thread_id,
                            container_id,
                            turn_id,
                            terminal_payload,
                            cited_refs,
                            tool_receipt_refs,
                        );
                    }
                    ProviderToolLoopStatus::Blocked | ProviderToolLoopStatus::Failed => {
                        chat_truth.append_event(
                            chat_thread_id,
                            container_id,
                            "chat_provider_tool_status_recoverable",
                            json!({
                                "schema_version": CHAT_TRUTH_SCHEMA_VERSION,
                                "turn_id": turn_id,
                                "status": status,
                                "payload": terminal_payload,
                            }),
                            None,
                        )?;
                        continue 'provider_loop;
                    }
                    _ => {}
                }
            }
        }
        let content = "ChatRuntime returned the read-only tool results to the model, but the model did not produce a final answer within the tool-loop limit. Narrow the question or use TASK for work that needs more tool steps.".to_string();
        chat_truth.append_event(
            chat_thread_id,
            container_id,
            "chat_tool_loop_recovered_without_model_answer",
            json!({
                "schema_version": CHAT_TRUTH_SCHEMA_VERSION,
                "turn_id": turn_id,
                "status": "answered",
                "runtime_generated": true,
                "provider_tool_execution_count": executions.len(),
                "model_call_ids": model_receipts.iter().map(|receipt| receipt.model_call_id.clone()).collect::<Vec<_>>(),
                "message": content,
            }),
            None,
        )?;
        self.chat_answer_result(
            chat_truth,
            chat_thread_id,
            container_id,
            turn_id,
            content,
            cited_refs,
            tool_receipt_refs,
            model_receipts,
            executions,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn append_chat_provider_tool_skipped_results(
        &self,
        chat_truth: &ChatTruthStore,
        chat_thread_id: &str,
        container_id: &str,
        turn_id: &str,
        model_truth: &ProcessTruthStore,
        token: &CapabilityToken,
        receipt: &ModelCallReceipt,
        loop_exec: &ProviderToolLoopExecutor,
        provider_tool_batch_id: &str,
        reason: &str,
    ) -> io::Result<()> {
        self.append_chat_provider_tool_skipped_results_from(
            chat_truth,
            chat_thread_id,
            container_id,
            turn_id,
            model_truth,
            token,
            receipt,
            loop_exec,
            provider_tool_batch_id,
            0,
            reason,
        )?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn append_chat_provider_tool_skipped_results_from(
        &self,
        chat_truth: &ChatTruthStore,
        chat_thread_id: &str,
        container_id: &str,
        turn_id: &str,
        model_truth: &ProcessTruthStore,
        token: &CapabilityToken,
        receipt: &ModelCallReceipt,
        loop_exec: &ProviderToolLoopExecutor,
        provider_tool_batch_id: &str,
        start_index: usize,
        reason: &str,
    ) -> io::Result<usize> {
        let mut skipped_count = 0_usize;
        for (index, tool_call) in receipt
            .provider_tool_calls
            .iter()
            .enumerate()
            .skip(start_index)
        {
            let tool_result =
                loop_exec.skipped_result(tool_call, reason, provider_tool_batch_id, index);
            record_provider_tool_result_with_metadata(
                model_truth,
                &token.pid,
                receipt.provider.as_str(),
                provider_protocol(&receipt.provider),
                &tool_call.id,
                &tool_result,
                ProviderToolResultMetadata {
                    provider_tool_call_index: Some(index),
                    provider_tool_batch_id: Some(provider_tool_batch_id.to_string()),
                },
            )?;
            chat_truth.append_event(
                chat_thread_id,
                container_id,
                "chat_provider_tool_result_appended",
                json!({
                    "schema_version": CHAT_TRUTH_SCHEMA_VERSION,
                    "turn_id": turn_id,
                    "model_call_id": receipt.model_call_id.clone(),
                    "provider_tool_batch_id": provider_tool_batch_id,
                    "provider_tool_call_id": tool_call.id.clone(),
                    "provider_tool_call_index": index,
                    "provider_tool_name": provider_tool_call_name(tool_call).ok(),
                    "status": "skipped",
                    "tool_result_status": "skipped",
                    "reason": reason,
                    "tool_result_completeness": "provider_tool_call_answered",
                }),
                None,
            )?;
            skipped_count = skipped_count.saturating_add(1);
        }
        Ok(skipped_count)
    }

    #[allow(clippy::too_many_arguments)]
    fn execute_chat_tool_call(
        &self,
        chat_truth: &ChatTruthStore,
        chat_thread_id: &str,
        container_id: &str,
        turn_id: &str,
        user_message: &str,
        model_truth: &ProcessTruthStore,
        token: &CapabilityToken,
        guard: &WorkspaceGuard,
        registry: &ProviderToolRegistry,
        receipt: &ModelCallReceipt,
        tool_call: &ProviderToolCall,
        provider_tool_batch_id: &str,
        provider_tool_call_index: usize,
    ) -> io::Result<ProviderToolExecution> {
        let provider_tool_name = provider_tool_call_name(tool_call).ok();
        let decision = match registry.decision_for_tool_call(tool_call) {
            Ok(decision) => decision,
            Err(err) => {
                let tool_result = json!({
                    "status": "needs_task",
                    "error_code": err.error_code,
                    "message": err.message,
                    "provider_tool_name": err.provider_tool_name,
                    "provider_tool_call_id": err.provider_tool_call_id,
                    "suggested_task": {
                        "goal": user_message,
                        "reason": "ChatRuntime can only call read-only tools. Use TaskRuntime for mutation, approval, terminal execution, or artifact delivery.",
                    },
                    "mutation_forbidden": true,
                });
                record_provider_tool_result_with_metadata(
                    model_truth,
                    &token.pid,
                    receipt.provider.as_str(),
                    provider_protocol(&receipt.provider),
                    &tool_call.id,
                    &tool_result,
                    ProviderToolResultMetadata {
                        provider_tool_call_index: Some(provider_tool_call_index),
                        provider_tool_batch_id: Some(provider_tool_batch_id.to_string()),
                    },
                )?;
                chat_truth.append_event(
                    chat_thread_id,
                    container_id,
                    "chat_mutation_or_unknown_tool_blocked",
                    json!({
                        "schema_version": CHAT_TRUTH_SCHEMA_VERSION,
                        "turn_id": turn_id,
                        "model_call_id": receipt.model_call_id,
                        "provider_tool_batch_id": provider_tool_batch_id,
                        "provider_tool_call_id": tool_call.id,
                        "provider_tool_name": provider_tool_name,
                        "mutation_allowed": false,
                        "error": tool_result,
                    }),
                    None,
                )?;
                return Ok(ProviderToolExecution {
                    provider_tool_batch_id: provider_tool_batch_id.to_string(),
                    provider_tool_call_id: tool_call.id.clone(),
                    provider_tool_call_index,
                    provider_tool_name,
                    capability_id: err.capability_id,
                    status: ProviderToolLoopStatus::NeedsTask,
                    tool_result,
                });
            }
        };
        if provider_tool_is_mutation_apply_capability(&decision.capability_id)
            || decision.capability_id.starts_with("process.preview")
            || decision.capability_id == "process.request_preview"
            || decision.capability_id == "terminal.run_command"
            || decision.capability_id == "process.complete"
        {
            let tool_result = json!({
                "status": "needs_task",
                "capability_id": decision.capability_id,
                "suggested_task": {
                    "goal": user_message,
                    "reason": "ChatRuntime can only call read-only tools. Use TaskRuntime for mutation, approval, terminal execution, process completion, or artifact delivery.",
                },
                "mutation_forbidden": true,
            });
            record_provider_tool_result_with_metadata(
                model_truth,
                &token.pid,
                receipt.provider.as_str(),
                provider_protocol(&receipt.provider),
                &tool_call.id,
                &tool_result,
                ProviderToolResultMetadata {
                    provider_tool_call_index: Some(provider_tool_call_index),
                    provider_tool_batch_id: Some(provider_tool_batch_id.to_string()),
                },
            )?;
            return Ok(ProviderToolExecution {
                provider_tool_batch_id: provider_tool_batch_id.to_string(),
                provider_tool_call_id: tool_call.id.clone(),
                provider_tool_call_index,
                provider_tool_name,
                capability_id: Some(decision.capability_id),
                status: ProviderToolLoopStatus::NeedsTask,
                tool_result,
            });
        }
        match decision.capability_id.as_str() {
            "chat.answer" => self.chat_control_execution(
                model_truth,
                token,
                receipt,
                tool_call,
                provider_tool_batch_id,
                provider_tool_call_index,
                provider_tool_name,
                decision,
                ProviderToolLoopStatus::Answered,
            ),
            "chat.clarify" => self.chat_control_execution(
                model_truth,
                token,
                receipt,
                tool_call,
                provider_tool_batch_id,
                provider_tool_call_index,
                provider_tool_name,
                decision,
                ProviderToolLoopStatus::Clarifying,
            ),
            "chat.needs_task" => self.chat_control_execution(
                model_truth,
                token,
                receipt,
                tool_call,
                provider_tool_batch_id,
                provider_tool_call_index,
                provider_tool_name,
                decision,
                ProviderToolLoopStatus::NeedsTask,
            ),
            _ => {
                let executor = ReadOnlyCapabilityExecutor::new(
                    guard.clone(),
                    model_truth.clone(),
                    token.clone(),
                    format!("chat_runtime_{chat_thread_id}"),
                )
                .without_process_truth_receipt_events();
                let receipt_value = match executor.execute(&decision) {
                    Ok(receipt) => receipt,
                    Err(err) => CapabilityReceipt {
                        capability_id: decision.capability_id.clone(),
                        job_id: token.job_id.clone(),
                        pid: token.pid.clone(),
                        status: "failed".to_string(),
                        data: json!({
                            "reason": "read-only capability execution failed",
                            "error": err.to_string(),
                            "model_call_id": receipt.model_call_id.clone(),
                            "provider_tool_batch_id": provider_tool_batch_id,
                            "provider_tool_call_id": tool_call.id.clone(),
                            "provider_tool_name": provider_tool_name.clone(),
                            "no_workspace_mutation": true,
                        }),
                    },
                };
                let receipt_status = receipt_value.status.clone();
                let receipt_data = receipt_value.data.clone();
                let capability_id = receipt_value.capability_id.clone();
                let tool_result =
                    executor.provider_tool_result_from_receipt(turn_id, &receipt_value)?;
                record_provider_tool_result_with_metadata(
                    model_truth,
                    &token.pid,
                    receipt.provider.as_str(),
                    provider_protocol(&receipt.provider),
                    &tool_call.id,
                    &tool_result,
                    ProviderToolResultMetadata {
                        provider_tool_call_index: Some(provider_tool_call_index),
                        provider_tool_batch_id: Some(provider_tool_batch_id.to_string()),
                    },
                )?;
                chat_truth.append_event(
                    chat_thread_id,
                    container_id,
                    "chat_readonly_tool_executed",
                    json!({
                        "schema_version": CHAT_TRUTH_SCHEMA_VERSION,
                        "turn_id": turn_id,
                        "model_call_id": receipt.model_call_id.clone(),
                        "provider_tool_batch_id": provider_tool_batch_id,
                        "provider_tool_call_id": tool_call.id.clone(),
                        "provider_tool_call_index": provider_tool_call_index,
                        "provider_tool_name": provider_tool_name.clone(),
                        "capability_id": capability_id.clone(),
                        "receipt_status": receipt_status.clone(),
                        "execution_error": if receipt_status == "failed" { receipt_data.get("error").cloned() } else { None },
                        "mutation_allowed": false,
                        "receipt_ref": tool_result.get("receipt_ref").cloned(),
                    }),
                    None,
                )?;
                chat_truth.append_event(
                    chat_thread_id,
                    container_id,
                    "chat_readonly_capability_receipt",
                    json!({
                        "schema_version": CHAT_TRUTH_SCHEMA_VERSION,
                        "turn_id": turn_id,
                        "model_call_id": receipt.model_call_id.clone(),
                        "provider_tool_batch_id": provider_tool_batch_id,
                        "provider_tool_call_id": tool_call.id.clone(),
                        "provider_tool_call_index": provider_tool_call_index,
                        "provider_tool_name": provider_tool_name.clone(),
                        "capability_id": capability_id.clone(),
                        "receipt_status": receipt_status.clone(),
                        "receipt_data": receipt_data,
                        "mutation_allowed": false,
                        "receipt_ref": tool_result.get("receipt_ref").cloned(),
                    }),
                    None,
                )?;
                Ok(ProviderToolExecution {
                    provider_tool_batch_id: provider_tool_batch_id.to_string(),
                    provider_tool_call_id: tool_call.id.clone(),
                    provider_tool_call_index,
                    provider_tool_name,
                    capability_id: Some(capability_id),
                    status: ProviderToolLoopStatus::Continue,
                    tool_result,
                })
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn chat_control_execution(
        &self,
        model_truth: &ProcessTruthStore,
        token: &CapabilityToken,
        receipt: &ModelCallReceipt,
        tool_call: &ProviderToolCall,
        provider_tool_batch_id: &str,
        provider_tool_call_index: usize,
        provider_tool_name: Option<String>,
        decision: NextActionDecision,
        status: ProviderToolLoopStatus,
    ) -> io::Result<ProviderToolExecution> {
        let tool_result = match decision.capability_id.as_str() {
            "chat.answer" => json!({
                "status": "answered",
                "capability_id": decision.capability_id,
                "content": decision.output_spec.get("content")
                    .or_else(|| decision.output_spec.get("answer"))
                    .or_else(|| decision.output_spec.get("message"))
                    .and_then(Value::as_str)
                    .unwrap_or(""),
                "cited_refs": string_array_value(&decision.output_spec, "cited_refs"),
                "reason": decision.reason,
            }),
            "chat.clarify" => json!({
                "status": "clarifying",
                "capability_id": decision.capability_id,
                "question": decision.output_spec.get("question").and_then(Value::as_str).unwrap_or(""),
                "missing_fact": decision.output_spec.get("missing_fact").and_then(Value::as_str),
                "reason": decision.reason,
            }),
            "chat.needs_task" => json!({
                "status": "needs_task",
                "capability_id": decision.capability_id,
                "goal": decision.output_spec.get("goal").and_then(Value::as_str).unwrap_or(""),
                "context_pack_id": decision.output_spec.get("context_pack_id").and_then(Value::as_str),
                "reason": decision.reason,
            }),
            _ => json!({
                "status": "blocked",
                "capability_id": decision.capability_id,
                "reason": "unsupported chat control capability",
            }),
        };
        let _control_receipt = CapabilityReceipt {
            capability_id: decision.capability_id.clone(),
            job_id: token.job_id.clone(),
            pid: token.pid.clone(),
            status: "success".to_string(),
            data: tool_result.clone(),
        };
        record_provider_tool_result_with_metadata(
            model_truth,
            &token.pid,
            receipt.provider.as_str(),
            provider_protocol(&receipt.provider),
            &tool_call.id,
            &tool_result,
            ProviderToolResultMetadata {
                provider_tool_call_index: Some(provider_tool_call_index),
                provider_tool_batch_id: Some(provider_tool_batch_id.to_string()),
            },
        )?;
        Ok(ProviderToolExecution {
            provider_tool_batch_id: provider_tool_batch_id.to_string(),
            provider_tool_call_id: tool_call.id.clone(),
            provider_tool_call_index,
            provider_tool_name,
            capability_id: Some(decision.capability_id),
            status,
            tool_result,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn chat_answer_result(
        &self,
        chat_truth: &ChatTruthStore,
        chat_thread_id: &str,
        container_id: &str,
        turn_id: &str,
        content: String,
        cited_refs: Vec<String>,
        tool_receipt_refs: Vec<String>,
        model_receipts: Vec<ModelCallReceipt>,
        executions: Vec<ProviderToolExecution>,
    ) -> io::Result<ChatTurnResult> {
        let content_ref = chat_truth.write_chat_blob(
            chat_thread_id,
            &format!("turns/{turn_id}_assistant.txt"),
            content.as_bytes(),
        )?;
        chat_truth.append_event(
            chat_thread_id,
            container_id,
            "chat_assistant_answered",
            json!({
                "schema_version": CHAT_TRUTH_SCHEMA_VERSION,
                "turn_id": turn_id,
                "assistant_content_ref": content_ref,
                "cited_refs": cited_refs,
                "tool_receipt_refs": tool_receipt_refs,
                "model_call_ids": model_receipts.iter().map(|receipt| receipt.model_call_id.clone()).collect::<Vec<_>>(),
                "provider_tool_execution_count": executions.len(),
            }),
            None,
        )?;
        Ok(ChatTurnResult {
            chat_thread_id: chat_thread_id.to_string(),
            turn_id: turn_id.to_string(),
            status: ChatTurnStatus::Answered,
            assistant_content: Some(content),
            suggested_task: None,
            cited_refs,
            tool_receipt_refs,
            context_window_receipt_ref: None,
            events: chat_truth.read_events(chat_thread_id)?,
        })
    }

    fn chat_clarifying_result(
        &self,
        chat_truth: &ChatTruthStore,
        chat_thread_id: &str,
        container_id: &str,
        turn_id: &str,
        payload: Value,
        cited_refs: Vec<String>,
        tool_receipt_refs: Vec<String>,
    ) -> io::Result<ChatTurnResult> {
        let question = payload
            .get("question")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        chat_truth.append_event(
            chat_thread_id,
            container_id,
            "chat_clarification_requested",
            json!({
                "schema_version": CHAT_TRUTH_SCHEMA_VERSION,
                "turn_id": turn_id,
                "question": question,
                "payload": payload,
            }),
            None,
        )?;
        Ok(ChatTurnResult {
            chat_thread_id: chat_thread_id.to_string(),
            turn_id: turn_id.to_string(),
            status: ChatTurnStatus::Clarifying,
            assistant_content: Some(question),
            suggested_task: None,
            cited_refs,
            tool_receipt_refs,
            context_window_receipt_ref: None,
            events: chat_truth.read_events(chat_thread_id)?,
        })
    }

    fn chat_needs_task_result(
        &self,
        chat_truth: &ChatTruthStore,
        chat_thread_id: &str,
        container_id: &str,
        turn_id: &str,
        payload: Value,
        cited_refs: Vec<String>,
        tool_receipt_refs: Vec<String>,
    ) -> io::Result<ChatTurnResult> {
        let suggested = suggested_task_from_payload(payload.clone());
        chat_truth.append_event(
            chat_thread_id,
            container_id,
            "chat_needs_task_suggested",
            json!({
                "schema_version": CHAT_TRUTH_SCHEMA_VERSION,
                "turn_id": turn_id,
                "suggested_task": suggested.clone(),
                "payload": payload.clone(),
            }),
            None,
        )?;
        Ok(ChatTurnResult {
            chat_thread_id: chat_thread_id.to_string(),
            turn_id: turn_id.to_string(),
            status: ChatTurnStatus::NeedsTask,
            assistant_content: None,
            suggested_task: suggested,
            cited_refs,
            tool_receipt_refs,
            context_window_receipt_ref: None,
            events: chat_truth.read_events(chat_thread_id)?,
        })
    }

    fn chat_failed_result(
        &self,
        chat_truth: &ChatTruthStore,
        chat_thread_id: &str,
        container_id: &str,
        turn_id: &str,
        receipt: &ModelCallReceipt,
    ) -> io::Result<ChatTurnResult> {
        chat_truth.append_event(
            chat_thread_id,
            container_id,
            "chat_turn_failed",
            json!({
                "schema_version": CHAT_TRUTH_SCHEMA_VERSION,
                "turn_id": turn_id,
                "model_call_id": receipt.model_call_id,
                "model_call_status": receipt.status,
                "error": receipt.error,
            }),
            None,
        )?;
        Ok(ChatTurnResult {
            chat_thread_id: chat_thread_id.to_string(),
            turn_id: turn_id.to_string(),
            status: ChatTurnStatus::Failed,
            assistant_content: None,
            suggested_task: None,
            cited_refs: Vec::new(),
            tool_receipt_refs: Vec::new(),
            context_window_receipt_ref: None,
            events: chat_truth.read_events(chat_thread_id)?,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn record_chat_model_receipt_projection(
        &self,
        chat_truth: &ChatTruthStore,
        chat_thread_id: &str,
        container_id: &str,
        turn_id: &str,
        subturn_index: usize,
        receipt: &ModelCallReceipt,
    ) -> io::Result<()> {
        let receipt_ref = chat_truth.write_chat_blob(
            chat_thread_id,
            &format!(
                "model_receipts/{}_{}_{}.json",
                safe_blob_name(turn_id),
                subturn_index,
                safe_blob_name(&receipt.model_call_id)
            ),
            &serde_json::to_vec_pretty(receipt).map_err(crate::json_err)?,
        )?;
        chat_truth.append_event(
            chat_thread_id,
            container_id,
            "chat_model_receipt_recorded",
            json!({
                "schema_version": CHAT_TRUTH_SCHEMA_VERSION,
                "turn_id": turn_id,
                "subturn_index": subturn_index,
                "model_call_id": receipt.model_call_id.clone(),
                "status": receipt.status.clone(),
                "provider": receipt.provider.clone(),
                "model": receipt.model.clone(),
                "operation": receipt.operation.clone(),
                "output_ref": receipt.output_ref.clone(),
                "provider_transcript_ref": receipt.provider_transcript_ref.clone(),
                "provider_transcript_summary_ref": receipt.provider_transcript_summary_ref.clone(),
                "provider_tool_call_count": receipt.provider_tool_calls.len(),
                "receipt_ref": receipt_ref,
                "truth_layer": "chat_truth_projection",
            }),
            Some(receipt_ref),
        )?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn sync_chat_provider_transcript(
        &self,
        chat_truth: &ChatTruthStore,
        chat_thread_id: &str,
        container_id: &str,
        turn_id: &str,
        receipt: &ModelCallReceipt,
        live_suffix_ref: Option<String>,
        compacted_until_seq: Option<u64>,
    ) -> io::Result<()> {
        let Some(transcript_id) = receipt.provider_transcript_id.clone() else {
            return Ok(());
        };
        let Some(messages_ref) = receipt.provider_transcript_ref.clone() else {
            return Ok(());
        };
        let transcript = ChatProviderTranscript {
            transcript_id,
            chat_thread_id: chat_thread_id.to_string(),
            provider: receipt.provider.clone(),
            model: receipt.model.clone(),
            messages_ref: messages_ref.clone(),
            summary_ref: receipt.provider_transcript_summary_ref.clone(),
            live_suffix_ref: live_suffix_ref.clone(),
            compacted_until_seq,
            updated_at_ms: now_ms() as i64,
        };
        chat_truth.upsert_provider_transcript(transcript.clone())?;
        chat_truth.append_event(
            chat_thread_id,
            container_id,
            "chat_provider_transcript_updated",
            json!({
                "schema_version": CHAT_TRUTH_SCHEMA_VERSION,
                "turn_id": turn_id,
                "model_call_id": receipt.model_call_id.clone(),
                "transcript_id": transcript.transcript_id.clone(),
                "provider": transcript.provider.clone(),
                "model": transcript.model.clone(),
                "messages_ref": messages_ref.clone(),
                "summary_ref": transcript.summary_ref.clone(),
                "live_suffix_ref": live_suffix_ref,
                "compacted_until_seq": compacted_until_seq,
            }),
            None,
        )?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn compact_chat_model_context(
        &self,
        chat_truth: &ChatTruthStore,
        chat_thread_id: &str,
        container_id: &str,
        turn_id: &str,
        message: &str,
        context_pack: &Option<ContextPack>,
        model_truth: &ProcessTruthStore,
        token: &CapabilityToken,
        provider: Arc<dyn ModelProvider>,
        model_config: &ModelInvocationConfig,
        model_config_ref: Option<String>,
        preflight: &crate::ContextWindowPreflight,
    ) -> io::Result<Option<String>> {
        chat_truth.append_event(
            chat_thread_id,
            container_id,
            "chat_context_compaction_started",
            json!({
                "schema_version": CHAT_TRUTH_SCHEMA_VERSION,
                "turn_id": turn_id,
                "scope": preflight.scope,
                "estimate": preflight.estimate,
                "decision": preflight.decision,
            }),
            None,
        )?;
        let existing_events = chat_truth.read_events(chat_thread_id)?;
        let checkpoint = json!({
            "schema": "supernova_chat_context_pre_compaction_checkpoint.v1",
            "chat_thread_id": chat_thread_id,
            "container_id": container_id,
            "turn_id": turn_id,
            "preflight": preflight,
            "events": existing_events,
            "fact_boundary": "Chat compaction is provider-visible context only. It does not create execution facts and does not mutate workspace state.",
        });
        let chat_checkpoint_ref = chat_truth.write_chat_blob(
            chat_thread_id,
            &format!("compactions/{turn_id}_checkpoint.json"),
            &serde_json::to_vec_pretty(&checkpoint).map_err(crate::json_err)?,
        )?;
        let model_checkpoint_ref = model_truth.write_blob(
            &format!("chat_context_compactions/{turn_id}_checkpoint.json"),
            &serde_json::to_vec_pretty(&checkpoint).map_err(crate::json_err)?,
        )?;
        chat_truth.append_event(
            chat_thread_id,
            container_id,
            "context_window_checkpoint_created",
            json!({
                "schema_version": crate::CONTEXT_WINDOW_EVENT_SCHEMA_VERSION,
                "turn_id": turn_id,
                "scope": preflight.scope,
                "chat_checkpoint_ref": chat_checkpoint_ref,
                "model_checkpoint_ref": model_checkpoint_ref.clone(),
                "estimate": preflight.estimate,
                "decision": preflight.decision,
            }),
            None,
        )?;
        let compaction_input = json!({
            "schema": "supernova_chat_context_compaction_input.v1",
            "chat_thread_id": chat_thread_id,
            "container_id": container_id,
            "turn_id": turn_id,
            "current_user_message": message,
            "context_pack": context_pack,
            "checkpoint_ref": model_checkpoint_ref,
            "target_summary_tokens": model_config.context_window.max_summary_tokens,
            "required_output": {
                "schema": "supernova_chat_context_summary.v1",
                "summary": "<provider-visible summary>",
                "important_decisions": [],
                "source_refs": [],
                "task_refs": [],
                "chat_refs": [],
                "memory_refs": [],
                "known_constraints": [],
                "open_questions": []
            },
            "fact_boundary": "Only Kernel receipts and typed refs are execution facts. This summary is context only.",
        });
        let instruction_ref = model_truth.write_blob(
            &format!("chat_context_compactions/{turn_id}_instruction.txt"),
            b"Compact the chat-visible context into strict JSON. Preserve user intent, cited refs, tool-result facts, open questions, and known constraints. Do not invent execution facts. Return only JSON matching schema supernova_chat_context_summary.v1.",
        )?;
        let input_ref = model_truth.write_blob(
            &format!("chat_context_compactions/{turn_id}_input.json"),
            &serde_json::to_vec_pretty(&compaction_input).map_err(crate::json_err)?,
        )?;
        let operation = ModelOperation::CompactChatContext;
        let budget =
            ModelContextProfile::for_provider(provider.as_ref(), &operation).budget_for(&operation);
        let provider_name = provider.provider_name().to_string();
        let provider_model_name = provider.model_name_for_operation(&ModelOperation::ChatTurn);
        let provider_protocol_name = provider_protocol(&provider_name).to_string();
        let mut action = chat_model_action(
            &token.job_id,
            &token.pid,
            turn_id,
            operation.clone(),
            instruction_ref,
            vec![input_ref],
            provider.as_ref(),
            budget,
        );
        action.model = effective_model_name(model_config, provider.as_ref(), &operation);
        action.required = preflight.decision.hard_block_if_compaction_fails;
        let model_call_id = format!("mcall_{}_chat_compact_{}", turn_id, now_ms());
        chat_truth.append_event(
            chat_thread_id,
            container_id,
            "context_window_compaction_model_call_started",
            json!({
                "schema_version": crate::CONTEXT_WINDOW_EVENT_SCHEMA_VERSION,
                "turn_id": turn_id,
                "scope": preflight.scope,
                "model_call_id": model_call_id,
                "operation": "model.compact_chat_context",
            }),
            None,
        )?;
        let receipt = ModelRuntime::new(model_truth.clone(), token.clone(), provider)
            .with_model_invocation_config(model_config.clone(), model_config_ref)
            .with_model_call_id_override(Some(model_call_id.clone()))
            .compact_chat_context(action)?;
        chat_truth.append_event(
            chat_thread_id,
            container_id,
            "context_window_compaction_model_call_completed",
            json!({
                "schema_version": crate::CONTEXT_WINDOW_EVENT_SCHEMA_VERSION,
                "turn_id": turn_id,
                "scope": preflight.scope,
                "model_call_id": receipt.model_call_id.clone(),
                "status": receipt.status.clone(),
                "output_ref": receipt.output_ref.clone(),
                "schema_validation": receipt.schema_validation.clone(),
                "error": receipt.error.clone(),
            }),
            None,
        )?;
        if receipt.status != "success" {
            return self.chat_compaction_failed(
                chat_truth,
                chat_thread_id,
                container_id,
                turn_id,
                preflight,
                json!({
                    "model_call_id": receipt.model_call_id,
                    "status": receipt.status,
                    "error": receipt.error,
                }),
            );
        }
        let output_ref = receipt.output_ref.clone().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "successful chat compaction did not produce output_ref",
            )
        })?;
        let summary_text = model_output_text(model_truth, &receipt);
        let summary_json = serde_json::from_str::<Value>(&summary_text).map_err(crate::json_err);
        let summary_json = match summary_json {
            Ok(value) => value,
            Err(err) => {
                return self.chat_compaction_failed(
                    chat_truth,
                    chat_thread_id,
                    container_id,
                    turn_id,
                    preflight,
                    json!({
                        "model_call_id": receipt.model_call_id,
                        "status": "failed",
                        "error": {
                            "error_code": "CHAT_COMPACTION_OUTPUT_INVALID_JSON",
                            "message": err.to_string(),
                        },
                    }),
                );
            }
        };
        if let Err(err) = crate::validate_chat_context_summary(&summary_json) {
            return self.chat_compaction_failed(
                chat_truth,
                chat_thread_id,
                container_id,
                turn_id,
                preflight,
                json!({
                    "model_call_id": receipt.model_call_id,
                    "status": "failed",
                    "error": {
                        "error_code": "CHAT_COMPACTION_SUMMARY_INVALID",
                        "message": err.to_string(),
                    },
                }),
            );
        }
        let chat_summary_ref = chat_truth.write_chat_blob(
            chat_thread_id,
            &format!("compactions/{turn_id}_summary.json"),
            &serde_json::to_vec_pretty(&summary_json).map_err(crate::json_err)?,
        )?;
        chat_truth.append_event(
            chat_thread_id,
            container_id,
            "chat_context_compaction_completed",
            json!({
                "schema_version": CHAT_TRUTH_SCHEMA_VERSION,
                "turn_id": turn_id,
                "chat_summary_ref": chat_summary_ref.clone(),
                "model_summary_ref": output_ref.clone(),
                "model_call_id": receipt.model_call_id.clone(),
                "compacted_until_event_seq": Value::Null,
            }),
            Some(chat_summary_ref.clone()),
        )?;
        let mut provider_replacement_ref = None;
        let mut provider_live_suffix_ref = None;
        let mut compacted_until_message_index = None;
        let mut protocol_validation = None;
        if let Some(replacement) = replace_provider_visible_transcript_with_summary(
            model_truth,
            &token.pid,
            &provider_name,
            &provider_protocol_name,
            &summary_text,
            model_config.context_window.min_live_suffix_turns,
            "chat_context_window_compaction",
        )? {
            provider_replacement_ref = Some(replacement.new_transcript_ref.clone());
            provider_live_suffix_ref = Some(replacement.live_suffix_ref.clone());
            compacted_until_message_index = Some(replacement.compacted_until_message_index);
            chat_truth.upsert_provider_transcript(ChatProviderTranscript {
                transcript_id: replacement.transcript_id.clone(),
                chat_thread_id: chat_thread_id.to_string(),
                provider: provider_name.clone(),
                model: provider_model_name.clone(),
                messages_ref: replacement.new_transcript_ref.clone(),
                summary_ref: Some(replacement.summary_ref.clone()),
                live_suffix_ref: Some(replacement.live_suffix_ref.clone()),
                compacted_until_seq: None,
                updated_at_ms: now_ms() as i64,
            })?;
            if let Some(state) = replay_provider_transcript_state(
                model_truth,
                &provider_name,
                &provider_protocol_name,
            )? {
                let messages = read_provider_messages(model_truth, &state)?;
                let validation =
                    ProviderTranscriptProtocolValidator::validate_deepseek_native_messages(
                        &messages,
                    )?;
                protocol_validation = Some(validation.clone());
                chat_truth.append_event(
                    chat_thread_id,
                    container_id,
                    "chat_provider_transcript_compacted",
                    json!({
                        "schema_version": CHAT_TRUTH_SCHEMA_VERSION,
                        "turn_id": turn_id,
                        "provider": provider_name.clone(),
                        "protocol": provider_protocol_name.clone(),
                        "transcript_id": replacement.transcript_id,
                        "old_transcript_ref": replacement.old_transcript_ref,
                        "new_transcript_ref": replacement.new_transcript_ref,
                        "summary_ref": replacement.summary_ref,
                        "live_suffix_ref": replacement.live_suffix_ref,
                        "compacted_until_message_index": replacement.compacted_until_message_index,
                        "message_count": replacement.message_count,
                        "pending_tool_call_count": replacement.pending_tool_call_count,
                        "protocol_validation": validation,
                    }),
                    None,
                )?;
                if !validation.valid {
                    return self.chat_compaction_failed(
                        chat_truth,
                        chat_thread_id,
                        container_id,
                        turn_id,
                        preflight,
                        json!({
                            "error_code": "CHAT_PROVIDER_TRANSCRIPT_PROTOCOL_INVALID_AFTER_COMPACTION",
                            "validation": validation,
                        }),
                    );
                }
            }
        }
        chat_truth.append_event(
            chat_thread_id,
            container_id,
            "context_window_visible_context_replaced",
            json!({
                "schema_version": crate::CONTEXT_WINDOW_EVENT_SCHEMA_VERSION,
                "turn_id": turn_id,
                "scope": preflight.scope,
                "summary_ref": output_ref.clone(),
                "chat_summary_ref": chat_summary_ref,
                "provider_transcript_ref": provider_replacement_ref.clone(),
                "live_suffix_ref": provider_live_suffix_ref.clone(),
                "replacement_kind": "chat_model_action_input_refs_and_provider_visible_transcript",
            }),
            None,
        )?;
        chat_truth.append_event(
            chat_thread_id,
            container_id,
            "context_window_protocol_validated",
            json!({
                "schema_version": crate::CONTEXT_WINDOW_EVENT_SCHEMA_VERSION,
                "turn_id": turn_id,
                "scope": preflight.scope,
                "valid": protocol_validation.as_ref().map(|item| item.valid).unwrap_or(true),
                "validation_kind": "chat_provider_visible_transcript",
                "pending_tool_call_ids": protocol_validation.as_ref().map(|item| item.pending_tool_call_ids.clone()).unwrap_or_default(),
                "errors": protocol_validation.as_ref().map(|item| item.errors.clone()).unwrap_or_default(),
            }),
            None,
        )?;
        let mut visible_input_ref = output_ref.clone();
        let reestimate_parts = ContextWindowRequestParts {
            provider: receipt.provider,
            model: receipt.model,
            context_window_tokens: preflight.estimate.context_window_tokens,
            input_payloads: vec![summary_text],
            reserved_output_tokens: Some(preflight.estimate.reserved_output_tokens),
            reserved_reasoning_tokens: Some(preflight.estimate.reserved_reasoning_tokens),
            ..ContextWindowRequestParts::default()
        };
        let reestimate =
            ContextWindowController::estimate(&model_config.context_window, &reestimate_parts);
        chat_truth.append_event(
            chat_thread_id,
            container_id,
            "context_window_reestimate_completed",
            json!({
                "schema_version": crate::CONTEXT_WINDOW_EVENT_SCHEMA_VERSION,
                "turn_id": turn_id,
                "scope": preflight.scope,
                "estimate": reestimate.clone(),
            }),
            None,
        )?;
        if reestimate.usage_ratio >= model_config.context_window.emergency_ratio
            || reestimate.estimated_total_tokens > reestimate.context_window_tokens
        {
            let emergency_payload = json!({
                "schema": "supernova_chat_context_emergency_trim.v1",
                "chat_thread_id": chat_thread_id,
                "container_id": container_id,
                "turn_id": turn_id,
                "summary_ref": output_ref.clone(),
                "current_user_message": message,
                "context_pack_ref": context_pack.as_ref().map(|pack| pack.context_pack_id.clone()),
                "provider_transcript_ref": provider_replacement_ref.clone(),
                "live_suffix_ref": provider_live_suffix_ref.clone(),
                "compacted_until_message_index": compacted_until_message_index,
                "instruction": "Continue the same chat turn from this compacted handoff. Use refs and read-only receipts as facts; do not treat summary text as execution proof.",
                "fact_boundary": "Chat emergency trim is provider-visible context only and cannot authorize mutation.",
            });
            let emergency_trim_ref = model_truth.write_blob(
                &format!("chat_context_compactions/{turn_id}_emergency_trim.json"),
                &serde_json::to_vec_pretty(&emergency_payload).map_err(crate::json_err)?,
            )?;
            visible_input_ref = emergency_trim_ref.clone();
            let emergency_trim_text = model_truth
                .resolve_blob_ref(&emergency_trim_ref)
                .ok()
                .and_then(|path| fs::read_to_string(path).ok())
                .unwrap_or_default();
            let emergency_reestimate_parts = ContextWindowRequestParts {
                provider: provider_name.clone(),
                model: provider_model_name.clone(),
                context_window_tokens: preflight.estimate.context_window_tokens,
                input_payloads: vec![emergency_trim_text],
                reserved_output_tokens: Some(preflight.estimate.reserved_output_tokens),
                reserved_reasoning_tokens: Some(preflight.estimate.reserved_reasoning_tokens),
                ..ContextWindowRequestParts::default()
            };
            let emergency_reestimate = ContextWindowController::estimate(
                &model_config.context_window,
                &emergency_reestimate_parts,
            );
            chat_truth.append_event(
                chat_thread_id,
                container_id,
                "context_window_emergency_trim_applied",
                json!({
                    "schema_version": crate::CONTEXT_WINDOW_EVENT_SCHEMA_VERSION,
                    "turn_id": turn_id,
                    "scope": preflight.scope,
                    "estimate_after_compaction": reestimate.clone(),
                    "estimate_after_emergency_trim": emergency_reestimate.clone(),
                    "summary_ref": output_ref.clone(),
                    "provider_transcript_ref": provider_replacement_ref.clone(),
                    "live_suffix_ref": provider_live_suffix_ref.clone(),
                    "emergency_trim_ref": emergency_trim_ref,
                    "compacted_until_message_index": compacted_until_message_index,
                    "error_code": if emergency_reestimate.estimated_total_tokens > emergency_reestimate.context_window_tokens {
                        "CHAT_CONTEXT_WINDOW_EXCEEDED_AFTER_EMERGENCY_TRIM"
                    } else {
                        "CHAT_CONTEXT_WINDOW_EMERGENCY_TRIM_APPLIED"
                    },
                }),
                None,
            )?;
            if emergency_reestimate.estimated_total_tokens
                > emergency_reestimate.context_window_tokens
            {
                return Err(io::Error::new(
                    io::ErrorKind::Other,
                    "CHAT_CONTEXT_WINDOW_EXCEEDED_AFTER_EMERGENCY_TRIM",
                ));
            }
        }
        Ok(Some(visible_input_ref))
    }

    #[allow(clippy::too_many_arguments)]
    fn chat_compaction_failed(
        &self,
        chat_truth: &ChatTruthStore,
        chat_thread_id: &str,
        container_id: &str,
        turn_id: &str,
        preflight: &crate::ContextWindowPreflight,
        error: Value,
    ) -> io::Result<Option<String>> {
        chat_truth.append_event(
            chat_thread_id,
            container_id,
            "context_window_compaction_failed",
            json!({
                "schema_version": crate::CONTEXT_WINDOW_EVENT_SCHEMA_VERSION,
                "turn_id": turn_id,
                "scope": preflight.scope,
                "decision": preflight.decision,
                "error": error,
            }),
            None,
        )?;
        if preflight.decision.hard_block_if_compaction_fails {
            Err(io::Error::new(
                io::ErrorKind::Other,
                "chat context compaction failed before a hard-threshold provider request",
            ))
        } else {
            Ok(None)
        }
    }
}

#[cfg(test)]
fn chat_system_prompt() -> String {
    chat_system_prompt_for_language(ResponseLanguage::EnUs)
}

fn chat_system_prompt_for_language(response_language: ResponseLanguage) -> String {
    format!(
        "{}\n\n{}",
        "You are SuperNova ChatRuntime, a Rust Kernel chat runtime. You may answer directly, ask a clarification question, or suggest a TaskRuntime job. Use provider-native tools only. Use chat.answer for a final chat answer, chat.clarify when a missing user fact blocks the answer, and chat.needs_task when the request requires mutation, approval, terminal execution, long-running work, or artifact delivery. When a context pack is provided in the input refs, use it as the primary context before scanning the workspace; call read-only tools only to fill missing facts or verify specific refs. You may call read-only workspace/ref/office/client_env tools to inspect context. Use client_env.* for sanitized local desktop environment facts; local IP, MAC, username, hostname, full PATH, environment variables, proxy details, credentials, and similar sensitive fields require explicit client-env disclosure authorization and must not be guessed. You must never claim that ChatRuntime mutated files or completed a task.",
        response_language.prompt_instruction()
    )
}

fn chat_runtime_model_config(mut config: ModelInvocationConfig) -> ModelInvocationConfig {
    config.decision_protocol = TaskAgentDecisionProtocol::ProviderNativeToolCalls;
    config.tool_calling.enabled = true;
    config.tool_calling.tool_choice = ToolChoicePolicy::Auto;
    config.tool_calling.toolset_mode = ProviderToolsetMode::DomainScoped;
    config
}

fn chat_runtime_capabilities(registry: &ProviderToolRegistry) -> Vec<String> {
    let mut capabilities = vec![
        "model.invoke".to_string(),
        "model.chat_turn".to_string(),
        "model.compact_chat_context".to_string(),
    ];
    capabilities.extend(
        registry
            .bindings
            .values()
            .map(|binding| binding.capability_id.clone()),
    );
    capabilities.sort();
    capabilities.dedup();
    capabilities
}

fn chat_model_action(
    chat_thread_id: &str,
    pid: &str,
    turn_id: &str,
    operation: ModelOperation,
    instruction_ref: String,
    input_refs: Vec<String>,
    provider: &dyn ModelProvider,
    budget: crate::ModelBudget,
) -> ModelAction {
    ModelAction {
        action_id: format!("chat_model_action_{turn_id}"),
        job_id: chat_thread_id.to_string(),
        pid: pid.to_string(),
        reasoning_step_id: turn_id.to_string(),
        operation: operation.clone(),
        instruction_ref,
        input_refs,
        preference_snapshot_ref: None,
        output_schema: match operation {
            ModelOperation::CompactChatContext => crate::chat_context_summary_output_schema(),
            _ => json!({"type": "string"}),
        },
        provider: provider.provider_name().to_string(),
        model: provider.model_name_for_operation(&operation),
        budget,
        failure_policy: ModelFailurePolicy::FailClosed,
        required: true,
    }
}

fn effective_model_name(
    model_config: &ModelInvocationConfig,
    provider: &dyn ModelProvider,
    operation: &ModelOperation,
) -> String {
    let provider_snapshot = provider.capability_snapshot();
    model_config.effective_model_for_operation(provider, operation, &provider_snapshot)
}

fn append_optional_ref(mut refs: Vec<String>, value: Option<String>) -> Vec<String> {
    if let Some(value) = value {
        refs.push(value);
    }
    refs
}

fn provider_protocol(provider: &str) -> &str {
    if provider == "deepseek" {
        "deepseek_chat_completions"
    } else {
        "model_provider_transcript"
    }
}

fn model_output_text(truth: &ProcessTruthStore, receipt: &ModelCallReceipt) -> String {
    receipt
        .output_ref
        .as_deref()
        .and_then(|output_ref| truth.resolve_blob_ref(output_ref).ok())
        .and_then(|path| fs::read_to_string(path).ok())
        .unwrap_or_default()
}

fn chat_tool_result_read_usage(tool_result: &Value) -> (u64, u64) {
    let mut bytes = 0_u64;
    let mut tokens = 0_u64;
    collect_read_usage(tool_result, &mut bytes, &mut tokens);
    (bytes, tokens)
}

fn collect_read_usage(value: &Value, bytes: &mut u64, tokens: &mut u64) {
    match value {
        Value::Object(map) => {
            for key in ["bytes", "size_bytes", "content_bytes", "total_size_bytes"] {
                if let Some(value) = map.get(key).and_then(Value::as_u64) {
                    *bytes = bytes.saturating_add(value);
                }
            }
            if let Some(value) = map.get("tokens_estimated").and_then(Value::as_u64) {
                *tokens = tokens.saturating_add(value);
            }
            if let Some(value) = map.get("content").and_then(Value::as_str) {
                *bytes = bytes.saturating_add(value.len() as u64);
                *tokens = tokens.saturating_add(crate::estimate_text_tokens_conservative(value));
            }
            if let Some(value) = map.get("page").and_then(Value::as_str) {
                *bytes = bytes.saturating_add(value.len() as u64);
                *tokens = tokens.saturating_add(crate::estimate_text_tokens_conservative(value));
            }
            for child in map.values() {
                collect_read_usage(child, bytes, tokens);
            }
        }
        Value::Array(items) => {
            for child in items {
                collect_read_usage(child, bytes, tokens);
            }
        }
        _ => {}
    }
}

fn string_array_value(value: &Value, key: &str) -> Vec<String> {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn suggested_task_from_payload(payload: Value) -> Option<SuggestedTaskRequest> {
    if let Some(task) = payload.get("suggested_task") {
        let goal = task
            .get("goal")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        if !goal.is_empty() {
            return Some(SuggestedTaskRequest {
                goal,
                reason: task
                    .get("reason")
                    .and_then(Value::as_str)
                    .unwrap_or("ChatRuntime determined this requires TaskRuntime.")
                    .to_string(),
                context_pack_id: task
                    .get("context_pack_id")
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
            });
        }
    }
    let goal = payload
        .get("goal")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if goal.is_empty() {
        return None;
    }
    Some(SuggestedTaskRequest {
        goal,
        reason: payload
            .get("reason")
            .and_then(Value::as_str)
            .unwrap_or("ChatRuntime determined this requires TaskRuntime.")
            .to_string(),
        context_pack_id: payload
            .get("context_pack_id")
            .and_then(Value::as_str)
            .map(ToString::to_string),
    })
}

// Kept as a concrete type marker for architecture tests and Rust API callers.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatRuntimeStatus {
    pub runtime_kind: String,
    pub mutation_policy: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use crate::{
        provider_tool_name_for_capability, ContextPackAutoPolicy, ContextPackIncludeMode,
        ContextPackItem, ContextPackItemKind, DeterministicModelProvider, ModelProviderFailure,
        ModelProviderRequest, ModelProviderResponse, ProviderTranscriptMessage,
        ReferenceSourceDirective, CHAT_RUNTIME_MUTATION_FORBIDDEN_POLICY,
        CHAT_RUNTIME_PROVIDER_TOOL_CAPABILITIES,
    };

    fn temp_workspace(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "supernova_chat_runtime_{}_{}",
            name,
            crate::now_ms()
        ));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn chat_runtime_exposes_client_env_readonly_tools() {
        let config = chat_runtime_model_config(ModelInvocationConfig::default());
        let registry =
            ProviderToolRegistry::chat_runtime_readonly(&default_capability_registry(), &config);
        let capabilities = chat_runtime_capabilities(&registry);
        assert!(capabilities.contains(&"client_env.scan_overview".to_string()));
        assert!(capabilities.contains(&"client_env.scan_runtimes".to_string()));
        assert!(CHAT_RUNTIME_PROVIDER_TOOL_CAPABILITIES.contains(&"client_env.scan_network"));
        assert!(chat_system_prompt().contains("client_env.*"));
    }

    #[test]
    fn chat_runtime_system_prompt_uses_response_language() {
        let zh_prompt = chat_system_prompt_for_language(ResponseLanguage::ZhCn);
        let en_prompt = chat_system_prompt_for_language(ResponseLanguage::EnUs);

        assert!(zh_prompt.contains("Use Simplified Chinese"));
        assert!(en_prompt.contains("Use English"));
        assert!(zh_prompt.contains("JSON keys"));
        assert!(en_prompt.contains("JSON keys"));
    }

    #[test]
    fn chat_runtime_plain_answer_records_chat_truth_without_agent_job() {
        let workspace = temp_workspace("plain");
        let provider = Arc::new(
            DeterministicModelProvider::new("deterministic", "chat-model")
                .with_output(ModelOperation::ChatTurn, "plain answer"),
        );
        let runtime = ChatRuntime::with_model_provider(&workspace, provider).unwrap();
        let result = runtime
            .start_turn(ChatTurnRequest {
                container_id: "container_1".to_string(),
                chat_thread_id: None,
                message: "hello".to_string(),
                context_pack: None,
                source_guidance: None,
                model_config_override: None,
            })
            .unwrap();

        assert_eq!(result.status, ChatTurnStatus::Answered);
        assert_eq!(result.assistant_content.as_deref(), Some("plain answer"));
        assert!(result
            .events
            .iter()
            .any(|event| event.event_type == "chat_assistant_answered"));
        let event_types = result
            .events
            .iter()
            .map(|event| event.event_type.as_str())
            .collect::<Vec<_>>();
        assert!(event_types.contains(&"chat_user_message_recorded"));
        assert!(event_types.contains(&"chat_context_window_checked"));
        assert!(event_types.contains(&"chat_model_call_started"));
        assert!(event_types.contains(&"chat_provider_transcript_updated"));

        let chat_thread_truth = ProcessTruthStore::new(&workspace, &result.chat_thread_id).unwrap();
        assert!(chat_thread_truth.read_events().unwrap().is_empty());
        assert!(chat_thread_truth
            .registry_snapshot()
            .unwrap()
            .jobs
            .is_empty());
        let chat_truth = ChatTruthStore::new(&workspace).unwrap();
        let transcripts = chat_truth
            .list_provider_transcripts(&result.chat_thread_id)
            .unwrap();
        assert_eq!(transcripts.len(), 1);
        assert_eq!(transcripts[0].chat_thread_id, result.chat_thread_id);
        assert!(transcripts[0].messages_ref.starts_with("blob://"));
        assert!(CHAT_RUNTIME_MUTATION_FORBIDDEN_POLICY.contains("read-only"));
    }

    #[test]
    fn chat_runtime_provider_transcript_records_user_turns() {
        let workspace = temp_workspace("provider_user_transcript");
        let provider = Arc::new(
            DeterministicModelProvider::new("deterministic", "chat-model")
                .with_output(ModelOperation::ChatTurn, "plain answer"),
        );
        let runtime = ChatRuntime::with_model_provider(&workspace, provider).unwrap();
        let first = runtime
            .start_turn(ChatTurnRequest {
                container_id: "container_1".to_string(),
                chat_thread_id: None,
                message: "first turn".to_string(),
                context_pack: None,
                source_guidance: None,
                model_config_override: None,
            })
            .unwrap();
        let second = runtime
            .start_turn(ChatTurnRequest {
                container_id: "container_1".to_string(),
                chat_thread_id: Some(first.chat_thread_id.clone()),
                message: "second turn".to_string(),
                context_pack: None,
                source_guidance: None,
                model_config_override: None,
            })
            .unwrap();

        assert_eq!(second.chat_thread_id, first.chat_thread_id);
        let chat_truth = ChatTruthStore::new(&workspace).unwrap();
        let transcripts = chat_truth
            .list_provider_transcripts(&first.chat_thread_id)
            .unwrap();
        assert_eq!(transcripts.len(), 1);
        let model_truth = ProcessTruthStore::new(
            &workspace,
            &chat_model_syscall_truth_id(&first.chat_thread_id),
        )
        .unwrap();
        let messages_path = model_truth
            .resolve_blob_ref(&transcripts[0].messages_ref)
            .unwrap();
        let messages: Vec<ProviderTranscriptMessage> =
            serde_json::from_slice(&std::fs::read(messages_path).unwrap()).unwrap();
        let roles = messages
            .iter()
            .map(|message| message.role.as_str())
            .collect::<Vec<_>>();

        assert_eq!(roles, vec!["user", "assistant", "user", "assistant"]);
        assert!(messages[0]
            .content
            .as_deref()
            .unwrap()
            .contains("first turn"));
        assert!(messages[2]
            .content
            .as_deref()
            .unwrap()
            .contains("second turn"));
    }

    #[test]
    fn chat_runtime_readonly_tool_then_answer_records_chat_truth() {
        let workspace = temp_workspace("readonly_tool");
        std::fs::write(workspace.join("README.md"), "hello from workspace").unwrap();
        let provider = Arc::new(SequencedChatProvider::new(vec![
            provider_tool_response("call_read", "os.read_file", json!({"path": "README.md"})),
            text_response("grounded answer"),
        ]));
        let runtime = ChatRuntime::with_model_provider(&workspace, provider).unwrap();
        let result = runtime
            .start_turn(ChatTurnRequest {
                container_id: "container_1".to_string(),
                chat_thread_id: None,
                message: "read the file".to_string(),
                context_pack: None,
                source_guidance: None,
                model_config_override: None,
            })
            .unwrap();

        assert_eq!(result.status, ChatTurnStatus::Answered);
        assert_eq!(result.assistant_content.as_deref(), Some("grounded answer"));
        assert!(result
            .events
            .iter()
            .any(|event| event.event_type == "chat_readonly_tool_executed"));
        assert!(result
            .events
            .iter()
            .any(|event| event.event_type == "chat_readonly_capability_receipt"));
        assert!(result
            .events
            .iter()
            .any(|event| event.event_type == "chat_provider_tool_call_decoded"));
        assert!(result
            .events
            .iter()
            .any(|event| event.event_type == "chat_provider_tool_result_appended"));
        assert_eq!(
            std::fs::read_to_string(workspace.join("README.md")).unwrap(),
            "hello from workspace"
        );
        let chat_thread_truth = ProcessTruthStore::new(&workspace, &result.chat_thread_id).unwrap();
        assert!(chat_thread_truth.read_events().unwrap().is_empty());
        let process_truth_events = ProcessTruthStore::new(
            &workspace,
            &chat_model_syscall_truth_id(&result.chat_thread_id),
        )
        .unwrap()
        .read_events()
        .unwrap();
        assert!(!process_truth_events.iter().any(|event| {
            event.event_type == "capability_receipt"
                && event.data.get("capability_id").and_then(Value::as_str) == Some("os.read_file")
        }));
    }

    #[test]
    fn chat_runtime_readonly_tool_error_still_appends_provider_tool_result() {
        let workspace = temp_workspace("readonly_tool_error");
        let provider = Arc::new(SequencedChatProvider::new(vec![
            provider_tool_response(
                "call_bad_dataset",
                "dataset.read_page",
                json!({"dataset_ref": "blob://missing/dataset.json"}),
            ),
            text_response("recovered answer"),
        ]));
        let runtime = ChatRuntime::with_model_provider(&workspace, provider).unwrap();
        let result = runtime
            .start_turn(ChatTurnRequest {
                container_id: "container_1".to_string(),
                chat_thread_id: None,
                message: "read the dataset".to_string(),
                context_pack: None,
                source_guidance: None,
                model_config_override: None,
            })
            .unwrap();

        assert_eq!(result.status, ChatTurnStatus::Answered);
        assert_eq!(
            result.assistant_content.as_deref(),
            Some("recovered answer")
        );
        assert!(result.events.iter().any(|event| {
            event.event_type == "chat_readonly_capability_receipt"
                && event.payload.get("receipt_status").and_then(Value::as_str) == Some("failed")
        }));
        assert!(result
            .events
            .iter()
            .any(|event| event.event_type == "chat_provider_tool_result_appended"));
    }

    #[test]
    fn chat_runtime_mid_batch_read_budget_answers_remaining_tool_calls() {
        let workspace = temp_workspace("readonly_mid_batch_budget");
        std::fs::write(workspace.join("large.md"), "x".repeat(512)).unwrap();
        std::fs::write(workspace.join("second.md"), "second").unwrap();
        std::fs::write(workspace.join("third.md"), "third").unwrap();
        let provider = Arc::new(SequencedChatProvider::new(vec![
            provider_tool_response_many(vec![
                (
                    "call_read_large",
                    "os.read_file",
                    json!({"path": "large.md"}),
                ),
                (
                    "call_read_second",
                    "os.read_file",
                    json!({"path": "second.md"}),
                ),
                (
                    "call_read_third",
                    "os.read_file",
                    json!({"path": "third.md"}),
                ),
            ]),
            text_response("recovered after skipped tool results"),
        ]));
        let mut config = ModelInvocationConfig::default();
        config.tool_calling.max_chat_read_bytes_per_turn = 1;
        config.tool_calling.max_chat_read_tokens_per_turn = 1_000_000;
        let runtime = ChatRuntime::with_model_provider(&workspace, provider)
            .unwrap()
            .with_model_config(config);
        let result = runtime
            .start_turn(ChatTurnRequest {
                container_id: "container_1".to_string(),
                chat_thread_id: None,
                message: "read the files".to_string(),
                context_pack: None,
                source_guidance: None,
                model_config_override: None,
            })
            .unwrap();

        assert_eq!(result.status, ChatTurnStatus::Answered);
        assert_eq!(
            result.assistant_content.as_deref(),
            Some("recovered after skipped tool results")
        );
        let budget_event = result
            .events
            .iter()
            .find(|event| event.event_type == "chat_readonly_budget_exceeded")
            .expect("read budget event should be recorded");
        assert_eq!(
            budget_event.payload["payload"]["remaining_skipped_tool_call_count"].as_u64(),
            Some(2)
        );
        assert_eq!(
            budget_event.payload["payload"]["tool_result_completeness"].as_str(),
            Some("remaining_provider_tool_calls_answered_with_skipped_results")
        );
        let skipped_results = result
            .events
            .iter()
            .filter(|event| {
                event.event_type == "chat_provider_tool_result_appended"
                    && event.payload["status"] == "skipped"
            })
            .collect::<Vec<_>>();
        assert_eq!(skipped_results.len(), 2);

        let model_truth = ProcessTruthStore::new(
            &workspace,
            &chat_model_syscall_truth_id(&result.chat_thread_id),
        )
        .unwrap();
        let transcript = replay_provider_transcript_state(
            &model_truth,
            "deterministic",
            "model_provider_transcript",
        )
        .unwrap()
        .expect("provider transcript should be replayable");
        let messages = read_provider_messages(&model_truth, &transcript).unwrap();
        let validation =
            ProviderTranscriptProtocolValidator::validate_deepseek_native_messages(&messages)
                .unwrap();
        assert!(
            validation.valid,
            "provider transcript should be complete before the next request: {:?}",
            validation
        );
        let tool_message_ids = messages
            .iter()
            .filter(|message| message.role == "tool")
            .filter_map(|message| message.tool_call_id.as_deref())
            .collect::<Vec<_>>();
        assert_eq!(
            tool_message_ids,
            vec!["call_read_large", "call_read_second", "call_read_third"]
        );
    }

    #[test]
    fn chat_runtime_tool_loop_limit_recovers_without_failed_or_blocked_turn() {
        let workspace = temp_workspace("readonly_tool_loop_recovery");
        std::fs::write(workspace.join("README.md"), "loop input").unwrap();
        let provider = Arc::new(SequencedChatProvider::new(vec![provider_tool_response(
            "call_read",
            "os.read_file",
            json!({"path": "README.md"}),
        )]));
        let mut config = ModelInvocationConfig::default();
        config.tool_calling.max_provider_subturns = 2;
        config.tool_calling.max_tool_calls_per_chat_turn = 8;
        let runtime = ChatRuntime::with_model_provider(&workspace, provider).unwrap();
        let result = runtime
            .start_turn(ChatTurnRequest {
                container_id: "container_1".to_string(),
                chat_thread_id: None,
                message: "read until loop limit".to_string(),
                context_pack: None,
                source_guidance: None,
                model_config_override: Some(config),
            })
            .unwrap();

        assert_eq!(result.status, ChatTurnStatus::Answered);
        assert!(result
            .assistant_content
            .unwrap_or_default()
            .contains("tool-loop limit"));
        assert!(result
            .events
            .iter()
            .any(|event| event.event_type == "chat_tool_loop_recovered_without_model_answer"));
        assert!(!result.events.iter().any(|event| {
            event.event_type == "chat_turn_failed" || event.event_type == "chat_turn_blocked"
        }));
    }

    #[test]
    fn chat_runtime_attaches_reference_source_guidance_as_model_input() {
        let workspace = temp_workspace("source_guidance");
        let provider = Arc::new(
            DeterministicModelProvider::new("deterministic", "chat-model")
                .with_output(ModelOperation::ChatTurn, "guided answer"),
        );
        let runtime = ChatRuntime::with_model_provider(&workspace, provider).unwrap();
        let guidance = SourceGuidance {
            semantics: "model_guidance_only".to_string(),
            materialized_content: false,
            source_scope_enforcement: "none".to_string(),
            selected_sources: vec![ReferenceSourceDirective {
                source_kind: "workspace_file".to_string(),
                ref_id: "workspace://README.md".to_string(),
                label: Some("README.md".to_string()),
                usage: "primary_reference".to_string(),
                include_mode: "reference_only".to_string(),
                selection_source: "composer_at_token".to_string(),
            }],
            user_intent: None,
        };
        let result = runtime
            .start_turn(ChatTurnRequest {
                container_id: "container_1".to_string(),
                chat_thread_id: None,
                message: "use @README".to_string(),
                context_pack: None,
                source_guidance: Some(guidance),
                model_config_override: None,
            })
            .unwrap();

        assert!(result
            .events
            .iter()
            .any(|event| event.event_type == "chat_reference_sources_attached"));
        assert!(result
            .events
            .iter()
            .any(|event| event.event_type == "chat_model_config_bound"));
        let guidance_event = result
            .events
            .iter()
            .find(|event| event.event_type == "chat_reference_sources_attached")
            .expect("chat reference guidance event recorded");
        let guidance_ref = guidance_event.payload["guidance_ref"]
            .as_str()
            .expect("guidance ref recorded");
        let model_truth = ProcessTruthStore::new(
            &workspace,
            &chat_model_syscall_truth_id(&result.chat_thread_id),
        )
        .unwrap();
        let guidance_text =
            std::fs::read_to_string(model_truth.resolve_blob_ref(guidance_ref).unwrap()).unwrap();
        assert!(guidance_text.contains("model_guidance_only"));
        assert!(guidance_text.contains("workspace://README.md"));
        let model_events = model_truth.read_events().unwrap();
        let bound = model_events
            .iter()
            .find(|event| event.event_type == "model_config_bound")
            .expect("chat model config binding should be recorded in model truth");
        assert_eq!(
            bound.data["effective_config"]["model_id"], "auto",
            "default chat config should be recorded as the active effective route"
        );
        assert!(bound
            .data
            .get("model_invocation_config_ref")
            .and_then(Value::as_str)
            .is_some());
    }

    #[test]
    fn chat_runtime_attaches_context_pack_as_model_input() {
        let workspace = temp_workspace("context_pack_input");
        let provider = Arc::new(
            DeterministicModelProvider::new("deterministic", "chat-model")
                .with_output(ModelOperation::ChatTurn, "context answer"),
        );
        let runtime = ChatRuntime::with_model_provider(&workspace, provider).unwrap();
        let pack = ContextPack {
            context_pack_id: "context_pack_1".to_string(),
            container_id: "container_1".to_string(),
            selected_items: vec![ContextPackItem {
                item_kind: ContextPackItemKind::SourceRef,
                ref_id: "workspace://notes.md".to_string(),
                label: Some("notes.md".to_string()),
                include_mode: ContextPackIncludeMode::Summary,
                priority: 10,
            }],
            excluded_items: Vec::new(),
            auto_policy: ContextPackAutoPolicy::default(),
            summary_ref: Some("blob://container/context-summary".to_string()),
            estimated_tokens: Some(128),
        };
        let result = runtime
            .start_turn(ChatTurnRequest {
                container_id: "container_1".to_string(),
                chat_thread_id: None,
                message: "use configured context".to_string(),
                context_pack: Some(pack),
                source_guidance: None,
                model_config_override: None,
            })
            .unwrap();

        assert_eq!(result.status, ChatTurnStatus::Answered);
        let materialized = result
            .events
            .iter()
            .find(|event| event.event_type == "chat_context_pack_materialized")
            .expect("context pack input should be materialized");
        let context_pack_ref = materialized.payload["context_pack_ref"]
            .as_str()
            .expect("context pack ref recorded");
        let model_truth = ProcessTruthStore::new(
            &workspace,
            &chat_model_syscall_truth_id(&result.chat_thread_id),
        )
        .unwrap();
        let payload =
            std::fs::read_to_string(model_truth.resolve_blob_ref(context_pack_ref).unwrap())
                .unwrap();
        assert!(payload.contains("supernova_context_pack_visible_payload.v1"));
        assert!(payload.contains("workspace://notes.md"));
        let started = model_truth
            .read_events()
            .unwrap()
            .into_iter()
            .find(|event| {
                event.event_type == "model_call_started"
                    && event.data["operation"] == ModelOperation::ChatTurn.as_str()
            })
            .expect("chat model call should be recorded");
        let input_refs = started.data["input_refs"].as_array().unwrap();
        assert!(
            input_refs
                .iter()
                .any(|value| value.as_str() == Some(context_pack_ref)),
            "context pack ref must be passed to the chat model action"
        );
    }

    #[test]
    fn chat_runtime_client_env_runtimes_receipt_is_structured_and_tolerates_missing_tools() {
        let workspace = temp_workspace("client_env_runtimes");
        let provider = Arc::new(SequencedChatProvider::new(vec![
            provider_tool_response(
                "call_runtimes",
                "client_env.scan_runtimes",
                json!({"reason": "Check runtime readiness."}),
            ),
            text_response("runtime readiness checked"),
        ]));
        let runtime = ChatRuntime::with_model_provider(&workspace, provider).unwrap();
        let result = runtime
            .start_turn(ChatTurnRequest {
                container_id: "container_1".to_string(),
                chat_thread_id: None,
                message: "检查本机 Python/Node 是否可用。".to_string(),
                context_pack: None,
                source_guidance: None,
                model_config_override: None,
            })
            .unwrap();

        assert_eq!(result.status, ChatTurnStatus::Answered);
        let receipt_event = result
            .events
            .iter()
            .find(|event| {
                event.event_type == "chat_readonly_capability_receipt"
                    && event.payload["capability_id"] == "client_env.scan_runtimes"
            })
            .expect("client_env.scan_runtimes receipt recorded in ChatTruth");
        assert_eq!(receipt_event.payload["receipt_status"], "success");
        let sections = receipt_event.payload["receipt_data"]["sections"]
            .as_array()
            .unwrap();
        let runtimes = sections
            .iter()
            .find(|section| section["section_id"] == "runtimes")
            .unwrap();
        for runtime_id in ["python", "node", "npm", "rustc", "cargo", "dotnet"] {
            assert!(runtimes["facts"][runtime_id]["available"].is_boolean());
            assert!(runtimes["facts"][runtime_id].get("version").is_some());
        }
        assert!(
            receipt_event.payload["receipt_data"]["sensitive_fields_returned"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn chat_runtime_client_env_sensitive_network_fields_block_without_authorization() {
        let workspace = temp_workspace("client_env_sensitive_block");
        let provider = Arc::new(SequencedChatProvider::new(vec![
            provider_tool_response(
                "call_network",
                "client_env.scan_network",
                json!({
                    "include_sensitive_fields": true,
                    "reason": "User asked for IP/MAC."
                }),
            ),
            text_response("需要显式授权后才能披露本机 IP 或 MAC。"),
        ]));
        let runtime = ChatRuntime::with_model_provider(&workspace, provider).unwrap();
        let result = runtime
            .start_turn(ChatTurnRequest {
                container_id: "container_1".to_string(),
                chat_thread_id: None,
                message: "告诉我本机 IP/MAC。".to_string(),
                context_pack: None,
                source_guidance: None,
                model_config_override: None,
            })
            .unwrap();

        assert_eq!(result.status, ChatTurnStatus::Answered);
        let receipt_event = result
            .events
            .iter()
            .find(|event| {
                event.event_type == "chat_readonly_capability_receipt"
                    && event.payload["capability_id"] == "client_env.scan_network"
            })
            .expect("client_env.scan_network receipt recorded in ChatTruth");
        assert_eq!(receipt_event.payload["receipt_status"], "blocked");
        assert_eq!(
            receipt_event.payload["receipt_data"]["requires_explicit_user_authorization"].as_bool(),
            Some(true)
        );
        assert_eq!(
            receipt_event.payload["receipt_data"]["no_sensitive_values_returned"].as_bool(),
            Some(true)
        );
        let answer = result.assistant_content.unwrap_or_default();
        assert!(!answer.contains("00-"));
        assert!(!answer.contains("00:"));
    }

    #[test]
    fn chat_runtime_mutation_tool_is_surfaced_as_task_without_writing() {
        let workspace = temp_workspace("mutation_tool");
        let provider = Arc::new(SequencedChatProvider::new(vec![provider_tool_response(
            "call_write",
            "os.write_file",
            json!({"path": "MUTATED.txt", "content": "bad"}),
        )]));
        let runtime = ChatRuntime::with_model_provider(&workspace, provider).unwrap();
        let result = runtime
            .start_turn(ChatTurnRequest {
                container_id: "container_1".to_string(),
                chat_thread_id: None,
                message: "write a file".to_string(),
                context_pack: None,
                source_guidance: None,
                model_config_override: None,
            })
            .unwrap();

        assert_eq!(result.status, ChatTurnStatus::NeedsTask);
        assert!(!workspace.join("MUTATED.txt").exists());
        assert!(result.suggested_task.is_some());
        assert!(result
            .events
            .iter()
            .any(|event| event.event_type == "chat_mutation_or_unknown_tool_blocked"));
        assert!(result
            .events
            .iter()
            .any(|event| event.event_type == "chat_needs_task_suggested"));
        assert!(!result
            .events
            .iter()
            .any(|event| event.event_type == "chat_task_suggested"));
    }

    #[test]
    fn chat_runtime_hard_context_compaction_uses_model_operation_before_answer() {
        let workspace = temp_workspace("compact");
        let provider = Arc::new(
            DeterministicModelProvider::new("deterministic", "chat-model")
                .with_output(
                    ModelOperation::CompactChatContext,
                    r#"{"schema":"supernova_chat_context_summary.v1","summary":"compacted chat context","open_questions":[]}"#,
                )
                .with_output(ModelOperation::ChatTurn, "answer after compaction"),
        );
        let mut config = ModelInvocationConfig::default();
        config.context_window.advisory_ratio = 0.000001;
        config.context_window.proactive_compact_ratio = 0.000001;
        config.context_window.hard_compact_ratio = 0.000001;
        config.context_window.emergency_ratio = 0.99;
        config.context_window.reserve_output_tokens = 0;
        config.context_window.reserve_reasoning_tokens = 0;
        let runtime = ChatRuntime::with_model_provider(&workspace, provider)
            .unwrap()
            .with_model_config(config);
        let result = runtime
            .start_turn(ChatTurnRequest {
                container_id: "container_1".to_string(),
                chat_thread_id: None,
                message: "summarize prior context before answering".to_string(),
                context_pack: None,
                source_guidance: None,
                model_config_override: None,
            })
            .unwrap();

        assert_eq!(result.status, ChatTurnStatus::Answered);
        assert_eq!(
            result.assistant_content.as_deref(),
            Some("answer after compaction")
        );
        let event_types = result
            .events
            .iter()
            .map(|event| event.event_type.as_str())
            .collect::<Vec<_>>();
        assert!(event_types.contains(&"context_window_checkpoint_created"));
        assert!(event_types.contains(&"context_window_compaction_model_call_started"));
        assert!(event_types.contains(&"context_window_compaction_model_call_completed"));
        assert!(event_types.contains(&"context_window_visible_context_replaced"));
        assert!(event_types.contains(&"context_window_protocol_validated"));
        assert!(event_types.contains(&"context_window_reestimate_completed"));
    }

    #[derive(Debug)]
    struct SequencedChatProvider {
        responses: Vec<ModelProviderResponse>,
        index: AtomicUsize,
    }

    impl SequencedChatProvider {
        fn new(responses: Vec<ModelProviderResponse>) -> Self {
            Self {
                responses,
                index: AtomicUsize::new(0),
            }
        }
    }

    impl ModelProvider for SequencedChatProvider {
        fn provider_name(&self) -> &str {
            "deterministic"
        }

        fn model_name(&self) -> &str {
            "chat-sequence"
        }

        fn capability_snapshot(&self) -> Value {
            json!({"provider": "deterministic", "supports_tools": true})
        }

        fn invoke(
            &self,
            _request: &ModelProviderRequest,
        ) -> Result<ModelProviderResponse, ModelProviderFailure> {
            let index = self.index.fetch_add(1, Ordering::SeqCst);
            Ok(self
                .responses
                .get(index)
                .cloned()
                .or_else(|| self.responses.last().cloned())
                .unwrap_or_else(|| text_response("fallback answer")))
        }
    }

    fn text_response(text: &str) -> ModelProviderResponse {
        ModelProviderResponse {
            output_text: text.to_string(),
            assistant_message: None,
            reasoning_content: None,
            tool_calls: Vec::new(),
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

    fn provider_tool_response(
        id: &str,
        capability_id: &str,
        arguments: Value,
    ) -> ModelProviderResponse {
        provider_tool_response_many(vec![(id, capability_id, arguments)])
    }

    fn provider_tool_response_many(calls: Vec<(&str, &str, Value)>) -> ModelProviderResponse {
        let tool_calls = calls
            .into_iter()
            .map(|(id, capability_id, arguments)| ProviderToolCall {
                id: id.to_string(),
                r#type: "function".to_string(),
                function: json!({
                    "name": provider_tool_name_for_capability(capability_id),
                    "arguments": arguments,
                }),
            })
            .collect::<Vec<_>>();
        ModelProviderResponse {
            output_text: String::new(),
            assistant_message: None,
            reasoning_content: Some("need a tool".to_string()),
            tool_calls,
            usage: json!({}),
            finish_reason: Some("tool_calls".to_string()),
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
}
