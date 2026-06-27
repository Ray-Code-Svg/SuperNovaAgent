use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::agent_container::{
    AgentContainer, AgentContainerStatus, ContainerTimelineItem, ContainerTimelineItemKind,
};
use crate::chat_runtime::{ChatRuntime, ChatTurnRequest, ChatTurnResult};
use crate::chat_truth::{ChatEvent, ChatThread, ChatTruthStore};
use crate::container_context::build_context_pack_visible_payload;
use crate::container_store::ContainerStore;
use crate::context_compaction::ContextCompactionReceipt;
use crate::context_pack::ContextPack;
use crate::context_window::{
    ContextScope, ContextWindowControlConfig, ContextWindowController, ContextWindowScopeAdapter,
};
use crate::root_process::RootAgentProcessController;
use crate::task_agent::TaskAgentRunResult;
use crate::{
    default_model_provider_from_env, model_provider_from_profile_root_or_env,
    ArtifactDestinationGuidance, CapabilityToken, ClientEnvRuntime, ContainerContextWindowAdapter,
    ModelAction, ModelFailurePolicy, ModelInvocationConfig, ModelOperation, ModelProvider,
    ModelRuntime, ModelStreamSink, ProcessEvent, ProcessTruthStore, ProviderCredentialStore,
    ProviderProfileRecord, ProviderTestReceipt, ReplaySession, SourceGuidance,
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct JobStatusView {
    pub job_id: String,
    pub status: String,
    pub event_count: usize,
    pub artifact_refs: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct KernelApi {
    controller: RootAgentProcessController,
    model_provider: Arc<dyn ModelProvider>,
    provider_profile_root: Option<PathBuf>,
}

impl KernelApi {
    pub fn new(workspace_root: impl AsRef<Path>) -> io::Result<Self> {
        let model_provider = default_model_provider_from_env();
        Ok(Self {
            controller: RootAgentProcessController::with_model_provider(
                workspace_root,
                model_provider.clone(),
            )?,
            model_provider,
            provider_profile_root: None,
        })
    }

    pub fn new_with_state_root(
        workspace_root: impl AsRef<Path>,
        state_root: impl AsRef<Path>,
    ) -> io::Result<Self> {
        let model_provider = default_model_provider_from_env();
        Ok(Self {
            controller: RootAgentProcessController::with_model_provider_and_state_root(
                workspace_root,
                state_root,
                model_provider.clone(),
            )?,
            model_provider,
            provider_profile_root: None,
        })
    }

    pub fn new_with_state_root_and_provider_profile_root(
        workspace_root: impl AsRef<Path>,
        state_root: impl AsRef<Path>,
        provider_profile_root: impl AsRef<Path>,
    ) -> io::Result<Self> {
        let provider_profile_root = provider_profile_root.as_ref().to_path_buf();
        let model_provider = model_provider_from_profile_root_or_env(&provider_profile_root);
        Ok(Self {
            controller: RootAgentProcessController::with_model_provider_and_state_root(
                workspace_root,
                state_root,
                model_provider.clone(),
            )?,
            model_provider,
            provider_profile_root: Some(provider_profile_root),
        })
    }

    fn workspace_root(&self) -> &Path {
        self.controller.workspace_root()
    }

    fn state_root(&self) -> &Path {
        self.controller.state_root()
    }

    fn container_store(&self) -> io::Result<ContainerStore> {
        ContainerStore::new_with_state_root(self.workspace_root(), self.state_root())
    }

    fn chat_truth(&self) -> io::Result<ChatTruthStore> {
        ChatTruthStore::new_with_state_root(self.workspace_root(), self.state_root())
    }

    fn process_truth(&self, job_id: &str) -> io::Result<ProcessTruthStore> {
        ProcessTruthStore::new_with_state_root(self.workspace_root(), self.state_root(), job_id)
    }

    fn bound_container_id_for_job(&self, job_id: &str) -> io::Result<Option<String>> {
        let truth = self.process_truth(job_id)?;
        let events = truth.read_events()?;
        Ok(events
            .into_iter()
            .rev()
            .find(|event| event.event_type == "agent_container_task_bound")
            .and_then(|event| {
                event
                    .data
                    .get("container_id")
                    .and_then(Value::as_str)
                    .map(ToString::to_string)
            }))
    }

    fn reconciled_task_status(&self, result: &TaskAgentRunResult) -> String {
        self.process_truth(&result.job_id)
            .and_then(|truth| truth.replay())
            .map(|replay| replay.status)
            .unwrap_or_else(|_| result.status.clone())
    }

    fn sync_task_timeline_status(&self, job_id: &str, status: &str) -> io::Result<()> {
        let Some(container_id) = self.bound_container_id_for_job(job_id)? else {
            return Ok(());
        };
        self.container_store()?.upsert_timeline_item(
            &container_id,
            ContainerTimelineItemKind::TaskRun,
            job_id.to_string(),
            status.to_string(),
            None,
            None,
        )?;
        Ok(())
    }

    fn sync_task_timeline_result(&self, result: &TaskAgentRunResult) -> io::Result<()> {
        let status = self.reconciled_task_status(result);
        self.sync_task_timeline_status(&result.job_id, &status)
    }

    fn chat_runtime(&self) -> io::Result<ChatRuntime> {
        ChatRuntime::with_model_provider_and_state_root(
            self.workspace_root(),
            self.state_root(),
            self.model_provider.clone(),
        )
    }

    fn provider_store(&self) -> ProviderCredentialStore {
        ProviderCredentialStore::new(
            self.provider_profile_root
                .clone()
                .unwrap_or_else(|| self.state_root().join("provider_credentials")),
        )
    }

    pub fn list_provider_profiles(&self) -> io::Result<Vec<ProviderProfileRecord>> {
        self.provider_store().list_profiles()
    }

    pub fn save_provider_profile(
        &self,
        provider_id: &str,
        api_base_url: Option<String>,
        api_key: Option<String>,
    ) -> io::Result<ProviderProfileRecord> {
        self.provider_store()
            .save_provider_profile(provider_id, api_base_url, api_key)
    }

    pub fn delete_provider_profile(&self, provider_id: &str) -> io::Result<()> {
        self.provider_store().delete_provider_profile(provider_id)
    }

    pub fn test_provider_profile(
        &self,
        provider_id: &str,
        live_check: bool,
    ) -> io::Result<ProviderTestReceipt> {
        self.provider_store().test_provider(provider_id, live_check)
    }

    pub fn start_job(&self, goal: &str) -> io::Result<TaskAgentRunResult> {
        self.controller.start_job(goal)
    }

    pub fn start_job_with_max_turns(
        &self,
        goal: &str,
        max_turns: Option<usize>,
    ) -> io::Result<TaskAgentRunResult> {
        self.controller.start_job_with_max_turns(goal, max_turns)
    }

    pub fn start_job_with_config(
        &self,
        goal: &str,
        max_turns: Option<usize>,
        model_config: ModelInvocationConfig,
    ) -> io::Result<TaskAgentRunResult> {
        self.controller
            .start_job_with_config(goal, max_turns, model_config)
    }

    pub fn start_task_in_container(
        &self,
        container_id: &str,
        goal: &str,
        max_turns: Option<usize>,
        model_config: ModelInvocationConfig,
    ) -> io::Result<TaskAgentRunResult> {
        self.start_task_in_container_with_options(
            container_id,
            goal,
            max_turns,
            model_config,
            None,
            false,
        )
    }

    pub fn start_task_in_container_with_options(
        &self,
        container_id: &str,
        goal: &str,
        max_turns: Option<usize>,
        model_config: ModelInvocationConfig,
        context_pack_id: Option<String>,
        auto_approve: bool,
    ) -> io::Result<TaskAgentRunResult> {
        self.start_task_in_container_with_options_and_started(
            container_id,
            goal,
            max_turns,
            model_config,
            context_pack_id,
            auto_approve,
            |_job_id, _root_pid| Ok(()),
        )
    }

    pub fn start_task_in_container_with_options_and_started<F>(
        &self,
        container_id: &str,
        goal: &str,
        max_turns: Option<usize>,
        model_config: ModelInvocationConfig,
        context_pack_id: Option<String>,
        auto_approve: bool,
        on_started: F,
    ) -> io::Result<TaskAgentRunResult>
    where
        F: FnOnce(&str, &str) -> io::Result<()>,
    {
        self.start_task_in_container_with_guidance_and_started(
            container_id,
            goal,
            max_turns,
            model_config,
            context_pack_id,
            None,
            None,
            auto_approve,
            on_started,
        )
    }

    pub fn start_task_in_container_with_guidance_and_started<F>(
        &self,
        container_id: &str,
        goal: &str,
        max_turns: Option<usize>,
        model_config: ModelInvocationConfig,
        context_pack_id: Option<String>,
        source_guidance: Option<SourceGuidance>,
        artifact_destination: Option<ArtifactDestinationGuidance>,
        auto_approve: bool,
        on_started: F,
    ) -> io::Result<TaskAgentRunResult>
    where
        F: FnOnce(&str, &str) -> io::Result<()>,
    {
        self.start_task_in_container_with_guidance_started_and_stream_sink(
            container_id,
            goal,
            max_turns,
            model_config,
            context_pack_id,
            source_guidance,
            artifact_destination,
            auto_approve,
            on_started,
            None,
        )
    }

    pub fn start_task_in_container_with_guidance_started_and_stream_sink<F>(
        &self,
        container_id: &str,
        goal: &str,
        max_turns: Option<usize>,
        model_config: ModelInvocationConfig,
        context_pack_id: Option<String>,
        source_guidance: Option<SourceGuidance>,
        artifact_destination: Option<ArtifactDestinationGuidance>,
        auto_approve: bool,
        on_started: F,
        model_stream_sink: Option<Arc<dyn ModelStreamSink>>,
    ) -> io::Result<TaskAgentRunResult>
    where
        F: FnOnce(&str, &str) -> io::Result<()>,
    {
        let context_pack = if let Some(context_pack_id) = context_pack_id.as_deref() {
            Some(self.container_store()?.get_context_pack(context_pack_id)?)
        } else {
            None
        };
        let initial_context = context_pack.as_ref().map(|pack| {
            let visible_context_pack = build_context_pack_visible_payload(
                self.controller.workspace_root(),
                self.controller.state_root(),
                pack,
            )
            .unwrap_or_else(|err| {
                json!({
                    "schema": "supernova_context_pack_visible_payload.v1",
                    "container_id": pack.container_id.clone(),
                    "context_pack_id": pack.context_pack_id.clone(),
                    "resolution": "failed",
                    "error": err.to_string(),
                })
            });
            json!({
                "schema": "supernova_container_task_initial_context.v1",
                "container_id": container_id,
                "context_pack_id": pack.context_pack_id.clone(),
                "context_pack": pack.clone(),
                "visible_context_pack": visible_context_pack,
                "auto_approve": auto_approve,
                "goal": goal,
            })
        });
        let container_store = self.container_store()?;
        let started_container_store = container_store.clone();
        let container_id_for_started = container_id.to_string();
        let task_title = goal.chars().take(80).collect::<String>();
        let task_title_for_started = task_title.clone();
        let result = self
            .controller
            .start_job_with_config_initial_context_started_and_stream_sink(
                goal,
                max_turns,
                model_config,
                initial_context,
                |job, process, truth| {
                    started_container_store.upsert_timeline_item(
                        &container_id_for_started,
                        ContainerTimelineItemKind::TaskRun,
                        job.job_id.clone(),
                        "running",
                        Some(task_title_for_started),
                        None,
                    )?;
                    if let Some(guidance) = source_guidance
                        .clone()
                        .map(SourceGuidance::normalized)
                        .filter(SourceGuidance::is_effective)
                    {
                        let guidance_ref = truth.write_blob(
                            &format!("guidance/task_reference_sources_{}.txt", crate::now_ms()),
                            guidance.provider_visible_text().as_bytes(),
                        )?;
                        truth.append_event(
                            Some(&process.pid),
                            "task_reference_sources_attached",
                            guidance.audit_payload(guidance_ref),
                        )?;
                    }
                    if let Some(destination) = artifact_destination
                        .clone()
                        .map(ArtifactDestinationGuidance::normalized)
                        .filter(ArtifactDestinationGuidance::is_effective)
                    {
                        let guidance_ref = truth.write_blob(
                            &format!("guidance/task_artifact_destination_{}.txt", crate::now_ms()),
                            destination.provider_visible_text().as_bytes(),
                        )?;
                        truth.append_event(
                            Some(&process.pid),
                            "task_artifact_destination_guidance_attached",
                            destination.audit_payload(guidance_ref),
                        )?;
                    }
                    on_started(&job.job_id, &process.pid)
                },
                model_stream_sink,
            )?;
        container_store.upsert_timeline_item(
            container_id,
            ContainerTimelineItemKind::TaskRun,
            result.job_id.clone(),
            result.status.clone(),
            Some(task_title),
            None,
        )?;
        let truth = self.process_truth(&result.job_id)?;
        truth.append_event(
            Some(&result.root_pid),
            "agent_container_task_bound",
            json!({
                "container_id": container_id,
                "job_id": result.job_id.clone(),
                "status": result.status.clone(),
                "context_pack_id": context_pack_id,
                "auto_approve": auto_approve,
            }),
        )?;
        Ok(result)
    }

    pub fn resume_job(&self, job_id: &str) -> io::Result<TaskAgentRunResult> {
        let result = self.controller.resume_job(job_id)?;
        self.sync_task_timeline_result(&result)?;
        Ok(result)
    }

    pub fn resume_job_with_max_turns(
        &self,
        job_id: &str,
        max_turns: Option<usize>,
    ) -> io::Result<TaskAgentRunResult> {
        let result = self
            .controller
            .resume_job_with_max_turns(job_id, max_turns)?;
        self.sync_task_timeline_result(&result)?;
        Ok(result)
    }

    pub fn approve_preview(
        &self,
        job_id: &str,
        approval_note: &str,
    ) -> io::Result<TaskAgentRunResult> {
        let result = self.controller.approve_preview(job_id, approval_note)?;
        self.sync_task_timeline_result(&result)?;
        Ok(result)
    }

    pub fn approve_preview_by_id(
        &self,
        job_id: &str,
        approval_id: &str,
        approval_note: &str,
    ) -> io::Result<TaskAgentRunResult> {
        let result = self
            .controller
            .approve_preview_by_id(job_id, approval_id, approval_note)?;
        self.sync_task_timeline_result(&result)?;
        Ok(result)
    }

    pub fn approve_preview_with_max_turns(
        &self,
        job_id: &str,
        approval_note: &str,
        max_turns: Option<usize>,
    ) -> io::Result<TaskAgentRunResult> {
        let result =
            self.controller
                .approve_preview_with_max_turns(job_id, approval_note, max_turns)?;
        self.sync_task_timeline_result(&result)?;
        Ok(result)
    }

    pub fn approve_preview_by_id_with_max_turns(
        &self,
        job_id: &str,
        approval_id: &str,
        approval_note: &str,
        max_turns: Option<usize>,
    ) -> io::Result<TaskAgentRunResult> {
        let result = self.controller.approve_preview_by_id_with_max_turns(
            job_id,
            approval_id,
            approval_note,
            max_turns,
        )?;
        self.sync_task_timeline_result(&result)?;
        Ok(result)
    }

    pub fn approve_client_env_disclosure(
        &self,
        job_id: &str,
        request_id: &str,
        allowed_fields: Vec<String>,
        note: &str,
    ) -> io::Result<Value> {
        let truth = self.process_truth(job_id)?;
        let pid = root_pid_for_truth(&truth).unwrap_or_else(|| format!("pid_client_env_{job_id}"));
        let token = ClientEnvRuntime::approve_sensitive_disclosure(
            &truth,
            &pid,
            request_id,
            allowed_fields,
            note,
        )?;
        Ok(json!({
            "job_id": job_id,
            "request_id": request_id,
            "authorization_id": token.authorization_id,
            "allowed_fields": token.allowed_fields,
            "expires_at_unix_ms": token.expires_at_unix_ms,
            "user_approved": true,
        }))
    }

    pub fn reject_client_env_disclosure(
        &self,
        job_id: &str,
        request_id: &str,
        reason: &str,
    ) -> io::Result<Value> {
        let truth = self.process_truth(job_id)?;
        let pid = root_pid_for_truth(&truth).unwrap_or_else(|| format!("pid_client_env_{job_id}"));
        ClientEnvRuntime::reject_sensitive_disclosure(&truth, &pid, request_id, reason)?;
        Ok(json!({
            "job_id": job_id,
            "request_id": request_id,
            "user_approved": false,
            "reason": reason,
        }))
    }

    pub fn submit_user_input(
        &self,
        job_id: &str,
        user_input: &str,
    ) -> io::Result<TaskAgentRunResult> {
        let result = self.controller.submit_user_input(job_id, user_input)?;
        self.sync_task_timeline_result(&result)?;
        Ok(result)
    }

    pub fn submit_user_input_for_approval(
        &self,
        job_id: &str,
        approval_id: &str,
        user_input: &str,
    ) -> io::Result<TaskAgentRunResult> {
        let result =
            self.controller
                .submit_user_input_for_approval(job_id, approval_id, user_input)?;
        self.sync_task_timeline_result(&result)?;
        Ok(result)
    }

    pub fn submit_user_input_with_max_turns(
        &self,
        job_id: &str,
        user_input: &str,
        max_turns: Option<usize>,
    ) -> io::Result<TaskAgentRunResult> {
        let result = self
            .controller
            .submit_user_input_with_max_turns(job_id, user_input, max_turns)?;
        self.sync_task_timeline_result(&result)?;
        Ok(result)
    }

    pub fn submit_user_input_for_approval_with_max_turns(
        &self,
        job_id: &str,
        approval_id: &str,
        user_input: &str,
        max_turns: Option<usize>,
    ) -> io::Result<TaskAgentRunResult> {
        let result = self
            .controller
            .submit_user_input_for_approval_with_max_turns(
                job_id,
                approval_id,
                user_input,
                max_turns,
            )?;
        self.sync_task_timeline_result(&result)?;
        Ok(result)
    }

    pub fn cancel_job(&self, job_id: &str, reason: &str) -> io::Result<()> {
        self.controller.cancel_job(job_id, reason)?;
        self.sync_task_timeline_status(job_id, "cancelled")
    }

    pub fn get_job_status(
        &self,
        _workspace_root: impl AsRef<Path>,
        job_id: &str,
    ) -> io::Result<JobStatusView> {
        let truth = self.process_truth(job_id)?;
        let replay = truth.replay()?;
        Ok(JobStatusView {
            job_id: job_id.to_string(),
            status: replay.status,
            event_count: replay.event_count,
            artifact_refs: replay.artifact_refs,
        })
    }

    pub fn stream_job_events(
        &self,
        _workspace_root: impl AsRef<Path>,
        job_id: &str,
        after_event_id: u64,
        limit: usize,
    ) -> io::Result<Vec<ProcessEvent>> {
        self.process_truth(job_id)?
            .stream_events(after_event_id, limit)
    }

    pub fn replay_job(
        &self,
        _workspace_root: impl AsRef<Path>,
        job_id: &str,
    ) -> io::Result<ReplaySession> {
        self.process_truth(job_id)?.replay_session()
    }

    pub fn create_container(
        &self,
        title: Option<String>,
        default_model_config: Option<ModelInvocationConfig>,
        context_policy: Option<ContextWindowControlConfig>,
    ) -> io::Result<AgentContainer> {
        self.container_store()?
            .create_container(title, default_model_config, context_policy)
    }

    pub fn get_container(&self, container_id: &str) -> io::Result<AgentContainer> {
        self.container_store()?.get_container(container_id)
    }

    pub fn list_containers(&self) -> io::Result<Vec<AgentContainer>> {
        self.container_store()?.list_containers()
    }

    pub fn archive_container(&self, container_id: &str) -> io::Result<()> {
        self.container_store()?.archive_container(container_id)
    }

    pub fn update_container(
        &self,
        container_id: &str,
        title: Option<String>,
        status: Option<String>,
        default_model_config: Option<ModelInvocationConfig>,
        context_policy: Option<ContextWindowControlConfig>,
    ) -> io::Result<AgentContainer> {
        self.container_store()?.update_container(
            container_id,
            title,
            status.as_deref().map(AgentContainerStatus::from_str),
            default_model_config,
            context_policy,
        )
    }

    pub fn append_container_timeline_item(
        &self,
        container_id: &str,
        item_kind: ContainerTimelineItemKind,
        ref_id: &str,
        status: &str,
        title: Option<String>,
        summary_ref: Option<String>,
    ) -> io::Result<ContainerTimelineItem> {
        self.container_store()?.append_timeline_item(
            container_id,
            item_kind,
            ref_id,
            status,
            title,
            summary_ref,
        )
    }

    pub fn list_container_timeline(
        &self,
        container_id: &str,
        limit: usize,
    ) -> io::Result<Vec<ContainerTimelineItem>> {
        self.container_store()?.list_timeline(container_id, limit)
    }

    pub fn list_container_tasks(
        &self,
        container_id: &str,
        limit: usize,
    ) -> io::Result<Vec<ContainerTimelineItem>> {
        Ok(self
            .container_store()?
            .list_timeline(container_id, limit)?
            .into_iter()
            .filter(|item| item.item_kind == ContainerTimelineItemKind::TaskRun)
            .collect())
    }

    pub fn upsert_context_pack(&self, pack: ContextPack) -> io::Result<ContextPack> {
        self.container_store()?.upsert_context_pack(pack)
    }

    pub fn get_context_pack(&self, context_pack_id: &str) -> io::Result<ContextPack> {
        self.container_store()?.get_context_pack(context_pack_id)
    }

    pub fn latest_context_pack(&self, container_id: &str) -> io::Result<Option<ContextPack>> {
        self.container_store()?
            .latest_context_pack_for_container(container_id)
    }

    pub fn estimate_context_pack(&self, pack: &ContextPack) -> io::Result<Value> {
        let store = self.container_store()?;
        let container = store.get_container(&pack.container_id)?;
        let timeline = store.list_timeline(&pack.container_id, 1_000)?;
        let memories = store.list_memory_bindings(&pack.container_id)?;
        let adapter = ContainerContextWindowAdapter::new(
            store,
            container,
            timeline,
            memories,
            Some(pack.clone()),
            "estimate_context_pack",
        );
        let parts = adapter.build_visible_request_parts()?;
        let estimate =
            ContextWindowController::estimate(&ContextWindowControlConfig::default(), &parts);
        Ok(json!({
            "container_id": pack.container_id,
            "context_pack_id": pack.context_pack_id,
            "estimate": estimate,
        }))
    }

    pub fn materialize_context_pack(&self, pack: &ContextPack) -> io::Result<ContextPack> {
        self.container_store()?
            .materialize_context_pack_auto_items(pack.clone())
    }

    pub fn compact_container_context(
        &self,
        container_id: &str,
        target_runtime: Option<String>,
    ) -> io::Result<Value> {
        let store = self.container_store()?;
        let container = store.get_container(container_id)?;
        let timeline = store.list_timeline(container_id, 1_000)?;
        let memories = store.list_memory_bindings(container_id)?;
        let latest_pack = store.latest_context_pack_for_container(container_id)?;
        let mut adapter = ContainerContextWindowAdapter::new(
            store.clone(),
            container.clone(),
            timeline,
            memories,
            latest_pack.clone(),
            target_runtime
                .clone()
                .unwrap_or_else(|| "unknown".to_string()),
        );
        let parts = adapter.build_visible_request_parts()?;
        let preflight = ContextWindowController::preflight(
            ContextScope::Container,
            &container.context_policy,
            &parts,
        )?;
        let checkpoint = adapter.run_pre_compaction_checkpoint(&preflight.estimate)?;
        let compaction_input = adapter.build_compaction_input(&preflight.estimate)?;
        let input = compaction_input.payload.clone();
        let checkpoint_ref = checkpoint.checkpoint_ref.clone();
        let model_truth = self.process_truth(container_id)?;
        let pid = format!("container_context_compaction_{container_id}");
        let token = CapabilityToken {
            token_id: format!("token_{pid}"),
            job_id: container_id.to_string(),
            pid: pid.clone(),
            workspace_root: self.controller.workspace_root().display().to_string(),
            capabilities: vec![
                "model.invoke".to_string(),
                "model.compact_container_context".to_string(),
            ],
            permissions: vec!["model:invoke".to_string()],
        };
        let instruction_ref = model_truth.write_blob(
            &format!("container_context_compactions/{}_instruction.txt", crate::now_ms()),
            b"Compact the AgentContainer context into strict JSON schema supernova_container_context_summary.v1. Preserve timeline refs, memory refs, active goals, constraints, and open questions. Do not invent execution facts.",
        )?;
        let input_ref = model_truth.write_blob(
            &format!(
                "container_context_compactions/{}_input.json",
                crate::now_ms()
            ),
            &serde_json::to_vec_pretty(&input).map_err(crate::json_err)?,
        )?;
        let provider = self.model_provider.clone();
        let operation = ModelOperation::CompactContainerContext;
        let budget = crate::ModelContextProfile::for_provider(provider.as_ref(), &operation)
            .budget_for(&operation);
        let action = ModelAction {
            action_id: format!("container_context_compaction_{}", crate::now_ms()),
            job_id: container_id.to_string(),
            pid: pid.clone(),
            reasoning_step_id: format!("container_context_compaction_{}", crate::now_ms()),
            operation: operation.clone(),
            instruction_ref,
            input_refs: vec![input_ref],
            preference_snapshot_ref: None,
            output_schema: crate::container_context_summary_output_schema(),
            provider: provider.provider_name().to_string(),
            model: provider.model_name_for_operation(&operation),
            budget,
            failure_policy: ModelFailurePolicy::FailClosed,
            required: true,
        };
        let model_call_id = format!("mcall_container_compact_{}", crate::now_ms());
        let receipt = ModelRuntime::new(model_truth.clone(), token, provider)
            .with_model_invocation_config(container.default_model_config.clone(), None)
            .with_model_call_id_override(Some(model_call_id))
            .compact_container_context(action)?;
        if receipt.status != "success" {
            store.append_timeline_item(
                container_id,
                ContainerTimelineItemKind::ContextCompaction,
                receipt.model_call_id.clone(),
                "failed",
                Some("Container context compaction".to_string()),
                Some(checkpoint_ref.clone()),
            )?;
            return Ok(json!({
                "container_id": container_id,
                "status": "failed",
                "checkpoint_ref": checkpoint_ref,
                "model_call_id": receipt.model_call_id,
                "error": receipt.error,
            }));
        }
        let output_ref = receipt.output_ref.clone().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "successful container compaction did not produce output_ref",
            )
        })?;
        let output_text = model_truth
            .resolve_blob_ref(&output_ref)
            .and_then(std::fs::read_to_string)?;
        let summary = serde_json::from_str::<Value>(&output_text).map_err(crate::json_err)?;
        crate::validate_container_context_summary(&summary, container_id)?;
        let summary_ref = store.write_container_blob(
            container_id,
            &format!("summaries/container_summary_{}.json", crate::now_ms()),
            &serde_json::to_vec_pretty(&summary).map_err(crate::json_err)?,
        )?;
        let _replacement = adapter.replace_visible_context(ContextCompactionReceipt {
            compaction_id: receipt.model_call_id.clone(),
            scope: ContextScope::Container,
            status: "completed".to_string(),
            summary_ref: Some(summary_ref.clone()),
            live_suffix_ref: None,
            model_call_receipt_ref: receipt.ledger_ref.clone(),
            compacted_until_message_index: None,
            errors: Vec::new(),
        })?;
        Ok(json!({
            "container_id": container_id,
            "status": "success",
            "preflight": preflight,
            "checkpoint_ref": checkpoint_ref,
            "summary_ref": summary_ref,
            "model_summary_ref": output_ref,
            "model_call_id": receipt.model_call_id,
        }))
    }

    pub fn bind_memory(
        &self,
        container_id: &str,
        memory_ref: &str,
        include_mode: &str,
        priority: u8,
    ) -> io::Result<crate::MemoryBinding> {
        self.container_store()?
            .bind_memory(container_id, memory_ref, include_mode, priority)
    }

    pub fn list_memory_bindings(
        &self,
        container_id: &str,
    ) -> io::Result<Vec<crate::MemoryBinding>> {
        self.container_store()?.list_memory_bindings(container_id)
    }

    pub fn unbind_memory(&self, binding_id: &str) -> io::Result<()> {
        self.container_store()?.unbind_memory(binding_id)
    }

    pub fn task_context_window(&self, job_id: &str) -> io::Result<Value> {
        let truth = self.process_truth(job_id)?;
        let events = truth.read_events()?;
        let context_events = events
            .iter()
            .filter(|event| event.event_type.starts_with("context_window_"))
            .cloned()
            .collect::<Vec<_>>();
        Ok(json!({
            "job_id": job_id,
            "events": context_events,
            "latest": context_events.last(),
            "task_process_truth_invariant": crate::TASK_PROCESS_TRUTH_NOT_COMPRESSED_INVARIANT,
        }))
    }

    pub fn chat_context_window(&self, chat_thread_id: &str) -> io::Result<Value> {
        let chat = self.chat_truth()?;
        let events = chat.read_events(chat_thread_id)?;
        let context_events = events
            .iter()
            .filter(|event| {
                event.event_type.contains("context_window")
                    || event.event_type.contains("context_compaction")
            })
            .cloned()
            .collect::<Vec<_>>();
        Ok(json!({
            "chat_thread_id": chat_thread_id,
            "events": context_events,
            "latest": context_events.last(),
        }))
    }

    pub fn compact_chat_thread(&self, chat_thread_id: &str) -> io::Result<Value> {
        let chat = self.chat_truth()?;
        let thread = chat.get_thread(chat_thread_id)?;
        let events = chat.read_events(chat_thread_id)?;
        chat.append_event(
            chat_thread_id,
            &thread.container_id,
            "chat_context_compaction_started",
            json!({
                "schema_version": crate::CHAT_TRUTH_SCHEMA_VERSION,
                "chat_thread_id": chat_thread_id,
                "event_count": events.len(),
            }),
            None,
        )?;
        let model_truth_id = crate::chat_model_syscall_truth_id(chat_thread_id);
        let model_truth = self.process_truth(&model_truth_id)?;
        let pid = format!("chat_compaction_{chat_thread_id}");
        let token = CapabilityToken {
            token_id: format!("token_{pid}"),
            job_id: model_truth_id.clone(),
            pid: pid.clone(),
            workspace_root: self.controller.workspace_root().display().to_string(),
            capabilities: vec![
                "model.invoke".to_string(),
                "model.compact_chat_context".to_string(),
            ],
            permissions: vec!["model:invoke".to_string()],
        };
        let input = json!({
            "schema": "supernova_chat_context_compaction_input.v1",
            "chat_thread_id": chat_thread_id,
            "container_id": thread.container_id.clone(),
            "compacted_until_event_seq": events.last().map(|event| event.event_seq).unwrap_or(0),
            "events": events.iter().rev().take(80).rev().map(|event| {
                json!({
                    "event_seq": event.event_seq,
                    "event_type": event.event_type,
                    "payload": event.payload,
                })
            }).collect::<Vec<_>>(),
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
            "fact_boundary": "This summary is provider-visible context only and is not execution fact.",
        });
        let instruction_ref = model_truth.write_blob(
            &format!("chat_context_compactions/manual_{}_instruction.txt", crate::now_ms()),
            b"Compact the chat thread into strict JSON schema supernova_chat_context_summary.v1. Preserve user intent, refs, tool-result facts, and open questions. Do not invent execution facts.",
        )?;
        let input_ref = model_truth.write_blob(
            &format!(
                "chat_context_compactions/manual_{}_input.json",
                crate::now_ms()
            ),
            &serde_json::to_vec_pretty(&input).map_err(crate::json_err)?,
        )?;
        let provider = self.model_provider.clone();
        let operation = ModelOperation::CompactChatContext;
        let budget = crate::ModelContextProfile::for_provider(provider.as_ref(), &operation)
            .budget_for(&operation);
        let action = ModelAction {
            action_id: format!("chat_manual_compaction_{}", crate::now_ms()),
            job_id: model_truth_id,
            pid: pid.clone(),
            reasoning_step_id: format!("chat_manual_compaction_{}", crate::now_ms()),
            operation: operation.clone(),
            instruction_ref,
            input_refs: vec![input_ref],
            preference_snapshot_ref: None,
            output_schema: crate::chat_context_summary_output_schema(),
            provider: provider.provider_name().to_string(),
            model: provider.model_name_for_operation(&operation),
            budget,
            failure_policy: ModelFailurePolicy::FailClosed,
            required: true,
        };
        let model_call_id = format!("mcall_chat_manual_compact_{}", crate::now_ms());
        chat.append_event(
            chat_thread_id,
            &thread.container_id,
            "context_window_compaction_model_call_started",
            json!({
                "schema_version": crate::CONTEXT_WINDOW_EVENT_SCHEMA_VERSION,
                "chat_thread_id": chat_thread_id,
                "model_call_id": model_call_id.clone(),
                "operation": "model.compact_chat_context",
            }),
            None,
        )?;
        let receipt = ModelRuntime::new(model_truth.clone(), token, provider)
            .with_model_invocation_config(ModelInvocationConfig::from_env(), None)
            .with_model_call_id_override(Some(model_call_id.clone()))
            .compact_chat_context(action)?;
        chat.append_event(
            chat_thread_id,
            &thread.container_id,
            "context_window_compaction_model_call_completed",
            json!({
                "schema_version": crate::CONTEXT_WINDOW_EVENT_SCHEMA_VERSION,
                "chat_thread_id": chat_thread_id,
                "model_call_id": receipt.model_call_id.clone(),
                "status": receipt.status.clone(),
                "output_ref": receipt.output_ref.clone(),
                "schema_validation": receipt.schema_validation.clone(),
                "error": receipt.error.clone(),
            }),
            None,
        )?;
        if receipt.status != "success" {
            chat.append_event(
                chat_thread_id,
                &thread.container_id,
                "context_window_compaction_failed",
                json!({
                    "schema_version": crate::CONTEXT_WINDOW_EVENT_SCHEMA_VERSION,
                    "chat_thread_id": chat_thread_id,
                    "model_call_id": receipt.model_call_id.clone(),
                    "status": receipt.status.clone(),
                    "error": receipt.error.clone(),
                }),
                None,
            )?;
            return Ok(json!({
                "chat_thread_id": chat_thread_id,
                "status": "failed",
                "error": receipt.error,
            }));
        }
        let output_ref = receipt.output_ref.clone().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "successful chat compaction did not produce output_ref",
            )
        })?;
        let output_text = model_truth
            .resolve_blob_ref(&output_ref)
            .and_then(std::fs::read_to_string)?;
        let summary = serde_json::from_str::<Value>(&output_text).map_err(crate::json_err)?;
        crate::validate_chat_context_summary(&summary)?;
        let summary_ref = chat.write_chat_blob(
            chat_thread_id,
            &format!("compactions/chat_compaction_{}.json", crate::now_ms()),
            &serde_json::to_vec_pretty(&summary).map_err(crate::json_err)?,
        )?;
        chat.append_event(
            chat_thread_id,
            &thread.container_id,
            "chat_context_compaction_completed",
            json!({
                "schema_version": crate::CHAT_TRUTH_SCHEMA_VERSION,
                "chat_thread_id": chat_thread_id,
                "summary_ref": summary_ref.clone(),
                "model_summary_ref": output_ref,
                "model_call_id": receipt.model_call_id.clone(),
                "compacted_until_event_seq": events.last().map(|event| event.event_seq).unwrap_or(0),
            }),
            Some(summary_ref.clone()),
        )?;
        self.container_store()?.append_timeline_item(
            &thread.container_id,
            ContainerTimelineItemKind::ContextCompaction,
            chat_thread_id.to_string(),
            "completed",
            Some("Chat context compaction".to_string()),
            Some(summary_ref.clone()),
        )?;
        Ok(json!({
            "chat_thread_id": chat_thread_id,
            "status": "success",
            "summary_ref": summary_ref,
        }))
    }

    pub fn compact_task_context(&self, job_id: &str) -> io::Result<Value> {
        let truth = self.process_truth(job_id)?;
        let events = truth.read_events()?;
        let checkpoint_ref = truth.write_blob(
            &format!("context_window/checkpoints/{}_events.json", crate::now_ms()),
            &serde_json::to_vec_pretty(&events).map_err(crate::json_err)?,
        )?;
        truth.append_event(
            None,
            "context_window_checkpoint_created",
            json!({
                "schema_version": crate::CONTEXT_WINDOW_EVENT_SCHEMA_VERSION,
                "scope": ContextScope::Task {
                    container_id: None,
                    job_id: job_id.to_string(),
                    process_id: "unknown".to_string(),
                },
                "checkpoint_ref": checkpoint_ref,
                "task_process_truth_invariant": crate::TASK_PROCESS_TRUTH_NOT_COMPRESSED_INVARIANT,
            }),
        )?;
        let summary = json!({
            "schema": "supernova_task_context_compaction_input.v1",
            "job_id": job_id,
            "events": events.iter().rev().take(120).rev().map(|event| {
                json!({
                    "event_id": event.event_id,
                    "event_type": event.event_type,
                    "pid": event.pid,
                    "data": event.data,
                })
            }).collect::<Vec<_>>(),
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
        let instruction_ref = truth.write_blob(
            &format!("context_window/compactions/task_manual_instruction_{}.txt", crate::now_ms()),
            b"Compact the task context into strict JSON schema supernova_task_context_summary.v1. Preserve Kernel receipts, typed refs, approvals, verification facts, constraints, and open questions. Do not invent execution facts.",
        )?;
        let input_ref = truth.write_blob(
            &format!(
                "context_window/compactions/task_manual_input_{}.json",
                crate::now_ms()
            ),
            &serde_json::to_vec_pretty(&summary).map_err(crate::json_err)?,
        )?;
        let provider = self.model_provider.clone();
        let operation = ModelOperation::CompactTaskContext;
        let budget = crate::ModelContextProfile::for_provider(provider.as_ref(), &operation)
            .budget_for(&operation);
        let pid = "task_context_compaction".to_string();
        let token = CapabilityToken {
            token_id: format!("token_{pid}_{job_id}"),
            job_id: job_id.to_string(),
            pid: pid.clone(),
            workspace_root: self.controller.workspace_root().display().to_string(),
            capabilities: vec![
                "model.invoke".to_string(),
                "model.compact_task_context".to_string(),
            ],
            permissions: vec!["model:invoke".to_string()],
        };
        let action = ModelAction {
            action_id: format!("task_manual_compaction_{}", crate::now_ms()),
            job_id: job_id.to_string(),
            pid: pid.clone(),
            reasoning_step_id: format!("task_manual_compaction_{}", crate::now_ms()),
            operation: operation.clone(),
            instruction_ref,
            input_refs: vec![input_ref],
            preference_snapshot_ref: None,
            output_schema: crate::task_context_summary_output_schema(),
            provider: provider.provider_name().to_string(),
            model: provider.model_name_for_operation(&operation),
            budget,
            failure_policy: ModelFailurePolicy::FailClosed,
            required: true,
        };
        let model_call_id = format!("mcall_task_manual_compact_{}", crate::now_ms());
        truth.append_event(
            Some(&pid),
            "context_window_compaction_model_call_started",
            json!({
                "schema_version": crate::CONTEXT_WINDOW_EVENT_SCHEMA_VERSION,
                "scope": ContextScope::Task {
                    container_id: None,
                    job_id: job_id.to_string(),
                    process_id: pid.clone(),
                },
                "model_call_id": model_call_id.clone(),
                "operation": "model.compact_task_context",
                "task_process_truth_invariant": crate::TASK_PROCESS_TRUTH_NOT_COMPRESSED_INVARIANT,
            }),
        )?;
        let receipt = ModelRuntime::new(truth.clone(), token, provider)
            .with_model_invocation_config(ModelInvocationConfig::from_env(), None)
            .with_model_call_id_override(Some(model_call_id.clone()))
            .compact_task_context(action)?;
        truth.append_event(
            Some(&pid),
            "context_window_compaction_model_call_completed",
            json!({
                "schema_version": crate::CONTEXT_WINDOW_EVENT_SCHEMA_VERSION,
                "scope": ContextScope::Task {
                    container_id: None,
                    job_id: job_id.to_string(),
                    process_id: pid.clone(),
                },
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
            truth.append_event(
                Some(&pid),
                "context_window_compaction_failed",
                json!({
                    "schema_version": crate::CONTEXT_WINDOW_EVENT_SCHEMA_VERSION,
                    "model_call_id": receipt.model_call_id.clone(),
                    "status": receipt.status.clone(),
                    "error": receipt.error.clone(),
                    "task_process_truth_invariant": crate::TASK_PROCESS_TRUTH_NOT_COMPRESSED_INVARIANT,
                }),
            )?;
            return Ok(json!({
                "job_id": job_id,
                "status": "failed",
                "checkpoint_ref": checkpoint_ref,
                "error": receipt.error,
            }));
        }
        let summary_ref = receipt.output_ref.clone().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "successful task compaction did not produce output_ref",
            )
        })?;
        let output_text = truth
            .resolve_blob_ref(&summary_ref)
            .and_then(std::fs::read_to_string)?;
        let summary_json = serde_json::from_str::<Value>(&output_text).map_err(crate::json_err)?;
        crate::validate_task_context_summary(&summary_json)?;
        truth.append_event(
            None,
            "context_window_visible_context_replaced",
            json!({
                "schema_version": crate::CONTEXT_WINDOW_EVENT_SCHEMA_VERSION,
                "summary_ref": summary_ref.clone(),
                "model_call_id": receipt.model_call_id.clone(),
                "replacement_kind": "task_provider_visible_context_summary",
                "task_process_truth_invariant": crate::TASK_PROCESS_TRUTH_NOT_COMPRESSED_INVARIANT,
            }),
        )?;
        Ok(json!({
            "job_id": job_id,
            "status": "success",
            "checkpoint_ref": checkpoint_ref,
            "summary_ref": summary_ref,
        }))
    }

    pub fn create_chat_thread(
        &self,
        container_id: &str,
        title: Option<String>,
    ) -> io::Result<ChatThread> {
        let thread = self
            .chat_truth()?
            .create_thread(container_id, title.clone())?;
        self.container_store()?.append_timeline_item(
            container_id,
            ContainerTimelineItemKind::ChatThread,
            thread.chat_thread_id.clone(),
            "active",
            title,
            None,
        )?;
        Ok(thread)
    }

    pub fn list_chat_threads(&self, container_id: &str) -> io::Result<Vec<ChatThread>> {
        self.chat_truth()?.list_threads_for_container(container_id)
    }

    pub fn read_chat_events(&self, chat_thread_id: &str) -> io::Result<Vec<ChatEvent>> {
        self.chat_truth()?.read_events(chat_thread_id)
    }

    pub fn force_close_chat_turn(
        &self,
        chat_thread_id: &str,
        reason: &str,
    ) -> io::Result<ChatEvent> {
        let chat_truth = self.chat_truth()?;
        let thread = chat_truth.get_thread(chat_thread_id)?;
        chat_truth.append_event(
            chat_thread_id,
            &thread.container_id,
            "chat_turn_user_forced_closed",
            json!({
                "chat_thread_id": chat_thread_id,
                "reason": reason,
                "user_forced": true,
                "user_facing_reason": "用户强制关闭",
                "status": "cancelled",
            }),
            None,
        )
    }

    pub fn start_chat_turn(&self, request: ChatTurnRequest) -> io::Result<ChatTurnResult> {
        self.start_chat_turn_with_stream_sink(request, None)
    }

    pub fn start_chat_turn_with_stream_sink(
        &self,
        request: ChatTurnRequest,
        stream_sink: Option<Arc<dyn ModelStreamSink>>,
    ) -> io::Result<ChatTurnResult> {
        let container_id = request.container_id.clone();
        let new_thread = request.chat_thread_id.is_none();
        let result = self
            .chat_runtime()?
            .start_turn_with_stream_sink(request, stream_sink)?;
        if new_thread {
            self.container_store()?.append_timeline_item(
                &container_id,
                ContainerTimelineItemKind::ChatThread,
                result.chat_thread_id.clone(),
                "active",
                None,
                None,
            )?;
        }
        self.container_store()?.append_timeline_item(
            &container_id,
            ContainerTimelineItemKind::ChatTurn,
            result.turn_id.clone(),
            format!("{:?}", result.status).to_ascii_lowercase(),
            None,
            None,
        )?;
        Ok(result)
    }
}

fn root_pid_for_truth(truth: &ProcessTruthStore) -> Option<String> {
    truth
        .registry_snapshot()
        .ok()?
        .processes
        .into_iter()
        .find(|process| process.process_type == "root_agent_process")
        .map(|process| process.pid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AgentJob, AgentProcess, TaskAgentRuntimeRecord};
    use std::fs;
    use std::path::PathBuf;

    #[test]
    fn sync_task_timeline_status_replaces_stale_waiting_approval() {
        let workspace = temp_workspace("kernel_api_timeline_sync");
        let state_root = workspace.join("state");
        let api = KernelApi::new_with_state_root(&workspace, &state_root).unwrap();
        let container = api
            .create_container(Some("Test container".to_string()), None, None)
            .unwrap();
        let job_id = "job_timeline_sync";
        let root_pid = "pid_root_timeline_sync";
        let truth =
            ProcessTruthStore::new_with_state_root(&workspace, &state_root, job_id).unwrap();
        truth
            .register_job(&AgentJob {
                job_id: job_id.to_string(),
                user_goal: "Delete a file".to_string(),
                workspace_root: workspace.to_string_lossy().to_string(),
                status: "running".to_string(),
                root_pid: root_pid.to_string(),
            })
            .unwrap();
        truth
            .register_process(&AgentProcess {
                pid: root_pid.to_string(),
                ppid: None,
                job_id: job_id.to_string(),
                process_type: "root_agent_process".to_string(),
                state: "running".to_string(),
                input_refs: vec![],
                output_refs: vec![],
                capability_tokens: vec![],
                budget_ms: None,
                exit_code: None,
            })
            .unwrap();
        truth
            .register_task_agent_runtime(&TaskAgentRuntimeRecord {
                runtime_id: "runtime_timeline_sync".to_string(),
                job_id: job_id.to_string(),
                root_pid: root_pid.to_string(),
                state: "running".to_string(),
                checkpoint_refs: vec![],
            })
            .unwrap();
        truth
            .append_event(
                Some(root_pid),
                "agent_container_task_bound",
                json!({
                    "container_id": container.container_id.clone(),
                    "job_id": job_id,
                    "status": "waiting_approval",
                }),
            )
            .unwrap();
        api.container_store()
            .unwrap()
            .upsert_timeline_item(
                &container.container_id,
                ContainerTimelineItemKind::TaskRun,
                job_id.to_string(),
                "waiting_approval",
                Some("Delete a file".to_string()),
                None,
            )
            .unwrap();

        truth.update_job_status("waiting_user").unwrap();
        api.sync_task_timeline_status(job_id, "waiting_user")
            .unwrap();

        let tasks = api
            .list_container_tasks(&container.container_id, 10)
            .unwrap();
        let task = tasks.iter().find(|item| item.ref_id == job_id).unwrap();
        assert_eq!(task.status, "waiting_user");
    }

    fn temp_workspace(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("supernova_{label}_{}", crate::now_ms()));
        fs::create_dir_all(&dir).unwrap();
        fs::canonicalize(dir).unwrap()
    }
}
