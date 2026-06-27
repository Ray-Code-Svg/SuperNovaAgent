use std::collections::BTreeSet;
use std::io;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{now_ms, safe_blob_name, ProcessTruthStore};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ExecutablePreviewOperation {
    pub capability_id: String,
    #[serde(default)]
    pub arguments: Value,
    pub target_paths: Vec<String>,
    pub human_description: String,
    pub rollback_policy: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PreviewTx {
    pub tx_id: String,
    pub preview_id: String,
    pub preview_ref: String,
    pub human_preview_markdown: String,
    pub executable_operations: Vec<ExecutablePreviewOperation>,
    // Backward-compatible display fields. Authorization must use
    // executable_operations[].capability_id, not natural-language text.
    pub proposed_actions: Vec<String>,
    pub target_paths: Vec<String>,
    pub risk_level: String,
    pub rollback_plan_ref: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ApprovalTokenRecord {
    pub approval_token_id: String,
    pub tx_id: String,
    pub preview_id: String,
    pub approved_operation_scope: Vec<String>,
    pub approved_action_scope: Vec<String>,
    pub approved_path_scope: Vec<String>,
    pub allowed_ops_hash: String,
    pub expires_at_ms: u128,
    pub approval_note: String,
}

#[derive(Clone, Debug)]
pub struct ApprovalRuntime {
    truth: ProcessTruthStore,
}

impl ApprovalRuntime {
    pub fn new(truth: ProcessTruthStore) -> Self {
        Self { truth }
    }

    pub fn create_preview_tx(
        &self,
        pid: &str,
        preview_markdown: &str,
        executable_operations: Vec<ExecutablePreviewOperation>,
        risk_level: impl Into<String>,
    ) -> io::Result<PreviewTx> {
        let executable_operations = normalize_executable_operations(executable_operations)?;
        let proposed_actions = operation_capability_ids(&executable_operations);
        let target_paths = operation_target_paths(&executable_operations);
        let preview_id = format!("preview_{}", now_ms());
        let tx_id = format!("preview_tx_{}", now_ms());
        let preview_ref = self.truth.write_blob(
            &format!("previews/{}_preview.md", safe_blob_name(&preview_id)),
            preview_markdown.as_bytes(),
        )?;
        let rollback_plan_ref = self.truth.write_blob(
            &format!("previews/{}_rollback_plan.json", safe_blob_name(&preview_id)),
            serde_json::to_string_pretty(&json!({
                "tx_id": tx_id,
                "preview_id": preview_id,
                "rollback_strategy": "runtime capability tx rollback refs are recorded after mutation",
                "target_paths": target_paths,
                "executable_operations": executable_operations.clone(),
            }))
            .map_err(crate::json_err)?
            .as_bytes(),
        )?;
        let preview = PreviewTx {
            tx_id,
            preview_id,
            preview_ref,
            human_preview_markdown: preview_markdown.to_string(),
            executable_operations,
            proposed_actions,
            target_paths,
            risk_level: risk_level.into(),
            rollback_plan_ref: Some(rollback_plan_ref),
        };
        self.truth.append_event(
            Some(pid),
            "preview_tx_created",
            crate::to_json_value(&preview)?,
        )?;
        self.truth.append_event(
            Some(pid),
            "preview_created",
            json!({
                "preview_id": preview.preview_id,
                "preview_ref": preview.preview_ref,
                "human_preview_markdown": preview.human_preview_markdown.clone(),
                "executable_operations": preview.executable_operations.clone(),
                "proposed_actions": preview.proposed_actions.clone(),
                "target_paths": preview.target_paths.clone(),
                "risk_level": preview.risk_level.clone(),
                "rollback_plan_ref": preview.rollback_plan_ref.clone(),
                "approval_required": true,
            }),
        )?;
        Ok(preview)
    }

    pub fn issue_token_for_latest_preview(
        &self,
        pid: &str,
        approval_note: &str,
    ) -> io::Result<ApprovalTokenRecord> {
        self.issue_token_for_preview_selection(pid, None, approval_note)
    }

    pub fn issue_token_for_preview(
        &self,
        pid: &str,
        approval_id: &str,
        approval_note: &str,
    ) -> io::Result<ApprovalTokenRecord> {
        self.issue_token_for_preview_selection(pid, Some(approval_id), approval_note)
    }

    fn issue_token_for_preview_selection(
        &self,
        pid: &str,
        approval_id: Option<&str>,
        approval_note: &str,
    ) -> io::Result<ApprovalTokenRecord> {
        let events = self.truth.read_events()?;
        let closed_or_approved_tx = events
            .iter()
            .filter(|event| {
                matches!(
                    event.event_type.as_str(),
                    "preview_tx_closed" | "preview_tx_approved" | "approval_token_issued"
                )
            })
            .filter_map(|event| event.data.get("tx_id").and_then(Value::as_str))
            .map(str::to_string)
            .collect::<BTreeSet<_>>();
        let approval_id = approval_id.map(str::trim).filter(|value| !value.is_empty());
        let preview = events
            .iter()
            .rev()
            .find(|event| {
                if event.event_type != "preview_tx_created" {
                    return false;
                }
                let tx_id = event
                    .data
                    .get("tx_id")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let preview_id = event
                    .data
                    .get("preview_id")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let matches_selection = approval_id
                    .map(|selected| selected == tx_id || selected == preview_id)
                    .unwrap_or(true);
                matches_selection && !closed_or_approved_tx.contains(tx_id)
            })
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    approval_id
                        .map(|id| format!("no open preview tx to approve for approval_id: {id}"))
                        .unwrap_or_else(|| "no preview tx to approve".to_string()),
                )
            })?;
        let preview_id = preview
            .data
            .get("preview_id")
            .and_then(Value::as_str)
            .unwrap_or("preview_unknown")
            .to_string();
        let tx_id = preview
            .data
            .get("tx_id")
            .and_then(Value::as_str)
            .unwrap_or(&preview_id)
            .to_string();
        let approved_operations = preview_operation_capability_ids(&preview.data)
            .filter(|items| !items.is_empty())
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "preview tx cannot be approved without explicit executable_operations capability_id scope",
                )
            })?;
        let target_paths = preview_operation_target_paths(&preview.data)
            .filter(|items| !items.is_empty())
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "preview tx cannot be approved without explicit target_paths",
                )
            })?;
        let token = ApprovalTokenRecord {
            approval_token_id: format!("approval_{}_{}", safe_blob_name(&preview_id), now_ms()),
            tx_id,
            preview_id,
            approved_operation_scope: approved_operations.clone(),
            approved_action_scope: approved_operations,
            approved_path_scope: target_paths,
            allowed_ops_hash: approval_scope_hash(&preview.data),
            expires_at_ms: now_ms().saturating_add(24 * 60 * 60 * 1000),
            approval_note: approval_note.to_string(),
        };
        self.truth.append_event(
            Some(pid),
            "preview_tx_approved",
            json!({
                "tx_id": token.tx_id,
                "preview_id": token.preview_id,
                "approval_token_id": token.approval_token_id,
                "approval_note": approval_note,
                "status": "approved",
            }),
        )?;
        self.truth.append_event(
            Some(pid),
            "approval_token_issued",
            crate::to_json_value(&token)?,
        )?;
        Ok(token)
    }

    pub fn validate_token(
        &self,
        approval_token_id: &str,
        capability_id: &str,
        target_paths: &[&str],
    ) -> io::Result<bool> {
        if approval_token_id.trim().is_empty() {
            return Ok(false);
        }
        let events = self.truth.read_events()?;
        let normalized_targets = target_paths
            .iter()
            .map(|item| item.replace('\\', "/"))
            .collect::<Vec<_>>();
        for event in events.iter().rev() {
            if event.event_type != "approval_token_issued" {
                continue;
            }
            if event.data.get("approval_token_id").and_then(Value::as_str)
                != Some(approval_token_id)
            {
                continue;
            }
            if token_is_consumed(&events, approval_token_id) {
                return Ok(false);
            }
            let actions = event
                .data
                .get("approved_operation_scope")
                .or_else(|| event.data.get("approved_action_scope"))
                .and_then(Value::as_array)
                .map(|items| values_to_strings(items))
                .unwrap_or_default();
            let paths = event
                .data
                .get("approved_path_scope")
                .and_then(Value::as_array)
                .map(|items| values_to_strings(items))
                .unwrap_or_default();
            let tx_id = event
                .data
                .get("tx_id")
                .and_then(Value::as_str)
                .unwrap_or("");
            let preview_hash_ok = events.iter().rev().any(|preview_event| {
                preview_event.event_type == "preview_tx_created"
                    && preview_event
                        .data
                        .get("tx_id")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        == tx_id
                    && event
                        .data
                        .get("allowed_ops_hash")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        == approval_scope_hash(&preview_event.data)
            });
            let action_ok = actions.iter().any(|item| item == capability_id);
            let path_ok = normalized_targets.iter().all(|target| {
                paths.iter().any(|scope| {
                    scope == target
                        || target.starts_with(&format!("{}/", scope.trim_end_matches('/')))
                })
            });
            let expires_ok = event
                .data
                .get("expires_at_ms")
                .and_then(Value::as_u64)
                .map(|expires| u128::from(expires) >= now_ms())
                .unwrap_or(true);
            return Ok(action_ok && path_ok && expires_ok && preview_hash_ok);
        }
        Ok(false)
    }

    pub fn mark_token_used(
        &self,
        pid: &str,
        approval_token_id: &str,
        capability_id: &str,
        target_paths: &[&str],
    ) -> io::Result<()> {
        let events = self.truth.read_events()?;
        let token_event = events.iter().rev().find(|event| {
            event.event_type == "approval_token_issued"
                && event.data.get("approval_token_id").and_then(Value::as_str)
                    == Some(approval_token_id)
        });
        let tx_id = token_event
            .and_then(|event| event.data.get("tx_id"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let preview_id = token_event
            .and_then(|event| event.data.get("preview_id"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let normalized_targets = target_paths
            .iter()
            .map(|item| item.replace('\\', "/"))
            .collect::<Vec<_>>();
        self.truth.append_event(
            Some(pid),
            "approval_token_consumed",
            json!({
                "approval_token_id": approval_token_id,
                "tx_id": tx_id,
                "preview_id": preview_id,
                "capability_id": capability_id,
                "target_paths": normalized_targets,
                "status": "consumed",
            }),
        )?;
        if !tx_id.is_empty() {
            self.truth.append_event(
                Some(pid),
                "preview_tx_applied",
                json!({
                    "tx_id": tx_id,
                    "preview_id": preview_id,
                    "approval_token_id": approval_token_id,
                    "capability_id": capability_id,
                    "target_paths": target_paths.iter().map(|item| item.replace('\\', "/")).collect::<Vec<_>>(),
                    "status": "applied",
                }),
            )?;
        }
        self.truth.append_event(
            Some(pid),
            "approval_token_used",
            json!({
                "approval_token_id": approval_token_id,
                "tx_id": tx_id,
                "preview_id": preview_id,
                "capability_id": capability_id,
                "target_paths": target_paths.iter().map(|item| item.replace('\\', "/")).collect::<Vec<_>>(),
            }),
        )?;
        if !tx_id.is_empty() {
            self.truth.append_event(
                Some(pid),
                "preview_tx_closed",
                json!({
                    "tx_id": tx_id,
                    "preview_id": preview_id,
                    "approval_token_id": approval_token_id,
                    "status": "applied",
                }),
            )?;
        }
        Ok(())
    }
}

fn token_is_consumed(events: &[crate::ProcessEvent], approval_token_id: &str) -> bool {
    events.iter().any(|event| {
        matches!(
            event.event_type.as_str(),
            "approval_token_consumed" | "approval_token_used"
        ) && event.data.get("approval_token_id").and_then(Value::as_str) == Some(approval_token_id)
    })
}

fn approval_scope_hash(value: &Value) -> String {
    let rendered = serde_json::to_string(value).unwrap_or_default();
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in rendered.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn normalize_executable_operations(
    operations: Vec<ExecutablePreviewOperation>,
) -> io::Result<Vec<ExecutablePreviewOperation>> {
    if operations.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "preview tx executable_operations cannot be empty",
        ));
    }
    let mut normalized = Vec::new();
    for mut operation in operations {
        operation.capability_id = operation.capability_id.trim().to_string();
        if operation.capability_id.is_empty() || operation.capability_id == "*" {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "preview tx executable_operations capability_id cannot be empty or wildcard",
            ));
        }
        operation.target_paths =
            normalize_preview_scope("executable_operations.target_paths", operation.target_paths)?;
        if operation.human_description.trim().is_empty() {
            operation.human_description = operation.capability_id.clone();
        }
        normalized.push(operation);
    }
    Ok(normalized)
}

fn operation_capability_ids(operations: &[ExecutablePreviewOperation]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    operations
        .iter()
        .filter_map(|operation| {
            let capability_id = operation.capability_id.trim();
            if capability_id.is_empty() || !seen.insert(capability_id.to_string()) {
                return None;
            }
            Some(capability_id.to_string())
        })
        .collect()
}

fn operation_target_paths(operations: &[ExecutablePreviewOperation]) -> Vec<String> {
    let mut paths = Vec::new();
    for operation in operations {
        paths.extend(operation.target_paths.clone());
    }
    normalize_preview_scope("target_paths", paths).unwrap_or_default()
}

fn preview_operations(data: &Value) -> Option<Vec<ExecutablePreviewOperation>> {
    let operations = data.get("executable_operations")?.as_array()?;
    let mut parsed = Vec::new();
    for operation in operations {
        let capability_id = operation
            .get("capability_id")
            .and_then(Value::as_str)?
            .to_string();
        let target_paths = operation
            .get("target_paths")
            .and_then(Value::as_array)
            .map(|items| values_to_strings(items))
            .unwrap_or_default();
        let human_description = operation
            .get("human_description")
            .and_then(Value::as_str)
            .unwrap_or(&capability_id)
            .to_string();
        let arguments = operation.get("arguments").cloned().unwrap_or(Value::Null);
        let rollback_policy = operation
            .get("rollback_policy")
            .and_then(Value::as_str)
            .map(ToString::to_string);
        parsed.push(ExecutablePreviewOperation {
            capability_id,
            arguments,
            target_paths,
            human_description,
            rollback_policy,
        });
    }
    Some(parsed)
}

fn preview_operation_capability_ids(data: &Value) -> Option<Vec<String>> {
    if let Some(operations) = preview_operations(data) {
        let ids = operation_capability_ids(&operations);
        if !ids.is_empty() {
            return Some(ids);
        }
    }
    data.get("proposed_actions")
        .and_then(Value::as_array)
        .map(|items| values_to_strings(items))
}

fn preview_operation_target_paths(data: &Value) -> Option<Vec<String>> {
    if let Some(operations) = preview_operations(data) {
        let paths = operation_target_paths(&operations);
        if !paths.is_empty() {
            return Some(paths);
        }
    }
    data.get("target_paths")
        .and_then(Value::as_array)
        .map(|items| values_to_strings(items))
}

fn values_to_strings(items: &[Value]) -> Vec<String> {
    items
        .iter()
        .filter_map(Value::as_str)
        .map(|item| item.replace('\\', "/"))
        .filter(|item| !item.trim().is_empty())
        .collect()
}

fn normalize_preview_scope(field: &str, items: Vec<String>) -> io::Result<Vec<String>> {
    let mut normalized = Vec::new();
    let mut seen = BTreeSet::new();
    for item in items {
        let value = item.replace('\\', "/").trim().to_string();
        if value == "*" {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("preview tx {field} cannot contain wildcard scope"),
            ));
        }
        if value.is_empty() || !seen.insert(value.clone()) {
            continue;
        }
        normalized.push(value);
    }
    if normalized.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("preview tx {field} cannot be empty"),
        ));
    }
    Ok(normalized)
}
