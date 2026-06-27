use std::collections::BTreeSet;
use std::{fs, io};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{
    terminal_command_mutation_detected, ApprovalRuntime, CapabilityDescriptor, CapabilityReceipt,
    CapabilityToken, ExecutablePreviewOperation, PreviewTx, ProcessEvent, ProcessTruthStore,
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum CapabilityApprovalPolicy {
    None,
    ReadOnly,
    PreviewOnly,
    WorkspaceMutation,
    PreviewBoundMutation,
    ArtifactCreate,
    TerminalUnknownSideEffect,
    // Legacy names kept for backward-compatible tests and old receipts. New
    // descriptors should normalize to the canonical policy names above.
    SourceMutationRequired,
    PreviewBoundArtifact,
}

impl CapabilityApprovalPolicy {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::ReadOnly => "read_only",
            Self::PreviewOnly => "preview_only",
            Self::WorkspaceMutation => "workspace_mutation",
            Self::PreviewBoundMutation => "preview_bound_mutation",
            Self::ArtifactCreate => "artifact_create",
            Self::TerminalUnknownSideEffect => "terminal_unknown_side_effect",
            Self::SourceMutationRequired => "source_mutation_required",
            Self::PreviewBoundArtifact => "preview_bound_artifact",
        }
    }

    pub fn from_descriptor(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "" | "none" => Self::None,
            "read_only" => Self::ReadOnly,
            "preview_only" => Self::PreviewOnly,
            "workspace_mutation" | "source_mutation_required" => Self::WorkspaceMutation,
            "preview_bound_mutation" | "preview_bound_artifact" => Self::PreviewBoundMutation,
            "artifact_create" => Self::ArtifactCreate,
            "terminal_unknown_side_effect" | "dynamic_terminal_mutation" => {
                Self::TerminalUnknownSideEffect
            }
            // Dynamic write policy is resolved from the action arguments before
            // prepare_capability_approval receives the request.
            "dynamic_write_kind" => Self::None,
            _ => Self::None,
        }
    }

    pub fn canonical(&self) -> Self {
        match self {
            Self::SourceMutationRequired => Self::WorkspaceMutation,
            Self::PreviewBoundArtifact => Self::PreviewBoundMutation,
            other => other.clone(),
        }
    }

    fn is_uncontrolled(&self) -> bool {
        matches!(
            self.canonical(),
            Self::None | Self::ReadOnly | Self::PreviewOnly
        )
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CapabilityApprovalRequest {
    pub capability_id: String,
    pub policy: CapabilityApprovalPolicy,
    pub target_paths: Vec<String>,
    pub target_path_schema: String,
    pub explicit_approval_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CapabilityApprovalGuard {
    pub approval_token_id: String,
    pub capability_id: String,
    pub target_paths: Vec<String>,
    pub policy: CapabilityApprovalPolicy,
    pub binding: String,
}

pub fn prepare_capability_approval(
    truth: &ProcessTruthStore,
    token: &CapabilityToken,
    mut request: CapabilityApprovalRequest,
) -> io::Result<Result<Option<CapabilityApprovalGuard>, CapabilityReceipt>> {
    request.policy = request.policy.canonical();
    request.target_paths = normalize_scope_items(request.target_paths);
    if !request.policy.is_uncontrolled()
        || request.policy == CapabilityApprovalPolicy::ArtifactCreate
    {
        truth.append_event(
            Some(&token.pid),
            "capability_approval_bypassed",
            json!({
                "capability_id": request.capability_id,
                "approval_policy": request.policy.as_str(),
                "target_paths": request.target_paths,
                "target_path_schema": request.target_path_schema,
                "provided_approval_id": request.explicit_approval_id,
                "mode": "rc0_run_through",
                "runtime_note": "Preview/approve/reject/edit blocking flow is disabled; capability execution proceeds through normal Kernel hard boundaries, runtime receipts, and rollback evidence.",
            }),
        )?;
    }
    Ok(Ok(None))
}

pub fn finalize_capability_approval(
    truth: &ProcessTruthStore,
    pid: &str,
    guard: Option<&CapabilityApprovalGuard>,
    receipt: &CapabilityReceipt,
) -> io::Result<()> {
    let Some(guard) = guard else {
        return Ok(());
    };
    if receipt.status != "success" {
        return Ok(());
    }
    let target_refs = guard
        .target_paths
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    ApprovalRuntime::new(truth.clone()).mark_token_used(
        pid,
        &guard.approval_token_id,
        &guard.capability_id,
        &target_refs,
    )?;
    truth.append_event(
        Some(pid),
        "capability_approval_finalized",
        json!({
            "approval_token_id": guard.approval_token_id,
            "capability_id": guard.capability_id,
            "target_paths": guard.target_paths,
            "approval_policy": guard.policy.as_str(),
            "binding": guard.binding,
            "receipt_status": receipt.status,
        }),
    )?;
    Ok(())
}

pub fn descriptor_approval_policy(capability_id: &str, side_effects: &[&str]) -> String {
    if capability_id == "os.write_file" {
        return "dynamic_write_kind".to_string();
    }
    if capability_id == "terminal.run_command" {
        return CapabilityApprovalPolicy::TerminalUnknownSideEffect
            .as_str()
            .to_string();
    }
    if preview_only_capability(capability_id) {
        return CapabilityApprovalPolicy::PreviewOnly.as_str().to_string();
    }
    if workspace_mutation_capability(capability_id) {
        return CapabilityApprovalPolicy::WorkspaceMutation
            .as_str()
            .to_string();
    }
    if artifact_create_capability(capability_id, side_effects) {
        return CapabilityApprovalPolicy::ArtifactCreate
            .as_str()
            .to_string();
    }
    CapabilityApprovalPolicy::ReadOnly.as_str().to_string()
}

pub fn build_capability_approval_request(
    truth: &ProcessTruthStore,
    descriptor: &CapabilityDescriptor,
    arguments: &Value,
    explicit_approval_id: Option<String>,
) -> io::Result<CapabilityApprovalRequest> {
    let mut policy = effective_approval_policy(descriptor, arguments);
    let target_paths = extract_approval_target_paths(truth, descriptor, arguments, &policy)?;
    policy = upgrade_artifact_create_policy_for_existing_targets(
        truth,
        descriptor,
        arguments,
        &target_paths,
        policy,
    )?;
    Ok(CapabilityApprovalRequest {
        capability_id: descriptor.capability_id.clone(),
        policy,
        target_paths,
        target_path_schema: descriptor.target_path_schema.clone(),
        explicit_approval_id,
    })
}

pub fn bind_preview_capability_receipt_to_tx(
    truth: &ProcessTruthStore,
    pid: &str,
    descriptor: &CapabilityDescriptor,
    receipt: &CapabilityReceipt,
) -> io::Result<Option<PreviewTx>> {
    if CapabilityApprovalPolicy::from_descriptor(&descriptor.approval_policy).canonical()
        != CapabilityApprovalPolicy::PreviewOnly
        || receipt.status != "success"
    {
        return Ok(None);
    }
    truth.append_event(
        Some(pid),
        "capability_preview_tx_bypassed",
        json!({
            "capability_id": receipt.capability_id,
            "receipt_status": receipt.status,
            "preview_disabled": true,
            "no_preview_created": true,
            "approval_required": false,
            "waiting_for_approval": false,
            "kernel_wrapper": "capability_preview_tx",
            "runtime_note": "Preview/approve/reject/edit blocking flow is disabled for RC0 run-through. Preview-only receipts no longer create approval transactions.",
        }),
    )?;
    Ok(None)
}

fn workspace_mutation_capability(capability_id: &str) -> bool {
    matches!(
        capability_id,
        "workspace.apply_organize_tx"
            | "workspace.rename_batch_apply"
            | "os.write_source_mutation_apply"
            | "os.move_path"
            | "os.rename_path"
            | "os.delete_path"
            | "office.docx.rewrite_in_place"
    )
}

pub fn preview_only_capability(capability_id: &str) -> bool {
    matches!(
        capability_id,
        "workspace.plan_organize"
            | "workspace.rename_batch_preview"
            | "os.write_source_mutation_preview"
            | "office.docx.rewrite_in_place_preview"
    )
}

fn artifact_create_capability(capability_id: &str, side_effects: &[&str]) -> bool {
    if !side_effects.iter().any(|item| *item == "write") {
        return false;
    }
    matches!(
        capability_id,
        "os.write_artifact"
            | "os.write_temp_dataset"
            | "os.copy_path"
            | "os.unzip"
            | "os.zip"
            | "workspace.tree_index"
            | "workspace.perf_inventory"
            | "dataset.export_csv"
            | "dataset.export_markdown"
            | "artifact.copy_source_set"
            | "office.docx.create"
            | "office.docx.rewrite_save_as"
            | "package.build_zip"
    )
}

fn normalize_scope_items(items: Vec<String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut normalized = Vec::new();
    for item in items {
        let value = item.replace('\\', "/").trim().to_string();
        if value.is_empty() || value == "*" || !seen.insert(value.clone()) {
            continue;
        }
        normalized.push(value);
    }
    normalized
}

fn effective_approval_policy(
    descriptor: &CapabilityDescriptor,
    arguments: &Value,
) -> CapabilityApprovalPolicy {
    match descriptor.capability_id.as_str() {
        "os.write_file" => match arguments.get("write_kind").and_then(Value::as_str) {
            None => CapabilityApprovalPolicy::ReadOnly,
            Some("source_mutation") => CapabilityApprovalPolicy::WorkspaceMutation,
            Some("temp_dataset") => CapabilityApprovalPolicy::ArtifactCreate,
            Some("artifact") => CapabilityApprovalPolicy::ArtifactCreate,
            _ => CapabilityApprovalPolicy::ReadOnly,
        },
        "terminal.run_command" => {
            let argv = arguments
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
            if terminal_command_mutation_detected(&argv) {
                let target_paths = json_string_array(arguments, "target_paths");
                if target_paths.is_empty() || terminal_command_high_risk_detected(&argv) {
                    CapabilityApprovalPolicy::TerminalUnknownSideEffect
                } else {
                    CapabilityApprovalPolicy::ArtifactCreate
                }
            } else {
                CapabilityApprovalPolicy::ReadOnly
            }
        }
        _ => CapabilityApprovalPolicy::from_descriptor(&descriptor.approval_policy).canonical(),
    }
}

fn upgrade_artifact_create_policy_for_existing_targets(
    truth: &ProcessTruthStore,
    descriptor: &CapabilityDescriptor,
    arguments: &Value,
    target_paths: &[String],
    policy: CapabilityApprovalPolicy,
) -> io::Result<CapabilityApprovalPolicy> {
    if policy != CapabilityApprovalPolicy::ArtifactCreate {
        return Ok(policy);
    }
    if source_target_overlap(arguments) {
        return Ok(CapabilityApprovalPolicy::PreviewBoundMutation);
    }
    let guard = crate::WorkspaceGuard::new(truth.workspace_root())?;
    for target_path in target_paths {
        if target_path.trim().is_empty() {
            continue;
        }
        let resolved = guard
            .resolve_workspace_path(target_path)
            .map_err(|err| io::Error::new(io::ErrorKind::PermissionDenied, err))?;
        if resolved.exists() {
            return Ok(CapabilityApprovalPolicy::PreviewBoundMutation);
        }
    }
    if descriptor.capability_id == "office.docx.rewrite_save_as"
        && target_paths.iter().any(|path| {
            arguments
                .get("input_path")
                .and_then(Value::as_str)
                .is_some_and(|input| normalize_path_string(input) == normalize_path_string(path))
        })
    {
        return Ok(CapabilityApprovalPolicy::PreviewBoundMutation);
    }
    Ok(policy)
}

pub fn expand_preview_target_paths_for_actions(
    proposed_actions: &[String],
    target_paths: Vec<String>,
) -> Vec<String> {
    let mut paths = normalize_scope_items(target_paths);
    let actions = proposed_actions
        .iter()
        .map(|item| item.trim())
        .collect::<BTreeSet<_>>();
    if actions.contains("package.build_zip") {
        for implicit in ["PACK_MANIFEST.md", "SHA256SUMS.txt", "PERF_NOTES.json"] {
            if !paths
                .iter()
                .any(|path| normalize_path_string(path) == implicit)
            {
                paths.push(implicit.to_string());
            }
        }
    }
    normalize_scope_items(paths)
}

pub fn executable_preview_operations_from_scope(
    proposed_actions: &[String],
    target_paths: &[String],
    human_description: Option<String>,
    arguments: Value,
    rollback_policy: Option<String>,
) -> Vec<ExecutablePreviewOperation> {
    let paths = normalize_scope_items(target_paths.to_vec());
    if paths.is_empty() {
        return Vec::new();
    }
    let mut seen = BTreeSet::new();
    proposed_actions
        .iter()
        .filter_map(|action| {
            let capability_id = action.trim();
            if !looks_like_canonical_capability_id(capability_id)
                || !seen.insert(capability_id.to_string())
            {
                return None;
            }
            Some(ExecutablePreviewOperation {
                capability_id: capability_id.to_string(),
                arguments: arguments.clone(),
                target_paths: paths.clone(),
                human_description: human_description
                    .clone()
                    .unwrap_or_else(|| capability_id.to_string()),
                rollback_policy: rollback_policy.clone(),
            })
        })
        .collect()
}

pub fn looks_like_canonical_capability_id(value: &str) -> bool {
    let value = value.trim();
    !value.is_empty()
        && value != "*"
        && value.contains('.')
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'))
}

pub fn artifact_path_generated_by_current_job(
    truth: &ProcessTruthStore,
    artifact_path: &str,
) -> io::Result<bool> {
    let normalized = normalize_path_string(artifact_path);
    Ok(truth.read_events()?.iter().any(|event| {
        event_payloads(event).into_iter().any(|payload| {
            if payload.get("status").and_then(Value::as_str) == Some("blocked") {
                return false;
            }
            if let Some(path) = payload
                .get("artifact_path")
                .or_else(|| payload.get("archive_path"))
                .or_else(|| payload.get("destination_path"))
                .and_then(Value::as_str)
            {
                if normalize_path_string(path) == normalized {
                    return true;
                }
            }
            payload
                .get("artifacts")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(Value::as_str)
                        .any(|path| normalize_path_string(path) == normalized)
                })
                .unwrap_or(false)
        })
    }))
}

fn source_target_overlap(arguments: &Value) -> bool {
    let sources = ["source_path", "input_path", "archive_path", "before_path"]
        .iter()
        .filter_map(|key| arguments.get(*key).and_then(Value::as_str))
        .map(normalize_path_string)
        .collect::<BTreeSet<_>>();
    if sources.is_empty() {
        return false;
    }
    [
        "path",
        "artifact_path",
        "output_path",
        "destination_path",
        "destination_zip_path",
        "destination_dir",
        "tree_path",
        "manifest_path",
        "checksums_path",
        "perf_notes_path",
        "after_path",
    ]
    .iter()
    .filter_map(|key| arguments.get(*key).and_then(Value::as_str))
    .map(normalize_path_string)
    .any(|target| sources.contains(&target))
}

fn terminal_command_high_risk_detected(argv: &[String]) -> bool {
    let joined = format!(" {} ", argv.join(" ").to_ascii_lowercase());
    [
        " remove-item ",
        " rm ",
        " del ",
        " erase ",
        " rmdir ",
        " move-item ",
        " move ",
        " mv ",
        " rename-item ",
        " ren ",
        " icacls ",
        " chmod ",
        " chown ",
        " takeown ",
        " set-acl ",
        " reg ",
        " reg.exe ",
        " set-executionpolicy ",
        " new-service ",
        " sc.exe ",
    ]
    .iter()
    .any(|pattern| joined.contains(pattern))
}

fn normalize_path_string(path: &str) -> String {
    path.replace('\\', "/")
        .trim()
        .trim_start_matches("./")
        .to_string()
}

fn extract_approval_target_paths(
    truth: &ProcessTruthStore,
    descriptor: &CapabilityDescriptor,
    arguments: &Value,
    policy: &CapabilityApprovalPolicy,
) -> io::Result<Vec<String>> {
    if policy.is_uncontrolled() {
        return Ok(Vec::new());
    }
    let mut paths = Vec::<String>::new();
    paths.extend(json_string_array(arguments, "target_paths"));
    for key in approval_target_path_keys(descriptor.capability_id.as_str(), policy) {
        if let Some(value) = arguments.get(key).and_then(Value::as_str) {
            paths.push(value.to_string());
        }
    }
    for key in ["organize_plan_ref", "rename_plan_ref"] {
        if let Some(plan_ref) = arguments.get(key).and_then(Value::as_str) {
            paths.extend(plan_target_paths(truth, plan_ref)?);
        }
    }
    match descriptor.capability_id.as_str() {
        "workspace.tree_index" if !has_any_path(arguments, &["tree_path", "path"]) => {
            paths.push("TREE.md".to_string());
        }
        "workspace.perf_inventory" if !has_any_path(arguments, &["output_path", "path"]) => {
            paths.push("PERF_NOTES.json".to_string());
        }
        "package.build_zip" => {
            if arguments
                .get("manifest_path")
                .and_then(Value::as_str)
                .is_none()
            {
                paths.push("PACK_MANIFEST.md".to_string());
            }
            if arguments
                .get("checksums_path")
                .and_then(Value::as_str)
                .is_none()
            {
                paths.push("SHA256SUMS.txt".to_string());
            }
            if arguments
                .get("perf_notes_path")
                .and_then(Value::as_str)
                .is_none()
            {
                paths.push("PERF_NOTES.json".to_string());
            }
        }
        _ => {}
    }
    Ok(normalize_scope_items(paths))
}

fn approval_target_path_keys(
    capability_id: &str,
    policy: &CapabilityApprovalPolicy,
) -> &'static [&'static str] {
    match capability_id {
        "office.docx.rewrite_save_as" => &["output_path"],
        "office.docx.rewrite_in_place" | "office.docx.rewrite_in_place_preview" => &["input_path"],
        "office.docx.create" => &["output_path"],
        "os.copy_path"
            if matches!(
                policy,
                CapabilityApprovalPolicy::ArtifactCreate
                    | CapabilityApprovalPolicy::PreviewBoundMutation
            ) =>
        {
            &["destination_path"]
        }
        "os.unzip"
            if matches!(
                policy,
                CapabilityApprovalPolicy::ArtifactCreate
                    | CapabilityApprovalPolicy::PreviewBoundMutation
            ) =>
        {
            &["destination_dir"]
        }
        "os.copy_path" | "os.move_path" | "os.rename_path" => &["source_path", "destination_path"],
        "os.delete_path" => &["path"],
        "os.write_file"
        | "os.write_artifact"
        | "os.write_temp_dataset"
        | "os.write_source_mutation_preview"
        | "os.write_source_mutation_apply" => &["path", "artifact_path"],
        "os.zip" => &["destination_zip_path"],
        "os.unzip" => &["archive_path", "destination_dir"],
        "artifact.copy_source_set" => &["destination_dir"],
        "workspace.tree_index" => &["tree_path"],
        "workspace.perf_inventory" => &["output_path"],
        "dataset.export_csv" | "dataset.export_markdown" => &["output_path"],
        "package.build_zip" => &[
            "destination_zip_path",
            "manifest_path",
            "checksums_path",
            "perf_notes_path",
        ],
        _ if matches!(
            policy,
            CapabilityApprovalPolicy::ArtifactCreate
                | CapabilityApprovalPolicy::PreviewBoundMutation
        ) =>
        {
            &[
                "path",
                "artifact_path",
                "output_path",
                "destination_path",
                "destination_zip_path",
                "destination_dir",
                "tree_path",
                "manifest_path",
                "checksums_path",
                "perf_notes_path",
                "after_path",
            ]
        }
        _ => &[
            "path",
            "artifact_path",
            "output_path",
            "destination_path",
            "destination_zip_path",
            "destination_dir",
            "tree_path",
            "manifest_path",
            "checksums_path",
            "perf_notes_path",
            "archive_path",
            "input_path",
            "source_path",
            "before_path",
            "after_path",
        ],
    }
}

fn plan_target_paths(truth: &ProcessTruthStore, plan_ref: &str) -> io::Result<Vec<String>> {
    let path = truth.resolve_blob_ref(plan_ref)?;
    let value: Value = serde_json::from_str(&fs::read_to_string(path)?).map_err(crate::json_err)?;
    let mut targets = Vec::new();
    if let Some(operations) = value.get("operations").and_then(Value::as_array) {
        for op in operations {
            if let Some(path) = op.get("source_path").and_then(Value::as_str) {
                targets.push(path.to_string());
            }
            if let Some(path) = op.get("destination_path").and_then(Value::as_str) {
                targets.push(path.to_string());
            }
        }
    }
    Ok(targets)
}

fn has_any_path(arguments: &Value, keys: &[&str]) -> bool {
    keys.iter().any(|key| {
        arguments
            .get(*key)
            .and_then(Value::as_str)
            .is_some_and(|value| !value.trim().is_empty())
    })
}

fn json_string_array(data: &Value, key: &str) -> Vec<String> {
    data.get(key)
        .or_else(|| data.get("data").and_then(|inner| inner.get(key)))
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

fn event_payloads(event: &ProcessEvent) -> Vec<&Value> {
    let mut payloads = vec![&event.data];
    if let Some(data) = event.data.get("data") {
        payloads.push(data);
    }
    payloads
}
