use std::collections::BTreeSet;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{json, Value};

use crate::model_config::ModelInvocationConfig;
use crate::model_runtime::{default_model_provider_from_env, ModelProvider, ModelStreamSink};
use crate::provider_transcript::{
    record_provider_tool_result_with_metadata, record_provider_user_control_message,
    ProviderToolResultMetadata,
};
use crate::task_agent::TaskAgentRunResult;
use crate::task_agent_runtime::TaskAgentRuntime;
use crate::{
    build_capability_approval_request, create_agent_job_with_state_root,
    default_capability_registry, finalize_capability_approval, now_ms, prepare_capability_approval,
    safe_blob_name, stop_terminal_services_for_job, terminal_runtime::TerminalApproval,
    to_json_value, AgentJob, AgentProcess, ApprovalRuntime, ApprovalTokenRecord, ArtifactRuntime,
    CapabilityReceipt, CapabilityToken, CheckpointRef, DataRuntime, OfficeRuntime, OsRuntime,
    PackageRuntime, ProcessTruthStore, TerminalRuntime, TerminalServiceHealthCheck, WorkspaceGuard,
};

#[derive(Clone, Debug)]
pub struct RootAgentProcessController {
    workspace_root: PathBuf,
    state_root: PathBuf,
    model_provider: Arc<dyn ModelProvider>,
}

impl RootAgentProcessController {
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
        })
    }

    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    pub fn state_root(&self) -> &Path {
        &self.state_root
    }

    pub fn start_job(&self, user_goal: &str) -> io::Result<TaskAgentRunResult> {
        self.start_job_with_max_turns(user_goal, None)
    }

    pub fn start_job_with_max_turns(
        &self,
        user_goal: &str,
        max_turns: Option<usize>,
    ) -> io::Result<TaskAgentRunResult> {
        self.start_job_with_config(user_goal, max_turns, ModelInvocationConfig::from_env())
    }

    pub fn start_job_with_config(
        &self,
        user_goal: &str,
        max_turns: Option<usize>,
        model_config: ModelInvocationConfig,
    ) -> io::Result<TaskAgentRunResult> {
        self.start_job_with_config_and_initial_context(user_goal, max_turns, model_config, None)
    }

    pub fn start_job_with_config_and_initial_context(
        &self,
        user_goal: &str,
        max_turns: Option<usize>,
        model_config: ModelInvocationConfig,
        initial_context: Option<Value>,
    ) -> io::Result<TaskAgentRunResult> {
        self.start_job_with_config_initial_context_and_started(
            user_goal,
            max_turns,
            model_config,
            initial_context,
            |_job, _process, _truth| Ok(()),
        )
    }

    pub fn start_job_with_config_initial_context_and_started<F>(
        &self,
        user_goal: &str,
        max_turns: Option<usize>,
        model_config: ModelInvocationConfig,
        initial_context: Option<Value>,
        on_started: F,
    ) -> io::Result<TaskAgentRunResult>
    where
        F: FnOnce(&AgentJob, &AgentProcess, &ProcessTruthStore) -> io::Result<()>,
    {
        self.start_job_with_config_initial_context_started_and_stream_sink(
            user_goal,
            max_turns,
            model_config,
            initial_context,
            on_started,
            None,
        )
    }

    pub fn start_job_with_config_initial_context_started_and_stream_sink<F>(
        &self,
        user_goal: &str,
        max_turns: Option<usize>,
        model_config: ModelInvocationConfig,
        initial_context: Option<Value>,
        on_started: F,
        model_stream_sink: Option<Arc<dyn ModelStreamSink>>,
    ) -> io::Result<TaskAgentRunResult>
    where
        F: FnOnce(&AgentJob, &AgentProcess, &ProcessTruthStore) -> io::Result<()>,
    {
        let mut model_config = model_config;
        model_config.enforce_task_agent_provider_native_tools();
        let (job, process, truth) =
            create_agent_job_with_state_root(&self.workspace_root, &self.state_root, user_goal)?;
        let config_ref =
            self.record_model_invocation_config(&truth, &process.pid, &model_config)?;
        if let Some(initial_context) = initial_context {
            let context_ref = truth.write_blob(
                &format!("task_initial_context/{}.json", crate::now_ms()),
                &serde_json::to_vec_pretty(&initial_context).map_err(crate::json_err)?,
            )?;
            truth.append_event(
                Some(&process.pid),
                "task_initial_context_bound",
                json!({
                    "context_ref": context_ref,
                    "context_pack_id": initial_context.get("context_pack_id").and_then(Value::as_str),
                    "container_id": initial_context.get("container_id").and_then(Value::as_str),
                    "auto_approve": initial_context.get("auto_approve").and_then(Value::as_bool).unwrap_or(false),
                    "fact_boundary": "Initial container context is provider-visible task input context; it does not bypass Process Kernel capability policy.",
                }),
            )?;
        }
        on_started(&job, &process, &truth)?;
        self.run_task_agent(
            job,
            process,
            truth,
            user_goal,
            max_turns,
            model_config,
            Some(config_ref),
            model_stream_sink,
        )
    }

    pub fn resume_job(&self, job_id: &str) -> io::Result<TaskAgentRunResult> {
        self.resume_job_with_max_turns(job_id, None)
    }

    pub fn resume_job_with_max_turns(
        &self,
        job_id: &str,
        max_turns: Option<usize>,
    ) -> io::Result<TaskAgentRunResult> {
        let truth =
            ProcessTruthStore::new_with_state_root(&self.workspace_root, &self.state_root, job_id)?;
        let snapshot = truth.registry_snapshot()?;
        let job = snapshot.jobs.first().cloned().ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, format!("job not found: {job_id}"))
        })?;
        let process = snapshot
            .processes
            .iter()
            .find(|item| item.pid == job.root_pid)
            .cloned()
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("root process not found for job: {job_id}"),
                )
            })?;
        let current_status = truth.replay()?.status;
        if job_resume_is_terminal(&current_status) {
            return current_job_result(&truth, &job, &current_status);
        }
        truth.append_event(
            Some(&process.pid),
            "job_resumed",
            json!({"job_id": job_id, "resume_source": "ProcessTruth replay"}),
        )?;
        truth.update_job_status("running")?;
        let (model_config, config_ref) = self.load_model_invocation_config(&truth, &process.pid)?;
        self.run_task_agent(
            job.clone(),
            process,
            truth,
            &job.user_goal,
            max_turns,
            model_config,
            config_ref,
            None,
        )
    }

    pub fn approve_preview(
        &self,
        job_id: &str,
        approval_note: &str,
    ) -> io::Result<TaskAgentRunResult> {
        self.approve_preview_selection_with_max_turns(job_id, None, approval_note, None)
    }

    pub fn approve_preview_by_id(
        &self,
        job_id: &str,
        approval_id: &str,
        approval_note: &str,
    ) -> io::Result<TaskAgentRunResult> {
        self.approve_preview_selection_with_max_turns(
            job_id,
            Some(approval_id),
            approval_note,
            None,
        )
    }

    pub fn approve_preview_with_max_turns(
        &self,
        job_id: &str,
        approval_note: &str,
        max_turns: Option<usize>,
    ) -> io::Result<TaskAgentRunResult> {
        self.approve_preview_selection_with_max_turns(job_id, None, approval_note, max_turns)
    }

    pub fn approve_preview_by_id_with_max_turns(
        &self,
        job_id: &str,
        approval_id: &str,
        approval_note: &str,
        max_turns: Option<usize>,
    ) -> io::Result<TaskAgentRunResult> {
        self.approve_preview_selection_with_max_turns(
            job_id,
            Some(approval_id),
            approval_note,
            max_turns,
        )
    }

    fn approve_preview_selection_with_max_turns(
        &self,
        job_id: &str,
        approval_id: Option<&str>,
        approval_note: &str,
        max_turns: Option<usize>,
    ) -> io::Result<TaskAgentRunResult> {
        let truth =
            ProcessTruthStore::new_with_state_root(&self.workspace_root, &self.state_root, job_id)?;
        let snapshot = truth.registry_snapshot()?;
        let job = snapshot.jobs.first().cloned().ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, format!("job not found: {job_id}"))
        })?;
        let process = snapshot
            .processes
            .iter()
            .find(|item| item.pid == job.root_pid)
            .cloned()
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("root process not found for job: {job_id}"),
                )
            })?;
        let current_status = truth.replay()?.status;
        if current_status != "waiting_approval" {
            return current_job_result(&truth, &job, &current_status);
        }
        truth.append_event(
            Some(&job.root_pid),
            "user_approval_received",
            json!({
                "job_id": job_id,
                "approval_note": approval_note,
                "approval_source": "RootAgentProcessController",
            }),
        )?;
        let approval_runtime = ApprovalRuntime::new(truth.clone());
        let token = match approval_id {
            Some(approval_id) => approval_runtime.issue_token_for_preview(
                &job.root_pid,
                approval_id,
                approval_note,
            )?,
            None => {
                approval_runtime.issue_token_for_latest_preview(&job.root_pid, approval_note)?
            }
        };
        let provider_tool_executed =
            self.execute_approved_pending_provider_tool_call(&truth, &job, &process, &token)?;
        if !provider_tool_executed {
            record_provider_user_control_message(
                &truth,
                &job.root_pid,
                "deepseek",
                "deepseek_chat_completions",
                "approval_resume",
                &json!({
                    "event": "user_approved_preview",
                    "preview_id": token.preview_id.clone(),
                    "approval_token_id": token.approval_token_id.clone(),
                    "instruction": "The Process Kernel recorded the approval. If no pending provider-native tool_call is bound to this preview, continue from the current ProcessTruth state. Do not invent approval tokens or execute mutations outside Kernel capabilities.",
                }),
            )?;
        }
        truth.append_event(
            Some(&job.root_pid),
            "job_resume_approval_token_ready",
            json!({
                "job_id": job_id,
                "approval_token_id": token.approval_token_id,
                "preview_id": token.preview_id,
                "provider_tool_call_executed": provider_tool_executed,
            }),
        )?;
        self.resume_job_with_max_turns(job_id, max_turns)
    }

    pub fn submit_user_input(
        &self,
        job_id: &str,
        user_input: &str,
    ) -> io::Result<TaskAgentRunResult> {
        self.submit_user_input_with_max_turns(job_id, user_input, None)
    }

    pub fn submit_user_input_for_approval(
        &self,
        job_id: &str,
        approval_id: &str,
        user_input: &str,
    ) -> io::Result<TaskAgentRunResult> {
        self.submit_user_input_for_approval_with_max_turns(job_id, approval_id, user_input, None)
    }

    pub fn submit_user_input_with_max_turns(
        &self,
        job_id: &str,
        user_input: &str,
        max_turns: Option<usize>,
    ) -> io::Result<TaskAgentRunResult> {
        let truth =
            ProcessTruthStore::new_with_state_root(&self.workspace_root, &self.state_root, job_id)?;
        let snapshot = truth.registry_snapshot()?;
        let job = snapshot.jobs.first().cloned().ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, format!("job not found: {job_id}"))
        })?;
        let current_status = truth.replay()?.status;
        if job_resume_is_terminal(&current_status) {
            return current_job_result(&truth, &job, &current_status);
        }
        let input_ref = truth.write_blob("user_input/resume.txt", user_input.as_bytes())?;
        truth.append_event(
            Some(&job.root_pid),
            "user_input_received",
            json!({"job_id": job_id, "input_ref": input_ref.clone()}),
        )?;
        append_provider_visible_user_input(&truth, &job.root_pid, &input_ref, user_input)?;
        self.resume_job_with_max_turns(job_id, max_turns)
    }

    pub fn submit_user_input_for_approval_with_max_turns(
        &self,
        job_id: &str,
        approval_id: &str,
        user_input: &str,
        max_turns: Option<usize>,
    ) -> io::Result<TaskAgentRunResult> {
        let truth =
            ProcessTruthStore::new_with_state_root(&self.workspace_root, &self.state_root, job_id)?;
        let snapshot = truth.registry_snapshot()?;
        let job = snapshot.jobs.first().cloned().ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, format!("job not found: {job_id}"))
        })?;
        let current_status = truth.replay()?.status;
        if job_resume_is_terminal(&current_status) {
            return current_job_result(&truth, &job, &current_status);
        }
        let provider_tool_result_recorded =
            self.record_pending_provider_tool_user_response(&truth, &job, approval_id, user_input)?;
        if !provider_tool_result_recorded {
            return self.submit_user_input_with_max_turns(job_id, user_input, max_turns);
        }
        truth.append_event(
            Some(&job.root_pid),
            "user_input_received",
            json!({
                "job_id": job_id,
                "approval_id": approval_id,
                "input_ref": truth.write_blob("user_input/approval_response.txt", user_input.as_bytes())?,
                "provider_tool_result_recorded": true,
            }),
        )?;
        self.resume_job_with_max_turns(job_id, max_turns)
    }

    fn execute_approved_pending_provider_tool_call(
        &self,
        truth: &ProcessTruthStore,
        job: &AgentJob,
        process: &AgentProcess,
        token: &ApprovalTokenRecord,
    ) -> io::Result<bool> {
        let events = truth.read_events()?;
        if events.iter().any(|event| {
            matches!(
                event.event_type.as_str(),
                "provider_tool_call_approval_executed"
                    | "provider_terminal_tool_call_approval_executed"
            ) && event.data.get("preview_id").and_then(Value::as_str)
                == Some(token.preview_id.as_str())
        }) {
            return Ok(false);
        }
        let Some(pending) = events.iter().rev().find(|event| {
            if !matches!(
                event.event_type.as_str(),
                "provider_tool_call_waiting_approval"
                    | "provider_terminal_tool_call_waiting_approval"
            ) {
                return false;
            }
            pending_event_matches_approval(
                &event.data,
                token.preview_id.as_str(),
                token.tx_id.as_str(),
            )
        }) else {
            return Ok(false);
        };
        let capability_id = pending
            .data
            .get("capability_id")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "pending provider tool capability_id missing",
                )
            })?;
        let arguments = pending
            .data
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| json!({}));
        let registry = default_capability_registry();
        let descriptor = registry
            .iter()
            .find(|item| item.capability_id == capability_id)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("{capability_id} descriptor not found"),
                )
            })?;
        let capability_token = self.root_capability_token(job, process);
        let approval_request = build_capability_approval_request(
            truth,
            descriptor,
            &arguments,
            Some(token.approval_token_id.clone()),
        )?;
        let approval_guard =
            match prepare_capability_approval(truth, &capability_token, approval_request)? {
                Ok(guard) => guard,
                Err(receipt) => {
                    let tool_result = provider_tool_result_from_receipt_for_root(truth, &receipt)?;
                    if let Some(tool_call_id) = pending
                        .data
                        .get("provider_tool_call_id")
                        .and_then(Value::as_str)
                    {
                        record_provider_tool_result_with_metadata(
                            truth,
                            &job.root_pid,
                            "deepseek",
                            "deepseek_chat_completions",
                            tool_call_id,
                            &tool_result,
                            provider_tool_result_metadata_from_pending(&pending.data),
                        )?;
                    }
                    truth.append_event(
                        Some(&job.root_pid),
                        "provider_tool_call_approval_execution_blocked",
                        json!({
                            "preview_id": token.preview_id,
                            "approval_token_id": token.approval_token_id,
                            "capability_id": capability_id,
                            "receipt": receipt,
                        }),
                    )?;
                    return Ok(true);
                }
            };
        let receipt = self.execute_root_provider_capability(
            truth,
            capability_token,
            capability_id,
            &arguments,
            Some(token.approval_token_id.as_str()),
        )?;
        finalize_capability_approval(truth, &job.root_pid, approval_guard.as_ref(), &receipt)?;
        let tool_result = provider_tool_result_from_receipt_for_root(truth, &receipt)?;
        let tool_call_id = pending
            .data
            .get("provider_tool_call_id")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "pending provider tool_call id missing",
                )
            })?;
        record_provider_tool_result_with_metadata(
            truth,
            &job.root_pid,
            "deepseek",
            "deepseek_chat_completions",
            tool_call_id,
            &tool_result,
            provider_tool_result_metadata_from_pending(&pending.data),
        )?;
        truth.append_event(
            Some(&job.root_pid),
            "provider_tool_call_approval_executed",
            json!({
                "preview_id": token.preview_id,
                "tx_id": token.tx_id,
                "approval_token_id": token.approval_token_id,
                "capability_id": capability_id,
                "provider_tool_call_id": tool_call_id,
                "receipt_status": receipt.status,
                "receipt_ref": tool_result.get("receipt_ref").cloned(),
                "provider_tool_result_recorded": true,
            }),
        )?;
        Ok(true)
    }

    fn record_pending_provider_tool_user_response(
        &self,
        truth: &ProcessTruthStore,
        job: &AgentJob,
        approval_id: &str,
        user_input: &str,
    ) -> io::Result<bool> {
        let events = truth.read_events()?;
        let Some(pending) = events.iter().rev().find(|event| {
            if !matches!(
                event.event_type.as_str(),
                "provider_tool_call_waiting_approval"
                    | "provider_terminal_tool_call_waiting_approval"
            ) {
                return false;
            }
            pending_event_matches_approval(&event.data, approval_id, approval_id)
        }) else {
            return Ok(false);
        };
        let lower = user_input.to_ascii_lowercase();
        let status = if lower.contains("edited") || lower.contains("修改") {
            "edited_by_user"
        } else {
            "rejected"
        };
        let tool_call_id = pending
            .data
            .get("provider_tool_call_id")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "pending provider tool_call id missing",
                )
            })?;
        let capability_id = pending
            .data
            .get("capability_id")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let tool_result = json!({
            "status": status,
            "receipt_status": status,
            "capability_id": capability_id,
            "approval_id": approval_id,
            "tool_executed": false,
            "recoverable": true,
            "message": user_input,
            "next_model_request_should_self_correct": true,
            "arguments": pending.data.get("arguments").cloned(),
        });
        record_provider_tool_result_with_metadata(
            truth,
            &job.root_pid,
            "deepseek",
            "deepseek_chat_completions",
            tool_call_id,
            &tool_result,
            provider_tool_result_metadata_from_pending(&pending.data),
        )?;
        let preview_id = pending
            .data
            .get("preview_id")
            .and_then(|value| {
                value
                    .as_str()
                    .or_else(|| value.get("preview_id").and_then(Value::as_str))
            })
            .unwrap_or(approval_id);
        let tx_id = pending
            .data
            .get("preview_tx_id")
            .and_then(|value| {
                value
                    .as_str()
                    .or_else(|| value.get("preview_tx_id").and_then(Value::as_str))
            })
            .unwrap_or(approval_id);
        truth.append_event(
            Some(&job.root_pid),
            "preview_tx_closed",
            json!({
                "tx_id": tx_id,
                "preview_id": preview_id,
                "status": status,
                "approval_id": approval_id,
            }),
        )?;
        truth.append_event(
            Some(&job.root_pid),
            "provider_tool_call_user_response_recorded",
            json!({
                "provider_tool_call_id": tool_call_id,
                "preview_id": preview_id,
                "tx_id": tx_id,
                "approval_id": approval_id,
                "capability_id": capability_id,
                "status": status,
                "tool_executed": false,
                "provider_tool_result_recorded": true,
            }),
        )?;
        Ok(true)
    }

    fn execute_root_provider_capability(
        &self,
        truth: &ProcessTruthStore,
        capability_token: CapabilityToken,
        capability_id: &str,
        arguments: &Value,
        approval_id: Option<&str>,
    ) -> io::Result<CapabilityReceipt> {
        let guard = WorkspaceGuard::new(&self.workspace_root)?;
        let os = || OsRuntime::new(guard.clone(), truth.clone(), capability_token.clone());
        let data = || DataRuntime::new(guard.clone(), truth.clone(), capability_token.clone());
        let office = || {
            OfficeRuntime::new(
                guard.clone(),
                truth.clone(),
                capability_token.clone(),
                root_office_worker_project(),
            )
        };
        match capability_id {
            "terminal.run_command" => {
                let argv = root_string_array_arg(arguments, "argv");
                if argv.is_empty() {
                    return Err(io::Error::new(io::ErrorKind::InvalidInput, "argv missing"));
                }
                let timeout_ms = arguments
                    .get("timeout_ms")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| {
                        io::Error::new(io::ErrorKind::InvalidInput, "timeout_ms missing")
                    })?;
                TerminalRuntime::new(guard, truth.clone(), capability_token)
                    .run_command_with_approval(
                        argv,
                        timeout_ms,
                        approval_id
                            .map(TerminalApproval::approved)
                            .unwrap_or_else(TerminalApproval::none),
                    )
            }
            "terminal.start_service" => {
                let argv = root_string_array_arg(arguments, "argv");
                if argv.is_empty() {
                    return Err(io::Error::new(io::ErrorKind::InvalidInput, "argv missing"));
                }
                TerminalRuntime::new(guard, truth.clone(), capability_token).start_service(
                    &root_string_arg(arguments, "service_id")?,
                    argv,
                    arguments
                        .get("startup_timeout_ms")
                        .and_then(Value::as_u64)
                        .ok_or_else(|| {
                            io::Error::new(
                                io::ErrorKind::InvalidInput,
                                "startup_timeout_ms missing",
                            )
                        })?,
                    root_terminal_service_health_check_arg(arguments),
                    root_terminal_expected_ports_arg(arguments),
                )
            }
            "terminal.stop_service" => TerminalRuntime::new(guard, truth.clone(), capability_token)
                .stop_service(
                    &root_string_arg(arguments, "service_id")?,
                    arguments.get("reason").and_then(Value::as_str),
                ),
            "terminal.service_status" => {
                TerminalRuntime::new(guard, truth.clone(), capability_token)
                    .service_status(&root_string_arg(arguments, "service_id")?)
            }
            "os.write_file" => os().write_file_with_approval(
                &root_path_arg(arguments, "path")?,
                root_content_arg(truth, arguments)?.as_bytes(),
                root_string_arg(arguments, "write_kind")?.as_str(),
                approval_id,
            ),
            "os.write_artifact" => os().write_artifact(
                &root_path_arg(arguments, "path")?,
                root_content_arg(truth, arguments)?.as_bytes(),
            ),
            "os.write_temp_dataset" => os().write_temp_dataset(
                &root_path_arg(arguments, "path")?,
                root_content_arg(truth, arguments)?.as_bytes(),
            ),
            "os.write_source_mutation_apply" => os().write_source_mutation_apply(
                &root_path_arg(arguments, "path")?,
                root_content_arg(truth, arguments)?.as_bytes(),
            ),
            "os.copy_path" => os().copy_path_with_approval(
                &root_path_arg(arguments, "source_path")?,
                &root_path_arg(arguments, "destination_path")?,
                approval_id,
            ),
            "os.move_path" => os().move_path_with_approval(
                &root_path_arg(arguments, "source_path")?,
                &root_path_arg(arguments, "destination_path")?,
                approval_id,
            ),
            "os.rename_path" => os().rename_path_with_approval(
                &root_path_arg(arguments, "source_path")?,
                &root_path_arg(arguments, "destination_path")?,
                approval_id,
            ),
            "os.delete_path" => {
                os().delete_path_with_approval(&root_path_arg(arguments, "path")?, approval_id)
            }
            "os.zip" => {
                let source_paths = root_string_array_arg(arguments, "source_paths");
                let source_refs = source_paths.iter().map(String::as_str).collect::<Vec<_>>();
                os().zip_paths(
                    &source_refs,
                    &root_path_arg(arguments, "destination_zip_path")?,
                )
            }
            "os.unzip" => os().unzip_archive(
                &root_path_arg(arguments, "archive_path")?,
                &root_path_arg(arguments, "destination_dir")?,
            ),
            "os.rollback_tx" => os().rollback_tx(&root_string_arg(arguments, "tx_id")?),
            "workspace.apply_organize_tx" => data().apply_organize_tx(
                &root_string_arg(arguments, "organize_plan_ref")?,
                approval_id,
            ),
            "workspace.rename_batch_apply" => data()
                .rename_batch_apply(&root_string_arg(arguments, "rename_plan_ref")?, approval_id),
            "workspace.tree_index" => data().tree_index(
                &root_string_arg(arguments, "source_set_ref")?,
                arguments.get("tree_path").and_then(Value::as_str),
            ),
            "workspace.perf_inventory" => data().perf_inventory(
                &root_string_arg(arguments, "source_set_ref")?,
                arguments
                    .get("output_path")
                    .or_else(|| arguments.get("path"))
                    .and_then(Value::as_str),
                None,
            ),
            "dataset.export_csv" => data().export_dataset_csv(
                &root_string_arg(arguments, "dataset_ref")?,
                &root_path_arg_any(arguments, &["output_path", "path"])?,
            ),
            "dataset.export_markdown" => data().export_dataset_markdown(
                &root_string_arg(arguments, "dataset_ref")?,
                &root_path_arg_any(arguments, &["output_path", "path"])?,
                arguments
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or("Dataset Export"),
            ),
            "artifact.copy_source_set" => data().copy_source_set(
                &root_string_arg(arguments, "source_set_ref")?,
                &root_path_arg(arguments, "destination_dir")?,
            ),
            "office.docx.create" => office().create_docx(
                &root_path_arg_any(arguments, &["output_path", "path"])?,
                &root_content_arg(truth, arguments)?,
                arguments.get("title").and_then(Value::as_str),
            ),
            "office.docx.rewrite_save_as" => office().rewrite_save_as(
                &root_path_arg(arguments, "input_path")?,
                &root_path_arg(arguments, "output_path")?,
                &root_content_arg(truth, arguments)?,
            ),
            "office.docx.rewrite_in_place" => office().rewrite_in_place_with_approval(
                &root_path_arg(arguments, "input_path")?,
                &root_content_arg(truth, arguments)?,
                approval_id,
            ),
            "package.build_zip" => PackageRuntime::new(guard, truth.clone(), capability_token)
                .build_zip(
                    &root_string_arg(arguments, "source_set_ref")?,
                    &root_path_arg(arguments, "destination_zip_path")?,
                    arguments.get("manifest_path").and_then(Value::as_str),
                    arguments.get("checksums_path").and_then(Value::as_str),
                    arguments.get("perf_notes_path").and_then(Value::as_str),
                    &root_string_array_arg(arguments, "exclude_globs"),
                ),
            "artifact.verify_coverage"
            | "artifact.source_coverage_verify"
            | "artifact.verify_typed"
            | "artifact.audit_quality" => {
                ArtifactRuntime::new(guard, truth.clone(), capability_token).verify_typed_artifact(
                    &root_path_arg_any(arguments, &["artifact_path", "path"])?,
                )
            }
            other => Ok(CapabilityReceipt {
                capability_id: other.to_string(),
                job_id: capability_token.job_id.clone(),
                pid: capability_token.pid.clone(),
                status: "blocked".to_string(),
                data: json!({
                    "reason": "approved provider capability is not dispatchable by RootAgentProcessController",
                    "capability_id": other,
                    "approval_id": approval_id,
                }),
            }),
        }
    }

    pub fn cancel_job(&self, job_id: &str, reason: &str) -> io::Result<()> {
        let truth =
            ProcessTruthStore::new_with_state_root(&self.workspace_root, &self.state_root, job_id)?;
        let snapshot = truth.registry_snapshot()?;
        let root_pid = snapshot
            .jobs
            .first()
            .map(|job| job.root_pid.clone())
            .unwrap_or_default();
        truth.append_event(
            Some(&root_pid),
            "user_cancel_requested",
            json!({
                "job_id": job_id,
                "reason": reason,
                "user_forced": true,
                "user_facing_reason": "用户强制关闭",
            }),
        )?;
        let _ = stop_terminal_services_for_job(&truth, &root_pid, reason)?;
        truth.update_job_status("cancelled")?;
        truth.append_event(
            Some(&root_pid),
            "job_cancelled",
            json!({
                "job_id": job_id,
                "reason": reason,
                "status": "cancelled",
                "user_forced": true,
                "user_facing_reason": "用户强制关闭",
            }),
        )?;
        Ok(())
    }

    fn run_task_agent(
        &self,
        job: AgentJob,
        process: AgentProcess,
        truth: ProcessTruthStore,
        goal: &str,
        max_turns: Option<usize>,
        model_config: ModelInvocationConfig,
        model_invocation_config_ref: Option<String>,
        model_stream_sink: Option<Arc<dyn ModelStreamSink>>,
    ) -> io::Result<TaskAgentRunResult> {
        let snapshot = truth.registry_snapshot()?;
        let runtime_id = snapshot
            .task_agent_runtimes
            .iter()
            .find(|item| item.root_pid == process.pid)
            .map(|item| item.runtime_id.clone())
            .unwrap_or_else(|| format!("tar_{}", process.pid));
        let token = self.root_capability_token(&job, &process);
        truth.append_event(
            Some(&process.pid),
            "capability_token_issued",
            to_json_value(&token)?,
        )?;
        let runtime = TaskAgentRuntime::new(
            WorkspaceGuard::new(&self.workspace_root)?,
            truth,
            token,
            runtime_id,
            Some(self.model_provider.clone()),
            model_config,
            model_invocation_config_ref,
            model_stream_sink,
        );
        let runtime = if let Some(max_turns) = max_turns {
            runtime.with_max_turns(max_turns)
        } else {
            runtime
        };
        runtime.run(goal)
    }

    fn record_model_invocation_config(
        &self,
        truth: &ProcessTruthStore,
        pid: &str,
        model_config: &ModelInvocationConfig,
    ) -> io::Result<String> {
        let config_ref = truth.write_blob(
            "model_config/model_invocation_config.json",
            &serde_json::to_vec_pretty(model_config).map_err(crate::json_err)?,
        )?;
        truth.append_event(
            Some(pid),
            "model_invocation_config_recorded",
            json!({
                "model_invocation_config_ref": config_ref.clone(),
                "model_invocation_config": model_config,
            }),
        )?;
        truth.append_event(
            Some(pid),
            "model_config_bound",
            json!({
                "schema": "supernova_model_config_bound.v1",
                "model_invocation_config_ref": config_ref.clone(),
                "effective_config": model_config.redacted_binding_summary(),
                "fact_boundary": "Model config is bound as provider-visible invocation configuration. Actual provider/model request facts are recorded by model_call_started and model_call_ledger.",
            }),
        )?;
        Ok(config_ref)
    }

    fn load_model_invocation_config(
        &self,
        truth: &ProcessTruthStore,
        pid: &str,
    ) -> io::Result<(ModelInvocationConfig, Option<String>)> {
        let mut config_ref: Option<String> = None;
        for event in truth.read_events()? {
            if event.event_type == "model_invocation_config_recorded" {
                if let Some(value) = event
                    .data
                    .get("model_invocation_config_ref")
                    .and_then(serde_json::Value::as_str)
                {
                    config_ref = Some(value.to_string());
                }
            }
        }
        let Some(config_ref) = config_ref else {
            let config = ModelInvocationConfig::from_env();
            let written_ref = self.record_model_invocation_config(truth, pid, &config)?;
            return Ok((config, Some(written_ref)));
        };
        let path = truth.resolve_blob_ref(&config_ref)?;
        let mut config = serde_json::from_slice::<ModelInvocationConfig>(&std::fs::read(path)?)
            .map_err(crate::json_err)?;
        config.enforce_task_agent_provider_native_tools();
        Ok((config, Some(config_ref)))
    }

    fn root_capability_token(&self, job: &AgentJob, process: &AgentProcess) -> CapabilityToken {
        let registry = default_capability_registry();
        let capabilities = registry
            .iter()
            .map(|item| item.capability_id.clone())
            .collect::<Vec<_>>();
        let permissions = registry
            .iter()
            .flat_map(|item| item.required_permissions.iter().cloned())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        CapabilityToken {
            token_id: format!("token_root_{}", process.pid),
            job_id: job.job_id.clone(),
            pid: process.pid.clone(),
            workspace_root: self.workspace_root.display().to_string(),
            capabilities,
            permissions,
        }
    }
}

fn job_resume_is_terminal(status: &str) -> bool {
    matches!(status, "completed" | "cancelled")
}

fn current_job_result(
    truth: &ProcessTruthStore,
    job: &AgentJob,
    status: &str,
) -> io::Result<TaskAgentRunResult> {
    let replay = truth.replay()?;
    let snapshot = truth.registry_snapshot()?;
    let runtime_id = snapshot
        .task_agent_runtimes
        .iter()
        .find(|item| item.job_id == job.job_id)
        .map(|item| item.runtime_id.clone())
        .unwrap_or_default();
    let checkpoints = truth
        .list_checkpoints()?
        .into_iter()
        .map(|checkpoint| CheckpointRef {
            checkpoint_id: checkpoint.checkpoint_id,
            job_id: checkpoint.job_id,
            pid: checkpoint.pid,
            runtime_id: checkpoint.runtime_id,
            kind: checkpoint.kind,
            state_ref: checkpoint.state_ref,
        })
        .collect::<Vec<_>>();
    let turn_count = truth
        .read_events()?
        .iter()
        .filter(|event| event.event_type == "task_agent_turn_completed")
        .count();
    Ok(TaskAgentRunResult {
        job_id: job.job_id.clone(),
        root_pid: job.root_pid.clone(),
        runtime_id,
        status: status.to_string(),
        artifacts: replay.artifact_refs,
        checkpoints,
        turn_count,
        waiting_for: (status == "waiting_approval").then(|| "approval".to_string()),
        last_error: None,
    })
}

fn provider_tool_result_from_receipt_for_root(
    truth: &ProcessTruthStore,
    receipt: &CapabilityReceipt,
) -> io::Result<Value> {
    let receipt_ref = truth.write_blob(
        &format!(
            "provider_tool_results/root_{}_receipt.json",
            safe_blob_name(&now_ms().to_string())
        ),
        &serde_json::to_vec_pretty(receipt).map_err(crate::json_err)?,
    )?;
    let receipt_value = to_json_value(receipt)?;
    let mut result = json!({
        "status": receipt.status.as_str(),
        "receipt_status": receipt.status.clone(),
        "capability_id": receipt.capability_id.clone(),
        "receipt_ref": receipt_ref,
        "receipt": receipt_value,
    });
    if let Some(object) = result.as_object_mut() {
        for key in [
            "stdout_ref",
            "stderr_ref",
            "stdout_bytes",
            "stderr_bytes",
            "exit_code",
            "timed_out",
            "duration_ms",
            "workspace_diff_ref",
            "approval_id",
            "preview_id",
            "target_paths",
            "reason",
            "approval_required",
            "mutation_detected",
        ] {
            if let Some(value) = receipt.data.get(key) {
                object.insert(key.to_string(), value.clone());
            }
        }
        if receipt.capability_id == "terminal.run_command" {
            if let Some(stdout) = receipt_blob_text_for_root(truth, receipt, "stdout_ref")? {
                object.insert("stdout_text".to_string(), json!(stdout));
            }
            if let Some(stderr) = receipt_blob_text_for_root(truth, receipt, "stderr_ref")? {
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

fn receipt_blob_text_for_root(
    truth: &ProcessTruthStore,
    receipt: &CapabilityReceipt,
    ref_key: &str,
) -> io::Result<Option<String>> {
    let Some(blob_ref) = receipt.data.get(ref_key).and_then(Value::as_str) else {
        return Ok(None);
    };
    let path = truth.resolve_blob_ref(blob_ref)?;
    let bytes = std::fs::read(path)?;
    Ok(Some(String::from_utf8_lossy(&bytes).to_string()))
}

fn provider_tool_result_metadata_from_pending(data: &Value) -> ProviderToolResultMetadata {
    ProviderToolResultMetadata {
        provider_tool_call_index: data
            .get("provider_tool_call_index")
            .and_then(Value::as_u64)
            .map(|value| value as usize),
        provider_tool_batch_id: data
            .get("provider_tool_batch_id")
            .and_then(Value::as_str)
            .map(ToString::to_string),
    }
}

fn pending_event_matches_approval(data: &Value, preview_id: &str, preview_tx_id: &str) -> bool {
    let event_preview_id = data
        .get("preview_id")
        .and_then(|value| {
            value
                .as_str()
                .or_else(|| value.get("preview_id").and_then(Value::as_str))
        })
        .unwrap_or("");
    let event_preview_tx_id = data
        .get("preview_tx_id")
        .and_then(|value| {
            value
                .as_str()
                .or_else(|| value.get("preview_tx_id").and_then(Value::as_str))
        })
        .unwrap_or("");
    event_preview_id == preview_id
        || event_preview_tx_id == preview_tx_id
        || event_preview_id == preview_tx_id
        || event_preview_tx_id == preview_id
}

fn root_string_arg(arguments: &Value, key: &str) -> io::Result<String> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, format!("{key} missing")))
}

fn root_path_arg(arguments: &Value, key: &str) -> io::Result<String> {
    root_string_arg(arguments, key).map(|value| value.replace('\\', "/"))
}

fn root_path_arg_any(arguments: &Value, keys: &[&str]) -> io::Result<String> {
    keys.iter()
        .find_map(|key| root_path_arg(arguments, key).ok())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("one of {} missing", keys.join(", ")),
            )
        })
}

fn root_string_array_arg(arguments: &Value, key: &str) -> Vec<String> {
    arguments
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn root_terminal_service_health_check_arg(arguments: &Value) -> Option<TerminalServiceHealthCheck> {
    let health = arguments.get("health_check")?.as_object()?;
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

fn root_terminal_expected_ports_arg(arguments: &Value) -> Vec<u16> {
    arguments
        .get("expected_ports")
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

pub(crate) fn append_provider_visible_user_input(
    truth: &ProcessTruthStore,
    root_pid: &str,
    input_ref: &str,
    user_input: &str,
) -> io::Result<bool> {
    record_provider_user_control_message(
        truth,
        root_pid,
        "deepseek",
        "deepseek_chat_completions",
        "user_clarification_resume",
        &json!({
            "event": "user_clarification_received",
            "input_ref": input_ref,
            "user_input": user_input,
            "instruction": "This is the user's latest clarification or answer for the active task. Treat it as provider-visible user input for the next step; do not search ProcessTruth for this answer unless additional historical context is needed.",
        }),
    )
    .map(|record| record.is_some())
}

fn root_content_arg(truth: &ProcessTruthStore, arguments: &Value) -> io::Result<String> {
    for key in ["content", "text", "bytes"] {
        if let Some(value) = arguments.get(key).and_then(Value::as_str) {
            return Ok(value.to_string());
        }
    }
    for key in ["content_ref", "text_ref", "source_ref"] {
        if let Some(blob_ref) = arguments.get(key).and_then(Value::as_str) {
            let path = truth.resolve_blob_ref(blob_ref)?;
            let bytes = std::fs::read(path)?;
            return Ok(String::from_utf8_lossy(&bytes).to_string());
        }
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidInput,
        "content/text/content_ref/text_ref missing",
    ))
}

fn root_office_worker_project() -> PathBuf {
    std::env::var("SUPERNOVA_OFFICE_WORKER_PROJECT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .join("office_worker")
                .join("SuperNova.OfficeWorker")
                .join("SuperNova.OfficeWorker.csproj")
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model_runtime::DeterministicModelProvider;

    fn temp_workspace(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "supernova_root_process_{}_{}",
            name,
            crate::now_ms()
        ));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn approve_preview_is_noop_for_completed_job() {
        let workspace = temp_workspace("completed_approval_noop");
        let state_root = workspace.join(".state");
        let (job, _process, truth) =
            crate::create_agent_job_with_state_root(&workspace, &state_root, "done").unwrap();
        truth.update_job_status("completed").unwrap();
        let controller = RootAgentProcessController::with_model_provider_and_state_root(
            &workspace,
            &state_root,
            Arc::new(DeterministicModelProvider::new(
                "deterministic",
                "test-model",
            )),
        )
        .unwrap();

        let result = controller
            .approve_preview(&job.job_id, "late approval")
            .unwrap();

        assert_eq!(result.status, "completed");
        let events = truth.read_events().unwrap();
        assert!(!events
            .iter()
            .any(|event| event.event_type == "user_approval_received"));
        assert!(!events.iter().any(|event| event.event_type == "job_resumed"));
    }

    #[test]
    fn user_input_resume_is_appended_to_provider_visible_transcript() {
        let workspace = temp_workspace("user_input_provider_visible");
        let state_root = workspace.join(".state");
        let (_job, process, truth) =
            crate::create_agent_job_with_state_root(&workspace, &state_root, "needs input")
                .unwrap();
        crate::record_provider_user_message(
            &truth,
            &process.pid,
            "deepseek",
            "deepseek_chat_completions",
            "model_call_initial",
            "Initial task request",
        )
        .unwrap();
        let input_ref = truth
            .write_blob(
                "user_input/resume.txt",
                "delete only a.txt and b.txt".as_bytes(),
            )
            .unwrap();

        let appended = append_provider_visible_user_input(
            &truth,
            &process.pid,
            &input_ref,
            "delete only a.txt and b.txt",
        )
        .unwrap();

        assert!(appended);
        let state = crate::replay_provider_transcript_state(
            &truth,
            "deepseek",
            "deepseek_chat_completions",
        )
        .unwrap()
        .expect("provider transcript exists");
        let messages_text =
            std::fs::read_to_string(truth.resolve_blob_ref(&state.messages_ref).unwrap()).unwrap();
        assert!(messages_text.contains("user_clarification_received"));
        assert!(messages_text.contains("delete only a.txt and b.txt"));
        assert!(truth.read_events().unwrap().iter().any(|event| {
            event.event_type == "provider_user_control_message_recorded"
                && event.data["control_kind"] == "user_clarification_resume"
        }));
    }
}
