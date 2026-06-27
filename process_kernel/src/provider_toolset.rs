use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

use crate::model_config::{ModelInvocationConfig, ProviderToolsetMode};
use crate::model_runtime::ModelOperation;
use crate::provider_debug::append_provider_native_debug;
use crate::provider_tool::{
    provider_tool_capability_is_task_runtime_exposable, provider_tool_domain,
    ProviderToolDefinition, ProviderToolRegistry, CHAT_RUNTIME_PROVIDER_TOOL_CAPABILITIES,
};
use crate::{json_err, now_ms, safe_blob_name, CapabilityDescriptor, ProcessTruthStore};

pub const DEEPSEEK_MAX_PROVIDER_TOOLS: usize = 128;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderToolsetOmission {
    pub capability_id: String,
    pub domain: String,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderToolGroupDescriptor {
    pub group_id: String,
    pub title: String,
    pub description: String,
    pub capability_ids: Vec<String>,
    pub always_on: bool,
    pub approval_gated: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ProviderToolsetRecord {
    pub model_call_id: String,
    pub operation: String,
    pub requested_mode: ProviderToolsetMode,
    pub effective_mode: ProviderToolsetMode,
    pub lifecycle_stage: String,
    #[serde(default)]
    pub selection_id: Option<String>,
    #[serde(default)]
    pub active_group_ids: Vec<String>,
    #[serde(default)]
    pub latest_selected_group_ids: Vec<String>,
    #[serde(default)]
    pub latest_selected_capability_ids: Vec<String>,
    #[serde(default)]
    pub toolset_index_guide: String,
    #[serde(default)]
    pub request_scoped_tool_guide: String,
    pub provider_limit: usize,
    pub schema_coverage_count: usize,
    pub model_capability_excluded_count: usize,
    pub selected_count: usize,
    pub selected_capability_ids: Vec<String>,
    pub selected_tools: Vec<ProviderToolDefinition>,
    pub omitted_tools: Vec<ProviderToolsetOmission>,
    pub domain_counts: BTreeMap<String, usize>,
    pub truncated_by_provider_limit: bool,
    pub downgraded_for_provider_limit: bool,
    pub progressive_disclosure: bool,
    pub created_at_ms: u128,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ProviderToolsetPlan {
    pub provider_toolset_ref: String,
    pub registry: ProviderToolRegistry,
    pub record: ProviderToolsetRecord,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderToolsetPlanError {
    pub error_code: String,
    pub message: String,
    pub requested_mode: ProviderToolsetMode,
    pub provider_limit: usize,
    pub schema_coverage_count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct LatestToolsetSelection {
    selection_id: String,
    accepted_groups: Vec<String>,
    accepted_capability_ids: Vec<String>,
    ttl_model_calls: u64,
    event_id: u64,
}

pub struct ProviderToolsetPlanner {
    registry: Vec<CapabilityDescriptor>,
    config: ModelInvocationConfig,
}

pub fn provider_tool_group_descriptors() -> Vec<ProviderToolGroupDescriptor> {
    vec![
        group(
            "core_control",
            "Core control",
            "Task completion, failure, clarification, ref/result navigation, ProcessTruth query, and toolset selection.",
            &[
                "process.toolset.select",
                "process.complete",
                "process.fail",
                "process.clarify",
                "process.read_ref",
                "process.query_events",
                "tool.result.page",
                "tool.result.search",
                "tool.result.inspect_schema",
            ],
            true,
            false,
        ),
        group(
            "read_workspace",
            "Read workspace",
            "Inspect workspace files, paths, hashes, diffs, inventories, and artifact facts without applying mutations.",
            &[
                "os.list_tree",
                "os.workspace_inventory",
                "os.stat_path",
                "os.read_file",
                "os.hash_path",
                "os.diff",
                "os.verify_artifact",
                "data.csv.read_dataset",
                "office.workbook.read_cells",
                "office.workbook.read_text",
                "document.pdf.extract_text",
            ],
            true,
            false,
        ),
        group(
            "ref_result_navigation",
            "Ref and result navigation",
            "Read ProcessTruth refs and page/search/inspect raw tool results.",
            &[
                "process.read_ref",
                "tool.result.page",
                "tool.result.search",
                "tool.result.inspect_schema",
            ],
            true,
            false,
        ),
        group(
            "source_set_batch",
            "SourceSet and batch workspace analysis",
            "Create/read SourceSets and run batch hash, duplicate, recent-change, and SourceSet coverage analysis.",
            &[
                "source_set.create",
                "source_set.read_page",
                "source_set.coverage_verify",
                "workspace.batch_hash",
                "workspace.find_duplicates",
                "workspace.recent_changes",
                "workspace.recent_changes_snapshot",
            ],
            true,
            false,
        ),
        group(
            "dataset_ops",
            "Dataset operations",
            "Verify and export dataset refs into user-visible CSV or Markdown artifacts.",
            &[
                "dataset.coverage_verify",
                "data.csv.read_dataset",
                "dataset.export_csv",
                "dataset.export_markdown",
            ],
            true,
            false,
        ),
        group(
            "artifact_write",
            "Artifact writing",
            "Create or materialize user-visible artifacts without mutating user source files.",
            &[
                "os.write_artifact",
                "os.write_temp_dataset",
                "workspace.tree_index",
                "workspace.perf_inventory",
                "dataset.export_csv",
                "dataset.export_markdown",
                "artifact.copy_source_set",
                "office.docx.create",
                "office.docx.rewrite_save_as",
            ],
            false,
            false,
        ),
        group(
            "artifact_quality",
            "Artifact quality checks",
            "Run local typed artifact verification and mechanical quality audit.",
            &["artifact.verify_typed", "artifact.audit_quality"],
            true,
            false,
        ),
        group(
            "mutation_preview",
            "Mutation preview",
            "Draft native source/workspace mutation plans when a plan ref is useful. Preview approval blocking is disabled on the RC0 run-through path.",
            &["workspace.plan_organize"],
            false,
            false,
        ),
        group(
            "mutation_apply",
            "Mutation intent",
            "Request workspace/source mutations. The Kernel executes directly under workspace boundaries, receipts, and rollback evidence.",
            &[
                "os.write_file",
                "os.write_source_mutation_apply",
                "os.copy_path",
                "os.move_path",
                "os.rename_path",
                "os.delete_path",
                "os.unzip",
                "workspace.apply_organize_tx",
                "workspace.rename_batch_apply",
                "office.docx.rewrite_in_place",
            ],
            false,
            false,
        ),
        group(
            "office_docx",
            "Office DOCX",
            "Read, validate, diff, create, and rewrite DOCX files through the Office runtime.",
            &[
                "office.docx.read_text",
                "office.workbook.read_cells",
                "office.workbook.read_text",
                "document.pdf.extract_text",
                "office.docx.batch_read_text",
                "office.docx.batch_extract_metadata",
                "office.docx.batch_validate",
                "office.docx.rewrite_preview",
                "office.docx.diff_summary",
                "office.docx.validate",
                "office.docx.create",
                "office.docx.rewrite_save_as",
                "office.docx.rewrite_in_place",
            ],
            true,
            false,
        ),
        group(
            "package_release",
            "Package and release artifacts",
            "Build zip packages, manifests, checksums, and related release-supporting artifacts.",
            &["package.build_zip", "os.zip"],
            false,
            false,
        ),
        group(
            "client_environment_overview",
            "Client environment overview",
            "Inspect the sanitized local desktop environment overview. Detailed device, storage, network, runtime, and sensitive disclosure tools are selectable through the client_environment group.",
            &["client_env.scan_overview"],
            true,
            false,
        ),
        group(
            "client_environment",
            "Client environment",
            "Inspect sanitized local desktop environment facts such as locale, device, storage, network readiness, and runtime availability. Sensitive fields require explicit user authorization.",
            &[
                "client_env.scan_overview",
                "client_env.scan_device",
                "client_env.scan_storage",
                "client_env.scan_network",
                "client_env.scan_runtimes",
                "client_env.read_snapshot",
                "client_env.request_sensitive_disclosure",
            ],
            false,
            false,
        ),
        group(
            "terminal_fallback",
            "Terminal fallback",
            "Use controlled terminal execution only when no native capability fits. Use run_command for bounded foreground commands and start_service/stop_service/service_status for long-running servers or dev services.",
            &[
                "terminal.run_command",
                "terminal.start_service",
                "terminal.stop_service",
                "terminal.service_status",
            ],
            false,
            true,
        ),
        group(
            "process_structure",
            "Process structure",
            "Fork child processes for explicit process-structure workflows.",
            &["process.fork_child"],
            false,
            false,
        ),
        group(
            "rollback_recovery",
            "Rollback and recovery",
            "Rollback recorded transactions when recovery is required.",
            &["os.rollback_tx"],
            false,
            true,
        ),
    ]
}

fn group(
    group_id: &str,
    title: &str,
    description: &str,
    capability_ids: &[&str],
    always_on: bool,
    approval_gated: bool,
) -> ProviderToolGroupDescriptor {
    ProviderToolGroupDescriptor {
        group_id: group_id.to_string(),
        title: title.to_string(),
        description: description.to_string(),
        capability_ids: capability_ids.iter().map(|item| item.to_string()).collect(),
        always_on,
        approval_gated,
    }
}

impl ProviderToolsetPlanner {
    pub fn new(registry: Vec<CapabilityDescriptor>, config: ModelInvocationConfig) -> Self {
        Self { registry, config }
    }

    pub fn coverage_registry(&self) -> ProviderToolRegistry {
        ProviderToolRegistry::phase6_schema_coverage(&self.registry, &self.config)
    }

    pub fn plan_domain(&self, domain: &str) -> ProviderToolRegistry {
        let capability_ids = self
            .coverage_capability_ids()
            .into_iter()
            .filter(|capability_id| provider_tool_domain(capability_id) == domain)
            .collect::<Vec<_>>();
        ProviderToolRegistry::phase6_selected(
            &self.registry,
            &self.config,
            &capability_ids,
            "phase6_domain_scoped",
        )
    }

    pub fn plan_and_record(
        &self,
        truth: &ProcessTruthStore,
        pid: &str,
        model_call_id: &str,
        operation: &ModelOperation,
    ) -> Result<ProviderToolsetPlan, ProviderToolsetPlanError> {
        let coverage_ids = self.coverage_capability_ids();
        let model_capability_excluded_count = self
            .registry
            .iter()
            .filter(|descriptor| descriptor.capability_id.starts_with("model."))
            .count();
        let requested_mode = self.config.tool_calling.toolset_mode.clone();
        let provider_limit = self
            .config
            .tool_calling
            .max_provider_tools_per_request
            .min(DEEPSEEK_MAX_PROVIDER_TOOLS)
            .max(1);
        let latest_selection = self.latest_toolset_selection(truth);
        let (
            mut selected_ids,
            effective_mode,
            downgraded_for_provider_limit,
            active_group_ids,
            latest_selected_group_ids,
            latest_selected_capability_ids,
        ) = self.select_capabilities(&coverage_ids, latest_selection.as_ref(), truth);
        let lifecycle_stage = match effective_mode {
            ProviderToolsetMode::FullRegistered => "full_registered",
            ProviderToolsetMode::Rc0FullVisible => "rc0_full_visible",
            ProviderToolsetMode::IndexedGroups => "indexed_groups",
            ProviderToolsetMode::StateAwareExpanded => "state_aware_expanded",
            ProviderToolsetMode::DomainScoped => "domain_scoped",
            ProviderToolsetMode::MinimalDecision => "minimal_decision",
        }
        .to_string();
        let selection_id = latest_selection
            .as_ref()
            .map(|selection| selection.selection_id.clone());

        if matches!(
            requested_mode,
            ProviderToolsetMode::FullRegistered | ProviderToolsetMode::Rc0FullVisible
        ) && selected_ids.len() > provider_limit
        {
            let error = ProviderToolsetPlanError {
                error_code: "PROVIDER_TOOLSET_LIMIT_EXCEEDED".to_string(),
                message: format!(
                    "{:?} provider toolset requested {} tools, exceeding provider limit {}",
                    requested_mode,
                    selected_ids.len(),
                    provider_limit
                ),
                requested_mode,
                provider_limit,
                schema_coverage_count: coverage_ids.len(),
            };
            let _ = truth.append_event(
                Some(pid),
                "provider_toolset_planning_failed",
                json!({
                    "model_call_id": model_call_id,
                    "error": error.clone(),
                    "fail_closed": true,
                }),
            );
            return Err(error);
        }

        let selected_before_limit = selected_ids.iter().cloned().collect::<BTreeSet<_>>();
        let mut truncated_by_provider_limit = false;
        if selected_ids.len() > provider_limit {
            selected_ids.truncate(provider_limit);
            truncated_by_provider_limit = true;
        }
        let selected_after_limit = selected_ids.iter().cloned().collect::<BTreeSet<_>>();
        let mut omitted_tools = Vec::new();
        for capability_id in &coverage_ids {
            if selected_after_limit.contains(capability_id) {
                continue;
            }
            let reason = if selected_before_limit.contains(capability_id) {
                "provider_limit_trimmed"
            } else {
                "indexed_group_not_selected"
            };
            omitted_tools.push(ProviderToolsetOmission {
                capability_id: capability_id.clone(),
                domain: provider_tool_domain(capability_id),
                reason: reason.to_string(),
            });
        }

        let registry = ProviderToolRegistry::phase6_selected(
            &self.registry,
            &self.config,
            &selected_ids,
            match effective_mode {
                ProviderToolsetMode::Rc0FullVisible => "rc0_full_visible",
                ProviderToolsetMode::FullRegistered => "full_registered",
                _ => "phase6_progressive_disclosure",
            },
        );
        let toolset_index_guide = provider_toolset_index_guide(&effective_mode);
        let request_scoped_tool_guide = request_scoped_tool_guide(
            &registry,
            &effective_mode,
            &active_group_ids,
            selection_id.as_deref(),
        );
        let domain_counts = domain_counts(&coverage_ids);
        let record = ProviderToolsetRecord {
            model_call_id: model_call_id.to_string(),
            operation: operation.as_str().to_string(),
            requested_mode,
            effective_mode,
            lifecycle_stage,
            selection_id,
            active_group_ids,
            latest_selected_group_ids,
            latest_selected_capability_ids,
            toolset_index_guide,
            request_scoped_tool_guide,
            provider_limit,
            schema_coverage_count: coverage_ids.len(),
            model_capability_excluded_count,
            selected_count: selected_ids.len(),
            selected_capability_ids: selected_ids,
            selected_tools: registry.tools.clone(),
            omitted_tools,
            domain_counts,
            truncated_by_provider_limit,
            downgraded_for_provider_limit,
            progressive_disclosure: matches!(
                self.config.tool_calling.toolset_mode,
                ProviderToolsetMode::IndexedGroups | ProviderToolsetMode::StateAwareExpanded
            ),
            created_at_ms: now_ms(),
        };
        let provider_toolset_ref = truth
            .write_blob(
                &format!(
                    "provider_toolsets/{}_{}.json",
                    safe_blob_name(model_call_id),
                    now_ms()
                ),
                &serde_json::to_vec_pretty(&record)
                    .map_err(json_err)
                    .map_err(|err| ProviderToolsetPlanError {
                        error_code: "PROVIDER_TOOLSET_RECORD_JSON_INVALID".to_string(),
                        message: err.to_string(),
                        requested_mode: self.config.tool_calling.toolset_mode.clone(),
                        provider_limit,
                        schema_coverage_count: coverage_ids.len(),
                    })?,
            )
            .map_err(|err| ProviderToolsetPlanError {
                error_code: "PROVIDER_TOOLSET_RECORD_WRITE_FAILED".to_string(),
                message: err.to_string(),
                requested_mode: self.config.tool_calling.toolset_mode.clone(),
                provider_limit,
                schema_coverage_count: coverage_ids.len(),
            })?;
        let _ = truth.append_event(
            Some(pid),
            "provider_toolset_planned",
            json!({
                "model_call_id": model_call_id,
                "provider_toolset_ref": provider_toolset_ref,
                "requested_mode": record.requested_mode,
                "effective_mode": record.effective_mode,
                "lifecycle_stage": record.lifecycle_stage,
                "selection_id": record.selection_id.clone(),
                "active_group_ids": record.active_group_ids.clone(),
                "latest_selected_group_ids": record.latest_selected_group_ids.clone(),
                "latest_selected_capability_ids": record.latest_selected_capability_ids.clone(),
                "schema_coverage_count": record.schema_coverage_count,
                "selected_count": record.selected_count,
                "omitted_count": record.omitted_tools.len(),
                "provider_limit": record.provider_limit,
                "truncated_by_provider_limit": record.truncated_by_provider_limit,
                "downgraded_for_provider_limit": record.downgraded_for_provider_limit,
                "model_capability_excluded_count": record.model_capability_excluded_count,
            }),
        );
        let _ = append_provider_native_debug(
            truth,
            "toolset_planned",
            json!({
                "model_call_id": model_call_id,
                "provider_toolset_ref": provider_toolset_ref,
                "toolset_contains": record.selected_capability_ids.clone(),
                "decision_protocol": "provider_native_tool_calls",
                "diagnostic": {
                    "requested_mode": record.requested_mode.clone(),
                    "effective_mode": record.effective_mode.clone(),
                    "lifecycle_stage": record.lifecycle_stage,
                    "selection_id": record.selection_id.clone(),
                    "active_group_ids": record.active_group_ids.clone(),
                    "latest_selected_group_ids": record.latest_selected_group_ids.clone(),
                    "latest_selected_capability_ids": record.latest_selected_capability_ids.clone(),
                    "provider_limit": record.provider_limit,
                    "selected_count": record.selected_count,
                    "omitted_count": record.omitted_tools.len(),
                    "domain_counts": record.domain_counts.clone(),
                    "truncated_by_provider_limit": record.truncated_by_provider_limit,
                    "downgraded_for_provider_limit": record.downgraded_for_provider_limit,
                    "progressive_disclosure": record.progressive_disclosure,
                    "selected_tools": record.selected_tools.iter().map(|tool| {
                        json!({
                            "name": tool.function.name.clone(),
                            "required": tool.function.parameters
                                .get("required")
                                .cloned()
                                .unwrap_or_else(|| json!([])),
                            "strict": tool.function.strict,
                        })
                    }).collect::<Vec<_>>(),
                }
            }),
        );
        if record.truncated_by_provider_limit {
            let _ = truth.append_event(
                Some(pid),
                "provider_toolset_trimmed",
                json!({
                    "model_call_id": model_call_id,
                    "provider_toolset_ref": provider_toolset_ref,
                    "provider_limit": record.provider_limit,
                    "omitted_count": record.omitted_tools.len(),
                }),
            );
        }
        if record.downgraded_for_provider_limit {
            let _ = truth.append_event(
                Some(pid),
                "provider_toolset_downgraded",
                json!({
                    "model_call_id": model_call_id,
                    "provider_toolset_ref": provider_toolset_ref,
                    "requested_mode": record.requested_mode,
                    "effective_mode": record.effective_mode,
                    "reason": "schema_coverage_exceeds_provider_limit",
                }),
            );
        }

        Ok(ProviderToolsetPlan {
            provider_toolset_ref,
            registry,
            record,
        })
    }

    pub fn plan_chat_runtime_readonly(
        &self,
        truth: &ProcessTruthStore,
        pid: &str,
        model_call_id: &str,
    ) -> Result<ProviderToolsetPlan, ProviderToolsetPlanError> {
        let provider_limit = self
            .config
            .tool_calling
            .max_provider_tools_per_request
            .min(DEEPSEEK_MAX_PROVIDER_TOOLS)
            .max(1);
        let mut selected_ids = CHAT_RUNTIME_PROVIDER_TOOL_CAPABILITIES
            .iter()
            .map(|item| item.to_string())
            .filter(|capability_id| {
                self.registry
                    .iter()
                    .any(|descriptor| descriptor.capability_id == *capability_id)
            })
            .collect::<Vec<_>>();
        let selected_before_limit = selected_ids.iter().cloned().collect::<BTreeSet<_>>();
        let truncated_by_provider_limit = selected_ids.len() > provider_limit;
        if truncated_by_provider_limit {
            selected_ids.truncate(provider_limit);
        }
        let selected_after_limit = selected_ids.iter().cloned().collect::<BTreeSet<_>>();
        let omitted_tools = CHAT_RUNTIME_PROVIDER_TOOL_CAPABILITIES
            .iter()
            .filter(|capability_id| !selected_after_limit.contains(**capability_id))
            .map(|capability_id| ProviderToolsetOmission {
                capability_id: capability_id.to_string(),
                domain: provider_tool_domain(capability_id),
                reason: if selected_before_limit.contains(*capability_id) {
                    "provider_limit_trimmed".to_string()
                } else {
                    "chat_runtime_not_selected".to_string()
                },
            })
            .collect::<Vec<_>>();
        let registry = ProviderToolRegistry::chat_runtime_readonly(&self.registry, &self.config);
        let selected_tools = registry.tools.clone();
        let record = ProviderToolsetRecord {
            model_call_id: model_call_id.to_string(),
            operation: ModelOperation::ChatTurn.as_str().to_string(),
            requested_mode: self.config.tool_calling.toolset_mode.clone(),
            effective_mode: ProviderToolsetMode::DomainScoped,
            lifecycle_stage: "chat_runtime_readonly".to_string(),
            selection_id: None,
            active_group_ids: vec![
                "chat_control".to_string(),
                "chat_readonly".to_string(),
                "client_environment".to_string(),
            ],
            latest_selected_group_ids: Vec::new(),
            latest_selected_capability_ids: Vec::new(),
            toolset_index_guide: "ChatRuntime exposes chat control tools plus read-only workspace/ref/office tools. Mutation, preview, task completion, and terminal tools are forbidden; use chat.needs_task for executable work.".to_string(),
            request_scoped_tool_guide: "Use chat.answer for the final chat reply, chat.clarify for missing user facts, and chat.needs_task when the request requires mutation, approval, long-running execution, or artifact delivery. Read-only tools may inspect context only.".to_string(),
            provider_limit,
            schema_coverage_count: CHAT_RUNTIME_PROVIDER_TOOL_CAPABILITIES.len(),
            model_capability_excluded_count: self
                .registry
                .iter()
                .filter(|descriptor| descriptor.capability_id.starts_with("model."))
                .count(),
            selected_count: selected_tools.len(),
            selected_capability_ids: selected_ids,
            selected_tools,
            omitted_tools,
            domain_counts: domain_counts(
                &CHAT_RUNTIME_PROVIDER_TOOL_CAPABILITIES
                    .iter()
                    .map(|item| item.to_string())
                    .collect::<Vec<_>>(),
            ),
            truncated_by_provider_limit,
            downgraded_for_provider_limit: false,
            progressive_disclosure: false,
            created_at_ms: now_ms(),
        };
        let provider_toolset_ref = truth
            .write_blob(
                &format!(
                    "provider_toolsets/{}_chat_runtime_{}.json",
                    safe_blob_name(model_call_id),
                    now_ms()
                ),
                &serde_json::to_vec_pretty(&record)
                    .map_err(json_err)
                    .map_err(|err| ProviderToolsetPlanError {
                        error_code: "CHAT_PROVIDER_TOOLSET_RECORD_JSON_INVALID".to_string(),
                        message: err.to_string(),
                        requested_mode: self.config.tool_calling.toolset_mode.clone(),
                        provider_limit,
                        schema_coverage_count: CHAT_RUNTIME_PROVIDER_TOOL_CAPABILITIES.len(),
                    })?,
            )
            .map_err(|err| ProviderToolsetPlanError {
                error_code: "CHAT_PROVIDER_TOOLSET_RECORD_WRITE_FAILED".to_string(),
                message: err.to_string(),
                requested_mode: self.config.tool_calling.toolset_mode.clone(),
                provider_limit,
                schema_coverage_count: CHAT_RUNTIME_PROVIDER_TOOL_CAPABILITIES.len(),
            })?;
        let _ = truth.append_event(
            Some(pid),
            "chat_provider_toolset_planned",
            json!({
                "model_call_id": model_call_id,
                "provider_toolset_ref": provider_toolset_ref,
                "selected_count": record.selected_count,
                "selected_capability_ids": record.selected_capability_ids,
                "omitted_count": record.omitted_tools.len(),
                "mutation_allowed": false,
            }),
        );
        Ok(ProviderToolsetPlan {
            provider_toolset_ref,
            registry,
            record,
        })
    }

    fn coverage_capability_ids(&self) -> Vec<String> {
        self.registry
            .iter()
            .filter(|descriptor| {
                provider_tool_capability_is_task_runtime_exposable(&descriptor.capability_id)
            })
            .map(|descriptor| descriptor.capability_id.clone())
            .collect()
    }

    fn select_capabilities(
        &self,
        coverage_ids: &[String],
        latest_selection: Option<&LatestToolsetSelection>,
        truth: &ProcessTruthStore,
    ) -> (
        Vec<String>,
        ProviderToolsetMode,
        bool,
        Vec<String>,
        Vec<String>,
        Vec<String>,
    ) {
        let requested_mode = self.config.tool_calling.toolset_mode.clone();
        let active_approval_capabilities = active_approval_operation_capabilities(truth);
        let mut active_group_ids = always_on_group_ids();
        let latest_selected_group_ids = latest_selection
            .map(|selection| selection.accepted_groups.clone())
            .unwrap_or_default();
        let latest_selected_capability_ids = latest_selection
            .map(|selection| selection.accepted_capability_ids.clone())
            .unwrap_or_default();
        for group_id in &latest_selected_group_ids {
            if !active_group_ids.contains(group_id) {
                active_group_ids.push(group_id.clone());
            }
        }
        let mut selected = match requested_mode {
            ProviderToolsetMode::MinimalDecision => minimal_decision_capabilities(),
            ProviderToolsetMode::DomainScoped => domain_scoped_capabilities(),
            ProviderToolsetMode::StateAwareExpanded | ProviderToolsetMode::IndexedGroups => {
                indexed_group_capabilities(
                    &active_group_ids,
                    &latest_selected_capability_ids,
                    &active_approval_capabilities,
                )
            }
            ProviderToolsetMode::Rc0FullVisible | ProviderToolsetMode::FullRegistered => {
                coverage_ids.to_vec()
            }
        };
        if !matches!(
            requested_mode,
            ProviderToolsetMode::FullRegistered | ProviderToolsetMode::Rc0FullVisible
        ) {
            selected.extend(active_approval_capabilities.iter().cloned());
        }
        let coverage = coverage_ids.iter().cloned().collect::<BTreeSet<_>>();
        let mut deduped = Vec::new();
        let mut seen = BTreeSet::new();
        for capability_id in selected {
            if coverage.contains(&capability_id) && seen.insert(capability_id.clone()) {
                deduped.push(capability_id);
            }
        }
        if matches!(
            requested_mode,
            ProviderToolsetMode::FullRegistered | ProviderToolsetMode::Rc0FullVisible
        ) {
            return (
                deduped,
                requested_mode.clone(),
                false,
                vec![match requested_mode {
                    ProviderToolsetMode::Rc0FullVisible => "rc0_full_visible",
                    _ => "full_registered",
                }
                .to_string()],
                latest_selected_group_ids,
                latest_selected_capability_ids,
            );
        }
        let coverage_exceeds_limit = coverage_ids.len()
            > self
                .config
                .tool_calling
                .max_provider_tools_per_request
                .min(DEEPSEEK_MAX_PROVIDER_TOOLS)
                .max(1);
        let effective_mode = if matches!(
            requested_mode,
            ProviderToolsetMode::StateAwareExpanded | ProviderToolsetMode::IndexedGroups
        ) {
            ProviderToolsetMode::IndexedGroups
        } else {
            requested_mode
        };
        (
            deduped,
            effective_mode,
            coverage_exceeds_limit,
            active_group_ids,
            latest_selected_group_ids,
            latest_selected_capability_ids,
        )
    }

    fn latest_toolset_selection(
        &self,
        truth: &ProcessTruthStore,
    ) -> Option<LatestToolsetSelection> {
        let events = truth.read_events().ok()?;
        let event = events
            .iter()
            .rev()
            .find(|event| event.event_type == "provider_toolset_selection_recorded")?;
        let ttl_model_calls = event
            .data
            .get("ttl_model_calls")
            .and_then(Value::as_u64)
            .unwrap_or(4)
            .clamp(1, 6);
        let calls_after_selection = events
            .iter()
            .filter(|candidate| {
                candidate.event_id > event.event_id && candidate.event_type == "model_call_started"
            })
            .count() as u64;
        if calls_after_selection >= ttl_model_calls {
            return None;
        }
        Some(LatestToolsetSelection {
            selection_id: event
                .data
                .get("selection_id")
                .and_then(Value::as_str)
                .unwrap_or("toolset_selection_unknown")
                .to_string(),
            accepted_groups: string_array_field(&event.data, "accepted_groups"),
            accepted_capability_ids: string_array_field(&event.data, "accepted_capability_ids"),
            ttl_model_calls,
            event_id: event.event_id,
        })
    }
}

fn minimal_decision_capabilities() -> Vec<String> {
    let mut ids = process_control_capabilities();
    ids.extend(read_context_capabilities());
    ids.extend(artifact_write_always_on_capabilities());
    ids
}

fn domain_scoped_capabilities() -> Vec<String> {
    let mut ids = process_control_capabilities();
    ids.extend(read_context_capabilities());
    ids.extend(artifact_write_always_on_capabilities());
    ids
}

fn process_control_capabilities() -> Vec<String> {
    [
        "process.complete",
        "process.fail",
        "process.clarify",
        "process.toolset.select",
        "process.read_ref",
        "process.query_events",
        "tool.result.page",
        "tool.result.search",
        "tool.result.inspect_schema",
    ]
    .iter()
    .map(|item| item.to_string())
    .collect()
}

fn read_context_capabilities() -> Vec<String> {
    [
        "os.list_tree",
        "os.workspace_inventory",
        "os.stat_path",
        "os.read_file",
        "os.hash_path",
        "os.diff",
        "os.verify_artifact",
        "data.csv.read_dataset",
        "office.workbook.read_cells",
        "office.workbook.read_text",
        "document.pdf.extract_text",
        "client_env.scan_overview",
        "source_set.create",
        "source_set.read_page",
        "source_set.coverage_verify",
        "workspace.batch_hash",
        "workspace.find_duplicates",
        "workspace.recent_changes",
        "workspace.recent_changes_snapshot",
        "dataset.coverage_verify",
        "artifact.verify_typed",
        "artifact.audit_quality",
        "office.docx.read_text",
        "office.workbook.read_cells",
        "office.workbook.read_text",
        "document.pdf.extract_text",
        "office.docx.batch_read_text",
        "office.docx.batch_extract_metadata",
        "office.docx.batch_validate",
        "office.docx.rewrite_preview",
        "office.docx.diff_summary",
        "office.docx.validate",
    ]
    .iter()
    .map(|item| item.to_string())
    .collect()
}

fn artifact_write_always_on_capabilities() -> Vec<String> {
    ["os.write_artifact", "os.write_temp_dataset"]
        .iter()
        .map(|item| item.to_string())
        .collect()
}

fn always_on_group_ids() -> Vec<String> {
    provider_tool_group_descriptors()
        .into_iter()
        .filter(|group| group.always_on)
        .map(|group| group.group_id)
        .collect()
}

fn indexed_group_capabilities(
    active_group_ids: &[String],
    selected_capability_ids: &[String],
    active_approval_capabilities: &BTreeSet<String>,
) -> Vec<String> {
    let active_groups = active_group_ids.iter().cloned().collect::<BTreeSet<_>>();
    let mut ids = Vec::new();
    for group in provider_tool_group_descriptors() {
        if !active_groups.contains(&group.group_id) {
            continue;
        }
        for capability_id in group.capability_ids {
            if indexed_capability_allowed(&capability_id, active_approval_capabilities) {
                ids.push(capability_id);
            }
        }
    }
    for capability_id in selected_capability_ids {
        if indexed_capability_allowed(capability_id, active_approval_capabilities) {
            ids.push(capability_id.clone());
        }
    }
    ids.extend(artifact_write_always_on_capabilities());
    dedupe_strings(ids)
}

fn indexed_capability_allowed(
    capability_id: &str,
    active_approval_capabilities: &BTreeSet<String>,
) -> bool {
    let _ = (capability_id, active_approval_capabilities);
    true
}

fn active_approval_operation_capabilities(truth: &ProcessTruthStore) -> BTreeSet<String> {
    let Ok(events) = truth.read_events() else {
        return BTreeSet::new();
    };
    let consumed = events
        .iter()
        .filter(|event| {
            matches!(
                event.event_type.as_str(),
                "approval_token_consumed" | "approval_token_used"
            )
        })
        .filter_map(|event| event.data.get("approval_token_id").and_then(Value::as_str))
        .map(str::to_string)
        .collect::<BTreeSet<_>>();
    let closed_txs = events
        .iter()
        .filter(|event| event.event_type == "preview_tx_closed")
        .filter_map(|event| event.data.get("tx_id").and_then(Value::as_str))
        .map(str::to_string)
        .collect::<BTreeSet<_>>();
    let mut capabilities = BTreeSet::new();
    for event in &events {
        if event.event_type != "approval_token_issued" {
            continue;
        }
        let Some(token_id) = event.data.get("approval_token_id").and_then(Value::as_str) else {
            continue;
        };
        if consumed.contains(token_id) {
            continue;
        }
        if event
            .data
            .get("tx_id")
            .and_then(Value::as_str)
            .is_some_and(|tx_id| closed_txs.contains(tx_id))
        {
            continue;
        }
        for capability_id in string_array_field(&event.data, "approved_operation_scope") {
            capabilities.insert(capability_id);
        }
    }
    capabilities
}

fn provider_toolset_index_guide(mode: &ProviderToolsetMode) -> String {
    let groups = provider_tool_group_descriptors()
        .into_iter()
        .filter(|group| !group.always_on)
        .map(|group| {
            format!(
                "- `{}`: {} {}",
                group.group_id, group.title, group.description
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    if matches!(mode, ProviderToolsetMode::Rc0FullVisible) {
        return format!(
            "[Toolset Index]\nRC0 full visible mode exposes all task-runtime provider tools that fit the provider limit. `cap_process_toolset_select` remains available only as a future optimization/backoff path; do not call it before using an already visible tool.\nSelectable groups retained for compatibility:\n{groups}"
        );
    }
    format!(
        "[Toolset Index]\n`cap_process_toolset_select` is always available. Call it only when the current exposed tools are insufficient for the next executable action.\nSelectable groups:\n{groups}\nDo not request all groups by default; choose at most the groups needed for the next step."
    )
}

fn request_scoped_tool_guide(
    registry: &ProviderToolRegistry,
    mode: &ProviderToolsetMode,
    active_group_ids: &[String],
    selection_id: Option<&str>,
) -> String {
    let tools = registry
        .tools
        .iter()
        .map(|tool| format!("- `{}`: {}", tool.function.name, tool.function.description))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "[Current Toolset]\nSelection id: {}\nActive groups: {}\nAvailable tools:\n{}",
        selection_id.unwrap_or(match mode {
            ProviderToolsetMode::Rc0FullVisible => "rc0_full_visible",
            ProviderToolsetMode::FullRegistered => "full_registered",
            _ => "default_indexed_groups",
        }),
        active_group_ids.join(", "),
        tools
    )
}

fn string_array_field(value: &Value, key: &str) -> Vec<String> {
    value
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

fn dedupe_strings(items: Vec<String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for item in items {
        if seen.insert(item.clone()) {
            out.push(item);
        }
    }
    out
}

fn domain_counts(capability_ids: &[String]) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for capability_id in capability_ids {
        *counts
            .entry(provider_tool_domain(capability_id))
            .or_insert(0) += 1;
    }
    counts
}
