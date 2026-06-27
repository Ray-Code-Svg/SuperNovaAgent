use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use local_runtime_protocol::{
    ArtifactDestinationGuidance, ContextPack, DisplayLanguage, ModelConfig, SourceGuidance,
};
use supernova_process_kernel::{
    AgentContainer, ArtifactDestinationGuidance as KernelArtifactDestinationGuidance, ChatEvent,
    ChatThread, ChatTurnRequest, ChatTurnResult, ContainerTimelineItem,
    ContextPack as KernelContextPack, ContextPackAutoPolicy as KernelContextPackAutoPolicy,
    ContextPackIncludeMode, ContextPackItem as KernelContextPackItem, ContextPackItemKind,
    ContextWindowControlConfig, KernelApi, ModelInvocationConfig, ModelRouteMode,
    ModelRoutePreference, ModelStreamSink, ProcessEvent, ProcessTruthStore, ProviderProfileRecord,
    ProviderTestReceipt, ReasoningEffort,
    ReferenceSourceDirective as KernelReferenceSourceDirective, ResponseLanguage,
    SourceGuidance as KernelSourceGuidance, TaskAgentRunResult, ThinkingMode,
};

#[derive(Clone, Debug)]
pub struct KernelBridge {
    workspace_root: PathBuf,
    kernel_state_root: PathBuf,
    provider_profile_root: PathBuf,
}

impl KernelBridge {
    pub fn new(
        workspace_root: PathBuf,
        kernel_state_root: PathBuf,
        provider_profile_root: PathBuf,
    ) -> Self {
        Self {
            workspace_root,
            kernel_state_root,
            provider_profile_root,
        }
    }

    pub fn create_chat_thread(
        &self,
        container_id: &str,
        title: Option<String>,
    ) -> io::Result<ChatThread> {
        self.api()?.create_chat_thread(container_id, title)
    }

    pub fn create_container(
        &self,
        title: Option<String>,
        model_config: Option<serde_json::Value>,
        context_policy: Option<serde_json::Value>,
    ) -> io::Result<AgentContainer> {
        self.api()?.create_container(
            title,
            model_config_value(model_config)?,
            context_policy_value(context_policy)?,
        )
    }

    pub fn list_containers(&self) -> io::Result<Vec<AgentContainer>> {
        self.api()?.list_containers()
    }

    pub fn get_container(&self, container_id: &str) -> io::Result<AgentContainer> {
        self.api()?.get_container(container_id)
    }

    pub fn update_container(
        &self,
        container_id: &str,
        title: Option<String>,
        status: Option<String>,
        model_config: Option<serde_json::Value>,
        context_policy: Option<serde_json::Value>,
    ) -> io::Result<AgentContainer> {
        self.api()?.update_container(
            container_id,
            title,
            status,
            model_config_value(model_config)?,
            context_policy_value(context_policy)?,
        )
    }

    pub fn archive_container(&self, container_id: &str) -> io::Result<AgentContainer> {
        self.api()?.archive_container(container_id)?;
        self.api()?.get_container(container_id)
    }

    pub fn list_chat_threads(&self, container_id: &str) -> io::Result<Vec<ChatThread>> {
        self.api()?.list_chat_threads(container_id)
    }

    pub fn read_chat_events(&self, chat_thread_id: &str) -> io::Result<Vec<ChatEvent>> {
        self.api()?.read_chat_events(chat_thread_id)
    }

    pub fn force_close_chat_turn(
        &self,
        chat_thread_id: &str,
        reason: &str,
    ) -> io::Result<ChatEvent> {
        self.api()?.force_close_chat_turn(chat_thread_id, reason)
    }

    pub fn read_chat_blob_text(&self, blob_ref: &str) -> io::Result<String> {
        let raw = blob_ref.strip_prefix("chat_blob://").ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "chat blob ref must use chat_blob://",
            )
        })?;
        let (chat_thread_id, relative) = raw.split_once('/').ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "chat blob ref must include thread and path",
            )
        })?;
        if chat_thread_id.is_empty() || relative.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "chat blob ref must include non-empty thread and path",
            ));
        }
        let mut clean = PathBuf::new();
        for component in Path::new(relative).components() {
            match component {
                Component::Normal(part) => clean.push(part),
                Component::CurDir => {}
                Component::ParentDir | Component::Prefix(_) | Component::RootDir => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "chat blob ref leaves chat blob root",
                    ));
                }
            }
        }
        let blob_root = self
            .kernel_state_root
            .join("blobs")
            .join("chat")
            .join(chat_thread_id);
        let path = blob_root.join(clean);
        std::fs::read_to_string(path)
    }

    pub fn start_chat_turn(
        &self,
        container_id: &str,
        chat_thread_id: Option<String>,
        message: String,
        context_pack: Option<ContextPack>,
        source_guidance: Option<SourceGuidance>,
        model_config: Option<ModelConfig>,
    ) -> io::Result<ChatTurnResult> {
        self.start_chat_turn_with_response_language(
            container_id,
            chat_thread_id,
            message,
            context_pack,
            source_guidance,
            model_config,
            DisplayLanguage::EnUs,
        )
    }

    pub fn start_chat_turn_with_response_language(
        &self,
        container_id: &str,
        chat_thread_id: Option<String>,
        message: String,
        context_pack: Option<ContextPack>,
        source_guidance: Option<SourceGuidance>,
        model_config: Option<ModelConfig>,
        response_language: DisplayLanguage,
    ) -> io::Result<ChatTurnResult> {
        self.api()?.start_chat_turn(ChatTurnRequest {
            container_id: container_id.to_string(),
            chat_thread_id,
            message,
            context_pack: context_pack.map(to_kernel_context_pack),
            source_guidance: source_guidance.map(to_kernel_source_guidance),
            model_config_override: Some(to_kernel_model_config_with_language(
                model_config,
                response_language,
            )),
        })
    }

    pub fn start_chat_turn_with_stream_sink(
        &self,
        container_id: &str,
        chat_thread_id: Option<String>,
        message: String,
        context_pack: Option<ContextPack>,
        source_guidance: Option<SourceGuidance>,
        model_config: Option<ModelConfig>,
        stream_sink: Arc<dyn ModelStreamSink>,
    ) -> io::Result<ChatTurnResult> {
        self.start_chat_turn_with_stream_sink_and_response_language(
            container_id,
            chat_thread_id,
            message,
            context_pack,
            source_guidance,
            model_config,
            stream_sink,
            DisplayLanguage::EnUs,
        )
    }

    pub fn start_chat_turn_with_stream_sink_and_response_language(
        &self,
        container_id: &str,
        chat_thread_id: Option<String>,
        message: String,
        context_pack: Option<ContextPack>,
        source_guidance: Option<SourceGuidance>,
        model_config: Option<ModelConfig>,
        stream_sink: Arc<dyn ModelStreamSink>,
        response_language: DisplayLanguage,
    ) -> io::Result<ChatTurnResult> {
        self.api()?.start_chat_turn_with_stream_sink(
            ChatTurnRequest {
                container_id: container_id.to_string(),
                chat_thread_id,
                message,
                context_pack: context_pack.map(to_kernel_context_pack),
                source_guidance: source_guidance.map(to_kernel_source_guidance),
                model_config_override: Some(to_kernel_model_config_with_language(
                    model_config,
                    response_language,
                )),
            },
            Some(stream_sink),
        )
    }

    pub fn list_container_tasks(
        &self,
        container_id: &str,
        limit: usize,
    ) -> io::Result<Vec<ContainerTimelineItem>> {
        self.api()?.list_container_tasks(container_id, limit)
    }

    pub fn start_task_in_container(
        &self,
        container_id: &str,
        goal: &str,
        context_pack_id: Option<String>,
        model_config: Option<ModelConfig>,
        auto_approve: bool,
    ) -> io::Result<TaskAgentRunResult> {
        self.api()?.start_task_in_container_with_options(
            container_id,
            goal,
            None,
            model_config
                .map(to_kernel_model_config)
                .unwrap_or_else(ModelInvocationConfig::from_env),
            context_pack_id,
            auto_approve,
        )
    }

    pub fn start_task_in_container_with_started<F>(
        &self,
        container_id: &str,
        goal: &str,
        context_pack_id: Option<String>,
        source_guidance: Option<SourceGuidance>,
        artifact_destination: Option<ArtifactDestinationGuidance>,
        model_config: Option<ModelConfig>,
        auto_approve: bool,
        on_started: F,
    ) -> io::Result<TaskAgentRunResult>
    where
        F: FnOnce(&str, &str) -> io::Result<()>,
    {
        self.start_task_in_container_with_started_and_stream_sink(
            container_id,
            goal,
            context_pack_id,
            source_guidance,
            artifact_destination,
            model_config,
            auto_approve,
            on_started,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn start_task_in_container_with_started_and_stream_sink<F>(
        &self,
        container_id: &str,
        goal: &str,
        context_pack_id: Option<String>,
        source_guidance: Option<SourceGuidance>,
        artifact_destination: Option<ArtifactDestinationGuidance>,
        model_config: Option<ModelConfig>,
        auto_approve: bool,
        on_started: F,
        model_stream_sink: Option<Arc<dyn ModelStreamSink>>,
    ) -> io::Result<TaskAgentRunResult>
    where
        F: FnOnce(&str, &str) -> io::Result<()>,
    {
        self.start_task_in_container_with_started_and_stream_sink_and_response_language(
            container_id,
            goal,
            context_pack_id,
            source_guidance,
            artifact_destination,
            model_config,
            auto_approve,
            on_started,
            model_stream_sink,
            DisplayLanguage::EnUs,
        )
    }

    pub fn start_task_in_container_with_started_and_stream_sink_and_response_language<F>(
        &self,
        container_id: &str,
        goal: &str,
        context_pack_id: Option<String>,
        source_guidance: Option<SourceGuidance>,
        artifact_destination: Option<ArtifactDestinationGuidance>,
        model_config: Option<ModelConfig>,
        auto_approve: bool,
        on_started: F,
        model_stream_sink: Option<Arc<dyn ModelStreamSink>>,
        response_language: DisplayLanguage,
    ) -> io::Result<TaskAgentRunResult>
    where
        F: FnOnce(&str, &str) -> io::Result<()>,
    {
        self.api()?
            .start_task_in_container_with_guidance_started_and_stream_sink(
                container_id,
                goal,
                None,
                to_kernel_model_config_with_language(model_config, response_language),
                context_pack_id,
                source_guidance.map(to_kernel_source_guidance),
                artifact_destination.map(to_kernel_artifact_destination_guidance),
                auto_approve,
                on_started,
                model_stream_sink,
            )
    }

    pub fn submit_user_input(
        &self,
        job_id: &str,
        user_input: &str,
    ) -> io::Result<TaskAgentRunResult> {
        self.api()?.submit_user_input(job_id, user_input)
    }

    pub fn cancel_job(&self, job_id: &str, reason: &str) -> io::Result<()> {
        self.api()?.cancel_job(job_id, reason)
    }

    pub fn read_process_events(&self, job_id: &str) -> io::Result<Vec<ProcessEvent>> {
        ProcessTruthStore::new_with_state_root(
            &self.workspace_root,
            &self.kernel_state_root,
            job_id,
        )?
        .read_events()
    }

    pub fn upsert_context_pack(&self, pack: ContextPack) -> io::Result<ContextPack> {
        let saved = self
            .api()?
            .upsert_context_pack(to_kernel_context_pack(pack))?;
        Ok(from_kernel_context_pack(saved))
    }

    pub fn latest_context_pack(&self, container_id: &str) -> io::Result<Option<ContextPack>> {
        self.api()?
            .latest_context_pack(container_id)
            .map(|value| value.map(from_kernel_context_pack))
    }

    pub fn estimate_context_pack(&self, pack: &ContextPack) -> io::Result<serde_json::Value> {
        self.api()?
            .estimate_context_pack(&to_kernel_context_pack(pack.clone()))
    }

    pub fn materialize_context_pack(&self, pack: ContextPack) -> io::Result<ContextPack> {
        self.api()?
            .materialize_context_pack(&to_kernel_context_pack(pack))
            .map(from_kernel_context_pack)
    }

    pub fn list_provider_profiles(&self) -> io::Result<Vec<ProviderProfileRecord>> {
        self.api()?.list_provider_profiles()
    }

    pub fn save_provider_profile(
        &self,
        provider_id: &str,
        api_base_url: Option<String>,
        api_key: Option<String>,
    ) -> io::Result<ProviderProfileRecord> {
        self.api()?
            .save_provider_profile(provider_id, api_base_url, api_key)
    }

    pub fn delete_provider_profile(&self, provider_id: &str) -> io::Result<()> {
        self.api()?.delete_provider_profile(provider_id)
    }

    pub fn test_provider_profile(
        &self,
        provider_id: &str,
        live_check: bool,
    ) -> io::Result<ProviderTestReceipt> {
        self.api()?.test_provider_profile(provider_id, live_check)
    }

    fn api(&self) -> io::Result<KernelApi> {
        KernelApi::new_with_state_root_and_provider_profile_root(
            &self.workspace_root,
            &self.kernel_state_root,
            &self.provider_profile_root,
        )
    }
}

fn to_kernel_model_config(model: ModelConfig) -> ModelInvocationConfig {
    let mut config = ModelInvocationConfig::from_env();
    config.provider = model.provider;
    config.model_route = match model.model.as_str() {
        "deepseek-v4-flash" => ModelRoutePreference {
            mode: ModelRouteMode::Flash,
            fixed_model: None,
        },
        "deepseek-v4-pro" => ModelRoutePreference {
            mode: ModelRouteMode::Pro,
            fixed_model: None,
        },
        other => ModelRoutePreference {
            mode: ModelRouteMode::Fixed,
            fixed_model: Some(other.to_string()),
        },
    };
    config.output_budget.max_tokens = model.token_budget.map(|value| value as u32);
    config.enforce_task_agent_provider_native_tools();
    config.thinking.mode = match model.thinking.as_str() {
        "disabled" | "off" | "false" => ThinkingMode::Disabled,
        "enabled" | "on" | "true" => ThinkingMode::Enabled,
        _ => ThinkingMode::Auto,
    };
    config.thinking.reasoning_effort = match model.reasoning_effort.as_str() {
        "max" | "xhigh" => ReasoningEffort::Max,
        _ => ReasoningEffort::High,
    };
    config
}

fn to_kernel_model_config_with_language(
    model: Option<ModelConfig>,
    response_language: DisplayLanguage,
) -> ModelInvocationConfig {
    let mut config = model
        .map(to_kernel_model_config)
        .unwrap_or_else(ModelInvocationConfig::from_env);
    config.response_language = to_kernel_response_language(response_language);
    config
}

fn to_kernel_response_language(response_language: DisplayLanguage) -> ResponseLanguage {
    match response_language {
        DisplayLanguage::ZhCn => ResponseLanguage::ZhCn,
        DisplayLanguage::EnUs => ResponseLanguage::EnUs,
    }
}

fn model_config_value(
    value: Option<serde_json::Value>,
) -> io::Result<Option<ModelInvocationConfig>> {
    let Some(value) = value else {
        return Ok(None);
    };
    if let Ok(config) = serde_json::from_value::<ModelInvocationConfig>(value.clone()) {
        return Ok(Some(config));
    }
    if let Ok(config) = serde_json::from_value::<ModelConfig>(value) {
        return Ok(Some(to_kernel_model_config(config)));
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidInput,
        "container model_config must be Product ModelConfig or Kernel ModelInvocationConfig",
    ))
}

fn context_policy_value(
    value: Option<serde_json::Value>,
) -> io::Result<Option<ContextWindowControlConfig>> {
    value
        .map(serde_json::from_value::<ContextWindowControlConfig>)
        .transpose()
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidInput, err))
}

fn to_kernel_context_pack(pack: ContextPack) -> KernelContextPack {
    KernelContextPack {
        context_pack_id: pack.context_pack_id,
        container_id: pack.container_id,
        selected_items: pack
            .selected_items
            .into_iter()
            .map(|item| KernelContextPackItem {
                item_kind: item_kind(&item.item_kind),
                ref_id: item.ref_id,
                label: item.label,
                include_mode: include_mode(&item.include_mode),
                priority: item.priority,
            })
            .collect(),
        excluded_items: pack
            .excluded_items
            .into_iter()
            .map(|item| KernelContextPackItem {
                item_kind: item_kind(&item.item_kind),
                ref_id: item.ref_id,
                label: item.label,
                include_mode: include_mode(&item.include_mode),
                priority: item.priority,
            })
            .collect(),
        auto_policy: KernelContextPackAutoPolicy {
            include_recent_chat_turns: pack.auto_policy.include_recent_chat_turns,
            include_recent_tasks: pack.auto_policy.include_recent_tasks,
            prefer_summaries: pack.auto_policy.prefer_summaries,
        },
        summary_ref: pack.summary_ref,
        estimated_tokens: pack.estimated_tokens,
    }
}

fn to_kernel_source_guidance(guidance: SourceGuidance) -> KernelSourceGuidance {
    KernelSourceGuidance {
        semantics: guidance.semantics,
        materialized_content: guidance.materialized_content,
        source_scope_enforcement: guidance.source_scope_enforcement,
        selected_sources: guidance
            .selected_sources
            .into_iter()
            .map(|source| KernelReferenceSourceDirective {
                source_kind: source.source_kind,
                ref_id: source.ref_id,
                label: source.label,
                usage: source.usage,
                include_mode: source.include_mode,
                selection_source: source.selection_source,
            })
            .collect(),
        user_intent: guidance.user_intent,
    }
}

fn to_kernel_artifact_destination_guidance(
    guidance: ArtifactDestinationGuidance,
) -> KernelArtifactDestinationGuidance {
    KernelArtifactDestinationGuidance {
        semantics: guidance.semantics,
        enforcement: guidance.enforcement,
        materialized_artifact: guidance.materialized_artifact,
        selected_output_dir: guidance.selected_output_dir,
        label: guidance.label,
    }
}

fn from_kernel_context_pack(pack: KernelContextPack) -> ContextPack {
    ContextPack {
        context_pack_id: pack.context_pack_id,
        container_id: pack.container_id,
        selected_items: pack
            .selected_items
            .into_iter()
            .map(|item| local_runtime_protocol::ContextPackItem {
                item_kind: item_kind_str(&item.item_kind).to_string(),
                ref_id: item.ref_id,
                label: item.label,
                include_mode: item.include_mode.as_str().to_string(),
                priority: item.priority,
            })
            .collect(),
        excluded_items: pack
            .excluded_items
            .into_iter()
            .map(|item| local_runtime_protocol::ContextPackItem {
                item_kind: item_kind_str(&item.item_kind).to_string(),
                ref_id: item.ref_id,
                label: item.label,
                include_mode: item.include_mode.as_str().to_string(),
                priority: item.priority,
            })
            .collect(),
        auto_policy: local_runtime_protocol::ContextPackAutoPolicy {
            include_recent_chat_turns: pack.auto_policy.include_recent_chat_turns,
            include_recent_tasks: pack.auto_policy.include_recent_tasks,
            prefer_summaries: pack.auto_policy.prefer_summaries,
        },
        summary_ref: pack.summary_ref,
        estimated_tokens: pack.estimated_tokens,
    }
}

fn item_kind(value: &str) -> ContextPackItemKind {
    match value {
        "chat_turn" => ContextPackItemKind::ChatTurn,
        "chat_thread" => ContextPackItemKind::ChatThread,
        "task_run" => ContextPackItemKind::TaskRun,
        "task_artifact" => ContextPackItemKind::TaskArtifact,
        "artifact" => ContextPackItemKind::Artifact,
        "memory_summary" => ContextPackItemKind::MemorySummary,
        "container_summary" => ContextPackItemKind::ContainerSummary,
        _ => ContextPackItemKind::SourceRef,
    }
}

fn item_kind_str(value: &ContextPackItemKind) -> &'static str {
    match value {
        ContextPackItemKind::ChatTurn => "chat_turn",
        ContextPackItemKind::ChatThread => "chat_thread",
        ContextPackItemKind::TaskRun => "task_run",
        ContextPackItemKind::TaskArtifact => "task_artifact",
        ContextPackItemKind::Artifact => "artifact",
        ContextPackItemKind::SourceRef => "source_ref",
        ContextPackItemKind::MemorySummary => "memory_summary",
        ContextPackItemKind::ContainerSummary => "container_summary",
    }
}

fn include_mode(value: &str) -> ContextPackIncludeMode {
    match value {
        "full" => ContextPackIncludeMode::Full,
        "metadata_only" => ContextPackIncludeMode::MetadataOnly,
        "ref_only" => ContextPackIncludeMode::RefOnly,
        _ => ContextPackIncludeMode::Summary,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use supernova_process_kernel::TaskAgentDecisionProtocol;

    fn product_model_config(model: &str) -> ModelConfig {
        ModelConfig {
            provider: "deepseek".into(),
            model: model.into(),
            thinking: "enabled".into(),
            reasoning_effort: "max".into(),
            token_budget: Some(4096),
            strict_tools: true,
        }
    }

    #[test]
    fn product_model_id_maps_to_kernel_route_preference() {
        let flash = to_kernel_model_config(product_model_config("deepseek-v4-flash"));
        assert_eq!(flash.model_route.mode, ModelRouteMode::Flash);
        assert_eq!(flash.model_route.fixed_model, None);

        let pro = to_kernel_model_config(product_model_config("deepseek-v4-pro"));
        assert_eq!(pro.model_route.mode, ModelRouteMode::Pro);
        assert_eq!(pro.model_route.fixed_model, None);
        assert_eq!(pro.output_budget.max_tokens, Some(4096));
        assert_eq!(pro.thinking.mode, ThinkingMode::Enabled);
        assert_eq!(pro.thinking.reasoning_effort, ReasoningEffort::Max);
        assert_eq!(
            pro.decision_protocol,
            TaskAgentDecisionProtocol::ProviderNativeToolCalls
        );
    }

    #[test]
    fn product_model_config_cannot_disable_provider_native_tools() {
        let mut model = product_model_config("deepseek-v4-flash");
        model.strict_tools = false;

        let config = to_kernel_model_config(model);

        assert_eq!(
            config.decision_protocol,
            TaskAgentDecisionProtocol::ProviderNativeToolCalls
        );
        assert!(config.tool_calling.enabled);
        assert!(config.tool_calling.strict_mode);
    }

    #[test]
    fn product_display_language_maps_to_kernel_response_language() {
        assert_eq!(
            to_kernel_response_language(DisplayLanguage::ZhCn),
            ResponseLanguage::ZhCn
        );
        assert_eq!(
            to_kernel_response_language(DisplayLanguage::EnUs),
            ResponseLanguage::EnUs
        );
    }

    #[test]
    fn product_model_config_with_language_sets_response_language() {
        let zh_config = to_kernel_model_config_with_language(
            Some(product_model_config("deepseek-v4-flash")),
            DisplayLanguage::ZhCn,
        );
        let default_model_config =
            to_kernel_model_config_with_language(None, DisplayLanguage::EnUs);

        assert_eq!(zh_config.response_language, ResponseLanguage::ZhCn);
        assert_eq!(
            default_model_config.response_language,
            ResponseLanguage::EnUs
        );
    }
}
