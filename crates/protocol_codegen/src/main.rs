use std::fs;
use std::path::Path;

use local_runtime_protocol::{
    AppSettings, AppearanceSettings, ArtifactDestinationGuidance, ArtifactRecord,
    ArtifactTargetOption, ArtifactTargetRequest, ChatThreadRecord, ContainerMessage,
    ContainerRecord, ContainerSnapshot, ContextPack, ContextPackEstimate, DiagnosticsSnapshot,
    ForceCloseRequest, ForceCloseResult, ModelConfig, ModelConfigDescriptor, ProtocolErrorEnvelope,
    ProtocolEvent, ProviderApiSettings, ProviderApiTestRequest, ProviderApiTestResult,
    ProviderApiUpdateRequest, ReferenceSourceDirective, RunRecord, RuntimeEventPayload,
    RuntimeMeta, SourceCandidate, SourceCandidateRequest, SourceGuidance, StreamOpenRequest,
    TaskApprovalActionResult, TaskDetail, TaskDraftArtifactRecord, TaskRecord,
    TaskUserInputRequest, UiCapabilityManifest, WorkspaceActivation, WorkspaceRecord,
};
use schemars::{schema_for, JsonSchema};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let repo_root = std::env::current_dir()?;
    let schema_dir = repo_root.join("reports").join("protocol").join("schema");
    let generated_dir = repo_root
        .join("desktop_shell")
        .join("ui")
        .join("src")
        .join("protocol")
        .join("generated");
    fs::create_dir_all(&schema_dir)?;
    fs::create_dir_all(&generated_dir)?;

    write_schema::<RuntimeMeta>(&schema_dir, "runtime_meta")?;
    write_schema::<RuntimeEventPayload>(&schema_dir, "runtime_event_payload")?;
    write_schema::<ProtocolEvent<RuntimeEventPayload>>(&schema_dir, "runtime_protocol_event")?;
    write_schema::<StreamOpenRequest>(&schema_dir, "stream_open_request")?;
    write_schema::<UiCapabilityManifest>(&schema_dir, "ui_capability_manifest")?;
    write_schema::<WorkspaceRecord>(&schema_dir, "workspace_record")?;
    write_schema::<ContainerRecord>(&schema_dir, "container_record")?;
    write_schema::<ContainerSnapshot>(&schema_dir, "container_snapshot")?;
    write_schema::<ContainerMessage>(&schema_dir, "container_message")?;
    write_schema::<ChatThreadRecord>(&schema_dir, "chat_thread_record")?;
    write_schema::<RunRecord>(&schema_dir, "run_record")?;
    write_schema::<TaskRecord>(&schema_dir, "task_record")?;
    write_schema::<TaskDetail>(&schema_dir, "task_detail")?;
    write_schema::<TaskDraftArtifactRecord>(&schema_dir, "task_draft_artifact_record")?;
    write_schema::<TaskApprovalActionResult>(&schema_dir, "task_approval_action_result")?;
    write_schema::<TaskUserInputRequest>(&schema_dir, "task_user_input_request")?;
    write_schema::<ForceCloseRequest>(&schema_dir, "force_close_request")?;
    write_schema::<ForceCloseResult>(&schema_dir, "force_close_result")?;
    write_schema::<ArtifactRecord>(&schema_dir, "artifact_record")?;
    write_schema::<ArtifactDestinationGuidance>(&schema_dir, "artifact_destination_guidance")?;
    write_schema::<ArtifactTargetRequest>(&schema_dir, "artifact_target_request")?;
    write_schema::<ArtifactTargetOption>(&schema_dir, "artifact_target_option")?;
    write_schema::<ContextPack>(&schema_dir, "context_pack")?;
    write_schema::<ContextPackEstimate>(&schema_dir, "context_pack_estimate")?;
    write_schema::<ReferenceSourceDirective>(&schema_dir, "reference_source_directive")?;
    write_schema::<SourceGuidance>(&schema_dir, "source_guidance")?;
    write_schema::<SourceCandidate>(&schema_dir, "source_candidate")?;
    write_schema::<SourceCandidateRequest>(&schema_dir, "source_candidate_request")?;
    write_schema::<ModelConfig>(&schema_dir, "model_config")?;
    write_schema::<ModelConfigDescriptor>(&schema_dir, "model_config_descriptor")?;
    write_schema::<AppSettings>(&schema_dir, "app_settings")?;
    write_schema::<AppearanceSettings>(&schema_dir, "appearance_settings")?;
    write_schema::<ProviderApiSettings>(&schema_dir, "provider_api_settings")?;
    write_schema::<ProviderApiUpdateRequest>(&schema_dir, "provider_api_update_request")?;
    write_schema::<ProviderApiTestRequest>(&schema_dir, "provider_api_test_request")?;
    write_schema::<ProviderApiTestResult>(&schema_dir, "provider_api_test_result")?;
    write_schema::<WorkspaceActivation>(&schema_dir, "workspace_activation")?;
    write_schema::<DiagnosticsSnapshot>(&schema_dir, "diagnostics_snapshot")?;
    write_schema::<ProtocolErrorEnvelope>(&schema_dir, "protocol_error")?;

    fs::write(generated_dir.join("types.ts"), generated_types_ts())?;
    fs::write(generated_dir.join("client.ts"), generated_client_ts())?;
    fs::write(
        generated_dir.join("routeManifest.ts"),
        generated_route_manifest_ts(),
    )?;

    println!(
        "generated protocol artifacts into {} and {}",
        display_path(&schema_dir),
        display_path(&generated_dir)
    );
    Ok(())
}

fn write_schema<T: JsonSchema>(
    schema_dir: &Path,
    name: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let schema = schema_for!(T);
    let path = schema_dir.join(format!("{name}.json"));
    fs::write(path, serde_json::to_vec_pretty(&schema)?)?;
    Ok(())
}

fn display_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn generated_types_ts() -> &'static str {
    r#"// Generated by crates/protocol_codegen. Do not edit by hand.

export const PROTOCOL_VERSION = "supernova.local_runtime.v1";

export interface ProtocolResponse<T> {
  protocol_version: string;
  schema_version: string;
  request_id: string;
  workspace_id: string;
  resource: string;
  data: T;
}

export interface Page<T> {
  items: T[];
  count: number;
  cursor?: Cursor | null;
}

export interface Cursor {
  kind: string;
  after?: string | null;
  after_event_id?: number | null;
}

export interface ProtocolEvent<T> {
  protocol_version: string;
  schema_version: string;
  event_id: string;
  event_type: string;
  cursor: Cursor;
  workspace_id: string;
  container_id?: string | null;
  chat_thread_id?: string | null;
  task_id?: string | null;
  job_id?: string | null;
  payload: T;
}

export interface StreamOpenRequest {
  request_id?: string | null;
  after_event_id?: number | null;
  limit?: number | null;
  lane?: "chat" | "task" | "runtime" | null;
}

export interface ProtocolErrorEnvelope {
  protocol_version: string;
  schema_version: string;
  request_id: string;
  workspace_id: string;
  error: {
    code: string;
    message: string;
    status: number;
    scope: string;
    retryable: boolean;
    detail: unknown;
  };
}

export interface RuntimeMeta {
  workspace_root: string;
  workspace_id: string;
  runtime_layer: "rust_product_runtime" | string;
  kernel_layer: "rust_process_kernel" | string;
  transport: "loopback_http_sse" | string;
  python_main_path: boolean;
  supports: {
    workspace_switch: boolean;
    sse: boolean;
    containers: boolean;
    chat_truth: boolean;
    process_truth: boolean;
    appdata_state: boolean;
  };
  capability_manifest_ref?: string | null;
}

export interface WorkspaceRecord {
  workspace_uid: string;
  workspace_root: string;
  display_name: string;
  created_at_ms: number;
  last_opened_at_ms: number;
  archived: boolean;
}

export interface CreateWorkspaceRequest {
  workspace_root: string;
  display_name?: string | null;
}

export interface WorkspaceActivation {
  workspace: WorkspaceRecord;
  recent_active_container_id?: string | null;
}

export interface ContainerBadges {
  running: number;
  approval: number;
  blocked: number;
  unread: number;
  artifact_ready: number;
}

export interface ContainerRecord {
  container_id: string;
  workspace_uid: string;
  title: string;
  status: "active" | "running" | "approval" | "blocked" | "archived" | "deleted";
  badges: ContainerBadges;
  created_at_ms: number;
  updated_at_ms: number;
  last_active_at_ms: number;
  default_model_config?: unknown;
  context_policy?: unknown;
}

export interface CreateContainerRequest {
  workspace_uid?: string | null;
  title?: string | null;
  model_config?: unknown;
  context_policy?: unknown;
}

export interface UpdateContainerRequest {
  title?: string | null;
  status?: ContainerRecord["status"] | null;
  model_config?: unknown;
  context_policy?: unknown;
}

export interface ContainerSnapshot {
  container: ContainerRecord;
  messages: ContainerMessage[];
  chat_threads: ChatThreadRecord[];
  tasks: TaskRecord[];
  context_pack?: ContextPack | null;
}

export interface ContainerMessage {
  message_id: string;
  workspace_uid: string;
  container_id: string;
  lane: "chat" | "task" | "runtime";
  role: "user" | "assistant" | "agent" | "tool" | "system";
  message_type: "text" | "reasoning" | "tool_call" | "tool_result" | "approval" | "artifact" | "phase" | "error";
  status: string;
  title?: string | null;
  body_text?: string | null;
  body_json: unknown;
  card_json: unknown;
  chat_thread_id?: string | null;
  task_id?: string | null;
  job_id?: string | null;
  source_kind: string;
  source_ref: string;
  source_seq?: number | null;
  created_at_ms: number;
  updated_at_ms: number;
  sort_key: string;
}

export interface ChatThreadRecord {
  chat_thread_id: string;
  container_id: string;
  title: string;
  created_at_ms: number;
  updated_at_ms: number;
}

export interface CreateChatThreadRequest {
  title?: string | null;
}

export interface ChatTurnStreamRequest {
  message: string;
  session_id?: string | null;
  context_pack_id?: string | null;
  context_pack?: ContextPack | null;
  source_guidance?: SourceGuidance | null;
  model_config?: ModelConfig | null;
}

export interface RunRecord {
  run_id: string;
  workspace_uid: string;
  container_id: string;
  run_kind: string;
  chat_thread_id?: string | null;
  task_id?: string | null;
  job_id?: string | null;
  worker_id: string;
  status: string;
  cancel_requested: boolean;
  heartbeat_at_ms?: number | null;
  started_at_ms: number;
  updated_at_ms: number;
  error_message?: string | null;
}

export interface TaskRecord {
  task_id: string;
  container_id: string;
  job_id?: string | null;
  title: string;
  goal: string;
  status: string;
  badges: ContainerBadges;
  created_at_ms: number;
  updated_at_ms: number;
}

export interface TaskStreamRequest {
  goal: string;
  session_id?: string | null;
  context_pack_id?: string | null;
  source_guidance?: SourceGuidance | null;
  model_config?: ModelConfig | null;
  artifact_destination?: ArtifactDestinationGuidance | null;
  artifact_target?: ArtifactTargetRequest | null;
  auto_approve: boolean;
}

export interface TaskDetail {
  task: TaskRecord;
  messages: ContainerMessage[];
  artifacts: ArtifactRecord[];
  approvals: ApprovalRecord[];
  receipts: TaskReceiptRecord[];
  selected_output_dir?: string | null;
  destination_fulfilled?: boolean | null;
}

export interface TaskDraftArtifactRecord {
  draft_id: string;
  workspace_uid: string;
  container_id: string;
  task_id: string;
  approval_id: string;
  preview_ref?: string | null;
  operation?: string | null;
  status: string;
  content_format: string;
  content_text: string;
  created_at_ms: number;
  updated_at_ms: number;
}

export interface ApprovalRecord {
  approval_id: string;
  task_id: string;
  operation?: string | null;
  preview_ref?: string | null;
  status: string;
  preview: unknown;
  draft_artifact?: TaskDraftArtifactRecord | null;
  created_at_ms: number;
  resolved_at_ms?: number | null;
}

export interface TaskReceiptRecord {
  receipt_id: string;
  task_id: string;
  capability_id?: string | null;
  status: string;
  kind: string;
  receipt_ref?: string | null;
  artifact_paths: string[];
  summary?: string | null;
  created_at_ms: number;
}

export interface TaskUserInputRequest {
  input: string;
}

export interface ForceCloseRequest {
  reason?: string | null;
}

export interface ForceCloseResult {
  action: string;
  status: string;
  messages: ContainerMessage[];
}

export interface TaskApprovalActionResult {
  action: string;
  task: TaskRecord;
  messages: ContainerMessage[];
  status: string;
}

export interface ContextPackItem {
  item_kind: string;
  ref_id: string;
  label?: string | null;
  include_mode: string;
  priority: number;
}

export interface SourceCandidate {
  item: ContextPackItem;
  source_kind: string;
  detail?: string | null;
  selected: boolean;
}

export interface SourceCandidateRequest {
  q?: string | null;
  limit?: number | null;
}

export interface ReferenceSourceDirective {
  source_kind: string;
  ref_id: string;
  label?: string | null;
  usage: string;
  include_mode: string;
  selection_source: string;
}

export interface SourceGuidance {
  semantics: string;
  materialized_content: boolean;
  source_scope_enforcement: string;
  selected_sources: ReferenceSourceDirective[];
  user_intent?: string | null;
}

export interface ContextPack {
  context_pack_id: string;
  container_id: string;
  selected_items: ContextPackItem[];
  excluded_items: ContextPackItem[];
  auto_policy: {
    include_recent_chat_turns: number;
    include_recent_tasks: number;
    prefer_summaries: boolean;
  };
  summary_ref?: string | null;
  estimated_tokens?: number | null;
}

export interface ContextPackEstimate {
  context_pack: ContextPack;
  estimated_tokens: number;
  context_window_tokens: number;
  usage_ratio: string;
}

export interface ModelConfig {
  provider: string;
  model: string;
  thinking: string;
  reasoning_effort: string;
  token_budget?: number | null;
  strict_tools: boolean;
}

export interface ModelConfigDescriptor {
  active: ModelConfig;
  providers: Array<{ provider: string; display_name: string; models: string[]; model_options: ModelConfigOption[]; supports_thinking: boolean; supports_strict_tools: boolean }>;
  thinking_options: ModelConfigOption[];
  reasoning_effort_options: ModelConfigOption[];
  token_budget_min: number;
  token_budget_max: number;
  token_budget_default: number;
  strict_tools_label: string;
  strict_tools_description: string;
  advanced_defaults_collapsed: boolean;
  user_summary: string;
}

export interface ModelConfigOption {
  value: string;
  label: string;
  description: string;
}

export interface ArtifactRecord {
  artifact_id: string;
  container_id: string;
  task_id?: string | null;
  title: string;
  artifact_type: string;
  path?: string | null;
  status: string;
  capability_id?: string | null;
  receipt_ref?: string | null;
  verified: boolean;
  kind?: string | null;
  created_at_ms: number;
}

export interface ArtifactTargetRequest {
  container_id: string;
  artifact_type: string;
  target_dir?: string | null;
  save_strategy: string;
}

export interface ArtifactDestinationGuidance {
  semantics: string;
  enforcement: string;
  materialized_artifact: boolean;
  selected_output_dir: string;
  label?: string | null;
}

export interface ArtifactTargetOption {
  target_id: string;
  label: string;
  target_dir: string;
  artifact_types: string[];
  save_strategies: string[];
  user_visible: boolean;
}

export interface AppSettings {
  provider_api: ProviderApiSettings;
  data_paths: {
    app_config_root: string;
    app_state_root: string;
    workspace_registry_path: string;
  };
  appearance: AppearanceSettings;
}

export interface AppearanceSettings {
  language: "zh-CN" | "en-US";
  theme: "light" | "dark";
}

export interface ProviderApiSettings {
  providers: ProviderApiRecord[];
}

export interface ProviderApiRecord {
  provider: string;
  api_base_url?: string | null;
  api_key_configured: boolean;
  credential_ref?: string | null;
  validation_status?: string | null;
  token_usage_summary?: string | null;
}

export interface ProviderApiUpdateRequest {
  provider: string;
  api_base_url?: string | null;
  api_key?: string | null;
}

export interface ProviderApiTestRequest {
  provider: string;
  live_check?: boolean | null;
}

export interface ProviderApiTestResult {
  provider: string;
  status: string;
  message: string;
  api_base_url?: string | null;
  api_key_configured: boolean;
  credential_ref?: string | null;
  live_check_performed: boolean;
  checked_by: string;
}

export interface DiagnosticsSnapshot {
  runtime_status: string;
  protocol_version: string;
  runtime_layer: string;
  kernel_layer: string;
  app_config_root: string;
  app_state_root: string;
  workspace_id: string;
  last_error?: string | null;
}

export interface RuntimeHealth {
  status: string;
  runtime_layer: string;
  workspace_id: string;
  uptime_ms: number;
}

export interface RuntimeEventPayload {
  summary?: string | null;
  message?: ContainerMessage | null;
  record?: unknown;
}

export interface UiCapabilityManifest {
  commands: Array<{ command_id: string; label: string; description: string; capability_id: string }>;
  workspace_actions: Array<{ action_id: string; label: string; capability_id: string; side_effect: string }>;
  container_actions: Array<{ action_id: string; label: string; capability_id: string; side_effect: string }>;
  composer_tokens: Array<{ token: string; label: string; capability_id: string }>;
  model_config: ModelConfigDescriptor;
  context_config: unknown;
  settings: Array<{ action_id: string; label: string; capability_id: string; side_effect: string }>;
}
"#
}

fn generated_client_ts() -> &'static str {
    r#"// Generated by crates/protocol_codegen. Do not edit by hand.
import type {
  AppSettings,
  ArtifactTargetOption,
  ChatThreadRecord,
  ChatTurnStreamRequest,
  ContainerMessage,
  ContainerRecord,
  ContainerSnapshot,
  ContextPack,
  ContextPackEstimate,
  CreateChatThreadRequest,
  CreateContainerRequest,
  CreateWorkspaceRequest,
  DiagnosticsSnapshot,
  ForceCloseRequest,
  ForceCloseResult,
  ModelConfig,
  ModelConfigDescriptor,
  Page,
  ProtocolErrorEnvelope,
  ProtocolEvent,
  ProviderApiSettings,
  ProviderApiTestRequest,
  ProviderApiTestResult,
  ProviderApiUpdateRequest,
  ProtocolResponse,
  RuntimeHealth,
  RuntimeEventPayload,
  RuntimeMeta,
  RunRecord,
  SourceCandidate,
  SourceCandidateRequest,
  StreamOpenRequest,
  TaskApprovalActionResult,
  TaskDetail,
  TaskRecord,
  TaskStreamRequest,
  TaskUserInputRequest,
  UiCapabilityManifest,
  UpdateContainerRequest,
  WorkspaceActivation,
  WorkspaceRecord
} from "./types";

export interface ProtocolClientOptions {
  baseUrl: string;
  runtimeToken?: string;
  fetchImpl?: typeof fetch;
}

export type StreamEventHandler<T> = (event: ProtocolEvent<T>) => void | Promise<void>;

export class LocalRuntimeProtocolClient {
  private baseUrl: string;
  private runtimeToken?: string;
  private fetchImpl: typeof fetch;

  constructor(options: ProtocolClientOptions) {
    this.baseUrl = options.baseUrl.replace(/\/$/, "");
    this.runtimeToken = options.runtimeToken;
    this.fetchImpl = options.fetchImpl || globalThis.fetch.bind(globalThis);
  }

  runtimeMeta() {
    return this.get<RuntimeMeta>("/api/v1/runtime/meta");
  }

  runtimeHealth() {
    return this.get<RuntimeHealth>("/api/v1/runtime/health");
  }

  runtimeCapabilities() {
    return this.get<UiCapabilityManifest>("/api/v1/runtime/capabilities");
  }

  runtimeEvents(request: StreamOpenRequest = {}, onEvent?: StreamEventHandler<RuntimeEventPayload>) {
    return this.streamGet<RuntimeEventPayload>(`/api/v1/runtime/events${queryString(request)}`, onEvent);
  }

  workspaces() {
    return this.get<Page<WorkspaceRecord>>("/api/v1/workspaces");
  }

  createWorkspace(request: CreateWorkspaceRequest) {
    return this.post<WorkspaceRecord>("/api/v1/workspaces", request);
  }

  activateWorkspace(request: { workspace_uid?: string | null; workspace_root?: string | null }) {
    return this.post<WorkspaceActivation>("/api/v1/workspaces/activate", request);
  }

  archiveWorkspace(workspaceUid: string) {
    return this.post<WorkspaceRecord>(`/api/v1/workspaces/${encodeURIComponent(workspaceUid)}/archive`, {});
  }

  workspaceContainers(workspaceUid: string) {
    return this.get<Page<ContainerRecord>>(`/api/v1/workspaces/${encodeURIComponent(workspaceUid)}/containers`);
  }

  containers() {
    return this.get<Page<ContainerRecord>>("/api/v1/containers");
  }

  createContainer(request: CreateContainerRequest = {}) {
    return this.post<ContainerRecord>("/api/v1/containers", request);
  }

  archivedContainers() {
    return this.get<Page<ContainerRecord>>("/api/v1/containers/archived");
  }

  container(containerId: string) {
    return this.get<ContainerRecord>(`/api/v1/containers/${encodeURIComponent(containerId)}`);
  }

  updateContainer(containerId: string, request: UpdateContainerRequest) {
    return this.patch<ContainerRecord>(`/api/v1/containers/${encodeURIComponent(containerId)}`, request);
  }

  archiveContainer(containerId: string) {
    return this.post<ContainerRecord>(`/api/v1/containers/${encodeURIComponent(containerId)}/archive`, {});
  }

  activateContainer(containerId: string) {
    return this.post<ContainerRecord>(`/api/v1/containers/${encodeURIComponent(containerId)}/activate`, {});
  }

  restoreContainer(containerId: string) {
    return this.post<ContainerRecord>(`/api/v1/containers/${encodeURIComponent(containerId)}/restore`, {});
  }

  deleteContainer(containerId: string) {
    return this.delete<ContainerRecord>(`/api/v1/containers/${encodeURIComponent(containerId)}`);
  }

  containerSnapshot(containerId: string) {
    return this.get<ContainerSnapshot>(`/api/v1/containers/${encodeURIComponent(containerId)}/snapshot`);
  }

  containerMessages(containerId: string, request: StreamOpenRequest = {}) {
    return this.get<Page<ContainerMessage>>(`/api/v1/containers/${encodeURIComponent(containerId)}/messages${queryString(request)}`);
  }

  chatThreads(containerId: string) {
    return this.get<Page<ChatThreadRecord>>(`/api/v1/containers/${encodeURIComponent(containerId)}/chat/threads`);
  }

  createChatThread(containerId: string, request: CreateChatThreadRequest = {}) {
    return this.post<ChatThreadRecord>(`/api/v1/containers/${encodeURIComponent(containerId)}/chat/threads`, request);
  }

  chatMessages(chatThreadId: string, request: StreamOpenRequest = {}) {
    return this.get<Page<ContainerMessage>>(`/api/v1/chat/threads/${encodeURIComponent(chatThreadId)}/messages${queryString(request)}`);
  }

  chatTurnStream(chatThreadId: string, request: ChatTurnStreamRequest, onEvent?: StreamEventHandler<unknown>) {
    return this.stream(`/api/v1/chat/threads/${encodeURIComponent(chatThreadId)}/turns/stream`, request, onEvent);
  }

  forceCloseChatTurn(chatThreadId: string, request: ForceCloseRequest = {}) {
    return this.post<ForceCloseResult>(`/api/v1/chat/threads/${encodeURIComponent(chatThreadId)}/force-close`, request);
  }

  runs(request: { container_id?: string | null } = {}) {
    return this.get<Page<RunRecord>>(`/api/v1/runs${queryString(request)}`);
  }

  tasks(containerId: string) {
    return this.get<Page<TaskRecord>>(`/api/v1/containers/${encodeURIComponent(containerId)}/tasks`);
  }

  startTaskStream(containerId: string, request: TaskStreamRequest, onEvent?: StreamEventHandler<unknown>) {
    return this.stream(`/api/v1/containers/${encodeURIComponent(containerId)}/tasks/stream`, request, onEvent);
  }

  task(taskId: string) {
    return this.get<TaskDetail>(`/api/v1/tasks/${encodeURIComponent(taskId)}`);
  }

  taskMessages(taskId: string, request: StreamOpenRequest = {}) {
    return this.get<Page<ContainerMessage>>(`/api/v1/tasks/${encodeURIComponent(taskId)}/messages${queryString(request)}`);
  }

  taskEventsStream(taskId: string, request: StreamOpenRequest = {}, onEvent?: StreamEventHandler<unknown>) {
    return this.streamGet<unknown>(`/api/v1/tasks/${encodeURIComponent(taskId)}/events/stream${queryString(request)}`, onEvent);
  }

  submitTaskUserInput(taskId: string, request: TaskUserInputRequest) {
    return this.post<TaskApprovalActionResult>(`/api/v1/tasks/${encodeURIComponent(taskId)}/input`, request);
  }

  forceCloseTask(taskId: string, request: ForceCloseRequest = {}) {
    return this.post<ForceCloseResult>(`/api/v1/tasks/${encodeURIComponent(taskId)}/force-close`, request);
  }

  artifactTargets(containerId: string) {
    return this.get<Page<ArtifactTargetOption>>(`/api/v1/containers/${encodeURIComponent(containerId)}/artifact-targets`);
  }

  sourceCandidates(containerId: string, request: SourceCandidateRequest = {}) {
    return this.get<Page<SourceCandidate>>(`/api/v1/containers/${encodeURIComponent(containerId)}/source-candidates${queryString(request)}`);
  }

  contextPack(containerId: string) {
    return this.get<ContextPack>(`/api/v1/containers/${encodeURIComponent(containerId)}/context-pack`);
  }

  saveContextPack(containerId: string, request: ContextPack) {
    return this.post<ContextPack>(`/api/v1/containers/${encodeURIComponent(containerId)}/context-pack`, request);
  }

  estimateContextPack(containerId: string, request: ContextPack) {
    return this.post<ContextPackEstimate>(`/api/v1/containers/${encodeURIComponent(containerId)}/context-pack/estimate`, request);
  }

  modelConfig() {
    return this.get<ModelConfigDescriptor>("/api/v1/model-config");
  }

  updateModelConfig(request: ModelConfig) {
    return this.patch<ModelConfigDescriptor>("/api/v1/model-config", request);
  }

  settings() {
    return this.get<AppSettings>("/api/v1/settings");
  }

  updateSettings(request: AppSettings) {
    return this.patch<AppSettings>("/api/v1/settings", request);
  }

  providerSettings() {
    return this.get<ProviderApiSettings>("/api/v1/settings/provider");
  }

  updateProviderSettings(request: ProviderApiUpdateRequest) {
    return this.patch<ProviderApiSettings>("/api/v1/settings/provider", request);
  }

  testProviderSettings(request: ProviderApiTestRequest) {
    return this.post<ProviderApiTestResult>("/api/v1/settings/provider/test", request);
  }

  diagnostics() {
    return this.get<DiagnosticsSnapshot>("/api/v1/diagnostics");
  }

  private async get<T>(path: string): Promise<T> {
    const response = await this.fetchImpl(`${this.baseUrl}${path}`, { headers: this.headers() });
    if (!response.ok) {
      throw await this.readError(response);
    }
    const envelope = (await response.json()) as ProtocolResponse<T>;
    return envelope.data;
  }

  private async post<T>(path: string, body: unknown): Promise<T> {
    return this.send<T>("POST", path, body);
  }

  private async patch<T>(path: string, body: unknown): Promise<T> {
    return this.send<T>("PATCH", path, body);
  }

  private async delete<T>(path: string): Promise<T> {
    const response = await this.fetchImpl(`${this.baseUrl}${path}`, {
      method: "DELETE",
      headers: this.headers()
    });
    if (!response.ok) {
      throw await this.readError(response);
    }
    const envelope = (await response.json()) as ProtocolResponse<T>;
    return envelope.data;
  }

  private async send<T>(method: "POST" | "PATCH", path: string, body: unknown): Promise<T> {
    const response = await this.fetchImpl(`${this.baseUrl}${path}`, {
      method,
      headers: this.headers({ "Content-Type": "application/json" }),
      body: JSON.stringify(body)
    });
    if (!response.ok) {
      throw await this.readError(response);
    }
    const envelope = (await response.json()) as ProtocolResponse<T>;
    return envelope.data;
  }

  private async streamGet<T = unknown>(
    path: string,
    onEvent?: StreamEventHandler<T>
  ): Promise<Array<ProtocolEvent<T>>> {
    const response = await this.fetchImpl(`${this.baseUrl}${path}`, { headers: this.headers() });
    if (!response.ok) {
      throw await this.readError(response);
    }
    return readSseEvents<T>(response, onEvent);
  }

  private async stream<T = unknown>(
    path: string,
    body: unknown,
    onEvent?: StreamEventHandler<T>
  ): Promise<Array<ProtocolEvent<T>>> {
    const response = await this.fetchImpl(`${this.baseUrl}${path}`, {
      method: "POST",
      headers: this.headers({ "Content-Type": "application/json" }),
      body: JSON.stringify(body)
    });
    if (!response.ok) {
      throw await this.readError(response);
    }
    return readSseEvents<T>(response, onEvent);
  }

  private async readError(response: Response): Promise<Error> {
    try {
      const envelope = (await response.json()) as ProtocolErrorEnvelope;
      return new Error(`${envelope.error.code}: ${envelope.error.message}`);
    } catch {
      return new Error(`Protocol request failed with HTTP ${response.status}`);
    }
  }

  private headers(base: Record<string, string> = {}): Record<string, string> {
    if (!this.runtimeToken) {
      return base;
    }
    return {
      ...base,
      "X-SuperNova-Runtime-Token": this.runtimeToken
    };
  }
}

async function readSseEvents<T>(
  response: Response,
  onEvent?: StreamEventHandler<T>
): Promise<Array<ProtocolEvent<T>>> {
  const events: Array<ProtocolEvent<T>> = [];
  if (!response.body) {
    await emitSseParts(await response.text(), events, onEvent);
    return events;
  }

  const reader = response.body.getReader();
  const decoder = new TextDecoder();
  let buffer = "";
  while (true) {
    const { done, value } = await reader.read();
    if (done) break;
    buffer += decoder.decode(value, { stream: true });
    buffer = await emitCompleteSseChunks(buffer, events, onEvent);
  }
  buffer += decoder.decode();
  await emitSseParts(buffer, events, onEvent);
  return events;
}

async function emitCompleteSseChunks<T>(
  buffer: string,
  events: Array<ProtocolEvent<T>>,
  onEvent?: StreamEventHandler<T>
): Promise<string> {
  const normalized = buffer.replace(/\r\n/g, "\n");
  const parts = normalized.split(/\n\s*\n/g);
  const remainder = parts.pop() || "";
  await emitSseParts(parts.join("\n\n"), events, onEvent);
  return remainder;
}

async function emitSseParts<T>(
  text: string,
  events: Array<ProtocolEvent<T>>,
  onEvent?: StreamEventHandler<T>
) {
  const chunks = text.split(/\n\s*\n/g).map((part) => part.trim()).filter(Boolean);
  for (const chunk of chunks) {
    const event = parseSseEvent<T>(chunk);
    if (!event) continue;
    events.push(event);
    await onEvent?.(event);
  }
}

function parseSseEvent<T>(chunk: string): ProtocolEvent<T> | null {
  const data = chunk
    .split(/\n/)
    .filter((line) => line.startsWith("data:"))
    .map((line) => line.slice("data:".length).trim())
    .join("\n");
  if (!data) return null;
  try {
    return JSON.parse(data) as ProtocolEvent<T>;
  } catch {
    return null;
  }
}

function queryString(request: (StreamOpenRequest & SourceCandidateRequest & { container_id?: string | null }) = {}): string {
  const params = new URLSearchParams();
  if (request.request_id) params.set("request_id", request.request_id);
  if (request.q) params.set("q", request.q);
  if (request.container_id) params.set("container_id", request.container_id);
  if (request.after_event_id !== undefined && request.after_event_id !== null) {
    params.set("after_event_id", String(request.after_event_id));
  }
  if (request.limit !== undefined && request.limit !== null) {
    params.set("limit", String(request.limit));
  }
  if (request.lane) params.set("lane", request.lane);
  const raw = params.toString();
  return raw ? `?${raw}` : "";
}
"#
}

fn generated_route_manifest_ts() -> &'static str {
    r#"// Generated by crates/protocol_codegen. Do not edit by hand.

export const protocolRouteManifest = {
  runtimeMeta: "GET /api/v1/runtime/meta",
  runtimeHealth: "GET /api/v1/runtime/health",
  runtimeCapabilities: "GET /api/v1/runtime/capabilities",
  runtimeEvents: "GET /api/v1/runtime/events",
  workspaces: "GET /api/v1/workspaces",
  createWorkspace: "POST /api/v1/workspaces",
  activateWorkspace: "POST /api/v1/workspaces/activate",
  archiveWorkspace: "POST /api/v1/workspaces/{workspace_uid}/archive",
  workspaceContainers: "GET /api/v1/workspaces/{workspace_uid}/containers",
  containers: "GET /api/v1/containers",
  createContainer: "POST /api/v1/containers",
  archivedContainers: "GET /api/v1/containers/archived",
  container: "GET /api/v1/containers/{container_id}",
  updateContainer: "PATCH /api/v1/containers/{container_id}",
  archiveContainer: "POST /api/v1/containers/{container_id}/archive",
  activateContainer: "POST /api/v1/containers/{container_id}/activate",
  restoreContainer: "POST /api/v1/containers/{container_id}/restore",
  deleteContainer: "DELETE /api/v1/containers/{container_id}",
  containerSnapshot: "GET /api/v1/containers/{container_id}/snapshot",
  containerMessages: "GET /api/v1/containers/{container_id}/messages",
  chatThreads: "GET /api/v1/containers/{container_id}/chat/threads",
  createChatThread: "POST /api/v1/containers/{container_id}/chat/threads",
  chatMessages: "GET /api/v1/chat/threads/{chat_thread_id}/messages",
  chatTurnStream: "POST /api/v1/chat/threads/{chat_thread_id}/turns/stream",
  forceCloseChatTurn: "POST /api/v1/chat/threads/{chat_thread_id}/force-close",
  runs: "GET /api/v1/runs",
  tasks: "GET /api/v1/containers/{container_id}/tasks",
  startTaskStream: "POST /api/v1/containers/{container_id}/tasks/stream",
  task: "GET /api/v1/tasks/{task_id}",
  taskMessages: "GET /api/v1/tasks/{task_id}/messages",
  taskEventsStream: "GET /api/v1/tasks/{task_id}/events/stream",
  submitTaskUserInput: "POST /api/v1/tasks/{task_id}/input",
  forceCloseTask: "POST /api/v1/tasks/{task_id}/force-close",
  artifactTargets: "GET /api/v1/containers/{container_id}/artifact-targets",
  sourceCandidates: "GET /api/v1/containers/{container_id}/source-candidates",
  contextPack: "GET /api/v1/containers/{container_id}/context-pack",
  saveContextPack: "POST /api/v1/containers/{container_id}/context-pack",
  estimateContextPack: "POST /api/v1/containers/{container_id}/context-pack/estimate",
  modelConfig: "GET /api/v1/model-config",
  updateModelConfig: "PATCH /api/v1/model-config",
  settings: "GET /api/v1/settings",
  updateSettings: "PATCH /api/v1/settings",
  providerSettings: "GET /api/v1/settings/provider",
  updateProviderSettings: "PATCH /api/v1/settings/provider",
  testProviderSettings: "POST /api/v1/settings/provider/test",
  diagnostics: "GET /api/v1/diagnostics"
} as const;
"#
}
