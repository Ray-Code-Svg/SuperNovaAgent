use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::Path;

use serde_json::{json, Value};

use crate::data_runtime::{DataSet, SourceSet};
use crate::{
    json_err, safe_blob_name, to_json_value, CapabilityReceipt, CapabilityToken, OfficeRuntime,
    ProcessTruthStore, WorkspaceGuard,
};

#[derive(Clone, Debug)]
pub struct ArtifactRuntime {
    guard: WorkspaceGuard,
    truth: ProcessTruthStore,
    token: CapabilityToken,
    emit_events: bool,
}

#[derive(Clone, Debug)]
struct CoverageEvidence {
    artifact_kind: &'static str,
    evidence_relation: &'static str,
    source_pass: bool,
    dataset_pass: bool,
}

#[derive(Clone, Debug)]
struct CoverageContractEvidence {
    relation: String,
    pass: bool,
    missing_sources: Vec<String>,
    unexpected_dataset_sources: Vec<String>,
    observed_ratio: f64,
}

impl ArtifactRuntime {
    pub fn new(guard: WorkspaceGuard, truth: ProcessTruthStore, token: CapabilityToken) -> Self {
        Self {
            guard,
            truth,
            token,
            emit_events: true,
        }
    }

    pub fn without_process_truth_events(mut self) -> Self {
        self.emit_events = false;
        self
    }

    pub fn verify_coverage(
        &self,
        artifact_path: &str,
        source_set_ref: Option<&str>,
        dataset_ref: Option<&str>,
    ) -> io::Result<CapabilityReceipt> {
        self.verify_coverage_with_contract(artifact_path, source_set_ref, dataset_ref, None)
    }

    pub fn verify_coverage_with_contract(
        &self,
        artifact_path: &str,
        source_set_ref: Option<&str>,
        dataset_ref: Option<&str>,
        coverage_contract: Option<&Value>,
    ) -> io::Result<CapabilityReceipt> {
        if let Some(receipt) = self.ensure_capability("artifact.verify_coverage") {
            return Ok(receipt);
        }
        let artifact = self
            .guard
            .resolve_workspace_path(artifact_path)
            .map_err(|err| io::Error::new(io::ErrorKind::PermissionDenied, err))?;
        if !artifact.exists() {
            return Ok(self.receipt(
                "artifact.verify_coverage",
                "failed",
                json!({"artifact_path": artifact_path, "reason": "artifact does not exist"}),
            )?);
        }
        let text = if artifact.is_file() {
            fs::read_to_string(&artifact).unwrap_or_default()
        } else {
            String::new()
        };
        let source_set = source_set_ref
            .map(|reference| self.read_source_set(reference))
            .transpose()?;
        let dataset = dataset_ref
            .map(|reference| self.read_dataset(reference))
            .transpose()?;
        let source_count = source_set.as_ref().map(|item| item.file_count).unwrap_or(0);
        let covered_sources = source_set
            .as_ref()
            .map(|item| {
                item.files
                    .iter()
                    .filter(|file| text.contains(&file.path))
                    .count()
            })
            .unwrap_or(0);
        let dataset_rows = dataset.as_ref().map(|item| item.row_count).unwrap_or(0);
        let dataset_source_rows = dataset
            .as_ref()
            .map(|item| {
                item.records
                    .iter()
                    .filter(|record| {
                        record
                            .get("source_path")
                            .and_then(Value::as_str)
                            .is_some_and(|path| !path.trim().is_empty())
                    })
                    .count()
            })
            .unwrap_or(0);
        let source_coverage_ratio = if source_count == 0 {
            1.0
        } else {
            covered_sources as f64 / source_count as f64
        };
        let dataset_source_ratio = if dataset_rows == 0 {
            1.0
        } else {
            dataset_source_rows as f64 / dataset_rows as f64
        };
        let evidence = coverage_evidence(
            artifact_path,
            &text,
            source_set_ref,
            source_count,
            covered_sources,
            dataset_ref,
            dataset_rows,
            dataset_source_rows,
        );
        let contract_evidence = evaluate_coverage_contract(
            coverage_contract,
            &text,
            source_set.as_ref(),
            dataset.as_ref(),
        );
        let pass = artifact.exists()
            && evidence.source_pass
            && evidence.dataset_pass
            && contract_evidence.pass;
        let review_ref = self.truth.write_blob(
            &format!(
                "artifact_reviews/{}_coverage.json",
                safe_blob_name(artifact_path)
            ),
            serde_json::to_string_pretty(&json!({
                "artifact_path": artifact_path,
                "source_set_ref": source_set_ref,
                "dataset_ref": dataset_ref,
                "source_count": source_count,
                "covered_sources": covered_sources,
                "source_coverage_ratio": source_coverage_ratio,
                "dataset_rows": dataset_rows,
                "dataset_source_rows": dataset_source_rows,
                "dataset_source_ratio": dataset_source_ratio,
                "artifact_kind": evidence.artifact_kind,
                "evidence_relation": evidence.evidence_relation,
                "coverage_contract": coverage_contract.cloned().unwrap_or(Value::Null),
                "contract_relation": contract_evidence.relation,
                "contract_pass": contract_evidence.pass,
                "missing_sources": contract_evidence.missing_sources,
                "unexpected_dataset_sources": contract_evidence.unexpected_dataset_sources,
                "contract_observed_ratio": contract_evidence.observed_ratio,
                "source_pass": evidence.source_pass,
                "dataset_pass": evidence.dataset_pass,
                "pass": pass,
            }))
            .map_err(json_err)?
            .as_bytes(),
        )?;
        self.receipt(
            "artifact.verify_coverage",
            if pass { "success" } else { "failed" },
            json!({
                "artifact_path": artifact_path.replace('\\', "/"),
                "source_set_ref": source_set_ref,
                "dataset_ref": dataset_ref,
                "source_count": source_count,
                "covered_sources": covered_sources,
                "source_coverage_ratio": source_coverage_ratio,
                "dataset_rows": dataset_rows,
                "dataset_source_rows": dataset_source_rows,
                "dataset_source_ratio": dataset_source_ratio,
                "artifact_kind": evidence.artifact_kind,
                "evidence_relation": evidence.evidence_relation,
                "coverage_contract": coverage_contract.cloned().unwrap_or(Value::Null),
                "contract_relation": contract_evidence.relation,
                "contract_pass": contract_evidence.pass,
                "missing_sources": contract_evidence.missing_sources,
                "unexpected_dataset_sources": contract_evidence.unexpected_dataset_sources,
                "contract_observed_ratio": contract_evidence.observed_ratio,
                "source_pass": evidence.source_pass,
                "dataset_pass": evidence.dataset_pass,
                "review_ref": review_ref,
            }),
        )
    }

    pub fn verify_typed_artifact(&self, artifact_path: &str) -> io::Result<CapabilityReceipt> {
        if let Some(receipt) = self.ensure_capability("artifact.verify_typed") {
            return Ok(receipt);
        }
        let artifact = self
            .guard
            .resolve_workspace_path(artifact_path)
            .map_err(|err| io::Error::new(io::ErrorKind::PermissionDenied, err))?;
        let verifier = expected_typed_verifier_for_path(artifact_path);
        let mut blocking_issues = Vec::new();
        let mut evidence = json!({});
        if !artifact.exists() || !artifact.is_file() {
            blocking_issues.push("artifact does not exist or is not a file".to_string());
        } else {
            match verifier {
                "zip_verifier" => match verify_store_zip_file(&artifact) {
                    Ok(entry_count) => {
                        evidence = json!({"entry_count": entry_count});
                        if entry_count == 0 {
                            blocking_issues.push("zip archive has no entries".to_string());
                        }
                    }
                    Err(err) => blocking_issues.push(format!("zip verifier failed: {err}")),
                },
                "checksum_verifier" => match self.verify_sha256sum_ledger(&artifact) {
                    Ok(value) => evidence = value,
                    Err(err) => blocking_issues.push(format!("checksum verifier failed: {err}")),
                },
                "package_manifest_verifier" => match verify_package_manifest(&artifact) {
                    Ok(value) => evidence = value,
                    Err(err) => {
                        blocking_issues.push(format!("package manifest verifier failed: {err}"))
                    }
                },
                "perf_json_schema_verifier" => match verify_perf_json(&artifact) {
                    Ok(value) => evidence = value,
                    Err(err) => blocking_issues.push(format!("perf JSON verifier failed: {err}")),
                },
                _ => {
                    evidence = json!({
                        "typed_verifier_supported": false,
                        "unsupported_verifier": verifier,
                        "informational": true,
                    });
                }
            }
        }
        let pass = blocking_issues.is_empty();
        self.receipt(
            "artifact.verify_typed",
            if pass { "success" } else { "failed" },
            json!({
                "artifact_path": artifact_path.replace('\\', "/"),
                "expected_verifier": verifier,
                "blocking_issues": blocking_issues,
                "unsupported_verifier": verifier == "os_verify_artifact" || verifier == "artifact_quality_and_model_audit" || verifier == "openxml_validation_plus_quality_audit",
                "informational": pass && (verifier == "os_verify_artifact" || verifier == "artifact_quality_and_model_audit" || verifier == "openxml_validation_plus_quality_audit"),
                "typed_verifier_pass": pass,
                "evidence": evidence,
                "runtime_note": "typed artifact verifier checks file facts only; TaskAgent decides how to act on failures",
            }),
        )
    }

    pub fn source_set_coverage_verify(
        &self,
        source_set_ref: &str,
    ) -> io::Result<CapabilityReceipt> {
        if let Some(receipt) = self.ensure_capability("source_set.coverage_verify") {
            return Ok(receipt);
        }
        let source_set = self.read_source_set(source_set_ref)?;
        let mut extension_counts = std::collections::BTreeMap::<String, usize>::new();
        for file in &source_set.files {
            let ext = std::path::Path::new(&file.path)
                .extension()
                .and_then(|item| item.to_str())
                .map(|item| format!(".{}", item.to_ascii_lowercase()))
                .unwrap_or_else(|| "<none>".to_string());
            *extension_counts.entry(ext).or_insert(0) += 1;
        }
        let review_ref = self.truth.write_blob(
            &format!(
                "artifact_reviews/{}_source_set_coverage.json",
                safe_blob_name(source_set_ref)
            ),
            serde_json::to_string_pretty(&json!({
                "source_set_ref": source_set_ref,
                "root_path": source_set.root_path,
                "file_count": source_set.file_count,
                "total_bytes": source_set.total_bytes,
                "include_extensions": source_set.include_extensions,
                "include_globs": source_set.include_globs,
                "exclude_globs": source_set.exclude_globs,
                "extension_counts": extension_counts,
                "zero_result": source_set.file_count == 0,
            }))
            .map_err(json_err)?
            .as_bytes(),
        )?;
        self.receipt(
            "source_set.coverage_verify",
            "success",
            json!({
                "source_set_ref": source_set_ref,
                "root_path": source_set.root_path,
                "file_count": source_set.file_count,
                "total_bytes": source_set.total_bytes,
                "include_extensions": source_set.include_extensions,
                "include_globs": source_set.include_globs,
                "exclude_globs": source_set.exclude_globs,
                "extension_counts": extension_counts,
                "zero_result": source_set.file_count == 0,
                "review_ref": review_ref,
                "runtime_note": "coverage verifier reports source-set facts only; TaskAgent decides whether coverage is sufficient for the user goal",
            }),
        )
    }

    pub fn dataset_coverage_verify(&self, dataset_ref: &str) -> io::Result<CapabilityReceipt> {
        if let Some(receipt) = self.ensure_capability("dataset.coverage_verify") {
            return Ok(receipt);
        }
        let dataset = self.read_dataset(dataset_ref)?;
        let rows_with_source_path = dataset
            .records
            .iter()
            .filter(|record| {
                record
                    .get("source_path")
                    .and_then(Value::as_str)
                    .is_some_and(|path| !path.trim().is_empty())
            })
            .count();
        let source_path_ratio = if dataset.row_count == 0 {
            1.0
        } else {
            rows_with_source_path as f64 / dataset.row_count as f64
        };
        let review_ref = self.truth.write_blob(
            &format!(
                "artifact_reviews/{}_dataset_coverage.json",
                safe_blob_name(dataset_ref)
            ),
            serde_json::to_string_pretty(&json!({
                "dataset_ref": dataset_ref,
                "dataset_id": dataset.dataset_id,
                "derivation_type": dataset.derivation_type,
                "row_count": dataset.row_count,
                "schema": dataset.schema,
                "rows_with_source_path": rows_with_source_path,
                "source_path_ratio": source_path_ratio,
            }))
            .map_err(json_err)?
            .as_bytes(),
        )?;
        self.receipt(
            "dataset.coverage_verify",
            "success",
            json!({
                "dataset_ref": dataset_ref,
                "dataset_id": dataset.dataset_id,
                "derivation_type": dataset.derivation_type,
                "row_count": dataset.row_count,
                "schema": dataset.schema,
                "rows_with_source_path": rows_with_source_path,
                "source_path_ratio": source_path_ratio,
                "review_ref": review_ref,
                "runtime_note": "coverage verifier reports dataset source-path facts only; TaskAgent decides whether to revise, inspect, or close",
            }),
        )
    }

    pub fn audit_quality(
        &self,
        artifact_path: &str,
        minimum_chars: usize,
        require_source_refs: bool,
    ) -> io::Result<CapabilityReceipt> {
        if let Some(receipt) = self.ensure_capability("artifact.audit_quality") {
            return Ok(receipt);
        }
        let artifact = self
            .guard
            .resolve_workspace_path(artifact_path)
            .map_err(|err| io::Error::new(io::ErrorKind::PermissionDenied, err))?;
        let mut read_error = None;
        let text = match self.artifact_text_for_audit(artifact_path) {
            Ok(text) => text,
            Err(err) => {
                read_error = Some(err.to_string());
                String::new()
            }
        };
        let has_source_refs = text.contains("source")
            || text.contains("Source")
            || text.contains("来源")
            || text.contains("`")
            || text.contains(".docx")
            || text.contains(".md")
            || text.contains(".txt");
        let mut mechanical_findings = Vec::new();
        if !artifact.exists() {
            mechanical_findings.push("artifact does not exist".to_string());
        } else if !artifact.is_file() {
            mechanical_findings.push("artifact is not a readable file".to_string());
        }
        if let Some(error) = &read_error {
            mechanical_findings.push(format!("artifact unreadable: {error}"));
        }
        if text.trim().chars().count() < minimum_chars {
            mechanical_findings.push("artifact is shorter than minimum_chars".to_string());
        }
        if require_source_refs && !has_source_refs {
            mechanical_findings.push("artifact lacks source references".to_string());
        }
        if leaks_structured_model_envelope(&text) {
            mechanical_findings
                .push("artifact leaks structured model JSON/schema wrapper".to_string());
        }
        if contains_internal_source_ref_leak(&text) {
            mechanical_findings.push(
                "artifact exposes internal blob/dataset refs as user-facing sources".to_string(),
            );
        }
        if contains_repeated_long_blocks(&text) {
            mechanical_findings
                .push("artifact contains repeated long paragraphs or lines".to_string());
        }
        if artifact_path.to_ascii_lowercase().ends_with(".docx")
            && contains_docx_markdown_rendering_artifacts(&text)
        {
            mechanical_findings.push(
                "DOCX artifact contains Markdown rendering markers or unformatted list syntax"
                    .to_string(),
            );
        }
        if artifact_path
            .to_ascii_lowercase()
            .ends_with("sha256sums.txt")
            && !looks_like_sha256sums(&text)
        {
            mechanical_findings
                .push("SHA256SUMS.txt is not a valid SHA-256 checksum ledger".to_string());
        }
        let hard_risk_issues = mechanical_findings
            .iter()
            .filter(|issue| is_hard_audit_issue(issue))
            .cloned()
            .collect::<Vec<_>>();
        let advisory_issues = mechanical_findings
            .iter()
            .filter(|issue| !is_hard_audit_issue(issue))
            .cloned()
            .collect::<Vec<_>>();
        let hard_risk_pass = hard_risk_issues.is_empty();
        let mechanical_audit_pass = hard_risk_pass;
        let hard_risk_count = hard_risk_issues.len();
        let advisory_issue_count = advisory_issues.len();
        self.receipt(
            "artifact.audit_quality",
            if mechanical_audit_pass {
                "success"
            } else {
                "failed"
            },
            json!({
                "audit_layer": "local_mechanical",
                "semantic_quality_judgement": false,
                "local_mechanical_audit_completed": true,
                "artifact_path": artifact_path.replace('\\', "/"),
                "artifact_exists": artifact.exists(),
                "artifact_readable": read_error.is_none() && artifact.is_file(),
                "char_count": text.trim().chars().count(),
                "minimum_chars": minimum_chars,
                "require_source_refs": require_source_refs,
                "has_source_refs": has_source_refs,
                "mechanical_findings": mechanical_findings,
                "blocking_issues": hard_risk_issues.clone(),
                "hard_risk_issues": hard_risk_issues,
                "advisory_issues": advisory_issues,
                "hard_risk_pass": hard_risk_pass,
                "mechanical_audit_pass": mechanical_audit_pass,
                "hard_risk_count": hard_risk_count,
                "advisory_issue_count": advisory_issue_count,
                "runtime_decision_policy": "Local artifact.audit_quality is mechanical screening only. It does not judge human acceptance or semantic deliverability; model.audit_artifact_quality handles semantic review.",
            }),
        )
    }

    fn artifact_text_for_audit(&self, artifact_path: &str) -> io::Result<String> {
        let artifact = self
            .guard
            .resolve_workspace_path(artifact_path)
            .map_err(|err| io::Error::new(io::ErrorKind::PermissionDenied, err))?;
        if !artifact.is_file() {
            return Ok(String::new());
        }
        if artifact_path.to_ascii_lowercase().ends_with(".docx") {
            let receipt = OfficeRuntime::new(
                self.guard.clone(),
                self.truth.clone(),
                self.token.clone(),
                office_worker_project(),
            )
            .read_text(artifact_path)?;
            if receipt.status != "success" {
                return Ok(String::new());
            }
            let Some(content_ref) = receipt.data.get("content_ref").and_then(Value::as_str) else {
                return Ok(String::new());
            };
            let path = self.truth.resolve_blob_ref(content_ref)?;
            return fs::read_to_string(path);
        }
        fs::read_to_string(&artifact)
    }

    fn read_source_set(&self, source_set_ref: &str) -> io::Result<SourceSet> {
        let path = self.truth.resolve_blob_ref(source_set_ref)?;
        serde_json::from_str(&fs::read_to_string(path)?).map_err(json_err)
    }

    fn read_dataset(&self, dataset_ref: &str) -> io::Result<DataSet> {
        let path = self.truth.resolve_blob_ref(dataset_ref)?;
        serde_json::from_str(&fs::read_to_string(path)?).map_err(json_err)
    }

    fn verify_sha256sum_ledger(&self, ledger_path: &Path) -> io::Result<Value> {
        let text = fs::read_to_string(ledger_path)?;
        let mut checked = 0usize;
        let mut mismatches = Vec::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut parts = line.split_whitespace();
            let expected = parts.next().ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "checksum line missing hash")
            })?;
            let relative_path = parts.next().ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "checksum line missing path")
            })?;
            if expected.len() != 64 || !expected.chars().all(|ch| ch.is_ascii_hexdigit()) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "checksum hash is not a 64-character SHA-256 hex digest",
                ));
            }
            let source = self
                .guard
                .resolve_workspace_path(relative_path)
                .map_err(|err| io::Error::new(io::ErrorKind::PermissionDenied, err))?;
            if !source.is_file() {
                mismatches.push(json!({
                    "path": relative_path,
                    "reason": "referenced file missing",
                    "expected": expected,
                }));
                continue;
            }
            let actual = crate::package_runtime::sha256_file_hex(&source)?;
            if !actual.eq_ignore_ascii_case(expected) {
                mismatches.push(json!({
                    "path": relative_path,
                    "expected": expected,
                    "actual": actual,
                }));
            }
            checked += 1;
        }
        if checked == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "checksum ledger has no file entries",
            ));
        }
        if !mismatches.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("checksum mismatches: {}", mismatches.len()),
            ));
        }
        Ok(json!({"checked_entries": checked, "algorithm": "sha256"}))
    }

    fn ensure_capability(&self, capability_id: &str) -> Option<CapabilityReceipt> {
        if self
            .token
            .capabilities
            .iter()
            .any(|item| item == capability_id)
        {
            None
        } else {
            Some(
                self.receipt(
                    capability_id,
                    "blocked",
                    json!({"reason": format!("{capability_id} not granted")}),
                )
                .unwrap_or_else(|_| CapabilityReceipt {
                    capability_id: capability_id.to_string(),
                    job_id: self.token.job_id.clone(),
                    pid: self.token.pid.clone(),
                    status: "blocked".to_string(),
                    data: json!({"reason": "not granted"}),
                }),
            )
        }
    }

    fn receipt(
        &self,
        capability_id: &str,
        status: &str,
        data: Value,
    ) -> io::Result<CapabilityReceipt> {
        let receipt = CapabilityReceipt {
            capability_id: capability_id.to_string(),
            job_id: self.token.job_id.clone(),
            pid: self.token.pid.clone(),
            status: status.to_string(),
            data,
        };
        if self.emit_events {
            self.truth.append_event(
                Some(&self.token.pid),
                "capability_receipt",
                to_json_value(&receipt)?,
            )?;
            if capability_id.starts_with("artifact.") {
                self.truth.append_event(
                    Some(&self.token.pid),
                    "artifact_review_receipt",
                    to_json_value(&receipt)?,
                )?;
            }
        }
        Ok(receipt)
    }
}

fn is_hard_audit_issue(issue: &str) -> bool {
    let lower = issue.to_ascii_lowercase();
    lower.contains("does not exist")
        || lower.contains("unreadable")
        || lower.contains("internal blob")
        || lower.contains("internal source ref")
        || lower.contains("blob/dataset")
        || lower.contains("json/schema wrapper")
        || lower.contains("structured model")
}

fn office_worker_project() -> std::path::PathBuf {
    std::env::var("SUPERNOVA_OFFICE_WORKER_PROJECT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .join("office_worker")
                .join("SuperNova.OfficeWorker")
                .join("SuperNova.OfficeWorker.csproj")
        })
}

fn coverage_evidence(
    artifact_path: &str,
    text: &str,
    source_set_ref: Option<&str>,
    source_count: usize,
    covered_sources: usize,
    dataset_ref: Option<&str>,
    dataset_rows: usize,
    dataset_source_rows: usize,
) -> CoverageEvidence {
    let aggregate_metrics = is_aggregate_metrics_artifact(artifact_path, text);
    let source_pass = if source_set_ref.is_none() || source_count == 0 {
        true
    } else if covered_sources > 0 {
        true
    } else if aggregate_metrics {
        let mentions_source_set = source_set_ref.is_some_and(|reference| text.contains(reference));
        mentions_source_set || aggregate_metrics_tokens_present(text)
    } else {
        false
    };
    let dataset_pass = dataset_ref.is_none() || dataset_source_rows == dataset_rows;
    CoverageEvidence {
        artifact_kind: if aggregate_metrics {
            "aggregate_metrics"
        } else {
            "source_derived_artifact"
        },
        evidence_relation: if aggregate_metrics {
            "aggregate metrics cite source_set_ref or source-count fields instead of listing every source path"
        } else {
            "artifact text names at least one source path and dataset rows preserve source_path when dataset_ref is provided"
        },
        source_pass,
        dataset_pass,
    }
}

fn evaluate_coverage_contract(
    coverage_contract: Option<&Value>,
    text: &str,
    source_set: Option<&SourceSet>,
    dataset: Option<&DataSet>,
) -> CoverageContractEvidence {
    let relation = coverage_contract_relation(coverage_contract);
    match relation.as_str() {
        "row_per_source" => {
            let source_paths = source_paths(source_set);
            let dataset_sources = dataset_source_paths(dataset);
            let missing_sources = source_paths
                .iter()
                .filter(|path| !dataset_sources.contains(*path))
                .cloned()
                .collect::<Vec<_>>();
            let observed_ratio = if source_paths.is_empty() {
                1.0
            } else {
                (source_paths.len().saturating_sub(missing_sources.len())) as f64
                    / source_paths.len() as f64
            };
            CoverageContractEvidence {
                relation,
                pass: source_set.is_some() && dataset.is_some() && missing_sources.is_empty(),
                missing_sources,
                unexpected_dataset_sources: Vec::new(),
                observed_ratio,
            }
        }
        "mentions_all_sources" => {
            let source_paths = source_paths(source_set);
            let missing_sources = source_paths
                .iter()
                .filter(|path| !text.contains(path.as_str()))
                .cloned()
                .collect::<Vec<_>>();
            let observed_ratio = if source_paths.is_empty() {
                1.0
            } else {
                (source_paths.len().saturating_sub(missing_sources.len())) as f64
                    / source_paths.len() as f64
            };
            CoverageContractEvidence {
                relation,
                pass: source_set.is_some() && missing_sources.is_empty(),
                missing_sources,
                unexpected_dataset_sources: Vec::new(),
                observed_ratio,
            }
        }
        "aggregate_metrics" => CoverageContractEvidence {
            relation,
            pass: aggregate_metrics_tokens_present(text),
            missing_sources: Vec::new(),
            unexpected_dataset_sources: Vec::new(),
            observed_ratio: if aggregate_metrics_tokens_present(text) {
                1.0
            } else {
                0.0
            },
        },
        "source_set_subset" => {
            let source_paths = source_paths(source_set);
            let dataset_sources = dataset_source_paths(dataset);
            let unexpected_dataset_sources = dataset_sources
                .iter()
                .filter(|path| !source_paths.contains(*path))
                .cloned()
                .collect::<Vec<_>>();
            let observed_ratio = if dataset_sources.is_empty() {
                1.0
            } else {
                (dataset_sources
                    .len()
                    .saturating_sub(unexpected_dataset_sources.len())) as f64
                    / dataset_sources.len() as f64
            };
            CoverageContractEvidence {
                relation,
                pass: source_set.is_some()
                    && dataset.is_some()
                    && unexpected_dataset_sources.is_empty(),
                missing_sources: Vec::new(),
                unexpected_dataset_sources,
                observed_ratio,
            }
        }
        _ => CoverageContractEvidence {
            relation,
            pass: true,
            missing_sources: Vec::new(),
            unexpected_dataset_sources: Vec::new(),
            observed_ratio: 1.0,
        },
    }
}

fn coverage_contract_relation(coverage_contract: Option<&Value>) -> String {
    match coverage_contract {
        Some(Value::String(value)) => value.trim().to_ascii_lowercase(),
        Some(Value::Object(map)) => map
            .get("relation")
            .or_else(|| map.get("type"))
            .and_then(Value::as_str)
            .unwrap_or("source_mentions_any_or_aggregate")
            .trim()
            .to_ascii_lowercase(),
        _ => "source_mentions_any_or_aggregate".to_string(),
    }
}

fn source_paths(source_set: Option<&SourceSet>) -> BTreeSet<String> {
    source_set
        .map(|set| {
            set.files
                .iter()
                .map(|file| file.path.replace('\\', "/"))
                .collect::<BTreeSet<_>>()
        })
        .unwrap_or_default()
}

fn dataset_source_paths(dataset: Option<&DataSet>) -> BTreeSet<String> {
    dataset
        .map(|set| {
            set.records
                .iter()
                .filter_map(|record| record.get("source_path").and_then(Value::as_str))
                .filter(|path| !path.trim().is_empty())
                .map(|path| path.replace('\\', "/"))
                .collect::<BTreeSet<_>>()
        })
        .unwrap_or_default()
}

fn is_aggregate_metrics_artifact(artifact_path: &str, text: &str) -> bool {
    let path = artifact_path.to_ascii_lowercase();
    path.contains("perf")
        || (aggregate_metrics_tokens_present(text)
            && (text.contains("source_set_ref") || text.contains("inventory_ref")))
}

fn aggregate_metrics_tokens_present(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    (lower.contains("file_count") || lower.contains("total_files"))
        && (lower.contains("elapsed_ms")
            || lower.contains("duration_ms")
            || lower.contains("total_bytes")
            || lower.contains("bytes"))
}

fn leaks_structured_model_envelope(text: &str) -> bool {
    let trimmed = text.trim_start();
    (trimmed.starts_with('{') || trimmed.starts_with("```json"))
        && (text.contains("\"rewritten_text\"")
            || text.contains("\"content\"")
            || text.contains("\"schema\"")
            || text.contains("\"properties\""))
}

fn contains_internal_source_ref_leak(text: &str) -> bool {
    text.contains("blob://")
        || text.contains(".supernova_v2")
        || text.contains("datasets/")
        || text.contains("raw_tool_results/")
}

fn contains_repeated_long_blocks(text: &str) -> bool {
    let mut seen = std::collections::BTreeSet::<String>::new();
    for line in text.lines() {
        let normalized = line.split_whitespace().collect::<Vec<_>>().join("");
        if normalized.chars().count() < 40 {
            continue;
        }
        if !seen.insert(normalized) {
            return true;
        }
    }
    false
}

fn contains_docx_markdown_rendering_artifacts(text: &str) -> bool {
    text.lines().any(|line| {
        let trimmed = line.trim();
        trimmed == "---"
            || trimmed == "***"
            || trimmed == "___"
            || trimmed.starts_with("# ")
            || trimmed.starts_with("## ")
            || trimmed.starts_with("### ")
            || trimmed.starts_with("```")
            || trimmed.starts_with("- ")
            || trimmed.starts_with("* ")
            || trimmed.contains("**")
    })
}

fn looks_like_sha256sums(text: &str) -> bool {
    let mut checked = 0usize;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split_whitespace();
        let Some(hash) = parts.next() else {
            return false;
        };
        let Some(_path) = parts.next() else {
            return false;
        };
        if hash.len() != 64 || !hash.chars().all(|ch| ch.is_ascii_hexdigit()) {
            return false;
        }
        checked += 1;
    }
    checked > 0 && !text.to_ascii_lowercase().contains("fnv1a")
}

fn expected_typed_verifier_for_path(path: &str) -> &'static str {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".zip") {
        "zip_verifier"
    } else if lower.ends_with("sha256sums.txt") {
        "checksum_verifier"
    } else if lower.ends_with("pack_manifest.md") {
        "package_manifest_verifier"
    } else if lower.ends_with("perf_notes.json") {
        "perf_json_schema_verifier"
    } else {
        "generic_artifact_verifier"
    }
}

fn verify_store_zip_file(path: &Path) -> io::Result<usize> {
    let bytes = fs::read(path)?;
    let mut offset = 0usize;
    let mut count = 0usize;
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
                "only store-method zip entries are supported by this verifier",
            ));
        }
        let compressed_size = read_u32_le(&bytes, offset + 18)? as usize;
        let name_len = read_u16_le(&bytes, offset + 26)? as usize;
        let extra_len = read_u16_le(&bytes, offset + 28)? as usize;
        let data_start = offset + 30 + name_len + extra_len;
        let data_end = data_start + compressed_size;
        if data_end > bytes.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "zip entry exceeds archive bounds",
            ));
        }
        count += 1;
        offset = data_end;
    }
    if count == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "no local zip entries found",
        ));
    }
    Ok(count)
}

fn read_u16_le(bytes: &[u8], offset: usize) -> io::Result<u16> {
    if offset + 2 > bytes.len() {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "zip header truncated",
        ));
    }
    Ok(u16::from_le_bytes([bytes[offset], bytes[offset + 1]]))
}

fn read_u32_le(bytes: &[u8], offset: usize) -> io::Result<u32> {
    if offset + 4 > bytes.len() {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "zip header truncated",
        ));
    }
    Ok(u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ]))
}

fn verify_package_manifest(path: &Path) -> io::Result<Value> {
    let text = fs::read_to_string(path)?;
    let required = [
        "# PACK_MANIFEST",
        "source_file_count",
        "included_count",
        "checksum_algorithm",
        "## Included Files",
    ];
    let missing = required
        .iter()
        .filter(|marker| !text.contains(**marker))
        .map(|marker| marker.to_string())
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("manifest missing markers: {}", missing.join(", ")),
        ));
    }
    Ok(json!({
        "required_markers_present": required,
        "bytes": text.len(),
    }))
}

fn verify_perf_json(path: &Path) -> io::Result<Value> {
    let value: Value = serde_json::from_str(&fs::read_to_string(path)?).map_err(json_err)?;
    let object = value.as_object().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "PERF_NOTES is not a JSON object",
        )
    })?;
    let has_elapsed = object.get("elapsed_ms").and_then(Value::as_u64).is_some()
        || object.get("duration_ms").and_then(Value::as_u64).is_some();
    let has_count = [
        "entry_count",
        "included_count",
        "file_count",
        "total_files",
        "document_count",
    ]
    .iter()
    .any(|key| object.get(*key).and_then(Value::as_u64).is_some());
    if !has_elapsed || !has_count {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "PERF_NOTES must include elapsed/duration and count fields",
        ));
    }
    Ok(json!({
        "elapsed_field_present": has_elapsed,
        "count_field_present": has_count,
    }))
}
