use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub mod action;
pub mod agent_container;
pub mod agent_prompt;
pub mod approval_runtime;
pub mod artifact_runtime;
pub mod capability_argument_error;
pub mod capability_kernel;
pub mod chat_runtime;
pub mod chat_truth;
pub mod child_process;
pub mod client_env_runtime;
pub mod closure_gate;
pub mod container_context;
pub mod container_store;
pub mod context_budget;
pub mod context_compaction;
pub mod context_pack;
pub mod context_window;
pub mod data_runtime;
pub mod deepseek_provider;
pub mod kernel_api;
pub const PROCESS_KERNEL_SCHEMA_VERSION: &str = "supernova_process_kernel.v2.mvp";
pub const PROCESS_TRUTH_EVENT_SCHEMA_VERSION: &str = "supernova_process_truth_event.v2";
pub const RUNTIME_DIR_NAME: &str = ".supernova_v2";
pub const PROCESS_TRUTH_DB_FILENAME: &str = "process_truth.sqlite3";
pub const PROCESS_TRUTH_EXPORT_FILENAME: &str = "process_truth.jsonl";
static TX_COUNTER: AtomicU64 = AtomicU64::new(1);
pub mod model_config;
pub mod model_retry;
pub mod model_runtime;
pub mod model_stream;
pub mod observation;
pub mod office_runtime;
pub mod os_runtime;
pub mod package_runtime;
pub mod provider_credentials;
pub mod provider_debug;
pub mod provider_tool;
pub mod provider_tool_loop_executor;
pub mod provider_toolset;
pub mod provider_transcript;
pub mod read_only_capability_executor;
pub mod reasoning;
pub mod root_process;
pub mod source_guidance;
pub mod task_agent;
pub mod task_agent_runtime;
pub mod task_agent_session;
pub mod task_context_state;
pub mod terminal_runtime;
pub use action::{
    ActionValidationResult, ProcessAction, ProcessActionKind, ProcessActionValidator,
};
pub use agent_container::{
    AgentContainer, AgentContainerStatus, ContainerTimelineItem, ContainerTimelineItemKind,
    MemoryBinding,
};
pub use agent_prompt::{
    task_agent_decision_instruction, task_agent_decision_instruction_for_protocol,
    task_agent_provider_native_system_prompt, task_agent_system_prompt,
    task_agent_system_prompt_for_protocol, TaskAgentPromptProtocol,
};
pub use approval_runtime::{
    ApprovalRuntime, ApprovalTokenRecord, ExecutablePreviewOperation, PreviewTx,
};
pub use artifact_runtime::ArtifactRuntime;
pub use capability_argument_error::{
    build_capability_argument_error, invalid_write_kind_argument_error, CapabilityArgumentError,
    CapabilityInvalidField,
};
pub use capability_kernel::{
    artifact_path_generated_by_current_job, bind_preview_capability_receipt_to_tx,
    build_capability_approval_request, descriptor_approval_policy,
    executable_preview_operations_from_scope, expand_preview_target_paths_for_actions,
    finalize_capability_approval, prepare_capability_approval, CapabilityApprovalGuard,
    CapabilityApprovalPolicy, CapabilityApprovalRequest,
};
pub use chat_runtime::{
    chat_model_syscall_truth_id, ChatRuntime, ChatRuntimeStatus, ChatTurnRequest, ChatTurnResult,
    ChatTurnStatus, SuggestedTaskRequest,
};
pub use chat_truth::{
    ChatEvent, ChatProviderTranscript, ChatThread, ChatTruthStore, CHAT_TRUTH_SCHEMA_VERSION,
};
pub use child_process::{ChildProcessController, ChildProcessKind, ChildProcessReceipt};
pub use client_env_runtime::{
    ClientEnvDisclosureRequest, ClientEnvDisclosureToken, ClientEnvRedactionReport,
    ClientEnvRuntime, ClientEnvScanOptions, ClientEnvSection, ClientEnvSnapshot,
    ClientLocaleContext, CLIENT_ENV_ORIGIN, CLIENT_ENV_SNAPSHOT_SCHEMA_VERSION,
    CLIENT_LOCALE_CONTEXT_SCHEMA_VERSION,
};
pub use closure_gate::{
    check_closure_gate, check_closure_gate_for_claimed_artifacts, closure_block_receipt,
    ClosureGateResult,
};
pub use container_context::ContainerContextWindowAdapter;
pub use container_store::ContainerStore;
pub use context_budget::{context_window_tokens_for_budget, ModelContextProfile};
pub use context_compaction::{
    chat_context_summary_output_schema, container_context_summary_output_schema,
    task_context_summary_output_schema, validate_chat_context_summary,
    validate_container_context_summary, validate_task_context_summary, ContextCheckpointReceipt,
    ContextCompactionInput, ContextCompactionReceipt, ProviderTranscriptProtocolValidator,
    ProviderTranscriptReplacement, ProviderTranscriptValidationReceipt,
};
pub use context_pack::{
    ContextPack, ContextPackAutoPolicy, ContextPackIncludeMode, ContextPackItem,
    ContextPackItemKind,
};
pub use context_window::{
    append_task_context_window_events, ContextScope, ContextWindowBreakdown,
    ContextWindowControlConfig, ContextWindowController, ContextWindowDecision,
    ContextWindowDecisionKind, ContextWindowEstimate, ContextWindowEvent, ContextWindowPreflight,
    ContextWindowRequestParts, ContextWindowScopeAdapter, RuntimeKind, TokenEstimatorKind,
    CHAT_RUNTIME_MUTATION_FORBIDDEN_POLICY, CONTEXT_WINDOW_EVENT_SCHEMA_VERSION,
    CONTEXT_WINDOW_EVENT_TYPES, TASK_PROCESS_TRUTH_NOT_COMPRESSED_INVARIANT,
};
pub use data_runtime::{DataRuntime, DataSet, SourceSet, SourceSetFile};
pub use kernel_api::{JobStatusView, KernelApi};
pub use model_config::{
    estimate_text_tokens_conservative, ModelInvocationConfig, ModelRouteMode, ModelRoutePreference,
    ProviderToolsetMode, ReasoningEffort, ResponseLanguage, TaskAgentDecisionProtocol,
    ThinkingConfig, ThinkingMode, TokenBudgetConfig, ToolCallingConfig, ToolChoicePolicy,
};
pub use model_runtime::{
    default_model_provider_from_env, operation_supports_task_reasoning_stream,
    DeepSeekModelProvider, DeterministicModelProvider, MissingModelProvider, ModelAction,
    ModelBudget, ModelCallLedger, ModelCallReceipt, ModelFailurePolicy, ModelOperation,
    ModelProvider, ModelProviderFailure, ModelProviderRequest, ModelProviderResponse, ModelRuntime,
    ModelSchemaValidation, ModelStreamDelta, ModelStreamDeltaKind, ModelStreamSink,
    ProviderAssistantMessage, ProviderToolCall,
};
pub use observation::{
    ObservationBuilder, ObservedCapabilityReceipt, RawObservationFrame, RawToolResult,
    TaskObservation,
};
pub use office_runtime::OfficeRuntime;
pub use os_runtime::{OsRuntime, OsTxRecord, WorkspaceInventoryEntry};
pub use package_runtime::PackageRuntime;
pub use provider_credentials::{
    model_provider_from_profile_root_or_env, ProviderCredentialStore, ProviderProfileRecord,
    ProviderTestReceipt,
};
pub use provider_tool::{
    protocol_error_to_io, provider_native_tool_calls_enabled, provider_native_tool_request_enabled,
    provider_tool_call_arguments, provider_tool_call_name, provider_tool_capability_is_exposable,
    provider_tool_choice_value, provider_tool_domain, provider_tool_name_for_capability,
    provider_tool_parameters_for_descriptor, provider_tool_requires_explicit_approval_id,
    provider_tool_schema_is_strict_compatible, provider_tool_strict_compatible,
    ProviderToolBinding, ProviderToolDefinition, ProviderToolFunction, ProviderToolProtocolError,
    ProviderToolRegistry, CHAT_RUNTIME_PROVIDER_TOOL_CAPABILITIES,
    PHASE4_PROVIDER_TOOL_CAPABILITIES, PHASE5_MUTATION_APPLY_PROVIDER_TOOL_CAPABILITIES,
    PHASE5_PREVIEW_PROVIDER_TOOL_CAPABILITIES,
    PHASE7_STRICT_COMPATIBLE_READONLY_PROVIDER_TOOL_CAPABILITIES,
    PROVIDER_TOOL_PHASE6_FULL_COVERAGE,
};
pub use provider_tool_loop_executor::{
    ProviderToolExecution, ProviderToolLoopAdapter, ProviderToolLoopBudgetError,
    ProviderToolLoopBudgetErrorKind, ProviderToolLoopExecutor, ProviderToolLoopOutcome,
    ProviderToolLoopPolicy, ProviderToolLoopStatus,
};
pub use provider_toolset::{
    ProviderToolsetOmission, ProviderToolsetPlan, ProviderToolsetPlanError, ProviderToolsetPlanner,
    ProviderToolsetRecord, DEEPSEEK_MAX_PROVIDER_TOOLS,
};
pub use provider_transcript::{
    read_provider_messages, record_provider_assistant_response, record_provider_tool_result,
    record_provider_tool_result_with_metadata, record_provider_user_control_message,
    record_provider_user_message, replace_provider_visible_transcript_with_summary,
    replay_provider_transcript_state, replay_provider_transcript_state_from_events,
    ProviderToolCallState, ProviderToolResultMetadata, ProviderToolResultRecord,
    ProviderTranscriptMessage, ProviderTranscriptRecord, ProviderTranscriptReplacementRecord,
    ProviderTranscriptState, ProviderTranscriptSummary, ProviderTranscriptTokenEstimate,
    ProviderUserControlMessageRecord, ProviderUserMessageRecord,
};
pub use read_only_capability_executor::ReadOnlyCapabilityExecutor;
pub use reasoning::{NextActionDecision, TaskAgentDecisionKind};
pub use root_process::RootAgentProcessController;
pub use source_guidance::{ArtifactDestinationGuidance, ReferenceSourceDirective, SourceGuidance};
pub use task_agent::{TaskAgent, TaskAgentRunResult};
pub use task_agent_runtime::TaskAgentRuntime;
pub use task_agent_session::TaskAgentSession;
pub use task_context_state::{
    active_approval_token_ids, active_preview_ids, replay_task_context_state,
    replay_task_context_state_from_events, ApprovalTokenState, ArtifactContextEntry,
    ClosureContextState, PendingUserDecision, PreviewTxState, TaskContextState,
    VerificationContextEntry,
};
pub use terminal_runtime::{
    stop_terminal_services_for_job, terminal_command_mutation_detected, TerminalApproval,
    TerminalRuntime, TerminalServiceHealthCheck, TerminalServiceRecord, WorkspaceDiff,
    WorkspaceEntryState,
};
pub const JOB_STATUSES: [&str; 9] = [
    "created",
    "running",
    "waiting_user",
    "waiting_approval",
    "blocked",
    "failed",
    "interrupted",
    "completed",
    "cancelled",
];

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentJob {
    pub job_id: String,
    pub user_goal: String,
    pub workspace_root: String,
    pub status: String,
    pub root_pid: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentProcess {
    pub pid: String,
    pub ppid: Option<String>,
    pub job_id: String,
    pub process_type: String,
    pub state: String,
    pub input_refs: Vec<String>,
    pub output_refs: Vec<String>,
    pub capability_tokens: Vec<String>,
    pub budget_ms: Option<u64>,
    pub exit_code: Option<i32>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskAgentRuntimeRecord {
    pub runtime_id: String,
    pub job_id: String,
    pub root_pid: String,
    pub state: String,
    pub checkpoint_refs: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CheckpointRef {
    pub checkpoint_id: String,
    pub job_id: String,
    pub pid: String,
    pub runtime_id: Option<String>,
    pub kind: String,
    pub state_ref: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CheckpointRecord {
    pub checkpoint_id: String,
    pub job_id: String,
    pub pid: String,
    pub runtime_id: Option<String>,
    pub kind: String,
    pub state_ref: String,
    pub input_refs: Vec<String>,
    pub output_refs: Vec<String>,
    pub created_event_id: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ProcessRegistrySnapshot {
    pub jobs: Vec<AgentJob>,
    pub processes: Vec<AgentProcess>,
    pub task_agent_runtimes: Vec<TaskAgentRuntimeRecord>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CapabilityDescriptor {
    pub capability_id: String,
    pub input_schema: String,
    pub output_schema: String,
    pub preconditions: Vec<String>,
    pub side_effects: Vec<String>,
    pub required_permissions: Vec<String>,
    pub receipt_schema: String,
    pub verifier: String,
    pub rollback: String,
    pub derivation_type: String,
    pub is_lossless: bool,
    pub source_refs_required: bool,
    pub coverage_required: bool,
    pub verification_required: bool,
    pub fallback_allowed: bool,
    pub drilldown_supported: bool,
    pub approval_policy: String,
    pub target_path_schema: String,
    pub artifact_role: String,
    pub rollback_policy: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct VerifierSpec {
    pub verifier_id: String,
    pub target_id: String,
    pub input_refs: Vec<String>,
    pub checks: Vec<String>,
    pub failure_policy: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CapabilityToken {
    pub token_id: String,
    pub job_id: String,
    pub pid: String,
    pub workspace_root: String,
    pub capabilities: Vec<String>,
    pub permissions: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CapabilityReceipt {
    pub capability_id: String,
    pub job_id: String,
    pub pid: String,
    pub status: String,
    pub data: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ProcessEvent {
    pub schema_version: String,
    pub event_id: u64,
    pub timestamp_ms: u128,
    pub job_id: String,
    pub pid: Option<String>,
    pub event_type: String,
    pub data: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReplayState {
    pub job_id: String,
    pub status: String,
    pub event_count: usize,
    pub artifact_refs: Vec<String>,
    pub artifact_provenance: Vec<ArtifactProvenance>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactProvenance {
    pub event_id: u64,
    pub capability_id: String,
    pub artifact_ref: String,
    pub artifact_path: String,
    pub tx_id: Option<String>,
    pub source_refs: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ReplaySession {
    pub replay_id: String,
    pub job_id: String,
    pub state: ReplayState,
    pub events: Vec<ProcessEvent>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceGuard {
    workspace_root: PathBuf,
}

impl WorkspaceGuard {
    pub fn new(workspace_root: impl AsRef<Path>) -> io::Result<Self> {
        let root = workspace_root.as_ref().canonicalize()?;
        Ok(Self {
            workspace_root: root,
        })
    }

    pub fn root(&self) -> &Path {
        &self.workspace_root
    }

    pub fn resolve_workspace_path(
        &self,
        relative_path: impl AsRef<Path>,
    ) -> Result<PathBuf, String> {
        let relative_path = relative_path.as_ref();
        if relative_path.is_absolute() {
            return Err("absolute paths are not workspace-scoped capability inputs".to_string());
        }
        let mut resolved = self.workspace_root.clone();
        for component in relative_path.components() {
            match component {
                Component::CurDir => {}
                Component::Normal(part) => resolved.push(part),
                Component::ParentDir => {
                    if resolved == self.workspace_root || !resolved.pop() {
                        return Err("path traversal leaves workspace boundary".to_string());
                    }
                    if !resolved.starts_with(&self.workspace_root) {
                        return Err("path traversal leaves workspace boundary".to_string());
                    }
                }
                Component::Prefix(_) | Component::RootDir => {
                    return Err(
                        "rooted paths are not workspace-scoped capability inputs".to_string()
                    );
                }
            }
        }
        if !resolved.starts_with(&self.workspace_root) {
            return Err("resolved path leaves workspace boundary".to_string());
        }
        Ok(resolved)
    }
}

#[derive(Clone, Debug)]
pub struct RuntimeStateRoot {
    pub workspace_root: PathBuf,
    pub state_root: PathBuf,
}

impl RuntimeStateRoot {
    pub fn legacy(workspace_root: impl AsRef<Path>) -> io::Result<Self> {
        let guard = WorkspaceGuard::new(workspace_root)?;
        Ok(Self {
            workspace_root: guard.root().to_path_buf(),
            state_root: guard.root().join(RUNTIME_DIR_NAME),
        })
    }

    pub fn new(workspace_root: impl AsRef<Path>, state_root: impl AsRef<Path>) -> io::Result<Self> {
        let guard = WorkspaceGuard::new(workspace_root)?;
        fs::create_dir_all(state_root.as_ref())?;
        Ok(Self {
            workspace_root: guard.root().to_path_buf(),
            state_root: state_root.as_ref().canonicalize()?,
        })
    }
}

#[derive(Clone, Debug)]
pub struct ProcessTruthStore {
    workspace_root: PathBuf,
    state_root: PathBuf,
    job_id: String,
    db_path: PathBuf,
}

impl ProcessTruthStore {
    pub fn new(workspace_root: impl AsRef<Path>, job_id: impl Into<String>) -> io::Result<Self> {
        let guard = WorkspaceGuard::new(workspace_root)?;
        let state_root = guard.root().join(RUNTIME_DIR_NAME);
        Self::new_with_state_root(guard.root(), state_root, job_id)
    }

    pub fn new_with_state_root(
        workspace_root: impl AsRef<Path>,
        state_root: impl AsRef<Path>,
        job_id: impl Into<String>,
    ) -> io::Result<Self> {
        let guard = WorkspaceGuard::new(workspace_root)?;
        fs::create_dir_all(state_root.as_ref())?;
        let state_root = state_root.as_ref().canonicalize()?;
        let job_id = job_id.into();
        let truth_dir = state_root.join("process_truth");
        fs::create_dir_all(&truth_dir)?;
        let db_path = truth_dir.join(PROCESS_TRUTH_DB_FILENAME);
        let store = Self {
            workspace_root: guard.root().to_path_buf(),
            state_root,
            job_id,
            db_path,
        };
        store.init_schema()?;
        Ok(store)
    }

    pub fn path(&self) -> &Path {
        &self.db_path
    }

    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    pub fn state_root(&self) -> &Path {
        &self.state_root
    }

    pub fn job_id(&self) -> &str {
        &self.job_id
    }

    pub fn export_path(&self) -> PathBuf {
        self.state_root
            .join("process_truth")
            .join(format!("{}_{}", self.job_id, PROCESS_TRUTH_EXPORT_FILENAME))
    }

    fn connect(&self) -> io::Result<Connection> {
        Connection::open(&self.db_path).map_err(sql_err)
    }

    fn init_schema(&self) -> io::Result<()> {
        let conn = self.connect()?;
        conn.execute_batch(
            r#"
            PRAGMA journal_mode = WAL;
            CREATE TABLE IF NOT EXISTS jobs (
                job_id TEXT PRIMARY KEY,
                user_goal TEXT NOT NULL,
                workspace_root TEXT NOT NULL,
                status TEXT NOT NULL,
                root_pid TEXT NOT NULL,
                data_json TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS processes (
                pid TEXT PRIMARY KEY,
                ppid TEXT,
                job_id TEXT NOT NULL,
                process_type TEXT NOT NULL,
                state TEXT NOT NULL,
                data_json TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS task_agent_runtimes (
                runtime_id TEXT PRIMARY KEY,
                job_id TEXT NOT NULL,
                root_pid TEXT NOT NULL,
                state TEXT NOT NULL,
                data_json TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS events (
                job_id TEXT NOT NULL,
                event_id INTEGER NOT NULL,
                timestamp_ms TEXT NOT NULL,
                pid TEXT,
                event_type TEXT NOT NULL,
                data_json TEXT NOT NULL,
                PRIMARY KEY (job_id, event_id)
            );
            CREATE TABLE IF NOT EXISTS checkpoints (
                checkpoint_id TEXT PRIMARY KEY,
                job_id TEXT NOT NULL,
                pid TEXT NOT NULL,
                runtime_id TEXT,
                kind TEXT NOT NULL,
                state_ref TEXT NOT NULL,
                input_refs_json TEXT NOT NULL,
                output_refs_json TEXT NOT NULL,
                created_event_id INTEGER NOT NULL
            );
            "#,
        )
        .map_err(sql_err)?;
        Ok(())
    }

    pub fn append_event(
        &self,
        pid: Option<&str>,
        event_type: &str,
        data: Value,
    ) -> io::Result<ProcessEvent> {
        let conn = self.connect()?;
        let next_event_id: u64 = conn
            .query_row(
                "SELECT COALESCE(MAX(event_id), 0) + 1 FROM events WHERE job_id = ?1",
                params![self.job_id],
                |row| row.get::<_, i64>(0),
            )
            .map_err(sql_err)? as u64;
        let event = ProcessEvent {
            schema_version: PROCESS_TRUTH_EVENT_SCHEMA_VERSION.to_string(),
            event_id: next_event_id,
            timestamp_ms: now_ms(),
            job_id: self.job_id.clone(),
            pid: pid.map(str::to_string),
            event_type: event_type.to_string(),
            data,
        };
        conn.execute(
            "INSERT INTO events (job_id, event_id, timestamp_ms, pid, event_type, data_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                event.job_id,
                event.event_id as i64,
                event.timestamp_ms.to_string(),
                event.pid,
                event.event_type,
                serde_json::to_string(&event.data).map_err(json_err)?,
            ],
        )
        .map_err(sql_err)?;
        Ok(event)
    }

    pub fn read_events(&self) -> io::Result<Vec<ProcessEvent>> {
        let conn = self.connect()?;
        let mut stmt = conn
            .prepare(
                "SELECT event_id, timestamp_ms, pid, event_type, data_json FROM events WHERE job_id = ?1 ORDER BY event_id ASC",
            )
            .map_err(sql_err)?;
        let rows = stmt
            .query_map(params![self.job_id], |row| {
                let timestamp_raw: String = row.get(1)?;
                let data_raw: String = row.get(4)?;
                let timestamp_ms = timestamp_raw.parse::<u128>().unwrap_or(0);
                let data = serde_json::from_str::<Value>(&data_raw).unwrap_or(Value::Null);
                Ok(ProcessEvent {
                    schema_version: PROCESS_TRUTH_EVENT_SCHEMA_VERSION.to_string(),
                    event_id: row.get::<_, i64>(0)? as u64,
                    timestamp_ms,
                    job_id: self.job_id.clone(),
                    pid: row.get(2)?,
                    event_type: row.get(3)?,
                    data,
                })
            })
            .map_err(sql_err)?;
        let mut events = Vec::new();
        for row in rows {
            events.push(row.map_err(sql_err)?);
        }
        Ok(events)
    }

    pub fn stream_events(
        &self,
        after_event_id: u64,
        limit: usize,
    ) -> io::Result<Vec<ProcessEvent>> {
        let conn = self.connect()?;
        let mut stmt = conn
            .prepare(
                "SELECT event_id, timestamp_ms, pid, event_type, data_json FROM events WHERE job_id = ?1 AND event_id > ?2 ORDER BY event_id ASC LIMIT ?3",
            )
            .map_err(sql_err)?;
        let rows = stmt
            .query_map(
                params![self.job_id, after_event_id as i64, limit as i64],
                |row| {
                    let timestamp_raw: String = row.get(1)?;
                    let data_raw: String = row.get(4)?;
                    Ok(ProcessEvent {
                        schema_version: PROCESS_TRUTH_EVENT_SCHEMA_VERSION.to_string(),
                        event_id: row.get::<_, i64>(0)? as u64,
                        timestamp_ms: timestamp_raw.parse::<u128>().unwrap_or(0),
                        job_id: self.job_id.clone(),
                        pid: row.get(2)?,
                        event_type: row.get(3)?,
                        data: serde_json::from_str::<Value>(&data_raw).unwrap_or(Value::Null),
                    })
                },
            )
            .map_err(sql_err)?;
        let mut events = Vec::new();
        for row in rows {
            events.push(row.map_err(sql_err)?);
        }
        Ok(events)
    }

    pub fn export_jsonl(&self, output_path: impl AsRef<Path>) -> io::Result<PathBuf> {
        let output_path = output_path.as_ref();
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = File::create(output_path)?;
        for event in self.read_events()? {
            serde_json::to_writer(&mut file, &event).map_err(json_err)?;
            file.write_all(b"\n")?;
        }
        Ok(output_path.to_path_buf())
    }

    pub fn register_job(&self, job: &AgentJob) -> io::Result<()> {
        if !JOB_STATUSES.contains(&job.status.as_str()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("invalid job status: {}", job.status),
            ));
        }
        let conn = self.connect()?;
        conn.execute(
            "INSERT OR REPLACE INTO jobs (job_id, user_goal, workspace_root, status, root_pid, data_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                job.job_id,
                job.user_goal,
                job.workspace_root,
                job.status,
                job.root_pid,
                serde_json::to_string(job).map_err(json_err)?,
            ],
        )
        .map_err(sql_err)?;
        Ok(())
    }

    pub fn update_job_status(&self, status: &str) -> io::Result<()> {
        if !JOB_STATUSES.contains(&status) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("invalid job status: {status}"),
            ));
        }
        let conn = self.connect()?;
        conn.execute(
            "UPDATE jobs SET status = ?1, data_json = json_set(data_json, '$.status', ?1) WHERE job_id = ?2",
            params![status, self.job_id],
        )
        .map_err(sql_err)?;
        conn.execute(
            "UPDATE processes SET state = ?1, data_json = json_set(data_json, '$.state', ?1) WHERE job_id = ?2",
            params![status, self.job_id],
        )
        .map_err(sql_err)?;
        conn.execute(
            "UPDATE task_agent_runtimes SET state = ?1, data_json = json_set(data_json, '$.state', ?1) WHERE job_id = ?2",
            params![status, self.job_id],
        )
        .map_err(sql_err)?;
        self.append_event(None, "job_status_changed", json!({"status": status}))?;
        Ok(())
    }

    pub fn register_process(&self, process: &AgentProcess) -> io::Result<()> {
        let conn = self.connect()?;
        conn.execute(
            "INSERT OR REPLACE INTO processes (pid, ppid, job_id, process_type, state, data_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                process.pid,
                process.ppid,
                process.job_id,
                process.process_type,
                process.state,
                serde_json::to_string(process).map_err(json_err)?,
            ],
        )
        .map_err(sql_err)?;
        Ok(())
    }

    pub fn register_task_agent_runtime(&self, runtime: &TaskAgentRuntimeRecord) -> io::Result<()> {
        let conn = self.connect()?;
        conn.execute(
            "INSERT OR REPLACE INTO task_agent_runtimes (runtime_id, job_id, root_pid, state, data_json) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                runtime.runtime_id,
                runtime.job_id,
                runtime.root_pid,
                runtime.state,
                serde_json::to_string(runtime).map_err(json_err)?,
            ],
        )
        .map_err(sql_err)?;
        Ok(())
    }

    pub fn registry_snapshot(&self) -> io::Result<ProcessRegistrySnapshot> {
        let conn = self.connect()?;
        let jobs = query_json_rows::<AgentJob>(
            &conn,
            "SELECT data_json FROM jobs WHERE job_id = ?1",
            &self.job_id,
        )?;
        let processes = query_json_rows::<AgentProcess>(
            &conn,
            "SELECT data_json FROM processes WHERE job_id = ?1 ORDER BY pid ASC",
            &self.job_id,
        )?;
        let runtimes = query_json_rows::<TaskAgentRuntimeRecord>(
            &conn,
            "SELECT data_json FROM task_agent_runtimes WHERE job_id = ?1 ORDER BY runtime_id ASC",
            &self.job_id,
        )?;
        Ok(ProcessRegistrySnapshot {
            jobs,
            processes,
            task_agent_runtimes: runtimes,
        })
    }

    pub fn save_checkpoint(
        &self,
        pid: &str,
        runtime_id: Option<&str>,
        kind: &str,
        state: &Value,
        input_refs: Vec<String>,
        output_refs: Vec<String>,
    ) -> io::Result<CheckpointRef> {
        let checkpoint_id = format!("ckpt_{}_{}", self.job_id, now_ms());
        let state_blob = serde_json::to_vec(state).map_err(json_err)?;
        let state_ref =
            self.write_blob(&format!("checkpoints/{checkpoint_id}.json"), &state_blob)?;
        let event = self.append_event(
            Some(pid),
            "checkpoint_saved",
            json!({
                "checkpoint_id": checkpoint_id,
                "runtime_id": runtime_id,
                "kind": kind,
                "state_ref": state_ref,
                "input_refs": input_refs,
                "output_refs": output_refs,
            }),
        )?;
        let record = CheckpointRecord {
            checkpoint_id: checkpoint_id.clone(),
            job_id: self.job_id.clone(),
            pid: pid.to_string(),
            runtime_id: runtime_id.map(str::to_string),
            kind: kind.to_string(),
            state_ref: state_ref.clone(),
            input_refs,
            output_refs,
            created_event_id: event.event_id,
        };
        let conn = self.connect()?;
        conn.execute(
            "INSERT INTO checkpoints (checkpoint_id, job_id, pid, runtime_id, kind, state_ref, input_refs_json, output_refs_json, created_event_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                record.checkpoint_id,
                record.job_id,
                record.pid,
                record.runtime_id,
                record.kind,
                record.state_ref,
                serde_json::to_string(&record.input_refs).map_err(json_err)?,
                serde_json::to_string(&record.output_refs).map_err(json_err)?,
                record.created_event_id as i64,
            ],
        )
        .map_err(sql_err)?;
        Ok(CheckpointRef {
            checkpoint_id,
            job_id: self.job_id.clone(),
            pid: pid.to_string(),
            runtime_id: runtime_id.map(str::to_string),
            kind: kind.to_string(),
            state_ref,
        })
    }

    pub fn list_checkpoints(&self) -> io::Result<Vec<CheckpointRecord>> {
        let conn = self.connect()?;
        let mut stmt = conn
            .prepare(
                "SELECT checkpoint_id, job_id, pid, runtime_id, kind, state_ref, input_refs_json, output_refs_json, created_event_id FROM checkpoints WHERE job_id = ?1 ORDER BY created_event_id ASC",
            )
            .map_err(sql_err)?;
        let rows = stmt
            .query_map(params![self.job_id], |row| {
                let input_refs_raw: String = row.get(6)?;
                let output_refs_raw: String = row.get(7)?;
                Ok(CheckpointRecord {
                    checkpoint_id: row.get(0)?,
                    job_id: row.get(1)?,
                    pid: row.get(2)?,
                    runtime_id: row.get(3)?,
                    kind: row.get(4)?,
                    state_ref: row.get(5)?,
                    input_refs: serde_json::from_str(&input_refs_raw).unwrap_or_default(),
                    output_refs: serde_json::from_str(&output_refs_raw).unwrap_or_default(),
                    created_event_id: row.get::<_, i64>(8)? as u64,
                })
            })
            .map_err(sql_err)?;
        let mut checkpoints = Vec::new();
        for row in rows {
            checkpoints.push(row.map_err(sql_err)?);
        }
        Ok(checkpoints)
    }

    pub fn replay(&self) -> io::Result<ReplayState> {
        let events = self.read_events()?;
        let mut status = "created".to_string();
        let mut artifact_refs: Vec<String> = Vec::new();
        let mut artifact_provenance: Vec<ArtifactProvenance> = Vec::new();
        for event in &events {
            match event.event_type.as_str() {
                "job_completed" => status = "completed".to_string(),
                "job_blocked" => status = "blocked".to_string(),
                "job_failed" => status = "failed".to_string(),
                "job_interrupted_by_model_protocol_error" => status = "interrupted".to_string(),
                "job_status_changed" => {
                    if let Some(value) = event.data.get("status").and_then(Value::as_str) {
                        status = value.to_string();
                    }
                }
                _ => {}
            }
            for payload in event_payloads(event) {
                if let Some(items) = payload.get("artifacts").and_then(Value::as_array) {
                    for item in items {
                        if let Some(path) = item.as_str() {
                            push_artifact_ref(&mut artifact_refs, path);
                        }
                    }
                }
                if let Some(path) = payload.get("artifact_path").and_then(Value::as_str) {
                    push_artifact_ref(&mut artifact_refs, path);
                }
                if let Some(path) = payload.get("archive_path").and_then(Value::as_str) {
                    push_artifact_ref(&mut artifact_refs, path);
                }
                if let Some(provenance) = artifact_provenance_from_event(event, payload) {
                    push_artifact_ref(&mut artifact_refs, &provenance.artifact_path);
                    artifact_provenance.push(provenance);
                }
            }
        }
        artifact_refs.sort();
        artifact_refs.dedup();
        artifact_provenance.sort_by(|left, right| {
            left.artifact_path
                .cmp(&right.artifact_path)
                .then(left.event_id.cmp(&right.event_id))
        });
        Ok(ReplayState {
            job_id: self.job_id.clone(),
            status,
            event_count: events.len(),
            artifact_refs,
            artifact_provenance,
        })
    }

    pub fn replay_artifact_provenance(&self) -> io::Result<Vec<ArtifactProvenance>> {
        Ok(self.replay()?.artifact_provenance)
    }

    pub fn replay_session(&self) -> io::Result<ReplaySession> {
        let events = self.read_events()?;
        let state = self.replay()?;
        Ok(ReplaySession {
            replay_id: format!("replay_{}", now_ms()),
            job_id: self.job_id.clone(),
            state,
            events,
        })
    }

    pub fn write_blob(&self, name: &str, content: &[u8]) -> io::Result<String> {
        let blob_dir = self.state_root.join("blobs").join(&self.job_id);
        fs::create_dir_all(&blob_dir)?;
        let path = blob_dir.join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, content)?;
        Ok(format!(
            "blob://{}/{}",
            self.job_id,
            name.replace('\\', "/")
        ))
    }

    pub fn resolve_blob_ref(&self, blob_ref: &str) -> io::Result<PathBuf> {
        let prefix = format!("blob://{}/", self.job_id);
        let relative = blob_ref.strip_prefix(&prefix).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "blob ref does not belong to this job",
            )
        })?;
        if relative.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "blob ref has no relative path",
            ));
        }

        let blob_root = self.state_root.join("blobs").join(&self.job_id);
        let mut resolved = blob_root.clone();
        for component in Path::new(relative).components() {
            match component {
                Component::Normal(part) => resolved.push(part),
                Component::CurDir => {}
                Component::ParentDir | Component::Prefix(_) | Component::RootDir => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "blob ref leaves blob store boundary",
                    ));
                }
            }
        }
        if !resolved.starts_with(&blob_root) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "blob ref leaves blob store boundary",
            ));
        }
        Ok(resolved)
    }
}

pub fn default_capability_registry() -> Vec<CapabilityDescriptor> {
    vec![
        descriptor(
            "os.list_tree",
            "workspace root",
            "SourceSetRef",
            &["fs:read"],
            &["read"],
            "source_set_verifier",
            "none",
        ),
        descriptor(
            "os.workspace_inventory",
            "workspace root + max_depth",
            "WorkspaceInventoryRefs",
            &["fs:read"],
            &["read"],
            "workspace_inventory_verifier",
            "none",
        ),
        descriptor(
            "source_set.create",
            "root_path + include/exclude filters",
            "SourceSetRef",
            &["fs:read"],
            &["read"],
            "source_set_verifier",
            "none",
        ),
        descriptor(
            "source_set.read_page",
            "SourceSetRef + offset + limit",
            "SourceSetPageRef",
            &["fs:read"],
            &["read"],
            "source_set_page_verifier",
            "none",
        ),
        descriptor(
            "dataset.read_page",
            "DataSetRef + offset + limit",
            "DataSetPageRef",
            &["fs:read"],
            &["read"],
            "dataset_page_verifier",
            "none",
        ),
        descriptor(
            "data.csv.read_dataset",
            "CsvPath + has_header + max_rows",
            "DataSetRef",
            &["fs:read"],
            &["read"],
            "csv_dataset_verifier",
            "none",
        ),
        descriptor(
            "source_set.coverage_verify",
            "SourceSetRef",
            "SourceSetCoverageReceipt",
            &["fs:read"],
            &["read"],
            "source_set_coverage_verifier",
            "none",
        ),
        descriptor(
            "workspace.batch_hash",
            "SourceSetRef",
            "DataSetRef",
            &["fs:read"],
            &["read"],
            "dataset_ref_verifier",
            "none",
        ),
        descriptor(
            "workspace.find_duplicates",
            "SourceSetRef",
            "DuplicateDataSetRef",
            &["fs:read"],
            &["read"],
            "duplicate_dataset_verifier",
            "none",
        ),
        descriptor(
            "workspace.recent_changes",
            "SourceSetRef + days",
            "RecentChangesDataSetRef",
            &["fs:read"],
            &["read"],
            "recent_changes_dataset_verifier",
            "none",
        ),
        descriptor(
            "workspace.plan_organize",
            "SourceSetRef + destination_root",
            "WorkspaceOrganizePlanRef",
            &["fs:read"],
            &["read"],
            "organize_plan_verifier",
            "none",
        ),
        descriptor(
            "workspace.apply_organize_tx",
            "WorkspaceOrganizePlanRef + approval_id",
            "WorkspaceMutationTxReceipt",
            &["fs:read", "fs:write"],
            &["write"],
            "workspace_mutation_tx_verifier",
            "tx_rollback",
        ),
        descriptor(
            "workspace.rename_batch_preview",
            "RenameMappings",
            "WorkspaceRenamePreviewRef",
            &["fs:read"],
            &["read"],
            "rename_preview_verifier",
            "none",
        ),
        descriptor(
            "workspace.rename_batch_apply",
            "RenameMappings + approval_id",
            "WorkspaceMutationTxReceipt",
            &["fs:read", "fs:write"],
            &["write"],
            "workspace_mutation_tx_verifier",
            "tx_rollback",
        ),
        descriptor(
            "workspace.tree_index",
            "SourceSetRef + optional tree_path",
            "TreeIndexArtifactRef",
            &["fs:read", "fs:write"],
            &["write"],
            "tree_index_verifier",
            "delete_artifact",
        ),
        descriptor(
            "workspace.perf_inventory",
            "SourceSetRef + optional output_path",
            "PerfNotesArtifactRef",
            &["fs:read", "fs:write"],
            &["write"],
            "perf_inventory_verifier",
            "delete_artifact",
        ),
        descriptor(
            "workspace.recent_changes_snapshot",
            "SourceSetRef + days",
            "RecentChangesDataSetRef",
            &["fs:read"],
            &["read"],
            "recent_changes_dataset_verifier",
            "none",
        ),
        descriptor(
            "dataset.export_csv",
            "DataSetRef + ArtifactPath",
            "ArtifactRef",
            &["fs:read", "fs:write"],
            &["write"],
            "artifact_verifier",
            "delete_artifact",
        ),
        descriptor(
            "dataset.export_markdown",
            "DataSetRef + ArtifactPath",
            "ArtifactRef",
            &["fs:read", "fs:write"],
            &["write"],
            "artifact_verifier",
            "delete_artifact",
        ),
        descriptor(
            "dataset.coverage_verify",
            "DataSetRef",
            "DataSetCoverageReceipt",
            &["fs:read"],
            &["read"],
            "dataset_coverage_verifier",
            "none",
        ),
        descriptor(
            "artifact.inspect",
            "ArtifactPath",
            "ArtifactInspectionReceipt",
            &["fs:read"],
            &["read"],
            "artifact_inspection_verifier",
            "none",
        ),
        descriptor(
            "artifact.audit_readonly",
            "ArtifactPath",
            "ArtifactReadonlyAuditReceipt",
            &["fs:read"],
            &["read"],
            "artifact_readonly_audit_verifier",
            "none",
        ),
        descriptor(
            "client_env.scan_overview",
            "ClientEnvScanOptions",
            "ClientEnvSnapshotRef",
            &["client_env:read"],
            &["read"],
            "client_env_snapshot_verifier",
            "none",
        ),
        descriptor(
            "client_env.scan_device",
            "ClientEnvScanOptions",
            "ClientEnvSnapshotRef",
            &["client_env:read"],
            &["read"],
            "client_env_snapshot_verifier",
            "none",
        ),
        descriptor(
            "client_env.scan_storage",
            "ClientEnvScanOptions",
            "ClientEnvSnapshotRef",
            &["client_env:read"],
            &["read"],
            "client_env_snapshot_verifier",
            "none",
        ),
        descriptor(
            "client_env.scan_network",
            "ClientEnvScanOptions",
            "ClientEnvSnapshotRef",
            &["client_env:read"],
            &["read"],
            "client_env_snapshot_verifier",
            "none",
        ),
        descriptor(
            "client_env.scan_runtimes",
            "ClientEnvScanOptions",
            "ClientEnvSnapshotRef",
            &["client_env:read"],
            &["read"],
            "client_env_snapshot_verifier",
            "none",
        ),
        descriptor(
            "client_env.read_snapshot",
            "ClientEnvSnapshotRef + offset + limit",
            "ClientEnvSnapshotPageRef",
            &["client_env:read"],
            &["read"],
            "client_env_snapshot_verifier",
            "none",
        ),
        descriptor(
            "client_env.request_sensitive_disclosure",
            "requested_fields + reason",
            "ClientEnvDisclosureRequestReceipt",
            &["client_env:read"],
            &["read"],
            "client_env_disclosure_verifier",
            "none",
        ),
        descriptor(
            "artifact.copy_source_set",
            "SourceSetRef + DestinationDir",
            "ArtifactTreeRef",
            &["fs:read", "fs:write"],
            &["write"],
            "artifact_tree_verifier",
            "delete_artifact",
        ),
        descriptor(
            "os.write_artifact",
            "artifact ref + bytes",
            "ArtifactRef",
            &["fs:write"],
            &["write"],
            "artifact_verifier",
            "delete_artifact",
        ),
        descriptor(
            "os.write_temp_dataset",
            "WorkspaceTempPath + bytes",
            "TempDataSetArtifactRef",
            &["fs:write"],
            &["write"],
            "temp_dataset_verifier",
            "delete_artifact",
        ),
        descriptor(
            "os.stat_path",
            "WorkspacePath",
            "PathStatReceipt",
            &["fs:read"],
            &["read"],
            "path_stat_verifier",
            "none",
        ),
        descriptor(
            "os.read_file",
            "WorkspaceFilePath",
            "DataSetRef",
            &["fs:read"],
            &["read"],
            "dataset_ref_verifier",
            "none",
        ),
        descriptor(
            "os.write_file",
            "WorkspaceFilePath + bytes + write_kind",
            "MutationReceipt",
            &["fs:write"],
            &["write"],
            "mutation_receipt_verifier",
            "tx_rollback",
        ),
        descriptor(
            "os.write_source_mutation_preview",
            "WorkspaceFilePath + bytes",
            "PreviewOnlyReceipt",
            &["fs:write"],
            &["preview"],
            "preview_tx_verifier",
            "none",
        ),
        descriptor(
            "os.write_source_mutation_apply",
            "WorkspaceFilePath + bytes + approval_id",
            "MutationReceipt",
            &["fs:write"],
            &["write"],
            "mutation_receipt_verifier",
            "tx_rollback",
        ),
        descriptor(
            "os.copy_path",
            "SourcePath + DestinationPath",
            "MutationReceipt",
            &["fs:read", "fs:write"],
            &["write"],
            "mutation_receipt_verifier",
            "tx_rollback",
        ),
        descriptor(
            "os.move_path",
            "SourcePath + DestinationPath",
            "MutationReceipt",
            &["fs:read", "fs:write"],
            &["write"],
            "mutation_receipt_verifier",
            "tx_rollback",
        ),
        descriptor(
            "os.rename_path",
            "SourcePath + DestinationPath",
            "MutationReceipt",
            &["fs:read", "fs:write"],
            &["write"],
            "mutation_receipt_verifier",
            "tx_rollback",
        ),
        descriptor(
            "os.delete_path",
            "WorkspacePath",
            "MutationReceipt",
            &["fs:write"],
            &["write"],
            "mutation_receipt_verifier",
            "tx_rollback",
        ),
        descriptor(
            "os.hash_path",
            "WorkspacePath",
            "HashReceipt",
            &["fs:read"],
            &["read"],
            "hash_receipt_verifier",
            "none",
        ),
        descriptor(
            "os.zip",
            "WorkspacePaths + DestinationZipPath",
            "ArchiveReceipt",
            &["fs:read", "fs:write"],
            &["write"],
            "archive_receipt_verifier",
            "tx_rollback",
        ),
        descriptor(
            "os.unzip",
            "ArchivePath + DestinationDir",
            "ArchiveReceipt",
            &["fs:read", "fs:write"],
            &["write"],
            "archive_receipt_verifier",
            "tx_rollback",
        ),
        descriptor(
            "os.diff",
            "LeftPath + RightPath",
            "DiffRef",
            &["fs:read"],
            &["read"],
            "diff_receipt_verifier",
            "none",
        ),
        descriptor(
            "os.rollback_tx",
            "TxId",
            "RollbackReceipt",
            &["fs:write"],
            &["write"],
            "rollback_receipt_verifier",
            "none",
        ),
        descriptor(
            "os.verify_artifact",
            "ArtifactRef",
            "VerifyReceipt",
            &["fs:read"],
            &["read"],
            "verify_receipt_verifier",
            "none",
        ),
        descriptor(
            "process.read_ref",
            "TypedRef",
            "RefReadReceipt",
            &["process:control"],
            &["read_process_truth"],
            "ref_read_receipt_verifier",
            "none",
        ),
        descriptor(
            "tool.result.page",
            "RawResultRef + offset + limit_bytes",
            "RawResultPageReceipt",
            &["process:control"],
            &["read_process_truth"],
            "raw_result_page_verifier",
            "none",
        ),
        descriptor(
            "tool.result.search",
            "RawResultRef + query + max_matches",
            "RawResultSearchReceipt",
            &["process:control"],
            &["read_process_truth"],
            "raw_result_search_verifier",
            "none",
        ),
        descriptor(
            "tool.result.inspect_schema",
            "RawResultRef",
            "RawResultSchemaReceipt",
            &["process:control"],
            &["read_process_truth"],
            "raw_result_schema_verifier",
            "none",
        ),
        descriptor(
            "process.query_events",
            "EventQuery",
            "ProcessTruthEventsReceipt",
            &["process:control"],
            &["read_process_truth"],
            "process_truth_query_verifier",
            "none",
        ),
        descriptor(
            "process.toolset.select",
            "ToolsetGroupSelection",
            "ProviderToolsetSelectionReceipt",
            &["process:control"],
            &["read_process_truth"],
            "provider_toolset_selection_verifier",
            "none",
        ),
        descriptor(
            "process.fork_child",
            "ChildProcessSpec + typed refs",
            "ChildProcessReceipt",
            &["process:control"],
            &["process"],
            "child_process_receipt_verifier",
            "none",
        ),
        descriptor(
            "process.request_preview",
            "PreviewSpec + executable_operations[] + artifact refs",
            "PreviewReceipt",
            &["process:control"],
            &["append_process_truth"],
            "preview_receipt_verifier",
            "none",
        ),
        descriptor(
            "process.preview.create",
            "PreviewTxSpec + executable_operations[]",
            "PreviewTxReceipt",
            &["process:control"],
            &["append_process_truth"],
            "preview_tx_verifier",
            "none",
        ),
        descriptor(
            "process.approval.record",
            "PreviewTxRef + approval note",
            "ApprovalToken",
            &["process:control"],
            &["append_process_truth"],
            "approval_token_verifier",
            "none",
        ),
        descriptor(
            "process.pending_approvals",
            "ProcessTruthQuery",
            "PendingApprovalsReceipt",
            &["process:control"],
            &["read_process_truth"],
            "pending_approval_verifier",
            "none",
        ),
        descriptor(
            "chat.answer",
            "AssistantContent + cited refs",
            "ChatAnswerControlReceipt",
            &["chat:control"],
            &["append_chat_truth"],
            "chat_answer_control_verifier",
            "none",
        ),
        descriptor(
            "chat.clarify",
            "ClarificationQuestion + missing fact",
            "ChatClarificationControlReceipt",
            &["chat:control"],
            &["append_chat_truth"],
            "chat_clarification_control_verifier",
            "none",
        ),
        descriptor(
            "chat.needs_task",
            "SuggestedTaskGoal + context pack id",
            "ChatNeedsTaskControlReceipt",
            &["chat:control"],
            &["append_chat_truth"],
            "chat_needs_task_control_verifier",
            "none",
        ),
        descriptor(
            "process.clarify",
            "ClarificationRequest",
            "ClarificationReceipt",
            &["process:control"],
            &["append_process_truth"],
            "clarification_receipt_verifier",
            "none",
        ),
        descriptor(
            "process.complete",
            "CompletionStatement + claimed_artifacts + hard-boundary facts",
            "CompletionReceipt",
            &["process:control"],
            &["append_process_truth"],
            "completion_receipt_verifier",
            "none",
        ),
        descriptor(
            "process.fail",
            "FailureEvidenceRefs",
            "FailureReceipt",
            &["process:control"],
            &["append_process_truth"],
            "failure_receipt_verifier",
            "none",
        ),
        descriptor(
            "terminal.run_command",
            "argv + explicit timeout_ms",
            "CommandReceipt",
            &["terminal:execute"],
            &["process"],
            "command_receipt_verifier",
            "policy_tx",
        ),
        descriptor(
            "terminal.start_service",
            "service_id + argv + startup_timeout_ms + optional health_check",
            "ServiceReceipt",
            &["terminal:execute"],
            &["process"],
            "service_receipt_verifier",
            "service_stop",
        ),
        descriptor(
            "terminal.stop_service",
            "service_id + reason",
            "ServiceReceipt",
            &["terminal:execute"],
            &["process"],
            "service_receipt_verifier",
            "none",
        ),
        descriptor(
            "terminal.service_status",
            "service_id",
            "ServiceReceipt",
            &["terminal:execute"],
            &["read"],
            "service_receipt_verifier",
            "none",
        ),
        descriptor(
            "model.invoke",
            "ModelAction + refs",
            "ModelCallReceipt",
            &["model:invoke"],
            &["network"],
            "model_ledger_verifier",
            "none",
        ),
        descriptor(
            "model.decide_next_action",
            "ModelAction + observation refs",
            "ModelCallReceipt",
            &["model:invoke"],
            &["network"],
            "model_ledger_verifier",
            "none",
        ),
        descriptor(
            "model.chat_turn",
            "ChatTurnAction + chat/context refs",
            "ModelCallReceipt",
            &["model:invoke"],
            &["network"],
            "model_chat_turn_verifier",
            "none",
        ),
        descriptor(
            "model.compact_container_context",
            "ContainerContextCompactionInput",
            "ContainerContextCompactionSummary",
            &["model:invoke"],
            &["network"],
            "container_context_compaction_verifier",
            "none",
        ),
        descriptor(
            "model.compact_chat_context",
            "ChatContextCompactionInput",
            "ChatContextCompactionSummary",
            &["model:invoke"],
            &["network"],
            "chat_context_compaction_verifier",
            "none",
        ),
        descriptor(
            "model.compact_task_context",
            "TaskContextCompactionInput",
            "TaskContextCompactionSummary",
            &["model:invoke"],
            &["network"],
            "task_context_compaction_verifier",
            "none",
        ),
        descriptor(
            "model.extract_json",
            "ModelAction + typed refs + output schema",
            "ModelCallReceipt",
            &["model:invoke"],
            &["network"],
            "model_schema_verifier",
            "none",
        ),
        descriptor(
            "model.extract_dataset",
            "ModelAction + RawDocumentSet/DataSet refs + output schema",
            "CandidateDataSetRef",
            &["model:invoke"],
            &["network"],
            "model_dataset_schema_verifier",
            "none",
        ),
        descriptor(
            "model.summarize_dataset",
            "ModelAction + DataSetRef",
            "ModelCallReceipt",
            &["model:invoke"],
            &["network"],
            "model_source_grounding_verifier",
            "none",
        ),
        descriptor(
            "model.synthesize_artifact_from_dataset",
            "ModelAction + DataSetRef + artifact instruction",
            "ModelCallReceipt",
            &["model:invoke"],
            &["network"],
            "model_artifact_generation_verifier",
            "none",
        ),
        descriptor(
            "model.audit_artifact",
            "model_audit_agent ArtifactAuditAction + artifact path + source/coverage refs",
            "ArtifactModelAuditReceipt",
            &["model:invoke"],
            &["network", "append_process_truth"],
            "model_artifact_audit_schema_verifier",
            "none",
        ),
        descriptor(
            "model.audit_artifact_quality",
            "model_audit_agent ArtifactAuditAction + artifact path + source/coverage refs",
            "ArtifactModelAuditReceipt",
            &["model:invoke"],
            &["network", "append_process_truth"],
            "model_artifact_audit_schema_verifier",
            "none",
        ),
        descriptor(
            "model.summarize",
            "ModelAction + source refs",
            "ModelCallReceipt",
            &["model:invoke"],
            &["network"],
            "model_source_grounding_verifier",
            "none",
        ),
        descriptor(
            "model.rewrite",
            "ModelAction + source refs + rewrite constraints",
            "ModelCallReceipt",
            &["model:invoke"],
            &["network"],
            "model_rewrite_verifier",
            "none",
        ),
        descriptor(
            "model.generate_artifact",
            "ModelAction + source refs + artifact schema",
            "ModelCallReceipt",
            &["model:invoke"],
            &["network"],
            "model_artifact_generation_verifier",
            "none",
        ),
        descriptor(
            "model.audit",
            "ModelAction + artifact refs + audit schema",
            "ModelCallReceipt",
            &["model:invoke"],
            &["network"],
            "model_audit_verifier",
            "none",
        ),
        descriptor(
            "model.render_entity_reply",
            "EntityRenderAction + context refs",
            "ModelCallReceipt",
            &["model:invoke"],
            &["network"],
            "model_entity_reply_verifier",
            "none",
        ),
        descriptor(
            "office.docx.read_text",
            "DocxRef",
            "DocxExtractionReceipt",
            &["office:read"],
            &["read"],
            "openxml_docx_verifier",
            "none",
        ),
        descriptor(
            "office.inspect_workbook",
            "WorkbookPath",
            "WorkbookInspectionReceipt",
            &["office:read"],
            &["read"],
            "workbook_inspection_verifier",
            "none",
        ),
        descriptor(
            "office.workbook.read_cells",
            "WorkbookPath + sheet + max_rows",
            "WorkbookCellsReceipt",
            &["office:read"],
            &["read"],
            "openxml_workbook_verifier",
            "none",
        ),
        descriptor(
            "office.workbook.read_text",
            "WorkbookPath + sheet + max_rows",
            "WorkbookTextReceipt",
            &["office:read"],
            &["read"],
            "openxml_workbook_verifier",
            "none",
        ),
        descriptor(
            "document.pdf.extract_text",
            "PdfPath",
            "PdfTextExtractionReceipt",
            &["fs:read"],
            &["read"],
            "pdf_text_layer_verifier",
            "none",
        ),
        descriptor(
            "office.docx.batch_read_text",
            "SourceSetRef",
            "RawDocumentSetRef",
            &["office:read"],
            &["read"],
            "openxml_batch_docx_verifier",
            "none",
        ),
        descriptor(
            "office.docx.batch_extract_metadata",
            "SourceSetRef",
            "DocxMetadataDataSetRef",
            &["office:read"],
            &["read"],
            "openxml_batch_metadata_verifier",
            "none",
        ),
        descriptor(
            "office.docx.batch_validate",
            "SourceSetRef",
            "OpenXmlBatchValidationReceipt",
            &["office:read"],
            &["read"],
            "openxml_batch_validation_verifier",
            "none",
        ),
        descriptor(
            "office.docx.create",
            "DocxArtifactSpec + TextRef",
            "OfficeArtifactReceipt",
            &["office:write"],
            &["write"],
            "openxml_validation_verifier",
            "tx_rollback",
        ),
        descriptor(
            "office.docx.rewrite_save_as",
            "DocxRef + OutputPath + RewrittenTextRef",
            "OfficeArtifactReceipt",
            &["office:write"],
            &["write"],
            "openxml_validation_verifier",
            "tx_rollback",
        ),
        descriptor(
            "office.docx.rewrite_preview",
            "DocxRef + RewrittenTextRef",
            "OfficePreviewReceipt",
            &["office:read"],
            &["read"],
            "office_preview_verifier",
            "none",
        ),
        descriptor(
            "office.docx.rewrite_in_place_preview",
            "DocxRef + RewrittenTextRef",
            "OfficePreviewReceipt",
            &["office:read"],
            &["read"],
            "office_preview_verifier",
            "none",
        ),
        descriptor(
            "office.docx.rewrite_in_place",
            "DocxRef + RewrittenTextRef + ApprovalToken",
            "OfficeMutationReceipt",
            &["office:write"],
            &["write"],
            "openxml_validation_verifier",
            "tx_rollback",
        ),
        descriptor(
            "office.docx.diff_summary",
            "BeforeDocxRef + AfterDocxRef",
            "OfficeDiffReceipt",
            &["office:read"],
            &["read"],
            "office_diff_verifier",
            "none",
        ),
        descriptor(
            "office.docx.validate",
            "DocxRef",
            "OpenXmlValidationReceipt",
            &["office:read"],
            &["read"],
            "openxml_validation_verifier",
            "none",
        ),
        descriptor(
            "package.build_zip",
            "SourceSetRef + package output paths + exclude rules",
            "PackageReceipt",
            &["fs:read", "fs:write"],
            &["write"],
            "package_receipt_verifier",
            "delete_artifact",
        ),
        descriptor(
            "artifact.verify_coverage",
            "ArtifactPath + SourceSetRef/DataSetRef",
            "ArtifactCoverageReceipt",
            &["fs:read"],
            &["read"],
            "artifact_coverage_verifier",
            "none",
        ),
        descriptor(
            "artifact.source_coverage_verify",
            "ArtifactPath + SourceSetRef/DataSetRef",
            "ArtifactCoverageReceipt",
            &["fs:read"],
            &["read"],
            "artifact_coverage_verifier",
            "none",
        ),
        descriptor(
            "artifact.verify_typed",
            "ArtifactPath",
            "TypedArtifactVerifierReceipt",
            &["fs:read"],
            &["read"],
            "typed_artifact_verifier",
            "none",
        ),
        descriptor(
            "artifact.audit_quality",
            "ArtifactPath + quality policy",
            "ArtifactQualityReceipt",
            &["fs:read"],
            &["read"],
            "artifact_quality_verifier",
            "none",
        ),
    ]
}

pub fn default_verifier_registry() -> Vec<VerifierSpec> {
    default_capability_registry()
        .into_iter()
        .map(|descriptor| VerifierSpec {
            verifier_id: descriptor.verifier,
            target_id: descriptor.capability_id,
            input_refs: vec![
                "capability_receipt_ref".to_string(),
                "process_truth_event_ref".to_string(),
            ],
            checks: vec![
                "schema_valid".to_string(),
                "workspace_boundary_valid".to_string(),
                "no_silent_fallback".to_string(),
            ],
            failure_policy: "fail_closed".to_string(),
        })
        .collect()
}

fn descriptor(
    capability_id: &str,
    input_schema: &str,
    output_schema: &str,
    permissions: &[&str],
    side_effects: &[&str],
    verifier: &str,
    rollback: &str,
) -> CapabilityDescriptor {
    CapabilityDescriptor {
        capability_id: capability_id.to_string(),
        input_schema: input_schema.to_string(),
        output_schema: output_schema.to_string(),
        preconditions: vec![
            "valid_capability_token".to_string(),
            "workspace_boundary_checked".to_string(),
        ],
        side_effects: side_effects.iter().map(|item| item.to_string()).collect(),
        required_permissions: permissions.iter().map(|item| item.to_string()).collect(),
        receipt_schema: format!("{}.receipt.v2", capability_id),
        verifier: verifier.to_string(),
        rollback: rollback.to_string(),
        derivation_type: derivation_type_for_capability(capability_id).to_string(),
        is_lossless: is_lossless_capability(capability_id),
        source_refs_required: source_refs_required_for_capability(capability_id),
        coverage_required: coverage_required_for_capability(capability_id),
        verification_required: true,
        fallback_allowed: false,
        drilldown_supported: drilldown_supported_for_capability(capability_id),
        approval_policy: descriptor_approval_policy(capability_id, side_effects),
        target_path_schema: target_path_schema_for_capability(capability_id).to_string(),
        artifact_role: artifact_role_for_capability(capability_id).to_string(),
        rollback_policy: rollback.to_string(),
    }
}

fn target_path_schema_for_capability(capability_id: &str) -> &'static str {
    match capability_id {
        "os.copy_path" | "os.move_path" | "os.rename_path" => "source_path + destination_path",
        "os.delete_path" => "path",
        "os.write_file"
        | "os.write_artifact"
        | "os.write_temp_dataset"
        | "os.write_source_mutation_preview"
        | "os.write_source_mutation_apply" => "path",
        "os.zip" => "destination_zip_path",
        "os.unzip" => "archive_path + destination_dir",
        "workspace.plan_organize" => "organize_plan_ref target_paths",
        "workspace.rename_batch_preview" | "workspace.rename_batch_apply" => {
            "rename_plan_ref target_paths"
        }
        "workspace.apply_organize_tx" => "organize_plan_ref target_paths",
        "workspace.tree_index" => "tree_path",
        "workspace.perf_inventory" => "output_path",
        "dataset.export_csv" | "dataset.export_markdown" => "output_path",
        "artifact.copy_source_set" => "destination_dir",
        "office.docx.create" | "office.docx.rewrite_save_as" => "output_path",
        "office.docx.rewrite_in_place_preview" | "office.docx.rewrite_in_place" => "input_path",
        "package.build_zip" => {
            "destination_zip_path + manifest_path + checksums_path + perf_notes_path"
        }
        "terminal.run_command" => "explicit target_paths required for mutation commands",
        "process.preview.create" | "process.request_preview" => {
            "operations[].capability_id + operations[].target_paths + human_description"
        }
        _ => "none",
    }
}

fn artifact_role_for_capability(capability_id: &str) -> &'static str {
    match capability_id {
        "workspace.perf_inventory" => "supporting_artifact",
        "package.build_zip" => "compound_artifact",
        "os.write_artifact"
        | "dataset.export_csv"
        | "dataset.export_markdown"
        | "artifact.copy_source_set"
        | "office.docx.create"
        | "office.docx.rewrite_save_as"
        | "workspace.tree_index" => "required_user_artifact",
        "os.write_temp_dataset" => "temporary_artifact",
        "os.write_source_mutation_apply" => "source_mutation",
        "source_set.coverage_verify"
        | "dataset.coverage_verify"
        | "artifact.verify_coverage"
        | "artifact.source_coverage_verify"
        | "artifact.verify_typed"
        | "artifact.audit_quality"
        | "model.audit_artifact" => "audit_artifact",
        _ => "none",
    }
}

fn derivation_type_for_capability(capability_id: &str) -> &'static str {
    if capability_id.starts_with("model.synthesize")
        || capability_id == "model.generate_artifact"
        || capability_id == "model.summarize"
        || capability_id == "model.summarize_dataset"
        || capability_id == "model.rewrite"
    {
        "abstractive"
    } else if capability_id.starts_with("model.extract")
        || capability_id == "office.docx.batch_extract_metadata"
    {
        "extractive"
    } else if capability_id.contains("hash")
        || capability_id.contains("duplicates")
        || capability_id.contains("recent_changes")
        || capability_id.contains("inventory")
        || capability_id.starts_with("source_set.")
    {
        "metadata"
    } else {
        "raw"
    }
}

fn is_lossless_capability(capability_id: &str) -> bool {
    matches!(
        capability_id,
        "os.read_file"
            | "os.write_artifact"
            | "os.write_temp_dataset"
            | "os.write_source_mutation_apply"
            | "os.copy_path"
            | "os.move_path"
            | "os.rename_path"
            | "office.docx.read_text"
            | "office.workbook.read_cells"
            | "office.workbook.read_text"
            | "data.csv.read_dataset"
            | "document.pdf.extract_text"
            | "office.docx.batch_read_text"
    )
}

fn source_refs_required_for_capability(capability_id: &str) -> bool {
    capability_id.starts_with("model.")
        || capability_id.starts_with("dataset.")
        || capability_id.starts_with("data.")
        || capability_id.starts_with("document.")
        || capability_id.starts_with("artifact.")
        || capability_id.starts_with("package.")
        || capability_id.starts_with("office.docx.batch")
}

fn coverage_required_for_capability(capability_id: &str) -> bool {
    capability_id.starts_with("source_set.")
        || capability_id.starts_with("dataset.")
        || capability_id.starts_with("data.")
        || capability_id.starts_with("document.")
        || capability_id.starts_with("office.docx.batch")
        || capability_id.starts_with("workspace.")
        || capability_id.starts_with("package.")
        || capability_id == "artifact.verify_coverage"
        || capability_id == "artifact.verify_typed"
}

fn drilldown_supported_for_capability(capability_id: &str) -> bool {
    capability_id.starts_with("source_set.")
        || capability_id.starts_with("dataset.")
        || capability_id.starts_with("office.docx.batch")
        || capability_id.starts_with("workspace.")
        || capability_id.starts_with("model.extract")
}

pub fn create_agent_job(
    workspace_root: impl AsRef<Path>,
    user_goal: &str,
) -> io::Result<(AgentJob, AgentProcess, ProcessTruthStore)> {
    let guard = WorkspaceGuard::new(workspace_root)?;
    create_agent_job_with_state_root(guard.root(), guard.root().join(RUNTIME_DIR_NAME), user_goal)
}

pub fn create_agent_job_with_state_root(
    workspace_root: impl AsRef<Path>,
    state_root: impl AsRef<Path>,
    user_goal: &str,
) -> io::Result<(AgentJob, AgentProcess, ProcessTruthStore)> {
    let guard = WorkspaceGuard::new(workspace_root)?;
    let suffix = now_ms();
    let job_id = format!("job_{}", suffix);
    let root_pid = format!("pid_{}", suffix);
    let truth = ProcessTruthStore::new_with_state_root(guard.root(), state_root, &job_id)?;
    let mut job = AgentJob {
        job_id: job_id.clone(),
        user_goal: user_goal.to_string(),
        workspace_root: guard.root().display().to_string(),
        status: "created".to_string(),
        root_pid: root_pid.clone(),
    };
    let process = AgentProcess {
        pid: root_pid.clone(),
        ppid: None,
        job_id: job_id.clone(),
        process_type: "root_agent_process".to_string(),
        state: "running".to_string(),
        input_refs: Vec::new(),
        output_refs: Vec::new(),
        capability_tokens: vec![format!("token_{}", suffix)],
        budget_ms: None,
        exit_code: None,
    };
    let runtime = TaskAgentRuntimeRecord {
        runtime_id: format!("tar_{}", suffix),
        job_id: job_id.clone(),
        root_pid: root_pid.clone(),
        state: "running".to_string(),
        checkpoint_refs: Vec::new(),
    };
    truth.register_job(&job)?;
    truth.append_event(Some(&root_pid), "job_created", to_json_value(&job)?)?;
    truth.update_job_status("running")?;
    job.status = "running".to_string();
    truth.register_process(&process)?;
    truth.register_task_agent_runtime(&runtime)?;
    truth.append_event(Some(&root_pid), "process_started", to_json_value(&process)?)?;
    truth.append_event(
        Some(&root_pid),
        "task_agent_runtime_started",
        to_json_value(&runtime)?,
    )?;
    Ok((job, process, truth))
}

fn event_payloads(event: &ProcessEvent) -> Vec<&Value> {
    let mut payloads = vec![&event.data];
    if let Some(data) = event.data.get("data") {
        payloads.push(data);
    }
    payloads
}

fn artifact_provenance_from_event(
    event: &ProcessEvent,
    payload: &Value,
) -> Option<ArtifactProvenance> {
    let capability_id = event
        .data
        .get("capability_id")
        .or_else(|| payload.get("capability_id"))
        .and_then(Value::as_str)?
        .to_string();
    let artifact_path = payload
        .get("artifact_path")
        .or_else(|| payload.get("archive_path"))
        .or_else(|| payload.get("destination_path"))
        .and_then(Value::as_str)?;
    if artifact_path.trim().is_empty() {
        return None;
    }
    let artifact_ref = payload
        .get("artifact_ref")
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .unwrap_or_else(|| format!("artifact://{}", artifact_path.replace('\\', "/")));
    let mut source_refs = Vec::new();
    for key in [
        "source_path",
        "source_set_ref",
        "dataset_ref",
        "left_path",
        "right_path",
        "before_path",
        "after_path",
    ] {
        if let Some(value) = payload.get(key).and_then(Value::as_str) {
            source_refs.push(value.to_string());
        }
    }
    source_refs.sort();
    source_refs.dedup();
    Some(ArtifactProvenance {
        event_id: event.event_id,
        capability_id,
        artifact_ref,
        artifact_path: artifact_path.replace('\\', "/"),
        tx_id: payload
            .get("tx_id")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        source_refs,
    })
}

fn push_artifact_ref(artifact_refs: &mut Vec<String>, path: &str) {
    let normalized = path.trim().replace('\\', "/");
    if !normalized.is_empty() {
        artifact_refs.push(normalized);
    }
}

pub(crate) fn walk_workspace(
    root: &Path,
    current: &Path,
    depth: usize,
    max_depth: usize,
    entries: &mut Vec<String>,
) -> io::Result<()> {
    if depth > max_depth {
        return Ok(());
    }
    let mut children = Vec::new();
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        if name.to_string_lossy() == RUNTIME_DIR_NAME {
            continue;
        }
        children.push(path);
    }
    children.sort();
    for path in children {
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .display()
            .to_string()
            .replace('\\', "/");
        entries.push(rel);
        if path.is_dir() {
            walk_workspace(root, &path, depth + 1, max_depth, entries)?;
        }
    }
    Ok(())
}

pub(crate) fn path_kind(path: &Path) -> Option<String> {
    if path.is_file() {
        Some("file".to_string())
    } else if path.is_dir() {
        Some("dir".to_string())
    } else if path.exists() {
        Some("other".to_string())
    } else {
        None
    }
}

pub(crate) fn path_size(path: &Path) -> io::Result<u64> {
    if path.is_file() {
        return Ok(path.metadata()?.len());
    }
    if path.is_dir() {
        let mut total = 0;
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let child = entry.path();
            let name = entry.file_name();
            if name.to_string_lossy() == RUNTIME_DIR_NAME {
                continue;
            }
            total += path_size(&child)?;
        }
        return Ok(total);
    }
    Ok(0)
}

pub(crate) fn path_fingerprint(root: &Path, path: &Path) -> io::Result<String> {
    if path.is_file() {
        return file_fingerprint(path);
    }
    if path.is_dir() {
        let mut parts = Vec::new();
        collect_path_fingerprints(root, path, &mut parts)?;
        return Ok(format!(
            "dir-fnv1a64:{:016x}",
            fnv1a64(parts.join("\n").as_bytes())
        ));
    }
    Ok("other".to_string())
}

fn collect_path_fingerprints(
    root: &Path,
    current: &Path,
    parts: &mut Vec<String>,
) -> io::Result<()> {
    let mut children = Vec::new();
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let name = entry.file_name();
        if name.to_string_lossy() == RUNTIME_DIR_NAME {
            continue;
        }
        children.push(entry.path());
    }
    children.sort();
    for child in children {
        let rel = child
            .strip_prefix(root)
            .unwrap_or(&child)
            .display()
            .to_string()
            .replace('\\', "/");
        if child.is_file() {
            parts.push(format!("{rel}:{}", file_fingerprint(&child)?));
        } else if child.is_dir() {
            parts.push(format!("{rel}:dir"));
            collect_path_fingerprints(root, &child, parts)?;
        }
    }
    Ok(())
}

pub(crate) fn remove_path_any(path: &Path) -> io::Result<()> {
    if path.is_dir() {
        fs::remove_dir_all(path)
    } else if path.exists() {
        fs::remove_file(path)
    } else {
        Ok(())
    }
}

pub(crate) fn copy_path_recursive(source: &Path, destination: &Path) -> io::Result<()> {
    if source.is_dir() {
        fs::create_dir_all(destination)?;
        let mut children = Vec::new();
        for entry in fs::read_dir(source)? {
            children.push(entry?.path());
        }
        children.sort();
        for child in children {
            let name = child.file_name().ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "path has no file name")
            })?;
            copy_path_recursive(&child, &destination.join(name))?;
        }
    } else if source.is_file() {
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(source, destination)?;
    } else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "unsupported path type",
        ));
    }
    Ok(())
}

pub(crate) fn restore_tx_backup(
    backup_ref: &str,
    truth: &ProcessTruthStore,
    destination: &Path,
) -> io::Result<()> {
    let prefix = format!("txbackup://{}/", truth.job_id);
    let relative = backup_ref.strip_prefix(&prefix).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "tx backup ref does not belong to this job",
        )
    })?;
    let mut backup_path = truth.state_root.join("tx_backups").join(&truth.job_id);
    for component in Path::new(relative).components() {
        match component {
            Component::Normal(part) => backup_path.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::Prefix(_) | Component::RootDir => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "tx backup ref leaves backup boundary",
                ));
            }
        }
    }
    if !backup_path.starts_with(truth.state_root.join("tx_backups").join(&truth.job_id)) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "tx backup ref leaves backup boundary",
        ));
    }
    copy_path_recursive(&backup_path, destination)
}

pub(crate) fn safe_blob_name(value: &str) -> String {
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

pub(crate) fn path_string(path: &Path) -> String {
    path.display().to_string()
}

pub(crate) fn new_tx_id(prefix: &str) -> String {
    let sequence = TX_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{}", now_ms(), sequence)
}

pub(crate) fn is_valid_write_kind(write_kind: &str) -> bool {
    matches!(write_kind, "artifact" | "source_mutation" | "temp_dataset")
}

pub(crate) fn text_file_diff(left: &Path, right: &Path) -> io::Result<String> {
    let left_text = String::from_utf8_lossy(&fs::read(left)?).to_string();
    let right_text = String::from_utf8_lossy(&fs::read(right)?).to_string();
    let left_lines: Vec<&str> = left_text.lines().collect();
    let right_lines: Vec<&str> = right_text.lines().collect();
    let max_len = left_lines.len().max(right_lines.len());
    let mut out = String::new();
    for index in 0..max_len {
        match (left_lines.get(index), right_lines.get(index)) {
            (Some(left), Some(right)) if left == right => {
                out.push_str(&format!(" {left}\n"));
            }
            (Some(left), Some(right)) => {
                out.push_str(&format!("-{left}\n+{right}\n"));
            }
            (Some(left), None) => out.push_str(&format!("-{left}\n")),
            (None, Some(right)) => out.push_str(&format!("+{right}\n")),
            (None, None) => {}
        }
    }
    Ok(out)
}

pub(crate) fn file_fingerprint(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut hash: u64 = 0xcbf29ce484222325;
    let mut buffer = [0_u8; 8192];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        for byte in &buffer[..read] {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
    }
    Ok(format!("fnv1a64:{hash:016x}"))
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

#[derive(Clone, Debug)]
struct ZipEntryData {
    name: String,
    bytes: Vec<u8>,
    crc32: u32,
    local_offset: u32,
}

pub(crate) fn write_store_zip(
    workspace_root: &Path,
    sources: &[(String, PathBuf)],
    destination: &Path,
) -> io::Result<usize> {
    let mut entries = Vec::new();
    for (_, source) in sources {
        collect_zip_entries(workspace_root, source, &mut entries)?;
    }
    entries.sort_by(|left, right| left.name.cmp(&right.name));
    entries.dedup_by(|left, right| left.name == right.name);

    let mut output = Vec::new();
    let mut written = Vec::new();
    for mut entry in entries {
        entry.local_offset = output.len() as u32;
        write_u32_le(&mut output, 0x0403_4b50);
        write_u16_le(&mut output, 20);
        write_u16_le(&mut output, 0x0800);
        write_u16_le(&mut output, 0);
        write_u16_le(&mut output, 0);
        write_u16_le(&mut output, 0);
        write_u32_le(&mut output, entry.crc32);
        write_u32_le(&mut output, entry.bytes.len() as u32);
        write_u32_le(&mut output, entry.bytes.len() as u32);
        write_u16_le(&mut output, entry.name.as_bytes().len() as u16);
        write_u16_le(&mut output, 0);
        output.extend_from_slice(entry.name.as_bytes());
        output.extend_from_slice(&entry.bytes);
        written.push(entry);
    }

    let central_offset = output.len() as u32;
    for entry in &written {
        write_u32_le(&mut output, 0x0201_4b50);
        write_u16_le(&mut output, 20);
        write_u16_le(&mut output, 20);
        write_u16_le(&mut output, 0x0800);
        write_u16_le(&mut output, 0);
        write_u16_le(&mut output, 0);
        write_u16_le(&mut output, 0);
        write_u32_le(&mut output, entry.crc32);
        write_u32_le(&mut output, entry.bytes.len() as u32);
        write_u32_le(&mut output, entry.bytes.len() as u32);
        write_u16_le(&mut output, entry.name.as_bytes().len() as u16);
        write_u16_le(&mut output, 0);
        write_u16_le(&mut output, 0);
        write_u16_le(&mut output, 0);
        write_u16_le(&mut output, 0);
        write_u32_le(&mut output, 0);
        write_u32_le(&mut output, entry.local_offset);
        output.extend_from_slice(entry.name.as_bytes());
    }
    let central_size = output.len() as u32 - central_offset;
    write_u32_le(&mut output, 0x0605_4b50);
    write_u16_le(&mut output, 0);
    write_u16_le(&mut output, 0);
    write_u16_le(&mut output, written.len() as u16);
    write_u16_le(&mut output, written.len() as u16);
    write_u32_le(&mut output, central_size);
    write_u32_le(&mut output, central_offset);
    write_u16_le(&mut output, 0);

    fs::write(destination, output)?;
    Ok(written.len())
}

fn collect_zip_entries(
    workspace_root: &Path,
    source: &Path,
    entries: &mut Vec<ZipEntryData>,
) -> io::Result<()> {
    if source.is_file() {
        let name = source
            .strip_prefix(workspace_root)
            .unwrap_or(source)
            .display()
            .to_string()
            .replace('\\', "/");
        let bytes = fs::read(source)?;
        entries.push(ZipEntryData {
            name,
            crc32: crc32(&bytes),
            bytes,
            local_offset: 0,
        });
    } else if source.is_dir() {
        let mut children = Vec::new();
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            let name = entry.file_name();
            if name.to_string_lossy() == RUNTIME_DIR_NAME {
                continue;
            }
            children.push(entry.path());
        }
        children.sort();
        for child in children {
            collect_zip_entries(workspace_root, &child, entries)?;
        }
    }
    Ok(())
}

pub(crate) fn read_store_zip(archive: &Path, destination: &Path) -> io::Result<usize> {
    let bytes = fs::read(archive)?;
    let mut offset = 0;
    let mut count = 0;
    while offset + 4 <= bytes.len() {
        let signature = read_u32_le(&bytes, offset)?;
        if signature == 0x0201_4b50 || signature == 0x0605_4b50 {
            break;
        }
        if signature != 0x0403_4b50 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "unsupported zip local header",
            ));
        }
        let method = read_u16_le(&bytes, offset + 8)?;
        if method != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "only store-method zip entries are supported",
            ));
        }
        let compressed_size = read_u32_le(&bytes, offset + 18)? as usize;
        let name_len = read_u16_le(&bytes, offset + 26)? as usize;
        let extra_len = read_u16_le(&bytes, offset + 28)? as usize;
        let name_start = offset + 30;
        let name_end = name_start + name_len;
        let data_start = name_end + extra_len;
        let data_end = data_start + compressed_size;
        if data_end > bytes.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "zip entry exceeds archive bounds",
            ));
        }
        let name = String::from_utf8(bytes[name_start..name_end].to_vec())
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        let target = resolve_archive_entry(destination, &name)?;
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(target, &bytes[data_start..data_end])?;
        count += 1;
        offset = data_end;
    }
    Ok(count)
}

fn resolve_archive_entry(destination: &Path, name: &str) -> io::Result<PathBuf> {
    let mut target = destination.to_path_buf();
    for component in Path::new(name).components() {
        match component {
            Component::Normal(part) => target.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::Prefix(_) | Component::RootDir => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "zip entry path leaves destination boundary",
                ));
            }
        }
    }
    if !target.starts_with(destination) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "zip entry path leaves destination boundary",
        ));
    }
    Ok(target)
}

fn write_u16_le(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn write_u32_le(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn read_u16_le(bytes: &[u8], offset: usize) -> io::Result<u16> {
    let slice = bytes
        .get(offset..offset + 2)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "unexpected end of zip data"))?;
    Ok(u16::from_le_bytes([slice[0], slice[1]]))
}

fn read_u32_le(bytes: &[u8], offset: usize) -> io::Result<u32> {
    let slice = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "unexpected end of zip data"))?;
    Ok(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffff_u32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = 0_u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

pub(crate) fn to_json_value<T: Serialize>(value: &T) -> io::Result<Value> {
    serde_json::to_value(value).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
}

fn query_json_rows<T: for<'de> Deserialize<'de>>(
    conn: &Connection,
    sql: &str,
    job_id: &str,
) -> io::Result<Vec<T>> {
    let mut stmt = conn.prepare(sql).map_err(sql_err)?;
    let rows = stmt
        .query_map(params![job_id], |row| {
            let raw: String = row.get(0)?;
            let value = serde_json::from_str::<T>(&raw).map_err(|err| {
                rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Text,
                    Box::new(err),
                )
            })?;
            Ok(value)
        })
        .map_err(sql_err)?;
    let mut values = Vec::new();
    for row in rows {
        values.push(row.map_err(sql_err)?);
    }
    Ok(values)
}

fn sql_err(err: rusqlite::Error) -> io::Error {
    io::Error::new(io::ErrorKind::Other, err)
}

pub(crate) fn json_err(err: serde_json::Error) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, err)
}

pub(crate) fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_millis(0))
        .as_millis()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, BTreeSet};
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::process::Command;
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::Duration as StdDuration;

    fn temp_workspace(name: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("supernova_process_kernel_{}_{}", name, now_ms()));
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn process_truth_uses_external_state_root_without_workspace_state() {
        let workspace = temp_workspace("truth_external_state_workspace");
        let state_root = std::env::temp_dir().join(format!(
            "supernova_process_truth_external_state_{}",
            now_ms()
        ));
        let (job, process, truth) =
            create_agent_job_with_state_root(&workspace, &state_root, "Use external state root")
                .unwrap();
        let workspace = workspace.canonicalize().unwrap();
        let state_root = state_root.canonicalize().unwrap();
        let blob_ref = truth
            .write_blob("model_inputs/source.txt", b"source")
            .unwrap();
        truth
            .append_event(
                Some(&process.pid),
                "external_state_test",
                json!({"blob_ref": blob_ref}),
            )
            .unwrap();
        let export_path = truth.export_jsonl(truth.export_path()).unwrap();

        assert_eq!(job.workspace_root, workspace.display().to_string());
        assert!(truth.path().starts_with(state_root.join("process_truth")));
        assert!(state_root
            .join("process_truth")
            .join(PROCESS_TRUTH_DB_FILENAME)
            .exists());
        assert!(state_root
            .join("blobs")
            .join(&job.job_id)
            .join("model_inputs")
            .join("source.txt")
            .exists());
        assert!(export_path.starts_with(state_root.join("process_truth")));
        assert!(!workspace.join(RUNTIME_DIR_NAME).exists());
    }

    fn read_http_request(stream: &mut TcpStream) -> String {
        let mut buffer = Vec::new();
        let mut chunk = [0_u8; 1024];
        loop {
            let read = stream.read(&mut chunk).unwrap();
            if read == 0 {
                break;
            }
            buffer.extend_from_slice(&chunk[..read]);
            let request = String::from_utf8_lossy(&buffer);
            if let Some(header_end) = request.find("\r\n\r\n") {
                let headers = &request[..header_end];
                let body_start = header_end + 4;
                let content_length = headers
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        if name.eq_ignore_ascii_case("content-length") {
                            value.trim().parse::<usize>().ok()
                        } else {
                            None
                        }
                    })
                    .unwrap_or(0);
                if buffer.len() >= body_start + content_length {
                    break;
                }
            }
        }
        String::from_utf8_lossy(&buffer).to_string()
    }

    fn phase4_native_tool_config() -> ModelInvocationConfig {
        let mut config = ModelInvocationConfig::default();
        config.decision_protocol = TaskAgentDecisionProtocol::ProviderNativeToolCalls;
        config.tool_calling.enabled = true;
        config.tool_calling.tool_choice = ToolChoicePolicy::Required;
        config.tool_calling.max_provider_subturns = 3;
        config.tool_calling.max_tool_calls_per_subturn = 1;
        config
    }

    fn deepseek_tool_call_body(
        call_id: &str,
        name: &str,
        arguments: Value,
        reasoning: &str,
    ) -> String {
        json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": null,
                    "reasoning_content": reasoning,
                    "tool_calls": [{
                        "id": call_id,
                        "type": "function",
                        "function": {
                            "name": name,
                            "arguments": arguments.to_string(),
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": {"prompt_tokens": 12, "completion_tokens": 3}
        })
        .to_string()
    }

    fn deepseek_tool_calls_body(calls: Vec<(&str, &str, Value)>, reasoning: &str) -> String {
        let tool_calls = calls
            .into_iter()
            .map(|(call_id, name, arguments)| {
                json!({
                    "id": call_id,
                    "type": "function",
                    "function": {
                        "name": name,
                        "arguments": arguments.to_string(),
                    }
                })
            })
            .collect::<Vec<_>>();
        json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": null,
                    "reasoning_content": reasoning,
                    "tool_calls": tool_calls
                },
                "finish_reason": "tool_calls"
            }],
            "usage": {"prompt_tokens": 16, "completion_tokens": 6}
        })
        .to_string()
    }

    fn write_json_response(stream: &mut TcpStream, body: &str) {
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.as_bytes().len(),
            body
        );
        stream.write_all(response.as_bytes()).unwrap();
    }

    #[test]
    fn task_agent_prompt_protocol_is_provider_native_only() {
        let capabilities = default_capability_registry();
        let capability_prompt = task_agent_system_prompt_for_protocol(
            &capabilities,
            TaskAgentPromptProtocol::ProviderNativeToolCalls,
        );
        let native_prompt = task_agent_provider_native_system_prompt(
            "[Toolset Index]\nSelectable groups:\n- `office_docx`: DOCX tools",
            "[Current Toolset]\nAvailable tools:\n- `cap_process_complete`: close\n- `cap_process_toolset_select`: select groups",
        );
        let native_instruction = task_agent_decision_instruction_for_protocol(
            "Inspect a file",
            TaskAgentPromptProtocol::ProviderNativeToolCalls,
        );

        assert!(capability_prompt.contains("Provider-native tool-call protocol"));
        assert!(capability_prompt.contains("tool_calls"));
        assert!(!capability_prompt.contains("Return only JSON"));
        assert!(!capability_prompt.contains("\"decision_type\""));
        assert!(native_prompt.contains("[Stable Kernel Contract]"));
        assert!(native_prompt.contains("[Toolset Index]"));
        assert!(native_prompt.contains("[Current Toolset]"));
        assert!(native_prompt.contains("do not return a SuperNova JSON decision object"));
        assert!(
            native_prompt.contains("Use DeepSeek provider `tool_calls` for executable progress")
        );
        assert!(native_prompt.contains("Plain assistant content is not task closure"));
        assert!(native_prompt.contains("do not wrap fields inside an `arguments` object"));
        assert!(native_prompt.contains("cap_process_toolset_select"));
        assert!(!native_prompt.contains("Return only JSON"));
        assert!(!native_prompt.contains("\"decision_type\""));
        assert!(!native_prompt.contains("Capability map"));
        assert!(native_instruction.contains("Use provider tool_calls for executable progress"));
        assert!(native_instruction
            .contains("Plain assistant content is treated only as intermediate content"));
    }

    #[test]
    fn deepseek_provider_tool_budget_defaults_are_hard_guards_not_business_limits() {
        let config = ToolCallingConfig::default();
        assert_eq!(config.max_provider_subturns, 16);
        assert_eq!(config.max_tool_calls_per_subturn, 32);
        assert_eq!(config.max_tool_calls_per_task, 512);
        assert_eq!(config.max_tool_calls_per_chat_turn, 256);
        assert_eq!(config.max_chat_read_bytes_per_turn, 64 * 1024 * 1024);
        assert_eq!(config.max_chat_read_tokens_per_turn, 1_000_000);
    }

    #[test]
    fn deepseek_context_profile_uses_64k_default_output_budget() {
        let provider =
            DeepSeekModelProvider::new("test-key", "http://127.0.0.1:1", "deepseek-v4", 5_000)
                .with_streaming(false);
        let profile =
            ModelContextProfile::for_provider(&provider, &ModelOperation::DecideNextAction);
        assert_eq!(profile.default_output_tokens, 65_536);
        assert_eq!(profile.long_output_tokens, 65_536);
        assert_eq!(
            profile
                .budget_for(&ModelOperation::DecideNextAction)
                .max_output_tokens,
            65_536
        );
        let decision_budget = profile.budget_for(&ModelOperation::DecideNextAction);
        assert_eq!(
            decision_budget.max_input_bytes,
            (1_000_000_u64 - 65_536_u64) * 4
        );
        assert_eq!(
            context_window_tokens_for_budget(&decision_budget),
            1_000_000
        );
        assert_eq!(
            profile
                .budget_for(&ModelOperation::GenerateArtifact)
                .max_output_tokens,
            65_536
        );
        assert_eq!(ModelBudget::default().max_output_tokens, 65_536);
    }

    fn artifact_audit_pass_output(path: &str) -> String {
        json!({
            "audit_kind": "model_backed_artifact_quality_findings",
            "audit_scope": "single_artifact",
            "artifact_path": path,
            "included_artifacts": [path],
            "quality_pass": true,
            "human_acceptance_pass": true,
            "findings": [
                {"severity": "info", "message": "Artifact is readable and aligned with the requested task."}
            ],
            "blocking_issues": [],
            "factual_risks": [],
            "deliverability_risks": [],
            "source_grounding": {
                "status": "sufficient_for_test",
                "notes": "Unit test source evidence is simple and explicit."
            },
            "coverage_assessment": {
                "status": "sufficient_for_test",
                "notes": "The artifact covers the small test fixture."
            },
            "suggested_review_focus": [],
            "auditor_limitations": []
        })
        .to_string()
    }

    #[derive(Debug)]
    struct SequencedModelProvider {
        provider: String,
        model: String,
        outputs: Mutex<BTreeMap<String, Vec<String>>>,
        tool_call_outputs: Mutex<BTreeMap<String, Vec<Vec<ProviderToolCall>>>>,
    }

    impl SequencedModelProvider {
        fn new(provider: impl Into<String>, model: impl Into<String>) -> Self {
            Self {
                provider: provider.into(),
                model: model.into(),
                outputs: Mutex::new(BTreeMap::new()),
                tool_call_outputs: Mutex::new(BTreeMap::new()),
            }
        }

        fn with_outputs(mut self, operation: ModelOperation, outputs: Vec<String>) -> Self {
            self.outputs
                .get_mut()
                .unwrap()
                .insert(operation.as_str().to_string(), outputs);
            self
        }

        fn with_tool_call_outputs(
            mut self,
            operation: ModelOperation,
            outputs: Vec<Vec<ProviderToolCall>>,
        ) -> Self {
            self.tool_call_outputs
                .get_mut()
                .unwrap()
                .insert(operation.as_str().to_string(), outputs);
            self
        }
    }

    impl ModelProvider for SequencedModelProvider {
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
                "protocol": "sequenced_test_provider",
                "supports_operations": ["decide_next_action", "generate_artifact", "audit"],
                "supports_schema_validation": true,
                "supports_ledger": true,
            })
        }

        fn invoke(
            &self,
            request: &ModelProviderRequest,
        ) -> Result<ModelProviderResponse, ModelProviderFailure> {
            let operation = request.action.operation.as_str().to_string();
            let tool_calls = {
                let mut tool_call_outputs = self.tool_call_outputs.lock().unwrap();
                match tool_call_outputs.get_mut(&operation) {
                    Some(queue) if !queue.is_empty() => queue.remove(0),
                    Some(_) => Vec::new(),
                    None => Vec::new(),
                }
            };
            let tool_calls = tool_calls
                .into_iter()
                .map(|call| substitute_tool_call_placeholders(call, request))
                .collect::<Vec<_>>();
            let mut outputs = self.outputs.lock().unwrap();
            let output_text = if !tool_calls.is_empty() {
                String::new()
            } else if let Some(queue) = outputs.get_mut(&operation) {
                if queue.is_empty() {
                    return Err(ModelProviderFailure {
                        error_code: "SEQUENCED_OUTPUT_EXHAUSTED".to_string(),
                        message: format!("exhausted sequenced output for {operation}"),
                        retryable: false,
                    });
                }
                substitute_test_placeholders(queue.remove(0), request)
            } else if tool_calls.is_empty() {
                return Err(ModelProviderFailure {
                    error_code: "SEQUENCED_OUTPUT_MISSING".to_string(),
                    message: format!("missing sequenced output for {operation}"),
                    retryable: false,
                });
            } else {
                String::new()
            };
            Ok(ModelProviderResponse {
                output_text,
                assistant_message: (!tool_calls.is_empty()).then(|| ProviderAssistantMessage {
                    role: "assistant".to_string(),
                    content: None,
                    reasoning_content: None,
                    tool_calls: tool_calls.clone(),
                }),
                reasoning_content: None,
                tool_calls,
                usage: json!({"operation": operation}),
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

    fn substitute_test_placeholders(
        mut output_text: String,
        request: &ModelProviderRequest,
    ) -> String {
        output_text = output_text.replace("{{job_id}}", &request.action.job_id);
        for (index, input_ref) in request.action.input_refs.iter().enumerate() {
            output_text = output_text.replace(&format!("{{{{input_ref_{index}}}}}"), input_ref);
        }
        output_text
    }

    fn substitute_tool_call_placeholders(
        mut call: ProviderToolCall,
        request: &ModelProviderRequest,
    ) -> ProviderToolCall {
        if let Some(arguments) = call.function.get_mut("arguments") {
            let encoded = match arguments {
                Value::String(value) => value.clone(),
                _ => serde_json::to_string(arguments).unwrap(),
            };
            let substituted = substitute_test_placeholders(encoded, request);
            *arguments = serde_json::from_str(&substituted).unwrap_or(Value::String(substituted));
        }
        call
    }

    fn provider_tool_call(
        id: &str,
        capability_id: &str,
        arguments: Value,
    ) -> ProviderToolCall {
        ProviderToolCall {
            id: id.to_string(),
            r#type: "function".to_string(),
            function: json!({
                "name": provider_tool_name_for_capability(capability_id),
                "arguments": arguments,
            }),
        }
    }

    #[test]
    fn process_truth_is_append_only_and_replayable() {
        let workspace = temp_workspace("truth");
        let (job, process, truth) = create_agent_job(&workspace, "Generate TREE.md").unwrap();
        truth
            .append_event(
                Some(&process.pid),
                "capability_receipt",
                json!({"capability_id": "os.write_artifact", "artifact_path": "TREE.md"}),
            )
            .unwrap();
        truth
            .append_event(
                Some(&process.pid),
                "capability_receipt",
                json!({"capability_id": "process.complete", "artifact_path": ""}),
            )
            .unwrap();
        truth
            .append_event(
                Some(&process.pid),
                "job_completed",
                json!({"artifacts": ["TREE.md", ""]}),
            )
            .unwrap();

        let events = truth.read_events().unwrap();
        assert_eq!(events[0].event_id, 1);
        assert_eq!(events[1].event_id, 2);
        assert!(truth.path().exists());
        assert_eq!(truth.path().file_name().unwrap(), PROCESS_TRUTH_DB_FILENAME);
        let replay = truth.replay().unwrap();
        assert_eq!(replay.job_id, job.job_id);
        assert_eq!(replay.status, "completed");
        assert!(replay.artifact_refs.contains(&"TREE.md".to_string()));
        assert!(!replay.artifact_refs.contains(&"".to_string()));
        assert!(replay.artifact_provenance.iter().any(
            |item| item.artifact_path == "TREE.md" && item.capability_id == "os.write_artifact"
        ));
        let session = truth.replay_session().unwrap();
        assert_eq!(session.state.job_id, replay.job_id);
        assert!(session.events.len() >= 2);
    }

    #[test]
    fn sqlite_process_truth_registry_stream_checkpoint_and_export_are_available() {
        let workspace = temp_workspace("phase1_sqlite");
        let (job, process, truth) = create_agent_job(&workspace, "Checkpoint test").unwrap();

        let snapshot = truth.registry_snapshot().unwrap();
        assert_eq!(snapshot.jobs.len(), 1);
        assert_eq!(snapshot.jobs[0].job_id, job.job_id);
        assert_eq!(snapshot.processes.len(), 1);
        assert_eq!(snapshot.processes[0].pid, process.pid);
        assert_eq!(snapshot.task_agent_runtimes.len(), 1);
        assert_eq!(snapshot.task_agent_runtimes[0].root_pid, process.pid);

        let ckpt = truth
            .save_checkpoint(
                &process.pid,
                Some(&snapshot.task_agent_runtimes[0].runtime_id),
                "task_agent_runtime",
                &json!({"phase": "observe", "refs": ["blob://source"]}),
                vec!["source_set_ref://initial".to_string()],
                vec!["artifact_ref://pending".to_string()],
            )
            .unwrap();
        assert!(ckpt.checkpoint_id.starts_with("ckpt_"));
        let checkpoints = truth.list_checkpoints().unwrap();
        assert_eq!(checkpoints.len(), 1);
        assert_eq!(checkpoints[0].state_ref, ckpt.state_ref);
        assert_eq!(
            checkpoints[0].input_refs,
            vec!["source_set_ref://initial".to_string()]
        );

        let streamed = truth.stream_events(2, 10).unwrap();
        assert!(streamed
            .iter()
            .any(|event| event.event_type == "task_agent_runtime_started"));
        assert!(streamed
            .iter()
            .any(|event| event.event_type == "checkpoint_saved"));

        let export_path = truth.export_jsonl(truth.export_path()).unwrap();
        let exported = fs::read_to_string(export_path).unwrap();
        assert!(exported.contains("checkpoint_saved"));

        truth.update_job_status("waiting_approval").unwrap();
        let waiting_snapshot = truth.registry_snapshot().unwrap();
        assert_eq!(waiting_snapshot.jobs[0].status, "waiting_approval");
        assert_eq!(waiting_snapshot.processes[0].state, "waiting_approval");
        assert_eq!(
            waiting_snapshot.task_agent_runtimes[0].state,
            "waiting_approval"
        );
        truth.update_job_status("cancelled").unwrap();
        let cancelled_snapshot = truth.registry_snapshot().unwrap();
        assert_eq!(cancelled_snapshot.jobs[0].status, "cancelled");
        assert_eq!(cancelled_snapshot.processes[0].state, "cancelled");
        assert_eq!(cancelled_snapshot.task_agent_runtimes[0].state, "cancelled");
        assert_eq!(truth.replay().unwrap().status, "cancelled");
    }

    #[test]
    fn job_status_surface_matches_phase1_plan() {
        assert_eq!(
            JOB_STATUSES,
            [
                "created",
                "running",
                "waiting_user",
                "waiting_approval",
                "blocked",
                "failed",
                "interrupted",
                "completed",
                "cancelled",
            ]
        );
    }

    #[test]
    fn workspace_guard_blocks_path_escape() {
        let workspace = temp_workspace("guard");
        let guard = WorkspaceGuard::new(&workspace).unwrap();

        assert!(guard
            .resolve_workspace_path("safe/report.md")
            .unwrap()
            .starts_with(guard.root()));
        assert!(guard.resolve_workspace_path("../outside.txt").is_err());
        assert!(guard
            .resolve_workspace_path(Path::new("C:\\outside.txt"))
            .is_err());
    }

    #[test]
    fn capability_registry_declares_v2_algebra_fields() {
        let registry = default_capability_registry();
        let verifier_registry = default_verifier_registry();
        let terminal = registry
            .iter()
            .find(|item| item.capability_id == "terminal.run_command")
            .unwrap();
        assert_eq!(terminal.output_schema, "CommandReceipt");
        assert!(terminal
            .required_permissions
            .contains(&"terminal:execute".to_string()));
        assert_eq!(terminal.verifier, "command_receipt_verifier");

        let office = registry
            .iter()
            .find(|item| item.capability_id == "office.docx.rewrite_save_as")
            .unwrap();
        assert_eq!(office.verifier, "openxml_validation_verifier");
        assert_eq!(office.rollback, "tx_rollback");
        assert_eq!(
            office.input_schema,
            "DocxRef + OutputPath + RewrittenTextRef"
        );
        assert!(registry
            .iter()
            .any(|item| item.capability_id == "office.docx.rewrite_preview"));
        assert!(registry
            .iter()
            .any(|item| item.capability_id == "office.docx.validate"));
        assert!(verifier_registry
            .iter()
            .any(|item| item.target_id == "os.write_file"
                && item.checks.contains(&"no_silent_fallback".to_string())));
    }

    #[test]
    fn terminal_service_capabilities_are_provider_visible_and_require_explicit_timeouts() {
        let registry = default_capability_registry();
        for capability_id in [
            "terminal.start_service",
            "terminal.stop_service",
            "terminal.service_status",
        ] {
            assert!(
                registry
                    .iter()
                    .any(|item| item.capability_id == capability_id),
                "missing descriptor {capability_id}"
            );
            assert!(
                crate::provider_tool::provider_tool_capability_is_task_runtime_exposable(
                    capability_id
                )
            );
        }
        let run_command = registry
            .iter()
            .find(|item| item.capability_id == "terminal.run_command")
            .unwrap();
        let run_schema = provider_tool_parameters_for_descriptor(run_command);
        let required = run_schema
            .get("required")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();
        assert!(required.contains(&"argv"));
        assert!(required.contains(&"timeout_ms"));

        let start_service = registry
            .iter()
            .find(|item| item.capability_id == "terminal.start_service")
            .unwrap();
        let start_schema = provider_tool_parameters_for_descriptor(start_service);
        let required = start_schema
            .get("required")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();
        assert!(required.contains(&"service_id"));
        assert!(required.contains(&"argv"));
        assert!(required.contains(&"startup_timeout_ms"));

        let terminal_group = crate::provider_toolset::provider_tool_group_descriptors()
            .into_iter()
            .find(|group| group.group_id == "terminal_fallback")
            .expect("terminal_fallback group exists");
        assert!(terminal_group
            .capability_ids
            .contains(&"terminal.start_service".to_string()));
        assert!(terminal_group
            .capability_ids
            .contains(&"terminal.stop_service".to_string()));
        assert!(terminal_group
            .capability_ids
            .contains(&"terminal.service_status".to_string()));
    }

    #[test]
    fn client_env_capabilities_are_readonly_and_provider_visible() {
        let registry = default_capability_registry();
        let client_env_ids = [
            "client_env.scan_overview",
            "client_env.scan_device",
            "client_env.scan_storage",
            "client_env.scan_network",
            "client_env.scan_runtimes",
            "client_env.read_snapshot",
            "client_env.request_sensitive_disclosure",
        ];
        for capability_id in client_env_ids {
            let descriptor = registry
                .iter()
                .find(|item| item.capability_id == capability_id)
                .unwrap_or_else(|| panic!("missing descriptor {capability_id}"));
            assert_eq!(
                descriptor.required_permissions,
                vec!["client_env:read".to_string()]
            );
            assert_eq!(descriptor.side_effects, vec!["read".to_string()]);
            assert_eq!(descriptor.approval_policy, "read_only");
            assert_eq!(descriptor.rollback, "none");
            assert!(!descriptor.fallback_allowed);
            assert!(descriptor.verification_required);
            assert!(
                !crate::provider_tool::provider_tool_is_mutation_apply_capability(capability_id)
            );
            assert!(crate::provider_tool::provider_tool_capability_is_exposable(
                capability_id
            ));
            let schema = provider_tool_parameters_for_descriptor(descriptor);
            assert!(schema
                .get("properties")
                .and_then(Value::as_object)
                .is_some_and(|properties| !properties.contains_key("raw_arguments")));
        }
        let groups = crate::provider_toolset::provider_tool_group_descriptors();
        let client_group = groups
            .iter()
            .find(|group| group.group_id == "client_environment")
            .expect("client_environment group is registered");
        assert!(client_group
            .capability_ids
            .contains(&"client_env.scan_runtimes".to_string()));
        assert!(!client_group.approval_gated);

        let config = ModelInvocationConfig::default();
        let chat_registry = ProviderToolRegistry::chat_runtime_readonly(&registry, &config);
        assert!(chat_registry
            .bindings
            .values()
            .any(|binding| binding.capability_id == "client_env.scan_overview"));
    }

    #[test]
    fn terminal_runtime_emits_command_receipt_with_refs() {
        let workspace = temp_workspace("terminal");
        let (job, process, truth) = create_agent_job(&workspace, "Run command").unwrap();
        let token = CapabilityToken {
            token_id: "token_test".to_string(),
            job_id: job.job_id,
            pid: process.pid,
            workspace_root: workspace.display().to_string(),
            capabilities: vec!["terminal.run_command".to_string()],
            permissions: vec!["terminal:execute".to_string()],
        };
        let runtime = TerminalRuntime::new(
            WorkspaceGuard::new(&workspace).unwrap(),
            truth.clone(),
            token,
        );
        let receipt = runtime
            .run_command(vec!["rustc".to_string(), "--version".to_string()], 20_000)
            .unwrap();

        assert_eq!(receipt.capability_id, "terminal.run_command");
        assert!(matches!(receipt.status.as_str(), "success" | "failed"));
        assert_eq!(receipt.data["argv"][0], "rustc");
        assert_eq!(
            receipt.data["cwd"],
            workspace.canonicalize().unwrap().display().to_string()
        );
        assert!(receipt.data["pid"].as_u64().unwrap_or(0) > 0);
        assert_eq!(receipt.data["process_tree"].as_array().unwrap().len(), 1);
        assert_eq!(
            receipt.data["env"]["SUPERNOVA_TERMINAL_WRAPPER"]
                .as_str()
                .unwrap(),
            "1"
        );
        assert!(receipt.data["stdout_ref"]
            .as_str()
            .unwrap()
            .starts_with("blob://"));
        assert!(receipt.data["workspace_diff_ref"]
            .as_str()
            .unwrap()
            .starts_with("blob://"));
        assert!(receipt.data["resource_usage"]["wall_time_ms"]
            .as_u64()
            .is_some());
        assert!(truth
            .read_events()
            .unwrap()
            .iter()
            .any(|event| event.event_type == "command_receipt"));
    }

    #[test]
    #[ignore = "RC0 run-through disables terminal preview approval blocking."]
    fn capability_kernel_blocks_terminal_mutation_without_preview_approval() {
        let workspace = temp_workspace("terminal_block");
        let (job, process, truth) = create_agent_job(&workspace, "Block mutation").unwrap();
        let token = CapabilityToken {
            token_id: "token_block".to_string(),
            job_id: job.job_id,
            pid: process.pid,
            workspace_root: workspace.display().to_string(),
            capabilities: vec!["terminal.run_command".to_string()],
            permissions: vec!["terminal:execute".to_string()],
        };
        let request = CapabilityApprovalRequest {
            capability_id: "terminal.run_command".to_string(),
            policy: CapabilityApprovalPolicy::SourceMutationRequired,
            target_paths: vec!["*".to_string()],
            target_path_schema: "explicit target_paths".to_string(),
            explicit_approval_id: None,
        };
        let receipt = prepare_capability_approval(&truth, &token, request)
            .unwrap()
            .unwrap_err();
        assert_eq!(receipt.status, "blocked");
        assert_eq!(receipt.data["approval_required"], true);
        assert_eq!(receipt.data["mutation_policy_blocked"], true);
        assert!(!workspace.join("blocked.txt").exists());
    }

    #[test]
    fn terminal_runtime_hard_blocks_parent_dir_mutation_even_with_approval() {
        let workspace = temp_workspace("terminal_boundary_block");
        let (job, process, truth) =
            create_agent_job(&workspace, "Never allow terminal boundary escape").unwrap();
        let token = CapabilityToken {
            token_id: "token_boundary_block".to_string(),
            job_id: job.job_id,
            pid: process.pid,
            workspace_root: workspace.display().to_string(),
            capabilities: vec!["terminal.run_command".to_string()],
            permissions: vec!["terminal:execute".to_string()],
        };
        let runtime = TerminalRuntime::new(
            WorkspaceGuard::new(&workspace).unwrap(),
            truth.clone(),
            token,
        );

        let receipt = runtime
            .run_powershell(
                "Set-Content -Path ../outside.txt -Value blocked",
                20_000,
                TerminalApproval::approved("approval_must_not_override_boundary"),
            )
            .unwrap();

        assert_eq!(receipt.status, "blocked");
        assert_eq!(receipt.data["approval_allowed"], false);
        assert_eq!(receipt.data["hard_block"], true);
        assert!(!workspace.parent().unwrap().join("outside.txt").exists());
    }

    #[test]
    fn terminal_runtime_allows_approved_powershell_mutation_and_records_diff() {
        let workspace = temp_workspace("terminal_approved");
        let (job, process, truth) = create_agent_job(&workspace, "Approved mutation").unwrap();
        let token = CapabilityToken {
            token_id: "token_approved".to_string(),
            job_id: job.job_id,
            pid: process.pid,
            workspace_root: workspace.display().to_string(),
            capabilities: vec!["terminal.run_command".to_string()],
            permissions: vec!["terminal:execute".to_string()],
        };
        let runtime = TerminalRuntime::new(
            WorkspaceGuard::new(&workspace).unwrap(),
            truth.clone(),
            token,
        );

        let receipt = runtime
            .run_powershell(
                "Set-Content -Path approved.txt -Value approved",
                20_000,
                TerminalApproval::approved("approval_terminal_test"),
            )
            .unwrap();

        assert_eq!(receipt.status, "success");
        assert_eq!(receipt.data["shell_kind"], "powershell");
        assert_eq!(receipt.data["mutation_detected"], true);
        assert_eq!(receipt.data["approval_id"], "approval_terminal_test");
        assert!(workspace.join("approved.txt").exists());
        let diff_ref = receipt.data["workspace_diff_ref"].as_str().unwrap();
        let diff_path = truth.resolve_blob_ref(diff_ref).unwrap();
        let diff: WorkspaceDiff =
            serde_json::from_str(&fs::read_to_string(diff_path).unwrap()).unwrap();
        assert!(diff.added_files.contains(&"approved.txt".to_string()));
    }

    #[test]
    fn os_runtime_uses_refs_and_runtime_verify_for_artifacts() {
        let workspace = temp_workspace("os_runtime");
        fs::create_dir_all(workspace.join("docs")).unwrap();
        fs::write(workspace.join("docs").join("brief.md"), "brief").unwrap();
        let (job, process, truth) = create_agent_job(&workspace, "Generate workspace map").unwrap();
        let token = CapabilityToken {
            token_id: "token_os".to_string(),
            job_id: job.job_id,
            pid: process.pid,
            workspace_root: workspace.display().to_string(),
            capabilities: vec![
                "os.list_tree".to_string(),
                "os.workspace_inventory".to_string(),
                "os.write_artifact".to_string(),
                "os.verify_artifact".to_string(),
            ],
            permissions: vec!["fs:read".to_string(), "fs:write".to_string()],
        };
        let runtime = OsRuntime::new(
            WorkspaceGuard::new(&workspace).unwrap(),
            truth.clone(),
            token,
        );

        let tree = runtime.list_tree(3).unwrap();
        assert_eq!(tree.status, "success");
        assert!(tree.data["source_set_ref"]
            .as_str()
            .unwrap()
            .starts_with("blob://"));
        let inventory = runtime.workspace_inventory(8).unwrap();
        assert_eq!(inventory.status, "success");
        assert_eq!(inventory.data["document_count"], 1);
        assert!(inventory.data["document_index_csv_ref"]
            .as_str()
            .unwrap()
            .starts_with("blob://"));

        let write = runtime
            .write_artifact("TREE.md", b"# Tree\n\n- docs/brief.md\n")
            .unwrap();
        assert_eq!(write.status, "success");
        assert_eq!(write.data["artifact_path"], "TREE.md");

        let verify = runtime.verify_artifact("TREE.md").unwrap();
        assert_eq!(verify.status, "success");
        assert_eq!(verify.data["exists"], true);
        assert!(truth
            .read_events()
            .unwrap()
            .iter()
            .any(|event| event.event_type == "verify_event"));
    }

    #[test]
    fn os_runtime_phase3_file_ops_record_tx_and_rollback() {
        let workspace = temp_workspace("os_phase3_mutation");
        fs::create_dir_all(workspace.join("docs")).unwrap();
        fs::write(workspace.join("docs").join("source.txt"), "source v1").unwrap();
        let (job, process, truth) = create_agent_job(&workspace, "OS mutation tx").unwrap();
        let token = CapabilityToken {
            token_id: "token_os_phase3".to_string(),
            job_id: job.job_id,
            pid: process.pid,
            workspace_root: workspace.display().to_string(),
            capabilities: vec![
                "os.write_file".to_string(),
                "os.copy_path".to_string(),
                "os.move_path".to_string(),
                "os.rename_path".to_string(),
                "os.delete_path".to_string(),
                "os.rollback_tx".to_string(),
            ],
            permissions: vec!["fs:read".to_string(), "fs:write".to_string()],
        };
        let runtime = OsRuntime::new(
            WorkspaceGuard::new(&workspace).unwrap(),
            truth.clone(),
            token,
        );

        let write = runtime
            .write_file("reports/new.md", b"new report", "artifact")
            .unwrap();
        assert_eq!(write.status, "success");
        let write_tx = write.data["tx_id"].as_str().unwrap();
        assert!(workspace.join("reports").join("new.md").exists());
        runtime.rollback_tx(write_tx).unwrap();
        assert!(!workspace.join("reports").join("new.md").exists());

        let direct_copy = runtime
            .copy_path("docs/source.txt", "docs/source_copy_direct.txt")
            .unwrap();
        assert_eq!(direct_copy.status, "success");
        assert!(workspace
            .join("docs")
            .join("source_copy_direct.txt")
            .exists());

        let copy = runtime
            .copy_path_with_approval(
                "docs/source.txt",
                "docs/source_copy.txt",
                Some("approval_os_copy"),
            )
            .unwrap();
        assert!(workspace.join("docs").join("source_copy.txt").exists());
        runtime
            .rollback_tx(copy.data["tx_id"].as_str().unwrap())
            .unwrap();
        assert!(!workspace.join("docs").join("source_copy.txt").exists());

        let moved = runtime
            .move_path_with_approval(
                "docs/source.txt",
                "archive/source.txt",
                Some("approval_os_move"),
            )
            .unwrap();
        assert!(!workspace.join("docs").join("source.txt").exists());
        assert!(workspace.join("archive").join("source.txt").exists());
        runtime
            .rollback_tx(moved.data["tx_id"].as_str().unwrap())
            .unwrap();
        assert!(workspace.join("docs").join("source.txt").exists());
        assert!(!workspace.join("archive").join("source.txt").exists());

        let renamed = runtime
            .rename_path_with_approval(
                "docs/source.txt",
                "docs/source_renamed.txt",
                Some("approval_os_rename"),
            )
            .unwrap();
        assert!(!workspace.join("docs").join("source.txt").exists());
        assert!(workspace.join("docs").join("source_renamed.txt").exists());
        runtime
            .rollback_tx(renamed.data["tx_id"].as_str().unwrap())
            .unwrap();
        assert!(workspace.join("docs").join("source.txt").exists());

        let delete_runtime_is_pure = runtime.delete_path("docs/source.txt").unwrap();
        assert_eq!(delete_runtime_is_pure.status, "success");
        runtime
            .rollback_tx(delete_runtime_is_pure.data["tx_id"].as_str().unwrap())
            .unwrap();
        assert!(workspace.join("docs").join("source.txt").exists());
        let deleted = runtime
            .delete_path_with_approval("docs/source.txt", Some("approval_os_delete"))
            .unwrap();
        assert!(!workspace.join("docs").join("source.txt").exists());
        runtime
            .rollback_tx(deleted.data["tx_id"].as_str().unwrap())
            .unwrap();
        assert_eq!(
            fs::read_to_string(workspace.join("docs").join("source.txt")).unwrap(),
            "source v1"
        );
        let events = truth.read_events().unwrap();
        assert!(events.iter().any(|event| event.event_type == "tx_recorded"));
        assert!(events.iter().any(|event| event.event_type == "tx_rollback"));
    }

    #[test]
    fn os_runtime_phase3_read_hash_diff_zip_unzip_use_refs() {
        let workspace = temp_workspace("os_phase3_dataflow");
        fs::create_dir_all(workspace.join("docs")).unwrap();
        fs::write(workspace.join("docs").join("a.txt"), "alpha\nsame\n").unwrap();
        fs::write(workspace.join("docs").join("b.txt"), "beta\nsame\n").unwrap();
        let (job, process, truth) = create_agent_job(&workspace, "OS refs").unwrap();
        let token = CapabilityToken {
            token_id: "token_os_phase3_refs".to_string(),
            job_id: job.job_id,
            pid: process.pid,
            workspace_root: workspace.display().to_string(),
            capabilities: vec![
                "os.stat_path".to_string(),
                "os.read_file".to_string(),
                "os.hash_path".to_string(),
                "os.diff".to_string(),
                "os.zip".to_string(),
                "os.unzip".to_string(),
                "os.rollback_tx".to_string(),
            ],
            permissions: vec!["fs:read".to_string(), "fs:write".to_string()],
        };
        let runtime = OsRuntime::new(
            WorkspaceGuard::new(&workspace).unwrap(),
            truth.clone(),
            token,
        );

        let stat = runtime.stat_path("docs/a.txt").unwrap();
        assert_eq!(stat.data["kind"], "file");
        let read = runtime.read_file("docs/a.txt").unwrap();
        assert!(read.data["dataset_ref"]
            .as_str()
            .unwrap()
            .starts_with("blob://"));
        let read_dir = runtime.read_file("docs").unwrap();
        assert_eq!(read_dir.status, "blocked");
        assert_eq!(read_dir.data["target_kind"], "dir");
        assert_eq!(read_dir.data["recoverable"], true);
        assert!(read_dir.data["recommended_capabilities"]
            .as_array()
            .unwrap()
            .contains(&json!("os.list_tree")));
        let hash = runtime.hash_path("docs").unwrap();
        assert!(hash.data["hash_ref"]
            .as_str()
            .unwrap()
            .starts_with("blob://"));
        let diff = runtime.diff_files("docs/a.txt", "docs/b.txt").unwrap();
        let diff_path = truth
            .resolve_blob_ref(diff.data["diff_ref"].as_str().unwrap())
            .unwrap();
        assert!(fs::read_to_string(diff_path).unwrap().contains("-alpha"));

        let zip = runtime.zip_paths(&["docs"], "out/docs.zip").unwrap();
        assert_eq!(zip.status, "success");
        assert!(workspace.join("out").join("docs.zip").exists());
        let direct_unzip = runtime
            .unzip_archive("out/docs.zip", "direct_unzip")
            .unwrap();
        assert_eq!(direct_unzip.status, "success");
        assert!(workspace.join("direct_unzip").exists());
        let unzip = runtime
            .unzip_archive_with_approval("out/docs.zip", "unzipped", Some("approval_os_unzip"))
            .unwrap();
        assert_eq!(unzip.data["entry_count"].as_u64().unwrap(), 2);
        assert!(workspace
            .join("unzipped")
            .join("docs")
            .join("a.txt")
            .exists());

        runtime
            .rollback_tx(unzip.data["tx_id"].as_str().unwrap())
            .unwrap();
        assert!(!workspace.join("unzipped").exists());
        runtime
            .rollback_tx(zip.data["tx_id"].as_str().unwrap())
            .unwrap();
        assert!(!workspace.join("out").join("docs.zip").exists());
    }

    #[test]
    fn model_runtime_records_schema_valid_ledger_and_process_truth() {
        let workspace = temp_workspace("model_runtime_success");
        let (job, process, truth) = create_agent_job(&workspace, "Use model runtime").unwrap();
        let job_id = job.job_id.clone();
        let pid = process.pid.clone();
        let instruction_ref = truth
            .write_blob("model_inputs/instruction.txt", b"Extract a ledger row.")
            .unwrap();
        let source_ref = truth
            .write_blob("model_inputs/source.md", b"# Source\nOwner: Chen\n")
            .unwrap();
        let token = CapabilityToken {
            token_id: "token_model".to_string(),
            job_id: job.job_id.clone(),
            pid: process.pid.clone(),
            workspace_root: workspace.display().to_string(),
            capabilities: vec!["model.invoke".to_string(), "model.extract_json".to_string()],
            permissions: vec!["model:invoke".to_string()],
        };
        let provider = DeterministicModelProvider::new("deterministic", "phase5-model")
            .with_output(
            ModelOperation::ExtractJson,
            "{\"title\":\"Source\",\"owner\":\"Chen\",\"source_path\":\"model_inputs/source.md\"}",
        );
        let runtime = ModelRuntime::new(truth.clone(), token, std::sync::Arc::new(provider));
        let receipt = runtime
            .extract_json(ModelAction {
                action_id: "act_model_extract".to_string(),
                job_id: job_id.clone(),
                pid: pid.clone(),
                reasoning_step_id: "reason_1".to_string(),
                operation: ModelOperation::ExtractJson,
                instruction_ref,
                input_refs: vec![source_ref],
                preference_snapshot_ref: None,
                output_schema: json!({"type": "object", "required": ["title", "owner", "source_path"]}),
                provider: "deterministic".to_string(),
                model: "phase5-model".to_string(),
                budget: ModelBudget::default(),
                failure_policy: ModelFailurePolicy::FailClosed,
                required: true,
            })
            .unwrap();

        assert_eq!(receipt.status, "success");
        assert_eq!(receipt.operation, ModelOperation::ExtractJson);
        assert!(receipt
            .output_ref
            .as_deref()
            .unwrap()
            .starts_with("blob://"));
        let expected_ledger_ref = format!("model_ledger://{job_id}/model_call_ledger.json");
        assert_eq!(
            receipt.ledger_ref.as_deref(),
            Some(expected_ledger_ref.as_str())
        );
        assert!(receipt.schema_validation.schema_valid);
        assert!(receipt.no_silent_fallback_pass);
        let ledger_path = workspace
            .join(RUNTIME_DIR_NAME)
            .join("model_call_ledger")
            .join(&job_id)
            .join("model_call_ledger.json");
        assert!(ledger_path.exists());
        let ledger = fs::read_to_string(ledger_path).unwrap();
        assert!(ledger.contains("model_call_id"));
        assert!(ledger.contains("model.extract_json"));
        let event_types: Vec<String> = truth
            .read_events()
            .unwrap()
            .into_iter()
            .map(|event| event.event_type)
            .collect();
        assert!(event_types.contains(&"model_call_started".to_string()));
        assert!(event_types.contains(&"model_call_completed".to_string()));
        assert!(event_types.contains(&"model_call_ledger".to_string()));
        assert!(event_types.contains(&"model_call_receipt".to_string()));
        assert!(event_types.contains(&"capability_receipt".to_string()));
    }

    #[test]
    fn model_runtime_renders_entity_reply_with_ledger() {
        let workspace = temp_workspace("model_runtime_entity_reply");
        let (job, process, truth) =
            create_agent_job(&workspace, "Render Super Agent Entity reply").unwrap();
        let instruction_ref = truth
            .write_blob("model_inputs/entity_instruction.txt", b"Reply to user.")
            .unwrap();
        let context_ref = truth
            .write_blob(
                "model_inputs/entity_context.json",
                br#"{"next_action":"complete"}"#,
            )
            .unwrap();
        let token = CapabilityToken {
            token_id: "token_entity_model".to_string(),
            job_id: job.job_id.clone(),
            pid: process.pid.clone(),
            workspace_root: workspace.display().to_string(),
            capabilities: vec![
                "model.invoke".to_string(),
                "model.render_entity_reply".to_string(),
            ],
            permissions: vec!["model:invoke".to_string()],
        };
        let provider = DeterministicModelProvider::new("deterministic", "phase7-model")
            .with_output(
                ModelOperation::RenderEntityReply,
                "Kernel 已完成本轮任务，产物可在任务面板查看。",
            );
        let receipt = ModelRuntime::new(truth.clone(), token, std::sync::Arc::new(provider))
            .render_entity_reply(ModelAction {
                action_id: "act_entity_reply".to_string(),
                job_id: job.job_id,
                pid: process.pid,
                reasoning_step_id: "entity_render".to_string(),
                operation: ModelOperation::RenderEntityReply,
                instruction_ref,
                input_refs: vec![context_ref],
                preference_snapshot_ref: None,
                output_schema: json!({"type": "string"}),
                provider: "deterministic".to_string(),
                model: "phase7-model".to_string(),
                budget: ModelBudget::default(),
                failure_policy: ModelFailurePolicy::OptionalVisible,
                required: false,
            })
            .unwrap();

        assert_eq!(receipt.status, "success");
        assert_eq!(receipt.capability_id, "model.render_entity_reply");
        assert_eq!(receipt.operation, ModelOperation::RenderEntityReply);
        let events = truth.read_events().unwrap();
        assert!(events.iter().any(|event| {
            event.event_type == "model_call_receipt"
                && event.data["capability_id"] == "model.render_entity_reply"
        }));
    }

    #[test]
    fn model_runtime_does_not_fail_closed_on_placeholder_words() {
        let workspace = temp_workspace("model_runtime_placeholder_words");
        let (job, process, truth) = create_agent_job(&workspace, "Summarize with model").unwrap();
        let job_id = job.job_id.clone();
        let pid = process.pid.clone();
        let instruction_ref = truth
            .write_blob("model_inputs/instruction.txt", b"Summarize the source.")
            .unwrap();
        let source_ref = truth
            .write_blob(
                "model_inputs/source.md",
                b"# Source\nReal source content.\n",
            )
            .unwrap();
        let token = CapabilityToken {
            token_id: "token_model_failure".to_string(),
            job_id: job.job_id.clone(),
            pid: process.pid.clone(),
            workspace_root: workspace.display().to_string(),
            capabilities: vec!["model.summarize".to_string()],
            permissions: vec!["model:invoke".to_string()],
        };
        let provider = DeterministicModelProvider::new("deterministic", "phase5-model")
            .with_output(ModelOperation::Summarize, "\u{9879}\u{76ee}\u{6750}\u{6599}\u{663e}\u{793a}\u{6295}\u{5c4f}\u{9002}\u{914d}\u{8bf4}\u{660e}\u{5f85}\u{8865}\u{5145}\u{ff0c}\u{4f46}\u{6838}\u{5fc3}\u{9a8c}\u{6536}\u{6307}\u{6807}\u{5df2}\u{5177}\u{5907}\u{4ea4}\u{63a5}\u{6761}\u{4ef6}\u{3002}");
        let runtime = ModelRuntime::new(truth.clone(), token, std::sync::Arc::new(provider));
        let receipt = runtime
            .summarize(ModelAction {
                action_id: "act_model_summarize".to_string(),
                job_id,
                pid,
                reasoning_step_id: "reason_2".to_string(),
                operation: ModelOperation::Summarize,
                instruction_ref,
                input_refs: vec![source_ref],
                preference_snapshot_ref: None,
                output_schema: json!({"type": "string"}),
                provider: "deterministic".to_string(),
                model: "phase5-model".to_string(),
                budget: ModelBudget::default(),
                failure_policy: ModelFailurePolicy::FailClosed,
                required: true,
            })
            .unwrap();

        assert_eq!(receipt.status, "success");
        assert!(receipt.no_silent_fallback_pass);
        assert!(receipt.fallback_risks.is_empty());
        assert!(receipt.error.is_none());
        let replay = truth.replay().unwrap();
        assert_eq!(replay.status, "running");
        let event_types: Vec<String> = truth
            .read_events()
            .unwrap()
            .into_iter()
            .map(|event| event.event_type)
            .collect();
        assert!(event_types.contains(&"model_call_completed".to_string()));
        assert!(!event_types.contains(&"model_call_failed".to_string()));
        assert!(!event_types.contains(&"required_model_operation_failed".to_string()));
        assert!(!event_types.contains(&"job_failed".to_string()));
    }

    #[test]
    fn deepseek_provider_invokes_openai_compatible_endpoint() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_http_request(&mut stream);
            assert!(request.contains("Authorization: Bearer test-key"));
            assert!(request.contains("\"stream\":false"));
            let body = r#"{"choices":[{"message":{"content":"Live DeepSeek provider response."},"finish_reason":"stop"}],"usage":{"total_tokens":12}}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).unwrap();
        });

        let workspace = temp_workspace("deepseek_provider");
        let (job, process, truth) = create_agent_job(&workspace, "Live provider").unwrap();
        let instruction_ref = truth
            .write_blob("model_inputs/instruction.txt", b"Summarize.")
            .unwrap();
        let source_ref = truth
            .write_blob("model_inputs/source.txt", b"Source text.")
            .unwrap();
        let token = CapabilityToken {
            token_id: "token_deepseek".to_string(),
            job_id: job.job_id.clone(),
            pid: process.pid.clone(),
            workspace_root: workspace.display().to_string(),
            capabilities: vec!["model.summarize".to_string()],
            permissions: vec!["model:invoke".to_string()],
        };
        let provider = DeepSeekModelProvider::new(
            "test-key",
            format!("http://{}", addr),
            "deepseek-v4",
            5_000,
        )
        .with_streaming(false);
        let receipt = ModelRuntime::new(truth, token, std::sync::Arc::new(provider))
            .summarize(ModelAction {
                action_id: "act_deepseek".to_string(),
                job_id: job.job_id,
                pid: process.pid,
                reasoning_step_id: "reason_deepseek".to_string(),
                operation: ModelOperation::Summarize,
                instruction_ref,
                input_refs: vec![source_ref],
                preference_snapshot_ref: None,
                output_schema: json!({"type": "string"}),
                provider: "deepseek".to_string(),
                model: "deepseek-v4".to_string(),
                budget: ModelBudget::default(),
                failure_policy: ModelFailurePolicy::FailClosed,
                required: true,
            })
            .unwrap();
        handle.join().unwrap();

        assert_eq!(receipt.status, "success");
        assert_eq!(receipt.provider, "deepseek");
        assert_eq!(receipt.model, "deepseek-v4");
        assert_eq!(receipt.finish_reason.as_deref(), Some("stop"));
        assert!(!receipt.streaming);
    }

    #[test]
    fn deepseek_provider_sends_thinking_payload_and_records_reasoning_content() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_http_request(&mut stream);
            assert!(request.contains("\"stream\":false"));
            assert!(request.contains("\"thinking\":{\"type\":\"enabled\"}"));
            assert!(request.contains("\"reasoning_effort\":\"high\""));
            assert!(!request.contains("\"temperature\""));
            let body = include_str!("../../tests/v2/fixtures/deepseek/thinking_no_tool_call.json");
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.as_bytes().len(),
                body
            );
            stream.write_all(response.as_bytes()).unwrap();
        });

        let workspace = temp_workspace("deepseek_thinking_reasoning");
        let (job, process, truth) =
            create_agent_job(&workspace, "Capture DeepSeek reasoning").unwrap();
        let instruction_ref = truth
            .write_blob("model_inputs/instruction.txt", b"Summarize.")
            .unwrap();
        let source_ref = truth
            .write_blob("model_inputs/source.txt", b"Source text.")
            .unwrap();
        let token = CapabilityToken {
            token_id: "token_deepseek_reasoning".to_string(),
            job_id: job.job_id.clone(),
            pid: process.pid.clone(),
            workspace_root: workspace.display().to_string(),
            capabilities: vec!["model.summarize".to_string()],
            permissions: vec!["model:invoke".to_string()],
        };
        let provider = DeepSeekModelProvider::new(
            "test-key",
            format!("http://{}", addr),
            "deepseek-v4",
            5_000,
        )
        .with_streaming(false);
        let receipt = ModelRuntime::new(truth.clone(), token, std::sync::Arc::new(provider))
            .summarize(ModelAction {
                action_id: "act_deepseek_reasoning".to_string(),
                job_id: job.job_id,
                pid: process.pid,
                reasoning_step_id: "reason_deepseek_reasoning".to_string(),
                operation: ModelOperation::Summarize,
                instruction_ref,
                input_refs: vec![source_ref],
                preference_snapshot_ref: None,
                output_schema: json!({"type": "string"}),
                provider: "deepseek".to_string(),
                model: "deepseek-v4".to_string(),
                budget: ModelBudget::default(),
                failure_policy: ModelFailurePolicy::FailClosed,
                required: true,
            })
            .unwrap();
        handle.join().unwrap();

        assert_eq!(receipt.status, "success");
        assert!(receipt.sampling_ignored_by_provider);
        assert!(receipt.reasoning_content_tokens_estimated > 0);
        let reasoning_ref = receipt.reasoning_content_ref.as_deref().unwrap();
        let reasoning = fs::read_to_string(truth.resolve_blob_ref(reasoning_ref).unwrap()).unwrap();
        assert!(reasoning.contains("avoid unsupported claims"));
        let events = truth.read_events().unwrap();
        assert!(events
            .iter()
            .any(|event| event.event_type == "provider_reasoning_content_recorded"));
    }

    #[test]
    fn deepseek_provider_parses_tool_call_finish_reason_without_content_missing() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_http_request(&mut stream);
            assert!(request.contains("\"stream\":false"));
            let body =
                include_str!("../../tests/v2/fixtures/deepseek/finish_reason_tool_calls.json");
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.as_bytes().len(),
                body
            );
            stream.write_all(response.as_bytes()).unwrap();
        });

        let provider = DeepSeekModelProvider::new(
            "test-key",
            format!("http://{}", addr),
            "deepseek-v4",
            5_000,
        )
        .with_streaming(false);
        let action = ModelAction {
            action_id: "act_tool_fixture".to_string(),
            job_id: "job_tool_fixture".to_string(),
            pid: "pid_tool_fixture".to_string(),
            reasoning_step_id: "reason_tool_fixture".to_string(),
            operation: ModelOperation::DecideNextAction,
            instruction_ref: "blob://job_tool_fixture/instruction.txt".to_string(),
            input_refs: vec!["blob://job_tool_fixture/input.txt".to_string()],
            preference_snapshot_ref: None,
            output_schema: json!({"type": "object"}),
            provider: "deepseek".to_string(),
            model: "deepseek-v4".to_string(),
            budget: ModelBudget::default(),
            failure_policy: ModelFailurePolicy::FailClosed,
            required: true,
        };
        let response = provider
            .invoke(&ModelProviderRequest {
                model_call_id: "mcall_tool_fixture".to_string(),
                action,
                input_payloads: BTreeMap::from([(
                    "blob://job_tool_fixture/input.txt".to_string(),
                    "input".to_string(),
                )]),
                capability_snapshot: provider.capability_snapshot(),
                model_config: ModelInvocationConfig::default(),
                client_locale_context_ref: None,
                client_locale_context: None,
                provider_tools: Vec::new(),
                provider_tool_choice: None,
                provider_transcript_messages: Vec::new(),
                provider_toolset_ref: None,
                current_user_message_required: false,
            })
            .unwrap();
        handle.join().unwrap();

        assert_eq!(response.finish_reason.as_deref(), Some("tool_calls"));
        assert_eq!(response.tool_calls.len(), 1);
        assert!(response.output_text.is_empty());
    }

    #[test]
    fn provider_transcript_replays_tool_call_reasoning_content_and_observation_refs() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_http_request(&mut stream);
            assert!(request.contains("\"thinking\":{\"type\":\"enabled\"}"));
            let body =
                include_str!("../../tests/v2/fixtures/deepseek/thinking_with_tool_call.json");
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.as_bytes().len(),
                body
            );
            stream.write_all(response.as_bytes()).unwrap();
        });

        let workspace = temp_workspace("provider_transcript_tool_reasoning");
        let (job, process, truth) =
            create_agent_job(&workspace, "Record provider transcript").unwrap();
        let instruction_ref = truth
            .write_blob("model_inputs/instruction.txt", b"Return next action.")
            .unwrap();
        let observation_ref = truth
            .write_blob("model_inputs/observation.json", br#"{"goal":"inspect"}"#)
            .unwrap();
        let token = CapabilityToken {
            token_id: "token_provider_transcript".to_string(),
            job_id: job.job_id.clone(),
            pid: process.pid.clone(),
            workspace_root: workspace.display().to_string(),
            capabilities: vec!["model.decide_next_action".to_string()],
            permissions: vec!["model:invoke".to_string()],
        };
        let provider = DeepSeekModelProvider::new(
            "test-key",
            format!("http://{}", addr),
            "deepseek-v4",
            5_000,
        )
        .with_streaming(false);
        let receipt = ModelRuntime::new(truth.clone(), token, std::sync::Arc::new(provider))
            .decide_next_action(ModelAction {
                action_id: "act_provider_transcript".to_string(),
                job_id: job.job_id.clone(),
                pid: process.pid.clone(),
                reasoning_step_id: "reason_provider_transcript".to_string(),
                operation: ModelOperation::DecideNextAction,
                instruction_ref,
                input_refs: vec![observation_ref],
                preference_snapshot_ref: None,
                output_schema: json!({"type": "object"}),
                provider: "deepseek".to_string(),
                model: "deepseek-v4".to_string(),
                budget: ModelBudget::default(),
                failure_policy: ModelFailurePolicy::OptionalVisible,
                required: false,
            })
            .unwrap();
        handle.join().unwrap();

        assert_eq!(receipt.status, "failed");
        let assistant_transcript_ref = receipt.provider_transcript_ref.as_deref().unwrap();
        assert!(receipt.provider_transcript_summary_ref.is_some());
        let messages: Vec<ProviderTranscriptMessage> = serde_json::from_slice(
            &fs::read(truth.resolve_blob_ref(assistant_transcript_ref).unwrap()).unwrap(),
        )
        .unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, "assistant");
        assert!(messages[0]
            .reasoning_content
            .as_deref()
            .unwrap()
            .contains("workspace read"));
        assert_eq!(messages[0].tool_calls.len(), 1);
        assert_eq!(messages[0].tool_calls[0].id, "call_inventory");

        let tool_record = record_provider_tool_result(
            &truth,
            &process.pid,
            "deepseek",
            "deepseek_chat_completions",
            "call_inventory",
            &json!({"status": "success", "receipt_ref": "receipt://read_file"}),
        )
        .unwrap();
        assert!(tool_record
            .tool_message_ref
            .contains("provider_transcripts/"));
        assert!(tool_record
            .tool_message_ref
            .contains("tool_call_inventory.json"));
        let transcript_ref = tool_record.messages_ref.clone();
        let transcript_summary_ref = tool_record.summary_ref.clone();
        let messages: Vec<ProviderTranscriptMessage> = serde_json::from_slice(
            &fs::read(truth.resolve_blob_ref(&transcript_ref).unwrap()).unwrap(),
        )
        .unwrap();
        assert_eq!(messages.len(), 2);
        assert!(messages[0]
            .reasoning_content
            .as_deref()
            .unwrap()
            .contains("workspace read"));
        assert_eq!(messages[1].role, "tool");
        assert_eq!(messages[1].tool_call_id.as_deref(), Some("call_inventory"));
        assert!(messages[1]
            .content
            .as_deref()
            .unwrap()
            .contains("receipt_ref"));

        let transcript_state =
            replay_provider_transcript_state(&truth, "deepseek", "deepseek_chat_completions")
                .unwrap()
                .unwrap();
        assert_eq!(transcript_state.messages_ref, transcript_ref);
        assert_eq!(
            transcript_state.summary_ref.as_deref(),
            Some(transcript_summary_ref.as_str())
        );
        assert_eq!(transcript_state.reasoning_content_refs.len(), 1);
        assert!(transcript_state.pending_tool_calls.is_empty());

        let context =
            replay_task_context_state(&truth, &process.pid, "tar_provider_transcript").unwrap();
        assert_eq!(
            context.provider_transcript_ref.as_deref(),
            Some(transcript_ref.as_str())
        );
        assert_eq!(
            context.provider_transcript_summary_ref.as_deref(),
            Some(transcript_summary_ref.as_str())
        );
        assert!(context
            .provider_transcript_refs
            .iter()
            .any(|item| item == &transcript_ref));

        let observation = ObservationBuilder::default_registry(truth.clone())
            .build(
                &process.pid,
                "tar_provider_transcript",
                "Record provider transcript",
            )
            .unwrap();
        assert_eq!(
            observation.provider_transcript_ref.as_deref(),
            Some(transcript_ref.as_str())
        );
        assert_eq!(
            observation.provider_transcript_summary_ref.as_deref(),
            Some(transcript_summary_ref.as_str())
        );
        let frame: RawObservationFrame = serde_json::from_slice(
            &fs::read(
                truth
                    .resolve_blob_ref(&observation.observation_frame_ref)
                    .unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            frame.provider_transcript_ref.as_deref(),
            Some(transcript_ref.as_str())
        );
        assert_eq!(
            frame.provider_transcript_summary_ref.as_deref(),
            Some(transcript_summary_ref.as_str())
        );
    }

    #[test]
    fn deepseek_phase4_native_tool_call_executes_read_file_and_replays_tool_result() {
        let workspace = temp_workspace("deepseek_phase4_read_file");
        fs::create_dir_all(workspace.join("docs")).unwrap();
        fs::write(
            workspace.join("docs").join("source.txt"),
            "phase4 source content",
        )
        .unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let (mut first, _) = listener.accept().unwrap();
            let request = read_http_request(&mut first);
            assert!(request.contains("\"stream\":false"));
            assert!(request.contains("\"tools\""));
            assert!(!request.contains("\"tool_choice\""));
            assert!(request.contains("\"cap_os_read_file\""));
            assert!(request.contains("\"cap_process_complete\""));
            assert!(request.contains("[Stable Kernel Contract]"));
            assert!(request.contains("[Toolset Index]"));
            assert!(request.contains("[Current Toolset]"));
            assert!(request.contains("Do not return a SuperNova JSON decision object"));
            assert!(!request.contains("Return only JSON"));
            let body = deepseek_tool_call_body(
                "call_read",
                "cap_os_read_file",
                json!({"path": "docs/source.txt", "reason": "Read the source file."}),
                "Need workspace read before completing.",
            );
            write_json_response(&mut first, &body);

            let (mut second, _) = listener.accept().unwrap();
            let request = read_http_request(&mut second);
            assert!(request.contains("\"role\":\"assistant\""));
            assert!(request.contains("Need workspace read before completing."));
            assert!(request.contains("\"role\":\"tool\""));
            assert!(request.contains("call_read"));
            assert!(request.contains("dataset_ref"));
            let body = deepseek_tool_call_body(
                "call_complete",
                "cap_process_complete",
                json!({
                    "completion_statement": "Read docs/source.txt through the Kernel and confirmed its content.",
                    "claimed_artifacts": [],
                    "key_sources": ["docs/source.txt"],
                    "known_limitations": [],
                    "user_review_notes": []
                }),
                "The read result is enough to close this read-only task.",
            );
            write_json_response(&mut second, &body);
        });

        let provider = DeepSeekModelProvider::new(
            "test-key",
            format!("http://{}", addr),
            "deepseek-v4",
            5_000,
        )
        .with_streaming(true);
        let controller =
            RootAgentProcessController::with_model_provider(&workspace, Arc::new(provider))
                .unwrap();
        let result = controller
            .start_job_with_config(
                "Read docs/source.txt and report that it was read",
                Some(1),
                phase4_native_tool_config(),
            )
            .unwrap();
        handle.join().unwrap();

        assert_eq!(result.status, "completed");
        assert_eq!(result.turn_count, 1);
        let truth = ProcessTruthStore::new(&workspace, &result.job_id).unwrap();
        let events = truth.read_events().unwrap();
        assert!(events.iter().any(|event| {
            event.event_type == "process_action_validated"
                && event.data["capability_id"] == "os.read_file"
        }));
        assert!(events.iter().any(|event| {
            event.event_type == "capability_receipt"
                && event.data["capability_id"] == "os.read_file"
                && event.data["status"] == "success"
        }));
        assert!(events.iter().any(|event| {
            event.event_type == "provider_tool_result_recorded"
                && event.data["provider_tool_call_id"] == "call_read"
        }));
        assert!(events.iter().any(|event| {
            event.event_type == "provider_tool_result_recorded"
                && event.data["provider_tool_call_id"] == "call_complete"
        }));
        assert!(events
            .iter()
            .any(|event| event.event_type == "job_completed"));
    }

    #[test]
    fn task_initial_context_pack_is_attached_to_model_action_input_refs() {
        let workspace = temp_workspace("task_context_pack_model_input");
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_http_request(&mut stream);
            assert!(request.contains("context_pack_test"));
            let body = deepseek_tool_call_body(
                "call_complete",
                "cap_process_complete",
                json!({
                    "completion_statement": "Context pack was visible to the model request.",
                    "claimed_artifacts": [],
                    "key_sources": [],
                    "known_limitations": [],
                    "user_review_notes": []
                }),
                "The context pack is enough to answer.",
            );
            write_json_response(&mut stream, &body);
        });

        let provider = DeepSeekModelProvider::new(
            "test-key",
            format!("http://{}", addr),
            "deepseek-v4",
            5_000,
        )
        .with_streaming(true);
        let controller =
            RootAgentProcessController::with_model_provider(&workspace, Arc::new(provider))
                .unwrap();
        let initial_context = json!({
            "schema": "supernova_container_task_initial_context.v1",
            "container_id": "container_test",
            "context_pack_id": "context_pack_test",
            "visible_context_pack": {
                "schema": "supernova_context_pack_visible_payload.v1",
                "items": [{
                    "label": "Previous task summary",
                    "content": "Use this context before scanning the workspace."
                }]
            },
            "auto_approve": false,
            "goal": "Use the provided context pack"
        });
        let result = controller
            .start_job_with_config_and_initial_context(
                "Use the provided context pack",
                Some(1),
                phase4_native_tool_config(),
                Some(initial_context),
            )
            .unwrap();
        handle.join().unwrap();

        assert_eq!(result.status, "completed");
        let truth = ProcessTruthStore::new(&workspace, &result.job_id).unwrap();
        let events = truth.read_events().unwrap();
        let context_ref = events
            .iter()
            .find(|event| event.event_type == "task_initial_context_bound")
            .and_then(|event| event.data.get("context_ref"))
            .and_then(Value::as_str)
            .expect("task initial context ref recorded");
        let model_action = events
            .iter()
            .find(|event| event.event_type == "model_action_emitted")
            .expect("model action emitted");
        let input_refs = model_action
            .data
            .get("input_refs")
            .and_then(Value::as_array)
            .expect("model action input refs");
        assert!(input_refs
            .iter()
            .any(|value| value.as_str() == Some(context_ref)));
        assert!(events.iter().any(|event| {
            event.event_type == "task_context_pack_model_input_attached"
                && event.data["context_pack_id"] == "context_pack_test"
        }));
    }

    #[test]
    fn deepseek_native_client_env_scan_records_process_truth_receipt() {
        let workspace = temp_workspace("deepseek_client_env_scan");
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let (mut first, _) = listener.accept().unwrap();
            let request = read_http_request(&mut first);
            assert!(request.contains("cap_client_env_scan_overview"));
            assert!(request.contains("[Client Context]"));
            let body = deepseek_tool_call_body(
                "call_env",
                "cap_client_env_scan_overview",
                json!({
                    "sections": ["device", "storage", "network", "runtimes"],
                    "detail_level": "summary",
                    "include_sensitive_fields": false,
                    "reason": "Collect sanitized environment facts."
                }),
                "Need sanitized client environment facts before answering.",
            );
            write_json_response(&mut first, &body);

            let (mut second, _) = listener.accept().unwrap();
            let request = read_http_request(&mut second);
            assert!(request.contains("client_env.scan_overview"));
            assert!(request.contains("snapshot_ref"));
            let body = deepseek_tool_call_body(
                "call_complete",
                "cap_process_complete",
                json!({
                    "completion_statement": "Collected sanitized client environment readiness facts through client_env.scan_overview.",
                    "claimed_artifacts": [],
                    "key_sources": ["client_env.scan_overview receipt"],
                    "known_limitations": [],
                    "user_review_notes": []
                }),
                "The sanitized Env receipt is enough to close this read-only task.",
            );
            write_json_response(&mut second, &body);
        });

        let provider = DeepSeekModelProvider::new(
            "test-key",
            format!("http://{}", addr),
            "deepseek-v4",
            5_000,
        )
        .with_streaming(false);
        let mut config = phase4_native_tool_config();
        config.tool_calling.max_provider_subturns = 4;
        let controller =
            RootAgentProcessController::with_model_provider(&workspace, Arc::new(provider))
                .unwrap();
        let result = controller
            .start_job_with_config(
                "Inspect sanitized local environment readiness",
                Some(1),
                config,
            )
            .unwrap();
        handle.join().unwrap();

        assert_eq!(result.status, "completed");
        let truth = ProcessTruthStore::new(&workspace, &result.job_id).unwrap();
        let events = truth.read_events().unwrap();
        assert!(events.iter().any(|event| {
            event.event_type == "process_action_validated"
                && event.data["capability_id"] == "client_env.scan_overview"
        }));
        assert!(events.iter().any(|event| {
            event.event_type == "capability_receipt"
                && event.data["capability_id"] == "client_env.scan_overview"
                && event.data["status"] == "success"
                && event.data["data"]["snapshot_ref"].as_str().is_some()
        }));
        assert!(events.iter().any(|event| {
            event.event_type == "model_call_started"
                && event
                    .data
                    .get("client_locale_context_ref")
                    .and_then(Value::as_str)
                    .is_some()
        }));
    }

    #[test]
    fn deepseek_native_client_env_sensitive_scan_blocks_without_authorization() {
        let workspace = temp_workspace("deepseek_client_env_sensitive_block");
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let (mut first, _) = listener.accept().unwrap();
            let request = read_http_request(&mut first);
            assert!(request.contains("cap_client_env_scan_network"));
            let body = deepseek_tool_call_body(
                "call_network_sensitive",
                "cap_client_env_scan_network",
                json!({
                    "include_sensitive_fields": true,
                    "reason": "User requested IP/MAC."
                }),
                "Need to check whether sensitive network fields can be disclosed.",
            );
            write_json_response(&mut first, &body);

            let (mut second, _) = listener.accept().unwrap();
            let request = read_http_request(&mut second);
            assert!(request.contains("requires_explicit_user_authorization"));
            assert!(request.contains("network.local_ip"));
            assert!(request.contains("network.mac_address"));
            let body = deepseek_tool_call_body(
                "call_fail",
                "cap_process_fail",
                json!({
                    "reason": "Local IP/MAC require explicit client-env disclosure authorization before they can be used in a report."
                }),
                "The task cannot disclose sensitive fields without explicit authorization.",
            );
            write_json_response(&mut second, &body);
        });

        let provider = DeepSeekModelProvider::new(
            "test-key",
            format!("http://{}", addr),
            "deepseek-v4",
            5_000,
        )
        .with_streaming(false);
        let mut config = phase4_native_tool_config();
        config.tool_calling.max_provider_subturns = 4;
        config.tool_calling.toolset_mode = ProviderToolsetMode::FullRegistered;
        let controller =
            RootAgentProcessController::with_model_provider(&workspace, Arc::new(provider))
                .unwrap();
        let result = controller
            .start_job_with_config("生成一份包含本机 IP 和 MAC 的环境报告。", Some(1), config)
            .unwrap();
        handle.join().unwrap();

        assert_eq!(result.status, "failed");
        let truth = ProcessTruthStore::new(&workspace, &result.job_id).unwrap();
        let events = truth.read_events().unwrap();
        assert!(events.iter().any(|event| {
            event.event_type == "capability_receipt"
                && event.data["capability_id"] == "client_env.scan_network"
                && event.data["status"] == "blocked"
                && event.data["data"]["requires_explicit_user_authorization"] == true
                && event.data["data"]["no_sensitive_values_returned"] == true
        }));
        assert!(!events.iter().any(|event| {
            event.event_type == "artifact_materialized"
                || (event.event_type == "capability_receipt"
                    && event.data["capability_id"] == "os.write_artifact"
                    && event.data["status"] == "success")
        }));
    }

    #[test]
    fn deepseek_phase4_blocks_unregistered_and_model_provider_tools() {
        let config = phase4_native_tool_config();
        let registry =
            ProviderToolRegistry::phase4_readonly(&default_capability_registry(), &config);
        assert!(registry.binding_for_tool_name("cap_os_read_file").is_some());
        assert!(registry
            .binding_for_tool_name("cap_model_summarize")
            .is_none());

        let unknown = ProviderToolCall {
            id: "call_unknown".to_string(),
            r#type: "function".to_string(),
            function: json!({
                "name": "cap_unknown_escape_hatch",
                "arguments": "{}"
            }),
        };
        let err = registry.decision_for_tool_call(&unknown).unwrap_err();
        assert_eq!(err.error_code, "PROVIDER_TOOL_FUNCTION_UNREGISTERED");

        let model_call = ProviderToolCall {
            id: "call_model".to_string(),
            r#type: "function".to_string(),
            function: json!({
                "name": "cap_model_summarize",
                "arguments": "{}"
            }),
        };
        let err = registry.decision_for_tool_call(&model_call).unwrap_err();
        assert_eq!(err.error_code, "PROVIDER_TOOL_MODEL_CAPABILITY_FORBIDDEN");
    }

    #[test]
    fn deepseek_phase4_unregistered_tool_records_protocol_error() {
        let workspace = temp_workspace("deepseek_phase4_unknown_tool");
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_http_request(&mut stream);
            assert!(request.contains("\"tools\""));
            let body = deepseek_tool_call_body(
                "call_unknown",
                "cap_unknown_escape_hatch",
                json!({}),
                "Try an unregistered function.",
            );
            write_json_response(&mut stream, &body);

            let (mut second, _) = listener.accept().unwrap();
            let request = read_http_request(&mut second);
            assert!(request.contains("call_unknown"));
            assert!(request.contains("PROVIDER_TOOL_FUNCTION_UNREGISTERED"));
            let body = deepseek_tool_call_body(
                "call_fail_after_unknown",
                "cap_process_fail",
                json!({
                    "error_code": "UNREGISTERED_TOOL_CORRECTED",
                    "reason": "The previous provider tool name was unavailable; fail explicitly."
                }),
                "Fail after observing the tool error.",
            );
            write_json_response(&mut second, &body);
        });
        let provider = DeepSeekModelProvider::new(
            "test-key",
            format!("http://{}", addr),
            "deepseek-v4",
            5_000,
        )
        .with_streaming(false);
        let controller =
            RootAgentProcessController::with_model_provider(&workspace, Arc::new(provider))
                .unwrap();
        let result = controller
            .start_job_with_config(
                "Exercise unknown provider tool",
                Some(1),
                phase4_native_tool_config(),
            )
            .unwrap();
        handle.join().unwrap();

        assert_eq!(result.status, "failed");
        let truth = ProcessTruthStore::new(&workspace, &result.job_id).unwrap();
        let events = truth.read_events().unwrap();
        assert!(events.iter().any(|event| {
            event.event_type == "provider_tool_protocol_error"
                && event.data["error_code"] == "PROVIDER_TOOL_FUNCTION_UNREGISTERED"
        }));
        assert!(events.iter().any(|event| {
            event.event_type == "provider_tool_result_recorded"
                && event.data["provider_tool_call_id"] == "call_unknown"
        }));
        assert!(events.iter().any(|event| {
            event.event_type == "provider_tool_call_recoverable_error"
                && event.data["error_code"] == "PROVIDER_TOOL_FUNCTION_UNREGISTERED"
        }));
    }

    #[test]
    fn deepseek_phase4_content_only_is_absorbed_without_task_completion() {
        let workspace = temp_workspace("deepseek_phase4_final_bypass");
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let (mut first, _) = listener.accept().unwrap();
            let first_request = read_http_request(&mut first);
            assert!(!first_request.contains("\"tool_choice\""));
            let first_body = json!({
                "choices": [{
                    "message": {
                        "role": "assistant",
                        "content": "Attempt to finish without calling process.complete.",
                        "reasoning_content": "Try to finish without process.complete."
                    },
                    "finish_reason": "stop"
                }],
                "usage": {"prompt_tokens": 10, "completion_tokens": 4}
            })
            .to_string();
            write_json_response(&mut first, &first_body);

            let (mut second, _) = listener.accept().unwrap();
            let second_request = read_http_request(&mut second);
            assert!(second_request.contains("attempt bypass"));
            let complete_body = deepseek_tool_call_body(
                "call_complete_after_content_only",
                "cap_process_complete",
                json!({
                    "completion_statement": "Closed only through process.complete.",
                    "claimed_artifacts": [],
                    "key_sources": [],
                    "known_limitations": [],
                    "user_review_notes": []
                }),
                "Use the required closure tool after content-only content was absorbed.",
            );
            write_json_response(&mut second, &complete_body);
        });
        let provider = DeepSeekModelProvider::new(
            "test-key",
            format!("http://{}", addr),
            "deepseek-v4",
            5_000,
        )
        .with_streaming(false);
        let controller =
            RootAgentProcessController::with_model_provider(&workspace, Arc::new(provider))
                .unwrap();
        let result = controller
            .start_job_with_config(
                "Do not allow assistant content to bypass process.complete",
                Some(1),
                phase4_native_tool_config(),
            )
            .unwrap();
        handle.join().unwrap();

        assert_eq!(result.status, "completed");
        let truth = ProcessTruthStore::new(&workspace, &result.job_id).unwrap();
        let events = truth.read_events().unwrap();
        assert!(events.iter().any(|event| {
            event.event_type == "provider_native_assistant_content_yielded"
                && event.data["closure_allowed"] == false
                && event.data["assistant_content"]
                    .as_str()
                    .is_some_and(|value| value.contains("attempt bypass"))
        }));
        assert!(events
            .iter()
            .any(|event| event.event_type == "job_completed"));
    }

    #[test]
    fn deepseek_native_process_complete_uses_kernel_complete_without_provider_preflight() {
        let workspace = temp_workspace("deepseek_complete_kernel_gate");
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let (mut first, _) = listener.accept().unwrap();
            let first_request = read_http_request(&mut first);
            assert!(first_request.contains("\"tools\""));
            let complete_body = deepseek_tool_call_body(
                "call_complete_missing",
                "cap_process_complete",
                json!({
                    "completion_statement": "missing.md is ready.",
                    "claimed_artifacts": ["missing.md"],
                    "key_sources": [],
                    "known_limitations": [],
                    "user_review_notes": []
                }),
                "Try to close through process.complete.",
            );
            write_json_response(&mut first, &complete_body);

            let (mut second, _) = listener.accept().unwrap();
            let second_request = read_http_request(&mut second);
            assert!(second_request.contains("call_complete_missing"));
            assert!(second_request.contains("claimed_artifact_hard_block"));
            assert!(second_request.contains("missing.md"));
            assert!(second_request.contains("receipt_status"));
            assert!(second_request.contains("receipt_ref"));
            let removed_preflight_event =
                ["provider_native", "completion_quality", "preflight"].join("_");
            assert!(!second_request.contains(&removed_preflight_event));
            let fail_body = deepseek_tool_call_body(
                "call_fail_after_complete_block",
                "cap_process_fail",
                json!({
                    "error_code": "COMPLETE_BLOCKED_AS_EXPECTED",
                    "reason": "Observed process.complete receipt and failed explicitly for this test."
                }),
                "Fail after observing the Kernel process.complete receipt.",
            );
            write_json_response(&mut second, &fail_body);
        });

        let provider = DeepSeekModelProvider::new(
            "test-key",
            format!("http://{}", addr),
            "deepseek-v4",
            5_000,
        )
        .with_streaming(false);
        let controller =
            RootAgentProcessController::with_model_provider(&workspace, Arc::new(provider))
                .unwrap();
        let result = controller
            .start_job_with_config(
                "Complete with a missing artifact through native provider tools",
                Some(1),
                phase4_native_tool_config(),
            )
            .unwrap();
        handle.join().unwrap();

        assert_eq!(result.status, "failed");
        let truth = ProcessTruthStore::new(&workspace, &result.job_id).unwrap();
        let events = truth.read_events().unwrap();
        assert!(events.iter().any(|event| {
            event.event_type == "process_action_emitted"
                && event.data["capability_id"] == "process.complete"
        }));
        assert!(events.iter().any(|event| {
            event.event_type == "completion_blocked"
                && event.data["reason"] == "claimed_artifact_hard_block"
        }));
        assert!(events.iter().any(|event| {
            event.event_type == "capability_receipt"
                && event.data["capability_id"] == "process.complete"
                && event.data["data"]["reason"] == "claimed_artifact_hard_block"
        }));
        let removed_preflight_event =
            ["provider_native", "completion_quality", "preflight"].join("_");
        assert!(!events
            .iter()
            .any(|event| event.event_type == removed_preflight_event));
    }

    #[test]
    fn deepseek_native_root_path_validation_error_is_recoverable() {
        let workspace = temp_workspace("deepseek_recover_root_path");
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let (mut first, _) = listener.accept().unwrap();
            let first_request = read_http_request(&mut first);
            assert!(first_request.contains("\"cap_source_set_create\""));
            let body = deepseek_tool_call_body(
                "call_bad_root",
                "cap_source_set_create",
                json!({"root_path": "/", "reason": "Scan the workspace root."}),
                "Try an invalid rooted workspace path.",
            );
            write_json_response(&mut first, &body);

            let (mut second, _) = listener.accept().unwrap();
            let second_request = read_http_request(&mut second);
            assert!(second_request.contains("call_bad_root"));
            assert!(second_request
                .contains("PROVIDER_NATIVE_SOURCE_SET_ROOT_PATH_NOT_WORKSPACE_SCOPED"));
            assert!(second_request.contains("provider_native_corrective_instruction"));
            assert!(second_request.contains("\\\"root_path\\\":\\\".\\\""));
            let fail_body = deepseek_tool_call_body(
                "call_fail_after_root_correction",
                "cap_process_fail",
                json!({
                    "error_code": "ROOT_PATH_RECOVERY_OBSERVED",
                    "reason": "Observed recoverable root_path correction."
                }),
                "Fail after observing the corrective message.",
            );
            write_json_response(&mut second, &fail_body);
        });

        let provider = DeepSeekModelProvider::new(
            "test-key",
            format!("http://{}", addr),
            "deepseek-v4",
            5_000,
        )
        .with_streaming(false);
        let controller =
            RootAgentProcessController::with_model_provider(&workspace, Arc::new(provider))
                .unwrap();
        let result = controller
            .start_job_with_config(
                "Recover from source_set.create root path error",
                Some(1),
                phase4_native_tool_config(),
            )
            .unwrap();
        handle.join().unwrap();

        assert_eq!(result.status, "failed");
        let truth = ProcessTruthStore::new(&workspace, &result.job_id).unwrap();
        let events = truth.read_events().unwrap();
        assert!(events.iter().any(|event| {
            event.event_type == "provider_tool_call_recoverable_error"
                && event.data["error_code"]
                    == "PROVIDER_NATIVE_SOURCE_SET_ROOT_PATH_NOT_WORKSPACE_SCOPED"
        }));
        assert!(events.iter().any(|event| {
            event.event_type == "provider_user_control_message_recorded"
                && event.data["control_kind"] == "recoverable_tool_error_correction"
        }));
        assert!(!events
            .iter()
            .any(|event| event.event_type == "job_interrupted_by_model_protocol_error"));
    }

    #[test]
    fn deepseek_native_raw_tool_result_path_validation_error_is_recoverable() {
        let workspace = temp_workspace("deepseek_recover_raw_tool_path");
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let (mut first, _) = listener.accept().unwrap();
            let first_request = read_http_request(&mut first);
            assert!(first_request.contains("\"cap_os_read_file\""));
            let body = deepseek_tool_call_body(
                "call_raw_result_as_path",
                "cap_os_read_file",
                json!({
                    "path": "/raw_tool_results/workspace_inventory/workspace_map.md",
                    "reason": "Read the inventory markdown."
                }),
                "Mistake a raw result for a workspace path.",
            );
            write_json_response(&mut first, &body);

            let (mut second, _) = listener.accept().unwrap();
            let second_request = read_http_request(&mut second);
            assert!(second_request.contains("call_raw_result_as_path"));
            assert!(second_request
                .contains("PROVIDER_NATIVE_RAW_TOOL_RESULT_PATH_USED_AS_WORKSPACE_PATH"));
            assert!(second_request.contains("process.read_ref"));
            assert!(second_request.contains("tool.result.page"));
            let fail_body = deepseek_tool_call_body(
                "call_fail_after_raw_path_correction",
                "cap_process_fail",
                json!({
                    "error_code": "RAW_TOOL_RESULT_PATH_RECOVERY_OBSERVED",
                    "reason": "Observed recoverable raw tool result path correction."
                }),
                "Fail after observing the corrective message.",
            );
            write_json_response(&mut second, &fail_body);
        });

        let provider = DeepSeekModelProvider::new(
            "test-key",
            format!("http://{}", addr),
            "deepseek-v4",
            5_000,
        )
        .with_streaming(false);
        let controller =
            RootAgentProcessController::with_model_provider(&workspace, Arc::new(provider))
                .unwrap();
        let result = controller
            .start_job_with_config(
                "Recover from raw_tool_results path misuse",
                Some(1),
                phase4_native_tool_config(),
            )
            .unwrap();
        handle.join().unwrap();

        assert_eq!(result.status, "failed");
        let truth = ProcessTruthStore::new(&workspace, &result.job_id).unwrap();
        let events = truth.read_events().unwrap();
        assert!(events.iter().any(|event| {
            event.event_type == "provider_tool_call_recoverable_error"
                && event.data["error_code"]
                    == "PROVIDER_NATIVE_RAW_TOOL_RESULT_PATH_USED_AS_WORKSPACE_PATH"
        }));
        assert!(events.iter().any(|event| {
            event.event_type == "provider_user_control_message_recorded"
                && event.data["control_kind"] == "recoverable_tool_error_correction"
        }));
        assert!(!events
            .iter()
            .any(|event| event.event_type == "job_interrupted_by_model_protocol_error"));
    }

    #[test]
    fn deepseek_native_fabricated_source_set_ref_validation_error_is_recoverable() {
        let workspace = temp_workspace("deepseek_recover_source_set_ref");
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let (mut first, _) = listener.accept().unwrap();
            let first_request = read_http_request(&mut first);
            assert!(first_request.contains("\"cap_source_set_create\""));
            let body = deepseek_tool_call_body(
                "call_create_source_set",
                "cap_source_set_create",
                json!({
                    "root_path": ".",
                    "include_extensions": [".docx"],
                    "reason": "Create a legitimate source set first."
                }),
                "Create the SourceSet.",
            );
            write_json_response(&mut first, &body);

            let (mut second, _) = listener.accept().unwrap();
            let second_request = read_http_request(&mut second);
            assert!(second_request.contains("call_create_source_set"));
            assert!(second_request.contains("source_set_ref"));
            assert!(second_request.contains("\"cap_office_docx_batch_read_text\""));
            let body = deepseek_tool_call_body(
                "call_fabricated_source_set",
                "cap_office_docx_batch_read_text",
                json!({
                    "source_set_ref": "blob://job_fabricated/source_sets/sourceset_job_fabricated.json",
                    "reason": "Read all DOCX files from a guessed SourceSet ref."
                }),
                "Guess a source_set_ref instead of copying it from a tool result.",
            );
            write_json_response(&mut second, &body);

            let (mut third, _) = listener.accept().unwrap();
            let third_request = read_http_request(&mut third);
            assert!(third_request.contains("call_fabricated_source_set"));
            assert!(third_request.contains("PROVIDER_NATIVE_SOURCE_SET_REF_INVALID_OR_FABRICATED"));
            assert!(third_request.contains("latest_valid_source_set_refs"));
            assert!(third_request.contains("source_set.create"));
            let fail_body = deepseek_tool_call_body(
                "call_fail_after_source_set_ref_correction",
                "cap_process_fail",
                json!({
                    "error_code": "SOURCE_SET_REF_RECOVERY_OBSERVED",
                    "reason": "Observed recoverable source_set_ref correction."
                }),
                "Fail after observing the corrective message.",
            );
            write_json_response(&mut third, &fail_body);
        });

        let provider = DeepSeekModelProvider::new(
            "test-key",
            format!("http://{}", addr),
            "deepseek-v4",
            5_000,
        )
        .with_streaming(false);
        let controller =
            RootAgentProcessController::with_model_provider(&workspace, Arc::new(provider))
                .unwrap();
        let result = controller
            .start_job_with_config(
                "Recover from fabricated source_set_ref",
                Some(1),
                phase4_native_tool_config(),
            )
            .unwrap();
        handle.join().unwrap();

        assert_eq!(result.status, "failed");
        let truth = ProcessTruthStore::new(&workspace, &result.job_id).unwrap();
        let events = truth.read_events().unwrap();
        assert!(events.iter().any(|event| {
            event.event_type == "provider_tool_call_recoverable_error"
                && event.data["error_code"]
                    == "PROVIDER_NATIVE_SOURCE_SET_REF_INVALID_OR_FABRICATED"
        }));
        assert!(events.iter().any(|event| {
            event.event_type == "provider_user_control_message_recorded"
                && event.data["control_kind"] == "recoverable_tool_error_correction"
        }));
        assert!(!events
            .iter()
            .any(|event| event.event_type == "job_interrupted_by_model_protocol_error"));
    }

    #[test]
    #[ignore = "RC0 run-through disables provider-native delete approval blocking."]
    fn deepseek_native_unapproved_delete_intent_is_kernel_preview_and_approval_executes_original_tool_call(
    ) {
        let workspace = temp_workspace("deepseek_kernel_owned_delete_approval");
        fs::write(workspace.join("old.md"), "old").unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let (mut first, _) = listener.accept().unwrap();
            let first_request = read_http_request(&mut first);
            assert!(first_request.contains("\"cap_os_delete_path\""));
            assert!(!first_request.contains("\"cap_process_request_preview\""));
            assert!(!first_request.contains("\"cap_process_preview_create\""));
            assert!(!first_request.contains("\"cap_process_pending_approvals\""));
            let body = deepseek_tool_call_body(
                "call_delete_old_md",
                "cap_os_delete_path",
                json!({
                    "path": "old.md",
                    "reason": "Delete the explicitly requested file."
                }),
                "Delete the explicitly requested file.",
            );
            write_json_response(&mut first, &body);

            let (mut second, _) = listener.accept().unwrap();
            let second_request = read_http_request(&mut second);
            assert!(second_request.contains("call_delete_old_md"));
            assert!(second_request.contains("receipt_status"));
            assert!(second_request.contains("success"));
            assert!(second_request.contains("capability_id"));
            assert!(second_request.contains("os.delete_path"));
            let complete_body = deepseek_tool_call_body(
                "call_complete_after_delete",
                "cap_process_complete",
                json!({
                    "answer": "Deleted old.md.",
                    "reason": "The approved delete receipt succeeded."
                }),
                "Complete after the approved delete receipt.",
            );
            write_json_response(&mut second, &complete_body);
        });

        let provider = DeepSeekModelProvider::new(
            "test-key",
            format!("http://{}", addr),
            "deepseek-v4",
            5_000,
        )
        .with_streaming(false);
        let controller =
            RootAgentProcessController::with_model_provider(&workspace, Arc::new(provider))
                .unwrap();
        let mut config = phase4_native_tool_config();
        config.tool_calling.toolset_mode = ProviderToolsetMode::FullRegistered;
        let result = controller
            .start_job_with_config("Delete old.md", Some(1), config)
            .unwrap();
        assert_eq!(result.status, "waiting_approval");
        assert!(workspace.join("old.md").exists());
        let approved = controller
            .approve_preview(&result.job_id, "approved delete old.md")
            .unwrap();
        handle.join().unwrap();

        assert_eq!(approved.status, "completed");
        assert!(!workspace.join("old.md").exists());
        let truth = ProcessTruthStore::new(&workspace, &result.job_id).unwrap();
        let events = truth.read_events().unwrap();
        assert!(events.iter().any(|event| {
            event.event_type == "provider_tool_call_waiting_approval"
                && event.data["capability_id"] == "os.delete_path"
                && event.data["pending_provider_tool_result"] == true
        }));
        assert!(events.iter().any(|event| {
            event.event_type == "provider_tool_call_approval_executed"
                && event.data["capability_id"] == "os.delete_path"
                && event.data["receipt_status"] == "success"
        }));
        assert!(!events.iter().any(|event| {
            event.event_type == "provider_terminal_tool_call_waiting_approval"
                || event.event_type == "provider_terminal_tool_call_approval_executed"
        }));
        assert!(!events
            .iter()
            .any(|event| event.event_type == "job_interrupted_by_model_protocol_error"));
        assert!(!events.iter().any(|event| event.event_type == "job_blocked"));
    }

    #[test]
    fn deepseek_native_generic_provider_tool_argument_error_is_recoverable() {
        let workspace = temp_workspace("deepseek_recover_generic_provider_args");
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let (mut first, _) = listener.accept().unwrap();
            let first_request = read_http_request(&mut first);
            assert!(first_request.contains("\"cap_os_read_file\""));
            let body = deepseek_tool_call_body(
                "call_read_missing_path",
                "cap_os_read_file",
                json!({"reason": "Read a file but omit path."}),
                "Omit a required provider tool argument.",
            );
            write_json_response(&mut first, &body);

            let (mut second, _) = listener.accept().unwrap();
            let second_request = read_http_request(&mut second);
            assert!(second_request.contains("call_read_missing_path"));
            assert!(second_request.contains("PROVIDER_NATIVE_TOOL_ARGUMENTS_INVALID"));
            assert!(second_request.contains("tool_schema_summary"));
            assert!(second_request.contains("\\\"required\\\":[\\\"path\\\"]"));
            let fail_body = deepseek_tool_call_body(
                "call_fail_after_generic_arg_correction",
                "cap_process_fail",
                json!({
                    "error_code": "GENERIC_ARG_RECOVERY_OBSERVED",
                    "reason": "Observed recoverable generic provider argument correction."
                }),
                "Fail after observing the recoverable argument result.",
            );
            write_json_response(&mut second, &fail_body);
        });

        let provider = DeepSeekModelProvider::new(
            "test-key",
            format!("http://{}", addr),
            "deepseek-v4",
            5_000,
        )
        .with_streaming(false);
        let controller =
            RootAgentProcessController::with_model_provider(&workspace, Arc::new(provider))
                .unwrap();
        let result = controller
            .start_job_with_config(
                "Recover from missing provider tool argument",
                Some(1),
                phase4_native_tool_config(),
            )
            .unwrap();
        handle.join().unwrap();

        assert_eq!(result.status, "failed");
        let truth = ProcessTruthStore::new(&workspace, &result.job_id).unwrap();
        let events = truth.read_events().unwrap();
        assert!(events.iter().any(|event| {
            event.event_type == "provider_tool_call_recoverable_error"
                && event.data["error_code"] == "PROVIDER_NATIVE_TOOL_ARGUMENTS_INVALID"
        }));
        assert!(events.iter().any(|event| {
            event.event_type == "provider_user_control_message_recorded"
                && event.data["control_kind"] == "recoverable_tool_error_correction"
        }));
        assert!(!events
            .iter()
            .any(|event| event.event_type == "job_interrupted_by_model_protocol_error"));
        assert!(!events.iter().any(|event| event.event_type == "job_blocked"));
    }

    #[test]
    #[ignore = "RC0 run-through converts kernel approval blocks into direct execution or ordinary recoverable errors."]
    fn deepseek_native_kernel_block_receipt_is_recoverable_without_blocking_task() {
        let workspace = temp_workspace("deepseek_recover_kernel_block_receipt");
        fs::write(workspace.join("source.md"), "source").unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let (mut first, _) = listener.accept().unwrap();
            let first_request = read_http_request(&mut first);
            assert!(first_request.contains("\"cap_os_copy_path\""));
            let body = deepseek_tool_call_body(
                "call_copy_with_invalid_approval",
                "cap_os_copy_path",
                json!({
                    "approval_id": "approval_fake",
                    "source_path": "source.md",
                    "destination_path": "copied.md",
                    "reason": "Try an apply tool with an invalid approval id."
                }),
                "Use an invalid approval token.",
            );
            write_json_response(&mut first, &body);

            let (mut second, _) = listener.accept().unwrap();
            let second_request = read_http_request(&mut second);
            assert!(second_request.contains("call_copy_with_invalid_approval"));
            assert!(second_request.contains("PROVIDER_NATIVE_KERNEL_BLOCKED_TOOL_CALL"));
            assert!(second_request.contains("provider_native_corrective_instruction"));
            let fail_body = deepseek_tool_call_body(
                "call_fail_after_kernel_block_correction",
                "cap_process_fail",
                json!({
                    "error_code": "KERNEL_BLOCK_RECOVERY_OBSERVED",
                    "reason": "Observed recoverable Kernel block result."
                }),
                "Fail after observing the recoverable Kernel block.",
            );
            write_json_response(&mut second, &fail_body);
        });

        let provider = DeepSeekModelProvider::new(
            "test-key",
            format!("http://{}", addr),
            "deepseek-v4",
            5_000,
        )
        .with_streaming(false);
        let controller =
            RootAgentProcessController::with_model_provider(&workspace, Arc::new(provider))
                .unwrap();
        let mut config = phase4_native_tool_config();
        config.tool_calling.toolset_mode = ProviderToolsetMode::FullRegistered;
        let result = controller
            .start_job_with_config("Recover from invalid approval block", Some(1), config)
            .unwrap();
        handle.join().unwrap();

        assert_eq!(result.status, "failed");
        assert!(workspace.join("source.md").exists());
        assert!(!workspace.join("copied.md").exists());
        let truth = ProcessTruthStore::new(&workspace, &result.job_id).unwrap();
        let events = truth.read_events().unwrap();
        assert!(events.iter().any(|event| {
            event.event_type == "capability_receipt"
                && event.data["capability_id"] == "os.copy_path"
                && event.data["status"] == "blocked"
        }));
        assert!(events.iter().any(|event| {
            event.event_type == "provider_tool_call_recoverable_error"
                && event.data["error_code"] == "PROVIDER_NATIVE_KERNEL_BLOCKED_TOOL_CALL"
        }));
        assert!(!events.iter().any(|event| event.event_type == "job_blocked"));
        assert!(!events
            .iter()
            .any(|event| event.event_type == "job_interrupted_by_model_protocol_error"));
    }

    #[test]
    #[ignore = "RC0 run-through disables unapproved mutation preview blocking."]
    fn deepseek_phase5_unapproved_mutation_intent_creates_kernel_preview_without_writing() {
        let workspace = temp_workspace("deepseek_phase5_unapproved_mutation");
        fs::write(workspace.join("UNAPPROVED.md"), "# Existing\n").unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_http_request(&mut stream);
            assert!(request.contains("\"tools\""));
            assert!(request.contains("\"cap_os_write_artifact\""));
            assert!(!request.contains("\"cap_process_request_preview\""));
            assert!(!request.contains("\"cap_process_preview_create\""));
            assert!(!request.contains("\"cap_process_pending_approvals\""));
            let body = deepseek_tool_call_body(
                "call_unapproved_write",
                "cap_os_write_artifact",
                json!({
                    "path": "UNAPPROVED.md",
                    "content": "# Unapproved\n\nThis must not be written.",
                    "reason": "Attempt to overwrite an existing artifact without approval."
                }),
                "Try to overwrite directly without asking for approval.",
            );
            write_json_response(&mut stream, &body);
        });

        let provider = DeepSeekModelProvider::new(
            "test-key",
            format!("http://{}", addr),
            "deepseek-v4",
            5_000,
        )
        .with_streaming(false);
        let controller =
            RootAgentProcessController::with_model_provider(&workspace, Arc::new(provider))
                .unwrap();
        let mut config = phase4_native_tool_config();
        config.tool_calling.toolset_mode = ProviderToolsetMode::FullRegistered;
        let result = controller
            .start_job_with_config(
                "Try a provider-native mutation without approval",
                Some(1),
                config,
            )
            .unwrap();
        handle.join().unwrap();

        assert_eq!(result.status, "waiting_approval");
        assert_eq!(
            fs::read_to_string(workspace.join("UNAPPROVED.md")).unwrap(),
            "# Existing\n"
        );
        let truth = ProcessTruthStore::new(&workspace, &result.job_id).unwrap();
        let events = truth.read_events().unwrap();
        assert!(events.iter().any(|event| {
            event.event_type == "process_action_validated"
                && event.data["capability_id"] == "process.request_preview"
        }));
        assert!(events.iter().any(|event| {
            event.event_type == "capability_receipt"
                && event.data["capability_id"] == "process.request_preview"
                && event.data["status"] == "success"
        }));
        assert!(events.iter().any(|event| {
            event.event_type == "agent_tool_action_executed"
                && event.data["capability_id"] == "process.request_preview"
                && event.data["receipt_data"]["provider_native_auto_preview"] == true
        }));
        assert!(events.iter().any(|event| {
            event.event_type == "provider_tool_call_waiting_approval"
                && event.data["provider_tool_call_id"] == "call_unapproved_write"
                && event.data["capability_id"] == "os.write_artifact"
                && event.data["pending_provider_tool_result"] == true
        }));
        assert!(!events.iter().any(|event| {
            event.event_type == "provider_tool_result_recorded"
                && event.data["provider_tool_call_id"] == "call_unapproved_write"
        }));
        assert!(events
            .iter()
            .any(|event| event.event_type == "preview_tx_created"));
    }

    #[test]
    #[ignore = "RC0 run-through removes reject approval resume semantics."]
    fn deepseek_phase5_rejected_approval_returns_tool_result_to_model_without_executing() {
        let workspace = temp_workspace("deepseek_phase5_rejected_approval_tool_result");
        fs::write(workspace.join("DELETE_ME.md"), "delete me").unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let (mut first, _) = listener.accept().unwrap();
            let first_request = read_http_request(&mut first);
            assert!(first_request.contains("\"tools\""));
            assert!(first_request.contains("\"cap_os_delete_path\""));
            assert!(!first_request.contains("\"cap_process_request_preview\""));
            assert!(!first_request.contains("\"cap_process_preview_create\""));
            let body = deepseek_tool_call_body(
                "call_unapproved_delete",
                "cap_os_delete_path",
                json!({
                    "path": "DELETE_ME.md",
                    "reason": "Attempt delete before approval."
                }),
                "Try an apply tool before approval.",
            );
            write_json_response(&mut first, &body);

            let (mut second, _) = listener.accept().unwrap();
            let second_request = read_http_request(&mut second);
            assert!(second_request.contains("call_unapproved_delete"));
            assert!(second_request.contains("\"role\":\"tool\""));
            assert!(second_request.contains("rejected"));
            assert!(second_request.contains("keep the file"));
            let complete_body = deepseek_tool_call_body(
                "call_complete_after_reject",
                "cap_process_complete",
                json!({
                    "completion_statement": "The user rejected the delete approval, so DELETE_ME.md was not deleted.",
                    "claimed_artifacts": [],
                    "key_sources": ["DELETE_ME.md"],
                    "known_limitations": [],
                    "user_review_notes": ["User rejected the pending delete."]
                }),
                "Complete after observing the rejected approval tool result.",
            );
            write_json_response(&mut second, &complete_body);
        });

        let provider = DeepSeekModelProvider::new(
            "test-key",
            format!("http://{}", addr),
            "deepseek-v4",
            5_000,
        )
        .with_streaming(false);
        let controller =
            RootAgentProcessController::with_model_provider(&workspace, Arc::new(provider))
                .unwrap();
        let mut config = phase4_native_tool_config();
        config.tool_calling.toolset_mode = ProviderToolsetMode::FullRegistered;
        let first = controller
            .start_job_with_config(
                "Try a provider-native delete without approval",
                Some(1),
                config,
            )
            .unwrap();
        assert_eq!(first.status, "waiting_approval");
        assert!(workspace.join("DELETE_ME.md").exists());
        let truth = ProcessTruthStore::new(&workspace, &first.job_id).unwrap();
        let events_before_response = truth.read_events().unwrap();
        let approval_id = events_before_response
            .iter()
            .rev()
            .find(|event| {
                event.event_type == "provider_tool_call_waiting_approval"
                    && event.data["provider_tool_call_id"] == "call_unapproved_delete"
            })
            .and_then(|event| event.data["preview_id"].as_str())
            .expect("approval preview id")
            .to_string();
        let result = controller
            .submit_user_input_for_approval_with_max_turns(
                &first.job_id,
                &approval_id,
                "reject: keep the file",
                Some(1),
            )
            .unwrap();
        handle.join().unwrap();

        assert_eq!(result.status, "completed");
        assert!(workspace.join("DELETE_ME.md").exists());
        let truth = ProcessTruthStore::new(&workspace, &first.job_id).unwrap();
        let events = truth.read_events().unwrap();
        assert!(events.iter().any(|event| {
            event.event_type == "provider_tool_call_user_response_recorded"
                && event.data["provider_tool_call_id"] == "call_unapproved_delete"
                && event.data["status"] == "rejected"
                && event.data["tool_executed"] == false
        }));
        assert!(events.iter().any(|event| {
            event.event_type == "provider_tool_result_recorded"
                && event.data["provider_tool_call_id"] == "call_unapproved_delete"
        }));
        assert!(!events.iter().any(|event| {
            event.event_type == "capability_receipt"
                && event.data["capability_id"] == "os.delete_path"
                && event.data["status"] == "success"
        }));
    }

    #[test]
    #[ignore = "RC0 run-through disables kernel-owned approval pause before provider writes."]
    fn deepseek_phase5_kernel_owned_approval_executes_original_write_tool_call() {
        let workspace = temp_workspace("deepseek_phase5_kernel_owned_approval_execute");
        fs::write(workspace.join("APPROVED.md"), "# Old\n").unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::<String>::new()));
        let request_log = requests.clone();
        let handle = thread::spawn(move || {
            let (mut first, _) = listener.accept().unwrap();
            let first_request = read_http_request(&mut first);
            assert!(first_request.contains("\"cap_os_write_artifact\""));
            assert!(!first_request.contains("\"cap_process_request_preview\""));
            assert!(!first_request.contains("\"cap_process_preview_create\""));
            request_log.lock().unwrap().push(first_request);
            let write_body = deepseek_tool_call_body(
                "call_approved_write",
                "cap_os_write_artifact",
                json!({
                    "path": "APPROVED.md",
                    "content": "# Approved\n\nThis write was approved.",
                    "reason": "Write the requested artifact; Kernel should preview before overwriting."
                }),
                "Request a write intent; Kernel owns preview and approval.",
            );
            write_json_response(&mut first, &write_body);

            let (mut second, _) = listener.accept().unwrap();
            let second_request = read_http_request(&mut second);
            assert!(second_request.contains("call_approved_write"));
            assert!(second_request.contains("\"role\":\"tool\""));
            assert!(second_request.contains("receipt_status"));
            assert!(second_request.contains("success"));
            assert!(second_request.contains("os.write_artifact"));
            assert!(!second_request.contains("call_apply"));
            request_log.lock().unwrap().push(second_request);
            let complete_body = deepseek_tool_call_body(
                "call_complete_after_apply",
                "cap_process_complete",
                json!({
                    "completion_statement": "APPROVED.md was written only after preview approval and Kernel token validation.",
                    "claimed_artifacts": [],
                    "key_sources": ["APPROVED.md"],
                    "known_limitations": [],
                    "user_review_notes": []
                }),
                "The approved mutation receipt is enough to close.",
            );
            write_json_response(&mut second, &complete_body);
        });

        let provider = DeepSeekModelProvider::new(
            "test-key",
            format!("http://{}", addr),
            "deepseek-v4",
            5_000,
        )
        .with_streaming(true);
        let controller =
            RootAgentProcessController::with_model_provider(&workspace, Arc::new(provider))
                .unwrap();
        let mut config = phase4_native_tool_config();
        config.tool_calling.toolset_mode = ProviderToolsetMode::FullRegistered;
        let first = controller
            .start_job_with_config(
                "Write APPROVED.md, but only after approval",
                Some(1),
                config,
            )
            .unwrap();
        assert_eq!(first.status, "waiting_approval");
        assert_eq!(
            fs::read_to_string(workspace.join("APPROVED.md")).unwrap(),
            "# Old\n"
        );

        let resumed = controller
            .approve_preview_with_max_turns(&first.job_id, "approved by phase5 test", Some(1))
            .unwrap();
        handle.join().unwrap();

        assert_eq!(resumed.status, "completed");
        assert!(workspace.join("APPROVED.md").exists());
        assert_eq!(
            fs::read_to_string(workspace.join("APPROVED.md")).unwrap(),
            "# Approved\n\nThis write was approved."
        );
        assert_eq!(requests.lock().unwrap().len(), 2);

        let truth = ProcessTruthStore::new(&workspace, &first.job_id).unwrap();
        let events = truth.read_events().unwrap();
        assert!(events.iter().any(|event| {
            event.event_type == "provider_tool_call_waiting_approval"
                && event.data["provider_tool_call_id"] == "call_approved_write"
                && event.data["preview_id"].as_str().is_some()
                && event.data["preview_ref"].as_str().is_some()
                && event.data["pending_provider_tool_result"] == true
        }));
        assert!(!events.iter().any(|event| {
            event.event_type == "provider_user_control_message_recorded"
                && event.data["control_kind"] == "approval_resume"
        }));
        assert!(events
            .iter()
            .any(|event| event.event_type == "approval_token_consumed"));
        assert!(events
            .iter()
            .any(|event| event.event_type == "approval_token_used"));
        assert!(events.iter().any(|event| {
            event.event_type == "capability_receipt"
                && event.data["capability_id"] == "os.write_artifact"
                && event.data["status"] == "success"
        }));
        assert!(events.iter().any(|event| {
            event.event_type == "provider_tool_result_recorded"
                && event.data["provider_tool_call_id"] == "call_approved_write"
        }));
        assert!(events.iter().any(|event| {
            event.event_type == "provider_tool_call_approval_executed"
                && event.data["provider_tool_call_id"] == "call_approved_write"
                && event.data["receipt_status"] == "success"
                && event.data["provider_tool_result_recorded"] == true
        }));
        let approval_token_id = events
            .iter()
            .find(|event| event.event_type == "approval_token_issued")
            .and_then(|event| event.data.get("approval_token_id"))
            .and_then(Value::as_str)
            .unwrap();
        assert!(!ApprovalRuntime::new(truth.clone())
            .validate_token(approval_token_id, "os.write_artifact", &["APPROVED.md"])
            .unwrap());
        let transcript =
            replay_provider_transcript_state(&truth, "deepseek", "deepseek_chat_completions")
                .unwrap()
                .unwrap();
        let messages = read_provider_messages(&truth, &transcript).unwrap();
        assert!(messages.iter().any(|message| {
            message.role == "assistant"
                && message
                    .reasoning_content
                    .as_deref()
                    .unwrap_or("")
                    .contains("Kernel owns preview and approval.")
        }));
    }

    #[test]
    fn deepseek_native_write_artifact_rejects_zip_and_docx_targets() {
        let workspace = temp_workspace("deepseek_native_reject_binary_artifact_write");
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let (mut first, _) = listener.accept().unwrap();
            let first_request = read_http_request(&mut first);
            assert!(first_request.contains("\"cap_os_write_artifact\""));
            let zip_body = deepseek_tool_call_body(
                "call_fake_zip",
                "cap_os_write_artifact",
                json!({
                    "path": "deliverable.zip",
                    "content": "placeholder",
                    "reason": "Incorrectly try to create zip as a text artifact."
                }),
                "Try to write a fake zip artifact.",
            );
            write_json_response(&mut first, &zip_body);

            let (mut second, _) = listener.accept().unwrap();
            let second_request = read_http_request(&mut second);
            assert!(
                second_request.contains("PROVIDER_NATIVE_WRITE_ARTIFACT_BINARY_TARGET_REJECTED")
            );
            assert!(
                second_request.contains("binary_or_compound_artifact_requires_native_capability")
            );
            let docx_body = deepseek_tool_call_body(
                "call_fake_docx",
                "cap_os_write_artifact",
                json!({
                    "path": "deliverables/leadership_brief.docx",
                    "content": "This is plain text, not an OpenXML document.",
                    "reason": "Incorrectly try to create docx as a text artifact."
                }),
                "Try to write a fake docx artifact.",
            );
            write_json_response(&mut second, &docx_body);

            let (mut third, _) = listener.accept().unwrap();
            let third_request = read_http_request(&mut third);
            assert!(third_request.contains("call_fake_docx"));
            assert!(third_request.contains("PROVIDER_NATIVE_WRITE_ARTIFACT_BINARY_TARGET_REJECTED"));
            let fail_body = deepseek_tool_call_body(
                "call_fail_after_binary_rejections",
                "cap_process_fail",
                json!({
                    "error_code": "BINARY_ARTIFACT_WRITE_REJECTED",
                    "reason": "Observed provider-native rejection for fake binary artifacts."
                }),
                "Fail after observing the binary artifact write rejections.",
            );
            write_json_response(&mut third, &fail_body);
        });

        let provider = DeepSeekModelProvider::new(
            "test-key",
            format!("http://{}", addr),
            "deepseek-v4",
            5_000,
        )
        .with_streaming(false);
        let controller =
            RootAgentProcessController::with_model_provider(&workspace, Arc::new(provider))
                .unwrap();
        let mut config = phase4_native_tool_config();
        config.tool_calling.toolset_mode = ProviderToolsetMode::FullRegistered;
        let result = controller
            .start_job_with_config(
                "Do not allow fake zip/docx artifacts through os.write_artifact",
                Some(1),
                config,
            )
            .unwrap();
        handle.join().unwrap();

        assert_eq!(result.status, "failed");
        assert!(!workspace.join("deliverable.zip").exists());
        assert!(!workspace
            .join("deliverables/leadership_brief.docx")
            .exists());
        let truth = ProcessTruthStore::new(&workspace, &result.job_id).unwrap();
        let events = truth.read_events().unwrap();
        assert!(events.iter().any(|event| {
            event.event_type == "provider_tool_call_recoverable_error"
                && event.data["provider_tool_call_id"] == "call_fake_zip"
                && event.data["error_code"]
                    == "PROVIDER_NATIVE_WRITE_ARTIFACT_BINARY_TARGET_REJECTED"
        }));
        assert!(events.iter().any(|event| {
            event.event_type == "provider_tool_call_recoverable_error"
                && event.data["provider_tool_call_id"] == "call_fake_docx"
                && event.data["error_code"]
                    == "PROVIDER_NATIVE_WRITE_ARTIFACT_BINARY_TARGET_REJECTED"
        }));
        assert!(!events
            .iter()
            .any(|event| event.event_type == "preview_tx_created"));
    }

    #[test]
    fn deepseek_native_write_artifact_rejects_mutation_completion_report_without_receipt() {
        let workspace = temp_workspace("deepseek_native_reject_fake_mutation_report");
        fs::create_dir_all(workspace.join("empty_dirs/待清理A")).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let (mut first, _) = listener.accept().unwrap();
            let first_request = read_http_request(&mut first);
            assert!(first_request.contains("\"cap_os_write_artifact\""));
            let report_body = deepseek_tool_call_body(
                "call_fake_cleanup_report",
                "cap_os_write_artifact",
                json!({
                    "path": "CLEANUP_PREVIEW.md",
                    "content": "# 空目录清理报告\n\n状态: 已执行\n\n已删除 `empty_dirs/待清理A`，清理执行完成。",
                    "reason": "Incorrectly claim cleanup execution without a delete receipt."
                }),
                "Try to write a cleanup report that claims execution.",
            );
            write_json_response(&mut first, &report_body);

            let (mut second, _) = listener.accept().unwrap();
            let second_request = read_http_request(&mut second);
            assert!(
                second_request.contains("PROVIDER_NATIVE_MUTATION_REPORT_WITHOUT_RECEIPT_REJECTED")
            );
            assert!(second_request.contains("workspace_mutation_report_requires_receipt"));
            let fail_body = deepseek_tool_call_body(
                "call_fail_after_fake_cleanup_report",
                "cap_process_fail",
                json!({
                    "error_code": "FAKE_MUTATION_REPORT_REJECTED",
                    "reason": "Observed provider-native rejection for mutation report without receipt."
                }),
                "Fail after observing the fake mutation report rejection.",
            );
            write_json_response(&mut second, &fail_body);
        });

        let provider = DeepSeekModelProvider::new(
            "test-key",
            format!("http://{}", addr),
            "deepseek-v4",
            5_000,
        )
        .with_streaming(false);
        let controller =
            RootAgentProcessController::with_model_provider(&workspace, Arc::new(provider))
                .unwrap();
        let mut config = phase4_native_tool_config();
        config.tool_calling.toolset_mode = ProviderToolsetMode::FullRegistered;
        let result = controller
            .start_job_with_config(
                "Do not allow cleanup reports to claim execution without receipts",
                Some(1),
                config,
            )
            .unwrap();
        handle.join().unwrap();

        assert_eq!(result.status, "failed");
        assert!(!workspace.join("CLEANUP_PREVIEW.md").exists());
        assert!(workspace.join("empty_dirs/待清理A").exists());
        let truth = ProcessTruthStore::new(&workspace, &result.job_id).unwrap();
        let events = truth.read_events().unwrap();
        assert!(events.iter().any(|event| {
            event.event_type == "provider_tool_call_recoverable_error"
                && event.data["provider_tool_call_id"] == "call_fake_cleanup_report"
                && event.data["error_code"]
                    == "PROVIDER_NATIVE_MUTATION_REPORT_WITHOUT_RECEIPT_REJECTED"
        }));
        assert!(!events
            .iter()
            .any(|event| event.event_type == "preview_tx_created"));
    }

    #[test]
    fn deepseek_phase6_provider_tool_schema_covers_all_non_model_capabilities_and_domains() {
        let config = phase4_native_tool_config();
        let registry = default_capability_registry();
        let planner = ProviderToolsetPlanner::new(registry.clone(), config);
        let coverage = planner.coverage_registry();
        let non_model = registry
            .iter()
            .filter(|descriptor| !descriptor.capability_id.starts_with("model."))
            .collect::<Vec<_>>();
        assert_eq!(coverage.bindings.len(), non_model.len());
        for descriptor in non_model {
            let provider_name = provider_tool_name_for_capability(&descriptor.capability_id);
            let binding = coverage.binding_for_tool_name(&provider_name).unwrap();
            assert_eq!(binding.capability_id, descriptor.capability_id);
            let tool = coverage
                .tools
                .iter()
                .find(|item| item.function.name == provider_name)
                .unwrap();
            assert_eq!(tool.function.parameters["type"], "object");
            assert!(tool.function.parameters.get("properties").is_some());
            assert!(!tool.function.description.trim().is_empty());
        }
        assert!(coverage
            .binding_for_tool_name("cap_model_summarize")
            .is_none());
        for domain in [
            "process",
            "os",
            "workspace",
            "source_set",
            "dataset",
            "artifact",
            "office",
            "package",
            "terminal",
            "child_process",
        ] {
            let domain_registry = planner.plan_domain(domain);
            assert!(
                !domain_registry.tools.is_empty(),
                "domain {domain} should have provider tool coverage"
            );
        }
    }

    #[test]
    fn task_runtime_provider_toolset_excludes_chat_control_manual_approval_and_manual_preview() {
        let config = phase4_native_tool_config();
        let registry = default_capability_registry();
        let schema_coverage = ProviderToolRegistry::phase6_schema_coverage(&registry, &config);
        let task_runtime = ProviderToolRegistry::phase6_full_coverage(&registry, &config);

        for capability_id in [
            "chat.answer",
            "chat.clarify",
            "chat.needs_task",
            "process.approval.record",
            "process.request_preview",
            "process.preview.create",
            "process.pending_approvals",
            "workspace.rename_batch_preview",
            "os.write_source_mutation_preview",
            "office.docx.rewrite_in_place_preview",
        ] {
            let provider_name = provider_tool_name_for_capability(capability_id);
            assert!(
                schema_coverage
                    .binding_for_tool_name(&provider_name)
                    .is_some(),
                "{capability_id} should remain schema-covered"
            );
            assert!(
                task_runtime.binding_for_tool_name(&provider_name).is_none(),
                "{capability_id} must not be exposed to TaskRuntime provider tool calls"
            );
        }
        assert!(task_runtime
            .binding_for_tool_name(&provider_tool_name_for_capability("process.complete"))
            .is_some());
        assert!(task_runtime
            .binding_for_tool_name(&provider_tool_name_for_capability("dataset.read_page"))
            .is_some());
    }

    fn provider_tool_parameters<'a>(
        registry: &'a ProviderToolRegistry,
        provider_name: &str,
    ) -> &'a Value {
        &registry
            .tools
            .iter()
            .find(|tool| tool.function.name == provider_name)
            .unwrap_or_else(|| panic!("missing provider tool {provider_name}"))
            .function
            .parameters
    }

    fn schema_required_fields(parameters: &Value) -> BTreeSet<String> {
        parameters
            .get("required")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect()
    }

    fn schema_property_fields(parameters: &Value) -> BTreeSet<String> {
        parameters
            .get("properties")
            .and_then(Value::as_object)
            .into_iter()
            .flat_map(|properties| properties.keys().cloned())
            .collect()
    }

    fn assert_no_loose_array_object_items(schema: &Value, path: &str) {
        match schema {
            Value::Object(object) => {
                if object.get("type").and_then(Value::as_str) == Some("array") {
                    if let Some(items) = object.get("items").and_then(Value::as_object) {
                        if items.get("type").and_then(Value::as_str) == Some("object") {
                            assert!(
                                items.get("properties").and_then(Value::as_object).is_some(),
                                "{path}.items object must expose explicit properties"
                            );
                            assert!(
                                items.get("required").and_then(Value::as_array).is_some(),
                                "{path}.items object must expose required fields"
                            );
                        }
                    }
                }
                if let Some(properties) = object.get("properties").and_then(Value::as_object) {
                    for (name, value) in properties {
                        assert_no_loose_array_object_items(value, &format!("{path}.{name}"));
                    }
                }
                if let Some(items) = object.get("items") {
                    assert_no_loose_array_object_items(items, &format!("{path}.items"));
                }
                if let Some(any_of) = object.get("anyOf").and_then(Value::as_array) {
                    for (index, value) in any_of.iter().enumerate() {
                        assert_no_loose_array_object_items(
                            value,
                            &format!("{path}.anyOf[{index}]"),
                        );
                    }
                }
            }
            Value::Array(items) => {
                for (index, value) in items.iter().enumerate() {
                    assert_no_loose_array_object_items(value, &format!("{path}[{index}]"));
                }
            }
            _ => {}
        }
    }

    #[test]
    fn deepseek_phase7_provider_tool_schemas_match_runtime_refs_without_readonly_approval() {
        let mut config = phase4_native_tool_config();
        config.tool_calling.toolset_mode = ProviderToolsetMode::FullRegistered;
        let registry =
            ProviderToolRegistry::phase6_full_coverage(&default_capability_registry(), &config);

        for (provider_name, binding) in &registry.bindings {
            if !provider_tool_requires_explicit_approval_id(&binding.capability_id) {
                let required =
                    schema_required_fields(provider_tool_parameters(&registry, provider_name));
                assert!(
                    !required.contains("approval_id"),
                    "{} ({}) must not require approval_id",
                    provider_name,
                    binding.capability_id
                );
            }
        }

        let page = provider_tool_parameters(&registry, "cap_tool_result_page");
        let page_required = schema_required_fields(page);
        let page_properties = schema_property_fields(page);
        assert!(!page_required.contains("approval_id"));
        for field in [
            "ref",
            "raw_result_ref",
            "receipt_ref",
            "offset",
            "limit_bytes",
        ] {
            assert!(
                page_properties.contains(field),
                "tool.result.page missing {field}"
            );
        }

        let batch_read = provider_tool_parameters(&registry, "cap_office_docx_batch_read_text");
        let batch_required = schema_required_fields(batch_read);
        assert!(batch_required.contains("source_set_ref"));
        assert!(!batch_required.contains("approval_id"));

        let coverage = provider_tool_parameters(&registry, "cap_artifact_verify_coverage");
        let coverage_required = schema_required_fields(coverage);
        let coverage_properties = schema_property_fields(coverage);
        assert!(coverage_required.contains("artifact_path"));
        assert!(!coverage_required.contains("approval_id"));
        assert!(coverage_properties.contains("source_set_ref"));
        assert!(coverage_properties.contains("dataset_ref"));
        assert!(coverage_properties.contains("coverage_contract"));
    }

    #[test]
    fn deepseek_phase7_provider_tool_schemas_are_explicit_for_all_exposed_tools() {
        let mut config = phase4_native_tool_config();
        config.tool_calling.toolset_mode = ProviderToolsetMode::FullRegistered;
        let registry =
            ProviderToolRegistry::phase6_full_coverage(&default_capability_registry(), &config);

        for tool in &registry.tools {
            let parameters = &tool.function.parameters;
            assert_eq!(
                parameters.get("type").and_then(Value::as_str),
                Some("object"),
                "{} parameters must be an object schema",
                tool.function.name
            );
            assert!(
                !schema_property_fields(parameters).contains("raw_arguments"),
                "{} must use explicit provider arguments instead of generic raw_arguments",
                tool.function.name
            );
            assert_no_loose_array_object_items(parameters, &tool.function.name);
        }

        assert!(registry
            .binding_for_tool_name(&provider_tool_name_for_capability(
                "workspace.rename_batch_preview"
            ))
            .is_none());
    }

    #[test]
    fn deepseek_phase7_provider_tool_mutation_schemas_omit_approval_and_keep_runtime_fields() {
        let mut config = phase4_native_tool_config();
        config.tool_calling.toolset_mode = ProviderToolsetMode::FullRegistered;
        let registry =
            ProviderToolRegistry::phase6_full_coverage(&default_capability_registry(), &config);

        for (provider_name, expected_fields) in [
            ("cap_workspace_apply_organize_tx", vec!["organize_plan_ref"]),
            ("cap_workspace_rename_batch_apply", vec!["rename_plan_ref"]),
            ("cap_dataset_export_csv", vec!["dataset_ref", "output_path"]),
            ("cap_os_write_file", vec!["path", "write_kind"]),
            (
                "cap_package_build_zip",
                vec!["source_set_ref", "destination_zip_path"],
            ),
            ("cap_terminal_run_command", vec!["argv"]),
        ] {
            let parameters = provider_tool_parameters(&registry, provider_name);
            let required = schema_required_fields(parameters);
            for field in expected_fields {
                assert!(
                    required.contains(field),
                    "{provider_name} must require {field}; required={required:?}"
                );
            }
            assert!(
                !schema_property_fields(parameters).contains("raw_arguments"),
                "{provider_name} should use explicit runtime arguments, not generic raw_arguments"
            );
            assert!(
                !schema_property_fields(parameters).contains("approval_id"),
                "{provider_name} must not expose Kernel approval_id to provider tools"
            );
        }
    }

    #[test]
    fn deepseek_phase6_default_rc0_full_visible_toolset_records_ref_and_terminal_visible() {
        let workspace = temp_workspace("deepseek_phase6_rc0_full_visible_default");
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_http_request(&mut stream);
            assert!(request.contains("\"tools\""));
            assert!(request.contains("\"cap_process_complete\""));
            assert!(request.contains("\"cap_process_toolset_select\""));
            assert!(request.contains("\"cap_os_read_file\""));
            assert!(request.contains("\"cap_os_write_artifact\""));
            assert!(request.contains("\"cap_terminal_run_command\""));
            assert!(request.contains("\"cap_artifact_verify_coverage\""));
            assert!(request.contains("\"cap_artifact_source_coverage_verify\""));
            let body = deepseek_tool_call_body(
                "call_complete_progressive",
                "cap_process_complete",
                json!({
                    "completion_statement": "Closed after inspecting the RC0 full visible provider toolset.",
                    "claimed_artifacts": [],
                    "key_sources": [],
                    "known_limitations": [],
                    "user_review_notes": []
                }),
                "Default RC0 full visible disclosure can complete with all task tools visible.",
            );
            write_json_response(&mut stream, &body);
        });
        let provider = DeepSeekModelProvider::new(
            "test-key",
            format!("http://{}", addr),
            "deepseek-v4",
            5_000,
        )
        .with_streaming(false);
        let controller =
            RootAgentProcessController::with_model_provider(&workspace, Arc::new(provider))
                .unwrap();
        let result = controller
            .start_job_with_config(
                "Exercise default RC0 full visible toolset",
                Some(1),
                phase4_native_tool_config(),
            )
            .unwrap();
        handle.join().unwrap();
        assert_eq!(result.status, "completed");

        let truth = ProcessTruthStore::new(&workspace, &result.job_id).unwrap();
        let events = truth.read_events().unwrap();
        let planned = events
            .iter()
            .find(|event| event.event_type == "provider_toolset_planned")
            .unwrap();
        let provider_toolset_ref = planned
            .data
            .get("provider_toolset_ref")
            .and_then(Value::as_str)
            .unwrap();
        let record: ProviderToolsetRecord = serde_json::from_slice(
            &fs::read(truth.resolve_blob_ref(provider_toolset_ref).unwrap()).unwrap(),
        )
        .unwrap();
        assert!(!record.progressive_disclosure);
        assert_eq!(record.requested_mode, ProviderToolsetMode::Rc0FullVisible);
        assert_eq!(record.effective_mode, ProviderToolsetMode::Rc0FullVisible);
        assert_eq!(record.lifecycle_stage, "rc0_full_visible");
        assert_eq!(
            record.active_group_ids,
            vec!["rc0_full_visible".to_string()]
        );
        assert_eq!(record.selected_count, record.schema_coverage_count);
        assert!(record
            .selected_capability_ids
            .contains(&"os.write_artifact".to_string()));
        assert!(record
            .selected_capability_ids
            .contains(&"process.toolset.select".to_string()));
        assert!(record.omitted_tools.is_empty());
        assert!(record
            .selected_capability_ids
            .contains(&"terminal.run_command".to_string()));
        assert!(events.iter().any(|event| {
            event.event_type == "model_call_receipt"
                && event.data["provider_toolset_ref"] == provider_toolset_ref
        }));
    }

    #[test]
    #[ignore = "RC0 run-through disables provider-native terminal approval pause."]
    fn deepseek_provider_native_terminal_approval_executes_pending_call_and_returns_output() {
        let workspace = temp_workspace("deepseek_provider_terminal_approval_executes");
        fs::write(workspace.join("existing.txt"), "old").unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let (mut first, _) = listener.accept().unwrap();
            let first_request = read_http_request(&mut first);
            assert!(first_request.contains("\"cap_terminal_run_command\""));
            let body = deepseek_tool_call_body(
                "call_terminal_mutation",
                "cap_terminal_run_command",
                json!({
                    "argv": [
                        "powershell.exe",
                        "-NoProfile",
                        "-NonInteractive",
                        "-Command",
                        "$text = ('x' * 9000) + 'approved-tail'; Set-Content -LiteralPath existing.txt -Value approved; Write-Output $text"
                    ],
                    "target_paths": ["existing.txt"],
                    "timeout_ms": 30_000
                }),
                "Modify an existing workspace file through terminal only after user approval.",
            );
            write_json_response(&mut first, &body);

            let (mut second, _) = listener.accept().unwrap();
            let second_request = read_http_request(&mut second);
            assert!(second_request.contains("call_terminal_mutation"));
            assert!(second_request.contains("stdout_text"));
            assert!(!second_request.contains("stdout_text_preview"));
            assert!(second_request.contains("approved-tail"));
            assert!(!second_request.contains("MODEL_TOOL_CALL_REQUIRED_BUT_MISSING"));
            let complete_body = deepseek_tool_call_body(
                "call_complete_after_terminal_approval",
                "cap_process_complete",
                json!({
                    "completion_statement": "Terminal approval executed and output was observed.",
                    "claimed_artifacts": [],
                    "key_sources": ["terminal stdout confirmed approved"],
                    "known_limitations": [],
                    "user_review_notes": []
                }),
                "Complete after observing the approved terminal command output.",
            );
            write_json_response(&mut second, &complete_body);
        });

        let provider = DeepSeekModelProvider::new(
            "test-key",
            format!("http://{}", addr),
            "deepseek-v4",
            5_000,
        )
        .with_streaming(false);
        let controller =
            RootAgentProcessController::with_model_provider(&workspace, Arc::new(provider))
                .unwrap();
        let mut config = phase4_native_tool_config();
        config.tool_calling.max_provider_subturns = 4;
        let waiting = controller
            .start_job_with_config(
                "Provider-native terminal mutation must wait for approval and then return stdout",
                Some(1),
                config,
            )
            .unwrap();
        assert_eq!(waiting.status, "waiting_approval");
        assert_eq!(
            fs::read_to_string(workspace.join("existing.txt")).unwrap(),
            "old"
        );

        let truth = ProcessTruthStore::new(&workspace, &waiting.job_id).unwrap();
        let events_before_approval = truth.read_events().unwrap();
        let pending = events_before_approval
            .iter()
            .rev()
            .find(|event| {
                event.event_type == "provider_tool_call_waiting_approval"
                    && event.data["capability_id"] == "terminal.run_command"
            })
            .expect("provider approval preview event");
        let approval_id = pending
            .data
            .get("preview_id")
            .and_then(Value::as_str)
            .expect("preview id")
            .to_string();
        assert_eq!(
            pending.data["arguments"]["target_paths"],
            json!(["existing.txt"])
        );
        assert!(events_before_approval.iter().any(|event| {
            event.event_type == "preview_tx_created"
                && event.data["executable_operations"][0]["capability_id"] == "terminal.run_command"
                && event.data["executable_operations"][0]["arguments"]["argv"][0]
                    == "powershell.exe"
        }));

        let completed = controller
            .approve_preview_by_id_with_max_turns(
                &waiting.job_id,
                &approval_id,
                "approve terminal mutation test",
                Some(1),
            )
            .unwrap();
        handle.join().unwrap();

        assert_eq!(completed.status, "completed");
        assert_eq!(
            fs::read_to_string(workspace.join("existing.txt"))
                .unwrap()
                .trim(),
            "approved"
        );
        let truth = ProcessTruthStore::new(&workspace, &waiting.job_id).unwrap();
        let events = truth.read_events().unwrap();
        assert!(events.iter().any(|event| {
            event.event_type == "provider_tool_call_approval_executed"
                && event.data["provider_tool_call_id"] == "call_terminal_mutation"
                && event.data["provider_tool_result_recorded"] == true
        }));
        let result_event = events
            .iter()
            .find(|event| {
                event.event_type == "provider_tool_result_recorded"
                    && event.data["provider_tool_call_id"] == "call_terminal_mutation"
            })
            .expect("terminal provider tool result");
        let tool_result_ref = result_event
            .data
            .get("tool_result_ref")
            .and_then(Value::as_str)
            .expect("tool result ref");
        let tool_message: ProviderTranscriptMessage = serde_json::from_slice(
            &fs::read(truth.resolve_blob_ref(tool_result_ref).unwrap()).unwrap(),
        )
        .unwrap();
        let tool_result: Value =
            serde_json::from_str(tool_message.content.as_deref().unwrap()).unwrap();
        assert_eq!(tool_result["capability_id"], "terminal.run_command");
        assert!(tool_result["stdout_text"]
            .as_str()
            .is_some_and(|value| value.contains("approved-tail")));
        assert!(tool_result.get("stdout_text_preview").is_none());
        assert_eq!(
            tool_result["terminal_output_metadata"]["stdout_bytes"],
            tool_result["receipt"]["data"]["stdout_bytes"]
        );
        assert!(events.iter().any(|event| {
            event.event_type == "job_resume_approval_token_ready"
                && event.data["provider_tool_call_executed"] == true
        }));
    }

    #[test]
    fn deepseek_phase6_indexed_groups_use_model_selection_without_ref_heuristics() {
        let workspace = temp_workspace("deepseek_phase6_indexed_groups");
        let (_job, process, truth) =
            create_agent_job(&workspace, "Plan indexed provider toolset").unwrap();
        truth
            .append_event(
                Some(&process.pid),
                "capability_receipt",
                json!({
                    "capability_id": "os.list_tree",
                    "status": "success",
                    "data": {
                        "tree_ref": "blob://job_source_set_progressive/source_set_tree.txt"
                    }
                }),
            )
            .unwrap();
        let mut config = phase4_native_tool_config();
        config.tool_calling.toolset_mode = ProviderToolsetMode::IndexedGroups;
        let planner = ProviderToolsetPlanner::new(default_capability_registry(), config.clone());
        let default_plan = planner
            .plan_and_record(
                &truth,
                &process.pid,
                "mcall_default_indexed",
                &ModelOperation::DecideNextAction,
            )
            .unwrap();
        assert_eq!(default_plan.record.lifecycle_stage, "indexed_groups");
        assert!(default_plan
            .record
            .selected_capability_ids
            .contains(&"source_set.create".to_string()));
        assert!(default_plan
            .record
            .selected_capability_ids
            .contains(&"source_set.read_page".to_string()));
        assert!(default_plan
            .record
            .selected_capability_ids
            .contains(&"workspace.find_duplicates".to_string()));
        assert!(!default_plan
            .record
            .selected_capability_ids
            .contains(&"process.fork_child".to_string()));

        truth
            .append_event(
                Some(&process.pid),
                "provider_toolset_selection_recorded",
                json!({
                    "selection_id": "toolset_sel_test_process_structure",
                    "accepted_groups": ["process_structure"],
                    "accepted_capability_ids": [],
                    "ttl_model_calls": 4,
                }),
            )
            .unwrap();
        let after_selection = ProviderToolsetPlanner::new(default_capability_registry(), config)
            .plan_and_record(
                &truth,
                &process.pid,
                "mcall_after_group_selection",
                &ModelOperation::DecideNextAction,
            )
            .unwrap();
        assert_eq!(
            after_selection.record.selection_id.as_deref(),
            Some("toolset_sel_test_process_structure")
        );
        assert!(after_selection
            .record
            .selected_capability_ids
            .contains(&"process.fork_child".to_string()));
        assert!(after_selection
            .record
            .request_scoped_tool_guide
            .contains("cap_process_fork_child"));
    }

    #[test]
    fn deepseek_phase6_indexed_groups_disclose_approved_mutation_apply_tools() {
        let workspace = temp_workspace("deepseek_phase6_approval_resume_toolset");
        let (_job, process, truth) =
            create_agent_job(&workspace, "Plan provider toolset after approval").unwrap();
        ApprovalRuntime::new(truth.clone())
            .create_preview_tx(
                &process.pid,
                "# Preview\n\nWrite approved artifact.",
                vec![ExecutablePreviewOperation {
                    capability_id: "os.delete_path".to_string(),
                    arguments: json!({"path": "OLD.md"}),
                    target_paths: vec!["OLD.md".to_string()],
                    human_description: "Delete OLD.md.".to_string(),
                    rollback_policy: Some("delete_artifact".to_string()),
                }],
                "medium",
            )
            .unwrap();
        ApprovalRuntime::new(truth.clone())
            .issue_token_for_latest_preview(&process.pid, "approved")
            .unwrap();
        truth
            .append_event(
                Some(&process.pid),
                "provider_toolset_selection_recorded",
                json!({
                    "selection_id": "toolset_sel_test_mutation_apply",
                    "accepted_groups": ["mutation_apply"],
                    "accepted_capability_ids": [],
                    "ttl_model_calls": 4,
                }),
            )
            .unwrap();
        let mut config = phase4_native_tool_config();
        config.tool_calling.toolset_mode = ProviderToolsetMode::IndexedGroups;
        let planner = ProviderToolsetPlanner::new(default_capability_registry(), config);
        let plan = planner
            .plan_and_record(
                &truth,
                &process.pid,
                "mcall_phase6_approval_resume",
                &ModelOperation::DecideNextAction,
            )
            .unwrap();
        assert_eq!(plan.record.lifecycle_stage, "indexed_groups");
        assert_eq!(
            plan.record.selection_id.as_deref(),
            Some("toolset_sel_test_mutation_apply")
        );
        assert!(plan
            .record
            .selected_capability_ids
            .iter()
            .any(|item| item == "os.delete_path"));
        assert!(!plan.record.truncated_by_provider_limit);
    }

    #[test]
    fn deepseek_phase6_indexed_groups_disclose_active_approval_tools_without_selection() {
        let workspace = temp_workspace("deepseek_phase6_active_approval_without_selection");
        let (_job, process, truth) = create_agent_job(
            &workspace,
            "Plan provider toolset from active approval token",
        )
        .unwrap();
        ApprovalRuntime::new(truth.clone())
            .create_preview_tx(
                &process.pid,
                "# Preview\n\nDelete OLD.md.",
                vec![ExecutablePreviewOperation {
                    capability_id: "os.delete_path".to_string(),
                    arguments: json!({"path": "OLD.md"}),
                    target_paths: vec!["OLD.md".to_string()],
                    human_description: "Delete OLD.md.".to_string(),
                    rollback_policy: Some("none".to_string()),
                }],
                "medium",
            )
            .unwrap();
        let token = ApprovalRuntime::new(truth.clone())
            .issue_token_for_latest_preview(&process.pid, "approved delete")
            .unwrap();
        let mut config = phase4_native_tool_config();
        config.tool_calling.toolset_mode = ProviderToolsetMode::IndexedGroups;
        let planner = ProviderToolsetPlanner::new(default_capability_registry(), config);
        let plan = planner
            .plan_and_record(
                &truth,
                &process.pid,
                "mcall_phase6_active_approval_without_selection",
                &ModelOperation::DecideNextAction,
            )
            .unwrap();
        assert_eq!(plan.record.selection_id, None);
        assert!(plan
            .record
            .selected_capability_ids
            .contains(&"os.delete_path".to_string()));
        assert!(plan
            .record
            .request_scoped_tool_guide
            .contains("cap_os_delete_path"));

        truth
            .append_event(
                Some(&process.pid),
                "approval_token_consumed",
                json!({"approval_token_id": token.approval_token_id}),
            )
            .unwrap();
        let after_consumed = planner
            .plan_and_record(
                &truth,
                &process.pid,
                "mcall_phase6_active_approval_consumed_without_selection",
                &ModelOperation::DecideNextAction,
            )
            .unwrap();
        assert!(!after_consumed
            .record
            .selected_capability_ids
            .contains(&"os.delete_path".to_string()));
    }

    #[test]
    fn deepseek_phase6_indexed_groups_expose_mutation_intent_without_token() {
        let workspace = temp_workspace("deepseek_phase6_expose_mutation_intent_tools");
        let (_job, process, truth) =
            create_agent_job(&workspace, "Plan unapproved provider toolset").unwrap();
        truth
            .append_event(
                Some(&process.pid),
                "provider_toolset_selection_recorded",
                json!({
                    "selection_id": "toolset_sel_unapproved_apply",
                    "accepted_groups": [
                        "artifact_write",
                        "mutation_apply",
                        "office_docx",
                        "package_release",
                        "terminal_fallback",
                        "rollback_recovery"
                    ],
                    "accepted_capability_ids": [
                        "package.build_zip",
                        "os.zip",
                        "terminal.run_command",
                        "office.docx.rewrite_save_as",
                        "office.docx.create",
                        "dataset.export_csv",
                        "dataset.export_markdown",
                        "artifact.copy_source_set",
                        "workspace.tree_index",
                        "workspace.perf_inventory",
                        "os.delete_path",
                        "os.rename_path",
                        "os.rollback_tx",
                        "workspace.apply_organize_tx",
                        "office.docx.rewrite_in_place"
                    ],
                    "ttl_model_calls": 4,
                }),
            )
            .unwrap();
        let mut config = phase4_native_tool_config();
        config.tool_calling.toolset_mode = ProviderToolsetMode::IndexedGroups;
        let planner = ProviderToolsetPlanner::new(default_capability_registry(), config);
        let plan = planner
            .plan_and_record(
                &truth,
                &process.pid,
                "mcall_phase6_unapproved_apply_hidden",
                &ModelOperation::DecideNextAction,
            )
            .unwrap();
        let selected = plan
            .record
            .selected_capability_ids
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        assert!(selected.contains("os.write_artifact"));
        assert!(selected.contains("os.write_temp_dataset"));
        for capability_id in [
            "os.delete_path",
            "os.rename_path",
            "os.rollback_tx",
            "workspace.apply_organize_tx",
            "office.docx.rewrite_in_place",
        ] {
            assert!(
                selected.contains(capability_id),
                "{capability_id} should be visible as a provider-native mutation intent"
            );
        }
    }

    #[test]
    fn deepseek_phase6_indexed_groups_keep_selected_mutation_intents_after_approval() {
        let workspace = temp_workspace("deepseek_phase6_mutation_intent_toolset");
        let (_job, process, truth) =
            create_agent_job(&workspace, "Plan mutation toolset after approval").unwrap();
        ApprovalRuntime::new(truth.clone())
            .create_preview_tx(
                &process.pid,
                "# Preview\n\nDelete OLD.md.",
                vec![ExecutablePreviewOperation {
                    capability_id: "os.delete_path".to_string(),
                    arguments: json!({"path": "OLD.md"}),
                    target_paths: vec!["OLD.md".to_string()],
                    human_description: "Delete OLD.md.".to_string(),
                    rollback_policy: Some("none".to_string()),
                }],
                "medium",
            )
            .unwrap();
        let token = ApprovalRuntime::new(truth.clone())
            .issue_token_for_latest_preview(&process.pid, "approved delete")
            .unwrap();
        truth
            .append_event(
                Some(&process.pid),
                "provider_toolset_selection_recorded",
                json!({
                    "selection_id": "toolset_sel_mutation_after_approval",
                    "accepted_groups": ["mutation_apply", "package_release", "terminal_fallback"],
                    "accepted_capability_ids": [
                        "os.delete_path",
                        "os.rename_path",
                        "package.build_zip",
                        "os.zip",
                        "terminal.run_command"
                    ],
                    "ttl_model_calls": 4,
                }),
            )
            .unwrap();
        let mut config = phase4_native_tool_config();
        config.tool_calling.toolset_mode = ProviderToolsetMode::IndexedGroups;
        let planner = ProviderToolsetPlanner::new(default_capability_registry(), config);
        let plan = planner
            .plan_and_record(
                &truth,
                &process.pid,
                "mcall_phase6_delete_approval_visible",
                &ModelOperation::DecideNextAction,
            )
            .unwrap();
        assert!(plan
            .record
            .selected_capability_ids
            .contains(&"os.delete_path".to_string()));
        assert!(plan
            .record
            .selected_capability_ids
            .contains(&"os.rename_path".to_string()));
        assert!(plan
            .record
            .selected_capability_ids
            .contains(&"package.build_zip".to_string()));
        assert!(plan
            .record
            .selected_capability_ids
            .contains(&"terminal.run_command".to_string()));

        truth
            .append_event(
                Some(&process.pid),
                "approval_token_consumed",
                json!({"approval_token_id": token.approval_token_id}),
            )
            .unwrap();
        let after_consumed = planner
            .plan_and_record(
                &truth,
                &process.pid,
                "mcall_phase6_package_approval_consumed",
                &ModelOperation::DecideNextAction,
            )
            .unwrap();
        assert!(after_consumed
            .record
            .selected_capability_ids
            .contains(&"os.delete_path".to_string()));
        assert!(after_consumed
            .record
            .selected_capability_ids
            .contains(&"package.build_zip".to_string()));
    }

    #[test]
    fn deepseek_phase6_full_registered_fail_closes_when_provider_limit_is_exceeded() {
        let workspace = temp_workspace("deepseek_phase6_full_registered_limit");
        let (_job, process, truth) =
            create_agent_job(&workspace, "Full registered toolset limit").unwrap();
        let mut config = phase4_native_tool_config();
        config.tool_calling.toolset_mode = ProviderToolsetMode::FullRegistered;
        config.tool_calling.max_provider_tools_per_request = 3;
        let planner = ProviderToolsetPlanner::new(default_capability_registry(), config);
        let err = planner
            .plan_and_record(
                &truth,
                &process.pid,
                "mcall_phase6_full_registered_limit",
                &ModelOperation::DecideNextAction,
            )
            .unwrap_err();
        assert_eq!(err.error_code, "PROVIDER_TOOLSET_LIMIT_EXCEEDED");
        assert!(truth.read_events().unwrap().iter().any(|event| {
            event.event_type == "provider_toolset_planning_failed"
                && event.data["fail_closed"] == true
        }));
    }

    #[test]
    fn deepseek_phase7_strict_mode_only_marks_whitelisted_readonly_tools() {
        let mut config = phase4_native_tool_config();
        config.tool_calling.strict_mode = true;
        config.tool_calling.toolset_mode = ProviderToolsetMode::FullRegistered;
        let registry =
            ProviderToolRegistry::phase6_full_coverage(&default_capability_registry(), &config);

        let read_file = registry
            .tools
            .iter()
            .find(|tool| tool.function.name == "cap_os_read_file")
            .unwrap();
        assert_eq!(read_file.function.strict, Some(true));
        assert!(provider_tool_strict_compatible("os.read_file"));
        assert!(provider_tool_schema_is_strict_compatible(
            &read_file.function.parameters
        ));
        assert_eq!(
            read_file.function.parameters["additionalProperties"],
            Value::Bool(false)
        );
        let required = read_file.function.parameters["required"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect::<BTreeSet<_>>();
        assert!(required.contains("path"));
        assert!(required.contains("reason"));

        let complete = registry
            .tools
            .iter()
            .find(|tool| tool.function.name == "cap_process_complete")
            .unwrap();
        assert_eq!(complete.function.strict, None);

        let write_artifact = registry
            .tools
            .iter()
            .find(|tool| tool.function.name == "cap_os_write_artifact")
            .unwrap();
        assert_eq!(write_artifact.function.strict, None);
        assert!(!provider_tool_strict_compatible("os.write_artifact"));
    }

    #[test]
    fn deepseek_phase7_thinking_native_tools_omit_provider_tool_choice() {
        let mut config = phase4_native_tool_config();
        config.thinking.mode = ThinkingMode::Enabled;
        config.tool_calling.tool_choice = ToolChoicePolicy::Required;
        assert_eq!(provider_tool_choice_value(&config), None);

        config.tool_calling.tool_choice = ToolChoicePolicy::Auto;
        assert_eq!(provider_tool_choice_value(&config), None);

        config.thinking.mode = ThinkingMode::Disabled;
        config.tool_calling.tool_choice = ToolChoicePolicy::Required;
        assert_eq!(provider_tool_choice_value(&config), Some(json!("required")));

        config.tool_calling.tool_choice = ToolChoicePolicy::Auto;
        assert_eq!(provider_tool_choice_value(&config), Some(json!("auto")));
    }

    #[test]
    fn deepseek_phase7_multi_tool_calls_are_serialized_with_batch_and_index() {
        let workspace = temp_workspace("deepseek_phase7_multi_tool_serialized");
        fs::create_dir_all(workspace.join("docs")).unwrap();
        fs::write(
            workspace.join("docs").join("multi.txt"),
            "phase7 multi tool content",
        )
        .unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_http_request(&mut stream);
            assert!(request.contains("\"tools\""));
            assert!(request.contains("\"cap_os_read_file\""));
            assert!(request.contains("\"cap_process_complete\""));
            let body = deepseek_tool_calls_body(
                vec![
                    (
                        "call_multi_read",
                        "cap_os_read_file",
                        json!({"path": "docs/multi.txt", "reason": "Read before closing."}),
                    ),
                    (
                        "call_multi_complete",
                        "cap_process_complete",
                        json!({
                            "completion_statement": "Read docs/multi.txt and closed in the same provider tool batch.",
                            "claimed_artifacts": [],
                            "key_sources": ["docs/multi.txt"],
                            "known_limitations": [],
                            "user_review_notes": [],
                            "reason": "Complete after the read receipt."
                        }),
                    ),
                ],
                "One assistant response can request a read and a completion; Kernel must serialize them.",
            );
            write_json_response(&mut stream, &body);
        });

        let provider = DeepSeekModelProvider::new(
            "test-key",
            format!("http://{}", addr),
            "deepseek-v4",
            5_000,
        )
        .with_streaming(false);
        let controller =
            RootAgentProcessController::with_model_provider(&workspace, Arc::new(provider))
                .unwrap();
        let mut config = phase4_native_tool_config();
        config.tool_calling.max_tool_calls_per_subturn = 4;
        config.tool_calling.max_tool_calls_per_task = 4;
        let result = controller
            .start_job_with_config("Read docs/multi.txt and then complete", Some(1), config)
            .unwrap();
        handle.join().unwrap();

        assert_eq!(result.status, "completed");
        let truth = ProcessTruthStore::new(&workspace, &result.job_id).unwrap();
        let events = truth.read_events().unwrap();
        let subturn = events
            .iter()
            .find(|event| event.event_type == "provider_tool_call_loop_subturn_started")
            .unwrap();
        assert_eq!(subturn.data["tool_call_count"], 2);
        assert_eq!(subturn.data["parallel_execution"], false);
        assert_eq!(subturn.data["mutation_parallel_execution_allowed"], false);
        assert_eq!(subturn.data["serialized_multi_tool_calls"], true);
        let batch_id = subturn
            .data
            .get("provider_tool_batch_id")
            .and_then(Value::as_str)
            .unwrap()
            .to_string();

        let decoded = events
            .iter()
            .filter(|event| event.event_type == "provider_tool_call_decoded")
            .collect::<Vec<_>>();
        assert_eq!(decoded.len(), 2);
        assert_eq!(decoded[0].data["provider_tool_call_id"], "call_multi_read");
        assert_eq!(decoded[0].data["provider_tool_call_index"], 0);
        assert_eq!(decoded[0].data["provider_tool_batch_id"], batch_id);
        assert_eq!(
            decoded[1].data["provider_tool_call_id"],
            "call_multi_complete"
        );
        assert_eq!(decoded[1].data["provider_tool_call_index"], 1);
        assert_eq!(decoded[1].data["provider_tool_batch_id"], batch_id);

        let recorded = events
            .iter()
            .filter(|event| event.event_type == "provider_tool_result_recorded")
            .collect::<Vec<_>>();
        let multi_recorded = recorded
            .iter()
            .filter(|event| {
                matches!(
                    event
                        .data
                        .get("provider_tool_call_id")
                        .and_then(Value::as_str),
                    Some("call_multi_read" | "call_multi_complete")
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(multi_recorded.len(), 2);
        assert_eq!(multi_recorded[0].data["provider_tool_call_index"], 0);
        assert_eq!(multi_recorded[1].data["provider_tool_call_index"], 1);
        assert_eq!(multi_recorded[0].data["provider_tool_batch_id"], batch_id);
        assert_eq!(multi_recorded[1].data["provider_tool_batch_id"], batch_id);

        let transcript =
            replay_provider_transcript_state(&truth, "deepseek", "deepseek_chat_completions")
                .unwrap()
                .unwrap();
        let messages = read_provider_messages(&truth, &transcript).unwrap();
        let tool_message_ids = messages
            .iter()
            .filter(|message| message.role == "tool")
            .filter_map(|message| message.tool_call_id.as_deref())
            .collect::<Vec<_>>();
        assert_eq!(
            tool_message_ids,
            vec!["call_multi_read", "call_multi_complete"]
        );
    }

    #[test]
    fn deepseek_phase7_per_subturn_tool_call_budget_interrupts_explicitly() {
        let workspace = temp_workspace("deepseek_phase7_subturn_budget");
        fs::create_dir_all(workspace.join("docs")).unwrap();
        fs::write(workspace.join("docs").join("budget.txt"), "budget").unwrap();
        fs::write(workspace.join("delete_budget.txt"), "delete budget").unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_http_request(&mut stream);
            assert!(request.contains("\"tools\""));
            let body = deepseek_tool_calls_body(
                vec![
                    (
                        "call_budget_read",
                        "cap_os_read_file",
                        json!({"path": "docs/budget.txt", "reason": "Read one."}),
                    ),
                    (
                        "call_budget_delete",
                        "cap_os_delete_path",
                        json!({
                            "path": "delete_budget.txt",
                            "reason": "Second call exceeds per-subturn budget because it is a mutation apply tool."
                        }),
                    ),
                ],
                "Return too many non-read-only tool calls for the configured subturn budget.",
            );
            write_json_response(&mut stream, &body);
        });

        let provider = DeepSeekModelProvider::new(
            "test-key",
            format!("http://{}", addr),
            "deepseek-v4",
            5_000,
        )
        .with_streaming(false);
        let controller =
            RootAgentProcessController::with_model_provider(&workspace, Arc::new(provider))
                .unwrap();
        let mut config = phase4_native_tool_config();
        config.tool_calling.toolset_mode = ProviderToolsetMode::FullRegistered;
        config.tool_calling.max_tool_calls_per_subturn = 1;
        config.tool_calling.max_tool_calls_per_task = 8;
        let result = controller
            .start_job_with_config(
                "Trigger a per-subturn tool budget interruption",
                Some(1),
                config,
            )
            .unwrap();
        handle.join().unwrap();

        assert_eq!(result.status, "interrupted");
        let truth = ProcessTruthStore::new(&workspace, &result.job_id).unwrap();
        let events = truth.read_events().unwrap();
        assert!(events.iter().any(|event| {
            event.event_type == "provider_tool_loop_budget_exceeded"
                && event.data["budget_kind"] == "per_subturn_tool_call_limit"
                && event.data["error_code"] == "MODEL_TOOL_LOOP_BUDGET_EXCEEDED"
        }));
        assert!(events.iter().any(|event| {
            event.event_type == "provider_tool_protocol_error"
                && event.data["error_code"] == "MODEL_TOOL_LOOP_BUDGET_EXCEEDED"
        }));
        assert!(!events.iter().any(|event| {
            event.event_type == "process_action_validated"
                && event.data["capability_id"] == "os.read_file"
        }));
    }

    #[test]
    fn deepseek_phase7_readonly_multi_tool_batch_bypasses_per_subturn_limit() {
        let workspace = temp_workspace("deepseek_phase7_readonly_subturn_unlimited");
        fs::create_dir_all(workspace.join("docs")).unwrap();
        fs::write(
            workspace.join("docs").join("readonly.txt"),
            "readonly batch",
        )
        .unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let (mut first, _) = listener.accept().unwrap();
            let request = read_http_request(&mut first);
            assert!(request.contains("\"tools\""));
            let body = deepseek_tool_calls_body(
                vec![
                    (
                        "call_readonly_read",
                        "cap_os_read_file",
                        json!({"path": "docs/readonly.txt", "reason": "Read one file."}),
                    ),
                    (
                        "call_readonly_events",
                        "cap_process_query_events",
                        json!({"event_type": "job_created", "limit": 5}),
                    ),
                ],
                "Return multiple read-only tool calls.",
            );
            write_json_response(&mut first, &body);

            let (mut second, _) = listener.accept().unwrap();
            let request = read_http_request(&mut second);
            assert!(request.contains("call_readonly_read"));
            assert!(request.contains("call_readonly_events"));
            let body = deepseek_tool_call_body(
                "call_readonly_complete",
                "cap_process_complete",
                json!({
                    "completion_statement": "Read-only batch completed.",
                    "claimed_artifacts": [],
                    "key_sources": ["docs/readonly.txt"],
                    "known_limitations": [],
                    "user_review_notes": []
                }),
                "Complete after read-only batch.",
            );
            write_json_response(&mut second, &body);
        });

        let provider = DeepSeekModelProvider::new(
            "test-key",
            format!("http://{}", addr),
            "deepseek-v4",
            5_000,
        )
        .with_streaming(false);
        let controller =
            RootAgentProcessController::with_model_provider(&workspace, Arc::new(provider))
                .unwrap();
        let mut config = phase4_native_tool_config();
        config.tool_calling.max_tool_calls_per_subturn = 1;
        let result = controller
            .start_job_with_config("Allow read-only multi-tool batch", Some(1), config)
            .unwrap();
        handle.join().unwrap();

        assert_eq!(result.status, "completed");
        let truth = ProcessTruthStore::new(&workspace, &result.job_id).unwrap();
        let events = truth.read_events().unwrap();
        assert!(!events.iter().any(|event| {
            event.event_type == "provider_tool_loop_budget_exceeded"
                && event.data["budget_kind"] == "per_subturn_tool_call_limit"
        }));
        assert!(events.iter().any(|event| {
            event.event_type == "process_action_validated"
                && event.data["capability_id"] == "os.read_file"
        }));
        assert!(events.iter().any(|event| {
            event.event_type == "process_action_validated"
                && event.data["capability_id"] == "process.query_events"
        }));
    }

    #[test]
    fn deepseek_phase7_max_provider_subturns_soft_yields_after_successful_tool_result() {
        let workspace = temp_workspace("deepseek_phase7_subturn_soft_yield");
        fs::create_dir_all(workspace.join("docs")).unwrap();
        fs::write(workspace.join("docs").join("soft.txt"), "soft yield").unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let (mut first, _) = listener.accept().unwrap();
            let request = read_http_request(&mut first);
            assert!(request.contains("\"tools\""));
            assert!(request.contains("[Stable Kernel Contract]"));
            assert!(request.contains("[Current Toolset]"));
            let body = deepseek_tool_call_body(
                "call_soft_read",
                "cap_os_read_file",
                json!({"path": "docs/soft.txt", "reason": "Read before yielding."}),
                "Read once, then let the client checkpoint if needed.",
            );
            write_json_response(&mut first, &body);

            let (mut second, _) = listener.accept().unwrap();
            let request = read_http_request(&mut second);
            assert!(request.contains("\"role\":\"tool\""));
            assert!(request.contains("call_soft_read"));
            assert!(request.contains("soft yield"));
            let body = deepseek_tool_call_body(
                "call_soft_complete",
                "cap_process_complete",
                json!({
                    "completion_statement": "Read docs/soft.txt after a provider subturn soft yield.",
                    "claimed_artifacts": [],
                    "key_sources": ["docs/soft.txt"],
                    "known_limitations": [],
                    "user_review_notes": []
                }),
                "The provider transcript retained the previous tool result across TaskAgent turns.",
            );
            write_json_response(&mut second, &body);
        });

        let provider = DeepSeekModelProvider::new(
            "test-key",
            format!("http://{}", addr),
            "deepseek-v4",
            5_000,
        )
        .with_streaming(false);
        let controller =
            RootAgentProcessController::with_model_provider(&workspace, Arc::new(provider))
                .unwrap();
        let mut config = phase4_native_tool_config();
        config.tool_calling.max_provider_subturns = 1;
        config.tool_calling.max_tool_calls_per_subturn = 1;
        config.tool_calling.max_tool_calls_per_task = 8;
        let result = controller
            .start_job_with_config(
                "Read docs/soft.txt and complete after a soft yield",
                Some(2),
                config,
            )
            .unwrap();
        handle.join().unwrap();

        assert_eq!(result.status, "completed");
        assert_eq!(result.turn_count, 2);
        let truth = ProcessTruthStore::new(&workspace, &result.job_id).unwrap();
        let events = truth.read_events().unwrap();
        assert!(events.iter().any(|event| {
            event.event_type == "provider_tool_loop_soft_yield"
                && event.data["budget_kind"] == "max_provider_subturns"
                && event.data["status"] == "running"
                && event.data["successful_tool_results_this_loop"] == 1
        }));
        assert!(!events.iter().any(|event| {
            event.event_type == "provider_tool_loop_budget_exceeded"
                && event.data["budget_kind"] == "max_provider_subturns"
        }));
        assert!(events.iter().any(|event| {
            event.event_type == "task_agent_turn_completed"
                && event.data["turn_index"] == 1
                && event.data["status"] == "running"
        }));
        assert!(events.iter().any(|event| {
            event.event_type == "provider_tool_call_loop_tool_completed"
                && event.data["provider_tool_call_id"] == "call_soft_complete"
                && event.data["executed_tool_calls_total"] == 2
        }));
    }

    #[test]
    fn deepseek_phase7_per_task_tool_call_budget_interrupts_runaway_loop() {
        let workspace = temp_workspace("deepseek_phase7_task_budget");
        fs::create_dir_all(workspace.join("docs")).unwrap();
        fs::write(workspace.join("docs").join("loop.txt"), "loop").unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            for call_id in ["call_loop_read_1", "call_loop_read_2"] {
                let (mut stream, _) = listener.accept().unwrap();
                let request = read_http_request(&mut stream);
                assert!(request.contains("\"tools\""));
                let body = deepseek_tool_call_body(
                    call_id,
                    "cap_os_read_file",
                    json!({"path": "docs/loop.txt", "reason": "Repeat read instead of completing."}),
                    "Keep reading and never complete.",
                );
                write_json_response(&mut stream, &body);
            }
        });

        let provider = DeepSeekModelProvider::new(
            "test-key",
            format!("http://{}", addr),
            "deepseek-v4",
            5_000,
        )
        .with_streaming(false);
        let controller =
            RootAgentProcessController::with_model_provider(&workspace, Arc::new(provider))
                .unwrap();
        let mut config = phase4_native_tool_config();
        config.tool_calling.max_tool_calls_per_subturn = 1;
        config.tool_calling.max_tool_calls_per_task = 1;
        let result = controller
            .start_job_with_config(
                "Interrupt a provider tool loop that keeps reading",
                Some(1),
                config,
            )
            .unwrap();
        handle.join().unwrap();

        assert_eq!(result.status, "interrupted");
        let truth = ProcessTruthStore::new(&workspace, &result.job_id).unwrap();
        let events = truth.read_events().unwrap();
        let successful_reads = events
            .iter()
            .filter(|event| {
                event.event_type == "capability_receipt"
                    && event.data["capability_id"] == "os.read_file"
                    && event.data["status"] == "success"
            })
            .count();
        assert_eq!(successful_reads, 1);
        assert!(events.iter().any(|event| {
            event.event_type == "provider_tool_loop_budget_exceeded"
                && event.data["budget_kind"] == "per_task_tool_call_limit"
                && event.data["error_code"] == "MODEL_TOOL_LOOP_BUDGET_EXCEEDED"
        }));
        assert!(events.iter().any(|event| {
            event.event_type == "provider_tool_protocol_error"
                && event.data["error_code"] == "MODEL_TOOL_LOOP_BUDGET_EXCEEDED"
        }));
    }

    #[test]
    fn model_runtime_retries_retryable_deepseek_failures_and_records_attempt_events() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            for attempt in 1..=3 {
                let (mut stream, _) = listener.accept().unwrap();
                let request = read_http_request(&mut stream);
                assert!(request.contains("Authorization: Bearer test-key"));
                let response = if attempt < 3 {
                    let body = format!(r#"{{"error":"temporary upstream failure {attempt}"}}"#);
                    format!(
                        "HTTP/1.1 500 Internal Server Error\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    )
                } else {
                    let body = r#"{"choices":[{"message":{"content":"Recovered after retry."},"finish_reason":"stop"}],"usage":{"total_tokens":21}}"#;
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    )
                };
                stream.write_all(response.as_bytes()).unwrap();
            }
        });

        let workspace = temp_workspace("deepseek_retry_provider");
        let (job, process, truth) =
            create_agent_job(&workspace, "Retry transient DeepSeek failures").unwrap();
        let instruction_ref = truth
            .write_blob("model_inputs/instruction.txt", b"Summarize.")
            .unwrap();
        let source_ref = truth
            .write_blob("model_inputs/source.txt", b"Source text.")
            .unwrap();
        let token = CapabilityToken {
            token_id: "token_deepseek_retry".to_string(),
            job_id: job.job_id.clone(),
            pid: process.pid.clone(),
            workspace_root: workspace.display().to_string(),
            capabilities: vec!["model.summarize".to_string()],
            permissions: vec!["model:invoke".to_string()],
        };
        let provider = DeepSeekModelProvider::new(
            "test-key",
            format!("http://{}", addr),
            "deepseek-v4",
            5_000,
        )
        .with_streaming(false);
        let receipt = ModelRuntime::new(truth.clone(), token, std::sync::Arc::new(provider))
            .summarize(ModelAction {
                action_id: "act_deepseek_retry".to_string(),
                job_id: job.job_id,
                pid: process.pid,
                reasoning_step_id: "reason_deepseek_retry".to_string(),
                operation: ModelOperation::Summarize,
                instruction_ref,
                input_refs: vec![source_ref],
                preference_snapshot_ref: None,
                output_schema: json!({"type": "string"}),
                provider: "deepseek".to_string(),
                model: "deepseek-v4".to_string(),
                budget: ModelBudget {
                    max_retries: 2,
                    ..ModelBudget::default()
                },
                failure_policy: ModelFailurePolicy::FailClosed,
                required: true,
            })
            .unwrap();
        handle.join().unwrap();

        assert_eq!(receipt.status, "success");
        assert_eq!(receipt.attempts, 3);
        assert_eq!(receipt.retry_count, 2);
        let event_types: Vec<String> = truth
            .read_events()
            .unwrap()
            .into_iter()
            .map(|event| event.event_type)
            .collect();
        assert_eq!(
            event_types
                .iter()
                .filter(|event| event.as_str() == "model_call_attempt_started")
                .count(),
            3
        );
        assert_eq!(
            event_types
                .iter()
                .filter(|event| event.as_str() == "model_call_attempt_failed")
                .count(),
            2
        );
        assert_eq!(
            event_types
                .iter()
                .filter(|event| event.as_str() == "model_call_attempt_backoff")
                .count(),
            2
        );
        assert!(event_types.contains(&"model_call_attempt_succeeded".to_string()));
        assert!(event_types.contains(&"model_call_completed".to_string()));
    }

    #[test]
    fn deepseek_provider_routes_next_action_to_flash_and_generation_to_pro_model() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_http_request(&mut stream);
            assert!(request.contains("Authorization: Bearer test-key"));
            assert!(request.contains("\"model\":\"deepseek-v4-flash\""));
            assert!(request.contains("\"stream\":false"));
            let body = r#"{"choices":[{"message":{"content":"No further action required."},"finish_reason":"stop"}],"usage":{"total_tokens":18}}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).unwrap();
        });

        let workspace = temp_workspace("deepseek_routing_provider");
        let (job, process, truth) =
            create_agent_job(&workspace, "Route complex next-action reasoning to pro").unwrap();
        let instruction_ref = truth
            .write_blob(
                "model_inputs/instruction.txt",
                b"Return a JSON next action.",
            )
            .unwrap();
        let token = CapabilityToken {
            token_id: "token_deepseek_route".to_string(),
            job_id: job.job_id.clone(),
            pid: process.pid.clone(),
            workspace_root: workspace.display().to_string(),
            capabilities: vec!["model.decide_next_action".to_string()],
            permissions: vec!["model:invoke".to_string()],
        };
        let provider = DeepSeekModelProvider::new(
            "test-key",
            format!("http://{}", addr),
            "deepseek-v4-flash",
            5_000,
        )
        .with_route_models("deepseek-v4-flash", "deepseek-v4-pro")
        .with_streaming(false);
        assert_eq!(
            provider.model_name_for_operation(&ModelOperation::RenderEntityReply),
            "deepseek-v4-flash"
        );
        assert_eq!(
            provider.model_name_for_operation(&ModelOperation::DecideNextAction),
            "deepseek-v4-flash"
        );
        assert_eq!(
            provider.model_name_for_operation(&ModelOperation::GenerateArtifact),
            "deepseek-v4-pro"
        );
        let receipt = ModelRuntime::new(truth, token, std::sync::Arc::new(provider))
            .decide_next_action(ModelAction {
                action_id: "act_deepseek_route".to_string(),
                job_id: job.job_id,
                pid: process.pid,
                reasoning_step_id: "reason_deepseek_route".to_string(),
                operation: ModelOperation::DecideNextAction,
                instruction_ref,
                input_refs: vec!["source_set_ref://route_test/source".to_string()],
                preference_snapshot_ref: None,
                output_schema: json!({"type": "object"}),
                provider: "deepseek".to_string(),
                model: "deepseek-v4-flash".to_string(),
                budget: ModelBudget::default(),
                failure_policy: ModelFailurePolicy::FailClosed,
                required: true,
            })
            .unwrap();
        handle.join().unwrap();

        assert_eq!(receipt.status, "success");
        assert_eq!(receipt.model, "deepseek-v4-flash");
        assert_eq!(
            receipt.provider_capability_snapshot["routing"]["simple_model"],
            "deepseek-v4-flash"
        );
        assert_eq!(
            receipt.provider_capability_snapshot["routing"]["complex_model"],
            "deepseek-v4-pro"
        );
    }

    #[test]
    fn deepseek_provider_streams_long_text_without_wall_timeout() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_http_request(&mut stream);
            assert!(request.contains("Authorization: Bearer test-key"));
            assert!(request.contains("Accept: text/event-stream"));
            assert!(request.contains("\"stream\":true"));
            assert!(request.contains("\"stream_options\":{\"include_usage\":true}"));
            let body = concat!(
                "data: {\"choices\":[{\"delta\":{\"role\":\"assistant\",\"content\":\"第一段内容。\"},\"finish_reason\":null}],\"usage\":null}\r\n\r\n",
                "data: {\"choices\":[{\"delta\":{\"content\":\"第二段内容。\"},\"finish_reason\":null}],\"usage\":null}\r\n\r\n",
                "data: {\"choices\":[],\"usage\":{\"total_tokens\":33}}\r\n\r\n",
                "data: {\"choices\":[{\"delta\":{\"content\":\"最终段落。\"},\"finish_reason\":\"stop\"}],\"usage\":null}\r\n\r\n",
                "data: [DONE]\r\n\r\n"
            );
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n{}",
                body
            );
            stream.write_all(response.as_bytes()).unwrap();
        });

        let workspace = temp_workspace("deepseek_streaming_provider");
        let (job, process, truth) =
            create_agent_job(&workspace, "Generate long content with streaming").unwrap();
        let instruction_ref = truth
            .write_blob("model_inputs/instruction.txt", b"Write a long deliverable.")
            .unwrap();
        let source_ref = truth
            .write_blob(
                "model_inputs/source.txt",
                b"Source material for long generation.",
            )
            .unwrap();
        let token = CapabilityToken {
            token_id: "token_deepseek_stream".to_string(),
            job_id: job.job_id.clone(),
            pid: process.pid.clone(),
            workspace_root: workspace.display().to_string(),
            capabilities: vec!["model.generate_artifact".to_string()],
            permissions: vec!["model:invoke".to_string()],
        };
        let provider = DeepSeekModelProvider::new(
            "test-key",
            format!("http://{}", addr),
            "deepseek-v4",
            5_000,
        )
        .with_streaming(true)
        .with_stream_timeouts(2_000, 5_000, 0);
        let receipt = ModelRuntime::new(truth.clone(), token, std::sync::Arc::new(provider))
            .generate_artifact(ModelAction {
                action_id: "act_deepseek_stream".to_string(),
                job_id: job.job_id,
                pid: process.pid,
                reasoning_step_id: "reason_deepseek_stream".to_string(),
                operation: ModelOperation::GenerateArtifact,
                instruction_ref,
                input_refs: vec![source_ref],
                preference_snapshot_ref: None,
                output_schema: json!({"type": "string"}),
                provider: "deepseek".to_string(),
                model: "deepseek-v4".to_string(),
                budget: ModelBudget::default(),
                failure_policy: ModelFailurePolicy::FailClosed,
                required: true,
            })
            .unwrap();
        handle.join().unwrap();

        assert_eq!(receipt.status, "success");
        assert!(receipt.streaming);
        assert_eq!(receipt.finish_reason.as_deref(), Some("stop"));
        assert_eq!(receipt.chunks_count, 3);
        assert!(receipt.stream_event_count >= 4);
        assert_eq!(receipt.idle_timeout_ms, Some(5_000));
        assert_eq!(receipt.usage["total_tokens"], 33);
        let output_ref = receipt.output_ref.as_ref().unwrap();
        let output_path = truth.resolve_blob_ref(output_ref).unwrap();
        let output = fs::read_to_string(output_path).unwrap();
        assert!(output.contains("第一段内容。第二段内容。最终段落。"));
    }

    #[test]
    fn deepseek_streaming_provider_records_reasoning_content_delta() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_http_request(&mut stream);
            assert!(request.contains("Accept: text/event-stream"));
            assert!(request.contains("\"stream\":true"));
            let body = include_str!(
                "../../tests/v2/fixtures/deepseek/streaming_reasoning_content_delta.sse"
            );
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n{}",
                body
            );
            stream.write_all(response.as_bytes()).unwrap();
        });

        let workspace = temp_workspace("deepseek_streaming_reasoning");
        let (job, process, truth) =
            create_agent_job(&workspace, "Generate content with streaming reasoning").unwrap();
        let instruction_ref = truth
            .write_blob("model_inputs/instruction.txt", b"Generate.")
            .unwrap();
        let source_ref = truth
            .write_blob("model_inputs/source.txt", b"Source text.")
            .unwrap();
        let token = CapabilityToken {
            token_id: "token_deepseek_stream_reasoning".to_string(),
            job_id: job.job_id.clone(),
            pid: process.pid.clone(),
            workspace_root: workspace.display().to_string(),
            capabilities: vec!["model.generate_artifact".to_string()],
            permissions: vec!["model:invoke".to_string()],
        };
        let provider = DeepSeekModelProvider::new(
            "test-key",
            format!("http://{}", addr),
            "deepseek-v4",
            5_000,
        )
        .with_streaming(true)
        .with_stream_timeouts(2_000, 5_000, 0);
        let receipt = ModelRuntime::new(truth.clone(), token, std::sync::Arc::new(provider))
            .generate_artifact(ModelAction {
                action_id: "act_deepseek_stream_reasoning".to_string(),
                job_id: job.job_id,
                pid: process.pid,
                reasoning_step_id: "reason_deepseek_stream_reasoning".to_string(),
                operation: ModelOperation::GenerateArtifact,
                instruction_ref,
                input_refs: vec![source_ref],
                preference_snapshot_ref: None,
                output_schema: json!({"type": "string"}),
                provider: "deepseek".to_string(),
                model: "deepseek-v4".to_string(),
                budget: ModelBudget::default(),
                failure_policy: ModelFailurePolicy::FailClosed,
                required: true,
            })
            .unwrap();
        handle.join().unwrap();

        assert_eq!(receipt.status, "success");
        assert!(receipt.streaming);
        assert!(receipt.output_ref.is_some());
        assert!(receipt.reasoning_content_tokens_estimated > 0);
        let reasoning_ref = receipt.reasoning_content_ref.as_deref().unwrap();
        let reasoning = fs::read_to_string(truth.resolve_blob_ref(reasoning_ref).unwrap()).unwrap();
        assert!(reasoning.contains("First reasoning chunk"));
        assert!(reasoning.contains("Second reasoning chunk"));
    }

    #[test]
    fn deepseek_stream_idle_timeout_fails_without_success_output_ref() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_http_request(&mut stream);
            assert!(request.contains("Accept: text/event-stream"));
            let headers =
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n";
            stream.write_all(headers.as_bytes()).unwrap();
            stream
                .write_all(
                    b"data: {\"choices\":[{\"delta\":{\"content\":\"Partial\"},\"finish_reason\":null}],\"usage\":null}\r\n\r\n",
                )
                .unwrap();
            stream.flush().unwrap();
            thread::sleep(StdDuration::from_millis(120));
        });

        let workspace = temp_workspace("deepseek_stream_idle_timeout");
        let (job, process, truth) =
            create_agent_job(&workspace, "Streaming idle timeout must fail explicitly").unwrap();
        let instruction_ref = truth
            .write_blob("model_inputs/instruction.txt", b"Generate long content.")
            .unwrap();
        let source_ref = truth
            .write_blob("model_inputs/source.txt", b"Source material.")
            .unwrap();
        let token = CapabilityToken {
            token_id: "token_deepseek_stream_idle".to_string(),
            job_id: job.job_id.clone(),
            pid: process.pid.clone(),
            workspace_root: workspace.display().to_string(),
            capabilities: vec!["model.generate_artifact".to_string()],
            permissions: vec!["model:invoke".to_string()],
        };
        let provider = DeepSeekModelProvider::new(
            "test-key",
            format!("http://{}", addr),
            "deepseek-v4",
            2_000,
        )
        .with_streaming(true)
        .with_stream_timeouts(1_000, 20, 0);
        let receipt = ModelRuntime::new(truth.clone(), token, std::sync::Arc::new(provider))
            .generate_artifact(ModelAction {
                action_id: "act_deepseek_stream_idle".to_string(),
                job_id: job.job_id,
                pid: process.pid,
                reasoning_step_id: "reason_deepseek_stream_idle".to_string(),
                operation: ModelOperation::GenerateArtifact,
                instruction_ref,
                input_refs: vec![source_ref],
                preference_snapshot_ref: None,
                output_schema: json!({"type": "string"}),
                provider: "deepseek".to_string(),
                model: "deepseek-v4".to_string(),
                budget: ModelBudget {
                    max_retries: 0,
                    ..ModelBudget::default()
                },
                failure_policy: ModelFailurePolicy::FailClosed,
                required: true,
            })
            .unwrap();
        handle.join().unwrap();

        assert_eq!(receipt.status, "failed");
        assert_eq!(
            receipt.error.as_ref().unwrap().error_code,
            "DEEPSEEK_STREAM_IDLE_TIMEOUT"
        );
        assert!(receipt.output_ref.is_none());
        let events = truth.read_events().unwrap();
        assert!(events.iter().any(|event| {
            event.event_type == "model_call_attempt_failed"
                && event.data["error"]["error_code"] == "DEEPSEEK_STREAM_IDLE_TIMEOUT"
        }));
        let replay = truth.replay().unwrap();
        assert_eq!(replay.status, "running");
    }

    #[test]
    fn deepseek_provider_rejects_truncated_finish_reason() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_http_request(&mut stream);
            assert!(request.contains("Authorization: Bearer test-key"));
            let body = r#"{"choices":[{"message":{"content":"Partial generated content."},"finish_reason":"length"}],"usage":{"total_tokens":8192}}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).unwrap();
        });

        let workspace = temp_workspace("deepseek_truncated_provider");
        let (job, process, truth) =
            create_agent_job(&workspace, "Generate content but provider truncates").unwrap();
        let instruction_ref = truth
            .write_blob(
                "model_inputs/instruction.txt",
                b"Generate a complete artifact.",
            )
            .unwrap();
        let source_ref = truth
            .write_blob("model_inputs/source.txt", b"Source material.")
            .unwrap();
        let token = CapabilityToken {
            token_id: "token_deepseek_length".to_string(),
            job_id: job.job_id.clone(),
            pid: process.pid.clone(),
            workspace_root: workspace.display().to_string(),
            capabilities: vec!["model.generate_artifact".to_string()],
            permissions: vec!["model:invoke".to_string()],
        };
        let provider = DeepSeekModelProvider::new(
            "test-key",
            format!("http://{}", addr),
            "deepseek-v4",
            5_000,
        )
        .with_streaming(false);
        let receipt = ModelRuntime::new(truth.clone(), token, std::sync::Arc::new(provider))
            .generate_artifact(ModelAction {
                action_id: "act_deepseek_length".to_string(),
                job_id: job.job_id,
                pid: process.pid,
                reasoning_step_id: "reason_deepseek_length".to_string(),
                operation: ModelOperation::GenerateArtifact,
                instruction_ref,
                input_refs: vec![source_ref],
                preference_snapshot_ref: None,
                output_schema: json!({"type": "string"}),
                provider: "deepseek".to_string(),
                model: "deepseek-v4".to_string(),
                budget: ModelBudget::default(),
                failure_policy: ModelFailurePolicy::FailClosed,
                required: true,
            })
            .unwrap();
        handle.join().unwrap();

        assert_eq!(receipt.status, "failed");
        assert_eq!(
            receipt.error.as_ref().unwrap().error_code,
            "MODEL_OUTPUT_TRUNCATED"
        );
        let replay = truth.replay().unwrap();
        assert_eq!(replay.status, "running");
    }

    #[test]
    fn phase6_task_agent_runtime_model_drives_workspace_tree_with_checkpoints() {
        let workspace = temp_workspace("phase6_tree");
        fs::create_dir_all(workspace.join("notes")).unwrap();
        fs::write(workspace.join("notes").join("alpha.txt"), "alpha").unwrap();
        let provider = SequencedModelProvider::new("sequenced", "phase6-tree-session")
            .with_tool_call_outputs(
                ModelOperation::DecideNextAction,
                vec![
                    vec![provider_tool_call("call_list_tree", "os.list_tree", json!({"max_depth":8}))],
                    vec![provider_tool_call("call_write_tree", "os.write_artifact", json!({"path":"TREE.md","content":"# Workspace Tree\n\n- notes/alpha.txt\n"}))],
                    vec![provider_tool_call("call_verify_tree", "os.verify_artifact", json!({"path":"TREE.md"}))],
                    vec![provider_tool_call("call_audit_tree", "artifact.audit_quality", json!({"artifact_path":"TREE.md","minimum_chars":20,"require_source_refs":false}))],
                    vec![provider_tool_call("call_complete_tree", "process.complete", json!({"completion_statement":"TREE.md exists, names notes/alpha.txt, and satisfies the requested workspace tree output.","claimed_artifacts":["TREE.md"],"key_sources":["notes/alpha.txt"],"known_limitations":[],"user_review_notes":"Open TREE.md to review the generated workspace tree."}))],
                ],
            );
        let controller = RootAgentProcessController::with_model_provider(
            &workspace,
            std::sync::Arc::new(provider),
        )
        .unwrap();

        let result = controller
            .start_job("Generate TREE.md for this workspace")
            .unwrap();

        assert_eq!(result.status, "completed");
        assert_eq!(result.waiting_for, None);
        assert!(result.artifacts.contains(&"TREE.md".to_string()));
        assert!(workspace.join("TREE.md").exists());
        let tree = fs::read_to_string(workspace.join("TREE.md")).unwrap();
        assert!(tree.contains("notes/alpha.txt"));
        assert!(result.turn_count >= 1);
        assert!(!result.checkpoints.is_empty());
        let truth = ProcessTruthStore::new(&workspace, &result.job_id).unwrap();
        let event_types: Vec<String> = truth
            .read_events()
            .unwrap()
            .into_iter()
            .map(|event| event.event_type)
            .collect();
        assert_eq!(
            event_types
                .iter()
                .filter(|event| event.as_str() == "provider_tool_call_decoded")
                .count(),
            5
        );
        for expected in [
            "task_agent_session_started",
            "task_agent_turn_started",
            "task_agent_observation_built",
            "provider_tool_call_decoded",
            "process_action_emitted",
            "process_action_validated",
            "agent_tool_action_validated",
            "agent_tool_action_executed",
            "checkpoint_saved",
            "task_agent_session_completed",
            "job_completed",
        ] {
            assert!(event_types.contains(&expected.to_string()), "{expected}");
        }
        let replay = truth.replay().unwrap();
        assert_eq!(replay.status, "completed");
        assert!(replay.artifact_refs.contains(&"TREE.md".to_string()));
    }

    #[test]
    fn phase_a_complete_records_statement_without_separate_completion_tool() {
        let workspace = temp_workspace("phase_a_complete_statement_only");
        let provider = SequencedModelProvider::new("sequenced", "phase-a-complete")
            .with_tool_call_outputs(
                ModelOperation::DecideNextAction,
                vec![
                    vec![provider_tool_call("call_write_report", "os.write_artifact", json!({"path":"REPORT.md","content":"# Report\n\nThis is the requested user-facing artifact."}))],
                    vec![provider_tool_call("call_complete_report", "process.complete", json!({
                        "completion_statement":"Generated REPORT.md as the requested deliverable.",
                        "claimed_artifacts":["REPORT.md"],
                        "key_sources":[],
                        "known_limitations":[],
                        "user_review_notes":["Open REPORT.md and review the generated content."]
                    }))],
                ],
            );
        let controller =
            RootAgentProcessController::with_model_provider(&workspace, Arc::new(provider))
                .unwrap();

        let result = controller
            .start_job("Create REPORT.md and close with completion statement")
            .unwrap();

        assert_eq!(result.status, "completed");
        assert!(workspace.join("REPORT.md").exists());
        let truth = ProcessTruthStore::new(&workspace, &result.job_id).unwrap();
        let events = truth.read_events().unwrap();
        assert!(events.iter().any(|event| {
            event.event_type == "completion_statement_recorded"
                && event.data["claimed_artifacts"]
                    .as_array()
                    .is_some_and(|items| items.iter().any(|item| item == "REPORT.md"))
        }));
        assert!(events.iter().any(|event| {
            event.event_type == "job_completed"
                && event.data["completion_statement"]
                    == "Generated REPORT.md as the requested deliverable."
        }));
    }

    #[test]
    fn phase_e_assistant_content_without_tool_calls_does_not_lose_artifact_facts() {
        let workspace = temp_workspace("phase_e_assistant_content");
        let provider = SequencedModelProvider::new("sequenced", "phase-e-interrupt")
            .with_tool_call_outputs(
                ModelOperation::DecideNextAction,
                vec![
                    vec![provider_tool_call("call_write_boundary", "os.write_artifact", json!({"path":"CAPABILITY_BOUNDARY_NOTE.md","content":"# Capability Boundary\n\nPDF/XLSX semantic extraction is not available in this run; use text/docx sources or provide converted text."}))],
                    Vec::new(),
                    vec![provider_tool_call("call_complete_boundary", "process.complete", json!({"completion_statement":"Recorded CAPABILITY_BOUNDARY_NOTE.md and preserved the artifact fact after an intermediate assistant content turn.","claimed_artifacts":["CAPABILITY_BOUNDARY_NOTE.md"],"key_sources":[],"known_limitations":[],"user_review_notes":"Review CAPABILITY_BOUNDARY_NOTE.md for the boundary note."}))],
                ],
            )
            .with_outputs(
                ModelOperation::DecideNextAction,
                vec!["I created CAPABILITY_BOUNDARY_NOTE.md.".to_string()],
            );
        let controller =
            RootAgentProcessController::with_model_provider(&workspace, Arc::new(provider))
                .unwrap();

        let result = controller
            .start_job("Explain PDF/XLSX extraction boundaries")
            .unwrap();

        assert_eq!(result.status, "completed");
        assert!(workspace.join("CAPABILITY_BOUNDARY_NOTE.md").exists());
        assert!(result
            .artifacts
            .contains(&"CAPABILITY_BOUNDARY_NOTE.md".to_string()));
        let truth = ProcessTruthStore::new(&workspace, &result.job_id).unwrap();
        let replay = truth.replay().unwrap();
        assert_eq!(replay.status, "completed");
        assert!(replay
            .artifact_refs
            .contains(&"CAPABILITY_BOUNDARY_NOTE.md".to_string()));
        let events = truth.read_events().unwrap();
        assert!(events
            .iter()
            .any(|event| event.event_type == "provider_native_assistant_content_yielded"));
        assert!(events.iter().any(|event| {
            event.event_type == "job_completed"
                && event.data["claimed_artifacts"]
                    .as_array()
                    .is_some_and(|items| items.iter().any(|item| item == "CAPABILITY_BOUNDARY_NOTE.md"))
        }));
    }

    #[test]
    fn phase6_task_agent_rejects_provider_native_model_runtime_tool_call() {
        let workspace = temp_workspace("phase6_model_artifact");
        let provider = SequencedModelProvider::new("sequenced", "phase6-model")
            .with_tool_call_outputs(
                ModelOperation::DecideNextAction,
                vec![
                    vec![provider_tool_call("call_model_generate", "model.generate_artifact", json!({"instruction":"Generate a concise artifact from the provided goal ref.","source_refs":["{{input_ref_0}}"],"output_schema":{"type":"string"}}))],
                    vec![provider_tool_call("call_complete_model_generate", "process.complete", json!({"completion_statement":"The model generation capability returned a successful receipt.","claimed_artifacts":[],"key_sources":[],"known_limitations":[],"user_review_notes":[]}))],
                ],
            );
        let controller = RootAgentProcessController::with_model_provider(
            &workspace,
            std::sync::Arc::new(provider),
        )
        .unwrap();

        let result = controller
            .start_job("Generate model artifact with LLM for this workspace")
            .unwrap();

        assert_eq!(result.status, "completed");
        assert!(result.artifacts.is_empty());
        let truth = ProcessTruthStore::new(&workspace, &result.job_id).unwrap();
        let events = truth.read_events().unwrap();
        let event_types: Vec<String> = events
            .iter()
            .map(|event| event.event_type.clone())
            .collect();
        assert!(events.iter().any(|event| {
            event.event_type == "provider_tool_protocol_error"
                && event.data["error_code"] == "PROVIDER_TOOL_MODEL_CAPABILITY_FORBIDDEN"
                && event.data["provider_tool_name"] == "cap_model_generate_artifact"
        }));
        assert!(!events.iter().any(|event| {
            event.event_type == "model_call_receipt"
                && event.data["capability_id"] == "model.generate_artifact"
        }));
        assert!(event_types.contains(&"job_completed".to_string()));
        let ledger_path = workspace
            .join(RUNTIME_DIR_NAME)
            .join("model_call_ledger")
            .join(&result.job_id)
            .join("model_call_ledger.json");
        assert!(ledger_path.exists());
    }

    #[test]
    fn phase6_interactive_session_model_drives_generic_task_to_verified_artifact() {
        let workspace = temp_workspace("phase6_interactive_session");
        fs::create_dir_all(workspace.join("projects")).unwrap();
        fs::write(
            workspace.join("projects").join("alpha.md"),
            "Alpha project has delivery risk and a Friday owner review.",
        )
        .unwrap();
        let provider = SequencedModelProvider::new("sequenced", "phase6-session-model")
            .with_tool_call_outputs(
                ModelOperation::DecideNextAction,
                vec![
                    vec![provider_tool_call("call_write_summary", "os.write_artifact", json!({"path":"PROJECT_SUMMARY.md","content":"# Project Summary\n\nSource: projects/alpha.md\n\nAlpha project risk summary."}))],
                    vec![provider_tool_call("call_verify_summary", "os.verify_artifact", json!({"path":"PROJECT_SUMMARY.md"}))],
                    vec![provider_tool_call("call_audit_summary", "artifact.audit_quality", json!({"artifact_path":"PROJECT_SUMMARY.md","minimum_chars":40,"require_source_refs":true}))],
                    vec![provider_tool_call("call_complete_summary", "process.complete", json!({"completion_statement":"PROJECT_SUMMARY.md is readable, source-backed, and summarizes projects/alpha.md for the requested task.","claimed_artifacts":["PROJECT_SUMMARY.md"],"key_sources":["projects/alpha.md"],"known_limitations":[],"user_review_notes":"Review PROJECT_SUMMARY.md as the final deliverable."}))],
                ],
            );
        let controller = RootAgentProcessController::with_model_provider(
            &workspace,
            std::sync::Arc::new(provider),
        )
        .unwrap();

        let result = controller
            .start_job("Create a project summary from available workspace materials")
            .unwrap();

        assert_eq!(result.status, "completed");
        assert!(result.turn_count >= 1);
        assert!(result.artifacts.contains(&"PROJECT_SUMMARY.md".to_string()));
        let artifact = fs::read_to_string(workspace.join("PROJECT_SUMMARY.md")).unwrap();
        assert!(artifact.contains("Alpha project risk summary"));
        let truth = ProcessTruthStore::new(&workspace, &result.job_id).unwrap();
        let events = truth.read_events().unwrap();
        let event_types: Vec<String> = events
            .iter()
            .map(|event| event.event_type.clone())
            .collect();
        assert_eq!(
            event_types
                .iter()
                .filter(|event| event.as_str() == "provider_tool_call_decoded")
                .count(),
            4
        );
        for expected in [
            "task_agent_session_started",
            "task_agent_turn_started",
            "task_agent_observation_built",
            "provider_tool_call_decoded",
            "agent_tool_action_executed",
            "task_agent_session_completed",
            "job_completed",
        ] {
            assert!(event_types.contains(&expected.to_string()), "{expected}");
        }
        assert!(events.iter().any(|event| {
            event.event_type == "capability_receipt"
                && event.data["capability_id"] == "artifact.audit_quality"
                && event.data["status"] == "success"
        }));
        assert!(!events.iter().any(|event| {
            event.event_type == "provider_tool_protocol_error"
                && event.data["error_code"] == "PROVIDER_TOOL_MODEL_CAPABILITY_FORBIDDEN"
        }));
        let removed_static_step_event = ["task", "plan", "step", "started"].join("_");
        assert!(!event_types.contains(&removed_static_step_event));
    }

    #[test]
    fn phase6_interactive_session_fails_closed_after_receipt_feedback() {
        let workspace = temp_workspace("phase6_interactive_fail");
        let provider = SequencedModelProvider::new("sequenced", "phase6-session-fail")
            .with_tool_call_outputs(
                ModelOperation::DecideNextAction,
                vec![
                    vec![provider_tool_call("call_verify_missing", "os.verify_artifact", json!({"path":"MISSING.md"}))],
                    vec![provider_tool_call("call_fail_missing", "process.fail", json!({"reason":"The verify receipt shows MISSING.md does not exist; closing as explicit failure."}))],
                ],
            );
        let controller = RootAgentProcessController::with_model_provider(
            &workspace,
            std::sync::Arc::new(provider),
        )
        .unwrap();

        let result = controller
            .start_job("Run a generic task that verifies a missing artifact")
            .unwrap();

        assert_eq!(result.status, "failed");
        assert!(!workspace.join("MISSING.md").exists());
        let truth = ProcessTruthStore::new(&workspace, &result.job_id).unwrap();
        let events = truth.read_events().unwrap();
        assert!(events.iter().any(|event| {
            event.event_type == "verify_event"
                && event.data["capability_id"] == "os.verify_artifact"
                && event.data["status"] == "failed"
        }));
        assert!(events.iter().any(|event| {
            event.event_type == "job_failed"
                && event.data.to_string().contains("MISSING.md does not exist")
        }));
    }

    #[test]
    fn phase6_interactive_session_observes_tool_failure_and_retries() {
        let workspace = temp_workspace("phase6_interactive_retry");
        fs::create_dir_all(workspace.join("docs")).unwrap();
        fs::write(workspace.join("docs").join("source.txt"), "source material").unwrap();
        let provider = SequencedModelProvider::new("sequenced", "phase6-session-retry")
            .with_tool_call_outputs(
                ModelOperation::DecideNextAction,
                vec![
                    vec![provider_tool_call("call_read_missing_arg", "os.read_file", json!({}))],
                    vec![provider_tool_call("call_read_retry", "os.read_file", json!({"path":"docs/source.txt"}))],
                    vec![provider_tool_call("call_complete_retry", "process.complete", json!({"completion_statement":"The source file was read successfully after retry.","claimed_artifacts":[],"key_sources":["docs/source.txt"],"known_limitations":[],"user_review_notes":[]}))],
                ],
            );
        let controller = RootAgentProcessController::with_model_provider(
            &workspace,
            std::sync::Arc::new(provider),
        )
        .unwrap();

        let result = controller
            .start_job("Read docs/source.txt and close after successful observation")
            .unwrap();

        assert_eq!(result.status, "completed");
        assert!(result.turn_count >= 1);
        let truth = ProcessTruthStore::new(&workspace, &result.job_id).unwrap();
        let events = truth.read_events().unwrap();
        assert!(
            events
                .iter()
                .filter(|event| event.event_type == "provider_tool_call_decoded")
                .count()
                >= 2
        );
        assert!(events.iter().any(|event| {
            event.event_type == "provider_tool_protocol_error"
                && event.data["capability_id"] == "os.read_file"
                && event.data["error_code"] == "PROVIDER_NATIVE_TOOL_ARGUMENTS_INVALID"
                && event.data["message"]
                    .as_str()
                    .is_some_and(|value| value.contains("path missing"))
        }));
        assert!(events.iter().any(|event| {
            event.event_type == "provider_tool_call_recoverable_error"
                && event.data["capability_id"] == "os.read_file"
                && event.data["next_model_request_should_self_correct"] == true
        }));
        assert!(events.iter().any(|event| {
            event.event_type == "capability_receipt"
                && event.data["capability_id"] == "os.read_file"
                && event.data["status"] == "success"
        }));
        assert!(events
            .iter()
            .any(|event| { event.event_type == "task_agent_session_completed" }));
    }

    #[test]
    #[ignore = "RC0 run-through disables approval pause in generic interactive dispatch."]
    fn phase6_interactive_session_dispatches_registered_os_and_terminal_capabilities() {
        let workspace = temp_workspace("phase6_registered_capabilities");
        fs::create_dir_all(workspace.join("docs")).unwrap();
        fs::write(workspace.join("docs").join("source.txt"), "source").unwrap();
        let powershell = std::env::var("SystemRoot")
            .map(|root| {
                Path::new(&root)
                    .join("System32")
                    .join("WindowsPowerShell")
                    .join("v1.0")
                    .join("powershell.exe")
            })
            .unwrap_or_else(|_| PathBuf::from("powershell.exe"))
            .display()
            .to_string();
        let provider = SequencedModelProvider::new("sequenced", "phase6-capability-session")
            .with_tool_call_outputs(
                ModelOperation::DecideNextAction,
                vec![
                    vec![provider_tool_call("call_preview_copy", "process.request_preview", json!({"proposed_actions":["os.copy_path"],"target_paths":["docs/source.txt","docs/source_copy.txt"],"preview_markdown":"# Copy Preview\n\nCopy docs/source.txt to docs/source_copy.txt."}))],
                    vec![provider_tool_call("call_copy_after_approval", "os.copy_path", json!({"source_path":"docs/source.txt","destination_path":"docs/source_copy.txt"}))],
                    vec![provider_tool_call("call_hash_copy", "os.hash_path", json!({"path":"docs/source_copy.txt"}))],
                    vec![provider_tool_call("call_zip_docs", "os.zip", json!({"source_paths":["docs"],"destination_zip_path":"out/docs.zip"}))],
                    vec![provider_tool_call("call_verify_zip", "artifact.verify_typed", json!({"artifact_path":"out/docs.zip"}))],
                    vec![provider_tool_call("call_terminal_inspect", "terminal.run_command", json!({"argv":[powershell,"-NoProfile","-NonInteractive","-Command","Get-ChildItem -LiteralPath . | Select-Object -First 1 | Out-String"],"timeout_ms":30000}))],
                    vec![provider_tool_call("call_complete_registered", "process.complete", json!({"completion_statement":"out/docs.zip was produced through registered copy/hash/zip/unzip/terminal capabilities and can be inspected by the user.","claimed_artifacts":["out/docs.zip"],"key_sources":["docs/source.txt","docs/source_copy.txt"],"known_limitations":[],"user_review_notes":"Inspect out/docs.zip and the unpacked output for evidence."}))],
                ],
            );
        let controller = RootAgentProcessController::with_model_provider(
            &workspace,
            std::sync::Arc::new(provider),
        )
        .unwrap();

        let waiting = controller
            .start_job("Use registered capabilities to copy, hash, archive, unpack, and run an approved command")
            .unwrap();
        assert_eq!(waiting.status, "waiting_approval");
        let result = controller
            .approve_preview(&waiting.job_id, "approve copy mutation")
            .unwrap();

        assert_eq!(result.status, "completed");
        assert!(workspace.join("docs").join("source_copy.txt").exists());
        assert!(workspace.join("out").join("docs.zip").exists());
        let truth = ProcessTruthStore::new(&workspace, &result.job_id).unwrap();
        let events = truth.read_events().unwrap();
        for capability_id in [
            "os.copy_path",
            "os.hash_path",
            "os.zip",
            "terminal.run_command",
        ] {
            assert!(
                events.iter().any(|event| {
                    event.event_type == "capability_receipt"
                        && event.data["capability_id"] == capability_id
                        && event.data["status"] == "success"
                }),
                "{capability_id}"
            );
        }
    }

    #[test]
    #[ignore = "RC0 run-through disables preview approval checkpoint resume."]
    fn phase6_preview_approval_resumes_from_process_truth_checkpoint() {
        let workspace = temp_workspace("phase6_preview");
        let provider = SequencedModelProvider::new("sequenced", "phase6-preview-session")
            .with_tool_call_outputs(
                ModelOperation::DecideNextAction,
                vec![
                    vec![provider_tool_call("call_preview_artifact", "process.request_preview", json!({"proposed_actions":["os.write_artifact"],"target_paths":["APPROVED_ARTIFACT.md"],"preview_markdown":"# Artifact Preview\n\nCreate APPROVED_ARTIFACT.md after approval."}))],
                    vec![provider_tool_call("call_write_approved", "os.write_artifact", json!({"path":"APPROVED_ARTIFACT.md","content":"# Approved Artifact\n\nCommitted after ProcessTruth approval."}))],
                    vec![provider_tool_call("call_verify_approved", "os.verify_artifact", json!({"path":"APPROVED_ARTIFACT.md"}))],
                    vec![provider_tool_call("call_audit_approved", "artifact.audit_quality", json!({"artifact_path":"APPROVED_ARTIFACT.md","minimum_chars":30,"require_source_refs":false}))],
                    vec![provider_tool_call("call_model_audit_approved", "model.audit_artifact_quality", json!({"artifact_path":"APPROVED_ARTIFACT.md"}))],
                    vec![provider_tool_call("call_complete_approved", "process.complete", json!({"completion_statement":"APPROVED_ARTIFACT.md was committed after approval and verified.","claimed_artifacts":["APPROVED_ARTIFACT.md"],"key_sources":[],"known_limitations":[],"user_review_notes":"Review APPROVED_ARTIFACT.md as the final approved artifact."}))],
                ],
            )
            .with_outputs(
                ModelOperation::Audit,
                vec![artifact_audit_pass_output("APPROVED_ARTIFACT.md")],
            );
        let controller = RootAgentProcessController::with_model_provider(
            &workspace,
            std::sync::Arc::new(provider),
        )
        .unwrap();

        let waiting = controller
            .start_job("Create preview artifact and wait for approval")
            .unwrap();

        assert_eq!(waiting.status, "waiting_approval");
        assert_eq!(waiting.waiting_for.as_deref(), Some("approval"));
        assert!(!workspace.join("APPROVED_ARTIFACT.md").exists());
        let resumed = controller
            .approve_preview(&waiting.job_id, "approved by test")
            .unwrap();
        assert_eq!(resumed.status, "completed");
        assert!(workspace.join("APPROVED_ARTIFACT.md").exists());
        let truth = ProcessTruthStore::new(&workspace, &waiting.job_id).unwrap();
        let events = truth.read_events().unwrap();
        let event_types: Vec<String> = events
            .iter()
            .map(|event| event.event_type.clone())
            .collect();
        assert!(event_types.contains(&"preview_created".to_string()));
        assert!(event_types.contains(&"user_approval_received".to_string()));
        assert!(event_types.contains(&"job_resumed".to_string()));
        assert!(event_types.contains(&"task_agent_session_resumed".to_string()));
        assert!(event_types.contains(&"task_context_state_updated".to_string()));
        assert!(event_types.contains(&"job_completed".to_string()));
        assert_eq!(
            event_types
                .iter()
                .filter(|event_type| event_type.as_str() == "task_agent_session_started")
                .count(),
            1
        );
        let turn_indices = events
            .iter()
            .filter(|event| event.event_type == "task_agent_turn_started")
            .filter_map(|event| event.data.get("turn_index").and_then(Value::as_u64))
            .collect::<Vec<_>>();
        assert_eq!(turn_indices, vec![1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn phase6_ambiguous_destructive_task_waits_for_user_without_mutation() {
        let workspace = temp_workspace("phase6_clarify");
        fs::write(workspace.join("old.txt"), "keep").unwrap();
        let provider = SequencedModelProvider::new("sequenced", "phase6-clarify-session")
            .with_tool_call_outputs(
                ModelOperation::DecideNextAction,
                vec![vec![provider_tool_call("call_clarify_delete", "process.clarify", json!({"question":"Which explicit paths should be deleted?"}))]],
            );
        let controller = RootAgentProcessController::with_model_provider(
            &workspace,
            std::sync::Arc::new(provider),
        )
        .unwrap();

        let result = controller.start_job("delete old files").unwrap();

        assert_eq!(result.status, "waiting_user");
        assert_eq!(result.waiting_for.as_deref(), Some("user_input"));
        assert!(workspace.join("old.txt").exists());
        assert!(result.artifacts.is_empty());
        let truth = ProcessTruthStore::new(&workspace, &result.job_id).unwrap();
        let event_types: Vec<String> = truth
            .read_events()
            .unwrap()
            .into_iter()
            .map(|event| event.event_type)
            .collect();
        assert!(event_types.contains(&"job_waiting_user".to_string()));
        assert!(event_types.contains(&"process_action_emitted".to_string()));
        assert!(!event_types.contains(&"os.delete_path".to_string()));
    }

    #[test]
    fn office_runtime_invokes_worker_records_receipts_and_rollback_tx() {
        let worker_project = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("office_worker")
            .join("SuperNova.OfficeWorker")
            .join("SuperNova.OfficeWorker.csproj");
        let build = Command::new("dotnet")
            .arg("build")
            .arg(&worker_project)
            .arg("--no-restore")
            .output();
        if !matches!(build, Ok(output) if output.status.success()) {
            eprintln!(
                "skipping OfficeRuntime integration test because .NET worker is not built/restored"
            );
            return;
        }

        let workspace = temp_workspace("office_runtime");
        fs::create_dir_all(workspace.join("docs")).unwrap();
        let (job, process, truth) = create_agent_job(&workspace, "Office runtime").unwrap();
        let token = CapabilityToken {
            token_id: "token_office".to_string(),
            job_id: job.job_id.clone(),
            pid: process.pid.clone(),
            workspace_root: workspace.display().to_string(),
            capabilities: vec![
                "office.docx.create".to_string(),
                "office.docx.read_text".to_string(),
                "office.docx.batch_read_text".to_string(),
                "office.docx.batch_extract_metadata".to_string(),
                "office.docx.batch_validate".to_string(),
                "office.docx.rewrite_save_as".to_string(),
                "office.docx.rewrite_in_place_preview".to_string(),
                "office.docx.rewrite_in_place".to_string(),
                "office.docx.validate".to_string(),
            ],
            permissions: vec!["office:read".to_string(), "office:write".to_string()],
        };
        let office = OfficeRuntime::new(
            WorkspaceGuard::new(&workspace).unwrap(),
            truth.clone(),
            token,
            worker_project,
        );

        let create = office
            .create_docx(
                "docs/source.docx",
                "Original paragraph one.\nOriginal paragraph two.",
                Some("Original title"),
            )
            .unwrap();
        assert_eq!(create.status, "success");
        let second = office
            .create_docx(
                "docs/second.docx",
                "Second document paragraph.\nSecond action paragraph.",
                Some("Second title"),
            )
            .unwrap();
        assert_eq!(second.status, "success");
        let source_set_token = CapabilityToken {
            token_id: "token_office_source_set".to_string(),
            job_id: job.job_id.clone(),
            pid: process.pid.clone(),
            workspace_root: workspace.display().to_string(),
            capabilities: vec!["source_set.create".to_string()],
            permissions: vec!["fs:read".to_string()],
        };
        let source_set = DataRuntime::new(
            WorkspaceGuard::new(&workspace).unwrap(),
            truth.clone(),
            source_set_token,
        )
        .create_source_set(".", &[".docx".to_string()], &[], &[], 8)
        .unwrap();
        let batch = office
            .batch_read_text(source_set.data["source_set_ref"].as_str().unwrap())
            .unwrap();
        assert_eq!(batch.status, "success");
        assert!(batch.data["raw_document_set_ref"].as_str().is_some());
        assert!(
            batch.data["worker_receipt"]["data"]["succeeded_files"]
                .as_u64()
                .unwrap()
                >= 2
        );
        let metadata = office
            .batch_extract_metadata(source_set.data["source_set_ref"].as_str().unwrap())
            .unwrap();
        assert_eq!(metadata.capability_id, "office.docx.batch_extract_metadata");
        assert_eq!(metadata.status, "success");
        assert!(metadata.data["dataset_ref"].as_str().is_some());
        let validation = office
            .batch_validate(source_set.data["source_set_ref"].as_str().unwrap())
            .unwrap();
        assert_eq!(validation.capability_id, "office.docx.batch_validate");
        assert_eq!(validation.status, "success");
        assert!(validation.data["validation_ref"].as_str().is_some());
        let read = office.read_text("docs/source.docx").unwrap();
        assert_eq!(read.status, "success");
        assert!(read.data["content_ref"].as_str().is_some());
        assert!(read.data["worker_receipt"]["data"]["text"]
            .as_str()
            .unwrap()
            .contains("Original paragraph one"));
        let save_as = office
            .rewrite_save_as(
                "docs/source.docx",
                "docs/leadership.docx",
                "# Leadership brief\n---\n## Key points\n- Confirm 98.7% coverage\n- Remove Markdown markers\n**Owner** keeps source facts.",
            )
            .unwrap();
        assert_eq!(save_as.status, "success");
        let save_as_read = office.read_text("docs/leadership.docx").unwrap();
        let save_as_text = save_as_read.data["worker_receipt"]["data"]["text"]
            .as_str()
            .unwrap();
        assert!(save_as_text.contains("Leadership brief"));
        assert!(save_as_text.contains("Key points"));
        assert!(save_as_text.contains("\u{2022} Confirm 98.7% coverage"));
        assert!(!save_as_text.contains("# Leadership brief"));
        assert!(!save_as_text.contains("---"));
        assert!(!save_as_text.contains("**Owner**"));
        let preview = office
            .preview_in_place_rewrite(
                "docs/source.docx",
                "Rewritten paragraph one.\nRewritten paragraph two.",
            )
            .unwrap();
        assert_eq!(preview.status, "success");
        assert_eq!(
            preview.data["worker_receipt"]["data"]["mutation_performed"],
            false
        );
        let rewrite = office
            .rewrite_in_place_with_approval(
                "docs/source.docx",
                "Rewritten paragraph one.\nRewritten paragraph two.",
                Some("approval_office_rewrite_test"),
            )
            .unwrap();
        assert_eq!(rewrite.status, "success");
        let tx_id = rewrite.data["tx_id"].as_str().unwrap();
        let rollback_token = CapabilityToken {
            token_id: "token_office_rollback".to_string(),
            job_id: job.job_id,
            pid: process.pid,
            workspace_root: workspace.display().to_string(),
            capabilities: vec!["os.rollback_tx".to_string()],
            permissions: vec!["fs:write".to_string()],
        };
        let os = OsRuntime::new(
            WorkspaceGuard::new(&workspace).unwrap(),
            truth.clone(),
            rollback_token,
        );
        os.rollback_tx(tx_id).unwrap();
        let restored = office.read_text("docs/source.docx").unwrap();
        assert!(restored.data["worker_receipt"]["data"]["text"]
            .as_str()
            .unwrap()
            .contains("Original paragraph one"));
        assert!(truth
            .read_events()
            .unwrap()
            .iter()
            .any(|event| event.event_type == "office_receipt"));
    }

    #[test]
    fn v2_runtime_capability_hardening_sourceset_dataset_package_and_artifact_review() {
        let workspace = temp_workspace("v2_capability_hardening_data_plane");
        fs::create_dir_all(workspace.join("docs")).unwrap();
        fs::write(workspace.join("docs").join("a.txt"), "same content").unwrap();
        fs::write(workspace.join("docs").join("b.txt"), "same content").unwrap();
        fs::write(workspace.join("docs").join("c.md"), "# Source\nunique").unwrap();
        let (job, process, truth) =
            create_agent_job(&workspace, "Data-plane capability hardening").unwrap();
        let token = CapabilityToken {
            token_id: "token_data_plane".to_string(),
            job_id: job.job_id.clone(),
            pid: process.pid.clone(),
            workspace_root: workspace.display().to_string(),
            capabilities: vec![
                "source_set.create".to_string(),
                "source_set.read_page".to_string(),
                "workspace.batch_hash".to_string(),
                "workspace.find_duplicates".to_string(),
                "workspace.recent_changes".to_string(),
                "dataset.export_csv".to_string(),
                "dataset.export_markdown".to_string(),
                "artifact.copy_source_set".to_string(),
                "package.build_zip".to_string(),
                "artifact.verify_coverage".to_string(),
                "artifact.verify_typed".to_string(),
                "artifact.audit_quality".to_string(),
            ],
            permissions: vec!["fs:read".to_string(), "fs:write".to_string()],
        };
        let data = DataRuntime::new(
            WorkspaceGuard::new(&workspace).unwrap(),
            truth.clone(),
            token.clone(),
        );
        let source_set = data
            .create_source_set(
                "docs",
                &[".txt".to_string(), ".md".to_string()],
                &[],
                &[],
                8,
            )
            .unwrap();
        assert_eq!(source_set.status, "success");
        let source_set_ref = source_set.data["source_set_ref"].as_str().unwrap();
        let page = data.read_source_set_page(source_set_ref, 0, 2).unwrap();
        assert_eq!(page.data["returned"], 2);
        let hashes = data.batch_hash(source_set_ref).unwrap();
        assert_eq!(hashes.data["row_count"], 3);
        let duplicates = data.find_duplicates(source_set_ref).unwrap();
        assert_eq!(duplicates.data["duplicate_group_count"], 1);
        let duplicate_dataset_ref = duplicates.data["dataset_ref"].as_str().unwrap();
        data.export_dataset_csv(duplicate_dataset_ref, "DUPLICATES.csv")
            .unwrap();
        data.export_dataset_markdown(duplicate_dataset_ref, "DUPLICATES.md", "Duplicates")
            .unwrap();
        data.recent_changes(source_set_ref, 7).unwrap();
        let package = PackageRuntime::new(
            WorkspaceGuard::new(&workspace).unwrap(),
            truth.clone(),
            token.clone(),
        )
        .build_zip(
            source_set_ref,
            "deliverable.zip",
            Some("PACK_MANIFEST.md"),
            Some("SHA256SUMS.txt"),
            Some("PERF_NOTES.json"),
            &[],
        )
        .unwrap();
        assert_eq!(package.status, "success");
        assert!(workspace.join("deliverable.zip").exists());
        assert!(workspace.join("PACK_MANIFEST.md").exists());
        assert!(workspace.join("SHA256SUMS.txt").exists());
        let artifact = ArtifactRuntime::new(
            WorkspaceGuard::new(&workspace).unwrap(),
            truth.clone(),
            token,
        );
        let coverage = artifact
            .verify_coverage("PACK_MANIFEST.md", Some(source_set_ref), None)
            .unwrap();
        assert_eq!(coverage.status, "success");
        for path in [
            "deliverable.zip",
            "PACK_MANIFEST.md",
            "SHA256SUMS.txt",
            "PERF_NOTES.json",
        ] {
            let typed = artifact.verify_typed_artifact(path).unwrap();
            assert_eq!(typed.status, "success", "{path}");
        }
        let quality = artifact
            .audit_quality("PACK_MANIFEST.md", 80, true)
            .unwrap();
        assert_eq!(quality.status, "success");
        let event_types = truth
            .read_events()
            .unwrap()
            .into_iter()
            .map(|event| event.event_type)
            .collect::<Vec<_>>();
        assert!(event_types.contains(&"artifact_review_receipt".to_string()));
    }

    #[test]
    #[ignore = "RC0 run-through bypasses preview-bound mutation blocking."]
    fn capability_kernel_dynamically_upgrades_existing_artifact_target_to_preview_bound_mutation() {
        let workspace = temp_workspace("v2_dynamic_artifact_policy");
        fs::write(workspace.join("REPORT.md"), "old report").unwrap();
        let (job, process, truth) =
            create_agent_job(&workspace, "Overwrite artifact only after approval").unwrap();
        let token = CapabilityToken {
            token_id: "token_dynamic_artifact_policy".to_string(),
            job_id: job.job_id,
            pid: process.pid,
            workspace_root: workspace.display().to_string(),
            capabilities: vec!["os.write_artifact".to_string()],
            permissions: vec!["fs:write".to_string()],
        };
        let registry = default_capability_registry();
        let descriptor = registry
            .iter()
            .find(|item| item.capability_id == "os.write_artifact")
            .unwrap();

        let new_request = build_capability_approval_request(
            &truth,
            descriptor,
            &json!({"path": "NEW_REPORT.md", "content": "new"}),
            None,
        )
        .unwrap();
        assert_eq!(new_request.policy, CapabilityApprovalPolicy::ArtifactCreate);
        assert!(prepare_capability_approval(&truth, &token, new_request)
            .unwrap()
            .unwrap()
            .is_none());

        let overwrite_request = build_capability_approval_request(
            &truth,
            descriptor,
            &json!({"path": "REPORT.md", "content": "replace"}),
            None,
        )
        .unwrap();
        assert_eq!(
            overwrite_request.policy,
            CapabilityApprovalPolicy::PreviewBoundMutation
        );
        let blocked = prepare_capability_approval(&truth, &token, overwrite_request)
            .unwrap()
            .unwrap_err();
        assert_eq!(blocked.status, "blocked");
        assert_eq!(
            blocked.data["reason"],
            "capability_requires_preview_approval"
        );
    }

    #[test]
    fn artifact_coverage_contract_row_per_source_is_fact_checked() {
        let workspace = temp_workspace("v2_coverage_contract");
        fs::create_dir_all(workspace.join("docs")).unwrap();
        fs::write(workspace.join("docs").join("a.md"), "alpha").unwrap();
        fs::write(workspace.join("docs").join("b.md"), "beta").unwrap();
        fs::write(
            workspace.join("LEDGER.md"),
            "docs/a.md\ndocs/b.md\nsummary ledger\n",
        )
        .unwrap();
        let (job, process, truth) =
            create_agent_job(&workspace, "Validate row-per-source coverage").unwrap();
        let token = CapabilityToken {
            token_id: "token_coverage_contract".to_string(),
            job_id: job.job_id,
            pid: process.pid,
            workspace_root: workspace.display().to_string(),
            capabilities: vec![
                "source_set.create".to_string(),
                "artifact.verify_coverage".to_string(),
            ],
            permissions: vec!["fs:read".to_string()],
        };
        let source_set = DataRuntime::new(
            WorkspaceGuard::new(&workspace).unwrap(),
            truth.clone(),
            token.clone(),
        )
        .create_source_set("docs", &[".md".to_string()], &[], &[], 8)
        .unwrap();
        let source_set_ref = source_set.data["source_set_ref"].as_str().unwrap();
        let incomplete_dataset_ref = truth
            .write_blob(
                "datasets/incomplete_rows.json",
                &serde_json::to_vec_pretty(&DataSet {
                    dataset_id: "incomplete".to_string(),
                    schema: vec!["source_path".to_string()],
                    row_count: 1,
                    source_set_ref: Some(source_set_ref.to_string()),
                    derivation_type: "extractive".to_string(),
                    records: vec![json!({"source_path": "docs/a.md"})],
                    coverage_report: json!({}),
                })
                .unwrap(),
            )
            .unwrap();
        let artifact = ArtifactRuntime::new(WorkspaceGuard::new(&workspace).unwrap(), truth, token);
        let failed = artifact
            .verify_coverage_with_contract(
                "LEDGER.md",
                Some(source_set_ref),
                Some(&incomplete_dataset_ref),
                Some(&json!({"relation": "row_per_source"})),
            )
            .unwrap();
        assert_eq!(failed.status, "failed");
        assert_eq!(failed.data["contract_relation"], "row_per_source");
        assert!(failed.data["missing_sources"]
            .as_array()
            .unwrap()
            .contains(&json!("docs/b.md")));
    }

    #[test]
    fn local_artifact_audit_advisory_is_not_semantic_quality_failure() {
        let workspace = temp_workspace("v2_local_audit_advisory_semantics");
        fs::write(workspace.join("REPORT.md"), "# Report\n\nShort.\n").unwrap();
        let (job, process, truth) =
            create_agent_job(&workspace, "Local audit should be mechanical only").unwrap();
        let token = CapabilityToken {
            token_id: "token_local_audit".to_string(),
            job_id: job.job_id,
            pid: process.pid.clone(),
            workspace_root: workspace.display().to_string(),
            capabilities: vec!["artifact.audit_quality".to_string()],
            permissions: vec!["fs:read".to_string()],
        };
        let receipt = ArtifactRuntime::new(
            WorkspaceGuard::new(&workspace).unwrap(),
            truth.clone(),
            token,
        )
        .audit_quality("REPORT.md", 200, false)
        .unwrap();

        assert_eq!(receipt.status, "success");
        assert_eq!(receipt.data["audit_layer"], "local_mechanical");
        assert_eq!(receipt.data["semantic_quality_judgement"], false);
        assert_eq!(receipt.data["mechanical_audit_pass"], true);
        assert_eq!(receipt.data["hard_risk_pass"], true);
        assert!(receipt.data["advisory_issue_count"].as_u64().unwrap() > 0);
        assert!(receipt.data.get("quality_pass").is_none());
        assert!(receipt.data.get("human_acceptance_pass").is_none());

        let context = replay_task_context_state(&truth, &process.pid, &process.pid).unwrap();
        let artifact = context
            .artifact_table
            .iter()
            .find(|item| item.path == "REPORT.md")
            .unwrap();
        assert!(artifact.audited);
        assert!(artifact.local_audit_completed);
        assert_eq!(artifact.local_audit_hard_risk_pass, Some(true));
    }

    #[test]
    fn closure_gate_treats_negative_model_artifact_audit_as_advisory_without_hard_risk() {
        let workspace = temp_workspace("v2_negative_model_audit_closure");
        let (job, process, truth) =
            create_agent_job(&workspace, "Do not complete on negative audit").unwrap();
        let token = CapabilityToken {
            token_id: "token_negative_audit".to_string(),
            job_id: job.job_id.clone(),
            pid: process.pid.clone(),
            workspace_root: workspace.display().to_string(),
            capabilities: vec![
                "os.write_artifact".to_string(),
                "os.verify_artifact".to_string(),
                "artifact.audit_quality".to_string(),
            ],
            permissions: vec!["fs:read".to_string(), "fs:write".to_string()],
        };
        let guard = WorkspaceGuard::new(&workspace).unwrap();
        let os = OsRuntime::new(guard.clone(), truth.clone(), token.clone());
        os.write_artifact(
            "REPORT.md",
            b"# Report\n\nSource: docs/a.md\n\nThis is long enough for audit.",
        )
        .unwrap();
        os.verify_artifact("REPORT.md").unwrap();
        ArtifactRuntime::new(guard.clone(), truth.clone(), token.clone())
            .audit_quality("REPORT.md", 20, true)
            .unwrap();
        let audit_receipt = CapabilityReceipt {
            capability_id: "model.audit_artifact_quality".to_string(),
            job_id: job.job_id,
            pid: process.pid.clone(),
            status: "success".to_string(),
            data: json!({
                "artifact_path": "REPORT.md",
                "quality_pass": false,
                "human_acceptance_pass": false,
                "blocking_issue_count": 1,
                "audit_output": {
                    "blocking_issues": ["report is not actually deliverable"]
                }
            }),
        };
        truth
            .append_event(
                Some(&process.pid),
                "artifact_model_audit_receipt",
                to_json_value(&audit_receipt).unwrap(),
            )
            .unwrap();

        let replay = truth.replay().unwrap();
        let gate = check_closure_gate(&guard, &truth, &replay).unwrap();
        assert!(gate.can_complete, "{gate:#?}");
        assert!(gate.hard_blocks.is_empty(), "{gate:#?}");
        assert!(gate
            .advisory_findings
            .iter()
            .any(|finding| finding.source == "model.audit_artifact_quality"));
        assert!(gate
            .model_audit_gaps
            .iter()
            .any(|gap| gap.contains("unresolved model audit findings")));
    }

    #[test]
    fn closure_gate_blocks_model_audit_hard_artifact_leak_findings() {
        let workspace = temp_workspace("v2_model_audit_hard_finding");
        fs::write(
            workspace.join("REPORT.md"),
            "# Report\n\nSource: docs/a.md\n",
        )
        .unwrap();
        let (job, process, truth) =
            create_agent_job(&workspace, "Block hard audit finding").unwrap();
        truth
            .append_event(
                Some(&process.pid),
                "capability_receipt",
                json!({
                    "capability_id": "os.write_artifact",
                    "status": "success",
                    "artifact_path": "REPORT.md",
                }),
            )
            .unwrap();
        let audit_receipt = CapabilityReceipt {
            capability_id: "model.audit_artifact_quality".to_string(),
            job_id: job.job_id,
            pid: process.pid.clone(),
            status: "success".to_string(),
            data: json!({
                "artifact_path": "REPORT.md",
                "quality_pass": false,
                "human_acceptance_pass": false,
                "blocking_issue_count": 1,
                "audit_output": {
                    "blocking_issues": ["artifact exposes internal blob/dataset refs as user-facing sources"]
                }
            }),
        };
        truth
            .append_event(
                Some(&process.pid),
                "artifact_model_audit_receipt",
                to_json_value(&audit_receipt).unwrap(),
            )
            .unwrap();

        let guard = WorkspaceGuard::new(&workspace).unwrap();
        let replay = truth.replay().unwrap();
        let gate = check_closure_gate(&guard, &truth, &replay).unwrap();
        assert!(!gate.can_complete, "{gate:#?}");
        assert!(gate
            .hard_blocks
            .iter()
            .any(|block| block.code == "hard_verification_failure"));
    }

    #[test]
    #[ignore = "RC0 run-through no longer binds package preview receipts to approval transactions."]
    fn capability_kernel_binds_package_build_zip_to_approval_transaction() {
        let workspace = temp_workspace("v2_package_approval_kernel");
        fs::create_dir_all(workspace.join("docs")).unwrap();
        fs::write(workspace.join("docs").join("a.txt"), "alpha").unwrap();
        fs::write(workspace.join("docs").join("b.txt"), "beta").unwrap();
        let (job, process, truth) =
            create_agent_job(&workspace, "Package should consume approval tx").unwrap();
        let token = CapabilityToken {
            token_id: "token_package_approval".to_string(),
            job_id: job.job_id.clone(),
            pid: process.pid.clone(),
            workspace_root: workspace.display().to_string(),
            capabilities: vec![
                "source_set.create".to_string(),
                "package.build_zip".to_string(),
            ],
            permissions: vec!["fs:read".to_string(), "fs:write".to_string()],
        };
        let data = DataRuntime::new(
            WorkspaceGuard::new(&workspace).unwrap(),
            truth.clone(),
            token.clone(),
        );
        let source_set = data
            .create_source_set("docs", &[".txt".to_string()], &[], &[], 8)
            .unwrap();
        let source_set_ref = source_set.data["source_set_ref"].as_str().unwrap();
        let approval = ApprovalRuntime::new(truth.clone());
        approval
            .create_preview_tx(
                &process.pid,
                "# Package Preview\n\nBuild deliverable.zip from docs.",
                vec![ExecutablePreviewOperation {
                    capability_id: "package.build_zip".to_string(),
                    arguments: json!({}),
                    target_paths: vec![
                        "deliverable.zip".to_string(),
                        "PACK_MANIFEST.md".to_string(),
                        "SHA256SUMS.txt".to_string(),
                        "PERF_NOTES.json".to_string(),
                    ],
                    human_description: "Build deliverable.zip from docs.".to_string(),
                    rollback_policy: Some("remove_package_outputs".to_string()),
                }],
                "medium",
            )
            .unwrap();
        approval
            .issue_token_for_latest_preview(&process.pid, "approved by package test")
            .unwrap();
        let approval_guard = prepare_capability_approval(
            &truth,
            &token,
            CapabilityApprovalRequest {
                capability_id: "package.build_zip".to_string(),
                policy: CapabilityApprovalPolicy::PreviewBoundArtifact,
                target_paths: vec![
                    "deliverable.zip".to_string(),
                    "PACK_MANIFEST.md".to_string(),
                    "SHA256SUMS.txt".to_string(),
                    "PERF_NOTES.json".to_string(),
                ],
                target_path_schema: "package outputs".to_string(),
                explicit_approval_id: None,
            },
        )
        .unwrap()
        .unwrap()
        .unwrap();
        let package = PackageRuntime::new(
            WorkspaceGuard::new(&workspace).unwrap(),
            truth.clone(),
            token.clone(),
        )
        .build_zip(source_set_ref, "deliverable.zip", None, None, None, &[])
        .unwrap();
        assert_eq!(package.status, "success");
        finalize_capability_approval(&truth, &token.pid, Some(&approval_guard), &package).unwrap();
        let event_types = truth
            .read_events()
            .unwrap()
            .into_iter()
            .map(|event| event.event_type)
            .collect::<Vec<_>>();
        assert!(event_types.contains(&"approval_token_consumed".to_string()));
        assert!(event_types.contains(&"preview_tx_applied".to_string()));
        assert!(event_types.contains(&"approval_token_used".to_string()));
        assert!(event_types.contains(&"preview_tx_closed".to_string()));
        assert!(event_types.contains(&"capability_approval_finalized".to_string()));
    }

    #[test]
    #[ignore = "RC0 run-through disables approval token authorization flow."]
    fn approval_token_from_preview_tx_authorizes_mutation_without_weakening_boundary() {
        let workspace = temp_workspace("v2_approval_token_runtime");
        fs::create_dir_all(workspace.join("docs")).unwrap();
        fs::write(workspace.join("docs").join("source.txt"), "approved").unwrap();
        let (job, process, truth) = create_agent_job(&workspace, "Approval token runtime").unwrap();
        let approval = ApprovalRuntime::new(truth.clone());
        let preview = approval
            .create_preview_tx(
                &process.pid,
                "# Preview\n\nCopy docs/source.txt to docs/copied.txt.",
                vec![ExecutablePreviewOperation {
                    capability_id: "os.copy_path".to_string(),
                    arguments: json!({}),
                    target_paths: vec![
                        "docs/source.txt".to_string(),
                        "docs/copied.txt".to_string(),
                    ],
                    human_description: "Copy docs/source.txt to docs/copied.txt.".to_string(),
                    rollback_policy: Some("restore_previous_path_state".to_string()),
                }],
                "medium",
            )
            .unwrap();
        assert!(preview.preview_ref.starts_with("blob://"));
        let token_record = approval
            .issue_token_for_latest_preview(&process.pid, "approved by unit test")
            .unwrap();
        let token = CapabilityToken {
            token_id: "token_approval_runtime".to_string(),
            job_id: job.job_id,
            pid: process.pid,
            workspace_root: workspace.display().to_string(),
            capabilities: vec!["os.copy_path".to_string()],
            permissions: vec!["fs:read".to_string(), "fs:write".to_string()],
        };
        let os = OsRuntime::new(
            WorkspaceGuard::new(&workspace).unwrap(),
            truth.clone(),
            token.clone(),
        );
        let blocked = prepare_capability_approval(
            &truth,
            &token,
            CapabilityApprovalRequest {
                capability_id: "os.copy_path".to_string(),
                policy: CapabilityApprovalPolicy::SourceMutationRequired,
                target_paths: vec![
                    "docs/source.txt".to_string(),
                    "docs/blocked.txt".to_string(),
                ],
                target_path_schema: "source_path + destination_path".to_string(),
                explicit_approval_id: Some("bad_token".to_string()),
            },
        )
        .unwrap()
        .unwrap_err();
        assert_eq!(blocked.status, "blocked");
        assert!(!workspace.join("docs").join("blocked.txt").exists());
        let approval_guard = prepare_capability_approval(
            &truth,
            &token,
            CapabilityApprovalRequest {
                capability_id: "os.copy_path".to_string(),
                policy: CapabilityApprovalPolicy::SourceMutationRequired,
                target_paths: vec!["docs/source.txt".to_string(), "docs/copied.txt".to_string()],
                target_path_schema: "source_path + destination_path".to_string(),
                explicit_approval_id: Some(token_record.approval_token_id.clone()),
            },
        )
        .unwrap()
        .unwrap()
        .unwrap();
        let copied = os.copy_path("docs/source.txt", "docs/copied.txt").unwrap();
        finalize_capability_approval(&truth, &token.pid, Some(&approval_guard), &copied).unwrap();
        assert_eq!(copied.status, "success");
        assert!(workspace.join("docs").join("copied.txt").exists());
        let reused = prepare_capability_approval(
            &truth,
            &token,
            CapabilityApprovalRequest {
                capability_id: "os.copy_path".to_string(),
                policy: CapabilityApprovalPolicy::SourceMutationRequired,
                target_paths: vec![
                    "docs/source.txt".to_string(),
                    "docs/copied_again.txt".to_string(),
                ],
                target_path_schema: "source_path + destination_path".to_string(),
                explicit_approval_id: Some(token_record.approval_token_id.clone()),
            },
        )
        .unwrap()
        .unwrap_err();
        assert_eq!(reused.status, "blocked");
        assert!(!workspace.join("docs").join("copied_again.txt").exists());
        let event_types = truth
            .read_events()
            .unwrap()
            .into_iter()
            .map(|event| event.event_type)
            .collect::<Vec<_>>();
        assert!(event_types.contains(&"preview_tx_approved".to_string()));
        assert!(event_types.contains(&"approval_token_consumed".to_string()));
        assert!(event_types.contains(&"preview_tx_applied".to_string()));
        assert!(event_types.contains(&"approval_token_used".to_string()));
        assert!(event_types.contains(&"preview_tx_closed".to_string()));
    }

    #[test]
    fn approval_scope_uses_executable_operation_not_human_description() {
        let workspace = temp_workspace("v2_preview_executable_operation_scope");
        let (_job, process, truth) =
            create_agent_job(&workspace, "Executable preview operation scope").unwrap();
        let approval = ApprovalRuntime::new(truth.clone());
        let preview = approval
            .create_preview_tx(
                &process.pid,
                "# Preview\n\n生成交付压缩包、清单、校验和和性能记录。",
                vec![ExecutablePreviewOperation {
                    capability_id: "package.build_zip".to_string(),
                    arguments: json!({"source_set_ref": "blob://source_set"}),
                    target_paths: vec![
                        "deliverable.zip".to_string(),
                        "PACK_MANIFEST.md".to_string(),
                        "SHA256SUMS.txt".to_string(),
                        "PERF_NOTES.json".to_string(),
                    ],
                    human_description: "使用 package.build_zip 从源集生成交付包".to_string(),
                    rollback_policy: Some("remove_package_outputs".to_string()),
                }],
                "medium",
            )
            .unwrap();
        assert_eq!(
            preview.proposed_actions,
            vec!["package.build_zip".to_string()]
        );
        assert_eq!(
            preview.executable_operations[0].human_description,
            "使用 package.build_zip 从源集生成交付包"
        );
        let token = approval
            .issue_token_for_latest_preview(&process.pid, "approved executable operation")
            .unwrap();
        assert_eq!(
            token.approved_operation_scope,
            vec!["package.build_zip".to_string()]
        );
        assert!(approval
            .validate_token(
                &token.approval_token_id,
                "package.build_zip",
                &[
                    "deliverable.zip",
                    "PACK_MANIFEST.md",
                    "SHA256SUMS.txt",
                    "PERF_NOTES.json"
                ],
            )
            .unwrap());
        assert!(!approval
            .validate_token(
                &token.approval_token_id,
                "使用 package.build_zip 从源集生成交付包",
                &["deliverable.zip"],
            )
            .unwrap());
    }

    #[test]
    #[ignore = "RC0 run-through no longer creates preview transactions from preview capabilities."]
    fn preview_capability_binding_requires_explicit_transaction_scope() {
        let workspace = temp_workspace("v2_preview_scope");
        let (_job, process, truth) =
            create_agent_job(&workspace, "Preview scope must be explicit").unwrap();
        let receipt = CapabilityReceipt {
            capability_id: "workspace.rename_batch_preview".to_string(),
            job_id: process.job_id.clone(),
            pid: process.pid.clone(),
            status: "success".to_string(),
            data: json!({
                "preview_ref": "blob://rename_preview",
                "target_paths": ["drafts/a.docx", "drafts/b.docx"],
                "proposed_actions": ["workspace.rename_batch_apply"],
            }),
        };
        let descriptor = default_capability_registry()
            .into_iter()
            .find(|item| item.capability_id == "workspace.rename_batch_preview")
            .unwrap();

        let mut weak_receipt = receipt.clone();
        weak_receipt.data = json!({
            "preview_ref": "blob://rename_preview",
            "target_paths": ["drafts/a.docx"],
            "proposed_actions": ["*"],
        });
        let rejected =
            bind_preview_capability_receipt_to_tx(&truth, &process.pid, &descriptor, &weak_receipt)
                .unwrap();
        assert!(rejected.is_none());

        let preview =
            bind_preview_capability_receipt_to_tx(&truth, &process.pid, &descriptor, &receipt)
                .unwrap()
                .unwrap();
        assert_eq!(
            preview.proposed_actions,
            vec!["workspace.rename_batch_apply"]
        );
        assert_eq!(
            preview.target_paths,
            vec!["drafts/a.docx".to_string(), "drafts/b.docx".to_string()]
        );
        let token = ApprovalRuntime::new(truth.clone())
            .issue_token_for_latest_preview(&process.pid, "approved explicit scope")
            .unwrap();
        assert_eq!(
            token.approved_action_scope,
            vec!["workspace.rename_batch_apply".to_string()]
        );
        assert!(!token.approved_path_scope.contains(&"*".to_string()));
    }

    #[test]
    fn approval_runtime_rejects_empty_or_wildcard_preview_scope() {
        let workspace = temp_workspace("v2_preview_scope_runtime");
        let (_job, process, truth) =
            create_agent_job(&workspace, "Preview scope runtime invariant").unwrap();
        let approval = ApprovalRuntime::new(truth.clone());
        let empty = approval.create_preview_tx(&process.pid, "# Preview", vec![], "medium");
        assert!(empty.is_err());
        let wildcard = approval.create_preview_tx(
            &process.pid,
            "# Preview",
            vec![ExecutablePreviewOperation {
                capability_id: "os.copy_path".to_string(),
                arguments: json!({}),
                target_paths: vec!["*".to_string()],
                human_description: "Copy a file".to_string(),
                rollback_policy: None,
            }],
            "medium",
        );
        assert!(wildcard.is_err());
    }

    #[test]
    fn descriptor_approval_engine_extracts_plan_target_paths_without_wildcard() {
        let workspace = temp_workspace("v2_descriptor_approval_targets");
        let (_job, _process, truth) =
            create_agent_job(&workspace, "Descriptor approval target extraction").unwrap();
        let plan_ref = truth
            .write_blob(
                "workspace_plans/test_organize_plan.json",
                serde_json::to_vec_pretty(&json!({
                    "operations": [
                        {
                            "operation": "move",
                            "source_path": "inbox/a.docx",
                            "destination_path": "archive/docs/a.docx"
                        },
                        {
                            "operation": "move",
                            "source_path": "inbox/b.md",
                            "destination_path": "archive/markdown/b.md"
                        }
                    ]
                }))
                .unwrap()
                .as_slice(),
            )
            .unwrap();
        let descriptor = default_capability_registry()
            .into_iter()
            .find(|item| item.capability_id == "workspace.apply_organize_tx")
            .unwrap();
        let request = build_capability_approval_request(
            &truth,
            &descriptor,
            &json!({"organize_plan_ref": plan_ref}),
            None,
        )
        .unwrap();
        assert_eq!(request.policy, CapabilityApprovalPolicy::WorkspaceMutation);
        assert!(request.target_paths.contains(&"inbox/a.docx".to_string()));
        assert!(request
            .target_paths
            .contains(&"archive/docs/a.docx".to_string()));
        assert!(!request.target_paths.contains(&"*".to_string()));
        assert_eq!(request.target_path_schema, "organize_plan_ref target_paths");
    }

    #[test]
    fn rewrite_save_as_approval_scope_uses_output_path_not_read_input() {
        let workspace = temp_workspace("v2_rewrite_save_as_targets");
        fs::create_dir_all(workspace.join("drafts")).unwrap();
        fs::create_dir_all(workspace.join("deliverables")).unwrap();
        fs::write(workspace.join("drafts").join("source.docx"), b"placeholder").unwrap();
        let (_job, _process, truth) =
            create_agent_job(&workspace, "Rewrite docx save-as target extraction").unwrap();
        let descriptor = default_capability_registry()
            .into_iter()
            .find(|item| item.capability_id == "office.docx.rewrite_save_as")
            .unwrap();
        let request = build_capability_approval_request(
            &truth,
            &descriptor,
            &json!({
                "input_path": "drafts/source.docx",
                "output_path": "deliverables/leadership_brief.docx",
                "content_ref": "blob://job/model_outputs/rewrite.txt"
            }),
            None,
        )
        .unwrap();
        assert_eq!(request.policy, CapabilityApprovalPolicy::ArtifactCreate);
        assert_eq!(
            request.target_paths,
            vec!["deliverables/leadership_brief.docx".to_string()]
        );
        assert!(!request
            .target_paths
            .contains(&"drafts/source.docx".to_string()));
    }

    #[test]
    fn create_only_approval_policy_allows_copy_to_new_target_but_not_existing_target() {
        let workspace = temp_workspace("v2_copy_create_only_policy");
        fs::create_dir_all(workspace.join("docs")).unwrap();
        fs::write(workspace.join("docs").join("source.txt"), b"source").unwrap();
        fs::write(workspace.join("docs").join("existing.txt"), b"existing").unwrap();
        let (_job, _process, truth) =
            create_agent_job(&workspace, "Copy target approval policy").unwrap();
        let descriptor = default_capability_registry()
            .into_iter()
            .find(|item| item.capability_id == "os.copy_path")
            .unwrap();

        let new_target = build_capability_approval_request(
            &truth,
            &descriptor,
            &json!({
                "source_path": "docs/source.txt",
                "destination_path": "docs/new-copy.txt"
            }),
            None,
        )
        .unwrap();
        assert_eq!(new_target.policy, CapabilityApprovalPolicy::ArtifactCreate);
        assert_eq!(
            new_target.target_paths,
            vec!["docs/new-copy.txt".to_string()]
        );

        let existing_target = build_capability_approval_request(
            &truth,
            &descriptor,
            &json!({
                "source_path": "docs/source.txt",
                "destination_path": "docs/existing.txt"
            }),
            None,
        )
        .unwrap();
        assert_eq!(
            existing_target.policy,
            CapabilityApprovalPolicy::PreviewBoundMutation
        );
        assert_eq!(
            existing_target.target_paths,
            vec!["docs/existing.txt".to_string()]
        );
    }

    #[test]
    #[ignore = "RC0 run-through lets terminal mutations execute through hard boundaries without preview approval."]
    fn terminal_dynamic_approval_policy_allows_readonly_and_new_create_only() {
        let workspace = temp_workspace("v2_terminal_dynamic_approval_policy");
        fs::write(workspace.join("existing.txt"), b"existing").unwrap();
        let (job, process, truth) =
            create_agent_job(&workspace, "Terminal dynamic approval policy").unwrap();
        let token = CapabilityToken {
            token_id: "token_terminal_dynamic".to_string(),
            job_id: job.job_id,
            pid: process.pid,
            workspace_root: workspace.display().to_string(),
            capabilities: vec!["terminal.run_command".to_string()],
            permissions: vec!["terminal:execute".to_string()],
        };
        let descriptor = default_capability_registry()
            .into_iter()
            .find(|item| item.capability_id == "terminal.run_command")
            .unwrap();

        let readonly = build_capability_approval_request(
            &truth,
            &descriptor,
            &json!({"argv": ["git", "status"]}),
            None,
        )
        .unwrap();
        assert_eq!(readonly.policy, CapabilityApprovalPolicy::ReadOnly);
        assert!(prepare_capability_approval(&truth, &token, readonly)
            .unwrap()
            .unwrap()
            .is_none());

        let create_new = build_capability_approval_request(
            &truth,
            &descriptor,
            &json!({
                "argv": ["powershell.exe", "-NoProfile", "-Command", "Set-Content -LiteralPath new.txt -Value ok"],
                "target_paths": ["new.txt"]
            }),
            None,
        )
        .unwrap();
        assert_eq!(create_new.policy, CapabilityApprovalPolicy::ArtifactCreate);
        assert!(prepare_capability_approval(&truth, &token, create_new)
            .unwrap()
            .unwrap()
            .is_none());

        let existing = build_capability_approval_request(
            &truth,
            &descriptor,
            &json!({
                "argv": ["powershell.exe", "-NoProfile", "-Command", "Set-Content -LiteralPath existing.txt -Value ok"],
                "target_paths": ["existing.txt"]
            }),
            None,
        )
        .unwrap();
        assert_eq!(
            existing.policy,
            CapabilityApprovalPolicy::PreviewBoundMutation
        );
        let receipt = prepare_capability_approval(&truth, &token, existing)
            .unwrap()
            .unwrap_err();
        assert_eq!(receipt.status, "blocked");
        assert_eq!(receipt.data["approval_required"], true);
    }

    #[test]
    fn csv_reader_creates_dataset_ref_from_workspace_file() {
        let workspace = temp_workspace("v2_csv_reader_dataset");
        fs::write(
            workspace.join("sample.csv"),
            "name,score\nAlice,10\nBob,\"20,with comma\"\n",
        )
        .unwrap();
        let (job, process, truth) = create_agent_job(&workspace, "Read CSV").unwrap();
        let token = CapabilityToken {
            token_id: "token_csv_reader".to_string(),
            job_id: job.job_id,
            pid: process.pid,
            workspace_root: workspace.display().to_string(),
            capabilities: vec!["data.csv.read_dataset".to_string()],
            permissions: vec!["fs:read".to_string()],
        };
        let runtime = DataRuntime::new(WorkspaceGuard::new(&workspace).unwrap(), truth, token);

        let receipt = runtime.read_csv_dataset("sample.csv", true, 100).unwrap();

        assert_eq!(receipt.status, "success");
        assert_eq!(receipt.capability_id, "data.csv.read_dataset");
        assert_eq!(receipt.data["row_count"], 2);
        assert_eq!(receipt.data["schema"], json!(["name", "score"]));
        assert!(receipt.data["dataset_ref"]
            .as_str()
            .unwrap()
            .starts_with("blob://"));
    }

    #[test]
    #[ignore = "RC0 run-through no longer binds preview-only receipts into approval transactions."]
    fn preview_only_descriptor_binding_applies_to_plan_organize_receipts() {
        let workspace = temp_workspace("v2_preview_plan_organize_binding");
        let (_job, process, truth) =
            create_agent_job(&workspace, "Plan organize preview binding").unwrap();
        let descriptor = default_capability_registry()
            .into_iter()
            .find(|item| item.capability_id == "workspace.plan_organize")
            .unwrap();
        assert_eq!(descriptor.approval_policy, "preview_only");
        let receipt = CapabilityReceipt {
            capability_id: "workspace.plan_organize".to_string(),
            job_id: process.job_id.clone(),
            pid: process.pid.clone(),
            status: "success".to_string(),
            data: json!({
                "preview_ref": "blob://organize_preview",
                "target_paths": [
                    "inbox/a.docx",
                    "archive/by_project/docs/a.docx"
                ],
                "proposed_actions": ["workspace.apply_organize_tx"],
            }),
        };
        let preview =
            bind_preview_capability_receipt_to_tx(&truth, &process.pid, &descriptor, &receipt)
                .unwrap()
                .unwrap();
        assert_eq!(
            preview.proposed_actions,
            vec!["workspace.apply_organize_tx"]
        );
        assert!(preview
            .target_paths
            .contains(&"archive/by_project/docs/a.docx".to_string()));
    }

    #[test]
    fn closure_gate_treats_package_supporting_artifacts_by_role() {
        let workspace = temp_workspace("v2_artifact_roles");
        fs::create_dir_all(workspace.join("docs")).unwrap();
        fs::write(workspace.join("docs").join("a.txt"), "alpha").unwrap();
        let (job, process, truth) = create_agent_job(&workspace, "Package artifact roles").unwrap();
        let token = CapabilityToken {
            token_id: "token_artifact_roles".to_string(),
            job_id: job.job_id,
            pid: process.pid.clone(),
            workspace_root: workspace.display().to_string(),
            capabilities: vec![
                "source_set.create".to_string(),
                "package.build_zip".to_string(),
                "artifact.verify_typed".to_string(),
            ],
            permissions: vec!["fs:read".to_string(), "fs:write".to_string()],
        };
        let guard = WorkspaceGuard::new(&workspace).unwrap();
        let source_set = DataRuntime::new(guard.clone(), truth.clone(), token.clone())
            .create_source_set("docs", &[".txt".to_string()], &[], &[], 8)
            .unwrap();
        PackageRuntime::new(guard.clone(), truth.clone(), token.clone())
            .build_zip(
                source_set.data["source_set_ref"].as_str().unwrap(),
                "deliverable.zip",
                Some("PACK_MANIFEST.md"),
                Some("SHA256SUMS.txt"),
                Some("PERF_NOTES.json"),
                &[],
            )
            .unwrap();
        let artifact = ArtifactRuntime::new(guard.clone(), truth.clone(), token);
        for path in [
            "deliverable.zip",
            "PACK_MANIFEST.md",
            "SHA256SUMS.txt",
            "PERF_NOTES.json",
        ] {
            assert_eq!(
                artifact.verify_typed_artifact(path).unwrap().status,
                "success"
            );
        }
        let replay = truth.replay().unwrap();
        let gate = check_closure_gate(&guard, &truth, &replay).unwrap();
        assert!(gate.can_complete, "{gate:#?}");
        assert!(gate.artifact_roles.iter().any(|item| {
            item.artifact_path == "PERF_NOTES.json" && item.role == "supporting_artifact"
        }));
        assert!(gate
            .artifact_roles
            .iter()
            .any(|item| item.artifact_path == "deliverable.zip"
                && item.role == "required_user_artifact"));
    }

    #[test]
    fn closure_gate_allows_completion_with_advisory_findings_only() {
        let workspace = temp_workspace("v2_closure_advisory_only");
        fs::write(
            workspace.join("REPORT.md"),
            "# Report\n\nShort but present.\n",
        )
        .unwrap();
        let (_job, process, truth) =
            create_agent_job(&workspace, "Complete with advisory evidence").unwrap();
        truth
            .append_event(
                Some(&process.pid),
                "capability_receipt",
                json!({
                    "capability_id": "os.write_artifact",
                    "status": "success",
                    "artifact_path": "REPORT.md",
                }),
            )
            .unwrap();
        truth
            .append_event(
                Some(&process.pid),
                "capability_receipt",
                json!({
                    "capability_id": "artifact.audit_quality",
                    "status": "success",
                    "data": {
                        "audit_layer": "local_mechanical",
                        "artifact_path": "REPORT.md",
                        "mechanical_audit_pass": true,
                        "hard_risk_pass": true,
                        "local_mechanical_audit_completed": true,
                        "mechanical_findings": ["artifact is shorter than minimum_chars"],
                        "blocking_issues": [],
                        "advisory_issues": ["artifact is shorter than minimum_chars"],
                        "hard_risk_issues": []
                    }
                }),
            )
            .unwrap();

        let guard = WorkspaceGuard::new(&workspace).unwrap();
        let replay = truth.replay().unwrap();
        let gate = check_closure_gate(&guard, &truth, &replay).unwrap();
        assert!(gate.can_complete, "{gate:#?}");
        assert!(gate.hard_blocks.is_empty(), "{gate:#?}");
        assert!(!gate.advisory_findings.is_empty(), "{gate:#?}");
    }

    #[test]
    fn closure_gate_blocks_missing_artifact_and_required_model_failure() {
        let workspace = temp_workspace("v2_closure_hard_blocks");
        let (_job, process, truth) =
            create_agent_job(&workspace, "Block hard closure failures").unwrap();
        truth
            .append_event(
                Some(&process.pid),
                "capability_receipt",
                json!({
                    "capability_id": "os.write_artifact",
                    "status": "success",
                    "artifact_path": "MISSING.md",
                }),
            )
            .unwrap();
        truth
            .append_event(
                Some(&process.pid),
                "model_call_failed",
                json!({
                    "capability_id": "model.generate_artifact",
                    "required": true,
                    "error": {"error_code": "TRANSPORT_TIMEOUT"}
                }),
            )
            .unwrap();

        let guard = WorkspaceGuard::new(&workspace).unwrap();
        let replay = truth.replay().unwrap();
        let gate = check_closure_gate(&guard, &truth, &replay).unwrap();
        assert!(!gate.can_complete, "{gate:#?}");
        assert!(gate
            .hard_blocks
            .iter()
            .any(|item| item.code == "artifact_missing_or_unreadable"));
        assert!(gate
            .hard_blocks
            .iter()
            .any(|item| item.code == "unresolved_required_failure"));
    }

    #[test]
    fn phase_a_claimed_missing_artifact_blocks_completion_without_extra_ledger_gate() {
        let workspace = temp_workspace("phase_a_claimed_missing");
        let (_job, _process, truth) =
            create_agent_job(&workspace, "Claimed artifact must exist").unwrap();
        let guard = WorkspaceGuard::new(&workspace).unwrap();
        let replay = truth.replay().unwrap();
        let gate = check_closure_gate_for_claimed_artifacts(
            &guard,
            &truth,
            &replay,
            &["MISSING.md".to_string()],
        )
        .unwrap();

        assert!(!gate.can_complete, "{gate:#?}");
        assert!(gate
            .hard_blocks
            .iter()
            .any(|item| item.code == "artifact_missing_or_unreadable"));
    }

    #[test]
    #[ignore = "RC0 run-through allows artifact revision under Kernel receipts without user approval pause."]
    fn artifact_create_revision_of_job_artifact_requires_user_approval() {
        let workspace = temp_workspace("v2_artifact_version_tx");
        let (job, process, truth) =
            create_agent_job(&workspace, "Revise generated artifact").unwrap();
        let token = CapabilityToken {
            token_id: "token_artifact_version".to_string(),
            job_id: job.job_id,
            pid: process.pid.clone(),
            workspace_root: workspace.display().to_string(),
            capabilities: vec!["os.write_artifact".to_string(), "os.copy_path".to_string()],
            permissions: vec!["fs:read".to_string(), "fs:write".to_string()],
        };
        let os = OsRuntime::new(
            WorkspaceGuard::new(&workspace).unwrap(),
            truth.clone(),
            token.clone(),
        );
        os.write_artifact("REPORT.md", b"first").unwrap();
        ApprovalRuntime::new(truth.clone())
            .create_preview_tx(
                &process.pid,
                "# unrelated preview",
                vec![ExecutablePreviewOperation {
                    capability_id: "os.copy_path".to_string(),
                    arguments: json!({}),
                    target_paths: vec!["a.txt".to_string(), "b.txt".to_string()],
                    human_description: "Copy a.txt to b.txt".to_string(),
                    rollback_policy: None,
                }],
                "medium",
            )
            .unwrap();
        ApprovalRuntime::new(truth.clone())
            .issue_token_for_latest_preview(&process.pid, "active unrelated approval")
            .unwrap();
        let descriptor = default_capability_registry()
            .into_iter()
            .find(|item| item.capability_id == "os.write_artifact")
            .unwrap();
        let request = build_capability_approval_request(
            &truth,
            &descriptor,
            &json!({"path": "REPORT.md", "content": "second"}),
            None,
        )
        .unwrap();
        assert_eq!(
            request.policy,
            CapabilityApprovalPolicy::PreviewBoundMutation
        );
        let blocked = prepare_capability_approval(&truth, &token, request)
            .unwrap()
            .unwrap_err();
        assert_eq!(blocked.status, "blocked");
        assert_eq!(blocked.data["approval_required"], true);
        assert_eq!(
            fs::read_to_string(workspace.join("REPORT.md")).unwrap(),
            "first"
        );
    }

    #[test]
    #[ignore = "RC0 run-through allows existing target overwrite under Kernel receipts without user approval pause."]
    fn existing_user_file_overwrite_still_requires_approval() {
        let workspace = temp_workspace("v2_source_overwrite_requires_approval");
        fs::write(workspace.join("source.md"), "user source").unwrap();
        let (job, process, truth) =
            create_agent_job(&workspace, "Do not overwrite user source without approval").unwrap();
        let token = CapabilityToken {
            token_id: "token_source_overwrite".to_string(),
            job_id: job.job_id,
            pid: process.pid,
            workspace_root: workspace.display().to_string(),
            capabilities: vec!["os.write_artifact".to_string()],
            permissions: vec!["fs:write".to_string()],
        };
        let descriptor = default_capability_registry()
            .into_iter()
            .find(|item| item.capability_id == "os.write_artifact")
            .unwrap();
        let request = build_capability_approval_request(
            &truth,
            &descriptor,
            &json!({"path": "source.md", "content": "overwrite"}),
            None,
        )
        .unwrap();
        assert_eq!(
            request.policy,
            CapabilityApprovalPolicy::PreviewBoundMutation
        );
        let blocked = prepare_capability_approval(&truth, &token, request)
            .unwrap()
            .unwrap_err();
        assert_eq!(blocked.status, "blocked");
    }

    #[test]
    fn package_preview_scope_expands_implicit_outputs() {
        let paths = expand_preview_target_paths_for_actions(
            &["package.build_zip".to_string()],
            vec!["deliverable.zip".to_string()],
        );
        assert!(paths.contains(&"deliverable.zip".to_string()));
        assert!(paths.contains(&"PACK_MANIFEST.md".to_string()));
        assert!(paths.contains(&"SHA256SUMS.txt".to_string()));
        assert!(paths.contains(&"PERF_NOTES.json".to_string()));
        assert!(!paths.contains(&"*".to_string()));
    }
}
