use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::action::{ProcessAction, ProcessActionKind, ProcessActionValidator};
use crate::model_config::ModelInvocationConfig;
use crate::model_runtime::{
    ModelAction, ModelCallReceipt, ModelFailurePolicy, ModelOperation, ModelProvider, ModelRuntime,
    ModelStreamSink, ProviderToolCall,
};
use crate::observation::{ObservationBuilder, TaskObservation};
use crate::provider_debug::{append_provider_native_debug, argument_shape};
use crate::provider_tool::{
    provider_native_tool_calls_enabled, provider_tool_call_name,
    provider_tool_is_mutation_apply_capability, provider_tool_name_for_capability,
    provider_tool_requires_explicit_approval_id, ProviderToolProtocolError, ProviderToolRegistry,
};
use crate::provider_tool_loop_executor::{
    ProviderToolLoopAdapter, ProviderToolLoopExecutor, ProviderToolLoopPolicy,
};
use crate::provider_toolset::{ProviderToolsetPlan, ProviderToolsetPlanner, ProviderToolsetRecord};
use crate::provider_transcript::{
    record_provider_tool_result_with_metadata, record_provider_user_control_message,
    replace_provider_visible_transcript_with_summary, replay_provider_transcript_state,
    ProviderToolResultMetadata,
};
use crate::reasoning::{NextActionDecision, TaskAgentDecisionKind};
use crate::task_context_state::{replay_task_context_state, TaskContextState};
use crate::{
    bind_preview_capability_receipt_to_tx, build_capability_approval_request,
    build_capability_argument_error, default_capability_registry,
    executable_preview_operations_from_scope, expand_preview_target_paths_for_actions,
    finalize_capability_approval, invalid_write_kind_argument_error, is_valid_write_kind, now_ms,
    prepare_capability_approval, safe_blob_name, to_json_value, ArtifactRuntime,
    CapabilityApprovalGuard, CapabilityDescriptor, CapabilityReceipt, CapabilityToken,
    CheckpointRef, ClientEnvRuntime, ClientEnvScanOptions, DataRuntime, ExecutablePreviewOperation,
    OfficeRuntime, OsRuntime, PackageRuntime, ProcessTruthStore, ReadOnlyCapabilityExecutor,
    RuntimeKind, TerminalRuntime, TerminalServiceHealthCheck, WorkspaceGuard,
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TaskAgentRunResult {
    pub job_id: String,
    pub root_pid: String,
    pub runtime_id: String,
    pub status: String,
    pub artifacts: Vec<String>,
    pub checkpoints: Vec<CheckpointRef>,
    pub turn_count: usize,
    pub waiting_for: Option<String>,
    pub last_error: Option<Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompletionEvidence {
    pub completion_statement: String,
    pub claimed_artifacts: Vec<String>,
    pub key_sources: Vec<String>,
    pub known_limitations: Vec<String>,
    pub user_review_notes: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct TaskAgent {
    guard: WorkspaceGuard,
    truth: ProcessTruthStore,
    token: CapabilityToken,
    runtime_id: String,
    registry: Vec<CapabilityDescriptor>,
    model_provider: Option<Arc<dyn ModelProvider>>,
    model_config: ModelInvocationConfig,
    model_invocation_config_ref: Option<String>,
    model_stream_sink: Option<Arc<dyn ModelStreamSink>>,
}

impl ProviderToolLoopAdapter for TaskAgent {
    fn runtime_kind(&self) -> RuntimeKind {
        RuntimeKind::Task
    }

    fn loop_policy(&self) -> ProviderToolLoopPolicy {
        ProviderToolLoopPolicy {
            max_provider_subturns: self.model_config.tool_calling.max_provider_subturns,
            max_tool_calls_per_subturn: self.model_config.tool_calling.max_tool_calls_per_subturn,
            max_tool_calls_total: self.model_config.tool_calling.max_tool_calls_per_task,
            allow_parallel_readonly: self.model_config.tool_calling.allow_parallel_readonly,
            mutation_allowed: true,
        }
    }

    fn provider_tool_calls_are_limit_exempt(&self, tool_calls: &[ProviderToolCall]) -> bool {
        let registry =
            ProviderToolRegistry::phase6_full_coverage(&self.registry, &self.model_config);
        tool_calls.iter().all(|tool_call| {
            provider_tool_call_name(tool_call)
                .ok()
                .and_then(|name| registry.binding_for_tool_name(&name))
                .is_some_and(|binding| binding.read_only_or_process_control)
        })
    }
}

#[derive(Clone, Debug)]
struct TaskProviderToolLoopAdapter {
    model_config: ModelInvocationConfig,
    registry: ProviderToolRegistry,
}

impl TaskProviderToolLoopAdapter {
    fn new(model_config: ModelInvocationConfig, registry: ProviderToolRegistry) -> Self {
        Self {
            model_config,
            registry,
        }
    }
}

impl ProviderToolLoopAdapter for TaskProviderToolLoopAdapter {
    fn runtime_kind(&self) -> RuntimeKind {
        RuntimeKind::Task
    }

    fn loop_policy(&self) -> ProviderToolLoopPolicy {
        ProviderToolLoopPolicy {
            max_provider_subturns: self.model_config.tool_calling.max_provider_subturns,
            max_tool_calls_per_subturn: self.model_config.tool_calling.max_tool_calls_per_subturn,
            max_tool_calls_total: self.model_config.tool_calling.max_tool_calls_per_task,
            allow_parallel_readonly: self.model_config.tool_calling.allow_parallel_readonly,
            mutation_allowed: true,
        }
    }

    fn provider_tool_calls_are_limit_exempt(&self, tool_calls: &[ProviderToolCall]) -> bool {
        tool_calls.iter().all(|tool_call| {
            provider_tool_call_name(tool_call)
                .ok()
                .and_then(|name| self.registry.binding_for_tool_name(&name))
                .is_some_and(|binding| binding.read_only_or_process_control)
        })
    }
}

#[derive(Clone, Debug)]
struct ProviderNativeRecoverableActionError {
    recovery_kind: String,
    protocol_error: ProviderToolProtocolError,
    corrective_message: Value,
    tool_result_extra: Value,
}

struct ResolvedContentArg {
    content: String,
    source_field: &'static str,
}

struct HydratedPreviewContent {
    content: String,
    source_field: &'static str,
    tx_id: String,
    preview_id: String,
}

impl TaskAgent {
    pub fn new_default(
        guard: WorkspaceGuard,
        truth: ProcessTruthStore,
        token: CapabilityToken,
        runtime_id: impl Into<String>,
        model_provider: Option<Arc<dyn ModelProvider>>,
        model_config: ModelInvocationConfig,
        model_invocation_config_ref: Option<String>,
        model_stream_sink: Option<Arc<dyn ModelStreamSink>>,
    ) -> Self {
        Self::new(
            guard,
            truth,
            token,
            runtime_id,
            model_provider,
            model_config,
            model_invocation_config_ref,
            model_stream_sink,
        )
    }

    pub fn new(
        guard: WorkspaceGuard,
        truth: ProcessTruthStore,
        token: CapabilityToken,
        runtime_id: impl Into<String>,
        model_provider: Option<Arc<dyn ModelProvider>>,
        model_config: ModelInvocationConfig,
        model_invocation_config_ref: Option<String>,
        model_stream_sink: Option<Arc<dyn ModelStreamSink>>,
    ) -> Self {
        let mut model_config = model_config;
        model_config.enforce_task_agent_provider_native_tools();
        Self {
            guard,
            truth,
            token,
            runtime_id: runtime_id.into(),
            registry: default_capability_registry(),
            model_provider,
            model_config,
            model_invocation_config_ref,
            model_stream_sink,
        }
    }

    pub fn with_model_stream_sink(
        mut self,
        model_stream_sink: Option<Arc<dyn ModelStreamSink>>,
    ) -> Self {
        self.model_stream_sink = model_stream_sink;
        self
    }

    pub fn runtime_id(&self) -> &str {
        &self.runtime_id
    }

    pub fn task_context_state(&self) -> io::Result<TaskContextState> {
        replay_task_context_state(&self.truth, &self.token.pid, &self.runtime_id)
    }

    pub fn start_or_resume_session(
        &self,
        goal: &str,
        context: &TaskContextState,
    ) -> io::Result<TaskContextState> {
        if context.started {
            self.truth.append_event(
                Some(&self.token.pid),
                "task_agent_session_resumed",
                json!({
                    "runtime_id": self.runtime_id,
                    "session_id": self.runtime_id,
                    "task_agent_session_id": self.runtime_id,
                    "root_pid": self.token.pid,
                    "next_turn_index": context.next_turn_index,
                    "current_turn_index": context.current_turn_index,
                    "resume_strategy": "continue_existing_task_context_state",
                }),
            )?;
        } else {
            self.start_session(goal)?;
        }
        let state = self.task_context_state()?;
        self.record_task_context_state(
            &state,
            if context.started {
                "session_resumed"
            } else {
                "session_started"
            },
        )?;
        Ok(state)
    }

    pub fn start_session(&self, goal: &str) -> io::Result<()> {
        let prompt = crate::agent_prompt::task_agent_system_prompt_for_protocol_and_language(
            &self.registry,
            crate::TaskAgentPromptProtocol::ProviderNativeToolCalls,
            self.model_config.response_language,
        );
        let prompt_ref = self.truth.write_blob(
            &format!(
                "task_agent/{}/system_prompt.txt",
                safe_blob_name(&self.runtime_id)
            ),
            prompt.as_bytes(),
        )?;
        self.truth.append_event(
            Some(&self.token.pid),
            "task_agent_session_started",
            json!({
                "runtime_id": self.runtime_id,
                "session_id": self.runtime_id,
                "task_agent_session_id": self.runtime_id,
                "agent_kind": "task_scoped_temporary_agent",
                "reasoner": "interactive_task_agent_session",
                "checkpoint_strategy": "process_truth_refs_only",
                "goal_ref": self.truth.write_blob(
                    &format!("task_agent/{}/goal.txt", safe_blob_name(&self.runtime_id)),
                    goal.as_bytes(),
                )?,
                "system_prompt_ref": prompt_ref,
            }),
        )?;
        Ok(())
    }

    pub fn start_turn(&self, turn_index: usize) -> io::Result<String> {
        let turn_id = format!("turn_{}_{}", self.runtime_id, turn_index);
        self.truth.append_event(
            Some(&self.token.pid),
            "task_agent_turn_started",
            json!({
                "runtime_id": self.runtime_id,
                "session_id": self.runtime_id,
                "task_agent_session_id": self.runtime_id,
                "turn_id": turn_id,
                "turn_index": turn_index,
            }),
        )?;
        Ok(turn_id)
    }

    pub fn observe_and_checkpoint(
        &self,
        goal: &str,
        turn_id: &str,
    ) -> io::Result<(TaskObservation, CheckpointRef)> {
        let observation = ObservationBuilder::new(self.truth.clone(), self.registry.clone())
            .build(&self.token.pid, &self.runtime_id, goal)?;
        let checkpoint = self.save_checkpoint(
            "task_observation",
            &json!({
                "runtime_id": self.runtime_id,
                "turn_id": turn_id,
                "observation": observation,
            }),
            vec![
                observation.observation_frame_ref.clone(),
                observation.task_context_ref.clone(),
            ],
            vec![observation.goal_ref.clone()],
        )?;
        Ok((observation, checkpoint))
    }

    pub fn checkpoint_after_action(
        &self,
        turn_id: &str,
        observation: &TaskObservation,
        decision: &NextActionDecision,
        status: &str,
    ) -> io::Result<CheckpointRef> {
        self.save_checkpoint(
            "after_action",
            &json!({
                "runtime_id": self.runtime_id,
                "turn_id": turn_id,
                "decision": decision,
                "status": status,
            }),
            vec![observation.observation_frame_ref.clone()],
            Vec::new(),
        )
    }

    pub fn execute_decision(
        &self,
        goal: &str,
        turn_id: &str,
        _observation: &TaskObservation,
        decision: &NextActionDecision,
    ) -> io::Result<String> {
        match decision.kind {
            TaskAgentDecisionKind::DecideNextAction => Ok("running".to_string()),
            TaskAgentDecisionKind::VerifyArtifact => {
                let path = decision.artifact_path.as_deref().ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidInput, "artifact_path missing")
                })?;
                let receipt = self.verify_artifact(turn_id, path, decision)?;
                self.record_capability_execution(decision, &receipt)?;
                Ok("running".to_string())
            }
            TaskAgentDecisionKind::RunCapability => {
                let receipt = self.run_registered_capability(goal, turn_id, decision)?;
                self.record_capability_execution(decision, &receipt)?;
                if receipt.status == "blocked" && is_hard_runtime_block(&receipt) {
                    self.block_job("RUNTIME_CAPABILITY_BLOCKED", &decision.reason)?;
                    return Ok("blocked".to_string());
                }
                if receipt
                    .data
                    .get("waiting_for_approval")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                {
                    return Ok("waiting_approval".to_string());
                }
                Ok("running".to_string())
            }
            TaskAgentDecisionKind::RequestPreview => self.request_preview(turn_id, decision),
            TaskAgentDecisionKind::Clarify => {
                self.clarify(turn_id, decision)?;
                Ok("waiting_user".to_string())
            }
            TaskAgentDecisionKind::Complete => self.complete(turn_id, decision),
            TaskAgentDecisionKind::Interrupted => {
                self.interrupt_by_model_protocol_error(turn_id, decision)
            }
            TaskAgentDecisionKind::Fail => {
                self.fail(turn_id, decision)?;
                Ok("failed".to_string())
            }
        }
    }

    pub fn record_action_error(
        &self,
        turn_id: &str,
        decision: &NextActionDecision,
        err: &io::Error,
    ) -> io::Result<()> {
        let error_message = err.to_string();
        let argument_error = build_capability_argument_error(
            &self.truth,
            &self.runtime_id,
            &decision.capability_id,
            &decision.output_spec,
            &error_message,
        )?;
        let receipt = CapabilityReceipt {
            capability_id: decision.capability_id.clone(),
            job_id: self.token.job_id.clone(),
            pid: self.token.pid.clone(),
            status: "failed".to_string(),
            data: argument_error.to_receipt_data(&self.runtime_id, turn_id, &decision.decision_id),
        };
        self.truth.append_event(
            Some(&self.token.pid),
            "process_action_failed",
            receipt.data.clone(),
        )?;
        self.truth.append_event(
            Some(&self.token.pid),
            "capability_receipt",
            to_json_value(&receipt)?,
        )?;
        self.record_capability_execution(decision, &receipt)?;
        Ok(())
    }

    pub fn complete_turn(
        &self,
        turn_index: usize,
        turn_id: &str,
        decision_id: &str,
        status: &str,
    ) -> io::Result<()> {
        self.truth.append_event(
            Some(&self.token.pid),
            "task_agent_turn_completed",
            json!({
                "runtime_id": self.runtime_id,
                "session_id": self.runtime_id,
                "task_agent_session_id": self.runtime_id,
                "turn_id": turn_id,
                "turn_index": turn_index,
                "decision_id": decision_id,
                "status": status,
            }),
        )?;
        Ok(())
    }

    pub fn record_task_context_state(
        &self,
        state: &TaskContextState,
        reason: &str,
    ) -> io::Result<String> {
        let mut state_value = state.to_value();
        if let Some(obj) = state_value.as_object_mut() {
            obj.insert("record_reason".to_string(), json!(reason));
        }
        let state_ref = self.truth.write_blob(
            &format!(
                "task_context/{}_turn_{}_{}_{}_state.json",
                safe_blob_name(&self.runtime_id),
                state.current_turn_index,
                safe_blob_name(reason),
                now_ms()
            ),
            &serde_json::to_vec_pretty(&state_value).map_err(crate::json_err)?,
        )?;
        self.truth.append_event(
            Some(&self.token.pid),
            "task_context_state_updated",
            json!({
                "runtime_id": self.runtime_id,
                "session_id": self.runtime_id,
                "task_agent_session_id": self.runtime_id,
                "root_pid": self.token.pid,
                "state_ref": state_ref,
                "reason": reason,
                "current_turn_index": state.current_turn_index,
                "next_turn_index": state.next_turn_index,
                "status": state.status,
                "waiting_for": state.waiting_for,
                "latest_observation_ref": state.latest_observation_ref,
                "last_turn_id": state.last_turn_id,
                "last_decision_id": state.last_decision_id,
                "started": state.started,
            }),
        )?;
        Ok(state_ref)
    }

    pub fn fail_max_steps(&self) -> io::Result<()> {
        self.fail_job(
            "TASK_AGENT_MAX_TURNS",
            "Task Agent Session exceeded max turns.",
        )
    }

    pub fn result(
        &self,
        status: &str,
        checkpoints: Vec<CheckpointRef>,
        turn_count: usize,
        waiting_for: Option<String>,
        last_error: Option<Value>,
    ) -> io::Result<TaskAgentRunResult> {
        let replay = self.truth.replay()?;
        Ok(TaskAgentRunResult {
            job_id: self.token.job_id.clone(),
            root_pid: self.token.pid.clone(),
            runtime_id: self.runtime_id.clone(),
            status: status.to_string(),
            artifacts: replay.artifact_refs,
            checkpoints,
            turn_count,
            waiting_for,
            last_error,
        })
    }

    pub fn provider_native_tool_calls_enabled(&self) -> bool {
        provider_native_tool_calls_enabled(&self.model_config)
    }

    pub fn run_provider_tool_call_loop(
        &self,
        goal: &str,
        turn_id: &str,
        observation: &TaskObservation,
    ) -> io::Result<(NextActionDecision, String)> {
        let mut loop_decision = crate::reasoning::decision(
            TaskAgentDecisionKind::DecideNextAction,
            "model.decide_next_action",
            "DeepSeek provider-native tool call loop requested the next action.",
        );
        loop_decision.input_refs = crate::reasoning::decision_model_input_refs(observation);
        let mut loop_exec = ProviderToolLoopExecutor::from_adapter(self);
        let max_subturns = loop_exec.policy().max_provider_subturns;
        let max_tool_calls_per_subturn = loop_exec.policy().max_tool_calls_per_subturn;
        let max_tool_calls_per_task = loop_exec.policy().max_tool_calls_total;
        let executed_tool_calls_before_loop = self.provider_tool_calls_executed_for_task()?;
        loop_exec.seed_executed_tool_calls(executed_tool_calls_before_loop);
        let mut executed_tool_calls = loop_exec.executed_tool_calls();
        let mut successful_tool_results_this_loop = 0_usize;
        'provider_subturns: for subturn_index in 0..max_subturns {
            let receipt =
                self.invoke_next_action_model(goal, turn_id, observation, &loop_decision)?;
            self.record_model_execution(&loop_decision, &receipt)?;
            if receipt.status != "success" {
                let error_code = receipt
                    .error
                    .as_ref()
                    .map(|err| err.error_code.as_str())
                    .unwrap_or("MODEL_DECIDE_NEXT_ACTION_FAILED");
                let message = receipt
                    .error
                    .as_ref()
                    .map(|err| err.message.as_str())
                    .unwrap_or("model call failed");
                if !matches!(
                    error_code,
                    "DEEPSEEK_RESPONSE_CONTENT_MISSING" | "MODEL_OUTPUT_SCHEMA_INVALID"
                ) {
                    let mut failed = crate::reasoning::decision(
                        TaskAgentDecisionKind::Fail,
                        "process.fail",
                        message,
                    );
                    failed.output_spec = json!({"error_code": error_code});
                    self.fail(turn_id, &failed)?;
                    return Ok((failed, "failed".to_string()));
                }
                return self.provider_tool_protocol_interrupted(
                    turn_id,
                    "MODEL_TOOL_CALL_REQUIRED_BUT_MISSING",
                    &format!(
                        "Provider-native tool decision did not return executable tool_calls: {}",
                        message
                    ),
                    receipt.output_ref.clone(),
                    None,
                );
            }
            if receipt.provider_tool_calls.is_empty() {
                let assistant_content = receipt
                    .output_ref
                    .as_deref()
                    .and_then(|output_ref| self.read_blob_text(output_ref).ok());
                let _ = append_provider_native_debug(
                    &self.truth,
                    "assistant_content_yielded",
                    json!({
                        "model_call_id": receipt.model_call_id.clone(),
                        "turn_id": turn_id,
                        "decision_protocol": "provider_native_tool_calls",
                        "diagnostic": {
                            "message": "Provider-native tool decision returned assistant content without tool_calls; content is absorbed and the task remains running.",
                            "finish_reason": receipt.finish_reason.clone(),
                            "output_ref": receipt.output_ref.clone(),
                            "provider_transcript_ref": receipt.provider_transcript_ref.clone(),
                        }
                    }),
                );
                self.truth.append_event(
                    Some(&self.token.pid),
                    "provider_native_assistant_content_yielded",
                    json!({
                        "runtime_id": self.runtime_id,
                        "turn_id": turn_id,
                        "model_call_id": receipt.model_call_id,
                        "output_ref": receipt.output_ref,
                        "assistant_content": assistant_content,
                        "finish_reason": receipt.finish_reason,
                        "provider_transcript_ref": receipt.provider_transcript_ref,
                        "provider_assistant_message_ref": receipt.provider_assistant_message_ref,
                        "task_status": "running",
                        "closure_allowed": false,
                        "required_closure_tool": "process.complete",
                    }),
                )?;
                continue 'provider_subturns;
            }
            let provider_tool_batch_id = loop_exec.batch_id(&receipt.model_call_id, subturn_index);
            let subturn_adapter = TaskProviderToolLoopAdapter::new(
                self.model_config.clone(),
                self.provider_tool_registry_for_receipt(&receipt),
            );
            let subturn = match loop_exec.begin_subturn(
                &subturn_adapter,
                &receipt.model_call_id,
                subturn_index,
                &receipt.provider_tool_calls,
            ) {
                Ok(subturn) => subturn,
                Err(budget_error) => {
                    let task_budget_kind = if budget_error.budget_kind() == "total_tool_call_limit"
                    {
                        "per_task_tool_call_limit"
                    } else {
                        budget_error.budget_kind()
                    };
                    self.record_provider_tool_loop_budget_exceeded(
                        turn_id,
                        &receipt,
                        &provider_tool_batch_id,
                        task_budget_kind,
                        budget_error.requested_tool_calls,
                        budget_error.limit,
                        budget_error.executed_tool_calls_before,
                    )?;
                    let _ = append_provider_native_debug(
                        &self.truth,
                        "preflight_failed",
                        json!({
                            "model_call_id": receipt.model_call_id.clone(),
                            "turn_id": turn_id,
                            "decision_protocol": "provider_native_tool_calls",
                            "diagnostic": {
                                "error_code": budget_error.error_code(),
                                "budget_kind": task_budget_kind,
                                "requested_tool_calls": budget_error.requested_tool_calls,
                                "limit": budget_error.limit,
                                "executed_tool_calls_before": budget_error.executed_tool_calls_before,
                                "provider_tool_batch_id": provider_tool_batch_id,
                                "all_read_only_or_process_control": budget_error.all_read_only_or_control,
                                "tool_calls": receipt.provider_tool_calls.iter().map(|call| {
                                    json!({
                                        "id": call.id.clone(),
                                        "name": provider_tool_call_name(call).ok(),
                                        "arguments_shape": call.function.get("arguments")
                                            .map(argument_shape)
                                            .unwrap_or_else(|| json!({"type": "missing"})),
                                    })
                                }).collect::<Vec<_>>(),
                            }
                        }),
                    );
                    self.append_provider_tool_error_results_for_batch(
                        &receipt.provider,
                        task_provider_protocol(&receipt.provider),
                        &receipt.provider_tool_calls,
                        &ProviderToolProtocolError {
                            error_code: budget_error.error_code().to_string(),
                            message: budget_error.message.clone(),
                            provider_tool_name: None,
                            provider_tool_call_id: None,
                            capability_id: None,
                        },
                        &provider_tool_batch_id,
                    )?;
                    return self.provider_tool_protocol_interrupted(
                        turn_id,
                        budget_error.error_code(),
                        &budget_error.message,
                        receipt.output_ref.clone(),
                        None,
                    );
                }
            };
            self.truth.append_event(
                Some(&self.token.pid),
                "provider_tool_call_loop_subturn_started",
                json!({
                    "runtime_id": self.runtime_id,
                    "turn_id": turn_id,
                    "subturn": subturn,
                    "subturn_index": subturn_index,
                    "model_call_id": receipt.model_call_id.clone(),
                    "provider_tool_batch_id": provider_tool_batch_id.clone(),
                    "tool_call_count": receipt.provider_tool_calls.len(),
                    "max_tool_calls_per_subturn": max_tool_calls_per_subturn,
                    "max_tool_calls_per_task": max_tool_calls_per_task,
                    "executed_tool_calls_before_subturn": executed_tool_calls,
                    "executed_tool_calls_before_loop": executed_tool_calls_before_loop,
                    "parallel_execution": false,
                    "parallel_readonly_requested": self.model_config.tool_calling.allow_parallel_readonly,
                    "all_read_only_or_process_control": subturn.all_read_only_or_control,
                    "tool_call_count_exempt_from_per_subturn_limit": subturn.all_read_only_or_control,
                    "mutation_parallel_execution_allowed": false,
                    "ordering_basis": "provider_tool_call.index_or_array_order",
                    "serialized_multi_tool_calls": receipt.provider_tool_calls.len() > 1,
                }),
            )?;
            for (provider_tool_call_index, tool_call) in
                receipt.provider_tool_calls.iter().enumerate()
            {
                let decision = match self.provider_tool_decision(&receipt, tool_call) {
                    Ok(decision) => decision,
                    Err(error) => {
                        self.record_provider_tool_protocol_error(
                            turn_id,
                            Some(&receipt),
                            Some(tool_call),
                            Some(&provider_tool_batch_id),
                            Some(provider_tool_call_index),
                            &error,
                        )?;
                        self.append_provider_tool_error_result(
                            &receipt.provider,
                            task_provider_protocol(&receipt.provider),
                            tool_call,
                            &error,
                            Some(&provider_tool_batch_id),
                            Some(provider_tool_call_index),
                        )?;
                        let _ = append_provider_native_debug(
                            &self.truth,
                            "preflight_failed",
                            json!({
                                "model_call_id": receipt.model_call_id.clone(),
                                "turn_id": turn_id,
                                "provider_tool_call_id": tool_call.id.clone(),
                                "provider_tool_name": provider_tool_call_name(tool_call).ok(),
                                "provider_tool_batch_id": provider_tool_batch_id.clone(),
                                "provider_tool_call_index": provider_tool_call_index,
                                "decision_protocol": "provider_native_tool_calls",
                                "arguments_shape": tool_call.function.get("arguments")
                                    .map(argument_shape)
                                    .unwrap_or_else(|| json!({"type": "missing"})),
                                "diagnostic": {
                                    "error_code": error.error_code.clone(),
                                    "message": error.message.clone(),
                                    "capability_id": error.capability_id.clone(),
                                    "provider_tool_name": error.provider_tool_name.clone(),
                                    "tool_error_result_appended": true,
                                }
                            }),
                        );
                        self.truth.append_event(
                            Some(&self.token.pid),
                            "provider_tool_call_recoverable_error",
                            json!({
                                "runtime_id": self.runtime_id,
                                "turn_id": turn_id,
                                "model_call_id": receipt.model_call_id,
                                "provider_tool_batch_id": provider_tool_batch_id,
                                "provider_tool_call_id": tool_call.id,
                                "provider_tool_call_index": provider_tool_call_index,
                                "provider_tool_name": provider_tool_call_name(tool_call).ok(),
                                "error_code": error.error_code,
                                "message": error.message,
                                "provider_transcript_appended": true,
                                "next_model_request_should_self_correct": true,
                            }),
                        )?;
                        continue;
                    }
                };
                self.truth.append_event(
                    Some(&self.token.pid),
                    "provider_tool_call_decoded",
                    json!({
                        "runtime_id": self.runtime_id,
                        "turn_id": turn_id,
                        "model_call_id": receipt.model_call_id,
                        "provider_tool_batch_id": provider_tool_batch_id,
                        "provider_tool_call_id": tool_call.id,
                        "provider_tool_call_index": provider_tool_call_index,
                        "decision": decision,
                    }),
                )?;
                let _ = append_provider_native_debug(
                    &self.truth,
                    "tool_decoded",
                    json!({
                        "model_call_id": receipt.model_call_id.clone(),
                        "turn_id": turn_id,
                        "provider_tool_call_id": tool_call.id.clone(),
                        "provider_tool_name": provider_tool_call_name(tool_call).ok(),
                        "provider_tool_batch_id": provider_tool_batch_id.clone(),
                        "provider_tool_call_index": provider_tool_call_index,
                        "capability_id": decision.capability_id.clone(),
                        "arguments_shape": argument_shape(&decision.output_spec),
                        "decision_protocol": "provider_native_tool_calls",
                        "diagnostic": {
                            "decision_kind": format!("{:?}", decision.kind),
                            "reason": decision.reason.clone(),
                            "requires_explicit_approval_id": provider_tool_requires_explicit_approval_id(&decision.capability_id),
                            "has_approval_id": approval_id_arg(&decision).is_some(),
                        }
                    }),
                );
                if let Some(recovery) = self.provider_native_preview_operation_scope_error(
                    &decision,
                    provider_tool_call_name(tool_call).ok(),
                    Some(tool_call.id.clone()),
                )? {
                    let error = recovery.protocol_error.clone();
                    self.record_provider_tool_protocol_error(
                        turn_id,
                        Some(&receipt),
                        Some(tool_call),
                        Some(&provider_tool_batch_id),
                        Some(provider_tool_call_index),
                        &error,
                    )?;
                    self.append_provider_tool_recoverable_error_result(
                        &receipt.provider,
                        task_provider_protocol(&receipt.provider),
                        tool_call,
                        &recovery,
                        Some(&provider_tool_batch_id),
                        Some(provider_tool_call_index),
                    )?;
                    loop_exec.mark_executed(1);
                    executed_tool_calls = loop_exec.executed_tool_calls();
                    self.truth.append_event(
                        Some(&self.token.pid),
                        "provider_tool_call_recoverable_error",
                        json!({
                            "runtime_id": self.runtime_id,
                            "turn_id": turn_id,
                            "model_call_id": receipt.model_call_id,
                            "provider_tool_batch_id": provider_tool_batch_id,
                            "provider_tool_call_id": tool_call.id,
                            "provider_tool_call_index": provider_tool_call_index,
                            "provider_tool_name": provider_tool_call_name(tool_call).ok(),
                            "capability_id": decision.capability_id,
                            "error_code": error.error_code,
                            "message": error.message,
                            "recovery_kind": recovery.recovery_kind,
                            "provider_transcript_appended": true,
                            "corrective_control_message_appended": true,
                            "next_model_request_should_self_correct": true,
                        }),
                    )?;
                    self.truth.append_event(
                        Some(&self.token.pid),
                        "provider_tool_call_loop_tool_completed",
                        json!({
                            "runtime_id": self.runtime_id,
                            "turn_id": turn_id,
                            "model_call_id": receipt.model_call_id,
                            "provider_tool_batch_id": provider_tool_batch_id,
                            "provider_tool_call_id": tool_call.id,
                            "provider_tool_call_index": provider_tool_call_index,
                            "capability_id": decision.capability_id,
                            "status": "recoverable_error",
                            "executed_tool_calls_total": executed_tool_calls,
                            "successful_tool_results_this_loop": successful_tool_results_this_loop,
                            "parallel_execution": false,
                        }),
                    )?;
                    let remaining_tool_calls = receipt
                        .provider_tool_calls
                        .iter()
                        .enumerate()
                        .skip(provider_tool_call_index.saturating_add(1))
                        .map(|(index, pending)| {
                            json!({
                                "provider_tool_call_index": index,
                                "provider_tool_call_id": pending.id.clone(),
                                "provider_tool_name": provider_tool_call_name(pending).ok(),
                            })
                        })
                        .collect::<Vec<_>>();
                    if !remaining_tool_calls.is_empty() {
                        self.append_provider_tool_error_results_for_remaining(
                            &receipt.provider,
                            task_provider_protocol(&receipt.provider),
                            &receipt.provider_tool_calls,
                            provider_tool_call_index.saturating_add(1),
                            &ProviderToolProtocolError {
                                error_code:
                                    "PROVIDER_TOOL_CALL_SKIPPED_DUE_TO_RECOVERABLE_ERROR"
                                        .to_string(),
                                message: "A previous provider tool_call in the same assistant message returned a recoverable validation error; remaining tool_calls were not executed."
                                    .to_string(),
                                provider_tool_name: None,
                                provider_tool_call_id: None,
                                capability_id: None,
                            },
                            &provider_tool_batch_id,
                        )?;
                    }
                    let control_record = record_provider_user_control_message(
                        &self.truth,
                        &self.token.pid,
                        &receipt.provider,
                        task_provider_protocol(&receipt.provider),
                        "recoverable_tool_error_correction",
                        &recovery.corrective_message,
                    )?;
                    let _ = append_provider_native_debug(
                        &self.truth,
                        "preflight_failed",
                        json!({
                            "model_call_id": receipt.model_call_id.clone(),
                            "turn_id": turn_id,
                            "provider_tool_call_id": tool_call.id.clone(),
                            "provider_tool_name": provider_tool_call_name(tool_call).ok(),
                            "provider_tool_batch_id": provider_tool_batch_id.clone(),
                            "provider_tool_call_index": provider_tool_call_index,
                            "capability_id": decision.capability_id.clone(),
                            "arguments_shape": argument_shape(&decision.output_spec),
                            "decision_protocol": "provider_native_tool_calls",
                            "diagnostic": {
                                "error_code": recovery.protocol_error.error_code.clone(),
                                "message": recovery.protocol_error.message.clone(),
                                "recovery_kind": recovery.recovery_kind.clone(),
                                "tool_error_result_appended": true,
                                "corrective_control_message_appended": control_record.is_some(),
                                "remaining_unexecuted_tool_call_count": remaining_tool_calls.len(),
                                "remaining_unexecuted_tool_calls": remaining_tool_calls,
                            }
                        }),
                    );
                    loop_decision = decision;
                    continue 'provider_subturns;
                }
                let (status, tool_result) = match self
                    .execute_provider_tool_decision(goal, turn_id, &decision)
                {
                    Ok(value) => value,
                    Err(err) => {
                        let provider_tool_name = provider_tool_call_name(tool_call).ok();
                        if let Some(recovery) = self.provider_native_recoverable_action_error(
                            &decision,
                            &err,
                            provider_tool_name.clone(),
                            Some(tool_call.id.clone()),
                        )? {
                            let error = recovery.protocol_error.clone();
                            self.record_provider_tool_protocol_error(
                                turn_id,
                                Some(&receipt),
                                Some(tool_call),
                                Some(&provider_tool_batch_id),
                                Some(provider_tool_call_index),
                                &error,
                            )?;
                            self.append_provider_tool_recoverable_error_result(
                                &receipt.provider,
                                task_provider_protocol(&receipt.provider),
                                tool_call,
                                &recovery,
                                Some(&provider_tool_batch_id),
                                Some(provider_tool_call_index),
                            )?;
                            loop_exec.mark_executed(1);
                            executed_tool_calls = loop_exec.executed_tool_calls();
                            self.truth.append_event(
                                Some(&self.token.pid),
                                "provider_tool_call_recoverable_error",
                                json!({
                                    "runtime_id": self.runtime_id,
                                    "turn_id": turn_id,
                                    "model_call_id": receipt.model_call_id,
                                    "provider_tool_batch_id": provider_tool_batch_id,
                                    "provider_tool_call_id": tool_call.id,
                                    "provider_tool_call_index": provider_tool_call_index,
                                    "provider_tool_name": provider_tool_name,
                                    "capability_id": decision.capability_id,
                                    "error_code": error.error_code,
                                    "message": error.message,
                                    "recovery_kind": recovery.recovery_kind,
                                    "provider_transcript_appended": true,
                                    "corrective_control_message_appended": true,
                                    "next_model_request_should_self_correct": true,
                                }),
                            )?;
                            self.truth.append_event(
                                Some(&self.token.pid),
                                "provider_tool_call_loop_tool_completed",
                                json!({
                                    "runtime_id": self.runtime_id,
                                    "turn_id": turn_id,
                                    "model_call_id": receipt.model_call_id,
                                    "provider_tool_batch_id": provider_tool_batch_id,
                                    "provider_tool_call_id": tool_call.id,
                                    "provider_tool_call_index": provider_tool_call_index,
                                    "capability_id": decision.capability_id,
                                    "status": "recoverable_error",
                                    "executed_tool_calls_total": executed_tool_calls,
                                    "successful_tool_results_this_loop": successful_tool_results_this_loop,
                                    "parallel_execution": false,
                                }),
                            )?;
                            let remaining_tool_calls = receipt
                                .provider_tool_calls
                                .iter()
                                .enumerate()
                                .skip(provider_tool_call_index.saturating_add(1))
                                .map(|(index, pending)| {
                                    json!({
                                        "provider_tool_call_index": index,
                                        "provider_tool_call_id": pending.id.clone(),
                                        "provider_tool_name": provider_tool_call_name(pending).ok(),
                                    })
                                })
                                .collect::<Vec<_>>();
                            if !remaining_tool_calls.is_empty() {
                                self.append_provider_tool_error_results_for_remaining(
                                    &receipt.provider,
                                    task_provider_protocol(&receipt.provider),
                                    &receipt.provider_tool_calls,
                                    provider_tool_call_index.saturating_add(1),
                                    &ProviderToolProtocolError {
                                        error_code:
                                            "PROVIDER_TOOL_CALL_SKIPPED_DUE_TO_RECOVERABLE_ERROR"
                                                .to_string(),
                                        message: "A previous provider tool_call in the same assistant message returned a recoverable validation error; remaining tool_calls were not executed."
                                            .to_string(),
                                        provider_tool_name: None,
                                        provider_tool_call_id: None,
                                        capability_id: None,
                                    },
                                    &provider_tool_batch_id,
                                )?;
                            }
                            let control_record = record_provider_user_control_message(
                                &self.truth,
                                &self.token.pid,
                                &receipt.provider,
                                task_provider_protocol(&receipt.provider),
                                "recoverable_tool_error_correction",
                                &recovery.corrective_message,
                            )?;
                            let _ = append_provider_native_debug(
                                &self.truth,
                                "preflight_failed",
                                json!({
                                    "model_call_id": receipt.model_call_id.clone(),
                                    "turn_id": turn_id,
                                    "provider_tool_call_id": tool_call.id.clone(),
                                    "provider_tool_name": provider_tool_call_name(tool_call).ok(),
                                    "provider_tool_batch_id": provider_tool_batch_id.clone(),
                                    "provider_tool_call_index": provider_tool_call_index,
                                    "capability_id": decision.capability_id.clone(),
                                    "arguments_shape": argument_shape(&decision.output_spec),
                                    "decision_protocol": "provider_native_tool_calls",
                                    "diagnostic": {
                                        "error_code": recovery.protocol_error.error_code.clone(),
                                        "message": recovery.protocol_error.message.clone(),
                                        "recovery_kind": recovery.recovery_kind.clone(),
                                        "tool_error_result_appended": true,
                                        "corrective_control_message_appended": control_record.is_some(),
                                        "remaining_unexecuted_tool_call_count": remaining_tool_calls.len(),
                                        "remaining_unexecuted_tool_calls": remaining_tool_calls,
                                    }
                                }),
                            );
                            loop_decision = decision;
                            continue 'provider_subturns;
                        }
                        let error = ProviderToolProtocolError {
                            error_code: "PROVIDER_TOOL_ACTION_VALIDATION_FAILED".to_string(),
                            message: err.to_string(),
                            provider_tool_name,
                            provider_tool_call_id: Some(tool_call.id.clone()),
                            capability_id: Some(decision.capability_id.clone()),
                        };
                        self.record_provider_tool_protocol_error(
                            turn_id,
                            Some(&receipt),
                            Some(tool_call),
                            Some(&provider_tool_batch_id),
                            Some(provider_tool_call_index),
                            &error,
                        )?;
                        self.append_provider_tool_error_result(
                            &receipt.provider,
                            task_provider_protocol(&receipt.provider),
                            tool_call,
                            &error,
                            Some(&provider_tool_batch_id),
                            Some(provider_tool_call_index),
                        )?;
                        let _ = append_provider_native_debug(
                            &self.truth,
                            "preflight_failed",
                            json!({
                                "model_call_id": receipt.model_call_id.clone(),
                                "turn_id": turn_id,
                                "provider_tool_call_id": tool_call.id.clone(),
                                "provider_tool_name": provider_tool_call_name(tool_call).ok(),
                                "provider_tool_batch_id": provider_tool_batch_id.clone(),
                                "provider_tool_call_index": provider_tool_call_index,
                                "capability_id": decision.capability_id.clone(),
                                "arguments_shape": argument_shape(&decision.output_spec),
                                "decision_protocol": "provider_native_tool_calls",
                                "diagnostic": {
                                    "error_code": error.error_code.clone(),
                                    "message": error.message.clone(),
                                    "tool_error_result_appended": true,
                                    "requires_explicit_approval_id": provider_tool_requires_explicit_approval_id(&decision.capability_id),
                                    "has_approval_id": approval_id_arg(&decision).is_some(),
                                }
                            }),
                        );
                        let error_code = error.error_code.clone();
                        let message = error.message.clone();
                        return self.provider_tool_protocol_interrupted(
                            turn_id,
                            &error_code,
                            &message,
                            receipt.output_ref.clone(),
                            Some(error),
                        );
                    }
                };
                if status == "waiting_approval" {
                    self.truth.append_event(
                        Some(&self.token.pid),
                        "provider_tool_call_waiting_approval",
                        json!({
                            "runtime_id": self.runtime_id,
                            "turn_id": turn_id,
                            "model_call_id": receipt.model_call_id.clone(),
                            "provider_tool_batch_id": provider_tool_batch_id.clone(),
                            "provider_tool_call_id": tool_call.id.clone(),
                            "provider_tool_call_index": provider_tool_call_index,
                            "provider_tool_name": provider_tool_call_name(tool_call).ok(),
                            "capability_id": decision.capability_id.clone(),
                            "arguments": decision.output_spec.clone(),
                            "tool_result_preview": tool_result.clone(),
                            "preview_id": tool_result.get("preview_id").cloned(),
                            "preview_tx_id": tool_result.get("preview_tx_id").cloned(),
                            "preview_ref": tool_result.get("preview_ref").cloned(),
                            "target_paths": tool_result.get("target_paths").cloned(),
                            "pending_provider_tool_result": true,
                            "approval_execution_mode": "approve_executes_original_provider_tool_call",
                        }),
                    )?;
                    let _ = append_provider_native_debug(
                        &self.truth,
                        "tool_waiting_approval",
                        json!({
                            "model_call_id": receipt.model_call_id.clone(),
                            "turn_id": turn_id,
                            "provider_tool_call_id": tool_call.id.clone(),
                            "provider_tool_name": provider_tool_call_name(tool_call).ok(),
                            "provider_tool_batch_id": provider_tool_batch_id.clone(),
                            "provider_tool_call_index": provider_tool_call_index,
                            "capability_id": decision.capability_id.clone(),
                            "decision_protocol": "provider_native_tool_calls",
                            "diagnostic": {
                                "status": status.clone(),
                                "preview_id": tool_result.get("preview_id").cloned(),
                                "preview_ref": tool_result.get("preview_ref").cloned(),
                                "pending_provider_tool_result": true,
                            }
                        }),
                    );
                    return Ok((decision, "waiting_approval".to_string()));
                }
                record_provider_tool_result_with_metadata(
                    &self.truth,
                    &self.token.pid,
                    &receipt.provider,
                    task_provider_protocol(&receipt.provider),
                    &tool_call.id,
                    &tool_result,
                    ProviderToolResultMetadata {
                        provider_tool_call_index: Some(provider_tool_call_index),
                        provider_tool_batch_id: Some(provider_tool_batch_id.clone()),
                    },
                )?;
                let _ = append_provider_native_debug(
                    &self.truth,
                    "tool_result_appended",
                    json!({
                        "model_call_id": receipt.model_call_id.clone(),
                        "turn_id": turn_id,
                        "provider_tool_call_id": tool_call.id.clone(),
                        "provider_tool_name": provider_tool_call_name(tool_call).ok(),
                        "provider_tool_batch_id": provider_tool_batch_id.clone(),
                        "provider_tool_call_index": provider_tool_call_index,
                        "capability_id": decision.capability_id.clone(),
                        "decision_protocol": "provider_native_tool_calls",
                        "diagnostic": {
                            "status": status.clone(),
                            "receipt_status": tool_result
                                .get("receipt_status")
                                .or_else(|| tool_result.get("status"))
                                .cloned(),
                            "receipt_ref": tool_result.get("receipt_ref").cloned(),
                            "preview_id": tool_result.get("preview_id").cloned(),
                            "preview_ref": tool_result.get("preview_ref").cloned(),
                            "approval_required": tool_result.get("approval_required").cloned(),
                            "waiting_for_approval": tool_result.get("waiting_for_approval").cloned(),
                        }
                    }),
                );
                loop_exec.mark_executed(1);
                executed_tool_calls = loop_exec.executed_tool_calls();
                if tool_result
                    .get("receipt_status")
                    .or_else(|| tool_result.get("status"))
                    .and_then(Value::as_str)
                    .is_some_and(|value| value == "success")
                {
                    successful_tool_results_this_loop += 1;
                }
                self.truth.append_event(
                    Some(&self.token.pid),
                    "provider_tool_call_loop_tool_completed",
                    json!({
                        "runtime_id": self.runtime_id,
                        "turn_id": turn_id,
                        "model_call_id": receipt.model_call_id,
                        "provider_tool_batch_id": provider_tool_batch_id,
                        "provider_tool_call_id": tool_call.id,
                        "provider_tool_call_index": provider_tool_call_index,
                        "capability_id": decision.capability_id,
                        "status": status,
                        "executed_tool_calls_total": executed_tool_calls,
                        "successful_tool_results_this_loop": successful_tool_results_this_loop,
                        "parallel_execution": false,
                    }),
                )?;
                if status == "recoverable_error" {
                    let remaining_tool_calls = receipt
                        .provider_tool_calls
                        .iter()
                        .enumerate()
                        .skip(provider_tool_call_index.saturating_add(1))
                        .map(|(index, pending)| {
                            json!({
                                "provider_tool_call_index": index,
                                "provider_tool_call_id": pending.id.clone(),
                                "provider_tool_name": provider_tool_call_name(pending).ok(),
                            })
                        })
                        .collect::<Vec<_>>();
                    if !remaining_tool_calls.is_empty() {
                        self.append_provider_tool_error_results_for_remaining(
                            &receipt.provider,
                            task_provider_protocol(&receipt.provider),
                            &receipt.provider_tool_calls,
                            provider_tool_call_index.saturating_add(1),
                            &ProviderToolProtocolError {
                                error_code:
                                    "PROVIDER_TOOL_CALL_SKIPPED_DUE_TO_RECOVERABLE_ERROR"
                                        .to_string(),
                                message: "A previous provider tool_call in the same assistant message returned a recoverable validation error; remaining tool_calls were not executed."
                                    .to_string(),
                                provider_tool_name: None,
                                provider_tool_call_id: None,
                                capability_id: None,
                            },
                            &provider_tool_batch_id,
                        )?;
                    }
                    let corrective_control_message_appended =
                        if let Some(message) = tool_result.get("corrective_control_message") {
                            record_provider_user_control_message(
                                &self.truth,
                                &self.token.pid,
                                &receipt.provider,
                                task_provider_protocol(&receipt.provider),
                                "recoverable_tool_error_correction",
                                message,
                            )?
                            .is_some()
                        } else {
                            false
                        };
                    self.truth.append_event(
                        Some(&self.token.pid),
                        "provider_tool_call_recoverable_error",
                        json!({
                            "runtime_id": self.runtime_id,
                            "turn_id": turn_id,
                            "model_call_id": receipt.model_call_id.clone(),
                            "provider_tool_batch_id": provider_tool_batch_id.clone(),
                            "provider_tool_call_id": tool_call.id.clone(),
                            "provider_tool_call_index": provider_tool_call_index,
                            "provider_tool_name": provider_tool_call_name(tool_call).ok(),
                            "capability_id": decision.capability_id.clone(),
                            "error_code": tool_result.get("error_code").cloned(),
                            "message": tool_result.get("corrective_instruction").cloned(),
                            "recovery_kind": tool_result.get("recovery_kind").cloned(),
                            "provider_transcript_appended": true,
                            "corrective_control_message_appended": corrective_control_message_appended,
                            "next_model_request_should_self_correct": true,
                        }),
                    )?;
                    let _ = append_provider_native_debug(
                        &self.truth,
                        "preflight_failed",
                        json!({
                            "model_call_id": receipt.model_call_id.clone(),
                            "turn_id": turn_id,
                            "provider_tool_call_id": tool_call.id.clone(),
                            "provider_tool_name": provider_tool_call_name(tool_call).ok(),
                            "provider_tool_batch_id": provider_tool_batch_id.clone(),
                            "provider_tool_call_index": provider_tool_call_index,
                            "capability_id": decision.capability_id.clone(),
                            "arguments_shape": argument_shape(&decision.output_spec),
                            "decision_protocol": "provider_native_tool_calls",
                            "diagnostic": {
                                "error_code": tool_result.get("error_code").cloned(),
                                "recovery_kind": tool_result.get("recovery_kind").cloned(),
                                "tool_error_result_appended": true,
                                "corrective_control_message_appended": corrective_control_message_appended,
                                "remaining_unexecuted_tool_call_count": remaining_tool_calls.len(),
                                "remaining_unexecuted_tool_calls": remaining_tool_calls,
                            }
                        }),
                    );
                    loop_decision = decision;
                    continue 'provider_subturns;
                }
                if matches!(
                    status.as_str(),
                    "completed"
                        | "failed"
                        | "blocked"
                        | "interrupted"
                        | "waiting_user"
                        | "waiting_approval"
                ) {
                    let remaining_tool_calls = receipt
                        .provider_tool_calls
                        .iter()
                        .enumerate()
                        .skip(provider_tool_call_index.saturating_add(1))
                        .map(|(index, pending)| {
                            json!({
                                "provider_tool_call_index": index,
                                "provider_tool_call_id": pending.id.clone(),
                                "provider_tool_name": provider_tool_call_name(pending).ok(),
                            })
                        })
                        .collect::<Vec<_>>();
                    if !remaining_tool_calls.is_empty() {
                        self.append_provider_tool_error_results_for_remaining(
                            &receipt.provider,
                            task_provider_protocol(&receipt.provider),
                            &receipt.provider_tool_calls,
                            provider_tool_call_index.saturating_add(1),
                            &ProviderToolProtocolError {
                                error_code:
                                    "PROVIDER_TOOL_CALL_SKIPPED_DUE_TO_TERMINAL_STATUS"
                                        .to_string(),
                                message: format!(
                                    "A previous provider tool_call in the same assistant message returned terminal status `{status}`; remaining tool_calls were not executed."
                                ),
                                provider_tool_name: None,
                                provider_tool_call_id: None,
                                capability_id: None,
                            },
                            &provider_tool_batch_id,
                        )?;
                    }
                    let _ = append_provider_native_debug(
                        &self.truth,
                        "transcript_check",
                        json!({
                            "model_call_id": receipt.model_call_id.clone(),
                            "turn_id": turn_id,
                            "provider_tool_call_id": tool_call.id.clone(),
                            "provider_tool_name": provider_tool_call_name(tool_call).ok(),
                            "provider_tool_batch_id": provider_tool_batch_id.clone(),
                            "provider_tool_call_index": provider_tool_call_index,
                            "capability_id": decision.capability_id.clone(),
                            "decision_protocol": "provider_native_tool_calls",
                            "diagnostic": {
                                "terminal_status": status.clone(),
                                "assistant_tool_call_count": receipt.provider_tool_calls.len(),
                                "remaining_unexecuted_tool_call_count": remaining_tool_calls.len(),
                                "remaining_unexecuted_tool_calls": remaining_tool_calls,
                                "next_request_requires_all_tool_calls_answered": true,
                            }
                        }),
                    );
                    return Ok((decision, status));
                }
                loop_decision = decision;
            }
        }
        if successful_tool_results_this_loop > 0 {
            self.truth.append_event(
                Some(&self.token.pid),
                "provider_tool_loop_soft_yield",
                json!({
                    "runtime_id": self.runtime_id,
                    "turn_id": turn_id,
                    "budget_kind": "max_provider_subturns",
                    "max_provider_subturns": max_subturns,
                    "executed_tool_calls_before_loop": executed_tool_calls_before_loop,
                    "executed_tool_calls_total": executed_tool_calls,
                    "successful_tool_results_this_loop": successful_tool_results_this_loop,
                    "status": "running",
                    "checkpoint_expected": true,
                    "provider_transcript_continues": true,
                    "reason": "max_provider_subturns reached after successful provider tool results; yielding to the next TaskAgent turn instead of interrupting the task",
                }),
            )?;
            return Ok((loop_decision, "running".to_string()));
        }
        self.record_provider_tool_loop_budget_exceeded_without_receipt(
            turn_id,
            "max_provider_subturns",
            max_subturns,
            max_subturns,
            executed_tool_calls,
        )?;
        self.provider_tool_protocol_interrupted(
            turn_id,
            "MODEL_TOOL_LOOP_BUDGET_EXCEEDED",
            "Provider-native tool call loop exceeded max_provider_subturns.",
            None,
            None,
        )
    }

    fn provider_tool_calls_executed_for_task(&self) -> io::Result<usize> {
        Ok(self
            .truth
            .read_events()?
            .into_iter()
            .filter(|event| event.event_type == "provider_tool_call_loop_tool_completed")
            .filter(|event| {
                event
                    .data
                    .get("runtime_id")
                    .and_then(Value::as_str)
                    .is_some_and(|value| value == self.runtime_id)
            })
            .count())
    }

    fn invoke_next_action_model(
        &self,
        goal: &str,
        turn_id: &str,
        observation: &TaskObservation,
        decision: &NextActionDecision,
    ) -> io::Result<ModelCallReceipt> {
        let action = self.action(
            turn_id,
            ProcessActionKind::ModelCall,
            "model.decide_next_action",
            decision.input_refs.clone(),
            json!({"operation": "decide_next_action", "protocol": "provider_native_tool_calls"}),
            &decision.reason,
        );
        self.emit_and_validate_action(&action)?;
        let provider = self.model_provider.clone().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "Task Agent has no ModelProvider for next-action decision",
            )
        })?;
        let prompt_protocol = crate::agent_prompt::TaskAgentPromptProtocol::ProviderNativeToolCalls;
        let model_call_id_override =
            Some(format!("mcall_{}_{}", safe_blob_name(turn_id), now_ms()));
        let preplanned_provider_toolset = Some(
            ProviderToolsetPlanner::new(self.registry.clone(), self.model_config.clone())
                .plan_and_record(
                    &self.truth,
                    &self.token.pid,
                    model_call_id_override.as_deref().unwrap_or(turn_id),
                    &ModelOperation::DecideNextAction,
                )
                .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err.message))?,
        );
        let plan = preplanned_provider_toolset
            .as_ref()
            .expect("provider-native task calls always preplan a provider toolset");
        let system_prompt = crate::agent_prompt::task_agent_provider_native_system_prompt_for_language(
            &plan.record.toolset_index_guide,
            &plan.record.request_scoped_tool_guide,
            self.model_config.response_language,
        );
        let user_instruction = crate::agent_prompt::task_agent_decision_instruction_for_protocol(
            goal,
            prompt_protocol,
        );
        let instruction = format!("{system_prompt}\n\n{user_instruction}");
        let instruction_ref = self.truth.write_blob(
            &format!(
                "model_inputs/next_action_instruction_{}.txt",
                stable_content_hash(instruction.as_bytes())
            ),
            instruction.as_bytes(),
        )?;
        let guidance_refs = self.latest_model_guidance_refs()?;
        let mut input_refs = if decision.input_refs.is_empty() {
            vec![
                observation.goal_ref.clone(),
                observation.observation_frame_ref.clone(),
                observation.task_context_ref.clone(),
            ]
        } else {
            decision.input_refs.clone()
        };
        for guidance_ref in guidance_refs {
            if !input_refs.iter().any(|value| value == &guidance_ref) {
                input_refs.push(guidance_ref);
            }
        }
        if let Some((context_ref, context_pack_id, bound_container_id)) =
            self.latest_task_initial_context_binding()?
        {
            if !input_refs.iter().any(|value| value == &context_ref) {
                input_refs.push(context_ref.clone());
                self.truth.append_event(
                    Some(&self.token.pid),
                    "task_context_pack_model_input_attached",
                    json!({
                        "runtime_id": self.runtime_id,
                        "turn_id": turn_id,
                        "context_ref": context_ref,
                        "context_pack_id": context_pack_id,
                        "container_id": bound_container_id,
                        "provider_visible": true,
                        "fact_boundary": "Task context pack is model input only; Kernel receipts remain the execution source of truth.",
                    }),
                )?;
            }
        }
        let operation = ModelOperation::DecideNextAction;
        let context_profile =
            crate::ModelContextProfile::for_provider(provider.as_ref(), &operation);
        let mut model_action = ModelAction {
            action_id: action.action_id.clone(),
            job_id: self.token.job_id.clone(),
            pid: self.token.pid.clone(),
            reasoning_step_id: turn_id.to_string(),
            operation: operation.clone(),
            instruction_ref,
            input_refs,
            preference_snapshot_ref: None,
            output_schema: json!({"protocol": "provider_native_tool_calls"}),
            provider: provider.provider_name().to_string(),
            model: self.effective_model_name(provider.as_ref(), &operation),
            budget: context_profile.budget_for(&operation),
            failure_policy: ModelFailurePolicy::FailClosed,
            required: true,
        };
        self.model_config
            .apply_budget_overrides(&mut model_action.budget);
        context_profile.clamp_budget_to_context_window(&mut model_action.budget);
        let context_preflight =
            self.append_task_context_window_preflight(&model_action, &preplanned_provider_toolset)?;
        if context_preflight.decision.compact_before_send {
            self.compact_task_model_context(
                &mut model_action,
                &context_preflight,
                provider.clone(),
            )?;
        }
        self.truth.append_event(
            Some(&self.token.pid),
            "model_action_emitted",
            to_json_value(&model_action)?,
        )?;
        let receipt = ModelRuntime::new(self.truth.clone(), self.token.clone(), provider)
            .with_model_invocation_config(
                self.model_config.clone(),
                self.model_invocation_config_ref.clone(),
            )
            .with_preplanned_provider_toolset(preplanned_provider_toolset)
            .with_model_call_id_override(model_call_id_override)
            .with_stream_sink(self.model_stream_sink.clone())
            .decide_next_action(model_action)?;
        Ok(receipt)
    }

    fn append_task_context_window_preflight(
        &self,
        model_action: &ModelAction,
        preplanned_provider_toolset: &Option<ProviderToolsetPlan>,
    ) -> io::Result<crate::ContextWindowPreflight> {
        let instruction = self.read_blob_text_for_context(&model_action.instruction_ref)?;
        let mut input_payloads = Vec::new();
        for input_ref in &model_action.input_refs {
            input_payloads.push(self.read_blob_text_for_context(input_ref)?);
        }
        let tool_schema = preplanned_provider_toolset
            .as_ref()
            .map(|plan| {
                json!({
                    "provider_toolset_ref": plan.provider_toolset_ref.clone(),
                    "tools": plan.registry.tools.clone(),
                })
            })
            .unwrap_or(Value::Null);
        let provider_protocol_name = task_provider_protocol(&model_action.provider);
        if let Some(provider_context) = self.provider_transcript_compaction_payload(
            &model_action.provider,
            provider_protocol_name,
        )? {
            input_payloads.push(serde_json::to_string(&provider_context).map_err(crate::json_err)?);
        }
        input_payloads.push(
            serde_json::to_string(&self.task_context_state_compaction_payload()?)
                .map_err(crate::json_err)?,
        );
        let context_window_tokens = crate::context_window_tokens_for_budget(&model_action.budget);
        let parts = crate::ContextWindowRequestParts {
            provider: model_action.provider.clone(),
            model: model_action.model.clone(),
            context_window_tokens,
            system_prompt: instruction,
            input_payloads,
            tool_schema,
            provider_options: json!({
                "operation": model_action.operation.as_str(),
                "model_config": self.model_config.clone(),
            }),
            reserved_output_tokens: Some(model_action.budget.max_output_tokens as u64),
            reserved_reasoning_tokens: Some(
                self.model_config.context_window.reserve_reasoning_tokens,
            ),
            ..crate::ContextWindowRequestParts::default()
        };
        let preflight = crate::ContextWindowController::preflight(
            crate::ContextScope::Task {
                container_id: None,
                job_id: self.token.job_id.clone(),
                process_id: self.token.pid.clone(),
            },
            &self.model_config.context_window,
            &parts,
        )?;
        crate::append_task_context_window_events(&self.truth, &self.token.pid, &preflight, None)?;
        Ok(preflight)
    }

    fn compact_task_model_context(
        &self,
        model_action: &mut ModelAction,
        preflight: &crate::ContextWindowPreflight,
        provider: Arc<dyn ModelProvider>,
    ) -> io::Result<()> {
        let checkpoint_payload = json!({
            "schema": "supernova_task_context_pre_compaction_checkpoint.v1",
            "job_id": self.token.job_id.clone(),
            "pid": self.token.pid.clone(),
            "model_action": model_action,
            "preflight": preflight,
            "task_process_truth_invariant": crate::TASK_PROCESS_TRUTH_NOT_COMPRESSED_INVARIANT,
        });
        let checkpoint_ref = self.truth.write_blob(
            &format!(
                "context_window/checkpoints/{}_{}.json",
                safe_blob_name(&model_action.reasoning_step_id),
                now_ms()
            ),
            &serde_json::to_vec_pretty(&checkpoint_payload).map_err(crate::json_err)?,
        )?;
        self.truth.append_event(
            Some(&self.token.pid),
            "context_window_checkpoint_created",
            json!({
                "schema_version": crate::CONTEXT_WINDOW_EVENT_SCHEMA_VERSION,
                "scope": preflight.scope,
                "checkpoint_ref": checkpoint_ref,
                "estimate": preflight.estimate,
                "decision": preflight.decision,
                "task_process_truth_invariant": crate::TASK_PROCESS_TRUTH_NOT_COMPRESSED_INVARIANT,
            }),
        )?;
        let mut input_summaries = Vec::new();
        for input_ref in &model_action.input_refs {
            let text = self
                .read_blob_text_for_context(input_ref)
                .unwrap_or_default();
            input_summaries.push(json!({
                "input_ref": input_ref,
                "tokens_estimated": crate::estimate_text_tokens_conservative(&text),
                "preview": text.chars().take(6000).collect::<String>(),
            }));
        }
        let provider_name = provider.provider_name().to_string();
        let provider_protocol_name = task_provider_protocol(&provider_name).to_string();
        let task_context_state = self.task_context_state_compaction_payload()?;
        let provider_transcript_context =
            self.provider_transcript_compaction_payload(&provider_name, &provider_protocol_name)?;
        let process_truth_summary = self.process_truth_compaction_summary(80)?;
        let container_context_pack = self.latest_task_initial_context_payload()?;
        let compaction_input = json!({
            "schema": "supernova_task_context_compaction_input.v1",
            "job_id": self.token.job_id.clone(),
            "pid": self.token.pid.clone(),
            "source_model_action_id": model_action.action_id.clone(),
            "current_observation": {
                "input_refs": model_action.input_refs.clone(),
                "latest_observation_ref": task_context_state.get("latest_observation_ref").cloned().unwrap_or(Value::Null),
            },
            "inputs": input_summaries,
            "task_context_state": task_context_state,
            "provider_transcript": provider_transcript_context,
            "process_truth_summary": process_truth_summary,
            "container_context_pack": container_context_pack,
            "live_suffix_policy": {
                "min_live_suffix_turns": self.model_config.context_window.min_live_suffix_turns,
            },
            "target_summary_tokens": self.model_config.context_window.max_summary_tokens,
            "required_output": {
                "schema": "supernova_task_context_summary.v1",
                "summary": "<provider-visible task context summary>",
                "important_decisions": [],
                "artifact_index": [],
                "source_refs": [],
                "task_refs": [],
                "memory_refs": [],
                "known_constraints": [],
                "open_questions": []
            },
            "fact_boundary": crate::TASK_PROCESS_TRUTH_NOT_COMPRESSED_INVARIANT,
        });
        let instruction_ref = self.truth.write_blob(
            &format!(
                "context_window/compactions/{}_instruction_{}.txt",
                safe_blob_name(&model_action.reasoning_step_id),
                now_ms()
            ),
            b"Compact the task-visible model context into strict JSON. Preserve Kernel receipts, typed refs, pending approvals, verification facts, open questions, and live suffix constraints. Do not invent execution facts. Return only JSON matching schema supernova_task_context_summary.v1.",
        )?;
        let input_ref = self.truth.write_blob(
            &format!(
                "context_window/compactions/{}_input_{}.json",
                safe_blob_name(&model_action.reasoning_step_id),
                now_ms()
            ),
            &serde_json::to_vec_pretty(&compaction_input).map_err(crate::json_err)?,
        )?;
        let operation = ModelOperation::CompactTaskContext;
        let budget = crate::ModelContextProfile::for_provider(provider.as_ref(), &operation)
            .budget_for(&operation);
        let compact_action = ModelAction {
            action_id: format!("{}_context_compaction", model_action.action_id),
            job_id: self.token.job_id.clone(),
            pid: self.token.pid.clone(),
            reasoning_step_id: format!("{}_context_compaction", model_action.reasoning_step_id),
            operation: operation.clone(),
            instruction_ref,
            input_refs: vec![input_ref],
            preference_snapshot_ref: None,
            output_schema: crate::task_context_summary_output_schema(),
            provider: provider.provider_name().to_string(),
            model: provider.model_name_for_operation(&operation),
            budget,
            failure_policy: ModelFailurePolicy::FailClosed,
            required: preflight.decision.hard_block_if_compaction_fails,
        };
        let model_call_id = format!(
            "mcall_{}_task_compact_{}",
            safe_blob_name(&model_action.reasoning_step_id),
            now_ms()
        );
        self.truth.append_event(
            Some(&self.token.pid),
            "context_window_compaction_model_call_started",
            json!({
                "schema_version": crate::CONTEXT_WINDOW_EVENT_SCHEMA_VERSION,
                "scope": preflight.scope,
                "model_call_id": model_call_id.clone(),
                "operation": "model.compact_task_context",
                "task_process_truth_invariant": crate::TASK_PROCESS_TRUTH_NOT_COMPRESSED_INVARIANT,
            }),
        )?;
        let receipt = ModelRuntime::new(self.truth.clone(), self.token.clone(), provider)
            .with_model_invocation_config(
                self.model_config.clone(),
                self.model_invocation_config_ref.clone(),
            )
            .with_model_call_id_override(Some(model_call_id.clone()))
            .compact_task_context(compact_action)?;
        self.truth.append_event(
            Some(&self.token.pid),
            "context_window_compaction_model_call_completed",
            json!({
                "schema_version": crate::CONTEXT_WINDOW_EVENT_SCHEMA_VERSION,
                "scope": preflight.scope,
                "model_call_id": receipt.model_call_id.clone(),
                "operation": "model.compact_task_context",
                "status": receipt.status.clone(),
                "output_ref": receipt.output_ref.clone(),
                "schema_validation": receipt.schema_validation.clone(),
                "error": receipt.error.clone(),
                "task_process_truth_invariant": crate::TASK_PROCESS_TRUTH_NOT_COMPRESSED_INVARIANT,
            }),
        )?;
        if receipt.status != "success" {
            self.truth.append_event(
                Some(&self.token.pid),
                "context_window_compaction_failed",
                json!({
                    "schema_version": crate::CONTEXT_WINDOW_EVENT_SCHEMA_VERSION,
                    "scope": preflight.scope,
                    "model_call_id": receipt.model_call_id,
                    "status": receipt.status,
                    "error": receipt.error,
                    "task_process_truth_invariant": crate::TASK_PROCESS_TRUTH_NOT_COMPRESSED_INVARIANT,
                }),
            )?;
            if preflight.decision.hard_block_if_compaction_fails {
                return Err(io::Error::new(
                    io::ErrorKind::Other,
                    "task context compaction failed before a hard-threshold provider request",
                ));
            }
            return Ok(());
        }
        let summary_ref = receipt.output_ref.clone().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "successful task compaction did not produce output_ref",
            )
        })?;
        let summary_text = self.read_blob_text_for_context(&summary_ref)?;
        let summary_json: Value = serde_json::from_str(&summary_text).map_err(crate::json_err)?;
        if let Err(err) = crate::validate_task_context_summary(&summary_json) {
            self.truth.append_event(
                Some(&self.token.pid),
                "context_window_compaction_failed",
                json!({
                    "schema_version": crate::CONTEXT_WINDOW_EVENT_SCHEMA_VERSION,
                    "scope": preflight.scope,
                    "model_call_id": receipt.model_call_id,
                    "status": "failed",
                    "error": {
                        "error_code": "TASK_COMPACTION_SUMMARY_INVALID",
                        "message": err.to_string(),
                    },
                    "task_process_truth_invariant": crate::TASK_PROCESS_TRUTH_NOT_COMPRESSED_INVARIANT,
                }),
            )?;
            if preflight.decision.hard_block_if_compaction_fails {
                return Err(io::Error::new(io::ErrorKind::InvalidData, err));
            }
            return Ok(());
        }
        let mut provider_replacement_ref = None;
        let mut provider_live_suffix_ref = None;
        let mut compacted_until_message_index = None;
        let mut provider_protocol_validation = None;
        if let Some(replacement) = replace_provider_visible_transcript_with_summary(
            &self.truth,
            &self.token.pid,
            &provider_name,
            &provider_protocol_name,
            &summary_text,
            self.model_config.context_window.min_live_suffix_turns,
            "task_context_window_compaction",
        )? {
            provider_replacement_ref = Some(replacement.new_transcript_ref.clone());
            provider_live_suffix_ref = Some(replacement.live_suffix_ref.clone());
            compacted_until_message_index = Some(replacement.compacted_until_message_index);
            let Some(state) = replay_provider_transcript_state(
                &self.truth,
                &provider_name,
                &provider_protocol_name,
            )?
            else {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "provider transcript replacement was recorded but could not be replayed",
                ));
            };
            let replacement_messages = crate::read_provider_messages(&self.truth, &state)?;
            let validation =
                crate::ProviderTranscriptProtocolValidator::validate_deepseek_native_messages(
                    &replacement_messages,
                )?;
            provider_protocol_validation = Some(validation.clone());
            self.truth.append_event(
                Some(&self.token.pid),
                "task_provider_transcript_compacted",
                json!({
                    "schema_version": crate::CONTEXT_WINDOW_EVENT_SCHEMA_VERSION,
                    "scope": preflight.scope,
                    "provider": provider_name,
                    "protocol": provider_protocol_name,
                    "transcript_id": replacement.transcript_id,
                    "old_transcript_ref": replacement.old_transcript_ref,
                    "new_transcript_ref": replacement.new_transcript_ref,
                    "summary_ref": replacement.summary_ref,
                    "live_suffix_ref": replacement.live_suffix_ref,
                    "compacted_until_message_index": replacement.compacted_until_message_index,
                    "message_count": replacement.message_count,
                    "pending_tool_call_count": replacement.pending_tool_call_count,
                    "protocol_validation": validation.clone(),
                    "task_process_truth_invariant": crate::TASK_PROCESS_TRUTH_NOT_COMPRESSED_INVARIANT,
                }),
            )?;
            if !validation.valid {
                self.truth.append_event(
                    Some(&self.token.pid),
                    "context_window_compaction_failed",
                    json!({
                        "schema_version": crate::CONTEXT_WINDOW_EVENT_SCHEMA_VERSION,
                        "scope": preflight.scope,
                        "error": {
                            "error_code": "TASK_PROVIDER_TRANSCRIPT_PROTOCOL_INVALID_AFTER_COMPACTION",
                            "validation": validation,
                        },
                        "task_process_truth_invariant": crate::TASK_PROCESS_TRUTH_NOT_COMPRESSED_INVARIANT,
                    }),
                )?;
                if preflight.decision.hard_block_if_compaction_fails {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "TASK_PROVIDER_TRANSCRIPT_PROTOCOL_INVALID_AFTER_COMPACTION",
                    ));
                }
                return Ok(());
            }
        }
        model_action.input_refs = vec![summary_ref.clone()];
        self.truth.append_event(
            Some(&self.token.pid),
            "context_window_visible_context_replaced",
            json!({
                "schema_version": crate::CONTEXT_WINDOW_EVENT_SCHEMA_VERSION,
                "scope": preflight.scope,
                "summary_ref": summary_ref.clone(),
                "model_call_id": receipt.model_call_id,
                "provider_transcript_ref": provider_replacement_ref.clone(),
                "live_suffix_ref": provider_live_suffix_ref.clone(),
                "compacted_until_message_index": compacted_until_message_index,
                "replacement_kind": "task_model_action_input_refs_and_provider_visible_transcript",
                "task_process_truth_invariant": crate::TASK_PROCESS_TRUTH_NOT_COMPRESSED_INVARIANT,
            }),
        )?;
        self.truth.append_event(
            Some(&self.token.pid),
            "context_window_protocol_validated",
            json!({
                "schema_version": crate::CONTEXT_WINDOW_EVENT_SCHEMA_VERSION,
                "scope": preflight.scope,
                "valid": provider_protocol_validation.as_ref().map(|item| item.valid).unwrap_or(true),
                "validation_kind": "task_provider_visible_transcript",
                "pending_tool_call_ids": provider_protocol_validation.as_ref().map(|item| item.pending_tool_call_ids.clone()).unwrap_or_default(),
                "errors": provider_protocol_validation.as_ref().map(|item| item.errors.clone()).unwrap_or_default(),
            }),
        )?;
        let reestimate_parts = crate::ContextWindowRequestParts {
            provider: model_action.provider.clone(),
            model: model_action.model.clone(),
            context_window_tokens: preflight.estimate.context_window_tokens,
            input_payloads: vec![summary_text],
            reserved_output_tokens: Some(preflight.estimate.reserved_output_tokens),
            reserved_reasoning_tokens: Some(preflight.estimate.reserved_reasoning_tokens),
            ..crate::ContextWindowRequestParts::default()
        };
        let reestimate = crate::ContextWindowController::estimate(
            &self.model_config.context_window,
            &reestimate_parts,
        );
        self.truth.append_event(
            Some(&self.token.pid),
            "context_window_reestimate_completed",
            json!({
                "schema_version": crate::CONTEXT_WINDOW_EVENT_SCHEMA_VERSION,
                "scope": preflight.scope,
                "estimate": reestimate.clone(),
                "task_process_truth_invariant": crate::TASK_PROCESS_TRUTH_NOT_COMPRESSED_INVARIANT,
            }),
        )?;
        if reestimate.usage_ratio >= self.model_config.context_window.emergency_ratio
            || reestimate.estimated_total_tokens > reestimate.context_window_tokens
        {
            let emergency_payload = json!({
                "schema": "supernova_task_context_emergency_trim.v1",
                "job_id": self.token.job_id.clone(),
                "pid": self.token.pid.clone(),
                "summary_ref": summary_ref.clone(),
                "provider_transcript_ref": provider_replacement_ref.clone(),
                "live_suffix_ref": provider_live_suffix_ref.clone(),
                "compacted_until_message_index": compacted_until_message_index,
                "task_context_state": self.task_context_state_compaction_payload()?,
                "container_context_pack": self.latest_task_initial_context_payload()?,
                "instruction": "Continue the same task from this compacted handoff. Resolve facts only through ProcessTruth refs and Kernel receipts; do not treat this summary as execution proof.",
                "fact_boundary": crate::TASK_PROCESS_TRUTH_NOT_COMPRESSED_INVARIANT,
            });
            let emergency_trim_ref = self.truth.write_blob(
                &format!(
                    "context_window/emergency_trim/{}_{}.json",
                    safe_blob_name(&model_action.reasoning_step_id),
                    now_ms()
                ),
                &serde_json::to_vec_pretty(&emergency_payload).map_err(crate::json_err)?,
            )?;
            model_action.input_refs = vec![emergency_trim_ref.clone()];
            let emergency_trim_text = self.read_blob_text_for_context(&emergency_trim_ref)?;
            let emergency_reestimate_parts = crate::ContextWindowRequestParts {
                provider: model_action.provider.clone(),
                model: model_action.model.clone(),
                context_window_tokens: preflight.estimate.context_window_tokens,
                input_payloads: vec![emergency_trim_text],
                reserved_output_tokens: Some(preflight.estimate.reserved_output_tokens),
                reserved_reasoning_tokens: Some(preflight.estimate.reserved_reasoning_tokens),
                ..crate::ContextWindowRequestParts::default()
            };
            let emergency_reestimate = crate::ContextWindowController::estimate(
                &self.model_config.context_window,
                &emergency_reestimate_parts,
            );
            self.truth.append_event(
                Some(&self.token.pid),
                "context_window_emergency_trim_applied",
                json!({
                    "schema_version": crate::CONTEXT_WINDOW_EVENT_SCHEMA_VERSION,
                    "scope": preflight.scope,
                    "estimate_after_compaction": reestimate.clone(),
                    "estimate_after_emergency_trim": emergency_reestimate.clone(),
                    "summary_ref": summary_ref.clone(),
                    "provider_transcript_ref": provider_replacement_ref.clone(),
                    "live_suffix_ref": provider_live_suffix_ref.clone(),
                    "emergency_trim_ref": emergency_trim_ref,
                    "compacted_until_message_index": compacted_until_message_index,
                    "error_code": if emergency_reestimate.estimated_total_tokens > emergency_reestimate.context_window_tokens {
                        "TASK_CONTEXT_WINDOW_EXCEEDED_AFTER_EMERGENCY_TRIM"
                    } else {
                        "TASK_CONTEXT_WINDOW_EMERGENCY_TRIM_APPLIED"
                    },
                    "task_process_truth_invariant": crate::TASK_PROCESS_TRUTH_NOT_COMPRESSED_INVARIANT,
                }),
            )?;
            if emergency_reestimate.estimated_total_tokens
                > emergency_reestimate.context_window_tokens
            {
                return Err(io::Error::new(
                    io::ErrorKind::Other,
                    "TASK_CONTEXT_WINDOW_EXCEEDED_AFTER_EMERGENCY_TRIM",
                ));
            }
        }
        Ok(())
    }

    fn task_context_state_compaction_payload(&self) -> io::Result<Value> {
        let state = replay_task_context_state(&self.truth, &self.token.pid, &self.runtime_id)?;
        Ok(json!({
            "schema": "supernova_task_context_state_snapshot.v1",
            "job_id": state.job_id,
            "root_pid": state.root_pid,
            "task_agent_session_id": state.task_agent_session_id,
            "current_turn_index": state.current_turn_index,
            "next_turn_index": state.next_turn_index,
            "goal_ref": state.goal_ref,
            "system_prompt_ref": state.system_prompt_ref,
            "latest_observation_ref": state.latest_observation_ref,
            "provider_transcript_ref": state.provider_transcript_ref,
            "provider_transcript_summary_ref": state.provider_transcript_summary_ref,
            "working_memory_ref": state.working_memory_ref,
            "pending_user_decision": state.pending_user_decision,
            "preview_tx_table": state.preview_tx_table,
            "approval_token_table": state.approval_token_table,
            "capability_receipt_cursor": state.capability_receipt_cursor,
            "artifact_table": state.artifact_table,
            "verification_table": state.verification_table,
            "closure_state": state.closure_state,
            "last_error_summary": state.last_error_summary,
            "last_model_protocol_error": state.last_model_protocol_error,
            "status": state.status,
            "waiting_for": state.waiting_for,
            "last_turn_id": state.last_turn_id,
            "last_decision_id": state.last_decision_id,
            "fact_boundary": crate::TASK_PROCESS_TRUTH_NOT_COMPRESSED_INVARIANT,
        }))
    }

    fn provider_transcript_compaction_payload(
        &self,
        provider_name: &str,
        provider_protocol_name: &str,
    ) -> io::Result<Option<Value>> {
        let Some(state) =
            replay_provider_transcript_state(&self.truth, provider_name, provider_protocol_name)?
        else {
            return Ok(None);
        };
        let messages = crate::read_provider_messages(&self.truth, &state)?;
        let live_suffix_count = self
            .model_config
            .context_window
            .min_live_suffix_turns
            .saturating_mul(2)
            .saturating_add(4)
            .max(8);
        let start = messages.len().saturating_sub(live_suffix_count);
        let live_suffix = messages.iter().skip(start).cloned().collect::<Vec<_>>();
        Ok(Some(json!({
            "schema": "supernova_provider_transcript_compaction_context.v1",
            "provider": provider_name,
            "protocol": provider_protocol_name,
            "messages_ref": state.messages_ref,
            "summary_ref": state.summary_ref,
            "message_count": messages.len(),
            "live_suffix_start_index": start,
            "live_suffix_message_count": live_suffix.len(),
            "pending_tool_call_count": state.pending_tool_calls.len(),
            "pending_tool_call_ids": state.pending_tool_calls.iter().map(|item| item.provider_tool_call_id.clone()).collect::<Vec<_>>(),
            "reasoning_content_ref_count": state.reasoning_content_refs.len(),
            "live_suffix": live_suffix,
            "protocol_boundary": "assistant tool_calls and matching tool messages in the live suffix must stay paired across compaction",
        })))
    }

    fn process_truth_compaction_summary(&self, recent_limit: usize) -> io::Result<Value> {
        let events = self.truth.read_events()?;
        let start = events.len().saturating_sub(recent_limit);
        let recent_events = events
            .iter()
            .skip(start)
            .map(|event| {
                json!({
                    "event_id": event.event_id,
                    "timestamp_ms": event.timestamp_ms,
                    "pid": event.pid.clone(),
                    "event_type": event.event_type.clone(),
                    "data_preview": compact_json_preview(&event.data, 1800),
                })
            })
            .collect::<Vec<_>>();
        Ok(json!({
            "schema": "supernova_process_truth_compaction_summary.v1",
            "job_id": self.token.job_id.clone(),
            "event_count": events.len(),
            "recent_event_limit": recent_limit,
            "recent_events": recent_events,
            "fact_boundary": crate::TASK_PROCESS_TRUTH_NOT_COMPRESSED_INVARIANT,
        }))
    }

    fn latest_task_initial_context_payload(&self) -> io::Result<Option<Value>> {
        let events = self.truth.read_events()?;
        for event in events.iter().rev() {
            if event.event_type != "task_initial_context_bound" {
                continue;
            }
            let Some(context_ref) = event.data.get("context_ref").and_then(Value::as_str) else {
                continue;
            };
            let context_text = self.read_blob_text_for_context(context_ref)?;
            let context_value = serde_json::from_str::<Value>(&context_text).unwrap_or_else(
                |_| json!({"raw_preview": context_text.chars().take(6000).collect::<String>()}),
            );
            return Ok(Some(json!({
                "schema": "supernova_task_initial_context_pack_snapshot.v1",
                "context_ref": context_ref,
                "context_pack_id": event.data.get("context_pack_id").cloned().unwrap_or(Value::Null),
                "container_id": event.data.get("container_id").cloned().unwrap_or(Value::Null),
                "context": context_value,
                "fact_boundary": event.data.get("fact_boundary").cloned().unwrap_or_else(|| json!("Initial context is provider-visible input only.")),
            })));
        }
        Ok(None)
    }

    fn latest_task_initial_context_binding(
        &self,
    ) -> io::Result<Option<(String, Option<String>, Option<String>)>> {
        let events = self.truth.read_events()?;
        for event in events.iter().rev() {
            if event.event_type != "task_initial_context_bound" {
                continue;
            }
            let Some(context_ref) = event.data.get("context_ref").and_then(Value::as_str) else {
                continue;
            };
            return Ok(Some((
                context_ref.to_string(),
                event
                    .data
                    .get("context_pack_id")
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
                event
                    .data
                    .get("container_id")
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
            )));
        }
        Ok(None)
    }

    fn latest_model_guidance_refs(&self) -> io::Result<Vec<String>> {
        let events = self.truth.read_events()?;
        let mut refs = Vec::new();
        for event_type in [
            "task_reference_sources_attached",
            "task_artifact_destination_guidance_attached",
        ] {
            if let Some(guidance_ref) = events
                .iter()
                .rev()
                .find(|event| event.event_type == event_type)
                .and_then(|event| event.data.get("guidance_ref"))
                .and_then(Value::as_str)
                .map(ToString::to_string)
            {
                refs.push(guidance_ref);
            }
        }
        Ok(refs)
    }

    fn effective_model_name(
        &self,
        provider: &dyn ModelProvider,
        operation: &ModelOperation,
    ) -> String {
        let provider_snapshot = provider.capability_snapshot();
        self.model_config
            .effective_model_for_operation(provider, operation, &provider_snapshot)
    }

    fn read_blob_text_for_context(&self, blob_ref: &str) -> io::Result<String> {
        let path = self.truth.resolve_blob_ref(blob_ref)?;
        let bytes = fs::read(path)?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    fn model_protocol_interrupted_decision(
        &self,
        error_code: &str,
        message: &str,
        model_output_ref: Option<String>,
        already_materialized_artifacts: Vec<String>,
        user_visible_artifact_candidates: Vec<Value>,
    ) -> NextActionDecision {
        let mut interrupted = crate::reasoning::decision(
            TaskAgentDecisionKind::Interrupted,
            "process.interrupt",
            message,
        );
        interrupted.output_spec = json!({
            "error_code": error_code,
            "reason": message,
            "model_output_ref": model_output_ref,
            "already_materialized_artifacts": already_materialized_artifacts,
            "user_visible_artifact_candidates": user_visible_artifact_candidates,
        });
        interrupted
    }

    fn verify_artifact(
        &self,
        reasoning_step_id: &str,
        artifact_path: &str,
        decision: &NextActionDecision,
    ) -> io::Result<CapabilityReceipt> {
        let action = self.action(
            reasoning_step_id,
            ProcessActionKind::Verify,
            "os.verify_artifact",
            vec![format!("artifact://{artifact_path}")],
            json!({"path": artifact_path}),
            &decision.reason,
        );
        self.emit_and_validate_action(&action)?;
        let os = OsRuntime::new(self.guard.clone(), self.truth.clone(), self.token.clone());
        os.verify_artifact(artifact_path)
    }

    fn run_registered_capability(
        &self,
        goal: &str,
        reasoning_step_id: &str,
        decision: &NextActionDecision,
    ) -> io::Result<CapabilityReceipt> {
        let capability_id = decision.capability_id.as_str();
        let action = self.action(
            reasoning_step_id,
            process_action_kind_for_capability(capability_id),
            capability_id,
            decision.input_refs.clone(),
            decision.output_spec.clone(),
            &decision.reason,
        );
        self.emit_and_validate_action(&action)?;

        let descriptor = self.capability_descriptor(capability_id)?;
        let approval_request = build_capability_approval_request(
            &self.truth,
            descriptor,
            &decision.output_spec,
            approval_id_arg(decision),
        )?;
        let approval_guard =
            match prepare_capability_approval(&self.truth, &self.token, approval_request)? {
                Ok(guard) => guard,
                Err(receipt) => {
                    if provider_native_write_intent_capability(capability_id)
                        && receipt
                            .data
                            .get("approval_required")
                            .and_then(Value::as_bool)
                            .unwrap_or(false)
                        && approval_id_arg(decision).is_none()
                    {
                        let preview_receipt = self.provider_native_write_intent_preview_receipt(
                            reasoning_step_id,
                            decision,
                        )?;
                        self.record_kernel_blocked_capability(&preview_receipt)?;
                        return Ok(preview_receipt);
                    }
                    if capability_id == "terminal.run_command"
                        && receipt
                            .data
                            .get("approval_required")
                            .and_then(Value::as_bool)
                            .unwrap_or(false)
                        && approval_id_arg(decision).is_none()
                    {
                        let preview_receipt = self
                            .provider_native_terminal_approval_preview_receipt(
                                reasoning_step_id,
                                decision,
                                &receipt,
                            )?;
                        self.record_kernel_blocked_capability(&preview_receipt)?;
                        return Ok(preview_receipt);
                    }
                    if receipt
                        .data
                        .get("approval_required")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                        && approval_id_arg(decision).is_none()
                    {
                        let preview_receipt = self.provider_native_mutation_auto_preview_receipt(
                            reasoning_step_id,
                            decision,
                        )?;
                        self.record_kernel_blocked_capability(&preview_receipt)?;
                        return Ok(preview_receipt);
                    }
                    self.record_kernel_blocked_capability(&receipt)?;
                    return Ok(receipt);
                }
            };

        let os = || OsRuntime::new(self.guard.clone(), self.truth.clone(), self.token.clone());
        let mut receipt = match capability_id {
            "os.list_tree" => os().list_tree(depth_arg(decision, 8)),
            "os.workspace_inventory" => os().workspace_inventory(depth_arg(decision, 32)),
            "os.stat_path" => os().stat_path(&path_arg(decision, "path")?),
            "os.read_file" => os().read_file(&path_arg(decision, "path")?),
            "os.write_file" => {
                let path = path_arg(decision, "path")?;
                let Some(write_kind) = decision
                    .output_spec
                    .get("write_kind")
                    .and_then(Value::as_str)
                else {
                    let mut argument_error = build_capability_argument_error(
                        &self.truth,
                        &self.runtime_id,
                        "os.write_file",
                        &decision.output_spec,
                        "write_kind missing",
                    )?;
                    argument_error
                        .invalid_fields
                        .retain(|field| field.field == "write_kind");
                    let mut data = argument_error.to_receipt_data(
                        &self.runtime_id,
                        reasoning_step_id,
                        &decision.decision_id,
                    );
                    data["reason"] = json!("missing_write_kind");
                    data["target_path"] = json!(path.replace('\\', "/"));
                    data["accepted_write_kinds"] =
                        json!(["artifact", "source_mutation", "temp_dataset"]);
                    data["detected_intent"] =
                        json!(detected_write_intent(&path.replace('\\', "/")));
                    data["no_file_written"] = json!(true);
                    data["runtime_note"] = json!(
                        "os.write_file is compatibility-only; prefer explicit os.write_artifact, os.write_temp_dataset, or source mutation preview/apply capabilities"
                    );
                    return Ok(CapabilityReceipt {
                        capability_id: "os.write_file".to_string(),
                        job_id: self.token.job_id.clone(),
                        pid: self.token.pid.clone(),
                        status: "failed".to_string(),
                        data,
                    });
                };
                if !is_valid_write_kind(write_kind) {
                    let argument_error = invalid_write_kind_argument_error(
                        &self.truth,
                        &self.runtime_id,
                        &decision.output_spec,
                        write_kind,
                    )?;
                    return Ok(CapabilityReceipt {
                        capability_id: capability_id.to_string(),
                        job_id: self.token.job_id.clone(),
                        pid: self.token.pid.clone(),
                        status: "failed".to_string(),
                        data: argument_error.to_receipt_data(
                            &self.runtime_id,
                            reasoning_step_id,
                            &decision.decision_id,
                        ),
                    });
                }
                let content = content_arg(&self.truth, decision)?;
                os().write_file(&path, content.as_bytes(), write_kind)
            }
            "os.write_artifact" => {
                let path = path_arg(decision, "path")?;
                let content = self.content_arg_or_approved_preview_content(
                    reasoning_step_id,
                    decision,
                    &path,
                )?;
                os().write_artifact(&path, content.as_bytes())
            }
            "os.write_temp_dataset" => {
                let path = path_arg(decision, "path")?;
                let content = self.content_arg_or_approved_preview_content(
                    reasoning_step_id,
                    decision,
                    &path,
                )?;
                os().write_temp_dataset(&path, content.as_bytes())
            }
            "os.write_source_mutation_preview" => {
                let path = path_arg(decision, "path")?;
                let content = content_arg(&self.truth, decision)?;
                os().write_source_mutation_preview(&path, content.as_bytes())
            }
            "os.write_source_mutation_apply" => {
                let path = path_arg(decision, "path")?;
                let content = content_arg(&self.truth, decision)?;
                os().write_source_mutation_apply(&path, content.as_bytes())
            }
            "os.copy_path" => os().copy_path(
                &path_arg(decision, "source_path")?,
                &path_arg(decision, "destination_path")?,
            ),
            "os.move_path" => os().move_path(
                &path_arg(decision, "source_path")?,
                &path_arg(decision, "destination_path")?,
            ),
            "os.rename_path" => os().rename_path(
                &path_arg(decision, "source_path")?,
                &path_arg(decision, "destination_path")?,
            ),
            "os.delete_path" => os().delete_path(&path_arg(decision, "path")?),
            "os.hash_path" => os().hash_path(&path_arg(decision, "path")?),
            "os.diff" => os().diff_files(
                &path_arg(decision, "left_path")?,
                &path_arg(decision, "right_path")?,
            ),
            "os.zip" => {
                let source_paths = decision
                    .output_spec
                    .get("source_paths")
                    .and_then(Value::as_array)
                    .ok_or_else(|| {
                        io::Error::new(io::ErrorKind::InvalidInput, "source_paths missing")
                    })?
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect::<Vec<_>>();
                let source_refs = source_paths.iter().map(String::as_str).collect::<Vec<_>>();
                os().zip_paths(&source_refs, &path_arg(decision, "destination_zip_path")?)
            }
            "os.unzip" => os().unzip_archive(
                &path_arg(decision, "archive_path")?,
                &path_arg(decision, "destination_dir")?,
            ),
            "os.rollback_tx" => os().rollback_tx(&string_arg(decision, "tx_id")?),
            "os.verify_artifact" => os().verify_artifact(&path_arg(decision, "path")?),
            "source_set.create" => {
                DataRuntime::new(self.guard.clone(), self.truth.clone(), self.token.clone())
                    .create_source_set(
                        &path_arg(decision, "root_path").unwrap_or_else(|_| ".".to_string()),
                        &string_array_arg(decision, "include_extensions"),
                        &string_array_arg(decision, "include_globs"),
                        &string_array_arg(decision, "exclude_globs"),
                        depth_arg(decision, 64),
                    )
            }
            "source_set.read_page" => {
                DataRuntime::new(self.guard.clone(), self.truth.clone(), self.token.clone())
                    .read_source_set_page(
                        &string_arg(decision, "source_set_ref")?,
                        decision
                            .output_spec
                            .get("offset")
                            .and_then(Value::as_u64)
                            .unwrap_or(0) as usize,
                        decision
                            .output_spec
                            .get("limit")
                            .and_then(Value::as_u64)
                            .unwrap_or(100) as usize,
                    )
            }
            "dataset.read_page" => ReadOnlyCapabilityExecutor::new(
                self.guard.clone(),
                self.truth.clone(),
                self.token.clone(),
                self.runtime_id.clone(),
            )
            .execute(decision),
            "data.csv.read_dataset" => {
                DataRuntime::new(self.guard.clone(), self.truth.clone(), self.token.clone())
                    .read_csv_dataset(
                        &path_arg(decision, "input_path")
                            .or_else(|_| path_arg(decision, "path"))?,
                        decision
                            .output_spec
                            .get("has_header")
                            .and_then(Value::as_bool)
                            .unwrap_or(true),
                        decision
                            .output_spec
                            .get("max_rows")
                            .and_then(Value::as_u64)
                            .unwrap_or(10_000) as usize,
                    )
            }
            "source_set.coverage_verify" => {
                ArtifactRuntime::new(self.guard.clone(), self.truth.clone(), self.token.clone())
                    .source_set_coverage_verify(&string_arg(decision, "source_set_ref")?)
            }
            "workspace.batch_hash" => {
                DataRuntime::new(self.guard.clone(), self.truth.clone(), self.token.clone())
                    .batch_hash(&string_arg(decision, "source_set_ref")?)
            }
            "workspace.find_duplicates" => {
                DataRuntime::new(self.guard.clone(), self.truth.clone(), self.token.clone())
                    .find_duplicates(&string_arg(decision, "source_set_ref")?)
            }
            "workspace.recent_changes" => {
                DataRuntime::new(self.guard.clone(), self.truth.clone(), self.token.clone())
                    .recent_changes(
                        &string_arg(decision, "source_set_ref")?,
                        decision
                            .output_spec
                            .get("days")
                            .and_then(Value::as_u64)
                            .unwrap_or(7),
                    )
            }
            "workspace.plan_organize" => {
                DataRuntime::new(self.guard.clone(), self.truth.clone(), self.token.clone())
                    .plan_organize(
                        &string_arg(decision, "source_set_ref")?,
                        decision
                            .output_spec
                            .get("destination_root")
                            .and_then(Value::as_str)
                            .unwrap_or("archive/by_project"),
                    )
            }
            "workspace.apply_organize_tx" => {
                DataRuntime::new(self.guard.clone(), self.truth.clone(), self.token.clone())
                    .apply_organize_tx(&string_arg(decision, "organize_plan_ref")?, None)
            }
            "workspace.rename_batch_preview" => {
                DataRuntime::new(self.guard.clone(), self.truth.clone(), self.token.clone())
                    .rename_batch_preview(&decision.output_spec)
            }
            "workspace.rename_batch_apply" => {
                DataRuntime::new(self.guard.clone(), self.truth.clone(), self.token.clone())
                    .rename_batch_apply(&string_arg(decision, "rename_plan_ref")?, None)
            }
            "workspace.tree_index" => {
                DataRuntime::new(self.guard.clone(), self.truth.clone(), self.token.clone())
                    .tree_index(
                        &string_arg(decision, "source_set_ref")?,
                        decision
                            .output_spec
                            .get("tree_path")
                            .and_then(Value::as_str),
                    )
            }
            "workspace.perf_inventory" => {
                DataRuntime::new(self.guard.clone(), self.truth.clone(), self.token.clone())
                    .perf_inventory(
                        &string_arg(decision, "source_set_ref")?,
                        decision
                            .output_spec
                            .get("output_path")
                            .and_then(Value::as_str),
                        None,
                    )
            }
            "workspace.recent_changes_snapshot" => {
                DataRuntime::new(self.guard.clone(), self.truth.clone(), self.token.clone())
                    .recent_changes(
                        &string_arg(decision, "source_set_ref")?,
                        decision
                            .output_spec
                            .get("days")
                            .and_then(Value::as_u64)
                            .unwrap_or(7),
                    )
            }
            "dataset.export_csv" => {
                DataRuntime::new(self.guard.clone(), self.truth.clone(), self.token.clone())
                    .export_dataset_csv(
                        &string_arg(decision, "dataset_ref")?,
                        &path_arg(decision, "output_path")
                            .or_else(|_| path_arg(decision, "path"))?,
                    )
            }
            "dataset.export_markdown" => {
                DataRuntime::new(self.guard.clone(), self.truth.clone(), self.token.clone())
                    .export_dataset_markdown(
                        &string_arg(decision, "dataset_ref")?,
                        &path_arg(decision, "output_path")
                            .or_else(|_| path_arg(decision, "path"))?,
                        decision
                            .output_spec
                            .get("title")
                            .and_then(Value::as_str)
                            .unwrap_or("Dataset Export"),
                    )
            }
            "dataset.coverage_verify" => {
                ArtifactRuntime::new(self.guard.clone(), self.truth.clone(), self.token.clone())
                    .dataset_coverage_verify(&string_arg(decision, "dataset_ref")?)
            }
            "artifact.inspect" | "artifact.audit_readonly" => ReadOnlyCapabilityExecutor::new(
                self.guard.clone(),
                self.truth.clone(),
                self.token.clone(),
                self.runtime_id.clone(),
            )
            .execute(decision),
            "artifact.copy_source_set" => {
                DataRuntime::new(self.guard.clone(), self.truth.clone(), self.token.clone())
                    .copy_source_set(
                        &string_arg(decision, "source_set_ref")?,
                        &path_arg(decision, "destination_dir")?,
                    )
            }
            "client_env.scan_overview" => {
                ClientEnvRuntime::new(self.guard.clone(), self.truth.clone(), self.token.clone())
                    .scan_overview(ClientEnvScanOptions::from_value(&decision.output_spec)?)
            }
            "client_env.scan_device" => {
                ClientEnvRuntime::new(self.guard.clone(), self.truth.clone(), self.token.clone())
                    .scan_device(ClientEnvScanOptions::from_value(&decision.output_spec)?)
            }
            "client_env.scan_storage" => {
                ClientEnvRuntime::new(self.guard.clone(), self.truth.clone(), self.token.clone())
                    .scan_storage(ClientEnvScanOptions::from_value(&decision.output_spec)?)
            }
            "client_env.scan_network" => {
                ClientEnvRuntime::new(self.guard.clone(), self.truth.clone(), self.token.clone())
                    .scan_network(ClientEnvScanOptions::from_value(&decision.output_spec)?)
            }
            "client_env.scan_runtimes" => {
                ClientEnvRuntime::new(self.guard.clone(), self.truth.clone(), self.token.clone())
                    .scan_runtimes(ClientEnvScanOptions::from_value(&decision.output_spec)?)
            }
            "client_env.read_snapshot" => {
                let snapshot_ref = string_arg(decision, "snapshot_ref")
                    .or_else(|_| string_arg(decision, "ref"))?;
                let offset = decision
                    .output_spec
                    .get("offset")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as usize;
                let limit = decision
                    .output_spec
                    .get("limit")
                    .and_then(Value::as_u64)
                    .unwrap_or(20) as usize;
                ClientEnvRuntime::new(self.guard.clone(), self.truth.clone(), self.token.clone())
                    .read_snapshot(&snapshot_ref, offset, limit)
            }
            "client_env.request_sensitive_disclosure" => {
                let requested_fields = string_array_arg(decision, "requested_fields");
                let reason = decision
                    .output_spec
                    .get("reason")
                    .and_then(Value::as_str)
                    .unwrap_or(&decision.reason);
                ClientEnvRuntime::new(self.guard.clone(), self.truth.clone(), self.token.clone())
                    .request_sensitive_disclosure(requested_fields, reason)
            }
            "terminal.run_command" => {
                let argv = decision
                    .output_spec
                    .get("argv")
                    .and_then(Value::as_array)
                    .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "argv missing"))?
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect::<Vec<_>>();
                let timeout_ms = decision
                    .output_spec
                    .get("timeout_ms")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| {
                        io::Error::new(io::ErrorKind::InvalidInput, "timeout_ms missing")
                    })?;
                let approval = approval_guard
                    .as_ref()
                    .map(|guard| {
                        crate::terminal_runtime::TerminalApproval::approved(
                            guard.approval_token_id.clone(),
                        )
                    })
                    .unwrap_or_else(crate::terminal_runtime::TerminalApproval::none);
                TerminalRuntime::new(self.guard.clone(), self.truth.clone(), self.token.clone())
                    .run_command_with_approval(argv, timeout_ms, approval)
            }
            "terminal.start_service" => {
                let service_id = string_arg(decision, "service_id")?;
                let argv = decision
                    .output_spec
                    .get("argv")
                    .and_then(Value::as_array)
                    .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "argv missing"))?
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect::<Vec<_>>();
                let startup_timeout_ms = decision
                    .output_spec
                    .get("startup_timeout_ms")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| {
                        io::Error::new(io::ErrorKind::InvalidInput, "startup_timeout_ms missing")
                    })?;
                let health_check = terminal_service_health_check_arg(&decision.output_spec);
                let expected_ports = terminal_expected_ports_arg(&decision.output_spec);
                TerminalRuntime::new(self.guard.clone(), self.truth.clone(), self.token.clone())
                    .start_service(
                        &service_id,
                        argv,
                        startup_timeout_ms,
                        health_check,
                        expected_ports,
                    )
            }
            "terminal.stop_service" => {
                let service_id = string_arg(decision, "service_id")?;
                let reason = decision.output_spec.get("reason").and_then(Value::as_str);
                TerminalRuntime::new(self.guard.clone(), self.truth.clone(), self.token.clone())
                    .stop_service(&service_id, reason)
            }
            "terminal.service_status" => {
                let service_id = string_arg(decision, "service_id")?;
                TerminalRuntime::new(self.guard.clone(), self.truth.clone(), self.token.clone())
                    .service_status(&service_id)
            }
            "model.extract_json"
            | "model.extract_dataset"
            | "model.summarize"
            | "model.summarize_dataset"
            | "model.rewrite"
            | "model.generate_artifact"
            | "model.synthesize_artifact_from_dataset"
            | "model.audit" => {
                self.run_model_runtime_capability(reasoning_step_id, &action.action_id, decision)
            }
            "model.audit_artifact" | "model.audit_artifact_quality" => self
                .run_model_artifact_audit_capability(
                    goal,
                    reasoning_step_id,
                    &action.action_id,
                    decision,
                ),
            "process.read_ref" => self.read_typed_ref(decision),
            "tool.result.page" => self.page_tool_result(decision),
            "tool.result.search" => self.search_tool_result(decision),
            "tool.result.inspect_schema" => self.inspect_tool_result_schema(decision),
            "process.query_events" => self.query_process_events(decision),
            "process.toolset.select" => self.provider_toolset_select_receipt(decision),
            "process.preview.create" => {
                self.create_preview_capability_receipt(reasoning_step_id, decision)
            }
            "process.pending_approvals" => self.pending_approvals(),
            "office.docx.read_text" => self
                .office_runtime()
                .read_text(&path_arg(decision, "input_path")?),
            "office.workbook.read_text" => self.office_runtime().read_workbook_text(
                &path_arg(decision, "input_path").or_else(|_| path_arg(decision, "path"))?,
                decision.output_spec.get("sheet").and_then(Value::as_str),
                decision
                    .output_spec
                    .get("max_rows")
                    .and_then(Value::as_u64)
                    .unwrap_or(200) as usize,
            ),
            "office.workbook.read_cells" => self.office_runtime().read_workbook_cells(
                &path_arg(decision, "input_path").or_else(|_| path_arg(decision, "path"))?,
                decision.output_spec.get("sheet").and_then(Value::as_str),
                decision
                    .output_spec
                    .get("max_rows")
                    .and_then(Value::as_u64)
                    .unwrap_or(200) as usize,
            ),
            "office.inspect_workbook" => ReadOnlyCapabilityExecutor::new(
                self.guard.clone(),
                self.truth.clone(),
                self.token.clone(),
                self.runtime_id.clone(),
            )
            .execute(decision),
            "document.pdf.extract_text" => ReadOnlyCapabilityExecutor::new(
                self.guard.clone(),
                self.truth.clone(),
                self.token.clone(),
                self.runtime_id.clone(),
            )
            .execute(decision),
            "office.docx.batch_read_text" => self
                .office_runtime()
                .batch_read_text(&string_arg(decision, "source_set_ref")?),
            "office.docx.batch_validate" => self
                .office_runtime()
                .batch_validate(&string_arg(decision, "source_set_ref")?),
            "office.docx.batch_extract_metadata" => self
                .office_runtime()
                .batch_extract_metadata(&string_arg(decision, "source_set_ref")?),
            "office.docx.create" => {
                let output_path =
                    path_arg(decision, "output_path").or_else(|_| path_arg(decision, "path"))?;
                let text = content_arg(&self.truth, decision)?;
                let title = decision.output_spec.get("title").and_then(Value::as_str);
                self.office_runtime()
                    .create_docx(&output_path, &text, title)
            }
            "office.docx.rewrite_save_as" => {
                let text = content_arg(&self.truth, decision)?;
                self.office_runtime().rewrite_save_as(
                    &path_arg(decision, "input_path")?,
                    &path_arg(decision, "output_path")?,
                    &text,
                )
            }
            "office.docx.rewrite_preview" => {
                let text = content_arg(&self.truth, decision)?;
                self.office_runtime()
                    .preview_rewrite(&path_arg(decision, "input_path")?, &text)
            }
            "office.docx.rewrite_in_place_preview" => {
                let text = content_arg(&self.truth, decision)?;
                self.office_runtime()
                    .preview_in_place_rewrite(&path_arg(decision, "input_path")?, &text)
            }
            "office.docx.rewrite_in_place" => {
                let text = content_arg(&self.truth, decision)?;
                self.office_runtime()
                    .rewrite_in_place(&path_arg(decision, "input_path")?, &text)
            }
            "office.docx.diff_summary" => self.office_runtime().diff_summary(
                &path_arg(decision, "before_path")?,
                &path_arg(decision, "after_path")?,
            ),
            "office.docx.validate" => self
                .office_runtime()
                .validate_docx(&path_arg(decision, "input_path")?),
            "package.build_zip" => {
                PackageRuntime::new(self.guard.clone(), self.truth.clone(), self.token.clone())
                    .build_zip(
                        &string_arg(decision, "source_set_ref")?,
                        &path_arg(decision, "destination_zip_path")?,
                        decision
                            .output_spec
                            .get("manifest_path")
                            .and_then(Value::as_str),
                        decision
                            .output_spec
                            .get("checksums_path")
                            .and_then(Value::as_str),
                        decision
                            .output_spec
                            .get("perf_notes_path")
                            .and_then(Value::as_str),
                        &string_array_arg(decision, "exclude_globs"),
                    )
            }
            "artifact.verify_coverage" => {
                ArtifactRuntime::new(self.guard.clone(), self.truth.clone(), self.token.clone())
                    .verify_coverage_with_contract(
                        &path_arg(decision, "artifact_path")
                            .or_else(|_| path_arg(decision, "path"))?,
                        decision
                            .output_spec
                            .get("source_set_ref")
                            .and_then(Value::as_str),
                        decision
                            .output_spec
                            .get("dataset_ref")
                            .and_then(Value::as_str),
                        decision
                            .output_spec
                            .get("coverage_contract")
                            .or_else(|| decision.output_spec.get("contract")),
                    )
            }
            "artifact.source_coverage_verify" => {
                ArtifactRuntime::new(self.guard.clone(), self.truth.clone(), self.token.clone())
                    .verify_coverage_with_contract(
                        &path_arg(decision, "artifact_path")
                            .or_else(|_| path_arg(decision, "path"))?,
                        decision
                            .output_spec
                            .get("source_set_ref")
                            .and_then(Value::as_str),
                        decision
                            .output_spec
                            .get("dataset_ref")
                            .and_then(Value::as_str),
                        decision
                            .output_spec
                            .get("coverage_contract")
                            .or_else(|| decision.output_spec.get("contract")),
                    )
            }
            "artifact.verify_typed" => {
                ArtifactRuntime::new(self.guard.clone(), self.truth.clone(), self.token.clone())
                    .verify_typed_artifact(
                        &path_arg(decision, "artifact_path")
                            .or_else(|_| path_arg(decision, "path"))?,
                    )
            }
            "artifact.audit_quality" => {
                ArtifactRuntime::new(self.guard.clone(), self.truth.clone(), self.token.clone())
                    .audit_quality(
                        &path_arg(decision, "artifact_path")
                            .or_else(|_| path_arg(decision, "path"))?,
                        decision
                            .output_spec
                            .get("minimum_chars")
                            .and_then(Value::as_u64)
                            .unwrap_or(80) as usize,
                        decision
                            .output_spec
                            .get("require_source_refs")
                            .and_then(Value::as_bool)
                            .unwrap_or(false),
                    )
            }
            other => Ok(CapabilityReceipt {
                capability_id: other.to_string(),
                job_id: self.token.job_id.clone(),
                pid: self.token.pid.clone(),
                status: "blocked".to_string(),
                data: json!({
                    "reason": "registered capability is not yet dispatchable by TaskAgent",
                    "capability_id": other,
                }),
            }),
        }?;
        self.finalize_registered_capability_approval(approval_guard.as_ref(), &receipt)?;
        if let Some(preview) = bind_preview_capability_receipt_to_tx(
            &self.truth,
            &self.token.pid,
            descriptor,
            &receipt,
        )? {
            if let Some(data) = receipt.data.as_object_mut() {
                data.insert("waiting_for_approval".to_string(), json!(true));
                data.insert("preview_tx_id".to_string(), json!(preview.tx_id));
                data.insert("preview_id".to_string(), json!(preview.preview_id));
                data.insert("preview_ref".to_string(), json!(preview.preview_ref));
                data.insert(
                    "proposed_actions".to_string(),
                    json!(preview.proposed_actions),
                );
                data.insert("target_paths".to_string(), json!(preview.target_paths));
                data.insert(
                    "executable_operations".to_string(),
                    json!(preview.executable_operations),
                );
            }
            self.truth.update_job_status("waiting_approval")?;
        }
        Ok(receipt)
    }

    fn provider_tool_decision(
        &self,
        receipt: &ModelCallReceipt,
        tool_call: &ProviderToolCall,
    ) -> Result<NextActionDecision, ProviderToolProtocolError> {
        self.provider_tool_registry_for_receipt(receipt)
            .decision_for_tool_call(tool_call)
    }

    fn provider_tool_registry_for_receipt(
        &self,
        receipt: &ModelCallReceipt,
    ) -> ProviderToolRegistry {
        if let Some(provider_toolset_ref) = &receipt.provider_toolset_ref {
            if let Ok(path) = self.truth.resolve_blob_ref(provider_toolset_ref) {
                if let Ok(bytes) = fs::read(path) {
                    if let Ok(record) = serde_json::from_slice::<ProviderToolsetRecord>(&bytes) {
                        return ProviderToolRegistry::phase6_selected(
                            &self.registry,
                            &self.model_config,
                            &record.selected_capability_ids,
                            "phase6_recorded_request_toolset",
                        );
                    }
                }
            }
        }
        ProviderToolRegistry::phase6_full_coverage(&self.registry, &self.model_config)
    }

    fn execute_provider_tool_decision(
        &self,
        goal: &str,
        turn_id: &str,
        decision: &NextActionDecision,
    ) -> io::Result<(String, Value)> {
        match decision.kind {
            TaskAgentDecisionKind::RunCapability => {
                if let Some(error) = self.provider_native_write_artifact_policy_error(decision)? {
                    return Err(error);
                }
                let receipt = self.run_registered_capability(goal, turn_id, decision)?;
                self.record_capability_execution(decision, &receipt)?;
                let mut status = "running".to_string();
                if receipt
                    .data
                    .get("waiting_for_approval")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                {
                    status = "waiting_approval".to_string();
                } else if provider_native_receipt_is_recoverable_error(&receipt) {
                    let tool_result = self.provider_native_recoverable_tool_result_from_receipt(
                        turn_id,
                        decision,
                        &receipt,
                        provider_native_receipt_recovery_kind(&receipt),
                        provider_native_receipt_error_code(&receipt),
                        provider_native_receipt_corrective_instruction(&receipt),
                    )?;
                    return Ok(("recoverable_error".to_string(), tool_result));
                }
                let tool_result = self.provider_tool_result_from_receipt(turn_id, &receipt)?;
                Ok((status, tool_result))
            }
            TaskAgentDecisionKind::RequestPreview => {
                let receipt = self.request_preview_receipt(turn_id, decision)?;
                self.record_capability_execution(decision, &receipt)?;
                let status = if receipt
                    .data
                    .get("waiting_for_approval")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                {
                    "waiting_approval".to_string()
                } else if provider_native_receipt_is_recoverable_error(&receipt) {
                    let tool_result = self.provider_native_recoverable_tool_result_from_receipt(
                        turn_id,
                        decision,
                        &receipt,
                        provider_native_receipt_recovery_kind(&receipt),
                        provider_native_receipt_error_code(&receipt),
                        provider_native_receipt_corrective_instruction(&receipt),
                    )?;
                    return Ok(("recoverable_error".to_string(), tool_result));
                } else {
                    "running".to_string()
                };
                let tool_result = self.provider_tool_result_from_receipt(turn_id, &receipt)?;
                Ok((status, tool_result))
            }
            TaskAgentDecisionKind::Clarify => {
                self.clarify(turn_id, decision)?;
                Ok((
                    "waiting_user".to_string(),
                    json!({
                        "status": "waiting_user",
                        "capability_id": "process.clarify",
                        "reason": decision.reason,
                    }),
                ))
            }
            TaskAgentDecisionKind::Complete => {
                let event_count_before = self.truth.read_events()?.len();
                let status = self.complete(turn_id, decision)?;
                let tool_result = self.provider_complete_tool_result(
                    turn_id,
                    &status,
                    decision,
                    event_count_before,
                )?;
                Ok((status, tool_result))
            }
            TaskAgentDecisionKind::Fail => {
                self.fail(turn_id, decision)?;
                Ok((
                    "failed".to_string(),
                    json!({
                        "status": "failed",
                        "capability_id": "process.fail",
                        "reason": decision.reason,
                    }),
                ))
            }
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "provider-native tool call decoded to unsupported decision kind",
            )),
        }
    }

    fn provider_native_write_intent_preview_receipt(
        &self,
        turn_id: &str,
        decision: &NextActionDecision,
    ) -> io::Result<CapabilityReceipt> {
        let capability_id = decision.capability_id.as_str();
        let (target_paths, target_path_schema) = self
            .capability_descriptor(capability_id)
            .ok()
            .and_then(|descriptor| {
                build_capability_approval_request(
                    &self.truth,
                    descriptor,
                    &decision.output_spec,
                    None,
                )
                .ok()
            })
            .map(|request| (request.target_paths, request.target_path_schema))
            .unwrap_or_else(|| (Vec::new(), "unknown".to_string()));
        let preview_markdown = provider_native_write_intent_preview_markdown(
            capability_id,
            &target_paths,
            &decision.reason,
            provider_native_write_artifact_inline_text(decision),
        );
        let mut preview_decision = crate::reasoning::decision(
            TaskAgentDecisionKind::RequestPreview,
            "process.request_preview",
            &format!(
                "Provider-native {capability_id} tool_call requires Kernel approval; creating Kernel preview before apply."
            ),
        );
        preview_decision.output_spec = json!({
            "preview_markdown": preview_markdown,
            "risk_level": "medium",
            "operations": [{
                "capability_id": capability_id,
                "arguments": decision.output_spec.clone(),
                "target_paths": target_paths,
                "human_description": format!("Apply provider-native {capability_id} write intent after approval."),
            }],
            "provider_native_auto_preview": true,
            "original_capability_id": capability_id,
            "target_path_schema": target_path_schema,
        });
        let receipt = self.request_preview_receipt(turn_id, &preview_decision)?;
        let mut data = receipt.data.clone();
        if let Some(object) = data.as_object_mut() {
            object.insert("provider_native_auto_preview".to_string(), json!(true));
            object.insert("original_capability_id".to_string(), json!(capability_id));
            object.insert(
                "runtime_note".to_string(),
                json!("Provider adapter converted a write intent into a Kernel approval preview. After the frontend approval decision, RootAgentProcessController executes or rejects the original provider tool_call and records the tool result in the provider transcript."),
            );
        }
        Ok(CapabilityReceipt { data, ..receipt })
    }

    fn provider_native_mutation_auto_preview_receipt(
        &self,
        turn_id: &str,
        decision: &NextActionDecision,
    ) -> io::Result<CapabilityReceipt> {
        let capability_id = decision.capability_id.as_str();
        let (target_paths, target_path_schema) = self
            .capability_descriptor(capability_id)
            .ok()
            .and_then(|descriptor| {
                build_capability_approval_request(
                    &self.truth,
                    descriptor,
                    &decision.output_spec,
                    None,
                )
                .ok()
            })
            .map(|request| (request.target_paths, request.target_path_schema))
            .unwrap_or_else(|| (Vec::new(), "unknown".to_string()));
        let operation_arguments =
            provider_native_arguments_without_approval_id(&decision.output_spec);
        let preview_markdown = provider_native_mutation_preview_markdown(
            capability_id,
            &target_paths,
            &decision.reason,
            &operation_arguments,
        );
        let mut preview_decision = crate::reasoning::decision(
            TaskAgentDecisionKind::RequestPreview,
            "process.request_preview",
            &format!(
                "Provider-native {capability_id} tool_call requires Kernel approval; creating Kernel preview before apply."
            ),
        );
        preview_decision.output_spec = json!({
            "preview_markdown": preview_markdown,
            "risk_level": provider_native_auto_preview_risk_level(capability_id),
            "operations": [{
                "capability_id": capability_id,
                "arguments": operation_arguments,
                "target_paths": target_paths,
                "human_description": format!("Apply provider-native {capability_id} mutation after approval."),
            }],
            "provider_native_auto_preview": true,
            "original_capability_id": capability_id,
            "target_path_schema": target_path_schema,
        });
        let receipt = self.request_preview_receipt(turn_id, &preview_decision)?;
        let mut data = receipt.data.clone();
        if let Some(object) = data.as_object_mut() {
            object.insert("provider_native_auto_preview".to_string(), json!(true));
            object.insert("original_capability_id".to_string(), json!(capability_id));
            object.insert(
                "runtime_note".to_string(),
                json!("Provider adapter converted a mutation/apply intent into a Kernel approval preview. After the frontend approval decision, RootAgentProcessController executes or rejects the original provider tool_call and records the tool result in the provider transcript."),
            );
        }
        Ok(CapabilityReceipt { data, ..receipt })
    }

    fn provider_native_terminal_approval_preview_receipt(
        &self,
        turn_id: &str,
        decision: &NextActionDecision,
        blocked_receipt: &CapabilityReceipt,
    ) -> io::Result<CapabilityReceipt> {
        let target_paths = blocked_receipt
            .data
            .get("target_paths")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let argv = decision
            .output_spec
            .get("argv")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let risk_reason = blocked_receipt
            .data
            .get("reason")
            .and_then(Value::as_str)
            .unwrap_or("capability_requires_preview_approval");
        let preview_markdown =
            provider_native_terminal_preview_markdown(&argv, &target_paths, risk_reason);
        let mut preview_decision = crate::reasoning::decision(
            TaskAgentDecisionKind::RequestPreview,
            "process.request_preview",
            "Provider-native terminal command requires Kernel approval before execution.",
        );
        preview_decision.output_spec = json!({
            "preview_markdown": preview_markdown,
            "risk_level": "high",
            "operations": [{
                "capability_id": "terminal.run_command",
                "arguments": decision.output_spec.clone(),
                "target_paths": target_paths,
                "human_description": "Execute the pending terminal command after approval.",
            }],
            "provider_native_terminal_preview": true,
            "original_capability_id": "terminal.run_command",
            "terminal_command": {
                "argv": argv,
                "target_paths": blocked_receipt.data.get("target_paths").cloned().unwrap_or_else(|| json!([])),
                "approval_policy": blocked_receipt.data.get("approval_policy").cloned(),
                "risk_reason": risk_reason,
            },
        });
        let receipt = self.request_preview_receipt(turn_id, &preview_decision)?;
        let mut data = receipt.data.clone();
        if let Some(object) = data.as_object_mut() {
            object.insert("provider_native_terminal_preview".to_string(), json!(true));
            object.insert(
                "original_capability_id".to_string(),
                json!("terminal.run_command"),
            );
            object.insert(
                "terminal_command".to_string(),
                preview_decision
                    .output_spec
                    .get("terminal_command")
                    .cloned()
                    .unwrap_or(Value::Null),
            );
            object.insert(
                "runtime_note".to_string(),
                json!("Provider adapter converted a terminal command requiring approval into a Kernel preview. User approval will issue an approval token; the command executes only through the Capability Kernel path."),
            );
        }
        Ok(CapabilityReceipt { data, ..receipt })
    }

    fn content_arg_or_approved_preview_content(
        &self,
        reasoning_step_id: &str,
        decision: &NextActionDecision,
        target_path: &str,
    ) -> io::Result<String> {
        if content_arg_field_present(&decision.output_spec) {
            return content_arg(&self.truth, decision);
        }
        if !self.provider_native_tool_calls_enabled()
            || !provider_native_write_intent_capability(&decision.capability_id)
        {
            return content_arg(&self.truth, decision);
        }
        let Some(approval_id) = approval_id_arg(decision) else {
            return content_arg(&self.truth, decision);
        };
        let hydrated = self.approved_preview_content_for_write_intent(
            &approval_id,
            &decision.capability_id,
            target_path,
        )?;
        self.truth.append_event(
            Some(&self.token.pid),
            "provider_native_hydrated_content_from_preview",
            json!({
                "runtime_id": self.runtime_id,
                "turn_id": reasoning_step_id,
                "decision_id": decision.decision_id,
                "capability_id": decision.capability_id,
                "approval_id": approval_id,
                "tx_id": hydrated.tx_id,
                "preview_id": hydrated.preview_id,
                "target_path": target_path.replace('\\', "/"),
                "content_source_field": hydrated.source_field,
                "content_bytes": hydrated.content.len(),
                "raw_content_logged": false,
                "runtime_note": "Provider-native approved write apply omitted content; TaskAgent reused the content/text/content_ref/text_ref from the exact approved preview operation after Kernel approval validation.",
            }),
        )?;
        Ok(hydrated.content)
    }

    fn approved_preview_content_for_write_intent(
        &self,
        approval_id: &str,
        capability_id: &str,
        target_path: &str,
    ) -> io::Result<HydratedPreviewContent> {
        let events = self.truth.read_events()?;
        let token_event = events
            .iter()
            .rev()
            .find(|event| {
                event.event_type == "approval_token_issued"
                    && event.data.get("approval_token_id").and_then(Value::as_str)
                        == Some(approval_id)
            })
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "approval_token_issued not found for provider-native content hydration",
                )
            })?;
        let tx_id = token_event
            .data
            .get("tx_id")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "approval token missing tx_id for provider-native content hydration",
                )
            })?;
        let preview_id = token_event
            .data
            .get("preview_id")
            .and_then(Value::as_str)
            .unwrap_or("preview_unknown");
        let preview_event = events
            .iter()
            .rev()
            .find(|event| {
                event.event_type == "preview_tx_created"
                    && event.data.get("tx_id").and_then(Value::as_str) == Some(tx_id)
            })
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "preview_tx_created not found for provider-native content hydration",
                )
            })?;
        let operations = preview_event
            .data
            .get("executable_operations")
            .cloned()
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "preview tx missing executable_operations for provider-native content hydration",
                )
            })?;
        let operations = serde_json::from_value::<Vec<ExecutablePreviewOperation>>(operations)
            .map_err(crate::json_err)?;
        let matching = operations
            .iter()
            .filter(|operation| {
                operation.capability_id == capability_id
                    && operation_target_paths_include(operation, target_path)
            })
            .collect::<Vec<_>>();
        let operation = match matching.as_slice() {
            [operation] => *operation,
            [] => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "no approved preview operation matches provider-native write capability and target path",
                ));
            }
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "multiple approved preview operations match provider-native write capability and target path",
                ));
            }
        };
        let resolved = content_value_arg(&self.truth, &operation.arguments)?;
        Ok(HydratedPreviewContent {
            content: resolved.content,
            source_field: resolved.source_field,
            tx_id: tx_id.to_string(),
            preview_id: preview_id.to_string(),
        })
    }

    fn provider_tool_result_from_receipt(
        &self,
        turn_id: &str,
        receipt: &CapabilityReceipt,
    ) -> io::Result<Value> {
        let receipt_ref = self.truth.write_blob(
            &format!(
                "provider_tool_results/{}/{}_{}_receipt.json",
                safe_blob_name(&self.runtime_id),
                safe_blob_name(turn_id),
                now_ms()
            ),
            &serde_json::to_vec_pretty(receipt).map_err(crate::json_err)?,
        )?;
        let waiting_for_approval = receipt
            .data
            .get("waiting_for_approval")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let receipt_value = to_json_value(receipt)?;
        let mut result = json!({
            "status": if waiting_for_approval { "waiting_approval" } else { receipt.status.as_str() },
            "receipt_status": receipt.status.clone(),
            "capability_id": receipt.capability_id.clone(),
            "receipt_ref": receipt_ref,
            "receipt": receipt_value,
        });
        if let Some(object) = result.as_object_mut() {
            for key in [
                "waiting_for_approval",
                "preview_id",
                "preview_tx_id",
                "preview_ref",
                "target_paths",
                "proposed_actions",
                "executable_operations",
                "approval_required",
                "mutation_policy_blocked",
                "reason",
            ] {
                if let Some(value) = receipt.data.get(key) {
                    object.insert(key.to_string(), value.clone());
                }
            }
            if receipt.capability_id == "terminal.run_command" {
                if let Some(stdout) = self.receipt_blob_text(receipt, "stdout_ref")? {
                    object.insert("stdout_text".to_string(), json!(stdout));
                }
                if let Some(stderr) = self.receipt_blob_text(receipt, "stderr_ref")? {
                    object.insert("stderr_text".to_string(), json!(stderr));
                }
                object.insert(
                    "terminal_output_metadata".to_string(),
                    json!({
                        "exit_code": receipt.data.get("exit_code").cloned(),
                        "timed_out": receipt.data.get("timed_out").cloned(),
                        "stdout_bytes": receipt.data.get("stdout_bytes").cloned(),
                        "stderr_bytes": receipt.data.get("stderr_bytes").cloned(),
                    }),
                );
            }
        }
        Ok(result)
    }

    fn receipt_blob_text(
        &self,
        receipt: &CapabilityReceipt,
        ref_key: &str,
    ) -> io::Result<Option<String>> {
        let Some(blob_ref) = receipt.data.get(ref_key).and_then(Value::as_str) else {
            return Ok(None);
        };
        let path = self.truth.resolve_blob_ref(blob_ref)?;
        let bytes = fs::read(path)?;
        let text = String::from_utf8_lossy(&bytes).to_string();
        Ok(Some(text))
    }

    fn provider_native_recoverable_tool_result_from_receipt(
        &self,
        turn_id: &str,
        decision: &NextActionDecision,
        receipt: &CapabilityReceipt,
        recovery_kind: &str,
        error_code: &str,
        instruction: &str,
    ) -> io::Result<Value> {
        let mut result = self.provider_tool_result_from_receipt(turn_id, receipt)?;
        let provider_tool_name = provider_tool_name_for_capability(&decision.capability_id);
        let schema_summary =
            self.provider_tool_schema_summary(Some(&provider_tool_name), &decision.capability_id);
        let corrective_message = json!({
            "event": "provider_native_corrective_instruction",
            "correction_kind": recovery_kind,
            "error_code": error_code,
            "capability_id": decision.capability_id,
            "provider_tool_name": provider_tool_name,
            "failed_arguments": decision.output_spec,
            "kernel_receipt_status": receipt.status,
            "kernel_receipt_reason": receipt.data.get("reason").cloned(),
            "instruction": instruction,
            "tool_schema_summary": schema_summary,
            "process_authority": "The Kernel receipt remains the execution fact. The provider adapter marks this tool result recoverable only so the model can retry with corrected arguments under Kernel hard boundaries.",
        });
        if let Some(object) = result.as_object_mut() {
            object.insert("status".to_string(), json!("recoverable_error"));
            object.insert("recoverable".to_string(), json!(true));
            object.insert("recovery_kind".to_string(), json!(recovery_kind));
            object.insert("error_code".to_string(), json!(error_code));
            object.insert("corrective_instruction".to_string(), json!(instruction));
            object.insert("corrective_control_message".to_string(), corrective_message);
            object.insert("tool_schema_summary".to_string(), schema_summary);
            object.insert(
                "next_model_request_should_self_correct".to_string(),
                json!(true),
            );
        }
        Ok(result)
    }

    fn provider_complete_tool_result(
        &self,
        turn_id: &str,
        status: &str,
        decision: &NextActionDecision,
        event_count_before: usize,
    ) -> io::Result<Value> {
        if let Some(receipt) = self.latest_process_complete_receipt_since(event_count_before)? {
            let mut result = self.provider_tool_result_from_receipt(turn_id, &receipt)?;
            if let Some(object) = result.as_object_mut() {
                object.insert("task_status".to_string(), json!(status));
                object.insert("process_complete_status".to_string(), json!(status));
                object
                    .entry("reason".to_string())
                    .or_insert_with(|| json!(decision.reason));
            }
            return Ok(result);
        }
        Ok(json!({
            "status": status,
            "capability_id": "process.complete",
            "reason": decision.reason,
        }))
    }

    fn latest_process_complete_receipt_since(
        &self,
        event_count_before: usize,
    ) -> io::Result<Option<CapabilityReceipt>> {
        let events = self.truth.read_events()?;
        let new_events = events
            .into_iter()
            .skip(event_count_before)
            .collect::<Vec<_>>();
        for event in new_events.into_iter().rev() {
            if event.event_type != "capability_receipt" {
                continue;
            }
            let receipt =
                serde_json::from_value::<CapabilityReceipt>(event.data).map_err(crate::json_err)?;
            if receipt.capability_id == "process.complete" {
                return Ok(Some(receipt));
            }
        }
        Ok(None)
    }

    fn provider_native_recoverable_action_error(
        &self,
        decision: &NextActionDecision,
        err: &io::Error,
        provider_tool_name: Option<String>,
        provider_tool_call_id: Option<String>,
    ) -> io::Result<Option<ProviderNativeRecoverableActionError>> {
        let raw_error = err.to_string();
        let (recovery_kind, error_code, instruction, retry_example) =
            provider_native_recoverable_error_hint(decision, &raw_error).unwrap_or_else(|| {
                provider_native_generic_recoverable_error_hint(decision, &raw_error)
            });
        let latest_source_set_refs = self.latest_source_set_refs(5)?;
        let schema_summary = self
            .provider_tool_schema_summary(provider_tool_name.as_deref(), &decision.capability_id);
        let corrective_message = json!({
            "event": "provider_native_corrective_instruction",
            "correction_kind": recovery_kind,
            "error_code": error_code,
            "capability_id": decision.capability_id,
            "provider_tool_name": provider_tool_name,
            "provider_tool_call_id": provider_tool_call_id,
            "failed_arguments": decision.output_spec,
            "kernel_error": raw_error,
            "instruction": instruction,
            "retry_example": retry_example,
            "tool_schema_summary": schema_summary,
            "latest_valid_source_set_refs": latest_source_set_refs,
            "process_authority": "The Process Kernel rejection remains authoritative; this provider message only helps the model retry with corrected arguments.",
        });
        let tool_result_extra = json!({
            "recoverable": true,
            "recovery_kind": recovery_kind,
            "corrective_instruction": instruction,
            "retry_example": retry_example,
            "tool_schema_summary": schema_summary,
            "latest_valid_source_set_refs": latest_source_set_refs,
        });
        Ok(Some(ProviderNativeRecoverableActionError {
            recovery_kind: recovery_kind.to_string(),
            protocol_error: ProviderToolProtocolError {
                error_code: error_code.to_string(),
                message: format!("{instruction} Kernel error: {raw_error}"),
                provider_tool_name,
                provider_tool_call_id,
                capability_id: Some(decision.capability_id.clone()),
            },
            corrective_message,
            tool_result_extra,
        }))
    }

    fn provider_tool_schema_summary(
        &self,
        provider_tool_name: Option<&str>,
        capability_id: &str,
    ) -> Value {
        let registry =
            ProviderToolRegistry::phase6_full_coverage(&self.registry, &self.model_config);
        let expected_tool_name = provider_tool_name
            .map(ToString::to_string)
            .unwrap_or_else(|| provider_tool_name_for_capability(capability_id));
        let tool = registry
            .tools
            .iter()
            .find(|tool| tool.function.name == expected_tool_name)
            .or_else(|| {
                registry.tools.iter().find(|tool| {
                    registry
                        .binding_for_tool_name(&tool.function.name)
                        .is_some_and(|binding| binding.capability_id == capability_id)
                })
            });
        let Some(tool) = tool else {
            return json!({
                "provider_tool_name": expected_tool_name,
                "capability_id": capability_id,
                "schema_available": false,
            });
        };
        let parameters = &tool.function.parameters;
        let required = parameters
            .get("required")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect::<Vec<_>>();
        let properties = parameters
            .get("properties")
            .and_then(Value::as_object)
            .map(|properties| properties.keys().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        json!({
            "provider_tool_name": tool.function.name,
            "capability_id": capability_id,
            "schema_available": true,
            "required": required,
            "properties": properties,
        })
    }

    fn provider_native_write_artifact_policy_error(
        &self,
        decision: &NextActionDecision,
    ) -> io::Result<Option<io::Error>> {
        if decision.capability_id != "os.write_artifact" {
            return Ok(None);
        }
        let Some(path) = decision.output_spec.get("path").and_then(Value::as_str) else {
            return Ok(None);
        };
        if let Some(extension) = provider_native_blocked_artifact_extension(path) {
            return Ok(Some(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "provider-native os.write_artifact cannot create `{extension}` targets; binary/compound artifacts require their native Kernel capability receipt"
                ),
            )));
        }
        if provider_native_write_artifact_claims_workspace_mutation(decision)
            && !self.provider_native_has_successful_workspace_mutation_receipt()?
        {
            return Ok(Some(io::Error::new(
                io::ErrorKind::InvalidInput,
                "provider-native os.write_artifact cannot claim workspace mutations were executed before a real mutation capability receipt exists",
            )));
        }
        Ok(None)
    }

    fn provider_native_has_successful_workspace_mutation_receipt(&self) -> io::Result<bool> {
        Ok(self.truth.read_events()?.iter().any(|event| {
            if event.event_type != "capability_receipt" {
                return false;
            }
            let Some(capability_id) = event.data.get("capability_id").and_then(Value::as_str)
            else {
                return false;
            };
            let status = event
                .data
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or_default();
            matches!(status, "success" | "completed")
                && provider_native_real_workspace_mutation_receipt(capability_id)
        }))
    }

    fn provider_native_preview_operation_scope_error(
        &self,
        decision: &NextActionDecision,
        provider_tool_name: Option<String>,
        provider_tool_call_id: Option<String>,
    ) -> io::Result<Option<ProviderNativeRecoverableActionError>> {
        let _ = (decision, provider_tool_name, provider_tool_call_id);
        Ok(None)
    }

    fn latest_source_set_refs(&self, limit: usize) -> io::Result<Vec<Value>> {
        let mut refs = Vec::new();
        for event in self.truth.read_events()?.into_iter().rev() {
            if event.event_type != "capability_receipt" {
                continue;
            }
            if event.data.get("capability_id").and_then(Value::as_str) != Some("source_set.create")
                || event.data.get("status").and_then(Value::as_str) != Some("success")
            {
                continue;
            }
            let Some(source_set_ref) = event
                .data
                .get("data")
                .and_then(|data| data.get("source_set_ref"))
                .and_then(Value::as_str)
                .or_else(|| event.data.get("source_set_ref").and_then(Value::as_str))
            else {
                continue;
            };
            refs.push(json!({
                "source_set_ref": source_set_ref,
                "event_id": event.event_id,
                "file_count": event.data.get("data")
                    .and_then(|data| data.get("file_count"))
                    .cloned()
                    .or_else(|| event.data.get("file_count").cloned()),
            }));
            if refs.len() >= limit {
                break;
            }
        }
        refs.reverse();
        Ok(refs)
    }

    fn record_provider_tool_protocol_error(
        &self,
        turn_id: &str,
        receipt: Option<&ModelCallReceipt>,
        tool_call: Option<&ProviderToolCall>,
        provider_tool_batch_id: Option<&str>,
        provider_tool_call_index: Option<usize>,
        error: &ProviderToolProtocolError,
    ) -> io::Result<()> {
        self.truth.append_event(
            Some(&self.token.pid),
            "provider_tool_protocol_error",
            json!({
                "runtime_id": self.runtime_id,
                "turn_id": turn_id,
                "model_call_id": receipt.map(|item| item.model_call_id.clone()),
                "provider_tool_call_id": tool_call.map(|item| item.id.clone()).or_else(|| error.provider_tool_call_id.clone()),
                "provider_tool_call_index": provider_tool_call_index,
                "provider_tool_batch_id": provider_tool_batch_id,
                "provider_tool_name": error.provider_tool_name,
                "capability_id": error.capability_id,
                "error_code": error.error_code,
                "message": error.message,
                "blocked": true,
            }),
        )?;
        Ok(())
    }

    fn record_provider_tool_loop_budget_exceeded(
        &self,
        turn_id: &str,
        receipt: &ModelCallReceipt,
        provider_tool_batch_id: &str,
        budget_kind: &str,
        requested_tool_calls: usize,
        limit: usize,
        executed_tool_calls_before: usize,
    ) -> io::Result<()> {
        self.truth.append_event(
            Some(&self.token.pid),
            "provider_tool_loop_budget_exceeded",
            json!({
                "runtime_id": self.runtime_id,
                "turn_id": turn_id,
                "model_call_id": receipt.model_call_id,
                "provider_tool_batch_id": provider_tool_batch_id,
                "budget_kind": budget_kind,
                "error_code": "MODEL_TOOL_LOOP_BUDGET_EXCEEDED",
                "requested_tool_calls": requested_tool_calls,
                "limit": limit,
                "executed_tool_calls_before": executed_tool_calls_before,
                "tool_call_count": receipt.provider_tool_calls.len(),
                "blocked": true,
            }),
        )?;
        Ok(())
    }

    fn record_provider_tool_loop_budget_exceeded_without_receipt(
        &self,
        turn_id: &str,
        budget_kind: &str,
        requested: usize,
        limit: usize,
        executed_tool_calls_before: usize,
    ) -> io::Result<()> {
        self.truth.append_event(
            Some(&self.token.pid),
            "provider_tool_loop_budget_exceeded",
            json!({
                "runtime_id": self.runtime_id,
                "turn_id": turn_id,
                "model_call_id": Value::Null,
                "provider_tool_batch_id": Value::Null,
                "budget_kind": budget_kind,
                "error_code": "MODEL_TOOL_LOOP_BUDGET_EXCEEDED",
                "requested_tool_calls": requested,
                "limit": limit,
                "executed_tool_calls_before": executed_tool_calls_before,
                "blocked": true,
            }),
        )?;
        Ok(())
    }

    fn append_provider_tool_error_result(
        &self,
        provider: &str,
        protocol: &str,
        tool_call: &ProviderToolCall,
        error: &ProviderToolProtocolError,
        provider_tool_batch_id: Option<&str>,
        provider_tool_call_index: Option<usize>,
    ) -> io::Result<()> {
        record_provider_tool_result_with_metadata(
            &self.truth,
            &self.token.pid,
            provider,
            protocol,
            &tool_call.id,
            &json!({
                "status": "blocked",
                "error_code": error.error_code,
                "message": error.message,
                "provider_tool_name": error.provider_tool_name,
                "capability_id": error.capability_id,
            }),
            ProviderToolResultMetadata {
                provider_tool_call_index,
                provider_tool_batch_id: provider_tool_batch_id.map(ToString::to_string),
            },
        )
        .map(|_| ())
    }

    fn append_provider_tool_recoverable_error_result(
        &self,
        provider: &str,
        protocol: &str,
        tool_call: &ProviderToolCall,
        recovery: &ProviderNativeRecoverableActionError,
        provider_tool_batch_id: Option<&str>,
        provider_tool_call_index: Option<usize>,
    ) -> io::Result<()> {
        let mut result = json!({
            "status": "blocked",
            "receipt_status": "blocked",
            "error_code": recovery.protocol_error.error_code,
            "message": recovery.protocol_error.message,
            "provider_tool_name": recovery.protocol_error.provider_tool_name,
            "capability_id": recovery.protocol_error.capability_id,
            "recoverable": true,
            "corrective_control_message": recovery.corrective_message,
        });
        merge_json_object(&mut result, &recovery.tool_result_extra);
        record_provider_tool_result_with_metadata(
            &self.truth,
            &self.token.pid,
            provider,
            protocol,
            &tool_call.id,
            &result,
            ProviderToolResultMetadata {
                provider_tool_call_index,
                provider_tool_batch_id: provider_tool_batch_id.map(ToString::to_string),
            },
        )
        .map(|_| ())
    }

    fn append_provider_tool_error_results_for_batch(
        &self,
        provider: &str,
        protocol: &str,
        tool_calls: &[ProviderToolCall],
        error: &ProviderToolProtocolError,
        provider_tool_batch_id: &str,
    ) -> io::Result<()> {
        self.append_provider_tool_error_results_for_remaining(
            provider,
            protocol,
            tool_calls,
            0,
            error,
            provider_tool_batch_id,
        )
    }

    fn append_provider_tool_error_results_for_remaining(
        &self,
        provider: &str,
        protocol: &str,
        tool_calls: &[ProviderToolCall],
        start_index: usize,
        error: &ProviderToolProtocolError,
        provider_tool_batch_id: &str,
    ) -> io::Result<()> {
        for (index, tool_call) in tool_calls.iter().enumerate().skip(start_index) {
            let mut tool_error = error.clone();
            tool_error.provider_tool_call_id = Some(tool_call.id.clone());
            if tool_error.provider_tool_name.is_none() {
                tool_error.provider_tool_name = provider_tool_call_name(tool_call).ok();
            }
            self.append_provider_tool_error_result(
                provider,
                protocol,
                tool_call,
                &tool_error,
                Some(provider_tool_batch_id),
                Some(index),
            )?;
        }
        Ok(())
    }

    fn provider_tool_protocol_interrupted(
        &self,
        turn_id: &str,
        error_code: &str,
        message: &str,
        model_output_ref: Option<String>,
        error: Option<ProviderToolProtocolError>,
    ) -> io::Result<(NextActionDecision, String)> {
        let mut decision = self.model_protocol_interrupted_decision(
            error_code,
            message,
            model_output_ref,
            Vec::new(),
            Vec::new(),
        );
        if let Some(error) = error {
            decision.output_spec["provider_tool_error"] = to_json_value(&error)?;
        }
        self.record_provider_tool_protocol_error(
            turn_id,
            None,
            None,
            None,
            None,
            &ProviderToolProtocolError {
                error_code: error_code.to_string(),
                message: message.to_string(),
                provider_tool_name: None,
                provider_tool_call_id: None,
                capability_id: None,
            },
        )?;
        let status = self.interrupt_by_model_protocol_error(turn_id, &decision)?;
        Ok((decision, status))
    }

    fn capability_descriptor(&self, capability_id: &str) -> io::Result<&CapabilityDescriptor> {
        self.registry
            .iter()
            .find(|descriptor| descriptor.capability_id == capability_id)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("capability descriptor not found: {capability_id}"),
                )
            })
    }

    fn record_kernel_blocked_capability(&self, receipt: &CapabilityReceipt) -> io::Result<()> {
        self.truth.append_event(
            Some(&self.token.pid),
            "capability_blocked",
            to_json_value(receipt)?,
        )?;
        self.truth.append_event(
            Some(&self.token.pid),
            "capability_receipt",
            to_json_value(receipt)?,
        )?;
        Ok(())
    }

    fn finalize_registered_capability_approval(
        &self,
        guard: Option<&CapabilityApprovalGuard>,
        receipt: &CapabilityReceipt,
    ) -> io::Result<()> {
        finalize_capability_approval(&self.truth, &self.token.pid, guard, receipt)
    }

    fn read_typed_ref(&self, decision: &NextActionDecision) -> io::Result<CapabilityReceipt> {
        let target_ref = decision
            .output_spec
            .get("ref")
            .and_then(Value::as_str)
            .or_else(|| decision.output_spec.get("path").and_then(Value::as_str))
            .or_else(|| decision.input_refs.first().map(String::as_str))
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "ref missing"))?;
        let Ok((content, source_kind)) = self.read_ref_text(target_ref) else {
            return Ok(self.process_capability_receipt(
                "process.read_ref",
                "blocked",
                json!({
                    "ref": target_ref,
                    "reason": "unsupported ref scheme for process.read_ref",
                }),
            )?);
        };
        let content_ref = self.truth.write_blob(
            &format!(
                "process_reads/{}_{}.txt",
                safe_blob_name(&self.runtime_id),
                now_ms()
            ),
            content.as_bytes(),
        )?;
        self.process_capability_receipt(
            "process.read_ref",
            "success",
            json!({
                "ref": target_ref,
                "source_kind": source_kind,
                "content_ref": content_ref,
                "content_preview": content.chars().take(8192).collect::<String>(),
                "bytes": content.len(),
            }),
        )
    }

    fn page_tool_result(&self, decision: &NextActionDecision) -> io::Result<CapabilityReceipt> {
        let target_ref = raw_result_ref_arg(decision)?;
        let (content, source_kind) = self.read_ref_text(&target_ref)?;
        let offset = decision
            .output_spec
            .get("offset")
            .and_then(Value::as_u64)
            .unwrap_or(0) as usize;
        let limit = decision
            .output_spec
            .get("limit_bytes")
            .or_else(|| decision.output_spec.get("limit"))
            .and_then(Value::as_u64)
            .unwrap_or(16 * 1024) as usize;
        let total_chars = content.chars().count();
        let page = content.chars().skip(offset).take(limit).collect::<String>();
        let page_ref = self.truth.write_blob(
            &format!(
                "tool_result_pages/{}_{}_{}.txt",
                safe_blob_name(&self.runtime_id),
                offset,
                now_ms()
            ),
            page.as_bytes(),
        )?;
        self.process_capability_receipt(
            "tool.result.page",
            "success",
            json!({
                "ref": target_ref,
                "source_kind": source_kind,
                "offset": offset,
                "limit": limit,
                "total_chars": total_chars,
                "has_more": offset + page.chars().count() < total_chars,
                "page_ref": page_ref,
                "page": page,
            }),
        )
    }

    fn search_tool_result(&self, decision: &NextActionDecision) -> io::Result<CapabilityReceipt> {
        let target_ref = raw_result_ref_arg(decision)?;
        let query = string_arg(decision, "query")?;
        let max_matches = decision
            .output_spec
            .get("max_matches")
            .and_then(Value::as_u64)
            .unwrap_or(20) as usize;
        let (content, source_kind) = self.read_ref_text(&target_ref)?;
        let mut matches = Vec::new();
        for (index, line) in content.lines().enumerate() {
            if line.contains(&query) {
                matches.push(json!({
                    "line_number": index + 1,
                    "line": line,
                }));
                if matches.len() >= max_matches {
                    break;
                }
            }
        }
        self.process_capability_receipt(
            "tool.result.search",
            "success",
            json!({
                "ref": target_ref,
                "source_kind": source_kind,
                "query": query,
                "match_count": matches.len(),
                "matches": matches,
            }),
        )
    }

    fn inspect_tool_result_schema(
        &self,
        decision: &NextActionDecision,
    ) -> io::Result<CapabilityReceipt> {
        let target_ref = raw_result_ref_arg(decision)?;
        let (content, source_kind) = self.read_ref_text(&target_ref)?;
        let parsed = serde_json::from_str::<Value>(&content).ok();
        let schema = parsed
            .as_ref()
            .map(inspect_json_shape)
            .unwrap_or_else(|| json!({"type": "text", "bytes": content.len()}));
        self.process_capability_receipt(
            "tool.result.inspect_schema",
            "success",
            json!({
                "ref": target_ref,
                "source_kind": source_kind,
                "schema": schema,
            }),
        )
    }

    fn read_ref_text(&self, target_ref: &str) -> io::Result<(String, &'static str)> {
        if target_ref.starts_with("blob://") {
            Ok((
                std::fs::read_to_string(self.truth.resolve_blob_ref(target_ref)?)?,
                "blob",
            ))
        } else if let Some(path) = target_ref.strip_prefix("artifact_ref://") {
            let artifact_path = self
                .guard
                .resolve_workspace_path(path)
                .map_err(|err| io::Error::new(io::ErrorKind::PermissionDenied, err))?;
            Ok((std::fs::read_to_string(artifact_path)?, "artifact"))
        } else if let Some(path) = target_ref.strip_prefix("artifact://") {
            let artifact_path = self
                .guard
                .resolve_workspace_path(path)
                .map_err(|err| io::Error::new(io::ErrorKind::PermissionDenied, err))?;
            Ok((std::fs::read_to_string(artifact_path)?, "artifact"))
        } else if target_ref.starts_with("chat_blob://") {
            Ok((
                crate::ChatTruthStore::new_with_state_root(
                    self.truth.workspace_root(),
                    self.truth.state_root(),
                )?
                .read_chat_blob_text(target_ref)?,
                "chat_blob",
            ))
        } else if target_ref.starts_with("chat://")
            || target_ref.starts_with("chat_thread://")
            || target_ref.starts_with("chat_turn://")
            || target_ref.starts_with("chat_")
        {
            Ok((
                crate::ChatTruthStore::new_with_state_root(
                    self.truth.workspace_root(),
                    self.truth.state_root(),
                )?
                .read_chat_ref_text(target_ref)?,
                "chat_truth",
            ))
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "unsupported ref scheme",
            ))
        }
    }

    fn query_process_events(&self, decision: &NextActionDecision) -> io::Result<CapabilityReceipt> {
        let limit = decision
            .output_spec
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(80) as usize;
        let event_type_filter = decision
            .output_spec
            .get("event_type")
            .and_then(Value::as_str)
            .map(str::to_string);
        let mut events = self.truth.read_events()?;
        if let Some(filter) = event_type_filter.as_deref() {
            events.retain(|event| event.event_type == filter);
        }
        let start = events.len().saturating_sub(limit);
        let selected = events[start..].to_vec();
        let events_ref = self.truth.write_blob(
            &format!(
                "process_reads/{}_events_{}.json",
                safe_blob_name(&self.runtime_id),
                now_ms()
            ),
            &serde_json::to_vec_pretty(&selected).map_err(crate::json_err)?,
        )?;
        self.process_capability_receipt(
            "process.query_events",
            "success",
            json!({
                "events_ref": events_ref,
                "event_count": selected.len(),
                "event_type": event_type_filter,
            }),
        )
    }

    fn provider_toolset_select_receipt(
        &self,
        decision: &NextActionDecision,
    ) -> io::Result<CapabilityReceipt> {
        let known_groups = crate::provider_toolset::provider_tool_group_descriptors()
            .into_iter()
            .map(|group| group.group_id)
            .collect::<BTreeSet<_>>();
        let requested_groups = string_array_arg(decision, "requested_groups")
            .into_iter()
            .take(4)
            .collect::<Vec<_>>();
        let accepted_groups = requested_groups
            .iter()
            .filter(|group| known_groups.contains(group.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        let rejected_groups = requested_groups
            .iter()
            .filter(|group| !known_groups.contains(group.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        let registered_capabilities = self
            .registry
            .iter()
            .map(|descriptor| descriptor.capability_id.clone())
            .collect::<BTreeSet<_>>();
        let requested_capability_ids = string_array_arg(decision, "required_capabilities")
            .into_iter()
            .take(8)
            .collect::<Vec<_>>();
        let accepted_capability_ids = requested_capability_ids
            .iter()
            .filter(|capability_id| registered_capabilities.contains(capability_id.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        let rejected_capability_ids = requested_capability_ids
            .iter()
            .filter(|capability_id| !registered_capabilities.contains(capability_id.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        let ttl_model_calls = decision
            .output_spec
            .get("ttl_model_calls")
            .and_then(Value::as_u64)
            .unwrap_or(4)
            .clamp(1, 6);
        let selection_id = format!("toolset_sel_{}", now_ms());
        let next_intent = decision
            .output_spec
            .get("next_intent")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        self.truth.append_event(
            Some(&self.token.pid),
            "provider_toolset_selection_recorded",
            json!({
                "runtime_id": self.runtime_id,
                "selection_id": selection_id,
                "requested_groups": requested_groups.clone(),
                "accepted_groups": accepted_groups.clone(),
                "rejected_groups": rejected_groups.clone(),
                "requested_capability_ids": requested_capability_ids.clone(),
                "accepted_capability_ids": accepted_capability_ids.clone(),
                "rejected_capability_ids": rejected_capability_ids.clone(),
                "next_intent": next_intent,
                "ttl_model_calls": ttl_model_calls,
                "reason": decision.reason.clone(),
            }),
        )?;
        self.process_capability_receipt(
            "process.toolset.select",
            "success",
            json!({
                "selection_id": selection_id,
                "accepted_groups": accepted_groups,
                "rejected_groups": rejected_groups,
                "accepted_capability_ids": accepted_capability_ids,
                "rejected_capability_ids": rejected_capability_ids,
                "ttl_model_calls": ttl_model_calls,
                "status_note": "Selected provider tool groups will be considered when building the next DeepSeek provider request. This selection does not authorize execution.",
            }),
        )
    }

    fn pending_approvals(&self) -> io::Result<CapabilityReceipt> {
        let state = self.task_context_state()?;
        let pending_preview_ids = state
            .preview_tx_table
            .iter()
            .filter(|item| item.status == "created")
            .map(|item| item.preview_id.clone())
            .collect::<Vec<_>>();
        let active_approval_token_ids = state
            .approval_token_table
            .iter()
            .filter(|item| item.status == "active")
            .map(|item| item.approval_token_id.clone())
            .collect::<Vec<_>>();
        self.process_capability_receipt(
            "process.pending_approvals",
            "success",
            json!({
                "pending_preview_ids": pending_preview_ids,
                "active_approval_token_ids": active_approval_token_ids,
                "preview_tx_table": state.preview_tx_table,
                "approval_token_table": state.approval_token_table,
                "pending_user_decision": state.pending_user_decision,
                "current_turn_index": state.current_turn_index,
                "next_turn_index": state.next_turn_index,
                "runtime_note": "This is the task context tx/token table, not a strategy recommendation. TaskAgent decides how to continue.",
            }),
        )
    }

    fn create_preview_capability_receipt(
        &self,
        _reasoning_step_id: &str,
        decision: &NextActionDecision,
    ) -> io::Result<CapabilityReceipt> {
        let executable_operations = self.preview_operations_from_decision(decision).ok();
        self.process_capability_receipt(
            "process.preview.create",
            "success",
            json!({
                "preview_disabled": true,
                "no_preview_created": true,
                "approval_required": false,
                "waiting_for_approval": false,
                "executable_operations": executable_operations,
                "runtime_note": "Preview/approve/reject/edit blocking flow is disabled for RC0 run-through. Execute the intended capability directly under Kernel hard boundaries and receipts.",
            }),
        )
    }

    fn preview_operations_from_decision(
        &self,
        decision: &NextActionDecision,
    ) -> io::Result<Vec<ExecutablePreviewOperation>> {
        if let Some(items) = decision
            .output_spec
            .get("operations")
            .and_then(Value::as_array)
        {
            let mut operations = Vec::new();
            for (index, item) in items.iter().enumerate() {
                let capability_id = item
                    .get("capability_id")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty() && *value != "*")
                    .ok_or_else(|| {
                        io::Error::new(
                            io::ErrorKind::InvalidInput,
                            format!("operations[{index}].capability_id missing"),
                        )
                    })?;
                self.capability_descriptor(capability_id)?;
                let operation_paths = item
                    .get("target_paths")
                    .and_then(Value::as_array)
                    .map(|paths| {
                        paths
                            .iter()
                            .filter_map(Value::as_str)
                            .map(str::to_string)
                            .collect::<Vec<_>>()
                    })
                    .filter(|paths| !paths.is_empty())
                    .unwrap_or_else(|| string_array_arg(decision, "target_paths"));
                let target_paths = expand_preview_target_paths_for_actions(
                    &[capability_id.to_string()],
                    operation_paths,
                );
                if target_paths.is_empty() || target_paths.iter().any(|item| item.trim() == "*") {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!("operations[{index}].target_paths missing or wildcard"),
                    ));
                }
                let human_description = item
                    .get("human_description")
                    .or_else(|| item.get("description"))
                    .or_else(|| decision.output_spec.get("human_description"))
                    .and_then(Value::as_str)
                    .unwrap_or(capability_id)
                    .to_string();
                let arguments = item
                    .get("arguments")
                    .cloned()
                    .or_else(|| decision.output_spec.get("arguments").cloned())
                    .unwrap_or_else(|| json!({}));
                let rollback_policy = item
                    .get("rollback_policy")
                    .or_else(|| decision.output_spec.get("rollback_policy"))
                    .and_then(Value::as_str)
                    .map(ToString::to_string);
                operations.push(ExecutablePreviewOperation {
                    capability_id: capability_id.to_string(),
                    arguments,
                    target_paths,
                    human_description,
                    rollback_policy,
                });
            }
            if !operations.is_empty() {
                return Ok(operations);
            }
        }

        let mut proposed_actions = string_array_arg(decision, "capability_ids");
        if proposed_actions.is_empty() {
            if let Some(capability_id) = decision
                .output_spec
                .get("capability_id")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
            {
                proposed_actions.push(capability_id.to_string());
            }
        }
        if proposed_actions.is_empty() {
            proposed_actions = string_array_arg(decision, "proposed_actions");
        }
        let target_paths = expand_preview_target_paths_for_actions(
            &proposed_actions,
            string_array_arg(decision, "target_paths"),
        );
        let operations = executable_preview_operations_from_scope(
            &proposed_actions,
            &target_paths,
            decision
                .output_spec
                .get("human_description")
                .or_else(|| decision.output_spec.get("description"))
                .and_then(Value::as_str)
                .map(ToString::to_string),
            decision
                .output_spec
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({})),
            decision
                .output_spec
                .get("rollback_policy")
                .and_then(Value::as_str)
                .map(ToString::to_string),
        );
        if operations.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "preview tx requires operations[].capability_id or exact canonical capability_ids; natural-language proposed_actions belong in human_description",
            ));
        }
        for operation in &operations {
            self.capability_descriptor(&operation.capability_id)?;
        }
        Ok(operations)
    }

    fn process_capability_receipt(
        &self,
        capability_id: &str,
        status: &str,
        data: Value,
    ) -> io::Result<CapabilityReceipt> {
        let receipt = CapabilityReceipt {
            capability_id: capability_id.to_string(),
            job_id: self.token.job_id.clone(),
            pid: self.token.pid.clone(),
            status: status.to_string(),
            data,
        };
        self.truth.append_event(
            Some(&self.token.pid),
            "capability_receipt",
            to_json_value(&receipt)?,
        )?;
        Ok(receipt)
    }

    fn run_model_artifact_audit_capability(
        &self,
        goal: &str,
        turn_id: &str,
        action_id: &str,
        decision: &NextActionDecision,
    ) -> io::Result<CapabilityReceipt> {
        let (audit_scope, included_artifacts) = model_audit_scope_and_artifacts(decision)?;
        let primary_artifact_path = included_artifacts.first().cloned().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "included artifact missing")
        })?;
        let provider = self.model_provider.clone().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "Task Agent has no ModelProvider for artifact audit capability",
            )
        })?;
        let mut artifact_inputs = Vec::new();
        let mut artifact_text_refs = Vec::new();
        for artifact_path in &included_artifacts {
            let (artifact_text, artifact_kind) = match self
                .read_artifact_text_for_model_audit(artifact_path)
            {
                Ok(value) => value,
                Err(err) => {
                    return self.emit_artifact_model_audit_receipt(
                        &primary_artifact_path,
                        &audit_scope,
                        &included_artifacts,
                        "failed",
                        json!({
                            "artifact_path": primary_artifact_path.replace('\\', "/"),
                            "audit_scope": audit_scope,
                            "included_artifacts": included_artifacts,
                            "unreadable_artifact_path": artifact_path.replace('\\', "/"),
                            "error": {"code": "ARTIFACT_AUDIT_INPUT_UNREADABLE", "message": err.to_string()},
                            "runtime_note": "model-backed artifact audit could not read an explicitly included user-visible artifact; TaskAgent must decide whether to revise, choose another artifact, or fail",
                        }),
                    );
                }
            };
            let artifact_text_ref = self.truth.write_blob(
                &format!(
                    "artifact_audit_inputs/{}_{}_artifact_text.txt",
                    safe_blob_name(turn_id),
                    safe_blob_name(artifact_path)
                ),
                artifact_text.as_bytes(),
            )?;
            let artifact_char_count = artifact_text.chars().count();
            artifact_text_refs.push(artifact_text_ref.clone());
            artifact_inputs.push(json!({
                "path": artifact_path.replace('\\', "/"),
                "kind": artifact_kind,
                "char_count": artifact_char_count,
                "artifact_text_ref": artifact_text_ref,
            }));
        }
        let artifact_text_ref = artifact_text_refs
            .first()
            .cloned()
            .unwrap_or_else(|| "".to_string());
        let user_goal_ref = self.truth.write_blob(
            &format!(
                "artifact_audit_inputs/{}_{}_user_goal.txt",
                safe_blob_name(turn_id),
                safe_blob_name(&primary_artifact_path)
            ),
            goal.as_bytes(),
        )?;
        let evidence = self.artifact_audit_evidence(
            goal,
            &user_goal_ref,
            &audit_scope,
            &included_artifacts,
            &artifact_inputs,
            decision,
        )?;
        let evidence_ref = self.truth.write_blob(
            &format!(
                "artifact_audit_inputs/{}_{}_evidence.json",
                safe_blob_name(turn_id),
                safe_blob_name(&primary_artifact_path)
            ),
            serde_json::to_string_pretty(&evidence)
                .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?
                .as_bytes(),
        )?;
        let instruction = artifact_model_audit_instruction();
        let instruction_ref = self.truth.write_blob(
            &format!(
                "model_inputs/{}_artifact_model_audit_instruction.txt",
                safe_blob_name(turn_id)
            ),
            instruction.as_bytes(),
        )?;
        let mut input_refs = vec![user_goal_ref.clone(), evidence_ref.clone()];
        input_refs.extend(artifact_text_refs.clone());
        input_refs.extend(model_audit_extra_input_refs(decision));
        input_refs.sort();
        input_refs.dedup();
        let operation = ModelOperation::Audit;
        let model_action = ModelAction {
            action_id: action_id.to_string(),
            job_id: self.token.job_id.clone(),
            pid: self.token.pid.clone(),
            reasoning_step_id: turn_id.to_string(),
            operation: operation.clone(),
            instruction_ref,
            input_refs,
            preference_snapshot_ref: None,
            output_schema: artifact_model_audit_output_schema(),
            provider: provider.provider_name().to_string(),
            model: provider.model_name_for_operation(&operation),
            budget: crate::ModelContextProfile::for_provider(provider.as_ref(), &operation)
                .budget_for(&operation),
            failure_policy: ModelFailurePolicy::FailClosed,
            required: true,
        };
        self.truth.append_event(
            Some(&self.token.pid),
            "model_action_emitted",
            to_json_value(&model_action)?,
        )?;
        self.truth.append_event(
            Some(&self.token.pid),
            "artifact_model_audit_started",
            json!({
                "auditor_agent": "model_audit_agent",
                "capability_id": decision.capability_id,
                "artifact_path": primary_artifact_path.replace('\\', "/"),
                "audit_scope": audit_scope,
                "included_artifacts": included_artifacts.clone(),
                "artifact_text_ref": artifact_text_ref,
                "artifact_text_refs": artifact_text_refs.clone(),
                "user_goal_ref": user_goal_ref,
                "audit_evidence_ref": evidence_ref,
                "runtime_note": "model_audit_agent reviews only explicitly included artifact content and returns findings only; TaskAgent remains responsible for task strategy and completion judgment",
            }),
        )?;
        let model_receipt = ModelRuntime::new(self.truth.clone(), self.token.clone(), provider)
            .with_model_invocation_config(
                self.model_config.clone(),
                self.model_invocation_config_ref.clone(),
            )
            .audit(model_action)?;
        let mut data = json!({
            "artifact_path": primary_artifact_path.replace('\\', "/"),
            "audit_scope": audit_scope,
            "included_artifacts": included_artifacts.clone(),
            "model_call_id": model_receipt.model_call_id,
            "model_output_ref": model_receipt.output_ref,
            "user_goal_ref": user_goal_ref,
            "artifact_text_ref": artifact_text_ref,
            "artifact_text_refs": artifact_text_refs.clone(),
            "audit_evidence_ref": evidence_ref,
            "auditor_agent": "model_audit_agent",
            "model_receipt_status": model_receipt.status,
            "model_error": model_receipt.error,
            "runtime_note": "model-backed artifact audit returns findings only; TaskAgent decides whether to revise, gather more evidence, clarify, fail, or complete",
        });
        let mut wrapper_status = model_receipt.status.clone();
        if model_receipt.status == "success" {
            let Some(output_ref) = data.get("model_output_ref").and_then(Value::as_str) else {
                wrapper_status = "failed".to_string();
                data["schema_errors"] = json!(["model audit succeeded without output_ref"]);
                return self.emit_artifact_model_audit_receipt(
                    &primary_artifact_path,
                    &audit_scope,
                    &included_artifacts,
                    &wrapper_status,
                    data,
                );
            };
            let output_text = self.read_blob_text(output_ref)?;
            match serde_json::from_str::<Value>(&output_text) {
                Ok(audit_output) => {
                    let schema_errors = validate_artifact_model_audit_output(
                        &audit_output,
                        &audit_scope,
                        &included_artifacts,
                    );
                    data["audit_output"] = audit_output.clone();
                    data["audit_scope"] = audit_output
                        .get("audit_scope")
                        .cloned()
                        .unwrap_or_else(|| json!(audit_scope));
                    data["included_artifacts"] = audit_output
                        .get("included_artifacts")
                        .cloned()
                        .unwrap_or_else(|| json!(included_artifacts.clone()));
                    data["auditor_limitations"] = audit_output
                        .get("auditor_limitations")
                        .cloned()
                        .unwrap_or_else(|| json!([]));
                    data["quality_pass"] = audit_output
                        .get("quality_pass")
                        .and_then(Value::as_bool)
                        .map(Value::Bool)
                        .unwrap_or(Value::Null);
                    data["human_acceptance_pass"] = audit_output
                        .get("human_acceptance_pass")
                        .and_then(Value::as_bool)
                        .map(Value::Bool)
                        .unwrap_or(Value::Null);
                    data["blocking_issue_count"] = json!(audit_output
                        .get("blocking_issues")
                        .and_then(Value::as_array)
                        .map(|items| items.len())
                        .unwrap_or(0));
                    data["finding_count"] = json!(audit_output
                        .get("findings")
                        .and_then(Value::as_array)
                        .map(|items| items.len())
                        .unwrap_or(0));
                    data["schema_errors"] = json!(schema_errors);
                    if !data["schema_errors"]
                        .as_array()
                        .is_some_and(|items| items.is_empty())
                    {
                        wrapper_status = "failed".to_string();
                    }
                }
                Err(err) => {
                    wrapper_status = "failed".to_string();
                    data["schema_errors"] =
                        json!([format!("model audit output is not JSON: {err}")]);
                }
            }
        }
        self.emit_artifact_model_audit_receipt(
            &primary_artifact_path,
            &audit_scope,
            &included_artifacts,
            &wrapper_status,
            data,
        )
    }

    fn emit_artifact_model_audit_receipt(
        &self,
        artifact_path: &str,
        audit_scope: &str,
        included_artifacts: &[String],
        status: &str,
        mut data: Value,
    ) -> io::Result<CapabilityReceipt> {
        data["artifact_path"] = json!(artifact_path.replace('\\', "/"));
        data["audit_scope"] = data
            .get("audit_scope")
            .cloned()
            .unwrap_or_else(|| json!(audit_scope));
        data["included_artifacts"] = data
            .get("included_artifacts")
            .cloned()
            .unwrap_or_else(|| json!(included_artifacts));
        data["auditor_agent"] = json!("model_audit_agent");
        data["semantic_audit_evidence_only"] = json!(true);
        data["runtime_decision_policy"] = json!(
            "Kernel records semantic audit evidence but does not decide task strategy from model findings"
        );
        let receipt = CapabilityReceipt {
            capability_id: "model.audit_artifact_quality".to_string(),
            job_id: self.token.job_id.clone(),
            pid: self.token.pid.clone(),
            status: status.to_string(),
            data,
        };
        self.truth.append_event(
            Some(&self.token.pid),
            "artifact_model_audit_receipt",
            to_json_value(&receipt)?,
        )?;
        self.truth.append_event(
            Some(&self.token.pid),
            "capability_receipt",
            to_json_value(&receipt)?,
        )?;
        Ok(receipt)
    }

    fn read_artifact_text_for_model_audit(
        &self,
        artifact_path: &str,
    ) -> io::Result<(String, &'static str)> {
        let artifact = self
            .guard
            .resolve_workspace_path(artifact_path)
            .map_err(|err| io::Error::new(io::ErrorKind::PermissionDenied, err))?;
        if !artifact.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "artifact path is not a readable file",
            ));
        }
        if artifact_path.to_ascii_lowercase().ends_with(".docx") {
            let receipt = self.office_runtime().read_text(artifact_path)?;
            if receipt.status != "success" {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "office.docx.read_text failed while preparing artifact audit",
                ));
            }
            let Some(content_ref) = receipt.data.get("content_ref").and_then(Value::as_str) else {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "office.docx.read_text did not return content_ref",
                ));
            };
            let text = fs::read_to_string(self.truth.resolve_blob_ref(content_ref)?)?;
            return Ok((text, "docx"));
        }
        Ok((fs::read_to_string(artifact)?, "text"))
    }

    fn artifact_audit_evidence(
        &self,
        goal: &str,
        user_goal_ref: &str,
        audit_scope: &str,
        included_artifacts: &[String],
        artifact_inputs: &[Value],
        decision: &NextActionDecision,
    ) -> io::Result<Value> {
        let normalized_paths = included_artifacts
            .iter()
            .map(|path| path.replace('\\', "/"))
            .collect::<Vec<_>>();
        let related_receipts = self
            .truth
            .read_events()?
            .into_iter()
            .filter_map(|event| {
                let event_id = event.event_id;
                let event_type = event.event_type;
                let payloads = task_event_payloads(&event.data);
                let matching = payloads
                    .into_iter()
                    .filter(|payload| {
                        payload_path(payload)
                            .map(|path| normalized_paths.contains(&path.replace('\\', "/")))
                            .unwrap_or(false)
                    })
                    .map(|payload| {
                        json!({
                            "event_id": event_id,
                            "event_type": event_type.clone(),
                            "capability_id": payload.get("capability_id").and_then(Value::as_str),
                            "status": payload.get("status").and_then(Value::as_str),
                            "payload": payload,
                        })
                    })
                    .collect::<Vec<_>>();
                if matching.is_empty() {
                    None
                } else {
                    Some(matching)
                }
            })
            .flatten()
            .collect::<Vec<_>>();
        Ok(json!({
            "audit_kind": "model_backed_artifact_quality_findings",
            "audit_scope": audit_scope,
            "included_artifacts": normalized_paths,
            "user_goal": goal,
            "user_goal_ref": user_goal_ref,
            "artifacts": artifact_inputs,
            "source_refs_declared_by_agent": declared_audit_source_refs(decision),
            "source_set_ref": decision.output_spec.get("source_set_ref").and_then(Value::as_str),
            "dataset_ref": decision.output_spec.get("dataset_ref").and_then(Value::as_str),
            "raw_document_set_ref": decision.output_spec.get("raw_document_set_ref").and_then(Value::as_str),
            "related_runtime_receipts": related_receipts,
            "audit_rules": [
                "Review only the explicitly included artifact content.",
                "Do not claim that sibling artifacts or other task deliverables are missing unless they are explicitly included in included_artifacts.",
                "Do not decide whether the task is complete.",
                "Judge deliverability of the included content from a real human user's perspective.",
                "Check whether the included artifact content answers the user goal within this audit scope.",
                "Check source grounding and factual risk from provided evidence only.",
                "Identify clarity, completeness, format, tone, JSON wrapper leakage, internal ref leakage, checksum, and DOCX rewrite quality issues.",
                "Return findings only; do not decide task completion or rewrite the artifact."
            ],
        }))
    }

    fn run_model_runtime_capability(
        &self,
        turn_id: &str,
        action_id: &str,
        decision: &NextActionDecision,
    ) -> io::Result<CapabilityReceipt> {
        let operation = match decision.capability_id.as_str() {
            "model.extract_json" | "model.extract_dataset" => ModelOperation::ExtractJson,
            "model.summarize" | "model.summarize_dataset" => ModelOperation::Summarize,
            "model.rewrite" => ModelOperation::Rewrite,
            "model.generate_artifact" | "model.synthesize_artifact_from_dataset" => {
                ModelOperation::GenerateArtifact
            }
            "model.audit" => ModelOperation::Audit,
            other => {
                return Ok(CapabilityReceipt {
                    capability_id: other.to_string(),
                    job_id: self.token.job_id.clone(),
                    pid: self.token.pid.clone(),
                    status: "failed".to_string(),
                    data: json!({"reason": "unsupported model capability", "capability_id": other}),
                })
            }
        };
        let provider = self.model_provider.clone().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "Task Agent has no ModelProvider for model capability",
            )
        })?;
        let instruction = decision
            .output_spec
            .get("instruction")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| {
                format!(
                    "Execute {} for the provided input refs.",
                    operation.as_str()
                )
            });
        let instruction_ref = self.truth.write_blob(
            &format!(
                "model_inputs/{}_{}_instruction.txt",
                safe_blob_name(turn_id),
                safe_blob_name(operation.as_str())
            ),
            instruction.as_bytes(),
        )?;
        let mut input_refs = decision.input_refs.clone();
        if input_refs.is_empty() {
            input_refs = decision
                .output_spec
                .get("source_refs")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
        }
        let output_schema = decision
            .output_spec
            .get("output_schema")
            .cloned()
            .unwrap_or_else(|| {
                if operation == ModelOperation::ExtractJson {
                    json!({"type": "object"})
                } else {
                    json!({"type": "string"})
                }
            });
        let model_action = ModelAction {
            action_id: action_id.to_string(),
            job_id: self.token.job_id.clone(),
            pid: self.token.pid.clone(),
            reasoning_step_id: turn_id.to_string(),
            operation: operation.clone(),
            instruction_ref,
            input_refs,
            preference_snapshot_ref: None,
            output_schema,
            provider: provider.provider_name().to_string(),
            model: provider.model_name_for_operation(&operation),
            budget: crate::ModelContextProfile::for_provider(provider.as_ref(), &operation)
                .budget_for(&operation),
            failure_policy: ModelFailurePolicy::FailClosed,
            required: true,
        };
        self.truth.append_event(
            Some(&self.token.pid),
            "model_action_emitted",
            to_json_value(&model_action)?,
        )?;
        let runtime = ModelRuntime::new(self.truth.clone(), self.token.clone(), provider)
            .with_model_invocation_config(
                self.model_config.clone(),
                self.model_invocation_config_ref.clone(),
            );
        let receipt = match operation {
            ModelOperation::ExtractJson => runtime.extract_json(model_action)?,
            ModelOperation::Summarize => runtime.summarize(model_action)?,
            ModelOperation::Rewrite => runtime.rewrite(model_action)?,
            ModelOperation::GenerateArtifact => runtime.generate_artifact(model_action)?,
            ModelOperation::Audit => runtime.audit(model_action)?,
            _ => unreachable!("unsupported model operation dispatch"),
        };
        let mut data = to_json_value(&receipt)?;
        if decision.capability_id == "model.extract_dataset" {
            if let Some(map) = data.as_object_mut() {
                if let Some(output_ref) = receipt.output_ref.clone() {
                    map.insert("dataset_ref".to_string(), json!(output_ref));
                }
                map.insert("candidate_dataset".to_string(), json!(true));
                map.insert("source_refs_required".to_string(), json!(true));
                map.insert("coverage_required".to_string(), json!(true));
            }
        }
        Ok(CapabilityReceipt {
            capability_id: decision.capability_id.clone(),
            job_id: self.token.job_id.clone(),
            pid: self.token.pid.clone(),
            status: receipt.status.clone(),
            data,
        })
    }

    fn office_runtime(&self) -> OfficeRuntime {
        OfficeRuntime::new(
            self.guard.clone(),
            self.truth.clone(),
            self.token.clone(),
            office_worker_project(),
        )
    }

    fn request_preview(
        &self,
        reasoning_step_id: &str,
        decision: &NextActionDecision,
    ) -> io::Result<String> {
        let receipt = self.request_preview_receipt(reasoning_step_id, decision)?;
        self.record_capability_execution(decision, &receipt)?;
        if receipt
            .data
            .get("waiting_for_approval")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            Ok("waiting_approval".to_string())
        } else {
            Ok("running".to_string())
        }
    }

    fn request_preview_receipt(
        &self,
        reasoning_step_id: &str,
        decision: &NextActionDecision,
    ) -> io::Result<CapabilityReceipt> {
        let executable_operations = self.preview_operations_from_decision(decision).ok();
        let action = self.action(
            reasoning_step_id,
            ProcessActionKind::RequestPreview,
            "process.request_preview",
            Vec::new(),
            json!({
                "preview_disabled": true,
                "no_preview_created": true,
                "approval_required": false,
                "waiting_for_approval": false,
                "executable_operations": executable_operations.clone(),
            }),
            &decision.reason,
        );
        self.emit_and_validate_action(&action)?;
        self.process_capability_receipt(
            "process.request_preview",
            "success",
            json!({
                "preview_disabled": true,
                "no_preview_created": true,
                "approval_required": false,
                "waiting_for_approval": false,
                "executable_operations": executable_operations,
                "runtime_note": "Preview/approve/reject/edit blocking flow is disabled for RC0 run-through. Execute the intended capability directly under Kernel hard boundaries and receipts.",
            }),
        )
    }

    fn clarify(&self, reasoning_step_id: &str, decision: &NextActionDecision) -> io::Result<()> {
        let action = self.action(
            reasoning_step_id,
            ProcessActionKind::Clarify,
            "process.clarify",
            Vec::new(),
            json!({"missing_fact": "explicit_paths_and_operation"}),
            &decision.reason,
        );
        self.emit_and_validate_action(&action)?;
        self.truth.update_job_status("waiting_user")?;
        self.truth.append_event(
            Some(&self.token.pid),
            "job_waiting_user",
            json!({
                "reason": decision.reason,
                "question": "Please provide explicit paths and the intended operation before this destructive task can continue.",
            }),
        )?;
        Ok(())
    }

    fn complete(
        &self,
        reasoning_step_id: &str,
        decision: &NextActionDecision,
    ) -> io::Result<String> {
        let replay = self.truth.replay()?;
        let claimed_artifacts = claimed_artifacts(decision, &replay.artifact_refs);
        let completion_statement = completion_statement(decision);
        let key_sources = string_or_array_values(decision.output_spec.get("key_sources"));
        let known_limitations =
            string_or_array_values(decision.output_spec.get("known_limitations"));
        let user_review_notes =
            string_or_array_values(decision.output_spec.get("user_review_notes"));
        let completion_statement_ref = self.truth.write_blob(
            &format!(
                "completion/{}_statement.json",
                safe_blob_name(reasoning_step_id)
            ),
            serde_json::to_string_pretty(&json!({
                "completion_statement": completion_statement,
                "claimed_artifacts": claimed_artifacts,
                "key_sources": key_sources,
                "known_limitations": known_limitations,
                "user_review_notes": user_review_notes,
            }))
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?
            .as_bytes(),
        )?;
        let action = self.action(
            reasoning_step_id,
            ProcessActionKind::Complete,
            "process.complete",
            decision.input_refs.clone(),
            json!({
                "completion_statement_ref": completion_statement_ref,
                "completion_statement": completion_statement,
                "claimed_artifacts": claimed_artifacts,
                "key_sources": key_sources,
                "known_limitations": known_limitations,
                "user_review_notes": user_review_notes,
            }),
            &decision.reason,
        );
        self.emit_and_validate_action(&action)?;
        if claimed_artifacts.is_empty()
            && decision
                .input_refs
                .iter()
                .any(|item| is_internal_artifact_claim(item))
        {
            self.truth.append_event(
                Some(&self.token.pid),
                "completion_blocked",
                json!({
                    "runtime_id": self.runtime_id,
                    "turn_id": reasoning_step_id,
                    "reason": "internal_data_refs_are_not_user_visible_artifacts",
                    "input_refs": decision.input_refs,
                    "runtime_note": "internal data refs are not user-visible artifacts",
                }),
            )?;
            self.process_capability_receipt(
                "process.complete",
                "blocked",
                json!({
                    "reason": "internal_data_refs_are_not_user_visible_artifacts",
                    "completion_recoverable": true,
                    "input_refs": decision.input_refs,
                }),
            )?;
            return Ok("running".to_string());
        }
        if claimed_artifacts.is_empty() && !replay.artifact_refs.is_empty() {
            self.truth.append_event(
                Some(&self.token.pid),
                "completion_claimed_artifacts_empty_warning",
                json!({
                    "runtime_id": self.runtime_id,
                    "turn_id": reasoning_step_id,
                    "materialized_artifacts": replay.artifact_refs.clone(),
                    "completion_statement_ref": completion_statement_ref,
                    "runtime_note": "TaskAgent submitted process.complete without claimed_artifacts even though user-visible artifact facts already exist; this is an advisory fact warning, not a hard completion block.",
                }),
            )?;
        }
        let claimed_missing_artifacts = self.missing_artifacts(&claimed_artifacts)?;
        let internal_claimed_artifacts = claimed_artifacts
            .iter()
            .filter(|path| is_internal_artifact_claim(path))
            .cloned()
            .collect::<Vec<_>>();
        if !claimed_missing_artifacts.is_empty() || !internal_claimed_artifacts.is_empty() {
            self.truth.append_event(
                Some(&self.token.pid),
                "completion_blocked",
                json!({
                    "runtime_id": self.runtime_id,
                    "turn_id": reasoning_step_id,
                    "reason": "claimed_artifact_hard_block",
                    "claimed_artifacts": claimed_artifacts,
                    "missing_artifacts": claimed_missing_artifacts,
                    "internal_claimed_artifacts": internal_claimed_artifacts,
                    "completion_statement_ref": completion_statement_ref,
                    "completion_recoverable": true,
                    "runtime_note": "process.complete checks claimed artifact file facts only; TaskAgent remains responsible for task judgment",
                }),
            )?;
            self.process_capability_receipt(
                "process.complete",
                "blocked",
                json!({
                    "reason": "claimed_artifact_hard_block",
                    "completion_recoverable": true,
                    "claimed_artifacts": claimed_artifacts,
                    "missing_artifacts": claimed_missing_artifacts,
                    "internal_claimed_artifacts": internal_claimed_artifacts,
                    "completion_statement_ref": completion_statement_ref,
                }),
            )?;
            return Ok("running".to_string());
        }
        let closure_gate = crate::check_closure_gate_for_claimed_artifacts(
            &self.guard,
            &self.truth,
            &replay,
            &claimed_artifacts,
        )?;
        self.truth.append_event(
            Some(&self.token.pid),
            "closure_gate_checked",
            to_json_value(&closure_gate)?,
        )?;
        if !closure_gate.can_complete {
            let receipt =
                crate::closure_block_receipt(&self.token.job_id, &self.token.pid, &closure_gate);
            self.truth.append_event(
                Some(&self.token.pid),
                "closure_gate_blocked",
                to_json_value(&receipt)?,
            )?;
            self.truth.append_event(
                Some(&self.token.pid),
                "capability_receipt",
                to_json_value(&receipt)?,
            )?;
            return Ok("running".to_string());
        }
        let advisory_count = closure_gate.advisory_findings.len();
        let hard_block_count = closure_gate.hard_blocks.len();
        if advisory_count > 0 {
            self.truth.append_event(
                Some(&self.token.pid),
                "completion_advisory_findings",
                json!({
                    "runtime_id": self.runtime_id,
                    "turn_id": reasoning_step_id,
                    "advisory_findings": closure_gate.advisory_findings.clone(),
                    "completion_facts": closure_gate.completion_facts.clone(),
                    "runtime_note": "advisory findings are recorded for TaskAgent/user review and did not block hard runtime completion",
                }),
            )?;
        }
        self.truth.append_event(
            Some(&self.token.pid),
            "completion_statement_recorded",
            json!({
                "runtime_id": self.runtime_id,
                "turn_id": reasoning_step_id,
                "completion_statement_ref": completion_statement_ref,
                "completion_statement": completion_statement,
                "claimed_artifacts": claimed_artifacts,
                "key_sources": key_sources,
                "known_limitations": known_limitations,
                "user_review_notes": user_review_notes,
                "runtime_note": "completion statement is the TaskAgent-owned delivery ledger recorded by process.complete",
            }),
        )?;
        self.truth.update_job_status("completed")?;
        self.truth.append_event(
            Some(&self.token.pid),
            "job_completed",
            json!({
                "status": "completed",
                "artifacts": claimed_artifacts,
                "all_artifacts": replay.artifact_refs,
                "claimed_artifacts": claimed_artifacts,
                "runtime_id": self.runtime_id,
                "turn_id": reasoning_step_id,
                "completion_statement": completion_statement,
                "completion_statement_ref": completion_statement_ref,
                "key_sources": key_sources,
                "known_limitations": known_limitations,
                "user_review_notes": user_review_notes,
                "completion_advisory_count": advisory_count,
                "completion_hard_block_count": hard_block_count,
            }),
        )?;
        self.truth.append_event(
            Some(&self.token.pid),
            "task_agent_session_completed",
            json!({"runtime_id": self.runtime_id, "session_id": self.runtime_id, "status": "completed"}),
        )?;
        Ok("completed".to_string())
    }

    fn fail(&self, reasoning_step_id: &str, decision: &NextActionDecision) -> io::Result<()> {
        let action = self.action(
            reasoning_step_id,
            ProcessActionKind::Fail,
            "process.fail",
            Vec::new(),
            json!({"reason": decision.reason}),
            &decision.reason,
        );
        self.emit_and_validate_action(&action)?;
        let code = decision
            .output_spec
            .get("error_code")
            .and_then(Value::as_str)
            .unwrap_or("TASK_AGENT_FAILED");
        self.fail_job(code, &decision.reason)
    }

    fn interrupt_by_model_protocol_error(
        &self,
        reasoning_step_id: &str,
        decision: &NextActionDecision,
    ) -> io::Result<String> {
        self.truth.update_job_status("interrupted")?;
        let error_code = decision
            .output_spec
            .get("error_code")
            .and_then(Value::as_str)
            .unwrap_or("MODEL_PROTOCOL_INTERRUPTED");
        let reason = decision
            .output_spec
            .get("reason")
            .and_then(Value::as_str)
            .unwrap_or(&decision.reason);
        self.truth.append_event(
            Some(&self.token.pid),
            "job_interrupted_by_model_protocol_error",
            json!({
                "status": "interrupted",
                "runtime_id": self.runtime_id,
                "turn_id": reasoning_step_id,
                "error": {
                    "code": error_code,
                    "message": reason,
                },
                "model_output_ref": decision.output_spec.get("model_output_ref").cloned(),
                "already_materialized_artifacts": decision.output_spec.get("already_materialized_artifacts").cloned().unwrap_or_else(|| json!([])),
                "user_visible_artifact_candidates": decision.output_spec.get("user_visible_artifact_candidates").cloned().unwrap_or_else(|| json!([])),
                "runtime_note": "model decision protocol failed after user-visible artifacts were materialized; execution is fail-closed but artifact facts are preserved for user/runner review",
            }),
        )?;
        self.truth.append_event(
            Some(&self.token.pid),
            "task_agent_session_interrupted",
            json!({
                "runtime_id": self.runtime_id,
                "session_id": self.runtime_id,
                "status": "interrupted",
                "turn_id": reasoning_step_id,
                "error_code": error_code,
            }),
        )?;
        Ok("interrupted".to_string())
    }

    fn fail_job(&self, code: &str, message: &str) -> io::Result<()> {
        self.truth.update_job_status("failed")?;
        self.truth.append_event(
            Some(&self.token.pid),
            "job_failed",
            json!({
                "error": {"code": code, "message": message},
                "runtime_id": self.runtime_id,
            }),
        )?;
        self.truth.append_event(
            Some(&self.token.pid),
            "task_agent_session_failed",
            json!({"runtime_id": self.runtime_id, "session_id": self.runtime_id, "error": {"code": code, "message": message}}),
        )?;
        Ok(())
    }

    fn block_job(&self, code: &str, message: &str) -> io::Result<()> {
        self.truth.update_job_status("blocked")?;
        self.truth.append_event(
            Some(&self.token.pid),
            "job_blocked",
            json!({
                "error": {"code": code, "message": message},
                "runtime_id": self.runtime_id,
            }),
        )?;
        self.truth.append_event(
            Some(&self.token.pid),
            "task_agent_session_blocked",
            json!({"runtime_id": self.runtime_id, "session_id": self.runtime_id, "error": {"code": code, "message": message}}),
        )?;
        Ok(())
    }

    fn missing_artifacts(&self, artifact_refs: &[String]) -> io::Result<Vec<String>> {
        let mut missing = Vec::new();
        for artifact_path in artifact_refs {
            let trimmed = artifact_path.trim();
            if trimmed.is_empty() || is_internal_artifact_claim(trimmed) {
                missing.push(artifact_path.clone());
                continue;
            }
            let path = match self.guard.resolve_workspace_path(trimmed) {
                Ok(path) => path,
                Err(err) => {
                    missing.push(format!("{trimmed}:{err}"));
                    continue;
                }
            };
            if !path.exists()
                || (path.is_file() && fs::File::open(&path).is_err())
                || (path.is_dir() && fs::read_dir(&path).is_err())
            {
                missing.push(trimmed.to_string());
            }
        }
        Ok(missing)
    }

    fn record_capability_execution(
        &self,
        decision: &NextActionDecision,
        receipt: &CapabilityReceipt,
    ) -> io::Result<()> {
        self.truth.append_event(
            Some(&self.token.pid),
            "agent_tool_action_executed",
            json!({
                "runtime_id": self.runtime_id,
                "decision_id": decision.decision_id,
                "capability_id": receipt.capability_id,
                "status": receipt.status,
                "receipt_data": receipt.data,
            }),
        )?;
        Ok(())
    }

    fn record_model_execution(
        &self,
        decision: &NextActionDecision,
        receipt: &crate::ModelCallReceipt,
    ) -> io::Result<()> {
        self.truth.append_event(
            Some(&self.token.pid),
            "agent_tool_action_executed",
            json!({
                "runtime_id": self.runtime_id,
                "decision_id": decision.decision_id,
                "capability_id": receipt.capability_id,
                "model_call_id": receipt.model_call_id,
                "operation": receipt.operation.as_str(),
                "status": receipt.status,
                "output_ref": receipt.output_ref,
            }),
        )?;
        Ok(())
    }

    fn action(
        &self,
        reasoning_step_id: &str,
        action_kind: ProcessActionKind,
        capability_id: &str,
        input_refs: Vec<String>,
        output_spec: Value,
        reason: &str,
    ) -> ProcessAction {
        ProcessAction {
            action_id: format!("act_{}_{}", safe_blob_name(reasoning_step_id), now_ms()),
            job_id: self.token.job_id.clone(),
            pid: self.token.pid.clone(),
            reasoning_step_id: reasoning_step_id.to_string(),
            action_kind,
            capability_id: capability_id.to_string(),
            input_refs,
            output_spec,
            policy: json!({"capability_token": self.token.token_id, "fail_closed": true}),
            verify_plan: json!({"runtime_verify_required": true, "no_silent_fallback": true}),
            reason: reason.to_string(),
        }
    }

    fn emit_and_validate_action(&self, action: &ProcessAction) -> io::Result<()> {
        self.truth.append_event(
            Some(&self.token.pid),
            "process_action_emitted",
            to_json_value(action)?,
        )?;
        let validator = ProcessActionValidator::new(&self.registry);
        match validator.validate_or_err(action, &self.token) {
            Ok(()) => {
                self.truth.append_event(
                    Some(&self.token.pid),
                    "process_action_validated",
                    json!({"action_id": action.action_id, "capability_id": action.capability_id}),
                )?;
                self.truth.append_event(
                    Some(&self.token.pid),
                    "agent_tool_action_validated",
                    json!({
                        "runtime_id": self.runtime_id,
                        "action_id": action.action_id,
                        "capability_id": action.capability_id,
                    }),
                )?;
                Ok(())
            }
            Err(err) => {
                self.truth.append_event(
                    Some(&self.token.pid),
                    "process_action_blocked",
                    json!({
                        "action_id": action.action_id,
                        "capability_id": action.capability_id,
                        "error": err.to_string(),
                    }),
                )?;
                Err(err)
            }
        }
    }

    fn save_checkpoint(
        &self,
        kind: &str,
        state: &Value,
        input_refs: Vec<String>,
        output_refs: Vec<String>,
    ) -> io::Result<CheckpointRef> {
        self.truth.save_checkpoint(
            &self.token.pid,
            Some(&self.runtime_id),
            kind,
            state,
            input_refs,
            output_refs,
        )
    }

    fn read_blob_text(&self, blob_ref: &str) -> io::Result<String> {
        std::fs::read_to_string(self.truth.resolve_blob_ref(blob_ref)?)
    }
}

fn process_action_kind_for_capability(capability_id: &str) -> ProcessActionKind {
    if capability_id.starts_with("model.") {
        ProcessActionKind::ModelCall
    } else if capability_id.starts_with("terminal.") {
        ProcessActionKind::RunCapability
    } else if capability_id.starts_with("client_env.") {
        ProcessActionKind::RunCapability
    } else if capability_id.starts_with("process.") {
        ProcessActionKind::RunCapability
    } else if capability_id.starts_with("tool.result.") {
        ProcessActionKind::RunCapability
    } else if capability_id == "os.verify_artifact"
        || capability_id.ends_with(".validate")
        || capability_id.starts_with("artifact.")
        || capability_id == "office.docx.batch_validate"
    {
        ProcessActionKind::Verify
    } else if capability_id.contains("preview") {
        ProcessActionKind::RequestPreview
    } else if capability_id.starts_with("os.")
        || capability_id.starts_with("office.")
        || capability_id.starts_with("data.")
        || capability_id.starts_with("document.")
        || capability_id.starts_with("source_set.")
        || capability_id.starts_with("workspace.")
        || capability_id.starts_with("dataset.")
        || capability_id.starts_with("artifact.")
        || capability_id.starts_with("package.")
        || capability_id == "process.fork_child"
    {
        ProcessActionKind::Commit
    } else {
        ProcessActionKind::Fail
    }
}

fn raw_result_ref_arg(decision: &NextActionDecision) -> io::Result<String> {
    decision
        .output_spec
        .get("raw_result_ref")
        .and_then(Value::as_str)
        .or_else(|| decision.output_spec.get("ref").and_then(Value::as_str))
        .or_else(|| {
            decision
                .output_spec
                .get("receipt_ref")
                .and_then(Value::as_str)
        })
        .or_else(|| decision.output_spec.get("path").and_then(Value::as_str))
        .or_else(|| decision.input_refs.first().map(String::as_str))
        .map(str::to_string)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "raw_result_ref/ref missing"))
}

fn inspect_json_shape(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let keys = map.keys().take(80).cloned().collect::<Vec<_>>();
            let fields = map
                .iter()
                .take(80)
                .map(|(key, value)| {
                    json!({
                        "key": key,
                        "type": json_value_kind(value),
                        "shape": shallow_json_shape(value),
                    })
                })
                .collect::<Vec<_>>();
            json!({
                "type": "object",
                "key_count": map.len(),
                "keys": keys,
                "fields": fields,
            })
        }
        Value::Array(items) => json!({
            "type": "array",
            "len": items.len(),
            "first_item_shape": items.first().map(shallow_json_shape).unwrap_or_else(|| json!({"type": "empty"})),
        }),
        other => json!({"type": json_value_kind(other)}),
    }
}

fn shallow_json_shape(value: &Value) -> Value {
    match value {
        Value::Object(map) => json!({
            "type": "object",
            "key_count": map.len(),
            "keys": map.keys().take(30).cloned().collect::<Vec<_>>(),
        }),
        Value::Array(items) => json!({
            "type": "array",
            "len": items.len(),
            "first_item_type": items.first().map(json_value_kind).unwrap_or("empty"),
        }),
        other => json!({"type": json_value_kind(other)}),
    }
}

fn json_value_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn path_arg(decision: &NextActionDecision, key: &str) -> io::Result<String> {
    decision
        .output_spec
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, format!("{key} missing")))
}

fn model_audit_scope_and_artifacts(
    decision: &NextActionDecision,
) -> io::Result<(String, Vec<String>)> {
    if let Some(items) = decision
        .output_spec
        .get("artifact_paths")
        .and_then(Value::as_array)
    {
        let mut paths = Vec::new();
        for item in items {
            let Some(path) = item.as_str() else {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "artifact_paths must contain only string workspace paths",
                ));
            };
            let normalized = path.trim().replace('\\', "/");
            if normalized.is_empty() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "artifact_paths must not contain empty paths",
                ));
            }
            if !paths.contains(&normalized) {
                paths.push(normalized);
            }
        }
        if paths.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "artifact_paths must include at least one artifact path",
            ));
        }
        return Ok(("explicit_artifact_set".to_string(), paths));
    }

    let artifact_path =
        path_arg(decision, "artifact_path").or_else(|_| path_arg(decision, "path"))?;
    Ok((
        "single_artifact".to_string(),
        vec![artifact_path.replace('\\', "/")],
    ))
}

fn string_arg(decision: &NextActionDecision, key: &str) -> io::Result<String> {
    decision
        .output_spec
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, format!("{key} missing")))
}

fn string_array_arg(decision: &NextActionDecision, key: &str) -> Vec<String> {
    decision
        .output_spec
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn terminal_service_health_check_arg(spec: &Value) -> Option<TerminalServiceHealthCheck> {
    let health = spec.get("health_check")?.as_object()?;
    let kind = health
        .get("kind")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())?
        .to_string();
    let url = health
        .get("url")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string);
    let port = health
        .get("port")
        .and_then(Value::as_u64)
        .and_then(|value| u16::try_from(value).ok());
    Some(TerminalServiceHealthCheck { kind, url, port })
}

fn terminal_expected_ports_arg(spec: &Value) -> Vec<u16> {
    spec.get("expected_ports")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_u64)
                .filter_map(|value| u16::try_from(value).ok())
                .filter(|value| *value > 0)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn string_or_array_values(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|item| {
                item.as_str().or_else(|| {
                    item.get("path")
                        .or_else(|| item.get("artifact_path"))
                        .and_then(Value::as_str)
                })
            })
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(str::to_string)
            .collect(),
        Some(Value::String(text)) => text
            .split([',', '\n', ';', '；'])
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

fn completion_statement(decision: &NextActionDecision) -> String {
    decision
        .output_spec
        .get("completion_statement")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(&decision.reason)
        .to_string()
}

fn claimed_artifacts(decision: &NextActionDecision, replay_artifacts: &[String]) -> Vec<String> {
    let _ = replay_artifacts;
    let mut artifacts = string_or_array_values(decision.output_spec.get("claimed_artifacts"));
    if artifacts.is_empty() {
        artifacts = string_or_array_values(decision.output_spec.get("artifacts"));
    }
    artifacts
        .into_iter()
        .map(|item| item.trim().replace('\\', "/"))
        .filter(|item| !item.is_empty())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn is_internal_artifact_claim(path: &str) -> bool {
    let lower = path.trim().to_ascii_lowercase();
    lower.starts_with("blob://")
        || lower.starts_with("dataset://")
        || lower.starts_with("artifact://")
        || lower.starts_with("artifact_ref://")
        || lower.starts_with("source_set://")
        || lower.starts_with("raw_document_set://")
        || lower.starts_with("model_ledger://")
        || lower.contains(".supernova_v2")
}

fn depth_arg(decision: &NextActionDecision, default_depth: usize) -> usize {
    decision
        .output_spec
        .get("max_depth")
        .and_then(Value::as_u64)
        .map(|value| value as usize)
        .unwrap_or(default_depth)
}

fn approval_id_arg(decision: &NextActionDecision) -> Option<String> {
    decision
        .output_spec
        .get("approval_id")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
}

fn is_hard_runtime_block(receipt: &CapabilityReceipt) -> bool {
    receipt
        .data
        .get("hard_block")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || receipt
            .data
            .get("mutation_policy_blocked")
            .and_then(Value::as_bool)
            .unwrap_or(false)
}

fn provider_native_receipt_is_recoverable_error(receipt: &CapabilityReceipt) -> bool {
    if receipt.status == "blocked" {
        return true;
    }
    receipt.status == "failed"
        && (receipt
            .data
            .get("schema_error")
            .and_then(Value::as_bool)
            .unwrap_or(false)
            || receipt
                .data
                .get("recoverable_by_task_agent")
                .and_then(Value::as_bool)
                .unwrap_or(false))
}

fn stable_content_hash(bytes: &[u8]) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn provider_native_receipt_recovery_kind(receipt: &CapabilityReceipt) -> &'static str {
    if receipt.status == "blocked" {
        "provider_tool_kernel_block_recoverable"
    } else {
        "provider_tool_receipt_validation_failed"
    }
}

fn provider_native_receipt_error_code(receipt: &CapabilityReceipt) -> &'static str {
    if receipt.status == "blocked" {
        "PROVIDER_NATIVE_KERNEL_BLOCKED_TOOL_CALL"
    } else {
        "PROVIDER_NATIVE_TOOL_ARGUMENTS_INVALID"
    }
}

fn provider_native_receipt_corrective_instruction(receipt: &CapabilityReceipt) -> &'static str {
    if receipt.status == "blocked" {
        "The Kernel blocked this provider tool_call. Treat the block as authoritative; retry only with workspace-scoped paths and schema-valid top-level arguments for the same provider tool if the task still requires it."
    } else {
        "The Kernel returned a recoverable validation receipt for this provider tool_call. Inspect the receipt facts and tool_schema_summary, then retry only with corrected top-level arguments if the task still requires this capability."
    }
}

fn content_arg(truth: &ProcessTruthStore, decision: &NextActionDecision) -> io::Result<String> {
    Ok(content_value_arg(truth, &decision.output_spec)?.content)
}

fn content_value_arg(
    truth: &ProcessTruthStore,
    arguments: &Value,
) -> io::Result<ResolvedContentArg> {
    if let Some(content) = arguments.get("content").and_then(Value::as_str) {
        return Ok(ResolvedContentArg {
            content: content.to_string(),
            source_field: "content",
        });
    }
    if let Some(text) = arguments.get("text").and_then(Value::as_str) {
        return Ok(ResolvedContentArg {
            content: text.to_string(),
            source_field: "text",
        });
    }
    if let Some(content_ref) = arguments.get("content_ref").and_then(Value::as_str) {
        return Ok(ResolvedContentArg {
            content: read_content_ref_text(truth, content_ref)?,
            source_field: "content_ref",
        });
    }
    if let Some(text_ref) = arguments.get("text_ref").and_then(Value::as_str) {
        return Ok(ResolvedContentArg {
            content: read_content_ref_text(truth, text_ref)?,
            source_field: "text_ref",
        });
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidInput,
        "content/text/content_ref/text_ref missing; pass literal content/text or a blob ref containing plain text or a JSON object with text/content/rewritten_text",
    ))
}

fn content_arg_field_present(arguments: &Value) -> bool {
    ["content", "text", "content_ref", "text_ref"]
        .iter()
        .any(|field| arguments.get(*field).is_some())
}

fn read_content_ref_text(truth: &ProcessTruthStore, content_ref: &str) -> io::Result<String> {
    let raw = std::fs::read_to_string(truth.resolve_blob_ref(content_ref)?)?;
    if let Ok(value) = serde_json::from_str::<Value>(&raw) {
        if let Some(text) = value.get("text").and_then(Value::as_str) {
            return Ok(text.to_string());
        }
        if let Some(content) = value.get("content").and_then(Value::as_str) {
            return Ok(content.to_string());
        }
        if let Some(rewritten_text) = value.get("rewritten_text").and_then(Value::as_str) {
            return Ok(rewritten_text.to_string());
        }
    }
    Ok(raw)
}

fn operation_target_paths_include(
    operation: &ExecutablePreviewOperation,
    target_path: &str,
) -> bool {
    let normalized_target = normalize_preview_target_path(target_path);
    operation
        .target_paths
        .iter()
        .any(|path| normalize_preview_target_path(path) == normalized_target)
}

fn normalize_preview_target_path(path: &str) -> String {
    path.trim().replace('\\', "/")
}

fn artifact_model_audit_instruction() -> String {
    [
        "You are model_audit_agent, an independent reviewer inside a SuperNova Task Process.",
        "Your only job is to audit a user-facing task artifact for real deliverability.",
        "Return JSON only. Do not rewrite the artifact. Do not decide whether the task should complete.",
        "Use only the user task goal, artifact text, and runtime evidence refs provided in the input payloads.",
        "Audit only the artifact paths explicitly listed in audit_scope/included_artifacts. Do not judge sibling artifacts, unprovided deliverables, artifact counts, or overall task completeness.",
        "Compare the included artifact content against the relevant user goal, target audience, source requirements, and explicit constraints only within that audit scope.",
        "Assess whether a human user could accept the artifact as clear, complete, source-grounded, factually careful, goal-aligned, and usable.",
        "Audit for placeholder, template-like, or silent-fallback risk semantically; do not flag a word just because it appears.",
        "Terms such as 'template' or 'to be supplemented' can be legitimate business facts. Flag them only when the artifact itself is using placeholders, generic filler, fabricated source claims, or template scaffolding as final user content.",
        "Separate hard user-facing defects from advisory quality findings in the JSON arrays.",
        "If evidence is insufficient, report that as findings and risks rather than inventing missing facts.",
        "Never emit fields such as task_should_continue, task_should_complete, task_complete, case_pass, all_deliverables_present, or missing_sibling_artifacts.",
        "The TaskAgent will inspect your findings and decide its next action.",
    ]
    .join("\n")
}

fn artifact_model_audit_output_schema() -> Value {
    json!({
        "type": "object",
        "required": [
            "audit_kind",
            "audit_scope",
            "artifact_path",
            "included_artifacts",
            "quality_pass",
            "human_acceptance_pass",
            "findings",
            "blocking_issues",
            "factual_risks",
            "deliverability_risks",
            "source_grounding",
            "coverage_assessment",
            "suggested_review_focus",
            "auditor_limitations"
        ],
        "properties": {
            "audit_kind": {"type": "string"},
            "audit_scope": {"type": "string", "enum": ["single_artifact", "explicit_artifact_set"]},
            "artifact_path": {"type": "string"},
            "included_artifacts": {"type": "array"},
            "quality_pass": {"type": "boolean"},
            "human_acceptance_pass": {"type": "boolean"},
            "findings": {"type": "array"},
            "blocking_issues": {"type": "array"},
            "factual_risks": {"type": "array"},
            "deliverability_risks": {"type": "array"},
            "source_grounding": {"type": "object"},
            "coverage_assessment": {"type": "object"},
            "suggested_review_focus": {"type": "array"},
            "auditor_limitations": {"type": "array"}
        }
    })
}

fn validate_artifact_model_audit_output(
    output: &Value,
    audit_scope: &str,
    included_artifacts: &[String],
) -> Vec<String> {
    let mut errors = Vec::new();
    let Some(object) = output.as_object() else {
        return vec!["audit output is not an object".to_string()];
    };
    for key in [
        "audit_kind",
        "audit_scope",
        "artifact_path",
        "included_artifacts",
        "quality_pass",
        "human_acceptance_pass",
        "findings",
        "blocking_issues",
        "factual_risks",
        "deliverability_risks",
        "source_grounding",
        "coverage_assessment",
        "suggested_review_focus",
        "auditor_limitations",
    ] {
        if !object.contains_key(key) {
            errors.push(format!("required key missing: {key}"));
        }
    }
    for key in [
        "findings",
        "blocking_issues",
        "factual_risks",
        "deliverability_risks",
        "suggested_review_focus",
        "auditor_limitations",
        "included_artifacts",
    ] {
        if object.get(key).is_some_and(|value| !value.is_array()) {
            errors.push(format!("{key} must be an array"));
        }
    }
    for key in ["source_grounding", "coverage_assessment"] {
        if object.get(key).is_some_and(|value| !value.is_object()) {
            errors.push(format!("{key} must be an object"));
        }
    }
    for key in ["quality_pass", "human_acceptance_pass"] {
        if object.get(key).is_some_and(|value| !value.is_boolean()) {
            errors.push(format!("{key} must be a boolean"));
        }
    }
    match object.get("audit_scope").and_then(Value::as_str) {
        Some(scope) if scope == audit_scope => {}
        Some(scope) => {
            errors.push(format!(
                "audit_scope mismatch: expected {}, got {}",
                audit_scope, scope
            ));
        }
        None => errors.push("audit_scope is required".to_string()),
    }
    match object.get("included_artifacts").and_then(Value::as_array) {
        Some(items) => {
            let actual = items
                .iter()
                .filter_map(Value::as_str)
                .map(normalize_artifact_path)
                .collect::<BTreeSet<_>>();
            let expected = included_artifacts
                .iter()
                .map(|path| normalize_artifact_path(path))
                .collect::<BTreeSet<_>>();
            if actual != expected {
                errors.push(format!(
                    "included_artifacts mismatch: expected {:?}, got {:?}",
                    expected, actual
                ));
            }
        }
        None => errors.push("included_artifacts is required".to_string()),
    }
    if let Some(path) = object.get("artifact_path").and_then(Value::as_str) {
        if let Some(primary) = included_artifacts.first() {
            if path.replace('\\', "/") != primary.replace('\\', "/") {
                errors.push(format!(
                    "artifact_path mismatch: expected {}, got {}",
                    primary.replace('\\', "/"),
                    path.replace('\\', "/")
                ));
            }
        }
    }
    for key in [
        "task_should_continue",
        "task_should_complete",
        "task_complete",
        "task_completed",
        "case_pass",
        "all_deliverables_present",
        "artifact_count_should_be",
        "missing_sibling_artifacts",
    ] {
        if object.contains_key(key) {
            errors.push(format!(
                "forbidden task-completion field present in artifact audit output: {key}"
            ));
        }
    }
    errors
}

fn normalize_artifact_path(path: &str) -> String {
    path.trim()
        .strip_prefix("artifact://")
        .unwrap_or(path.trim())
        .replace('\\', "/")
}

fn detected_write_intent(normalized_path: &str) -> &'static str {
    let path = normalized_path.to_ascii_lowercase();
    if path.contains("/tmp/") || path.starts_with("tmp/") || path.ends_with(".jsonl") {
        "temp_dataset_candidate"
    } else {
        "artifact_candidate"
    }
}

fn model_audit_extra_input_refs(decision: &NextActionDecision) -> Vec<String> {
    let mut refs = decision
        .input_refs
        .iter()
        .filter(|item| item.starts_with("blob://"))
        .cloned()
        .collect::<Vec<_>>();
    for key in [
        "source_set_ref",
        "dataset_ref",
        "raw_document_set_ref",
        "coverage_ref",
        "local_audit_ref",
    ] {
        if let Some(value) = decision.output_spec.get(key).and_then(Value::as_str) {
            if value.starts_with("blob://") {
                refs.push(value.to_string());
            }
        }
    }
    if let Some(items) = decision
        .output_spec
        .get("source_refs")
        .and_then(Value::as_array)
    {
        for item in items {
            if let Some(value) = item.as_str() {
                if value.starts_with("blob://") {
                    refs.push(value.to_string());
                }
            }
        }
    }
    refs
}

fn declared_audit_source_refs(decision: &NextActionDecision) -> Vec<String> {
    let mut refs = decision.input_refs.clone();
    for key in [
        "source_set_ref",
        "dataset_ref",
        "raw_document_set_ref",
        "coverage_ref",
        "local_audit_ref",
    ] {
        if let Some(value) = decision.output_spec.get(key).and_then(Value::as_str) {
            refs.push(value.to_string());
        }
    }
    if let Some(items) = decision
        .output_spec
        .get("source_refs")
        .and_then(Value::as_array)
    {
        for item in items {
            if let Some(value) = item.as_str() {
                refs.push(value.to_string());
            }
        }
    }
    refs.sort();
    refs.dedup();
    refs
}

fn task_event_payloads(data: &Value) -> Vec<Value> {
    if data.get("capability_id").is_some() {
        return vec![data.clone()];
    }
    let mut values = Vec::new();
    if let Some(receipt) = data.get("receipt") {
        values.push(receipt.clone());
    }
    values
}

fn payload_path(payload: &Value) -> Option<String> {
    payload
        .get("artifact_path")
        .or_else(|| payload.get("archive_path"))
        .or_else(|| {
            payload
                .get("data")
                .and_then(|data| data.get("artifact_path"))
        })
        .or_else(|| {
            payload
                .get("data")
                .and_then(|data| data.get("archive_path"))
        })
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn provider_native_write_intent_capability(capability_id: &str) -> bool {
    matches!(capability_id, "os.write_artifact" | "os.write_temp_dataset")
}

fn provider_native_blocked_artifact_extension(path: &str) -> Option<&'static str> {
    let normalized = path.trim().replace('\\', "/").to_ascii_lowercase();
    if normalized.ends_with(".zip") {
        return Some(".zip");
    }
    if normalized.ends_with(".docx") {
        return Some(".docx");
    }
    None
}

fn provider_native_write_artifact_inline_text(decision: &NextActionDecision) -> Option<&str> {
    decision
        .output_spec
        .get("content")
        .and_then(Value::as_str)
        .or_else(|| decision.output_spec.get("text").and_then(Value::as_str))
}

fn provider_native_write_artifact_claims_workspace_mutation(decision: &NextActionDecision) -> bool {
    let Some(text) = provider_native_write_artifact_inline_text(decision) else {
        return false;
    };
    let lower = text.to_ascii_lowercase();
    let has_completion_claim = [
        "已执行",
        "执行完成",
        "已完成",
        "成功删除",
        "成功移动",
        "成功重命名",
        "已删除",
        "已移动",
        "已重命名",
        "status: completed",
        "status\": \"completed",
        "completed",
        "executed",
        "deleted",
        "moved",
        "renamed",
    ]
    .iter()
    .any(|needle| lower.contains(needle));
    let has_workspace_mutation_claim = [
        "删除",
        "移动",
        "重命名",
        "清理",
        "归档",
        "rename",
        "renamed",
        "move",
        "moved",
        "delete",
        "deleted",
        "cleanup",
        "archive",
        "organize",
    ]
    .iter()
    .any(|needle| lower.contains(needle));
    has_completion_claim && has_workspace_mutation_claim
}

fn provider_native_real_workspace_mutation_receipt(capability_id: &str) -> bool {
    provider_tool_is_mutation_apply_capability(capability_id)
        && !matches!(
            capability_id,
            "os.write_artifact"
                | "os.write_temp_dataset"
                | "dataset.export_csv"
                | "dataset.export_markdown"
                | "artifact.copy_source_set"
        )
}

fn provider_native_write_intent_preview_markdown(
    capability_id: &str,
    target_paths: &[String],
    reason: &str,
    draft_text: Option<&str>,
) -> String {
    let paths = if target_paths.is_empty() {
        "- target path not resolved from arguments".to_string()
    } else {
        target_paths
            .iter()
            .map(|path| format!("- `{path}`"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let draft_preview = draft_text
        .map(|text| {
            let truncated = text.chars().take(12_000).collect::<String>();
            let suffix = if text.chars().count() > 12_000 {
                "\n\n_Preview truncated at 12000 characters; approve only if the visible draft is sufficient for this operation._"
            } else {
                ""
            };
            format!("\n\n## Draft content\n\n```text\n{truncated}\n```{suffix}")
        })
        .unwrap_or_else(|| {
            "\n\n## Draft content\n\nNo inline draft content was supplied with this write intent.".to_string()
        });
    format!(
        "# Provider Native Write Preview\n\nCapability: `{capability_id}`\n\nTarget paths:\n{paths}\n\nReason:\n{reason}\n{draft_preview}"
    )
}

fn provider_native_mutation_preview_markdown(
    capability_id: &str,
    target_paths: &[String],
    reason: &str,
    arguments: &Value,
) -> String {
    let paths = if target_paths.is_empty() {
        "- target path not resolved from arguments".to_string()
    } else {
        target_paths
            .iter()
            .map(|path| format!("- `{path}`"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let arguments = serde_json::to_string_pretty(arguments).unwrap_or_else(|_| "{}".to_string());
    let truncated = arguments.chars().take(8_000).collect::<String>();
    let suffix = if arguments.chars().count() > 8_000 {
        "\n\n_Arguments preview truncated at 8000 characters._"
    } else {
        ""
    };
    format!(
        "# Provider Native Mutation Preview\n\nCapability: `{capability_id}`\n\nTarget paths:\n{paths}\n\nReason:\n{reason}\n\n## Operation arguments\n\n```json\n{truncated}\n```{suffix}\n\nThe mutation has not been executed. Approve to let the Capability Kernel execute this exact operation scope."
    )
}

fn provider_native_arguments_without_approval_id(arguments: &Value) -> Value {
    let mut value = arguments.clone();
    if let Some(object) = value.as_object_mut() {
        object.remove("approval_id");
    }
    value
}

fn provider_native_auto_preview_risk_level(capability_id: &str) -> &'static str {
    match capability_id {
        "os.delete_path"
        | "os.move_path"
        | "os.rename_path"
        | "os.rollback_tx"
        | "workspace.apply_organize_tx"
        | "workspace.rename_batch_apply"
        | "office.docx.rewrite_in_place" => "high",
        _ => "medium",
    }
}

fn provider_native_terminal_preview_markdown(
    argv: &[String],
    target_paths: &[String],
    risk_reason: &str,
) -> String {
    let command = if argv.is_empty() {
        "- argv missing".to_string()
    } else {
        argv.iter()
            .enumerate()
            .map(|(index, value)| format!("- argv[{index}]: `{value}`"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let paths = if target_paths.is_empty() {
        "- target path not resolved from arguments".to_string()
    } else {
        target_paths
            .iter()
            .map(|path| format!("- `{path}`"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    format!(
        "# Terminal Command Approval\n\n## Command\n\n{command}\n\n## Target paths\n\n{paths}\n\n## Kernel risk\n\n- classification: terminal command requires approval\n- reason: `{risk_reason}`\n\nThe command has not been executed. Approve to let the Capability Kernel execute this exact argv and return stdout/stderr as the provider tool result."
    )
}

#[cfg(test)]
mod provider_native_preview_tests {
    use super::{
        provider_native_arguments_without_approval_id, provider_native_auto_preview_risk_level,
        provider_native_mutation_preview_markdown, provider_native_terminal_preview_markdown,
        provider_native_write_intent_preview_markdown,
    };
    use serde_json::json;

    #[test]
    fn provider_native_write_preview_includes_inline_draft_content() {
        let markdown = provider_native_write_intent_preview_markdown(
            "os.write_artifact",
            &["out.md".to_string()],
            "write requested artifact",
            Some("draft body"),
        );

        assert!(markdown.contains("## Draft content"));
        assert!(markdown.contains("draft body"));
        assert!(markdown.contains("`out.md`"));
    }

    #[test]
    fn provider_native_mutation_preview_uses_high_contrast_operation_scope() {
        let arguments = provider_native_arguments_without_approval_id(&json!({
            "path": "OLD.png",
            "approval_id": "approval_should_not_be_in_preview"
        }));
        let markdown = provider_native_mutation_preview_markdown(
            "os.delete_path",
            &["OLD.png".to_string()],
            "delete requested file",
            &arguments,
        );

        assert_eq!(
            provider_native_auto_preview_risk_level("os.delete_path"),
            "high"
        );
        assert!(markdown.contains("Provider Native Mutation Preview"));
        assert!(markdown.contains("`OLD.png`"));
        assert!(markdown.contains("\"path\": \"OLD.png\""));
        assert!(!arguments
            .to_string()
            .contains("approval_should_not_be_in_preview"));
    }

    #[test]
    fn provider_native_terminal_preview_includes_command_object() {
        let markdown = provider_native_terminal_preview_markdown(
            &[
                "powershell.exe".to_string(),
                "-Command".to_string(),
                "Remove-Item old.txt".to_string(),
            ],
            &["old.txt".to_string()],
            "capability_requires_preview_approval",
        );

        assert!(markdown.contains("Terminal Command Approval"));
        assert!(markdown.contains("argv[0]"));
        assert!(markdown.contains("Remove-Item old.txt"));
        assert!(markdown.contains("`old.txt`"));
    }
}

fn provider_native_recoverable_error_hint(
    decision: &NextActionDecision,
    raw_error: &str,
) -> Option<(&'static str, &'static str, &'static str, Value)> {
    if provider_native_raw_tool_result_path_field(decision).is_some() {
        return Some((
            "raw_tool_result_path_used_as_workspace_path",
            "PROVIDER_NATIVE_RAW_TOOL_RESULT_PATH_USED_AS_WORKSPACE_PATH",
            "raw_tool_results live in the ProcessTruth/blob data plane, not the workspace filesystem. Do not pass /raw_tool_results/... or blob raw-result refs to os.* path arguments; read them with process.read_ref or tool.result.page using the exact ref from the previous tool result.",
            json!({
                "capability_id": "process.read_ref",
                "arguments": {"ref": "blob://<job_id>/raw_tool_results/..."}
            }),
        ));
    }
    if decision.capability_id == "os.write_artifact" {
        if let Some(path) = decision.output_spec.get("path").and_then(Value::as_str) {
            if let Some(extension) = provider_native_blocked_artifact_extension(path) {
                return Some((
                    "binary_or_compound_artifact_requires_native_capability",
                    "PROVIDER_NATIVE_WRITE_ARTIFACT_BINARY_TARGET_REJECTED",
                    "os.write_artifact is for text user artifacts such as markdown, csv, json, and txt. Do not create .zip or .docx by writing raw text, placeholder bytes, or inline content. Compound artifacts must be produced by a real package/office Kernel capability receipt.",
                    json!({
                        "zip": {
                            "capability_id": "package.build_zip",
                            "arguments": {
                                "source_set_ref": "<source_set_ref from source_set.create>",
                                "destination_zip_path": "deliverable.zip"
                            }
                        },
                        "docx": {
                            "capability_id": "office.docx.rewrite_save_as",
                            "arguments": {
                                "input_path": "<workspace-relative source .docx>",
                                "output_path": "deliverables/output.docx"
                            }
                        },
                        "rejected_extension": extension
                    }),
                ));
            }
        }
        if provider_native_write_artifact_claims_workspace_mutation(decision) {
            return Some((
                "workspace_mutation_report_requires_receipt",
                "PROVIDER_NATIVE_MUTATION_REPORT_WITHOUT_RECEIPT_REJECTED",
                "Do not use os.write_artifact to claim workspace mutations were executed unless ProcessTruth already contains a successful real mutation receipt. First execute the approved os/workspace/office/package mutation capability; otherwise write only a preview or analysis that does not claim completion.",
                json!({
                    "required_order": [
                        "call the real mutation capability as provider-visible intent",
                        "wait for Kernel-owned preview approval if approval is required",
                        "let Kernel execute the original pending tool call after approval",
                        "write a report only after the mutation receipt exists"
                    ]
                }),
            ));
        }
    }
    if decision.capability_id == "source_set.create"
        && (root_path_argument_is_rooted(decision)
            || raw_error.contains("rooted paths are not workspace-scoped")
            || raw_error.contains("absolute paths are not workspace-scoped"))
    {
        return Some((
            "source_set_root_path_not_workspace_scoped",
            "PROVIDER_NATIVE_SOURCE_SET_ROOT_PATH_NOT_WORKSPACE_SCOPED",
            "source_set.create root_path must be workspace-relative. Use \".\" for the workspace root or a relative subdirectory; never use \"/\", rooted paths, absolute paths, or paths outside the workspace.",
            json!({
                "capability_id": "source_set.create",
                "arguments": {"root_path": "."}
            }),
        ));
    }
    if decision.capability_id == "workspace.rename_batch_preview"
        && rename_batch_preview_error_is_recoverable(raw_error)
    {
        return Some((
            "rename_batch_preview_mappings_invalid",
            "PROVIDER_NATIVE_RENAME_BATCH_PREVIEW_MAPPINGS_INVALID",
            "workspace.rename_batch_preview requires mappings to be an array of objects with exactly source_path and destination_path workspace-relative strings. Do not use from/to, old/new, source/destination, or nested arguments.",
            json!({
                "capability_id": "workspace.rename_batch_preview",
                "arguments": {
                    "mappings": [
                        {
                            "source_path": "relative/source.ext",
                            "destination_path": "relative/destination.ext"
                        }
                    ]
                }
            }),
        ));
    }
    if provider_native_source_set_ref_consumer(&decision.capability_id)
        && source_set_ref_error_is_recoverable(decision, raw_error)
    {
        return Some((
            "source_set_ref_invalid_or_fabricated",
            "PROVIDER_NATIVE_SOURCE_SET_REF_INVALID_OR_FABRICATED",
            "source_set_ref must be copied exactly from a successful source_set.create tool result. Do not synthesize blob:// refs from a job id, and do not pass workspace maps, document indexes, source_set_tree text blobs, or raw_tool_results as source_set_ref.",
            json!({
                "capability_id": decision.capability_id,
                "arguments": {"source_set_ref": "<copy exact source_set_ref from prior source_set.create receipt>"}
            }),
        ));
    }
    None
}

fn provider_native_generic_recoverable_error_hint(
    decision: &NextActionDecision,
    _raw_error: &str,
) -> (&'static str, &'static str, &'static str, Value) {
    (
        "provider_tool_argument_or_runtime_validation_failed",
        "PROVIDER_NATIVE_TOOL_ARGUMENTS_INVALID",
        "The Process Kernel rejected this provider tool_call. Treat the rejection as authoritative, inspect kernel_error and tool_schema_summary, then retry only if the task still requires this same capability with schema-valid top-level arguments.",
        json!({
            "capability_id": decision.capability_id,
            "arguments": "<retry with top-level JSON fields from tool_schema_summary; do not wrap inside arguments>"
        }),
    )
}

fn rename_batch_preview_error_is_recoverable(raw_error: &str) -> bool {
    let lower = raw_error.to_ascii_lowercase();
    lower.contains("mappings missing")
        || lower.contains("mappings must be an array")
        || lower.contains("source_path missing")
        || lower.contains("destination_path missing")
}

fn root_path_argument_is_rooted(decision: &NextActionDecision) -> bool {
    decision
        .output_spec
        .get("root_path")
        .and_then(Value::as_str)
        .is_some_and(provider_native_string_is_rooted_or_absolute)
}

fn provider_native_string_is_rooted_or_absolute(value: &str) -> bool {
    let trimmed = value.trim();
    trimmed.starts_with('/')
        || trimmed.starts_with('\\')
        || (trimmed.len() >= 3
            && trimmed.as_bytes()[1] == b':'
            && matches!(trimmed.as_bytes()[2], b'/' | b'\\'))
}

fn provider_native_raw_tool_result_path_field(
    decision: &NextActionDecision,
) -> Option<(&'static str, String)> {
    for field in provider_native_workspace_path_fields() {
        let Some(value) = decision.output_spec.get(field).and_then(Value::as_str) else {
            continue;
        };
        let normalized = value.trim().replace('\\', "/");
        if normalized == "raw_tool_results"
            || normalized.starts_with("raw_tool_results/")
            || normalized.starts_with("/raw_tool_results/")
            || normalized.contains("/raw_tool_results/")
            || normalized.contains("blob://") && normalized.contains("/raw_tool_results/")
        {
            return Some((field, value.to_string()));
        }
    }
    None
}

fn provider_native_workspace_path_fields() -> &'static [&'static str] {
    &[
        "path",
        "root_path",
        "input_path",
        "output_path",
        "artifact_path",
        "source_path",
        "destination_path",
        "left_path",
        "right_path",
        "archive_path",
        "destination_dir",
        "destination_zip_path",
        "tree_path",
        "manifest_path",
        "checksums_path",
        "perf_notes_path",
    ]
}

fn provider_native_source_set_ref_consumer(capability_id: &str) -> bool {
    matches!(
        capability_id,
        "source_set.read_page"
            | "source_set.coverage_verify"
            | "workspace.batch_hash"
            | "workspace.find_duplicates"
            | "workspace.recent_changes"
            | "workspace.recent_changes_snapshot"
            | "workspace.plan_organize"
            | "workspace.tree_index"
            | "workspace.perf_inventory"
            | "dataset.coverage_verify"
            | "artifact.copy_source_set"
            | "artifact.source_coverage_verify"
            | "office.docx.batch_read_text"
            | "office.docx.batch_extract_metadata"
            | "office.docx.batch_validate"
            | "package.build_zip"
    )
}

fn source_set_ref_error_is_recoverable(decision: &NextActionDecision, raw_error: &str) -> bool {
    let Some(source_set_ref) = decision
        .output_spec
        .get("source_set_ref")
        .and_then(Value::as_str)
    else {
        return false;
    };
    let normalized_ref = source_set_ref.trim().replace('\\', "/");
    if normalized_ref.is_empty()
        || !normalized_ref.starts_with("blob://")
        || !normalized_ref.contains("/source_sets/")
        || normalized_ref.contains("/raw_tool_results/")
        || normalized_ref.contains("source_set_tree")
        || normalized_ref.contains("workspace_map")
        || normalized_ref.contains("document_index")
    {
        return true;
    }
    let lower_error = raw_error.to_ascii_lowercase();
    lower_error.contains("blob ref does not belong to this job")
        || lower_error.contains("blob ref has no relative path")
        || lower_error.contains("blob ref leaves blob store boundary")
        || lower_error.contains("cannot find the file")
        || lower_error.contains("no such file")
        || lower_error.contains("os error 2")
        || lower_error.contains("expected value at line 1 column 1")
        || lower_error.contains("eof while parsing")
}

fn merge_json_object(target: &mut Value, extra: &Value) {
    let (Some(target), Some(extra)) = (target.as_object_mut(), extra.as_object()) else {
        return;
    };
    for (key, value) in extra {
        target.insert(key.clone(), value.clone());
    }
}

fn task_provider_protocol(provider: &str) -> &'static str {
    if provider == "deepseek" {
        "deepseek_chat_completions"
    } else {
        "model_provider_transcript"
    }
}

fn compact_json_preview(value: &Value, max_chars: usize) -> Value {
    let text = serde_json::to_string(value).unwrap_or_else(|_| "null".to_string());
    let char_count = text.chars().count();
    let preview = text.chars().take(max_chars).collect::<String>();
    json!({
        "preview": preview,
        "truncated": char_count > max_chars,
        "chars": char_count,
    })
}

fn office_worker_project() -> std::path::PathBuf {
    std::env::var("SUPERNOVA_OFFICE_WORKER_PROJECT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .join("office_worker")
                .join("SuperNova.OfficeWorker")
                .join("SuperNova.OfficeWorker.csproj")
        })
}
