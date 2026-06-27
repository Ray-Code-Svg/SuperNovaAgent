use std::collections::{BTreeMap, BTreeSet};
use std::io;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{now_ms, ProcessEvent, ProcessTruthStore};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskContextState {
    pub job_id: String,
    pub root_pid: String,
    pub task_agent_session_id: String,
    pub current_turn_index: usize,
    pub next_turn_index: usize,
    pub goal_ref: Option<String>,
    pub system_prompt_ref: Option<String>,
    pub conversation_refs: Vec<String>,
    pub transcript_refs: Vec<String>,
    pub provider_transcript_refs: Vec<String>,
    pub provider_transcript_ref: Option<String>,
    pub provider_transcript_summary_ref: Option<String>,
    pub working_memory_ref: Option<String>,
    pub latest_observation_ref: Option<String>,
    pub pending_user_decision: Option<PendingUserDecision>,
    pub preview_tx_table: Vec<PreviewTxState>,
    pub approval_token_table: Vec<ApprovalTokenState>,
    pub capability_receipt_cursor: u64,
    pub artifact_table: Vec<ArtifactContextEntry>,
    pub verification_table: Vec<VerificationContextEntry>,
    pub closure_state: ClosureContextState,
    pub last_error_summary: Option<String>,
    pub last_model_protocol_error: Option<Value>,
    pub status: String,
    pub waiting_for: Option<String>,
    pub started: bool,
    pub start_event_count: usize,
    pub last_turn_id: Option<String>,
    pub last_decision_id: Option<String>,
    pub updated_at_ms: u128,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingUserDecision {
    pub decision_type: String,
    pub preview_id: Option<String>,
    pub user_input_ref: Option<String>,
    pub status: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PreviewTxState {
    pub tx_id: String,
    pub preview_id: String,
    pub status: String,
    pub preview_ref: Option<String>,
    pub executable_operations: Vec<PreviewOperationState>,
    pub proposed_actions: Vec<String>,
    pub target_paths: Vec<String>,
    pub approval_token_id: Option<String>,
    pub applied_capability_id: Option<String>,
    pub applied_event_id: Option<u64>,
    pub closed_event_id: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PreviewOperationState {
    pub capability_id: String,
    pub target_paths: Vec<String>,
    pub human_description: Option<String>,
    pub rollback_policy: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ApprovalTokenState {
    pub approval_token_id: String,
    pub tx_id: String,
    pub preview_id: String,
    pub status: String,
    pub approved_operation_scope: Vec<String>,
    pub approved_action_scope: Vec<String>,
    pub approved_path_scope: Vec<String>,
    pub issued_event_id: u64,
    pub consumed_event_id: Option<u64>,
    pub used_capability_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactContextEntry {
    pub path: String,
    pub last_event_id: u64,
    pub producing_capability_id: Option<String>,
    pub verified: bool,
    pub audited: bool,
    pub local_audit_completed: bool,
    pub local_audit_hard_risk_pass: Option<bool>,
    pub model_audited: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct VerificationContextEntry {
    pub artifact_path: Option<String>,
    pub capability_id: String,
    pub status: String,
    pub event_id: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClosureContextState {
    pub status: String,
    pub last_attempt_event_id: Option<u64>,
    pub last_blocked_event_id: Option<u64>,
    pub last_completed_event_id: Option<u64>,
    pub open_gaps: Vec<String>,
}

impl TaskContextState {
    pub fn initial(job_id: &str, root_pid: &str, session_id: &str) -> Self {
        Self {
            job_id: job_id.to_string(),
            root_pid: root_pid.to_string(),
            task_agent_session_id: session_id.to_string(),
            current_turn_index: 0,
            next_turn_index: 1,
            goal_ref: None,
            system_prompt_ref: None,
            conversation_refs: Vec::new(),
            transcript_refs: Vec::new(),
            provider_transcript_refs: Vec::new(),
            provider_transcript_ref: None,
            provider_transcript_summary_ref: None,
            working_memory_ref: None,
            latest_observation_ref: None,
            pending_user_decision: None,
            preview_tx_table: Vec::new(),
            approval_token_table: Vec::new(),
            capability_receipt_cursor: 0,
            artifact_table: Vec::new(),
            verification_table: Vec::new(),
            closure_state: ClosureContextState {
                status: "open".to_string(),
                last_attempt_event_id: None,
                last_blocked_event_id: None,
                last_completed_event_id: None,
                open_gaps: Vec::new(),
            },
            last_error_summary: None,
            last_model_protocol_error: None,
            status: "created".to_string(),
            waiting_for: None,
            started: false,
            start_event_count: 0,
            last_turn_id: None,
            last_decision_id: None,
            updated_at_ms: now_ms(),
        }
    }

    pub fn to_value(&self) -> Value {
        json!(self)
    }
}

pub fn replay_task_context_state(
    truth: &ProcessTruthStore,
    root_pid: &str,
    session_id: &str,
) -> io::Result<TaskContextState> {
    let events = truth.read_events()?;
    Ok(replay_task_context_state_from_events(
        truth.job_id(),
        root_pid,
        session_id,
        &events,
    ))
}

pub fn replay_task_context_state_from_events(
    job_id: &str,
    root_pid: &str,
    session_id: &str,
    events: &[ProcessEvent],
) -> TaskContextState {
    let mut state = TaskContextState::initial(job_id, root_pid, session_id);
    let mut preview_txs: BTreeMap<String, PreviewTxState> = BTreeMap::new();
    let mut tokens: BTreeMap<String, ApprovalTokenState> = BTreeMap::new();
    let mut artifacts: BTreeMap<String, ArtifactContextEntry> = BTreeMap::new();
    let mut verifications: Vec<VerificationContextEntry> = Vec::new();
    let mut transcript_refs = BTreeSet::new();
    let mut provider_transcript_refs = BTreeSet::new();

    for event in events {
        state.capability_receipt_cursor = state.capability_receipt_cursor.max(event.event_id);
        match event.event_type.as_str() {
            "task_agent_session_started" if event_runtime_id(event) == Some(session_id) => {
                state.started = true;
                state.start_event_count = state.start_event_count.saturating_add(1);
                state.status = "running".to_string();
                state.waiting_for = None;
                state.goal_ref = event
                    .data
                    .get("goal_ref")
                    .and_then(Value::as_str)
                    .map(ToString::to_string)
                    .or_else(|| state.goal_ref.clone());
                state.system_prompt_ref = event
                    .data
                    .get("system_prompt_ref")
                    .and_then(Value::as_str)
                    .map(ToString::to_string)
                    .or_else(|| state.system_prompt_ref.clone());
            }
            "task_context_state_updated" if event_session_id(event) == Some(session_id) => {
                apply_context_update(&mut state, &event.data);
            }
            "task_agent_observation_built" if event_runtime_id(event) == Some(session_id) => {
                state.latest_observation_ref = event
                    .data
                    .get("observation_frame_ref")
                    .and_then(Value::as_str)
                    .map(ToString::to_string)
                    .or_else(|| state.latest_observation_ref.clone());
                if let Some(goal_ref) = event.data.get("goal_ref").and_then(Value::as_str) {
                    state.goal_ref = Some(goal_ref.to_string());
                }
            }
            "task_agent_turn_started" if event_runtime_id(event) == Some(session_id) => {
                let turn_index = turn_index_from_event(event).unwrap_or(state.next_turn_index);
                state.current_turn_index = state.current_turn_index.max(turn_index);
                state.next_turn_index = state.next_turn_index.max(turn_index.saturating_add(1));
                state.last_turn_id = event
                    .data
                    .get("turn_id")
                    .and_then(Value::as_str)
                    .map(ToString::to_string)
                    .or_else(|| state.last_turn_id.clone());
                state.status = "running".to_string();
                state.waiting_for = None;
            }
            "task_agent_turn_completed" if event_runtime_id(event) == Some(session_id) => {
                let turn_index = turn_index_from_event(event).unwrap_or(state.current_turn_index);
                state.current_turn_index = state.current_turn_index.max(turn_index);
                state.next_turn_index = state.next_turn_index.max(turn_index.saturating_add(1));
                state.last_turn_id = event
                    .data
                    .get("turn_id")
                    .and_then(Value::as_str)
                    .map(ToString::to_string)
                    .or_else(|| state.last_turn_id.clone());
                state.last_decision_id = event
                    .data
                    .get("decision_id")
                    .and_then(Value::as_str)
                    .map(ToString::to_string)
                    .or_else(|| state.last_decision_id.clone());
                if let Some(status) = event.data.get("status").and_then(Value::as_str) {
                    state.status = status.to_string();
                    state.waiting_for = waiting_for_status(status).map(ToString::to_string);
                    state.pending_user_decision = match status {
                        "waiting_approval" => Some(PendingUserDecision {
                            decision_type: "approval".to_string(),
                            preview_id: latest_open_preview_id(&preview_txs),
                            user_input_ref: None,
                            status: "pending".to_string(),
                        }),
                        "waiting_user" => Some(PendingUserDecision {
                            decision_type: "user_input".to_string(),
                            preview_id: None,
                            user_input_ref: None,
                            status: "pending".to_string(),
                        }),
                        _ => state.pending_user_decision.clone(),
                    };
                }
            }
            "preview_tx_created" => {
                let tx_id = string_field(&event.data, "tx_id");
                let preview_id = string_field(&event.data, "preview_id");
                if !tx_id.is_empty() && !preview_id.is_empty() {
                    preview_txs.insert(
                        tx_id.clone(),
                        PreviewTxState {
                            tx_id,
                            preview_id,
                            status: "created".to_string(),
                            preview_ref: event
                                .data
                                .get("preview_ref")
                                .and_then(Value::as_str)
                                .map(ToString::to_string),
                            executable_operations: preview_operations_state(&event.data),
                            proposed_actions: strings_field(&event.data, "proposed_actions"),
                            target_paths: strings_field(&event.data, "target_paths"),
                            approval_token_id: None,
                            applied_capability_id: None,
                            applied_event_id: None,
                            closed_event_id: None,
                        },
                    );
                }
            }
            "preview_tx_approved" => {
                if let Some(tx) = event_tx_id(event).and_then(|tx_id| preview_txs.get_mut(tx_id)) {
                    tx.status = "approved".to_string();
                    tx.approval_token_id = event
                        .data
                        .get("approval_token_id")
                        .and_then(Value::as_str)
                        .map(ToString::to_string);
                }
                if let Some(pending) = state.pending_user_decision.as_mut() {
                    if pending.decision_type == "approval" {
                        pending.status = "resolved".to_string();
                    }
                }
            }
            "approval_token_issued" => {
                let token_id = string_field(&event.data, "approval_token_id");
                let tx_id = string_field(&event.data, "tx_id");
                let preview_id = string_field(&event.data, "preview_id");
                if !token_id.is_empty() {
                    tokens.insert(
                        token_id.clone(),
                        ApprovalTokenState {
                            approval_token_id: token_id.clone(),
                            tx_id: tx_id.clone(),
                            preview_id: preview_id.clone(),
                            status: "active".to_string(),
                            approved_operation_scope: strings_field(
                                &event.data,
                                "approved_operation_scope",
                            ),
                            approved_action_scope: strings_field(
                                &event.data,
                                "approved_action_scope",
                            ),
                            approved_path_scope: strings_field(&event.data, "approved_path_scope"),
                            issued_event_id: event.event_id,
                            consumed_event_id: None,
                            used_capability_id: None,
                        },
                    );
                    if let Some(tx) = preview_txs.get_mut(&tx_id) {
                        tx.status = "approved".to_string();
                        tx.approval_token_id = Some(token_id);
                    }
                }
            }
            "approval_token_consumed" | "approval_token_used" => {
                let token_id = string_field(&event.data, "approval_token_id");
                let capability_id = string_field(&event.data, "capability_id");
                if let Some(token) = tokens.get_mut(&token_id) {
                    token.status = "consumed".to_string();
                    token.consumed_event_id = Some(event.event_id);
                    token.used_capability_id = if capability_id.is_empty() {
                        None
                    } else {
                        Some(capability_id.clone())
                    };
                    if let Some(tx) = preview_txs.get_mut(&token.tx_id) {
                        tx.status = "applied".to_string();
                        tx.applied_capability_id = token.used_capability_id.clone();
                        tx.applied_event_id = Some(event.event_id);
                    }
                }
            }
            "preview_tx_closed" => {
                if let Some(tx) = event_tx_id(event).and_then(|tx_id| preview_txs.get_mut(tx_id)) {
                    tx.status = event
                        .data
                        .get("status")
                        .and_then(Value::as_str)
                        .unwrap_or("closed")
                        .to_string();
                    tx.closed_event_id = Some(event.event_id);
                }
            }
            "capability_receipt" | "model_call_receipt" => {
                collect_artifacts_and_verifications(event, &mut artifacts, &mut verifications);
                if let Some(capability_id) = event_capability_id(event) {
                    if capability_id == "process.complete" {
                        state.closure_state.last_attempt_event_id = Some(event.event_id);
                    }
                }
            }
            "artifact_model_audit_receipt" => {
                collect_artifacts_and_verifications(event, &mut artifacts, &mut verifications);
            }
            "provider_transcript_appended" | "provider_transcript_compacted" => {
                if let Some(messages_ref) = event.data.get("messages_ref").and_then(Value::as_str) {
                    state.provider_transcript_ref = Some(messages_ref.to_string());
                    provider_transcript_refs.insert(messages_ref.to_string());
                }
                if let Some(summary_ref) = event.data.get("summary_ref").and_then(Value::as_str) {
                    state.provider_transcript_summary_ref = Some(summary_ref.to_string());
                    provider_transcript_refs.insert(summary_ref.to_string());
                }
            }
            "closure_gate_blocked" => {
                state.closure_state.status = "blocked".to_string();
                state.closure_state.last_blocked_event_id = Some(event.event_id);
                state.closure_state.open_gaps = closure_gaps(&event.data);
            }
            "job_completed" => {
                state.status = "completed".to_string();
                state.waiting_for = None;
                state.pending_user_decision = None;
                state.closure_state.status = "completed".to_string();
                state.closure_state.last_completed_event_id = Some(event.event_id);
            }
            "job_blocked" => {
                state.status = "blocked".to_string();
                state.waiting_for = None;
                state.last_error_summary =
                    event_error_summary(event).or(state.last_error_summary.clone());
            }
            "job_failed" => {
                state.status = "failed".to_string();
                state.waiting_for = None;
                state.last_error_summary =
                    event_error_summary(event).or(state.last_error_summary.clone());
            }
            "job_interrupted_by_model_protocol_error" => {
                state.status = "interrupted".to_string();
                state.waiting_for = None;
                state.last_model_protocol_error = Some(event.data.clone());
                state.last_error_summary =
                    event_error_summary(event).or(state.last_error_summary.clone());
            }
            "job_status_changed" => {
                if let Some(status) = event.data.get("status").and_then(Value::as_str) {
                    state.status = status.to_string();
                    state.waiting_for = waiting_for_status(status).map(ToString::to_string);
                }
            }
            "user_approval_received" => {
                if let Some(pending) = state.pending_user_decision.as_mut() {
                    if pending.decision_type == "approval" {
                        pending.status = "resolved".to_string();
                    }
                }
            }
            "user_input_received" => {
                state.pending_user_decision = Some(PendingUserDecision {
                    decision_type: "user_input".to_string(),
                    preview_id: None,
                    user_input_ref: event
                        .data
                        .get("input_ref")
                        .and_then(Value::as_str)
                        .map(ToString::to_string),
                    status: "received".to_string(),
                });
            }
            "process_action_failed" => {
                state.last_error_summary = event_error_summary(event);
            }
            _ => {}
        }

        if let Some(ref_value) = event
            .data
            .get("output_ref")
            .and_then(Value::as_str)
            .or_else(|| event.data.get("state_ref").and_then(Value::as_str))
        {
            transcript_refs.insert(ref_value.to_string());
        }
    }

    state.preview_tx_table = preview_txs.into_values().collect();
    state.approval_token_table = tokens.into_values().collect();
    state.artifact_table = artifacts.into_values().collect();
    state.verification_table = verifications;
    state.transcript_refs = transcript_refs.into_iter().collect();
    state.provider_transcript_refs = provider_transcript_refs.into_iter().collect();
    state.updated_at_ms = now_ms();
    state
}

pub fn active_preview_ids(state: &TaskContextState) -> Vec<String> {
    state
        .preview_tx_table
        .iter()
        .filter(|item| item.status == "created")
        .map(|item| item.preview_id.clone())
        .collect()
}

pub fn active_approval_token_ids(state: &TaskContextState) -> Vec<String> {
    state
        .approval_token_table
        .iter()
        .filter(|item| item.status == "active")
        .map(|item| item.approval_token_id.clone())
        .collect()
}

fn apply_context_update(state: &mut TaskContextState, data: &Value) {
    if let Some(value) = data.get("current_turn_index").and_then(Value::as_u64) {
        state.current_turn_index = value as usize;
    }
    if let Some(value) = data.get("next_turn_index").and_then(Value::as_u64) {
        state.next_turn_index = value as usize;
    }
    if let Some(value) = data.get("last_turn_id").and_then(Value::as_str) {
        state.last_turn_id = Some(value.to_string());
    }
    if let Some(value) = data.get("last_decision_id").and_then(Value::as_str) {
        state.last_decision_id = Some(value.to_string());
    }
    if let Some(value) = data.get("latest_observation_ref").and_then(Value::as_str) {
        state.latest_observation_ref = Some(value.to_string());
    }
    if let Some(value) = data.get("provider_transcript_ref").and_then(Value::as_str) {
        state.provider_transcript_ref = Some(value.to_string());
    }
    if let Some(value) = data
        .get("provider_transcript_summary_ref")
        .and_then(Value::as_str)
    {
        state.provider_transcript_summary_ref = Some(value.to_string());
    }
    if let Some(value) = data.get("working_memory_ref").and_then(Value::as_str) {
        state.working_memory_ref = Some(value.to_string());
    }
    if let Some(value) = data.get("status").and_then(Value::as_str) {
        state.status = value.to_string();
        state.waiting_for = waiting_for_status(value).map(ToString::to_string);
    }
}

fn collect_artifacts_and_verifications(
    event: &ProcessEvent,
    artifacts: &mut BTreeMap<String, ArtifactContextEntry>,
    verifications: &mut Vec<VerificationContextEntry>,
) {
    let capability_id = event_capability_id(event).map(ToString::to_string);
    for payload in event_payloads(event) {
        if let Some(path) = payload.get("artifact_path").and_then(Value::as_str) {
            let entry = artifacts
                .entry(path.to_string())
                .or_insert(ArtifactContextEntry {
                    path: path.to_string(),
                    last_event_id: event.event_id,
                    producing_capability_id: capability_id.clone(),
                    verified: false,
                    audited: false,
                    local_audit_completed: false,
                    local_audit_hard_risk_pass: None,
                    model_audited: false,
                });
            entry.last_event_id = event.event_id;
            if capability_id.as_deref() == Some("os.verify_artifact")
                && receipt_status(event, payload) == Some("success")
            {
                entry.verified = true;
            }
            if capability_id.as_deref() == Some("artifact.audit_quality") {
                entry.audited = true;
                entry.local_audit_completed = payload
                    .get("data")
                    .and_then(|data| data.get("local_mechanical_audit_completed"))
                    .or_else(|| payload.get("local_mechanical_audit_completed"))
                    .and_then(Value::as_bool)
                    .unwrap_or(true);
                entry.local_audit_hard_risk_pass = payload
                    .get("data")
                    .and_then(|data| data.get("hard_risk_pass"))
                    .or_else(|| payload.get("hard_risk_pass"))
                    .and_then(Value::as_bool);
            }
            if (capability_id.as_deref() == Some("model.audit_artifact_quality")
                || event.event_type == "artifact_model_audit_receipt")
                && receipt_status(event, payload) == Some("success")
            {
                entry.model_audited = true;
            }
        }
        if let Some(path) = payload.get("archive_path").and_then(Value::as_str) {
            artifacts
                .entry(path.to_string())
                .or_insert(ArtifactContextEntry {
                    path: path.to_string(),
                    last_event_id: event.event_id,
                    producing_capability_id: capability_id.clone(),
                    verified: false,
                    audited: false,
                    local_audit_completed: false,
                    local_audit_hard_risk_pass: None,
                    model_audited: false,
                });
        }
        if matches!(
            capability_id.as_deref(),
            Some("os.verify_artifact")
                | Some("artifact.audit_quality")
                | Some("artifact.verify_coverage")
                | Some("artifact.verify_typed")
                | Some("model.audit_artifact_quality")
        ) || event.event_type == "artifact_model_audit_receipt"
        {
            verifications.push(VerificationContextEntry {
                artifact_path: payload
                    .get("artifact_path")
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
                capability_id: capability_id
                    .clone()
                    .unwrap_or_else(|| event.event_type.clone()),
                status: receipt_status(event, payload)
                    .unwrap_or("unknown")
                    .to_string(),
                event_id: event.event_id,
            });
        }
    }
}

fn closure_gaps(data: &Value) -> Vec<String> {
    let mut gaps = Vec::new();
    for key in [
        "audit_gaps",
        "coverage_gaps",
        "failed_verifications",
        "missing_artifacts",
        "model_audit_gaps",
        "pending_approvals",
        "pending_mutations",
        "typed_artifact_verifier_gaps",
        "unresolved_required_failures",
    ] {
        if let Some(items) = data
            .get("data")
            .and_then(|inner| inner.get(key))
            .or_else(|| data.get(key))
            .and_then(Value::as_array)
        {
            for item in items {
                if let Some(text) = item.as_str() {
                    gaps.push(format!("{key}: {text}"));
                }
            }
        }
    }
    gaps
}

fn event_payloads(event: &ProcessEvent) -> Vec<&Value> {
    let mut payloads = vec![&event.data];
    if let Some(data) = event.data.get("data") {
        payloads.push(data);
        if let Some(nested) = data.get("data") {
            payloads.push(nested);
        }
    }
    payloads
}

fn event_runtime_id(event: &ProcessEvent) -> Option<&str> {
    event.data.get("runtime_id").and_then(Value::as_str)
}

fn event_session_id(event: &ProcessEvent) -> Option<&str> {
    event
        .data
        .get("task_agent_session_id")
        .and_then(Value::as_str)
        .or_else(|| event.data.get("session_id").and_then(Value::as_str))
        .or_else(|| event_runtime_id(event))
}

fn event_capability_id(event: &ProcessEvent) -> Option<&str> {
    event
        .data
        .get("capability_id")
        .and_then(Value::as_str)
        .or_else(|| {
            event
                .data
                .get("data")
                .and_then(|data| data.get("capability_id"))
                .and_then(Value::as_str)
        })
}

fn event_tx_id(event: &ProcessEvent) -> Option<&str> {
    event.data.get("tx_id").and_then(Value::as_str)
}

fn turn_index_from_event(event: &ProcessEvent) -> Option<usize> {
    event
        .data
        .get("turn_index")
        .and_then(Value::as_u64)
        .map(|value| value as usize)
}

fn string_field(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn strings_field(value: &Value, key: &str) -> Vec<String> {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(|item| item.replace('\\', "/"))
                .filter(|item| !item.trim().is_empty())
                .collect()
        })
        .unwrap_or_default()
}

fn preview_operations_state(value: &Value) -> Vec<PreviewOperationState> {
    value
        .get("executable_operations")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let capability_id = item.get("capability_id").and_then(Value::as_str)?;
                    Some(PreviewOperationState {
                        capability_id: capability_id.to_string(),
                        target_paths: strings_field(item, "target_paths"),
                        human_description: item
                            .get("human_description")
                            .and_then(Value::as_str)
                            .map(ToString::to_string),
                        rollback_policy: item
                            .get("rollback_policy")
                            .and_then(Value::as_str)
                            .map(ToString::to_string),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn latest_open_preview_id(previews: &BTreeMap<String, PreviewTxState>) -> Option<String> {
    previews
        .values()
        .rev()
        .find(|item| matches!(item.status.as_str(), "created" | "approved"))
        .map(|item| item.preview_id.clone())
}

fn waiting_for_status(status: &str) -> Option<&'static str> {
    match status {
        "waiting_approval" => Some("approval"),
        "waiting_user" => Some("user_input"),
        _ => None,
    }
}

fn receipt_status<'a>(event: &'a ProcessEvent, payload: &'a Value) -> Option<&'a str> {
    payload
        .get("status")
        .and_then(Value::as_str)
        .or_else(|| event.data.get("status").and_then(Value::as_str))
}

fn event_error_summary(event: &ProcessEvent) -> Option<String> {
    event
        .data
        .get("error")
        .and_then(|err| {
            err.get("message")
                .and_then(Value::as_str)
                .or_else(|| err.get("code").and_then(Value::as_str))
        })
        .or_else(|| event.data.get("message").and_then(Value::as_str))
        .or_else(|| event.data.get("reason").and_then(Value::as_str))
        .map(ToString::to_string)
}
