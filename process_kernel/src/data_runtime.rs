use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Instant, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{
    copy_path_recursive, file_fingerprint, json_err, new_tx_id, remove_path_any, safe_blob_name,
    to_json_value, CapabilityReceipt, CapabilityToken, ProcessTruthStore, WorkspaceGuard,
    RUNTIME_DIR_NAME,
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SourceSetFile {
    pub path: String,
    pub extension: String,
    pub size_bytes: u64,
    pub modified_ms: u128,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SourceSet {
    pub source_set_id: String,
    pub root_path: String,
    pub include_extensions: Vec<String>,
    pub include_globs: Vec<String>,
    pub exclude_globs: Vec<String>,
    pub file_count: usize,
    pub total_bytes: u64,
    pub files: Vec<SourceSetFile>,
    pub excluded_count: usize,
    pub scan_diagnostics: SourceSetScanDiagnostics,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SourceSetScanDiagnostics {
    pub scanned_file_count: usize,
    pub excluded_by_explicit_exclude: usize,
    pub excluded_by_extension: usize,
    pub excluded_by_include_glob: usize,
    pub skipped_by_depth: usize,
    pub zero_match_reason: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct DataSet {
    pub dataset_id: String,
    pub schema: Vec<String>,
    pub row_count: usize,
    pub source_set_ref: Option<String>,
    pub derivation_type: String,
    pub records: Vec<Value>,
    pub coverage_report: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceMutationOp {
    pub source_path: String,
    pub destination_path: String,
    pub operation: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceOrganizePlan {
    pub plan_id: String,
    pub source_set_ref: String,
    pub destination_root: String,
    pub operations: Vec<WorkspaceMutationOp>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceRenamePlan {
    pub plan_id: String,
    pub operations: Vec<WorkspaceMutationOp>,
}

#[derive(Clone, Debug)]
pub struct DataRuntime {
    guard: WorkspaceGuard,
    truth: ProcessTruthStore,
    token: CapabilityToken,
    emit_events: bool,
}

impl DataRuntime {
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

    pub fn create_source_set(
        &self,
        root_path: &str,
        include_extensions: &[String],
        include_globs: &[String],
        exclude_globs: &[String],
        max_depth: usize,
    ) -> io::Result<CapabilityReceipt> {
        if let Some(receipt) = self.ensure_capability("source_set.create") {
            return Ok(receipt);
        }
        let root = self.resolve_path(root_path)?;
        if !root.exists() {
            return Ok(self.blocked_receipt("source_set.create", "source set root does not exist"));
        }
        let include_extensions = normalize_extensions(include_extensions);
        let mut files = Vec::new();
        let mut diagnostics = SourceSetScanDiagnostics::default();
        collect_source_files(
            self.guard.root(),
            &root,
            0,
            max_depth,
            &include_extensions,
            include_globs,
            exclude_globs,
            &mut files,
            &mut diagnostics,
        )?;
        diagnostics.zero_match_reason = zero_match_reason(
            files.len(),
            &diagnostics,
            !include_extensions.is_empty(),
            !include_globs.is_empty(),
        );
        files.sort_by(|left, right| left.path.cmp(&right.path));
        let total_bytes = files.iter().map(|item| item.size_bytes).sum::<u64>();
        let excluded_count = diagnostics.excluded_by_explicit_exclude
            + diagnostics.excluded_by_extension
            + diagnostics.excluded_by_include_glob;
        let source_set = SourceSet {
            source_set_id: format!("sourceset_{}", crate::now_ms()),
            root_path: root_path.replace('\\', "/"),
            include_extensions,
            include_globs: include_globs.to_vec(),
            exclude_globs: exclude_globs.to_vec(),
            file_count: files.len(),
            total_bytes,
            files,
            excluded_count,
            scan_diagnostics: diagnostics.clone(),
        };
        let source_set_ref = self.truth.write_blob(
            &format!(
                "source_sets/{}.json",
                safe_blob_name(&source_set.source_set_id)
            ),
            &serde_json::to_vec_pretty(&source_set).map_err(json_err)?,
        )?;
        let receipt = CapabilityReceipt {
            capability_id: "source_set.create".to_string(),
            job_id: self.token.job_id.clone(),
            pid: self.token.pid.clone(),
            status: "success".to_string(),
            data: json!({
                "source_set_ref": source_set_ref,
                "source_set_id": source_set.source_set_id,
                "file_count": source_set.file_count,
                "total_bytes": source_set.total_bytes,
                "excluded_count": source_set.excluded_count,
                "scan_diagnostics": diagnostics,
                "empty_result_actionability": source_set.scan_diagnostics.zero_match_reason.as_ref().map(|reason| {
                    format!("file_count=0 because {reason}; inspect root_path/include_extensions/include_globs/max_depth and retry if this was unexpected")
                }),
                "derivation_type": "metadata",
                "is_lossless": false,
                "source_of_truth": "workspace files",
            }),
        };
        self.emit_receipt(&receipt)?;
        Ok(receipt)
    }

    pub fn read_source_set_page(
        &self,
        source_set_ref: &str,
        offset: usize,
        limit: usize,
    ) -> io::Result<CapabilityReceipt> {
        if let Some(receipt) = self.ensure_capability("source_set.read_page") {
            return Ok(receipt);
        }
        let source_set = self.read_source_set(source_set_ref)?;
        let safe_limit = limit.clamp(1, 500);
        let rows = source_set
            .files
            .iter()
            .skip(offset)
            .take(safe_limit)
            .cloned()
            .collect::<Vec<_>>();
        let page_ref = self.truth.write_blob(
            &format!(
                "source_sets/{}_page_{}.json",
                source_set.source_set_id, offset
            ),
            &serde_json::to_vec_pretty(&rows).map_err(json_err)?,
        )?;
        let receipt = CapabilityReceipt {
            capability_id: "source_set.read_page".to_string(),
            job_id: self.token.job_id.clone(),
            pid: self.token.pid.clone(),
            status: "success".to_string(),
            data: json!({
                "source_set_ref": source_set_ref,
                "page_ref": page_ref,
                "offset": offset,
                "limit": safe_limit,
                "returned": rows.len(),
                "total": source_set.file_count,
            }),
        };
        self.emit_receipt(&receipt)?;
        Ok(receipt)
    }

    pub fn batch_hash(&self, source_set_ref: &str) -> io::Result<CapabilityReceipt> {
        if let Some(receipt) = self.ensure_capability("workspace.batch_hash") {
            return Ok(receipt);
        }
        let source_set = self.read_source_set(source_set_ref)?;
        let mut records = Vec::new();
        for file in &source_set.files {
            let path = self.resolve_path(&file.path)?;
            records.push(json!({
                "source_path": file.path,
                "size_bytes": file.size_bytes,
                "modified_ms": file.modified_ms,
                "fingerprint": file_fingerprint(&path)?,
                "fingerprint_algorithm": "fnv1a64",
            }));
        }
        let dataset = self.write_dataset(
            "batch_hash",
            Some(source_set_ref.to_string()),
            "metadata",
            records,
            json!({
                "source_count": source_set.file_count,
                "covered_count": source_set.file_count,
                "coverage_ratio": 1.0,
            }),
        )?;
        let receipt = CapabilityReceipt {
            capability_id: "workspace.batch_hash".to_string(),
            job_id: self.token.job_id.clone(),
            pid: self.token.pid.clone(),
            status: "success".to_string(),
            data: json!({
                "source_set_ref": source_set_ref,
                "dataset_ref": dataset.0,
                "row_count": dataset.1.row_count,
                "fingerprint_algorithm": "fnv1a64",
            }),
        };
        self.emit_receipt(&receipt)?;
        Ok(receipt)
    }

    pub fn find_duplicates(&self, source_set_ref: &str) -> io::Result<CapabilityReceipt> {
        if let Some(receipt) = self.ensure_capability("workspace.find_duplicates") {
            return Ok(receipt);
        }
        let source_set = self.read_source_set(source_set_ref)?;
        let mut groups: BTreeMap<(u64, String), Vec<&SourceSetFile>> = BTreeMap::new();
        for file in &source_set.files {
            let path = self.resolve_path(&file.path)?;
            groups
                .entry((file.size_bytes, file_fingerprint(&path)?))
                .or_default()
                .push(file);
        }
        let records = groups
            .into_iter()
            .filter_map(|((size_bytes, fingerprint), files)| {
                if files.len() < 2 {
                    return None;
                }
                let paths = files
                    .iter()
                    .map(|item| item.path.clone())
                    .collect::<Vec<_>>();
                let recommended_keep = paths.iter().min().cloned().unwrap_or_default();
                Some(json!({
                    "fingerprint": fingerprint,
                    "fingerprint_algorithm": "fnv1a64",
                    "size_bytes": size_bytes,
                    "duplicate_count": paths.len(),
                    "paths": paths,
                    "recommended_keep": recommended_keep,
                    "reason": "same size and same file fingerprint",
                }))
            })
            .collect::<Vec<_>>();
        let dataset = self.write_dataset(
            "duplicate_groups",
            Some(source_set_ref.to_string()),
            "metadata",
            records,
            json!({
                "source_count": source_set.file_count,
                "covered_count": source_set.file_count,
                "coverage_ratio": 1.0,
            }),
        )?;
        let receipt = CapabilityReceipt {
            capability_id: "workspace.find_duplicates".to_string(),
            job_id: self.token.job_id.clone(),
            pid: self.token.pid.clone(),
            status: "success".to_string(),
            data: json!({
                "source_set_ref": source_set_ref,
                "dataset_ref": dataset.0,
                "duplicate_group_count": dataset.1.row_count,
                "source_count": source_set.file_count,
            }),
        };
        self.emit_receipt(&receipt)?;
        Ok(receipt)
    }

    pub fn recent_changes(&self, source_set_ref: &str, days: u64) -> io::Result<CapabilityReceipt> {
        if let Some(receipt) = self.ensure_capability("workspace.recent_changes") {
            return Ok(receipt);
        }
        let source_set = self.read_source_set(source_set_ref)?;
        let now_ms = crate::now_ms();
        let cutoff_ms = now_ms.saturating_sub(u128::from(days) * 24 * 60 * 60 * 1000);
        let records = source_set
            .files
            .iter()
            .filter(|file| file.modified_ms >= cutoff_ms)
            .map(|file| {
                json!({
                    "source_path": file.path,
                    "size_bytes": file.size_bytes,
                    "modified_ms": file.modified_ms,
                    "age_days_max": days,
                    "purpose": purpose_from_path(&file.path),
                })
            })
            .collect::<Vec<_>>();
        let covered = records.len();
        let dataset = self.write_dataset(
            "recent_changes",
            Some(source_set_ref.to_string()),
            "metadata",
            records,
            json!({
                "source_count": source_set.file_count,
                "recent_count": covered,
                "coverage_ratio": 1.0,
                "days": days,
            }),
        )?;
        let receipt = CapabilityReceipt {
            capability_id: "workspace.recent_changes".to_string(),
            job_id: self.token.job_id.clone(),
            pid: self.token.pid.clone(),
            status: "success".to_string(),
            data: json!({
                "source_set_ref": source_set_ref,
                "dataset_ref": dataset.0,
                "recent_count": covered,
                "days": days,
            }),
        };
        self.emit_receipt(&receipt)?;
        Ok(receipt)
    }

    pub fn plan_organize(
        &self,
        source_set_ref: &str,
        destination_root: &str,
    ) -> io::Result<CapabilityReceipt> {
        if let Some(receipt) = self.ensure_capability("workspace.plan_organize") {
            return Ok(receipt);
        }
        let source_set = self.read_source_set(source_set_ref)?;
        let destination_root = normalize_rel(destination_root);
        let mut used = BTreeSet::new();
        let mut operations = Vec::new();
        for file in &source_set.files {
            let group = organize_group_for_path(&file.path);
            let ext = if file.extension.trim().is_empty() {
                "no_extension".to_string()
            } else {
                file.extension.trim_start_matches('.').to_string()
            };
            let file_name = Path::new(&file.path)
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("file");
            let mut destination = format!("{destination_root}/{group}/{ext}/{file_name}");
            if used.contains(&destination) {
                destination = disambiguate_destination(&destination, &mut used);
            }
            used.insert(destination.clone());
            operations.push(WorkspaceMutationOp {
                source_path: file.path.clone(),
                destination_path: destination,
                operation: "move".to_string(),
            });
        }
        let plan = WorkspaceOrganizePlan {
            plan_id: format!("organize_plan_{}", crate::now_ms()),
            source_set_ref: source_set_ref.to_string(),
            destination_root,
            operations,
        };
        let plan_ref = self.truth.write_blob(
            &format!("workspace_plans/{}.json", safe_blob_name(&plan.plan_id)),
            &serde_json::to_vec_pretty(&plan).map_err(json_err)?,
        )?;
        let preview_ref = self.truth.write_blob(
            &format!(
                "workspace_plans/{}_preview.md",
                safe_blob_name(&plan.plan_id)
            ),
            organize_plan_markdown(&plan).as_bytes(),
        )?;
        let receipt = CapabilityReceipt {
            capability_id: "workspace.plan_organize".to_string(),
            job_id: self.token.job_id.clone(),
            pid: self.token.pid.clone(),
            status: "success".to_string(),
            data: json!({
                "source_set_ref": source_set_ref,
                "organize_plan_ref": plan_ref,
                "preview_ref": preview_ref,
                "operation_count": plan.operations.len(),
                "destination_root": plan.destination_root,
                "target_paths": plan.operations.iter()
                    .flat_map(|op| [op.source_path.clone(), op.destination_path.clone()])
                    .collect::<Vec<_>>(),
                "proposed_actions": ["workspace.apply_organize_tx"],
                "requires_approval": true,
            }),
        };
        self.emit_receipt(&receipt)?;
        Ok(receipt)
    }

    pub fn apply_organize_tx(
        &self,
        organize_plan_ref: &str,
        _approval_id: Option<&str>,
    ) -> io::Result<CapabilityReceipt> {
        if let Some(receipt) = self.ensure_capability("workspace.apply_organize_tx") {
            return Ok(receipt);
        }
        let plan = self.read_organize_plan(organize_plan_ref)?;
        let tx_id = new_tx_id("workspace_organize");
        let mut moved = Vec::new();
        for op in &plan.operations {
            let source = self.resolve_path(&op.source_path)?;
            let destination = self.resolve_path(&op.destination_path)?;
            if !source.exists() {
                continue;
            }
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent)?;
            }
            if destination.exists() {
                remove_path_any(&destination)?;
            }
            match fs::rename(&source, &destination) {
                Ok(()) => {}
                Err(_) => {
                    copy_path_recursive(&source, &destination)?;
                    remove_path_any(&source)?;
                }
            }
            moved.push(op.clone());
        }
        let rollback_ref = self.truth.write_blob(
            &format!("tx/{}_rollback_plan.json", safe_blob_name(&tx_id)),
            &serde_json::to_vec_pretty(&json!({
                "tx_id": tx_id,
                "operation": "workspace.apply_organize_tx",
                "rollback_strategy": "move destination_path back to source_path in reverse order",
                "operations": moved,
            }))
            .map_err(json_err)?,
        )?;
        if self.emit_events {
            self.truth.append_event(
                Some(&self.token.pid),
                "tx_recorded",
                json!({
                    "tx_id": tx_id,
                    "operation": "workspace.apply_organize_tx",
                    "tx_ref": rollback_ref,
                    "rollback_ref": rollback_ref,
                }),
            )?;
        }
        let receipt = CapabilityReceipt {
            capability_id: "workspace.apply_organize_tx".to_string(),
            job_id: self.token.job_id.clone(),
            pid: self.token.pid.clone(),
            status: "success".to_string(),
            data: json!({
                "organize_plan_ref": organize_plan_ref,
                "tx_id": tx_id,
                "rollback_ref": rollback_ref,
                "moved_count": moved.len(),
                "destination_root": plan.destination_root,
            }),
        };
        self.emit_receipt(&receipt)?;
        Ok(receipt)
    }

    pub fn rename_batch_preview(&self, mappings: &Value) -> io::Result<CapabilityReceipt> {
        if let Some(receipt) = self.ensure_capability("workspace.rename_batch_preview") {
            return Ok(receipt);
        }
        let operations = parse_rename_mappings(mappings)?;
        if operations.is_empty() {
            return Ok(self.blocked_receipt(
                "workspace.rename_batch_preview",
                "rename mappings are empty",
            ));
        }
        for op in &operations {
            let _ = self.resolve_path(&op.source_path)?;
            let _ = self.resolve_path(&op.destination_path)?;
        }
        let plan = WorkspaceRenamePlan {
            plan_id: format!("rename_plan_{}", crate::now_ms()),
            operations,
        };
        let plan_ref = self.truth.write_blob(
            &format!("workspace_plans/{}.json", safe_blob_name(&plan.plan_id)),
            &serde_json::to_vec_pretty(&plan).map_err(json_err)?,
        )?;
        let preview_ref = self.truth.write_blob(
            &format!(
                "workspace_plans/{}_preview.md",
                safe_blob_name(&plan.plan_id)
            ),
            rename_plan_markdown(&plan).as_bytes(),
        )?;
        let receipt = CapabilityReceipt {
            capability_id: "workspace.rename_batch_preview".to_string(),
            job_id: self.token.job_id.clone(),
            pid: self.token.pid.clone(),
            status: "success".to_string(),
            data: json!({
                "rename_plan_ref": plan_ref,
                "preview_ref": preview_ref,
                "operation_count": plan.operations.len(),
                "target_paths": plan.operations.iter()
                    .flat_map(|op| [op.source_path.clone(), op.destination_path.clone()])
                    .collect::<Vec<_>>(),
                "proposed_actions": ["workspace.rename_batch_apply"],
                "requires_approval": true,
            }),
        };
        self.emit_receipt(&receipt)?;
        Ok(receipt)
    }

    pub fn rename_batch_apply(
        &self,
        rename_plan_ref: &str,
        _approval_id: Option<&str>,
    ) -> io::Result<CapabilityReceipt> {
        if let Some(receipt) = self.ensure_capability("workspace.rename_batch_apply") {
            return Ok(receipt);
        }
        let plan = self.read_rename_plan(rename_plan_ref)?;
        let tx_id = new_tx_id("workspace_rename_batch");
        let mut renamed = Vec::new();
        for op in &plan.operations {
            let source = self.resolve_path(&op.source_path)?;
            let destination = self.resolve_path(&op.destination_path)?;
            if !source.exists() {
                continue;
            }
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent)?;
            }
            if destination.exists() {
                remove_path_any(&destination)?;
            }
            match fs::rename(&source, &destination) {
                Ok(()) => {}
                Err(_) => {
                    copy_path_recursive(&source, &destination)?;
                    remove_path_any(&source)?;
                }
            }
            renamed.push(op.clone());
        }
        let rollback_ref = self.truth.write_blob(
            &format!("tx/{}_rollback_plan.json", safe_blob_name(&tx_id)),
            &serde_json::to_vec_pretty(&json!({
                "tx_id": tx_id,
                "operation": "workspace.rename_batch_apply",
                "rollback_strategy": "move destination_path back to source_path in reverse order",
                "operations": renamed,
            }))
            .map_err(json_err)?,
        )?;
        if self.emit_events {
            self.truth.append_event(
                Some(&self.token.pid),
                "tx_recorded",
                json!({
                    "tx_id": tx_id,
                    "operation": "workspace.rename_batch_apply",
                    "tx_ref": rollback_ref,
                    "rollback_ref": rollback_ref,
                }),
            )?;
        }
        let receipt = CapabilityReceipt {
            capability_id: "workspace.rename_batch_apply".to_string(),
            job_id: self.token.job_id.clone(),
            pid: self.token.pid.clone(),
            status: "success".to_string(),
            data: json!({
                "rename_plan_ref": rename_plan_ref,
                "tx_id": tx_id,
                "rollback_ref": rollback_ref,
                "renamed_count": renamed.len(),
            }),
        };
        self.emit_receipt(&receipt)?;
        Ok(receipt)
    }

    pub fn tree_index(
        &self,
        source_set_ref: &str,
        tree_path: Option<&str>,
    ) -> io::Result<CapabilityReceipt> {
        if let Some(receipt) = self.ensure_capability("workspace.tree_index") {
            return Ok(receipt);
        }
        let source_set = self.read_source_set(source_set_ref)?;
        let tree = tree_markdown(&source_set);
        let tree_ref = self.truth.write_blob(
            &format!(
                "workspace_indexes/{}_tree.md",
                safe_blob_name(&source_set.source_set_id)
            ),
            tree.as_bytes(),
        )?;
        let artifact_path = tree_path.unwrap_or("TREE.md");
        let artifact = self.resolve_path(artifact_path)?;
        if let Some(parent) = artifact.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&artifact, tree.as_bytes())?;
        let receipt = CapabilityReceipt {
            capability_id: "workspace.tree_index".to_string(),
            job_id: self.token.job_id.clone(),
            pid: self.token.pid.clone(),
            status: "success".to_string(),
            data: json!({
                "source_set_ref": source_set_ref,
                "tree_ref": tree_ref,
                "artifact_path": artifact_path.replace('\\', "/"),
                "artifact_ref": format!("artifact://{}", artifact_path.replace('\\', "/")),
                "entry_count": source_set.file_count,
            }),
        };
        self.emit_receipt(&receipt)?;
        Ok(receipt)
    }

    pub fn perf_inventory(
        &self,
        source_set_ref: &str,
        output_path: Option<&str>,
        elapsed_ms: Option<u128>,
    ) -> io::Result<CapabilityReceipt> {
        if let Some(receipt) = self.ensure_capability("workspace.perf_inventory") {
            return Ok(receipt);
        }
        let started = Instant::now();
        let source_set = self.read_source_set(source_set_ref)?;
        let notes = json!({
            "capability": "workspace.perf_inventory",
            "source_set_ref": source_set_ref,
            "file_count": source_set.file_count,
            "total_bytes": source_set.total_bytes,
            "elapsed_ms": elapsed_ms.unwrap_or_else(|| started.elapsed().as_millis()),
        });
        let output_path = output_path.unwrap_or("PERF_NOTES.json");
        let output = self.resolve_path(output_path)?;
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(
            &output,
            serde_json::to_string_pretty(&notes)
                .map_err(json_err)?
                .as_bytes(),
        )?;
        let receipt = CapabilityReceipt {
            capability_id: "workspace.perf_inventory".to_string(),
            job_id: self.token.job_id.clone(),
            pid: self.token.pid.clone(),
            status: "success".to_string(),
            data: json!({
                "source_set_ref": source_set_ref,
                "artifact_path": output_path.replace('\\', "/"),
                "artifact_ref": format!("artifact://{}", output_path.replace('\\', "/")),
                "file_count": source_set.file_count,
                "total_bytes": source_set.total_bytes,
            }),
        };
        self.emit_receipt(&receipt)?;
        Ok(receipt)
    }

    pub fn export_dataset_csv(
        &self,
        dataset_ref: &str,
        output_path: &str,
    ) -> io::Result<CapabilityReceipt> {
        if let Some(receipt) = self.ensure_capability("dataset.export_csv") {
            return Ok(receipt);
        }
        let dataset = self.read_dataset(dataset_ref)?;
        let csv = dataset_to_csv(&dataset);
        let output = self.resolve_path(output_path)?;
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&output, csv.as_bytes())?;
        let receipt = CapabilityReceipt {
            capability_id: "dataset.export_csv".to_string(),
            job_id: self.token.job_id.clone(),
            pid: self.token.pid.clone(),
            status: "success".to_string(),
            data: json!({
                "dataset_ref": dataset_ref,
                "artifact_ref": format!("artifact://{}", output_path.replace('\\', "/")),
                "artifact_path": output_path.replace('\\', "/"),
                "row_count": dataset.row_count,
            }),
        };
        self.emit_receipt(&receipt)?;
        Ok(receipt)
    }

    pub fn export_dataset_markdown(
        &self,
        dataset_ref: &str,
        output_path: &str,
        title: &str,
    ) -> io::Result<CapabilityReceipt> {
        if let Some(receipt) = self.ensure_capability("dataset.export_markdown") {
            return Ok(receipt);
        }
        let dataset = self.read_dataset(dataset_ref)?;
        let markdown = dataset_to_markdown(&dataset, title);
        let output = self.resolve_path(output_path)?;
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&output, markdown.as_bytes())?;
        let receipt = CapabilityReceipt {
            capability_id: "dataset.export_markdown".to_string(),
            job_id: self.token.job_id.clone(),
            pid: self.token.pid.clone(),
            status: "success".to_string(),
            data: json!({
                "dataset_ref": dataset_ref,
                "artifact_ref": format!("artifact://{}", output_path.replace('\\', "/")),
                "artifact_path": output_path.replace('\\', "/"),
                "row_count": dataset.row_count,
            }),
        };
        self.emit_receipt(&receipt)?;
        Ok(receipt)
    }

    pub fn copy_source_set(
        &self,
        source_set_ref: &str,
        destination_dir: &str,
    ) -> io::Result<CapabilityReceipt> {
        if let Some(receipt) = self.ensure_capability("artifact.copy_source_set") {
            return Ok(receipt);
        }
        let source_set = self.read_source_set(source_set_ref)?;
        let destination_root = self.resolve_path(destination_dir)?;
        if destination_root.exists() {
            remove_path_any(&destination_root)?;
        }
        fs::create_dir_all(&destination_root)?;
        let mut copied = Vec::new();
        for file in &source_set.files {
            let source = self.resolve_path(&file.path)?;
            let target = destination_root.join(&file.path);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            copy_path_recursive(&source, &target)?;
            copied.push(file.path.clone());
        }
        let receipt = CapabilityReceipt {
            capability_id: "artifact.copy_source_set".to_string(),
            job_id: self.token.job_id.clone(),
            pid: self.token.pid.clone(),
            status: "success".to_string(),
            data: json!({
                "source_set_ref": source_set_ref,
                "artifact_path": destination_dir.replace('\\', "/"),
                "copied_count": copied.len(),
                "copied_paths": copied,
            }),
        };
        self.emit_receipt(&receipt)?;
        Ok(receipt)
    }

    pub fn read_source_set(&self, source_set_ref: &str) -> io::Result<SourceSet> {
        let path = self.truth.resolve_blob_ref(source_set_ref)?;
        serde_json::from_str(&fs::read_to_string(path)?).map_err(json_err)
    }

    pub fn read_dataset(&self, dataset_ref: &str) -> io::Result<DataSet> {
        let path = self.truth.resolve_blob_ref(dataset_ref)?;
        serde_json::from_str(&fs::read_to_string(path)?).map_err(json_err)
    }

    pub fn read_csv_dataset(
        &self,
        input_path: &str,
        has_header: bool,
        max_rows: usize,
    ) -> io::Result<CapabilityReceipt> {
        if let Some(receipt) = self.ensure_capability("data.csv.read_dataset") {
            return Ok(receipt);
        }
        let path = self.resolve_path(input_path)?;
        let text = fs::read_to_string(&path)?;
        let rows = parse_csv_records(&text)?;
        let mut iter = rows.into_iter();
        let first = iter.next().unwrap_or_default();
        let schema = if has_header && !first.is_empty() {
            first
                .iter()
                .enumerate()
                .map(|(index, value)| {
                    let trimmed = value.trim();
                    if trimmed.is_empty() {
                        format!("column_{}", index + 1)
                    } else {
                        trimmed.to_string()
                    }
                })
                .collect::<Vec<_>>()
        } else {
            (0..first.len())
                .map(|index| format!("column_{}", index + 1))
                .collect::<Vec<_>>()
        };
        let data_rows = if has_header {
            iter.collect::<Vec<_>>()
        } else if first.is_empty() {
            iter.collect::<Vec<_>>()
        } else {
            let mut items = vec![first];
            items.extend(iter);
            items
        };
        let limited = data_rows
            .into_iter()
            .take(max_rows.max(1))
            .collect::<Vec<_>>();
        let records = limited
            .into_iter()
            .map(|row| {
                let mut object = serde_json::Map::new();
                for (index, column) in schema.iter().enumerate() {
                    object.insert(
                        column.clone(),
                        json!(row.get(index).cloned().unwrap_or_default()),
                    );
                }
                Value::Object(object)
            })
            .collect::<Vec<_>>();
        let source_path = input_path.replace('\\', "/");
        let row_count = records.len();
        let (dataset_ref, dataset) = self.write_dataset(
            "csv_read",
            None,
            "csv",
            records,
            json!({
                "source_path": source_path,
                "has_header": has_header,
                "max_rows": max_rows,
                "truncated": row_count >= max_rows.max(1),
            }),
        )?;
        let receipt = CapabilityReceipt {
            capability_id: "data.csv.read_dataset".to_string(),
            job_id: self.token.job_id.clone(),
            pid: self.token.pid.clone(),
            status: "success".to_string(),
            data: json!({
                "input_path": source_path,
                "dataset_ref": dataset_ref,
                "dataset_id": dataset.dataset_id,
                "schema": dataset.schema,
                "row_count": dataset.row_count,
                "derivation_type": "raw_table",
                "no_workspace_mutation": true,
            }),
        };
        self.emit_receipt(&receipt)?;
        Ok(receipt)
    }

    fn read_organize_plan(&self, organize_plan_ref: &str) -> io::Result<WorkspaceOrganizePlan> {
        let path = self.truth.resolve_blob_ref(organize_plan_ref)?;
        serde_json::from_str(&fs::read_to_string(path)?).map_err(json_err)
    }

    fn read_rename_plan(&self, rename_plan_ref: &str) -> io::Result<WorkspaceRenamePlan> {
        let path = self.truth.resolve_blob_ref(rename_plan_ref)?;
        serde_json::from_str(&fs::read_to_string(path)?).map_err(json_err)
    }

    fn write_dataset(
        &self,
        name: &str,
        source_set_ref: Option<String>,
        derivation_type: &str,
        records: Vec<Value>,
        coverage_report: Value,
    ) -> io::Result<(String, DataSet)> {
        let dataset = DataSet {
            dataset_id: format!("dataset_{}_{}", safe_blob_name(name), crate::now_ms()),
            schema: schema_from_records(&records),
            row_count: records.len(),
            source_set_ref,
            derivation_type: derivation_type.to_string(),
            records,
            coverage_report,
        };
        let dataset_ref = self.truth.write_blob(
            &format!("datasets/{}.json", safe_blob_name(&dataset.dataset_id)),
            &serde_json::to_vec_pretty(&dataset).map_err(json_err)?,
        )?;
        Ok((dataset_ref, dataset))
    }

    fn resolve_path(&self, relative_path: &str) -> io::Result<PathBuf> {
        self.guard
            .resolve_workspace_path(if relative_path.trim().is_empty() {
                "."
            } else {
                relative_path
            })
            .map_err(|err| io::Error::new(io::ErrorKind::PermissionDenied, err))
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
            Some(self.blocked_receipt(capability_id, &format!("{capability_id} not granted")))
        }
    }

    fn emit_receipt(&self, receipt: &CapabilityReceipt) -> io::Result<()> {
        if self.emit_events {
            self.truth.append_event(
                Some(&self.token.pid),
                "capability_receipt",
                to_json_value(receipt)?,
            )?;
        }
        Ok(())
    }

    fn blocked_receipt(&self, capability_id: &str, reason: &str) -> CapabilityReceipt {
        self.blocked_receipt_with_data(capability_id, json!({"reason": reason}))
    }

    fn blocked_receipt_with_data(&self, capability_id: &str, data: Value) -> CapabilityReceipt {
        let receipt = CapabilityReceipt {
            capability_id: capability_id.to_string(),
            job_id: self.token.job_id.clone(),
            pid: self.token.pid.clone(),
            status: "blocked".to_string(),
            data,
        };
        let _ = self.emit_receipt(&receipt);
        receipt
    }
}

fn normalize_rel(value: &str) -> String {
    value
        .trim()
        .trim_matches('/')
        .replace('\\', "/")
        .trim_start_matches("./")
        .to_string()
}

fn organize_group_for_path(path: &str) -> String {
    let normalized = path.replace('\\', "/");
    let stem = Path::new(&normalized)
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("misc");
    let mut token = stem
        .split(['_', '-', ' ', '（', '(', '['])
        .find(|item| !item.trim().is_empty())
        .unwrap_or("misc")
        .trim()
        .to_string();
    if token.chars().count() > 24 {
        token = token.chars().take(24).collect();
    }
    safe_component(&token)
}

fn safe_component(value: &str) -> String {
    let cleaned = value
        .chars()
        .map(|ch| match ch {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            _ => ch,
        })
        .collect::<String>();
    if cleaned.trim().is_empty() {
        "misc".to_string()
    } else {
        cleaned
    }
}

fn disambiguate_destination(destination: &str, used: &mut BTreeSet<String>) -> String {
    let path = Path::new(destination);
    let parent = path
        .parent()
        .map(|value| value.display().to_string().replace('\\', "/"))
        .unwrap_or_default();
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("file");
    let ext = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    for index in 2..10_000 {
        let candidate = if ext.is_empty() {
            format!("{parent}/{stem}_{index}")
        } else {
            format!("{parent}/{stem}_{index}.{ext}")
        };
        if !used.contains(&candidate) {
            return candidate;
        }
    }
    destination.to_string()
}

fn parse_rename_mappings(value: &Value) -> io::Result<Vec<WorkspaceMutationOp>> {
    let mappings = value
        .get("mappings")
        .and_then(Value::as_array)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "mappings missing"))?;
    let mut operations = Vec::new();
    for item in mappings {
        let source_path = item
            .get("source_path")
            .and_then(Value::as_str)
            .map(normalize_rel)
            .filter(|path| !path.trim().is_empty())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "source_path missing"))?;
        let destination_path = item
            .get("destination_path")
            .and_then(Value::as_str)
            .map(normalize_rel)
            .filter(|path| !path.trim().is_empty())
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "destination_path missing")
            })?;
        operations.push(WorkspaceMutationOp {
            source_path,
            destination_path,
            operation: "rename".to_string(),
        });
    }
    Ok(operations)
}

fn organize_plan_markdown(plan: &WorkspaceOrganizePlan) -> String {
    let mut out = String::new();
    out.push_str("# ORGANIZE_PREVIEW\n\n");
    out.push_str(&format!("- plan_id: `{}`\n", plan.plan_id));
    out.push_str(&format!("- source_set_ref: `{}`\n", plan.source_set_ref));
    out.push_str(&format!(
        "- destination_root: `{}`\n",
        plan.destination_root
    ));
    out.push_str(&format!(
        "- operation_count: `{}`\n\n",
        plan.operations.len()
    ));
    out.push_str("| operation | source_path | destination_path |\n");
    out.push_str("|---|---|---|\n");
    for op in &plan.operations {
        out.push_str(&format!(
            "| {} | `{}` | `{}` |\n",
            op.operation, op.source_path, op.destination_path
        ));
    }
    out
}

fn rename_plan_markdown(plan: &WorkspaceRenamePlan) -> String {
    let mut out = String::new();
    out.push_str("# RENAME_PREVIEW\n\n");
    out.push_str(&format!("- plan_id: `{}`\n", plan.plan_id));
    out.push_str(&format!(
        "- operation_count: `{}`\n\n",
        plan.operations.len()
    ));
    out.push_str("| operation | source_path | destination_path |\n");
    out.push_str("|---|---|---|\n");
    for op in &plan.operations {
        out.push_str(&format!(
            "| {} | `{}` | `{}` |\n",
            op.operation, op.source_path, op.destination_path
        ));
    }
    out
}

fn tree_markdown(source_set: &SourceSet) -> String {
    let mut out = String::new();
    out.push_str("# TREE\n\n");
    out.push_str(&format!(
        "- source_set_id: `{}`\n",
        source_set.source_set_id
    ));
    out.push_str(&format!("- file_count: `{}`\n", source_set.file_count));
    out.push_str(&format!("- total_bytes: `{}`\n\n", source_set.total_bytes));
    out.push_str("## Files\n\n");
    for file in &source_set.files {
        out.push_str(&format!("- `{}` ({} bytes)\n", file.path, file.size_bytes));
    }
    out
}

fn collect_source_files(
    root: &Path,
    current: &Path,
    depth: usize,
    max_depth: usize,
    include_extensions: &[String],
    include_globs: &[String],
    exclude_globs: &[String],
    files: &mut Vec<SourceSetFile>,
    diagnostics: &mut SourceSetScanDiagnostics,
) -> io::Result<()> {
    if depth > max_depth {
        diagnostics.skipped_by_depth += 1;
        return Ok(());
    }
    let mut children = Vec::new();
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        if entry.file_name().to_string_lossy() == RUNTIME_DIR_NAME {
            continue;
        }
        children.push(entry.path());
    }
    children.sort();
    for path in children {
        if path.is_dir() {
            collect_source_files(
                root,
                &path,
                depth + 1,
                max_depth,
                include_extensions,
                include_globs,
                exclude_globs,
                files,
                diagnostics,
            )?;
            continue;
        }
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .display()
            .to_string()
            .replace('\\', "/");
        if exclude_globs
            .iter()
            .any(|pattern| glob_like_match(&rel, pattern))
        {
            diagnostics.scanned_file_count += 1;
            diagnostics.excluded_by_explicit_exclude += 1;
            continue;
        }
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .map(|value| format!(".{}", value.to_ascii_lowercase()))
            .unwrap_or_default();
        if !include_extensions.is_empty()
            && !include_extensions.iter().any(|item| item == &extension)
        {
            diagnostics.scanned_file_count += 1;
            diagnostics.excluded_by_extension += 1;
            continue;
        }
        if !include_globs.is_empty()
            && !include_globs
                .iter()
                .any(|pattern| glob_like_match(&rel, pattern))
        {
            diagnostics.scanned_file_count += 1;
            diagnostics.excluded_by_include_glob += 1;
            continue;
        }
        diagnostics.scanned_file_count += 1;
        let metadata = path.metadata()?;
        let modified_ms = metadata
            .modified()
            .ok()
            .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
            .map(|value| value.as_millis())
            .unwrap_or(0);
        files.push(SourceSetFile {
            path: rel,
            extension,
            size_bytes: metadata.len(),
            modified_ms,
        });
    }
    Ok(())
}

fn zero_match_reason(
    file_count: usize,
    diagnostics: &SourceSetScanDiagnostics,
    extension_filter_present: bool,
    include_glob_filter_present: bool,
) -> Option<String> {
    if file_count > 0 {
        return None;
    }
    if diagnostics.scanned_file_count == 0 && diagnostics.skipped_by_depth > 0 {
        return Some("max_depth prevented traversal from reaching files".to_string());
    }
    if diagnostics.scanned_file_count == 0 {
        return Some("root_path contains no files within max_depth".to_string());
    }
    if extension_filter_present
        && diagnostics.excluded_by_extension == diagnostics.scanned_file_count
    {
        return Some("include_extensions matched no scanned files".to_string());
    }
    if include_glob_filter_present
        && diagnostics.excluded_by_include_glob == diagnostics.scanned_file_count
    {
        return Some("include_globs matched no scanned files".to_string());
    }
    if diagnostics.excluded_by_explicit_exclude == diagnostics.scanned_file_count {
        return Some("exclude_globs filtered every scanned file".to_string());
    }
    Some("filters matched no files".to_string())
}

fn normalize_extensions(values: &[String]) -> Vec<String> {
    values
        .iter()
        .map(|item| item.trim().to_ascii_lowercase())
        .filter(|item| !item.is_empty())
        .map(|item| {
            if item.starts_with('.') {
                item
            } else {
                format!(".{item}")
            }
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn glob_like_match(path: &str, pattern: &str) -> bool {
    let pattern = pattern.replace('\\', "/");
    if pattern == "*" || pattern == "**/*" {
        return true;
    }
    if !pattern.contains('*') {
        return path == pattern || path.contains(&pattern);
    }
    let parts = pattern.split('*').collect::<Vec<_>>();
    let mut cursor = 0;
    if let Some(first) = parts.first() {
        if !first.is_empty() {
            if !path.starts_with(first) {
                return false;
            }
            cursor = first.len();
        }
    }
    for part in parts.iter().skip(1) {
        if part.is_empty() {
            continue;
        }
        if let Some(found) = path[cursor..].find(part) {
            cursor += found + part.len();
        } else {
            return false;
        }
    }
    if let Some(last) = parts.last() {
        if !last.is_empty() && !pattern.ends_with('*') {
            return path.ends_with(last);
        }
    }
    true
}

fn schema_from_records(records: &[Value]) -> Vec<String> {
    let mut keys = BTreeSet::new();
    for record in records {
        if let Some(map) = record.as_object() {
            for key in map.keys() {
                keys.insert(key.clone());
            }
        }
    }
    keys.into_iter().collect()
}

fn dataset_to_csv(dataset: &DataSet) -> String {
    let headers = if dataset.schema.is_empty() {
        vec!["value".to_string()]
    } else {
        dataset.schema.clone()
    };
    let mut out = String::new();
    out.push_str(
        &headers
            .iter()
            .map(|item| csv_escape(item))
            .collect::<Vec<_>>()
            .join(","),
    );
    out.push('\n');
    for record in &dataset.records {
        let row = headers
            .iter()
            .map(|key| record.get(key).map(value_to_cell).unwrap_or_default())
            .map(|item| csv_escape(&item))
            .collect::<Vec<_>>()
            .join(",");
        out.push_str(&row);
        out.push('\n');
    }
    out
}

fn dataset_to_markdown(dataset: &DataSet, title: &str) -> String {
    let mut out = format!("# {title}\n\n");
    out.push_str(&format!("- row_count: {}\n", dataset.row_count));
    out.push_str(&format!("- derivation_type: {}\n", dataset.derivation_type));
    if let Some(source_set_ref) = &dataset.source_set_ref {
        out.push_str(&format!("- source_set_ref: `{source_set_ref}`\n"));
    }
    out.push_str("\n## Records\n\n");
    for (index, record) in dataset.records.iter().enumerate() {
        out.push_str(&format!("### Record {}\n\n", index + 1));
        if let Some(map) = record.as_object() {
            for (key, value) in map {
                out.push_str(&format!("- {key}: {}\n", value_to_cell(value)));
            }
        } else {
            out.push_str(&format!("- value: {}\n", value_to_cell(record)));
        }
        out.push('\n');
    }
    out
}

fn value_to_cell(value: &Value) -> String {
    match value {
        Value::Null => "".to_string(),
        Value::String(value) => value.clone(),
        Value::Number(value) => value.to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Array(items) => items
            .iter()
            .map(value_to_cell)
            .collect::<Vec<_>>()
            .join("; "),
        Value::Object(_) => serde_json::to_string(value).unwrap_or_default(),
    }
}

fn csv_escape(value: &str) -> String {
    if value.contains(',') || value.contains('"') || value.contains('\n') || value.contains('\r') {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

fn parse_csv_records(text: &str) -> io::Result<Vec<Vec<String>>> {
    let mut rows = Vec::<Vec<String>>::new();
    let mut row = Vec::<String>::new();
    let mut field = String::new();
    let mut chars = text.chars().peekable();
    let mut in_quotes = false;
    while let Some(ch) = chars.next() {
        match ch {
            '"' if in_quotes && chars.peek() == Some(&'"') => {
                field.push('"');
                chars.next();
            }
            '"' => in_quotes = !in_quotes,
            ',' if !in_quotes => {
                row.push(std::mem::take(&mut field));
            }
            '\n' if !in_quotes => {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
            }
            '\r' if !in_quotes => {}
            _ => field.push(ch),
        }
    }
    if in_quotes {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unterminated quoted CSV field",
        ));
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    Ok(rows)
}

fn purpose_from_path(path: &str) -> &'static str {
    let lower = path.to_ascii_lowercase();
    if lower.contains("meeting") || path.contains("会议") {
        "meeting record or action item source"
    } else if lower.contains("risk") || path.contains("风险") {
        "risk or issue management source"
    } else if lower.contains("feedback") || path.contains("反馈") {
        "customer feedback source"
    } else if lower.contains("deliverable") || path.contains("交付") {
        "delivery artifact source"
    } else if lower.contains("draft") || path.contains("草稿") {
        "draft or versioned source"
    } else {
        "workspace source"
    }
}
