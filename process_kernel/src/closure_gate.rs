use std::collections::BTreeSet;
use std::io;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{CapabilityReceipt, ProcessEvent, ProcessTruthStore, ReplayState, WorkspaceGuard};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClosureGateResult {
    pub can_complete: bool,
    pub hard_blocks: Vec<ClosureBlock>,
    pub advisory_findings: Vec<ClosureFinding>,
    pub completion_facts: Vec<ClosureFact>,
    pub artifact_roles: Vec<ArtifactRoleEvidence>,
    pub missing_artifacts: Vec<String>,
    pub failed_verifications: Vec<String>,
    pub pending_mutations: Vec<String>,
    pub pending_approvals: Vec<String>,
    pub coverage_gaps: Vec<String>,
    pub audit_gaps: Vec<String>,
    pub model_audit_gaps: Vec<String>,
    pub typed_artifact_verifier_gaps: Vec<String>,
    pub unresolved_required_failures: Vec<String>,
    pub human_review_required: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClosureBlock {
    pub code: String,
    pub source: String,
    pub message: String,
    pub artifact_path: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClosureFinding {
    pub category: String,
    pub source: String,
    pub message: String,
    pub artifact_path: Option<String>,
    pub severity: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClosureFact {
    pub fact_type: String,
    pub source: String,
    pub message: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactRoleEvidence {
    pub artifact_path: String,
    pub role: String,
    pub expected_verifier: String,
}

impl ClosureGateResult {
    pub fn pass() -> Self {
        Self {
            can_complete: true,
            hard_blocks: Vec::new(),
            advisory_findings: Vec::new(),
            completion_facts: Vec::new(),
            artifact_roles: Vec::new(),
            missing_artifacts: Vec::new(),
            failed_verifications: Vec::new(),
            pending_mutations: Vec::new(),
            pending_approvals: Vec::new(),
            coverage_gaps: Vec::new(),
            audit_gaps: Vec::new(),
            model_audit_gaps: Vec::new(),
            typed_artifact_verifier_gaps: Vec::new(),
            unresolved_required_failures: Vec::new(),
            human_review_required: false,
        }
    }
}

pub fn check_closure_gate(
    guard: &WorkspaceGuard,
    truth: &ProcessTruthStore,
    replay: &ReplayState,
) -> io::Result<ClosureGateResult> {
    check_closure_gate_for_artifacts(guard, truth, replay, &replay.artifact_refs)
}

pub fn check_closure_gate_for_claimed_artifacts(
    guard: &WorkspaceGuard,
    truth: &ProcessTruthStore,
    replay: &ReplayState,
    claimed_artifacts: &[String],
) -> io::Result<ClosureGateResult> {
    check_closure_gate_for_artifacts(guard, truth, replay, claimed_artifacts)
}

fn check_closure_gate_for_artifacts(
    guard: &WorkspaceGuard,
    truth: &ProcessTruthStore,
    _replay: &ReplayState,
    artifact_refs: &[String],
) -> io::Result<ClosureGateResult> {
    let events = truth.read_events()?;
    let mut result = ClosureGateResult::pass();
    let scoped_artifacts = artifact_scope_set(artifact_refs);
    result.artifact_roles = artifact_role_evidence(&scoped_artifacts);
    result.missing_artifacts = missing_artifacts(guard, artifact_refs)?;
    result.failed_verifications = hard_failed_verifications(&events);
    result
        .failed_verifications
        .extend(hard_model_audit_failures(&events));
    result.pending_approvals = pending_approvals(&events);
    result.pending_mutations = pending_mutations(&events);
    result.unresolved_required_failures = unresolved_required_failures(&events);
    result.coverage_gaps = coverage_gaps(&events, artifact_refs);
    result.audit_gaps = audit_gaps(&events, artifact_refs);
    result.model_audit_gaps = model_audit_gaps(&events, artifact_refs);
    result.typed_artifact_verifier_gaps = typed_artifact_verifier_gaps(&events, &scoped_artifacts);
    result.human_review_required = !scoped_artifacts.is_empty();
    result.hard_blocks = hard_blocks(&result);
    result.advisory_findings = advisory_findings(&result, &events);
    result.completion_facts = completion_facts(&result, &scoped_artifacts);
    result.can_complete = result.hard_blocks.is_empty();
    Ok(result)
}

pub fn closure_block_receipt(
    job_id: &str,
    pid: &str,
    gate: &ClosureGateResult,
) -> CapabilityReceipt {
    CapabilityReceipt {
        capability_id: "process.complete".to_string(),
        job_id: job_id.to_string(),
        pid: pid.to_string(),
        status: "blocked".to_string(),
        data: json!({
            "reason": "closure_gate_failed",
            "completion_recoverable": true,
            "hard_blocks": gate.hard_blocks,
            "advisory_findings": gate.advisory_findings,
            "completion_facts": gate.completion_facts,
            "artifact_roles": gate.artifact_roles,
            "missing_artifacts": gate.missing_artifacts,
            "failed_verifications": gate.failed_verifications,
            "pending_mutations": gate.pending_mutations,
            "pending_approvals": gate.pending_approvals,
            "coverage_gaps": gate.coverage_gaps,
            "audit_gaps": gate.audit_gaps,
            "model_audit_gaps": gate.model_audit_gaps,
            "typed_artifact_verifier_gaps": gate.typed_artifact_verifier_gaps,
            "unresolved_required_failures": gate.unresolved_required_failures,
            "runtime_note": "closure gate blocks only hard safety/execution facts; advisory findings are evidence for TaskAgent judgment, not runtime strategy",
        }),
    }
}

fn hard_blocks(gate: &ClosureGateResult) -> Vec<ClosureBlock> {
    let mut blocks = Vec::new();
    for item in &gate.missing_artifacts {
        blocks.push(ClosureBlock {
            code: if is_internal_ref(item) {
                "internal_ref_not_user_artifact".to_string()
            } else {
                "artifact_missing_or_unreadable".to_string()
            },
            source: "artifact_ref".to_string(),
            message: item.clone(),
            artifact_path: Some(item.clone()),
        });
    }
    for item in &gate.pending_approvals {
        blocks.push(ClosureBlock {
            code: "pending_approval".to_string(),
            source: "approval_runtime".to_string(),
            message: item.clone(),
            artifact_path: None,
        });
    }
    for item in &gate.pending_mutations {
        blocks.push(ClosureBlock {
            code: "pending_mutation".to_string(),
            source: "capability_kernel".to_string(),
            message: item.clone(),
            artifact_path: None,
        });
    }
    for item in &gate.failed_verifications {
        blocks.push(ClosureBlock {
            code: "hard_verification_failure".to_string(),
            source: "runtime_verifier".to_string(),
            message: item.clone(),
            artifact_path: artifact_path_from_message(item),
        });
    }
    for item in &gate.unresolved_required_failures {
        blocks.push(ClosureBlock {
            code: "unresolved_required_failure".to_string(),
            source: "required_capability".to_string(),
            message: item.clone(),
            artifact_path: None,
        });
    }
    blocks
}

fn advisory_findings(gate: &ClosureGateResult, events: &[ProcessEvent]) -> Vec<ClosureFinding> {
    let mut findings = Vec::new();
    extend_findings(
        &mut findings,
        "coverage",
        "coverage_verifier",
        "advisory",
        &gate.coverage_gaps,
    );
    extend_findings(
        &mut findings,
        "local_audit",
        "artifact.audit_quality",
        "advisory",
        &gate.audit_gaps,
    );
    extend_findings(
        &mut findings,
        "model_audit",
        "model.audit_artifact_quality",
        "advisory",
        &gate.model_audit_gaps,
    );
    extend_findings(
        &mut findings,
        "typed_verifier",
        "artifact.verify_typed",
        "advisory",
        &gate.typed_artifact_verifier_gaps,
    );
    findings.extend(verification_advisory_findings(events));
    findings
}

fn completion_facts(
    gate: &ClosureGateResult,
    materialized_artifacts: &BTreeSet<String>,
) -> Vec<ClosureFact> {
    let mut facts = Vec::new();
    facts.push(ClosureFact {
        fact_type: "artifact_count".to_string(),
        source: "process_truth_replay".to_string(),
        message: materialized_artifacts.len().to_string(),
    });
    if gate.human_review_required {
        facts.push(ClosureFact {
            fact_type: "human_review_required".to_string(),
            source: "closure_gate".to_string(),
            message: "user-facing artifacts are present; final acceptance remains a runner/human review concern".to_string(),
        });
    }
    facts
}

fn extend_findings(
    findings: &mut Vec<ClosureFinding>,
    category: &str,
    source: &str,
    severity: &str,
    messages: &[String],
) {
    for message in messages {
        findings.push(ClosureFinding {
            category: category.to_string(),
            source: source.to_string(),
            message: message.clone(),
            artifact_path: artifact_path_from_message(message),
            severity: severity.to_string(),
        });
    }
}

fn audit_gaps(events: &[ProcessEvent], artifact_refs: &[String]) -> Vec<String> {
    let materialized_artifacts = materialized_artifact_refs(events);
    let audited = completed_local_artifact_audits(events);
    artifact_refs
        .iter()
        .filter(|path| materialized_artifacts.contains(&path.replace('\\', "/")))
        .filter(|path| requires_semantic_audit(path))
        .filter(|path| !audited.contains(&path.replace('\\', "/")))
        .map(|path| {
            format!(
                "artifact lacks completed local mechanical artifact.audit_quality receipt: {path}"
            )
        })
        .collect()
}

fn model_audit_gaps(events: &[ProcessEvent], artifact_refs: &[String]) -> Vec<String> {
    let materialized_artifacts = materialized_artifact_refs(events);
    let audited = latest_model_artifact_audits(events);
    artifact_refs
        .iter()
        .filter(|path| materialized_artifacts.contains(&path.replace('\\', "/")))
        .filter(|path| requires_semantic_audit(path))
        .filter_map(|path| {
            let normalized = path.replace('\\', "/");
            let Some(audit) = audited.get(&normalized) else {
                return Some(format!(
                    "artifact lacks successful model-backed artifact audit receipt: {path}"
                ));
            };
            let status = audit.get("status").and_then(Value::as_str).unwrap_or("");
            if status != "success" {
                return Some(format!(
                    "artifact model-backed audit did not succeed: {path}:{status}"
                ));
            }
            if model_audit_issue_strings(audit)
                .iter()
                .any(|issue| is_hard_artifact_issue(issue))
            {
                return None;
            }
            let quality_pass = payload_value(audit, "quality_pass")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let human_acceptance_pass = payload_value(audit, "human_acceptance_pass")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let blocking_issue_count = payload_value(audit, "blocking_issue_count")
                .and_then(Value::as_u64)
                .unwrap_or_else(|| {
                    payload_value(audit, "audit_output")
                        .and_then(|output| output.get("blocking_issues"))
                        .and_then(Value::as_array)
                        .map(|items| items.len() as u64)
                        .unwrap_or(0)
                });
            if !quality_pass || !human_acceptance_pass || blocking_issue_count > 0 {
                return Some(format!(
                    "artifact has unresolved model audit findings: {path}:quality_pass={quality_pass}:human_acceptance_pass={human_acceptance_pass}:blocking_issues={blocking_issue_count}"
                ));
            }
            None
        })
        .collect()
}

fn typed_artifact_verifier_gaps(
    events: &[ProcessEvent],
    materialized_artifacts: &BTreeSet<String>,
) -> Vec<String> {
    let typed_verified = typed_verified_artifacts(events);
    materialized_artifacts
        .iter()
        .filter(|path| {
            matches!(
                expected_verifier_for_path(path),
                "zip_verifier"
                    | "checksum_verifier"
                    | "perf_json_schema_verifier"
                    | "package_manifest_verifier"
            )
        })
        .filter(|path| !typed_verified.contains(&path.replace('\\', "/")))
        .map(|path| {
            format!(
                "artifact lacks typed verifier evidence: {} requires {}",
                path,
                expected_verifier_for_path(path)
            )
        })
        .collect()
}

fn missing_artifacts(guard: &WorkspaceGuard, artifact_refs: &[String]) -> io::Result<Vec<String>> {
    let mut missing = Vec::new();
    for artifact_path in artifact_refs {
        let trimmed = artifact_path.trim();
        if trimmed.is_empty() || is_internal_ref(trimmed) {
            missing.push(artifact_path.clone());
            continue;
        }
        let path = match guard.resolve_workspace_path(trimmed) {
            Ok(path) => path,
            Err(err) => {
                missing.push(format!("{trimmed}:{err}"));
                continue;
            }
        };
        if !path.exists()
            || (path.is_file() && std::fs::File::open(&path).is_err())
            || (path.is_dir() && std::fs::read_dir(&path).is_err())
        {
            missing.push(trimmed.to_string());
        }
    }
    Ok(missing)
}

fn is_internal_ref(value: &str) -> bool {
    let lower = value.trim().to_ascii_lowercase();
    lower.starts_with("blob://")
        || lower.starts_with("dataset://")
        || lower.starts_with("artifact://")
        || lower.starts_with("artifact_ref://")
        || lower.starts_with("source_set://")
        || lower.starts_with("raw_document_set://")
}

fn artifact_scope_set(artifact_refs: &[String]) -> BTreeSet<String> {
    artifact_refs
        .iter()
        .map(|item| item.trim().replace('\\', "/"))
        .filter(|item| !item.is_empty())
        .collect()
}

fn typed_verified_artifacts(events: &[ProcessEvent]) -> BTreeSet<String> {
    let mut refs = BTreeSet::new();
    for event in events {
        for payload in event_payloads(event) {
            if payload.get("status").and_then(Value::as_str) != Some("success") {
                continue;
            }
            match payload.get("capability_id").and_then(Value::as_str) {
                Some("artifact.verify_typed") => {
                    if let Some(path) = payload_value(payload, "artifact_path")
                        .or_else(|| payload_value(payload, "archive_path"))
                        .and_then(Value::as_str)
                    {
                        refs.insert(path.replace('\\', "/"));
                    }
                }
                _ => {}
            }
        }
    }
    refs
}

#[derive(Clone, Debug)]
struct VerificationFailure {
    capability_id: String,
    artifact_path: String,
    status: String,
    issues: Vec<String>,
}

fn hard_failed_verifications(events: &[ProcessEvent]) -> Vec<String> {
    verification_failures(events)
        .into_iter()
        .filter(is_hard_verification_failure)
        .map(|failure| {
            format!(
                "{}:{}:{}:{}",
                failure.capability_id,
                failure.artifact_path,
                failure.status,
                failure.issues.join("|")
            )
        })
        .collect()
}

fn verification_advisory_findings(events: &[ProcessEvent]) -> Vec<ClosureFinding> {
    verification_failures(events)
        .into_iter()
        .filter(|failure| !is_hard_verification_failure(failure))
        .map(|failure| ClosureFinding {
            category: "verification".to_string(),
            source: failure.capability_id.clone(),
            message: format!(
                "{}:{}:{}",
                failure.capability_id,
                failure.artifact_path,
                if failure.issues.is_empty() {
                    failure.status
                } else {
                    format!("{}:{}", failure.status, failure.issues.join("|"))
                }
            ),
            artifact_path: Some(failure.artifact_path),
            severity: "advisory".to_string(),
        })
        .collect()
}

fn hard_model_audit_failures(events: &[ProcessEvent]) -> Vec<String> {
    latest_model_artifact_audits(events)
        .into_iter()
        .flat_map(|(path, audit)| {
            let issues = model_audit_issue_strings(&audit);
            issues
                .into_iter()
                .filter(|issue| is_hard_artifact_issue(issue))
                .map(move |issue| format!("model.audit_artifact_quality:{path}:hard:{issue}"))
        })
        .collect()
}

fn model_audit_issue_strings(audit: &Value) -> Vec<String> {
    let mut issues = Vec::new();
    for key in [
        "blocking_issues",
        "factual_risks",
        "coverage_risks",
        "deliverability_risks",
        "findings",
        "schema_errors",
    ] {
        if let Some(items) = payload_value(audit, key)
            .or_else(|| payload_value(audit, "audit_output").and_then(|output| output.get(key)))
            .and_then(Value::as_array)
        {
            for item in items {
                match item {
                    Value::String(text) => issues.push(text.clone()),
                    Value::Object(_) => issues.push(item.to_string()),
                    _ => {}
                }
            }
        }
    }
    issues
}

fn verification_failures(events: &[ProcessEvent]) -> Vec<VerificationFailure> {
    let mut latest = std::collections::BTreeMap::<String, VerificationFailure>::new();
    for event in events {
        for payload in event_payloads(event) {
            let capability_id = payload.get("capability_id").and_then(Value::as_str);
            if !matches!(
                capability_id,
                Some(
                    "os.verify_artifact"
                        | "artifact.verify_coverage"
                        | "artifact.source_coverage_verify"
                        | "artifact.verify_typed"
                        | "artifact.audit_quality"
                )
            ) {
                continue;
            }
            let status = payload.get("status").and_then(Value::as_str).unwrap_or("");
            let path = payload_value(payload, "artifact_path")
                .and_then(Value::as_str)
                .unwrap_or("<unknown_artifact>")
                .replace('\\', "/");
            let issues = if capability_id == Some("artifact.audit_quality") {
                payload_value(payload, "hard_risk_issues")
            } else {
                payload_value(payload, "blocking_issues")
            }
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
            latest.insert(
                format!("{}:{path}", capability_id.unwrap_or("verify")),
                VerificationFailure {
                    capability_id: capability_id.unwrap_or("verify").to_string(),
                    artifact_path: path,
                    status: status.to_string(),
                    issues,
                },
            );
        }
    }
    latest
        .into_values()
        .filter(|failure| failure.status == "failed" || failure.status == "blocked")
        .collect()
}

fn is_hard_verification_failure(failure: &VerificationFailure) -> bool {
    if failure.capability_id == "os.verify_artifact" {
        return true;
    }
    failure
        .issues
        .iter()
        .any(|issue| is_hard_artifact_issue(issue))
}

fn is_hard_artifact_issue(issue: &str) -> bool {
    let lower = issue.to_ascii_lowercase();
    lower.contains("does not exist")
        || lower.contains("unreadable")
        || lower.contains("internal blob")
        || lower.contains("internal source ref")
        || lower.contains("blob/dataset")
        || lower.contains("dataset refs")
        || lower.contains("json/schema wrapper")
        || lower.contains("structured model")
}

fn pending_approvals(events: &[ProcessEvent]) -> Vec<String> {
    let preview_ids = events
        .iter()
        .filter(|event| event.event_type == "preview_tx_created")
        .filter_map(|event| event.data.get("preview_id").and_then(Value::as_str))
        .collect::<BTreeSet<_>>();
    let approved_preview_ids = events
        .iter()
        .filter(|event| event.event_type == "approval_token_issued")
        .filter_map(|event| event.data.get("preview_id").and_then(Value::as_str))
        .collect::<BTreeSet<_>>();
    let closed_preview_ids = events
        .iter()
        .filter(|event| event.event_type == "preview_tx_closed")
        .filter_map(|event| event.data.get("preview_id").and_then(Value::as_str))
        .collect::<BTreeSet<_>>();
    preview_ids
        .difference(&approved_preview_ids)
        .filter(|preview_id| !closed_preview_ids.contains(**preview_id))
        .map(|item| item.to_string())
        .collect()
}

fn pending_mutations(events: &[ProcessEvent]) -> Vec<String> {
    let mut pending = events
        .iter()
        .filter(|event| event.event_type == "capability_blocked")
        .filter(|event| {
            event
                .data
                .get("data")
                .and_then(|data| data.get("mutation_policy_blocked"))
                .and_then(Value::as_bool)
                .unwrap_or(false)
        })
        .filter_map(|event| event.data.get("capability_id").and_then(Value::as_str))
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    let consumed_tokens = events
        .iter()
        .filter(|event| {
            matches!(
                event.event_type.as_str(),
                "approval_token_consumed" | "approval_token_used"
            )
        })
        .filter_map(|event| event.data.get("approval_token_id").and_then(Value::as_str))
        .collect::<BTreeSet<_>>();
    let closed_txs = events
        .iter()
        .filter(|event| event.event_type == "preview_tx_closed")
        .filter_map(|event| event.data.get("tx_id").and_then(Value::as_str))
        .collect::<BTreeSet<_>>();
    for event in events
        .iter()
        .filter(|event| event.event_type == "approval_token_issued")
    {
        let token_id = event
            .data
            .get("approval_token_id")
            .and_then(Value::as_str)
            .unwrap_or("");
        let tx_id = event
            .data
            .get("tx_id")
            .and_then(Value::as_str)
            .unwrap_or("");
        if token_id.is_empty() {
            continue;
        }
        if !consumed_tokens.contains(token_id) || (!tx_id.is_empty() && !closed_txs.contains(tx_id))
        {
            pending.push(format!(
                "approved preview tx not consumed or closed: token={token_id} tx={tx_id}"
            ));
        }
    }
    pending
}

fn unresolved_required_failures(events: &[ProcessEvent]) -> Vec<String> {
    let last_success_event_id = events
        .iter()
        .filter(|event| event.event_type == "model_call_completed")
        .map(|event| event.event_id)
        .max()
        .unwrap_or(0);
    let mut failures = Vec::new();
    for event in events {
        if event.event_type != "model_call_failed" {
            continue;
        }
        if event.event_id < last_success_event_id {
            continue;
        }
        let required = event
            .data
            .get("required")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        if required {
            let capability = event
                .data
                .get("capability_id")
                .and_then(Value::as_str)
                .unwrap_or("model");
            let error = event
                .data
                .get("error")
                .and_then(|err| err.get("error_code"))
                .and_then(Value::as_str)
                .unwrap_or("model_call_failed");
            failures.push(format!("{capability}:{error}"));
        }
    }
    failures
}

fn coverage_gaps(events: &[ProcessEvent], artifact_refs: &[String]) -> Vec<String> {
    let materialized_artifacts = materialized_artifact_refs(events);
    let candidate_artifacts = if materialized_artifacts.is_empty() {
        Vec::new()
    } else {
        artifact_refs
            .iter()
            .filter(|path| materialized_artifacts.contains(&path.replace('\\', "/")))
            .filter(|path| requires_coverage_verification(path))
            .cloned()
            .collect::<Vec<_>>()
    };
    if candidate_artifacts.is_empty() {
        return Vec::new();
    }
    let mut generated_from_sources = false;
    let mut verified = BTreeSet::new();
    for event in events {
        for payload in event_payloads(event) {
            if payload_value(payload, "source_set_ref").is_some()
                || payload_value(payload, "dataset_ref").is_some()
                || payload_value(payload, "raw_document_set_ref").is_some()
            {
                generated_from_sources = true;
            }
            let capability_id = payload.get("capability_id").and_then(Value::as_str);
            if matches!(
                capability_id,
                Some(
                    "os.verify_artifact"
                        | "artifact.verify_coverage"
                        | "artifact.source_coverage_verify"
                        | "artifact.audit_quality"
                )
            ) && payload.get("status").and_then(Value::as_str) == Some("success")
            {
                if let Some(path) = payload_value(payload, "artifact_path").and_then(Value::as_str)
                {
                    verified.insert(path.replace('\\', "/"));
                }
            }
        }
    }
    if !generated_from_sources {
        return Vec::new();
    }
    candidate_artifacts
        .iter()
        .filter(|path| !path.starts_with("blob://"))
        .filter(|path| !verified.contains(&path.replace('\\', "/")))
        .map(|path| format!("artifact lacks successful runtime verification: {path}"))
        .collect()
}

fn materialized_artifact_refs(events: &[ProcessEvent]) -> BTreeSet<String> {
    let mut refs = BTreeSet::new();
    for event in events {
        for payload in event_payloads(event) {
            let capability_id = payload.get("capability_id").and_then(Value::as_str);
            let materializes = matches!(
                capability_id,
                Some(
                    "os.write_file"
                        | "os.write_artifact"
                        | "os.zip"
                        | "dataset.export_csv"
                        | "dataset.export_markdown"
                        | "artifact.copy_source_set"
                        | "package.build_zip"
                        | "office.docx.create"
                        | "office.docx.rewrite_save_as"
                        | "office.docx.rewrite_in_place"
                        | "workspace.tree_index"
                        | "workspace.perf_inventory"
                )
            );
            if !materializes {
                continue;
            }
            if let Some(path) = payload_value(payload, "artifact_path")
                .or_else(|| payload_value(payload, "archive_path"))
                .and_then(Value::as_str)
            {
                refs.insert(path.replace('\\', "/"));
            }
            if let Some(items) = payload_value(payload, "artifacts").and_then(Value::as_array) {
                for item in items {
                    if let Some(path) = item.as_str() {
                        refs.insert(path.replace('\\', "/"));
                    }
                }
            }
        }
    }
    refs
}

fn completed_local_artifact_audits(events: &[ProcessEvent]) -> BTreeSet<String> {
    let mut refs = BTreeSet::new();
    for event in events {
        for payload in event_payloads(event) {
            if payload.get("capability_id").and_then(Value::as_str)
                != Some("artifact.audit_quality")
            {
                continue;
            }
            let status = payload.get("status").and_then(Value::as_str).unwrap_or("");
            if !matches!(status, "success" | "failed") {
                continue;
            }
            let completed = payload_value(payload, "local_mechanical_audit_completed")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            if !completed {
                continue;
            }
            if let Some(path) = payload_value(payload, "artifact_path").and_then(Value::as_str) {
                refs.insert(path.replace('\\', "/"));
            }
        }
    }
    refs
}

fn latest_model_artifact_audits(
    events: &[ProcessEvent],
) -> std::collections::BTreeMap<String, Value> {
    let mut refs = std::collections::BTreeMap::new();
    for event in events {
        if event.event_type != "artifact_model_audit_receipt" {
            continue;
        }
        if let Some(path) = payload_value(&event.data, "artifact_path").and_then(Value::as_str) {
            refs.insert(path.replace('\\', "/"), event.data.clone());
        }
    }
    refs
}

fn artifact_role_evidence(materialized_artifacts: &BTreeSet<String>) -> Vec<ArtifactRoleEvidence> {
    materialized_artifacts
        .iter()
        .map(|path| ArtifactRoleEvidence {
            artifact_path: path.clone(),
            role: artifact_role_for_path(path).to_string(),
            expected_verifier: expected_verifier_for_path(path).to_string(),
        })
        .collect()
}

fn artifact_role_for_path(path: &str) -> &'static str {
    let lower = path.replace('\\', "/").to_ascii_lowercase();
    if lower.starts_with("previews/")
        || lower.contains("/previews/")
        || lower.starts_with("audit/")
        || lower.contains("/audit/")
        || lower.contains("artifact_quality_review")
    {
        return "audit_artifact";
    }
    if lower.starts_with("tmp/")
        || lower.starts_with("temp/")
        || lower.contains("/tmp/")
        || lower.contains("/temp/")
        || lower.ends_with(".preview.md")
    {
        return "temporary_artifact";
    }
    if lower.ends_with("perf_notes.json")
        || lower.ends_with("sha256sums.txt")
        || lower.ends_with("pack_manifest.md")
        || lower.ends_with("command_receipts.json")
        || lower.ends_with("model_call_ledger.json")
    {
        return "supporting_artifact";
    }
    "required_user_artifact"
}

fn expected_verifier_for_path(path: &str) -> &'static str {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".zip") {
        "zip_verifier"
    } else if lower.ends_with("sha256sums.txt") {
        "checksum_verifier"
    } else if lower.ends_with("perf_notes.json") {
        "perf_json_schema_verifier"
    } else if lower.ends_with("pack_manifest.md") {
        "package_manifest_verifier"
    } else if lower.ends_with(".docx") {
        "openxml_validation_plus_quality_audit"
    } else if requires_semantic_audit(path) {
        "artifact_quality_and_model_audit"
    } else {
        "os_verify_artifact"
    }
}

fn requires_semantic_audit(path: &str) -> bool {
    if artifact_role_for_path(path) != "required_user_artifact" {
        return false;
    }
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".md")
        || lower.ends_with(".txt")
        || lower.ends_with(".csv")
        || lower.ends_with(".docx")
}

fn requires_coverage_verification(path: &str) -> bool {
    if artifact_role_for_path(path) != "required_user_artifact" {
        return false;
    }
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".md")
        || lower.ends_with(".txt")
        || lower.ends_with(".csv")
        || lower.ends_with(".docx")
}

fn event_payloads(event: &ProcessEvent) -> Vec<&Value> {
    if event.data.get("capability_id").is_some() {
        return vec![&event.data];
    }
    let mut values = Vec::new();
    if let Some(receipt) = event.data.get("receipt") {
        values.push(receipt);
    }
    values
}

fn payload_value<'a>(payload: &'a Value, key: &str) -> Option<&'a Value> {
    payload
        .get(key)
        .or_else(|| payload.get("data").and_then(|data| data.get(key)))
}

fn artifact_path_from_message(message: &str) -> Option<String> {
    let parts = message.split(':').collect::<Vec<_>>();
    if parts.len() >= 2 {
        let candidate = parts[1].trim();
        if !candidate.is_empty()
            && candidate != "<unknown_artifact>"
            && (candidate.contains('/')
                || candidate.contains('.')
                || candidate.starts_with("blob://"))
        {
            return Some(candidate.to_string());
        }
    }
    None
}
