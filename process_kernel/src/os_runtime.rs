use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{
    artifact_path_generated_by_current_job, copy_path_recursive, file_fingerprint,
    is_valid_write_kind, json_err, new_tx_id, path_fingerprint, path_kind, path_size,
    read_store_zip, remove_path_any, restore_tx_backup, safe_blob_name, text_file_diff,
    to_json_value, walk_workspace, write_store_zip, CapabilityReceipt, CapabilityToken,
    ProcessEvent, ProcessTruthStore, WorkspaceGuard, RUNTIME_DIR_NAME,
};
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct OsTxRecord {
    pub tx_id: String,
    pub operation: String,
    pub source_path: Option<String>,
    pub destination_path: Option<String>,
    pub write_kind: Option<String>,
    pub source_before_exists: bool,
    pub destination_before_exists: bool,
    pub source_backup_ref: Option<String>,
    pub destination_backup_ref: Option<String>,
}

#[derive(Clone, Debug)]
pub struct OsRuntime {
    guard: WorkspaceGuard,
    truth: ProcessTruthStore,
    token: CapabilityToken,
    emit_events: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceInventoryEntry {
    pub path: String,
    pub kind: String,
    pub top_level: String,
    pub extension: String,
    pub size_bytes: u64,
    pub readable_document: bool,
    pub document_type: String,
    pub title: String,
    pub purpose_hint: String,
}

impl OsRuntime {
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

    pub fn list_tree(&self, max_depth: usize) -> io::Result<CapabilityReceipt> {
        if !self
            .token
            .capabilities
            .iter()
            .any(|item| item == "os.list_tree")
        {
            return Ok(self.blocked_receipt("os.list_tree", "os.list_tree not granted"));
        }
        let mut entries = Vec::new();
        walk_workspace(
            self.guard.root(),
            self.guard.root(),
            0,
            max_depth,
            &mut entries,
        )?;
        let content = entries.join("\n");
        let source_set_ref = self
            .truth
            .write_blob("source_set_tree.txt", content.as_bytes())?;
        let receipt = CapabilityReceipt {
            capability_id: "os.list_tree".to_string(),
            job_id: self.token.job_id.clone(),
            pid: self.token.pid.clone(),
            status: "success".to_string(),
            data: json!({
                "source_set_ref": source_set_ref,
                "entry_count": entries.len(),
                "max_depth": max_depth,
            }),
        };
        self.emit_receipt("capability_receipt", &receipt)?;
        Ok(receipt)
    }

    pub fn workspace_inventory(&self, max_depth: usize) -> io::Result<CapabilityReceipt> {
        if let Some(receipt) = self.ensure_capability("os.workspace_inventory") {
            return Ok(receipt);
        }
        let mut entries = Vec::new();
        collect_workspace_inventory(
            self.guard.root(),
            self.guard.root(),
            0,
            max_depth,
            &mut entries,
        )?;
        let document_entries = entries
            .iter()
            .filter(|item| item.readable_document)
            .cloned()
            .collect::<Vec<_>>();
        let total_size_bytes = entries.iter().map(|item| item.size_bytes).sum::<u64>();
        let mut extension_counts: BTreeMap<String, usize> = BTreeMap::new();
        let mut top_level_counts: BTreeMap<String, usize> = BTreeMap::new();
        for entry in &entries {
            *extension_counts.entry(entry.extension.clone()).or_insert(0) += 1;
            *top_level_counts.entry(entry.top_level.clone()).or_insert(0) += 1;
        }
        let inventory_ref = self.truth.write_blob(
            "raw_tool_results/workspace_inventory/workspace_inventory.json",
            &serde_json::to_vec_pretty(&entries).map_err(json_err)?,
        )?;
        let document_index_csv = document_index_csv(&document_entries);
        let document_index_csv_ref = self.truth.write_blob(
            "raw_tool_results/workspace_inventory/document_index.csv",
            document_index_csv.as_bytes(),
        )?;
        let workspace_map = workspace_map_markdown(&entries, &extension_counts);
        let workspace_map_ref = self.truth.write_blob(
            "raw_tool_results/workspace_inventory/workspace_map.md",
            workspace_map.as_bytes(),
        )?;
        let receipt = CapabilityReceipt {
            capability_id: "os.workspace_inventory".to_string(),
            job_id: self.token.job_id.clone(),
            pid: self.token.pid.clone(),
            status: "success".to_string(),
            data: json!({
                "inventory_ref": inventory_ref,
                "document_index_csv_ref": document_index_csv_ref,
                "workspace_map_ref": workspace_map_ref,
                "entry_count": entries.len(),
                "document_count": document_entries.len(),
                "total_size_bytes": total_size_bytes,
                "extension_counts": extension_counts,
                "top_level_counts": top_level_counts,
                "max_depth": max_depth,
            }),
        };
        self.emit_receipt("capability_receipt", &receipt)?;
        Ok(receipt)
    }

    pub fn write_artifact(
        &self,
        relative_path: &str,
        content: &[u8],
    ) -> io::Result<CapabilityReceipt> {
        self.write_file_with_capability("os.write_artifact", relative_path, content, "artifact")
    }

    pub fn write_temp_dataset(
        &self,
        relative_path: &str,
        content: &[u8],
    ) -> io::Result<CapabilityReceipt> {
        self.write_file_with_capability(
            "os.write_temp_dataset",
            relative_path,
            content,
            "temp_dataset",
        )
    }

    pub fn write_source_mutation_preview(
        &self,
        relative_path: &str,
        content: &[u8],
    ) -> io::Result<CapabilityReceipt> {
        if let Some(receipt) = self.ensure_capability("os.write_source_mutation_preview") {
            return Ok(receipt);
        }
        let path = match self.guard.resolve_workspace_path(relative_path) {
            Ok(path) => path,
            Err(err) => {
                return Ok(self.blocked_receipt_with_data(
                    "os.write_source_mutation_preview",
                    json!({
                        "reason": "workspace_boundary_violation",
                        "message": err,
                        "path": relative_path.replace('\\', "/"),
                        "no_file_written": true,
                    }),
                ));
            }
        };
        let normalized_path = relative_path.replace('\\', "/");
        let content_ref = self.truth.write_blob(
            &format!(
                "source_mutation_previews/{}_content.bin",
                safe_blob_name(&normalized_path)
            ),
            content,
        )?;
        let preview_text = String::from_utf8_lossy(content);
        let preview_markdown = format!(
            "# Source Mutation Preview\n\n- target_path: `{}`\n- target_exists: `{}`\n- content_bytes: `{}`\n\n```text\n{}\n```\n",
            normalized_path,
            path.exists(),
            content.len(),
            preview_text.chars().take(4000).collect::<String>()
        );
        let preview_ref = self.truth.write_blob(
            &format!(
                "source_mutation_previews/{}_preview.md",
                safe_blob_name(&normalized_path)
            ),
            preview_markdown.as_bytes(),
        )?;
        let receipt = CapabilityReceipt {
            capability_id: "os.write_source_mutation_preview".to_string(),
            job_id: self.token.job_id.clone(),
            pid: self.token.pid.clone(),
            status: "success".to_string(),
            data: json!({
                "path": normalized_path,
                "target_paths": [normalized_path],
                "proposed_actions": ["os.write_source_mutation_apply"],
                "content_ref": content_ref,
                "preview_ref": preview_ref,
                "content_bytes": content.len(),
                "target_exists": path.exists(),
                "no_file_written": true,
                "runtime_note": "preview only; TaskAgent must wait for the user decision. Kernel-owned approval execution will run the original pending mutation after approval and return the tool result to the model.",
            }),
        };
        self.emit_receipt("capability_receipt", &receipt)?;
        Ok(receipt)
    }

    pub fn write_source_mutation_apply(
        &self,
        relative_path: &str,
        content: &[u8],
    ) -> io::Result<CapabilityReceipt> {
        self.write_file_with_capability(
            "os.write_source_mutation_apply",
            relative_path,
            content,
            "source_mutation",
        )
    }

    pub fn stat_path(&self, relative_path: &str) -> io::Result<CapabilityReceipt> {
        if let Some(receipt) = self.ensure_capability("os.stat_path") {
            return Ok(receipt);
        }
        let path = self.resolve_path(relative_path)?;
        let exists = path.exists();
        let kind = path_kind(&path);
        let size_bytes = if exists { path_size(&path)? } else { 0 };
        let fingerprint = if exists {
            Some(path_fingerprint(self.guard.root(), &path)?)
        } else {
            None
        };
        let receipt = CapabilityReceipt {
            capability_id: "os.stat_path".to_string(),
            job_id: self.token.job_id.clone(),
            pid: self.token.pid.clone(),
            status: if exists { "success" } else { "failed" }.to_string(),
            data: json!({
                "path": relative_path.replace('\\', "/"),
                "exists": exists,
                "kind": kind,
                "size_bytes": size_bytes,
                "fingerprint": fingerprint,
            }),
        };
        self.emit_receipt("capability_receipt", &receipt)?;
        Ok(receipt)
    }

    pub fn read_file(&self, relative_path: &str) -> io::Result<CapabilityReceipt> {
        if let Some(receipt) = self.ensure_capability("os.read_file") {
            return Ok(receipt);
        }
        let path = self.resolve_path(relative_path)?;
        if !path.is_file() {
            return Ok(self.blocked_receipt_with_data(
                "os.read_file",
                json!({
                    "reason": "read target is not a file",
                    "path": relative_path.replace('\\', "/"),
                    "exists": path.exists(),
                    "target_kind": path_kind(&path),
                    "recoverable": true,
                    "recommended_capabilities": [
                        "os.list_tree",
                        "os.stat_path",
                        "source_set.create",
                        "source_set.read_page"
                    ],
                    "corrective_instruction": "Use os.list_tree or os.stat_path for directories. For project-scale source inspection, create a source set and read pages instead of repeatedly calling os.read_file on directories."
                }),
            ));
        }
        let content = fs::read(&path)?;
        let safe_name = safe_blob_name(relative_path);
        let dataset_ref = self
            .truth
            .write_blob(&format!("datasets/{safe_name}"), &content)?;
        let receipt = CapabilityReceipt {
            capability_id: "os.read_file".to_string(),
            job_id: self.token.job_id.clone(),
            pid: self.token.pid.clone(),
            status: "success".to_string(),
            data: json!({
                "source_path": relative_path.replace('\\', "/"),
                "dataset_ref": dataset_ref,
                "size_bytes": content.len(),
                "fingerprint": file_fingerprint(&path)?,
            }),
        };
        self.emit_receipt("capability_receipt", &receipt)?;
        Ok(receipt)
    }

    pub fn write_file(
        &self,
        relative_path: &str,
        content: &[u8],
        write_kind: &str,
    ) -> io::Result<CapabilityReceipt> {
        self.write_file_with_capability("os.write_file", relative_path, content, write_kind)
    }

    pub fn write_file_with_approval(
        &self,
        relative_path: &str,
        content: &[u8],
        write_kind: &str,
        _approval_id: Option<&str>,
    ) -> io::Result<CapabilityReceipt> {
        self.write_file_with_capability("os.write_file", relative_path, content, write_kind)
    }

    pub fn write_file_missing_write_kind(
        &self,
        relative_path: &str,
    ) -> io::Result<CapabilityReceipt> {
        if let Some(receipt) = self.ensure_capability("os.write_file") {
            return Ok(receipt);
        }
        let normalized_path = relative_path.replace('\\', "/");
        let detected_intent = if looks_like_temp_dataset_path(&normalized_path) {
            "temp_dataset_candidate"
        } else {
            "artifact_candidate"
        };
        Ok(self.blocked_receipt_with_data(
            "os.write_file",
            json!({
                "reason": "missing_write_kind",
                "schema_error": true,
                "recoverable_by_task_agent": true,
                "target_path": normalized_path,
                "accepted_write_kinds": ["artifact", "source_mutation", "temp_dataset"],
                "detected_intent": detected_intent,
                "no_file_written": true,
                "minimal_valid_examples": {
                    "artifact": {
                        "capability_id": "os.write_artifact",
                        "arguments": {"path": normalized_path, "content": "..."}
                    },
                    "temp_dataset": {
                        "capability_id": "os.write_temp_dataset",
                        "arguments": {"path": normalized_path, "content": "..."}
                    },
                    "source_mutation": {
                        "capability_id": "os.write_source_mutation_preview",
                        "arguments": {"path": normalized_path, "content": "..."}
                    }
                },
                "runtime_note": "os.write_file is compatibility-only; prefer explicit write capabilities",
            }),
        ))
    }

    fn write_file_with_capability(
        &self,
        capability_id: &str,
        relative_path: &str,
        content: &[u8],
        write_kind: &str,
    ) -> io::Result<CapabilityReceipt> {
        if let Some(receipt) = self.ensure_capability(capability_id) {
            return Ok(receipt);
        }
        if !is_valid_write_kind(write_kind) {
            return Ok(self.blocked_receipt_with_data(
                capability_id,
                json!({
                    "reason": "invalid_write_kind",
                    "message": "write_kind must be artifact, source_mutation, or temp_dataset",
                    "schema_error": true,
                    "recoverable_by_task_agent": true,
                    "received_write_kind": write_kind,
                    "accepted_write_kinds": ["artifact", "source_mutation", "temp_dataset"],
                    "no_file_written": true,
                }),
            ));
        }
        let path = self
            .guard
            .resolve_workspace_path(relative_path)
            .map_err(|err| io::Error::new(io::ErrorKind::PermissionDenied, err))?;
        let normalized_path = relative_path.replace('\\', "/");
        let _mutation_lock =
            match self.acquire_mutation_locks(capability_id, &[normalized_path.clone()])? {
                MutationLockResult::Acquired(locks) => locks,
                MutationLockResult::Blocked(receipt) => return Ok(receipt),
            };
        let tx_id = new_tx_id("os_write");
        let is_artifact_revision = write_kind == "artifact"
            && path.exists()
            && artifact_path_generated_by_current_job(&self.truth, &normalized_path)?;
        let previous_hash = if is_artifact_revision {
            Some(file_fingerprint(&path)?)
        } else {
            None
        };
        let destination_backup = self.backup_workspace_path(&tx_id, "destination", &path)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, content)?;
        let new_hash = if is_artifact_revision {
            Some(file_fingerprint(&path)?)
        } else {
            None
        };
        let tx = OsTxRecord {
            tx_id: tx_id.clone(),
            operation: capability_id.to_string(),
            source_path: None,
            destination_path: Some(normalized_path.clone()),
            write_kind: Some(write_kind.to_string()),
            source_before_exists: false,
            destination_before_exists: destination_backup.0,
            source_backup_ref: None,
            destination_backup_ref: destination_backup.1,
        };
        let tx_ref = self.record_tx(&tx)?;
        let version_tx_ref = if is_artifact_revision {
            let version_index = artifact_revision_count(&self.truth, &normalized_path)? + 1;
            let diff_summary_ref = self.truth.write_blob(
                &format!(
                    "artifact_versions/{}_{}_diff_summary.json",
                    safe_blob_name(&normalized_path),
                    version_index
                ),
                serde_json::to_string_pretty(&json!({
                    "artifact_path": normalized_path.clone(),
                    "previous_hash": previous_hash.clone(),
                    "new_hash": new_hash.clone(),
                    "revision_index": version_index,
                    "producer_capability": capability_id,
                    "tx_id": tx_id.clone(),
                    "summary": "artifact content was rewritten by the same AgentJob; this is recorded as an artifact version transaction, not a user source mutation",
                }))
                .map_err(json_err)?
                .as_bytes(),
            )?;
            if self.emit_events {
                self.truth.append_event(
                    Some(&self.token.pid),
                    "artifact_version_tx",
                    json!({
                        "artifact_path": normalized_path.clone(),
                        "previous_hash": previous_hash.clone(),
                        "new_hash": new_hash.clone(),
                        "revision_index": version_index,
                        "producer_capability": capability_id,
                        "source_refs": [],
                        "diff_summary_ref": diff_summary_ref,
                        "tx_id": tx_id.clone(),
                        "status": "recorded",
                    }),
                )?;
            }
            Some(diff_summary_ref)
        } else {
            None
        };
        let artifact_ref = format!("artifact://{}", normalized_path);
        let receipt = CapabilityReceipt {
            capability_id: capability_id.to_string(),
            job_id: self.token.job_id.clone(),
            pid: self.token.pid.clone(),
            status: "success".to_string(),
            data: json!({
                "artifact_ref": artifact_ref,
                "artifact_path": normalized_path,
                "size_bytes": content.len(),
                "write_kind": write_kind,
                "tx_id": tx_id,
                "tx_ref": tx_ref,
                "artifact_version_tx_ref": version_tx_ref,
            }),
        };
        self.emit_receipt("capability_receipt", &receipt)?;
        Ok(receipt)
    }

    pub fn copy_path(
        &self,
        source_path: &str,
        destination_path: &str,
    ) -> io::Result<CapabilityReceipt> {
        self.copy_like("os.copy_path", source_path, destination_path, false)
    }

    pub fn copy_path_with_approval(
        &self,
        source_path: &str,
        destination_path: &str,
        _approval_id: Option<&str>,
    ) -> io::Result<CapabilityReceipt> {
        self.copy_like("os.copy_path", source_path, destination_path, false)
    }

    pub fn move_path(
        &self,
        source_path: &str,
        destination_path: &str,
    ) -> io::Result<CapabilityReceipt> {
        self.copy_like("os.move_path", source_path, destination_path, true)
    }

    pub fn move_path_with_approval(
        &self,
        source_path: &str,
        destination_path: &str,
        _approval_id: Option<&str>,
    ) -> io::Result<CapabilityReceipt> {
        self.copy_like("os.move_path", source_path, destination_path, true)
    }

    pub fn rename_path(
        &self,
        source_path: &str,
        destination_path: &str,
    ) -> io::Result<CapabilityReceipt> {
        self.copy_like("os.rename_path", source_path, destination_path, true)
    }

    pub fn rename_path_with_approval(
        &self,
        source_path: &str,
        destination_path: &str,
        _approval_id: Option<&str>,
    ) -> io::Result<CapabilityReceipt> {
        self.copy_like("os.rename_path", source_path, destination_path, true)
    }

    pub fn delete_path(&self, relative_path: &str) -> io::Result<CapabilityReceipt> {
        self.delete_path_with_approval(relative_path, None)
    }

    pub fn delete_path_with_approval(
        &self,
        relative_path: &str,
        _approval_id: Option<&str>,
    ) -> io::Result<CapabilityReceipt> {
        if let Some(receipt) = self.ensure_capability("os.delete_path") {
            return Ok(receipt);
        }
        let path = self.resolve_path(relative_path)?;
        if !path.exists() {
            return Ok(self.blocked_receipt("os.delete_path", "delete target does not exist"));
        }
        let normalized_path = relative_path.replace('\\', "/");
        let _mutation_lock =
            match self.acquire_mutation_locks("os.delete_path", &[normalized_path.clone()])? {
                MutationLockResult::Acquired(locks) => locks,
                MutationLockResult::Blocked(receipt) => return Ok(receipt),
            };
        let tx_id = new_tx_id("os_delete");
        let source_backup = self.backup_workspace_path(&tx_id, "source", &path)?;
        remove_path_any(&path)?;
        let tx = OsTxRecord {
            tx_id: tx_id.clone(),
            operation: "os.delete_path".to_string(),
            source_path: Some(normalized_path.clone()),
            destination_path: None,
            write_kind: Some("source_mutation".to_string()),
            source_before_exists: source_backup.0,
            destination_before_exists: false,
            source_backup_ref: source_backup.1,
            destination_backup_ref: None,
        };
        let tx_ref = self.record_tx(&tx)?;
        let receipt = CapabilityReceipt {
            capability_id: "os.delete_path".to_string(),
            job_id: self.token.job_id.clone(),
            pid: self.token.pid.clone(),
            status: "success".to_string(),
            data: json!({
                "target_path": normalized_path,
                "tx_id": tx_id,
                "tx_ref": tx_ref,
            }),
        };
        self.emit_receipt("capability_receipt", &receipt)?;
        Ok(receipt)
    }

    pub fn hash_path(&self, relative_path: &str) -> io::Result<CapabilityReceipt> {
        if let Some(receipt) = self.ensure_capability("os.hash_path") {
            return Ok(receipt);
        }
        let path = self.resolve_path(relative_path)?;
        if !path.exists() {
            return Ok(self.blocked_receipt("os.hash_path", "hash target does not exist"));
        }
        let hash = path_fingerprint(self.guard.root(), &path)?;
        let hash_ref = self.truth.write_blob(
            &format!("hash/{}.txt", safe_blob_name(relative_path)),
            hash.as_bytes(),
        )?;
        let receipt = CapabilityReceipt {
            capability_id: "os.hash_path".to_string(),
            job_id: self.token.job_id.clone(),
            pid: self.token.pid.clone(),
            status: "success".to_string(),
            data: json!({
                "path": relative_path.replace('\\', "/"),
                "hash": hash,
                "hash_ref": hash_ref,
            }),
        };
        self.emit_receipt("capability_receipt", &receipt)?;
        Ok(receipt)
    }

    pub fn diff_files(&self, left_path: &str, right_path: &str) -> io::Result<CapabilityReceipt> {
        if let Some(receipt) = self.ensure_capability("os.diff") {
            return Ok(receipt);
        }
        let left = self.resolve_path(left_path)?;
        let right = self.resolve_path(right_path)?;
        if !left.is_file() || !right.is_file() {
            return Ok(self.blocked_receipt("os.diff", "diff targets must both be files"));
        }
        let diff = text_file_diff(&left, &right)?;
        let diff_ref = self.truth.write_blob(
            &format!(
                "diff/{}_to_{}.diff",
                safe_blob_name(left_path),
                safe_blob_name(right_path)
            ),
            diff.as_bytes(),
        )?;
        let receipt = CapabilityReceipt {
            capability_id: "os.diff".to_string(),
            job_id: self.token.job_id.clone(),
            pid: self.token.pid.clone(),
            status: "success".to_string(),
            data: json!({
                "left_path": left_path.replace('\\', "/"),
                "right_path": right_path.replace('\\', "/"),
                "diff_ref": diff_ref,
            }),
        };
        self.emit_receipt("capability_receipt", &receipt)?;
        Ok(receipt)
    }

    pub fn zip_paths(
        &self,
        source_paths: &[&str],
        destination_zip_path: &str,
    ) -> io::Result<CapabilityReceipt> {
        if let Some(receipt) = self.ensure_capability("os.zip") {
            return Ok(receipt);
        }
        let tx_id = new_tx_id("os_zip");
        let destination = self.resolve_path(destination_zip_path)?;
        let destination_backup = self.backup_workspace_path(&tx_id, "destination", &destination)?;
        let mut sources = Vec::new();
        for source in source_paths {
            sources.push((source.replace('\\', "/"), self.resolve_path(source)?));
        }
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        let entry_count = write_store_zip(self.guard.root(), &sources, &destination)?;
        let tx = OsTxRecord {
            tx_id: tx_id.clone(),
            operation: "os.zip".to_string(),
            source_path: None,
            destination_path: Some(destination_zip_path.replace('\\', "/")),
            write_kind: Some("artifact".to_string()),
            source_before_exists: false,
            destination_before_exists: destination_backup.0,
            source_backup_ref: None,
            destination_backup_ref: destination_backup.1,
        };
        let tx_ref = self.record_tx(&tx)?;
        let receipt = CapabilityReceipt {
            capability_id: "os.zip".to_string(),
            job_id: self.token.job_id.clone(),
            pid: self.token.pid.clone(),
            status: "success".to_string(),
            data: json!({
                "archive_path": destination_zip_path.replace('\\', "/"),
                "entry_count": entry_count,
                "tx_id": tx_id,
                "tx_ref": tx_ref,
            }),
        };
        self.emit_receipt("capability_receipt", &receipt)?;
        Ok(receipt)
    }

    pub fn unzip_archive(
        &self,
        archive_path: &str,
        destination_dir: &str,
    ) -> io::Result<CapabilityReceipt> {
        self.unzip_archive_with_approval(archive_path, destination_dir, None)
    }

    pub fn unzip_archive_with_approval(
        &self,
        archive_path: &str,
        destination_dir: &str,
        _approval_id: Option<&str>,
    ) -> io::Result<CapabilityReceipt> {
        if let Some(receipt) = self.ensure_capability("os.unzip") {
            return Ok(receipt);
        }
        let archive = self.resolve_path(archive_path)?;
        let destination = self.resolve_path(destination_dir)?;
        if !archive.is_file() {
            return Ok(self.blocked_receipt("os.unzip", "archive target is not a file"));
        }
        let tx_id = new_tx_id("os_unzip");
        let destination_backup = self.backup_workspace_path(&tx_id, "destination", &destination)?;
        fs::create_dir_all(&destination)?;
        let entry_count = read_store_zip(&archive, &destination)?;
        let tx = OsTxRecord {
            tx_id: tx_id.clone(),
            operation: "os.unzip".to_string(),
            source_path: Some(archive_path.replace('\\', "/")),
            destination_path: Some(destination_dir.replace('\\', "/")),
            write_kind: Some("artifact".to_string()),
            source_before_exists: true,
            destination_before_exists: destination_backup.0,
            source_backup_ref: None,
            destination_backup_ref: destination_backup.1,
        };
        let tx_ref = self.record_tx(&tx)?;
        let receipt = CapabilityReceipt {
            capability_id: "os.unzip".to_string(),
            job_id: self.token.job_id.clone(),
            pid: self.token.pid.clone(),
            status: "success".to_string(),
            data: json!({
                "archive_path": archive_path.replace('\\', "/"),
                "destination_dir": destination_dir.replace('\\', "/"),
                "entry_count": entry_count,
                "tx_id": tx_id,
                "tx_ref": tx_ref,
            }),
        };
        self.emit_receipt("capability_receipt", &receipt)?;
        Ok(receipt)
    }

    pub fn rollback_tx(&self, tx_id: &str) -> io::Result<CapabilityReceipt> {
        if let Some(receipt) = self.ensure_capability("os.rollback_tx") {
            return Ok(receipt);
        }
        let tx_ref = format!(
            "blob://{}/tx/{}.json",
            self.token.job_id,
            safe_blob_name(tx_id)
        );
        let tx_path = self.truth.resolve_blob_ref(&tx_ref)?;
        if !tx_path.is_file() {
            return Ok(self.blocked_receipt("os.rollback_tx", "tx record does not exist"));
        }
        let record: OsTxRecord =
            serde_json::from_str(&fs::read_to_string(&tx_path)?).map_err(json_err)?;
        self.restore_tx_record(&record)?;
        let receipt = CapabilityReceipt {
            capability_id: "os.rollback_tx".to_string(),
            job_id: self.token.job_id.clone(),
            pid: self.token.pid.clone(),
            status: "success".to_string(),
            data: json!({
                "tx_id": tx_id,
                "rolled_back_operation": record.operation,
            }),
        };
        self.emit_receipt("tx_rollback", &receipt)?;
        self.emit_receipt("capability_receipt", &receipt)?;
        Ok(receipt)
    }

    pub fn verify_artifact(&self, relative_path: &str) -> io::Result<CapabilityReceipt> {
        if !self
            .token
            .capabilities
            .iter()
            .any(|item| item == "os.verify_artifact")
        {
            return Ok(self.blocked_receipt("os.verify_artifact", "os.verify_artifact not granted"));
        }
        let path = self
            .guard
            .resolve_workspace_path(relative_path)
            .map_err(|err| io::Error::new(io::ErrorKind::PermissionDenied, err))?;
        let exists = path.exists();
        let size_bytes = if exists && path.is_file() {
            path.metadata()?.len()
        } else {
            0
        };
        let receipt = CapabilityReceipt {
            capability_id: "os.verify_artifact".to_string(),
            job_id: self.token.job_id.clone(),
            pid: self.token.pid.clone(),
            status: if exists { "success" } else { "failed" }.to_string(),
            data: json!({
                "artifact_path": relative_path,
                "exists": exists,
                "size_bytes": size_bytes,
            }),
        };
        self.emit_receipt("verify_event", &receipt)?;
        Ok(receipt)
    }

    fn copy_like(
        &self,
        capability_id: &str,
        source_path: &str,
        destination_path: &str,
        remove_source: bool,
    ) -> io::Result<CapabilityReceipt> {
        if let Some(receipt) = self.ensure_capability(capability_id) {
            return Ok(receipt);
        }
        let source = self.resolve_path(source_path)?;
        let destination = self.resolve_path(destination_path)?;
        if !source.exists() {
            return Ok(self.blocked_receipt(capability_id, "source path does not exist"));
        }
        let normalized_source = source_path.replace('\\', "/");
        let normalized_destination = destination_path.replace('\\', "/");
        let _mutation_lock = match self.acquire_mutation_locks(
            capability_id,
            &[normalized_source.clone(), normalized_destination.clone()],
        )? {
            MutationLockResult::Acquired(locks) => locks,
            MutationLockResult::Blocked(receipt) => return Ok(receipt),
        };
        let tx_id = new_tx_id("os_mutation");
        let source_backup = self.backup_workspace_path(&tx_id, "source", &source)?;
        let destination_backup = self.backup_workspace_path(&tx_id, "destination", &destination)?;
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        if destination.exists() {
            remove_path_any(&destination)?;
        }
        copy_path_recursive(&source, &destination)?;
        if remove_source {
            remove_path_any(&source)?;
        }
        let tx = OsTxRecord {
            tx_id: tx_id.clone(),
            operation: capability_id.to_string(),
            source_path: Some(normalized_source.clone()),
            destination_path: Some(normalized_destination.clone()),
            write_kind: Some("source_mutation".to_string()),
            source_before_exists: source_backup.0,
            destination_before_exists: destination_backup.0,
            source_backup_ref: source_backup.1,
            destination_backup_ref: destination_backup.1,
        };
        let tx_ref = self.record_tx(&tx)?;
        let receipt = CapabilityReceipt {
            capability_id: capability_id.to_string(),
            job_id: self.token.job_id.clone(),
            pid: self.token.pid.clone(),
            status: "success".to_string(),
            data: json!({
                "source_path": normalized_source,
                "destination_path": normalized_destination,
                "tx_id": tx_id,
                "tx_ref": tx_ref,
            }),
        };
        self.emit_receipt("capability_receipt", &receipt)?;
        Ok(receipt)
    }

    fn restore_tx_record(&self, record: &OsTxRecord) -> io::Result<()> {
        if let Some(destination_path) = &record.destination_path {
            let destination = self.resolve_path(destination_path)?;
            if destination.exists() {
                remove_path_any(&destination)?;
            }
            if record.destination_before_exists {
                if let Some(backup_ref) = &record.destination_backup_ref {
                    restore_tx_backup(backup_ref, &self.truth, &destination)?;
                }
            }
        }
        if let Some(source_path) = &record.source_path {
            let source = self.resolve_path(source_path)?;
            if source.exists() {
                remove_path_any(&source)?;
            }
            if record.source_before_exists {
                if let Some(backup_ref) = &record.source_backup_ref {
                    restore_tx_backup(backup_ref, &self.truth, &source)?;
                }
            }
        }
        Ok(())
    }

    fn backup_workspace_path(
        &self,
        tx_id: &str,
        slot: &str,
        path: &Path,
    ) -> io::Result<(bool, Option<String>)> {
        if !path.exists() {
            return Ok((false, None));
        }
        let backup_root = self
            .truth
            .state_root()
            .join("tx_backups")
            .join(&self.token.job_id)
            .join(tx_id);
        fs::create_dir_all(&backup_root)?;
        let backup_path = backup_root.join(slot);
        if backup_path.exists() {
            remove_path_any(&backup_path)?;
        }
        copy_path_recursive(path, &backup_path)?;
        Ok((
            true,
            Some(format!(
                "txbackup://{}/{}/{}",
                self.token.job_id,
                tx_id,
                slot.replace('\\', "/")
            )),
        ))
    }

    fn record_tx(&self, record: &OsTxRecord) -> io::Result<String> {
        let tx_ref = self.truth.write_blob(
            &format!("tx/{}.json", safe_blob_name(&record.tx_id)),
            &serde_json::to_vec(record).map_err(json_err)?,
        )?;
        if self.emit_events {
            self.truth.append_event(
                Some(&self.token.pid),
                "tx_recorded",
                json!({
                    "tx_id": record.tx_id,
                    "operation": record.operation,
                    "tx_ref": tx_ref,
                }),
            )?;
        }
        Ok(tx_ref)
    }

    fn acquire_mutation_locks(
        &self,
        capability_id: &str,
        normalized_paths: &[String],
    ) -> io::Result<MutationLockResult> {
        let mut lock_paths = normalized_paths.to_vec();
        lock_paths.sort();
        lock_paths.dedup();
        let mut locks = Vec::new();
        for normalized_path in &lock_paths {
            match WorkspaceMutationLock::try_acquire(
                self.truth.state_root(),
                &self.token.job_id,
                normalized_path,
            )? {
                Some(lock) => locks.push(lock),
                None => {
                    drop(locks);
                    return Ok(MutationLockResult::Blocked(self.blocked_receipt_with_data(
                        capability_id,
                        json!({
                            "reason": "workspace_file_lock_conflict",
                            "message": "target path is locked by another active run",
                            "target_paths": lock_paths,
                            "conflict_path": normalized_path,
                            "no_file_written": true,
                            "recoverable_by_task_agent": true,
                        }),
                    )));
                }
            }
        }
        Ok(MutationLockResult::Acquired(locks))
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

    fn resolve_path(&self, relative_path: &str) -> io::Result<PathBuf> {
        self.guard
            .resolve_workspace_path(relative_path)
            .map_err(|err| io::Error::new(io::ErrorKind::PermissionDenied, err))
    }

    fn emit_receipt(
        &self,
        event_type: &str,
        receipt: &CapabilityReceipt,
    ) -> io::Result<Option<ProcessEvent>> {
        if self.emit_events {
            self.truth
                .append_event(Some(&self.token.pid), event_type, to_json_value(receipt)?)
                .map(Some)
        } else {
            Ok(None)
        }
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
        if self.emit_events {
            let _ = self.truth.append_event(
                Some(&self.token.pid),
                "capability_blocked",
                to_json_value(&receipt).unwrap_or_else(|_| json!({"reason": "blocked"})),
            );
            let _ = self.truth.append_event(
                Some(&self.token.pid),
                "capability_receipt",
                to_json_value(&receipt).unwrap_or_else(|_| json!({"reason": "blocked"})),
            );
        }
        receipt
    }
}

enum MutationLockResult {
    Acquired(Vec<WorkspaceMutationLock>),
    Blocked(CapabilityReceipt),
}

#[derive(Debug)]
struct WorkspaceMutationLock {
    path: PathBuf,
}

impl WorkspaceMutationLock {
    fn try_acquire(
        state_root: &Path,
        job_id: &str,
        normalized_path: &str,
    ) -> io::Result<Option<Self>> {
        let lock_dir = state_root.join("workspace_file_locks");
        fs::create_dir_all(&lock_dir)?;
        let lock_path = lock_dir.join(format!("{}.lock", safe_blob_name(normalized_path)));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_path)
        {
            Ok(mut file) => {
                use std::io::Write;
                writeln!(
                    file,
                    "{}",
                    serde_json::to_string(&json!({
                        "schema_version": "supernova.workspace_file_lock.v1",
                        "job_id": job_id,
                        "path": normalized_path,
                    }))
                    .map_err(json_err)?
                )?;
                Ok(Some(Self { path: lock_path }))
            }
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => Ok(None),
            Err(err) => Err(err),
        }
    }
}

impl Drop for WorkspaceMutationLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn artifact_revision_count(truth: &ProcessTruthStore, artifact_path: &str) -> io::Result<usize> {
    let normalized = artifact_path.replace('\\', "/");
    Ok(truth
        .read_events()?
        .iter()
        .filter(|event| event.event_type == "artifact_version_tx")
        .filter(|event| {
            event
                .data
                .get("artifact_path")
                .and_then(Value::as_str)
                .is_some_and(|path| path.replace('\\', "/") == normalized)
        })
        .count())
}

fn looks_like_temp_dataset_path(path: &str) -> bool {
    let value = path.to_ascii_lowercase();
    value.contains("tmp/")
        || value.contains("temp/")
        || value.contains("dataset")
        || value.ends_with(".jsonl")
}

fn collect_workspace_inventory(
    root: &Path,
    current: &Path,
    depth: usize,
    max_depth: usize,
    entries: &mut Vec<WorkspaceInventoryEntry>,
) -> io::Result<()> {
    if depth > max_depth {
        return Ok(());
    }
    let mut children = Vec::new();
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_name().to_string_lossy() == RUNTIME_DIR_NAME {
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
        let metadata = path.metadata()?;
        let kind = if path.is_dir() { "dir" } else { "file" }.to_string();
        let extension = if path.is_file() {
            path.extension()
                .and_then(|value| value.to_str())
                .map(|value| format!(".{}", value.to_ascii_lowercase()))
                .unwrap_or_else(|| "".to_string())
        } else {
            "".to_string()
        };
        let top_level = rel.split('/').next().unwrap_or("").to_string();
        let readable_document = is_readable_document_extension(&extension);
        let document_type = document_type_for_extension(&extension).to_string();
        let title = path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("")
            .to_string();
        let purpose_hint = purpose_hint_for_path(&rel);
        entries.push(WorkspaceInventoryEntry {
            path: rel.clone(),
            kind,
            top_level,
            extension,
            size_bytes: if path.is_file() { metadata.len() } else { 0 },
            readable_document,
            document_type,
            title,
            purpose_hint,
        });
        if path.is_dir() {
            collect_workspace_inventory(root, &path, depth + 1, max_depth, entries)?;
        }
    }
    Ok(())
}

fn is_readable_document_extension(extension: &str) -> bool {
    matches!(
        extension,
        ".md" | ".txt" | ".json" | ".csv" | ".log" | ".yaml" | ".yml" | ".docx"
    )
}

fn document_type_for_extension(extension: &str) -> &'static str {
    match extension {
        ".md" => "Markdown",
        ".txt" => "Text",
        ".json" => "JSON",
        ".csv" => "CSV",
        ".log" => "Log",
        ".yaml" | ".yml" => "YAML",
        ".docx" => "DOCX",
        ".xlsx" => "XLSX",
        ".pdf" => "PDF",
        ".png" | ".jpg" | ".jpeg" => "Image",
        ".zip" => "Archive",
        ".bin" | ".dat" => "Binary",
        "" => "Directory",
        _ => "File",
    }
}

fn purpose_hint_for_path(path: &str) -> String {
    let lower = path.to_ascii_lowercase();
    if path.contains("会议") || lower.contains("meeting") {
        "会议纪要、行动项或沟通记录".to_string()
    } else if path.contains("风险") || lower.contains("risk") {
        "风险识别、处置跟踪或复盘材料".to_string()
    } else if path.contains("反馈") || lower.contains("feedback") {
        "客户反馈、回访或问题闭环材料".to_string()
    } else if path.contains("交付") || path.contains("发布") || lower.contains("deliverable") {
        "交付说明、发布说明或运维培训材料".to_string()
    } else if path.contains("草稿") || path.contains("终稿") || lower.contains("draft") {
        "文稿版本、修订稿或待定稿材料".to_string()
    } else if path.contains("简报") || path.contains("通知") || lower.contains("official") {
        "正式公文、简报或通报材料".to_string()
    } else if lower.contains("inbox") || path.contains("待整理") {
        "待整理收件箱材料或混杂附件".to_string()
    } else if lower.contains("tmp") || path.contains("临时") {
        "临时运行数据或缓存材料".to_string()
    } else if lower.contains("private") || path.contains("敏感") {
        "私有或敏感资料".to_string()
    } else if path.contains("项目") || lower.contains("project") {
        "项目资料、源材料或阶段记录".to_string()
    } else {
        "工作区资料".to_string()
    }
}

fn document_index_csv(entries: &[WorkspaceInventoryEntry]) -> String {
    let mut out = String::from("source_path,type,title,purpose\n");
    for entry in entries {
        out.push_str(&csv_escape(&entry.path));
        out.push(',');
        out.push_str(&csv_escape(&entry.document_type));
        out.push(',');
        out.push_str(&csv_escape(&entry.title));
        out.push(',');
        out.push_str(&csv_escape(&entry.purpose_hint));
        out.push('\n');
    }
    out
}

#[derive(Default)]
struct TopLevelSummary {
    entry_count: usize,
    readable_documents: usize,
    total_size_bytes: u64,
    purpose_hints: Vec<String>,
    examples: Vec<String>,
}

fn workspace_map_markdown(
    entries: &[WorkspaceInventoryEntry],
    extension_counts: &BTreeMap<String, usize>,
) -> String {
    let entry_count = entries.len();
    let document_count = entries.iter().filter(|item| item.readable_document).count();
    let total_size_bytes = entries.iter().map(|item| item.size_bytes).sum::<u64>();
    let mut top_level_summaries: BTreeMap<String, TopLevelSummary> = BTreeMap::new();
    for entry in entries {
        let key = if entry.top_level.is_empty() {
            "<root>".to_string()
        } else {
            entry.top_level.clone()
        };
        let summary = top_level_summaries.entry(key).or_default();
        summary.entry_count += 1;
        summary.total_size_bytes += entry.size_bytes;
        if entry.readable_document {
            summary.readable_documents += 1;
        }
        if !summary
            .purpose_hints
            .iter()
            .any(|item| item == &entry.purpose_hint)
        {
            summary.purpose_hints.push(entry.purpose_hint.clone());
        }
        if entry.kind == "file" && summary.examples.len() < 8 {
            summary.examples.push(entry.path.clone());
        }
    }
    let mut out = String::new();
    out.push_str("# WORKSPACE_MAP\n\n");
    out.push_str("## Overview\n\n");
    out.push_str(&format!("- entries: {entry_count}\n"));
    out.push_str(&format!("- readable_documents: {document_count}\n"));
    out.push_str(&format!("- total_size_bytes: {total_size_bytes}\n\n"));
    out.push_str("## Top Level Areas\n\n");
    for (name, summary) in top_level_summaries {
        out.push_str(&format!("### `{name}`\n\n"));
        out.push_str(&format!("- entries: {}\n", summary.entry_count));
        out.push_str(&format!(
            "- readable_documents: {}\n",
            summary.readable_documents
        ));
        out.push_str(&format!(
            "- total_size_bytes: {}\n",
            summary.total_size_bytes
        ));
        if !summary.purpose_hints.is_empty() {
            out.push_str("- purpose_hints:\n");
            for hint in summary.purpose_hints.iter().take(4) {
                out.push_str(&format!("  - {hint}\n"));
            }
        }
        if !summary.examples.is_empty() {
            out.push_str("- representative_source_paths:\n");
            for path in summary.examples.iter().take(8) {
                out.push_str(&format!("  - `{path}`\n"));
            }
        }
        out.push('\n');
    }
    out.push_str("\n## File Types\n\n");
    for (extension, count) in extension_counts {
        let label = if extension.is_empty() {
            "<dir>"
        } else {
            extension
        };
        out.push_str(&format!("- `{label}`: {count}\n"));
    }
    out
}

fn csv_escape(value: &str) -> String {
    if value.contains(',') || value.contains('"') || value.contains('\n') || value.contains('\r') {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;
    use crate::{create_agent_job_with_state_root, CapabilityToken};

    fn temp_workspace(name: &str) -> (PathBuf, PathBuf) {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "supernova_os_runtime_{name}_{}_{}",
            std::process::id(),
            suffix
        ));
        let workspace = root.join("workspace");
        let state = root.join("state");
        fs::create_dir_all(&workspace).unwrap();
        fs::create_dir_all(&state).unwrap();
        (workspace, state)
    }

    fn runtime_with_capabilities(
        workspace: &Path,
        state: &Path,
        capabilities: &[&str],
    ) -> OsRuntime {
        let (job, process, truth) =
            create_agent_job_with_state_root(workspace, state, "OS mutation lock").unwrap();
        let token = CapabilityToken {
            token_id: "token_os_lock".to_string(),
            job_id: job.job_id,
            pid: process.pid,
            workspace_root: workspace.display().to_string(),
            capabilities: capabilities
                .iter()
                .map(|capability| capability.to_string())
                .collect(),
            permissions: vec!["fs:write".to_string()],
        };
        OsRuntime::new(WorkspaceGuard::new(workspace).unwrap(), truth, token)
    }

    #[test]
    fn write_artifact_blocks_when_target_lock_exists() {
        let (workspace, state) = temp_workspace("write_conflict");
        fs::create_dir_all(workspace.join("reports")).unwrap();
        fs::write(workspace.join("reports/out.txt"), "original").unwrap();
        let runtime = runtime_with_capabilities(&workspace, &state, &["os.write_artifact"]);
        let _held_lock = WorkspaceMutationLock::try_acquire(&state, "other_job", "reports/out.txt")
            .unwrap()
            .expect("lock should be acquired");

        let receipt = runtime
            .write_artifact("reports/out.txt", b"changed")
            .unwrap();

        assert_eq!(receipt.status, "blocked");
        assert_eq!(receipt.data["reason"], "workspace_file_lock_conflict");
        assert_eq!(receipt.data["conflict_path"], "reports/out.txt");
        assert_eq!(receipt.data["no_file_written"], true);
        assert_eq!(
            fs::read_to_string(workspace.join("reports/out.txt")).unwrap(),
            "original"
        );
        fs::remove_dir_all(workspace.parent().unwrap()).unwrap();
    }

    #[test]
    fn write_artifact_releases_lock_after_success() {
        let (workspace, state) = temp_workspace("write_release");
        let runtime = runtime_with_capabilities(&workspace, &state, &["os.write_artifact"]);

        let first = runtime.write_artifact("reports/out.txt", b"first").unwrap();
        let second = runtime
            .write_artifact("reports/out.txt", b"second")
            .unwrap();

        assert_eq!(first.status, "success");
        assert_eq!(second.status, "success");
        assert_eq!(
            fs::read_to_string(workspace.join("reports/out.txt")).unwrap(),
            "second"
        );
        fs::remove_dir_all(workspace.parent().unwrap()).unwrap();
    }

    #[test]
    fn delete_path_blocks_when_target_lock_exists() {
        let (workspace, state) = temp_workspace("delete_conflict");
        fs::write(workspace.join("target.txt"), "keep").unwrap();
        let runtime = runtime_with_capabilities(&workspace, &state, &["os.delete_path"]);
        let _held_lock = WorkspaceMutationLock::try_acquire(&state, "other_job", "target.txt")
            .unwrap()
            .expect("lock should be acquired");

        let receipt = runtime.delete_path("target.txt").unwrap();

        assert_eq!(receipt.status, "blocked");
        assert_eq!(receipt.data["reason"], "workspace_file_lock_conflict");
        assert!(workspace.join("target.txt").exists());
        assert_eq!(
            fs::read_to_string(workspace.join("target.txt")).unwrap(),
            "keep"
        );
        fs::remove_dir_all(workspace.parent().unwrap()).unwrap();
    }

    #[test]
    fn copy_path_blocks_when_destination_lock_exists() {
        let (workspace, state) = temp_workspace("copy_conflict");
        fs::write(workspace.join("source.txt"), "source").unwrap();
        fs::write(workspace.join("dest.txt"), "dest").unwrap();
        let runtime = runtime_with_capabilities(&workspace, &state, &["os.copy_path"]);
        let _held_lock = WorkspaceMutationLock::try_acquire(&state, "other_job", "dest.txt")
            .unwrap()
            .expect("lock should be acquired");

        let receipt = runtime.copy_path("source.txt", "dest.txt").unwrap();

        assert_eq!(receipt.status, "blocked");
        assert_eq!(receipt.data["reason"], "workspace_file_lock_conflict");
        assert_eq!(
            fs::read_to_string(workspace.join("source.txt")).unwrap(),
            "source"
        );
        assert_eq!(
            fs::read_to_string(workspace.join("dest.txt")).unwrap(),
            "dest"
        );
        fs::remove_dir_all(workspace.parent().unwrap()).unwrap();
    }
}
