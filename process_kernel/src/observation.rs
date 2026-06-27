use std::io;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{
    active_approval_token_ids, active_preview_ids, default_capability_registry, json_err,
    replay_task_context_state, safe_blob_name, to_json_value, CapabilityDescriptor, ProcessEvent,
    ProcessTruthStore,
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ObservedCapabilityReceipt {
    pub event_id: u64,
    pub event_type: String,
    pub capability_id: String,
    pub status: String,
    pub data: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TaskObservation {
    pub observation_id: String,
    pub job_id: String,
    pub root_pid: String,
    pub runtime_id: String,
    pub goal_ref: String,
    pub acceptance_contract_ref: Option<String>,
    pub observation_frame_ref: String,
    pub task_context_ref: String,
    pub provider_transcript_ref: Option<String>,
    pub provider_transcript_summary_ref: Option<String>,
    pub latest_raw_result_ref: Option<String>,
    pub recent_raw_result_refs: Vec<String>,
    pub process_events_ref: String,
    pub capability_registry_ref: String,
    pub latest_receipts: Vec<ObservedCapabilityReceipt>,
    pub artifact_refs: Vec<String>,
    pub pending_approvals: Vec<String>,
    pub approval_tokens: Vec<String>,
    pub waiting_user: bool,
    pub approved: bool,
    pub cancelled: bool,
    pub event_count: usize,
}

impl TaskObservation {
    pub fn has_successful_capability(&self, capability_id: &str) -> bool {
        self.latest_receipts
            .iter()
            .any(|item| item.capability_id == capability_id && item.status == "success")
    }

    pub fn latest_success_data(&self, capability_id: &str) -> Option<&Value> {
        self.latest_receipts
            .iter()
            .rev()
            .find(|item| item.capability_id == capability_id && item.status == "success")
            .map(|item| &item.data)
    }

    pub fn latest_success_ref(&self, capability_id: &str, key: &str) -> Option<String> {
        self.latest_success_data(capability_id)
            .and_then(|value| value.get(key))
            .and_then(Value::as_str)
            .map(ToString::to_string)
    }

    pub fn has_artifact(&self, path: &str) -> bool {
        self.artifact_refs.iter().any(|item| item == path)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct RawToolResult {
    pub event_id: u64,
    pub event_type: String,
    pub capability_id: String,
    pub status: String,
    pub raw_result_ref: String,
    pub raw_event: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct RawObservationFrame {
    pub observation_id: String,
    pub job_id: String,
    pub root_pid: String,
    pub runtime_id: String,
    pub goal_ref: String,
    pub task_context_ref: String,
    pub task_context_state: Value,
    pub provider_transcript_ref: Option<String>,
    pub provider_transcript_summary_ref: Option<String>,
    pub capability_registry_ref: String,
    pub process_events_ref: String,
    pub recent_raw_results: Vec<RawToolResult>,
    pub artifact_refs: Vec<String>,
    pub pending_approvals: Vec<String>,
    pub approval_tokens: Vec<String>,
    pub waiting_user: bool,
    pub approved: bool,
    pub cancelled: bool,
    pub event_count: usize,
}

#[derive(Clone, Debug)]
pub struct ObservationBuilder {
    truth: ProcessTruthStore,
    registry: Vec<CapabilityDescriptor>,
}

impl ObservationBuilder {
    pub fn new(truth: ProcessTruthStore, registry: Vec<CapabilityDescriptor>) -> Self {
        Self { truth, registry }
    }

    pub fn default_registry(truth: ProcessTruthStore) -> Self {
        Self {
            truth,
            registry: default_capability_registry(),
        }
    }

    pub fn build(
        &self,
        root_pid: &str,
        runtime_id: &str,
        user_goal: &str,
    ) -> io::Result<TaskObservation> {
        let events = self.truth.read_events()?;
        let observation_id = format!("obs_{}_{}", safe_blob_name(runtime_id), events.len());
        let goal_ref = self.truth.write_blob(
            &format!("observations/{observation_id}_goal.txt"),
            user_goal.as_bytes(),
        )?;
        let task_context = replay_task_context_state(&self.truth, root_pid, runtime_id)?;
        let provider_transcript_ref = task_context.provider_transcript_ref.clone();
        let provider_transcript_summary_ref = task_context.provider_transcript_summary_ref.clone();
        let task_context_state = task_context.to_value();
        let task_context_ref = self.truth.write_blob(
            &format!("observations/{observation_id}_task_context_state.json"),
            &serde_json::to_vec_pretty(&task_context_state).map_err(json_err)?,
        )?;
        let process_events_ref = self.truth.write_blob(
            &format!("observations/{observation_id}_process_events.json"),
            &serde_json::to_vec_pretty(&events).map_err(json_err)?,
        )?;
        let capability_registry_ref = self.truth.write_blob(
            &format!("observations/{observation_id}_capabilities.json"),
            &serde_json::to_vec_pretty(&self.registry).map_err(json_err)?,
        )?;
        let latest_receipts = observed_receipts(&events);
        let replay = self.truth.replay()?;
        let recent_raw_results = self.recent_raw_tool_results(&events, 8)?;
        let latest_raw_result_ref = recent_raw_results
            .last()
            .map(|item| item.raw_result_ref.clone());
        let recent_raw_result_refs = recent_raw_results
            .iter()
            .map(|item| item.raw_result_ref.clone())
            .collect::<Vec<_>>();
        let context_for_lists = replay_task_context_state(&self.truth, root_pid, runtime_id)?;
        let pending_approvals = active_preview_ids(&context_for_lists);
        let approval_tokens = active_approval_token_ids(&context_for_lists);
        let approved = events
            .iter()
            .any(|event| event.event_type == "user_approval_received");
        let cancelled = events
            .iter()
            .any(|event| event.event_type == "user_cancel_requested");
        let waiting_user = events
            .iter()
            .any(|event| event.event_type == "job_waiting_user");
        let frame = RawObservationFrame {
            observation_id: observation_id.clone(),
            job_id: self.truth.job_id().to_string(),
            root_pid: root_pid.to_string(),
            runtime_id: runtime_id.to_string(),
            goal_ref: goal_ref.clone(),
            task_context_ref: task_context_ref.clone(),
            task_context_state: task_context_state.clone(),
            provider_transcript_ref: provider_transcript_ref.clone(),
            provider_transcript_summary_ref: provider_transcript_summary_ref.clone(),
            capability_registry_ref: capability_registry_ref.clone(),
            process_events_ref: process_events_ref.clone(),
            recent_raw_results: recent_raw_results.clone(),
            artifact_refs: replay.artifact_refs.clone(),
            pending_approvals: pending_approvals.clone(),
            approval_tokens: approval_tokens.clone(),
            waiting_user,
            approved,
            cancelled,
            event_count: events.len(),
        };
        let observation_frame_ref = self.truth.write_blob(
            &format!("observations/{observation_id}_raw_frame.json"),
            &serde_json::to_vec_pretty(&frame).map_err(json_err)?,
        )?;
        let observation = TaskObservation {
            observation_id,
            job_id: self.truth.job_id().to_string(),
            root_pid: root_pid.to_string(),
            runtime_id: runtime_id.to_string(),
            goal_ref,
            acceptance_contract_ref: None,
            observation_frame_ref,
            task_context_ref,
            provider_transcript_ref,
            provider_transcript_summary_ref,
            latest_raw_result_ref,
            recent_raw_result_refs,
            process_events_ref,
            capability_registry_ref,
            latest_receipts,
            artifact_refs: replay.artifact_refs,
            pending_approvals,
            approval_tokens,
            waiting_user,
            approved,
            cancelled,
            event_count: events.len(),
        };
        self.truth.append_event(
            Some(root_pid),
            "task_agent_observation_built",
            json!({
                "observation_id": observation.observation_id,
                "runtime_id": runtime_id,
                "goal_ref": observation.goal_ref,
                "observation_frame_ref": observation.observation_frame_ref,
                "task_context_ref": observation.task_context_ref,
                "provider_transcript_ref": observation.provider_transcript_ref,
                "provider_transcript_summary_ref": observation.provider_transcript_summary_ref,
                "latest_raw_result_ref": observation.latest_raw_result_ref,
                "recent_raw_result_refs": observation.recent_raw_result_refs,
                "process_events_ref": observation.process_events_ref,
                "capability_registry_ref": observation.capability_registry_ref,
                "receipt_count": observation.latest_receipts.len(),
                "artifact_count": observation.artifact_refs.len(),
                "pending_approvals": observation.pending_approvals.clone(),
                "approval_tokens": observation.approval_tokens.clone(),
                "approved": observation.approved,
                "cancelled": observation.cancelled,
            }),
        )?;
        Ok(observation)
    }

    fn recent_raw_tool_results(
        &self,
        events: &[ProcessEvent],
        limit: usize,
    ) -> io::Result<Vec<RawToolResult>> {
        let mut results = Vec::new();
        for event in events.iter().rev() {
            if results.len() >= limit {
                break;
            }
            let Some((capability_id, status)) = raw_tool_result_identity(event) else {
                continue;
            };
            if capability_id == "model.decide_next_action" {
                continue;
            }
            let raw_event = to_json_value(event)?;
            let raw_result_ref = self.truth.write_blob(
                &format!(
                    "raw_tool_results/event_{}_{}.json",
                    event.event_id,
                    safe_blob_name(&capability_id)
                ),
                &serde_json::to_vec_pretty(&raw_event).map_err(json_err)?,
            )?;
            results.push(RawToolResult {
                event_id: event.event_id,
                event_type: event.event_type.clone(),
                capability_id,
                status,
                raw_result_ref,
                raw_event,
            });
        }
        results.reverse();
        Ok(results)
    }
}

fn raw_tool_result_identity(event: &ProcessEvent) -> Option<(String, String)> {
    match event.event_type.as_str() {
        "capability_receipt" | "command_receipt" | "office_receipt" | "verify_event"
        | "model_call_receipt" => {
            let capability_id = event
                .data
                .get("capability_id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            if capability_id.is_empty() {
                return None;
            }
            let status = event
                .data
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_string();
            Some((capability_id, status))
        }
        "process_action_failed" => {
            let capability_id = event
                .data
                .get("capability_id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            if capability_id.is_empty() {
                return None;
            }
            Some((capability_id, "failed".to_string()))
        }
        "capability_blocked" => {
            let capability_id = event
                .data
                .get("capability_id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            if capability_id.is_empty() {
                return None;
            }
            Some((capability_id, "blocked".to_string()))
        }
        _ => None,
    }
}

fn observed_receipts(events: &[ProcessEvent]) -> Vec<ObservedCapabilityReceipt> {
    let mut receipts = Vec::new();
    for event in events {
        match event.event_type.as_str() {
            "capability_receipt" | "verify_event" | "model_call_receipt" | "command_receipt"
            | "office_receipt" => {
                if let Some(receipt) = event_to_receipt(event) {
                    receipts.push(receipt);
                }
            }
            _ => {}
        }
    }
    receipts
}

fn event_to_receipt(event: &ProcessEvent) -> Option<ObservedCapabilityReceipt> {
    let mut value = event.data.clone();
    if event.event_type == "model_call_receipt" {
        value = json!({
            "capability_id": event.data.get("capability_id"),
            "status": event.data.get("status"),
            "data": event.data,
        });
    }
    let capability_id = value
        .get("capability_id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if capability_id.is_empty() {
        return None;
    }
    let status = value
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("success")
        .to_string();
    let data = value
        .get("data")
        .cloned()
        .unwrap_or_else(|| to_json_value(&value).unwrap_or(Value::Null));
    Some(ObservedCapabilityReceipt {
        event_id: event.event_id,
        event_type: event.event_type.clone(),
        capability_id,
        status,
        data,
    })
}
