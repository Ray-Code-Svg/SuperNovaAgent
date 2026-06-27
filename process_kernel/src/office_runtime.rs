use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;
use serde_json::{json, Value};

use crate::{
    child_process::suppress_child_window, copy_path_recursive, json_err, new_tx_id, now_ms,
    path_string, remove_path_any, safe_blob_name, to_json_value, CapabilityReceipt,
    CapabilityToken, OsTxRecord, ProcessTruthStore, SourceSet, WorkspaceGuard,
};
#[derive(Clone, Debug)]
pub struct OfficeRuntime {
    guard: WorkspaceGuard,
    truth: ProcessTruthStore,
    token: CapabilityToken,
    worker_project: PathBuf,
    emit_events: bool,
}

#[derive(Clone, Debug, Deserialize)]
struct OfficeWorkerReceipt {
    #[serde(rename = "CapabilityId")]
    capability_id: String,
    #[serde(rename = "Status")]
    status: String,
    #[serde(rename = "Data")]
    data: Value,
}

impl OfficeRuntime {
    pub fn new(
        guard: WorkspaceGuard,
        truth: ProcessTruthStore,
        token: CapabilityToken,
        worker_project: impl AsRef<Path>,
    ) -> Self {
        Self {
            guard,
            truth,
            token,
            worker_project: worker_project.as_ref().to_path_buf(),
            emit_events: true,
        }
    }

    pub fn without_process_truth_events(mut self) -> Self {
        self.emit_events = false;
        self
    }

    pub fn read_text(&self, input_path: &str) -> io::Result<CapabilityReceipt> {
        let input = self.resolve_path(input_path)?;
        let mut receipt = self.run_worker_receipt(
            "office.docx.read_text",
            vec![
                "read-text".to_string(),
                "--input".to_string(),
                path_string(&input),
            ],
            None,
        )?;
        if receipt.status == "success" {
            let extracted_text = receipt
                .data
                .get("worker_receipt")
                .and_then(|value| value.get("data"))
                .and_then(|value| value.get("text"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_string);
            if let Some(text) = extracted_text {
                let content_ref = self.truth.write_blob(
                    &format!(
                        "office_text/{}_{}.txt",
                        safe_blob_name(input_path),
                        now_ms()
                    ),
                    text.as_bytes(),
                )?;
                if let Some(data) = receipt.data.as_object_mut() {
                    data.insert("content_ref".to_string(), serde_json::json!(content_ref));
                    data.insert(
                        "char_count".to_string(),
                        serde_json::json!(text.chars().count()),
                    );
                }
            }
        }
        Ok(receipt)
    }

    pub fn read_workbook_text(
        &self,
        input_path: &str,
        sheet: Option<&str>,
        max_rows: usize,
    ) -> io::Result<CapabilityReceipt> {
        self.read_workbook(
            input_path,
            sheet,
            max_rows,
            "office.workbook.read_text",
            "read-workbook-text",
        )
    }

    pub fn read_workbook_cells(
        &self,
        input_path: &str,
        sheet: Option<&str>,
        max_rows: usize,
    ) -> io::Result<CapabilityReceipt> {
        self.read_workbook(
            input_path,
            sheet,
            max_rows,
            "office.workbook.read_cells",
            "read-workbook-cells",
        )
    }

    fn read_workbook(
        &self,
        input_path: &str,
        sheet: Option<&str>,
        max_rows: usize,
        capability_id: &str,
        worker_command: &str,
    ) -> io::Result<CapabilityReceipt> {
        let input = self.resolve_path(input_path)?;
        let mut args = vec![
            worker_command.to_string(),
            "--input".to_string(),
            path_string(&input),
            "--max-rows".to_string(),
            max_rows.max(1).min(10_000).to_string(),
        ];
        if let Some(sheet) = sheet.map(str::trim).filter(|value| !value.is_empty()) {
            args.push("--sheet".to_string());
            args.push(sheet.to_string());
        }
        let mut receipt = self.run_worker_receipt(capability_id, args, None)?;
        if receipt.status == "success" {
            let worker_data = receipt
                .data
                .get("worker_receipt")
                .and_then(|value| value.get("data"))
                .cloned()
                .unwrap_or_else(|| json!({}));
            if let Some(text) = worker_data.get("text").and_then(Value::as_str) {
                let content_ref = self.truth.write_blob(
                    &format!(
                        "office_workbook/{}_{}_text.txt",
                        safe_blob_name(input_path),
                        now_ms()
                    ),
                    text.as_bytes(),
                )?;
                if let Some(data) = receipt.data.as_object_mut() {
                    data.insert("content_ref".to_string(), json!(content_ref));
                    data.insert("char_count".to_string(), json!(text.chars().count()));
                }
            }
            if let Some(cells) = worker_data.get("cells") {
                let cells_ref = self.truth.write_blob(
                    &format!(
                        "office_workbook/{}_{}_cells.json",
                        safe_blob_name(input_path),
                        now_ms()
                    ),
                    &serde_json::to_vec_pretty(cells).map_err(json_err)?,
                )?;
                if let Some(data) = receipt.data.as_object_mut() {
                    data.insert("cells_ref".to_string(), json!(cells_ref));
                    data.insert(
                        "cell_count".to_string(),
                        worker_data
                            .get("cell_count")
                            .cloned()
                            .unwrap_or_else(|| json!(0)),
                    );
                }
            }
        }
        Ok(receipt)
    }

    pub fn batch_read_text(&self, source_set_ref: &str) -> io::Result<CapabilityReceipt> {
        let source_set = self.read_source_set(source_set_ref)?;
        let docx_paths = self.docx_paths_from_source_set(&source_set)?;
        let input_list = docx_paths
            .iter()
            .map(|(_, path)| path_string(path))
            .collect::<Vec<_>>()
            .join("\n");
        let input_list_ref = self.truth.write_blob(
            &format!("office_inputs/batch_docx_list_{}.txt", now_ms()),
            input_list.as_bytes(),
        )?;
        let input_list_path = self.truth.resolve_blob_ref(&input_list_ref)?;
        let mut receipt = self.run_worker_receipt(
            "office.docx.batch_read_text",
            vec![
                "batch-read-text".to_string(),
                "--input-list".to_string(),
                path_string(&input_list_path),
            ],
            None,
        )?;
        let raw_document_set = normalize_batch_raw_documents(
            self.guard.root(),
            source_set_ref,
            receipt
                .data
                .get("worker_receipt")
                .and_then(|value| value.get("data"))
                .cloned()
                .unwrap_or_else(|| json!({})),
        );
        let raw_document_set_ref = self.truth.write_blob(
            &format!("raw_document_sets/docx_batch_{}.json", now_ms()),
            &serde_json::to_vec_pretty(&raw_document_set).map_err(json_err)?,
        )?;
        if let Some(data) = receipt.data.as_object_mut() {
            data.insert("source_set_ref".to_string(), json!(source_set_ref));
            data.insert(
                "raw_document_set_ref".to_string(),
                json!(raw_document_set_ref),
            );
            data.insert("docx_count".to_string(), json!(docx_paths.len()));
            data.insert("derivation_type".to_string(), json!("raw"));
            data.insert("source_refs_required".to_string(), json!(true));
            data.insert("coverage_required".to_string(), json!(true));
        }
        if self.emit_events {
            self.truth.append_event(
                Some(&self.token.pid),
                "raw_document_set_created",
                json!({
                    "source_set_ref": source_set_ref,
                    "raw_document_set_ref": receipt.data.get("raw_document_set_ref").cloned().unwrap_or(Value::Null),
                    "docx_count": docx_paths.len(),
                }),
            )?;
            self.truth.append_event(
                Some(&self.token.pid),
                "capability_receipt",
                to_json_value(&receipt)?,
            )?;
        }
        Ok(receipt)
    }

    pub fn batch_extract_metadata(&self, source_set_ref: &str) -> io::Result<CapabilityReceipt> {
        if !self
            .token
            .capabilities
            .iter()
            .any(|item| item == "office.docx.batch_extract_metadata")
        {
            return Ok(self.blocked_receipt(
                "office.docx.batch_extract_metadata",
                "office.docx.batch_extract_metadata not granted",
            ));
        }
        let source_set = self.read_source_set(source_set_ref)?;
        let docx_paths = self.docx_paths_from_source_set(&source_set)?;
        let mut records = Vec::new();
        let mut errors = Vec::new();
        for (relative_path, absolute_path) in &docx_paths {
            let receipt = self.run_worker_receipt(
                "office.docx.batch_extract_metadata",
                vec![
                    "read-text".to_string(),
                    "--input".to_string(),
                    path_string(absolute_path),
                ],
                None,
            )?;
            let worker_data = receipt
                .data
                .get("worker_receipt")
                .and_then(|value| value.get("data"))
                .cloned()
                .unwrap_or_else(|| json!({}));
            if receipt.status == "success" {
                let text = worker_data
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                records.push(json!({
                    "source_path": relative_path,
                    "title": Path::new(relative_path).file_stem().and_then(|value| value.to_str()).unwrap_or(""),
                    "char_count": text.chars().count(),
                    "paragraph_count": text.lines().filter(|line| !line.trim().is_empty()).count(),
                    "size_bytes": absolute_path.metadata().map(|meta| meta.len()).unwrap_or(0),
                    "valid_read_text": true,
                }));
            } else {
                errors.push(json!({
                    "source_path": relative_path,
                    "status": receipt.status,
                    "worker_receipt": worker_data,
                }));
            }
        }
        let dataset = json!({
            "dataset_id": format!("docx_metadata_{}", now_ms()),
            "schema": ["source_path", "title", "char_count", "paragraph_count", "size_bytes", "valid_read_text"],
            "row_count": records.len(),
            "source_set_ref": source_set_ref,
            "derivation_type": "metadata",
            "records": records,
            "coverage_report": {
                "docx_count": docx_paths.len(),
                "succeeded_files": records.len(),
                "failed_files": errors.len(),
                "errors": errors,
            }
        });
        let dataset_ref = self.truth.write_blob(
            &format!("datasets/docx_metadata_{}.json", now_ms()),
            &serde_json::to_vec_pretty(&dataset).map_err(json_err)?,
        )?;
        let receipt = CapabilityReceipt {
            capability_id: "office.docx.batch_extract_metadata".to_string(),
            job_id: self.token.job_id.clone(),
            pid: self.token.pid.clone(),
            status: if errors.is_empty() {
                "success"
            } else {
                "failed"
            }
            .to_string(),
            data: json!({
                "source_set_ref": source_set_ref,
                "dataset_ref": dataset_ref,
                "docx_count": docx_paths.len(),
                "succeeded_files": records.len(),
                "failed_files": errors.len(),
                "errors": errors,
                "derivation_type": "metadata",
                "source_refs_required": true,
                "coverage_required": true,
            }),
        };
        if self.emit_events {
            self.truth.append_event(
                Some(&self.token.pid),
                "office_receipt",
                to_json_value(&receipt)?,
            )?;
            self.truth.append_event(
                Some(&self.token.pid),
                "capability_receipt",
                to_json_value(&receipt)?,
            )?;
        }
        Ok(receipt)
    }

    pub fn batch_validate(&self, source_set_ref: &str) -> io::Result<CapabilityReceipt> {
        if !self
            .token
            .capabilities
            .iter()
            .any(|item| item == "office.docx.batch_validate")
        {
            return Ok(self.blocked_receipt(
                "office.docx.batch_validate",
                "office.docx.batch_validate not granted",
            ));
        }
        let source_set = self.read_source_set(source_set_ref)?;
        let docx_paths = self.docx_paths_from_source_set(&source_set)?;
        let mut results = Vec::new();
        let mut failed = 0usize;
        for (relative_path, absolute_path) in &docx_paths {
            let receipt = self.run_worker_receipt(
                "office.docx.batch_validate",
                vec![
                    "validate".to_string(),
                    "--input".to_string(),
                    path_string(absolute_path),
                ],
                None,
            )?;
            if receipt.status != "success" {
                failed += 1;
            }
            results.push(json!({
                "source_path": relative_path,
                "status": receipt.status,
                "worker_receipt": receipt.data.get("worker_receipt").cloned().unwrap_or(Value::Null),
            }));
        }
        let validation_ref = self.truth.write_blob(
            &format!("office_validation/docx_batch_validate_{}.json", now_ms()),
            &serde_json::to_vec_pretty(&json!({
                "source_set_ref": source_set_ref,
                "docx_count": docx_paths.len(),
                "succeeded_files": docx_paths.len().saturating_sub(failed),
                "failed_files": failed,
                "results": results,
            }))
            .map_err(json_err)?,
        )?;
        let receipt = CapabilityReceipt {
            capability_id: "office.docx.batch_validate".to_string(),
            job_id: self.token.job_id.clone(),
            pid: self.token.pid.clone(),
            status: if failed == 0 { "success" } else { "failed" }.to_string(),
            data: json!({
                "source_set_ref": source_set_ref,
                "validation_ref": validation_ref,
                "docx_count": docx_paths.len(),
                "succeeded_files": docx_paths.len().saturating_sub(failed),
                "failed_files": failed,
                "coverage_ratio": if docx_paths.is_empty() { 1.0 } else { (docx_paths.len().saturating_sub(failed)) as f64 / docx_paths.len() as f64 },
            }),
        };
        if self.emit_events {
            self.truth.append_event(
                Some(&self.token.pid),
                "office_receipt",
                to_json_value(&receipt)?,
            )?;
            self.truth.append_event(
                Some(&self.token.pid),
                "capability_receipt",
                to_json_value(&receipt)?,
            )?;
        }
        Ok(receipt)
    }

    pub fn create_docx(
        &self,
        output_path: &str,
        text: &str,
        title: Option<&str>,
    ) -> io::Result<CapabilityReceipt> {
        let output = self.resolve_path(output_path)?;
        let text_path = self.write_text_input("create_docx", text)?;
        let mut args = vec![
            "create-docx".to_string(),
            "--output".to_string(),
            path_string(&output),
            "--text-file".to_string(),
            path_string(&text_path),
        ];
        if let Some(title) = title {
            args.push("--title".to_string());
            args.push(title.to_string());
        }
        let tx_id = new_tx_id("office_create");
        let destination_backup = self.backup_workspace_path(&tx_id, "destination", &output)?;
        self.run_worker_receipt(
            "office.docx.create",
            args,
            Some(OfficeTxPlan {
                tx_id,
                operation: "office.docx.create".to_string(),
                source_path: None,
                destination_path: Some(output_path.replace('\\', "/")),
                write_kind: "artifact".to_string(),
                source_before_exists: false,
                destination_before_exists: destination_backup.0,
                source_backup_ref: None,
                destination_backup_ref: destination_backup.1,
            }),
        )
    }

    pub fn rewrite_save_as(
        &self,
        input_path: &str,
        output_path: &str,
        text: &str,
    ) -> io::Result<CapabilityReceipt> {
        let input = self.resolve_path(input_path)?;
        let output = self.resolve_path(output_path)?;
        let text_path = self.write_text_input("rewrite_save_as", text)?;
        let tx_id = new_tx_id("office_rewrite_save_as");
        let destination_backup = self.backup_workspace_path(&tx_id, "destination", &output)?;
        self.run_worker_receipt(
            "office.docx.rewrite_save_as",
            vec![
                "rewrite-save-as".to_string(),
                "--input".to_string(),
                path_string(&input),
                "--output".to_string(),
                path_string(&output),
                "--text-file".to_string(),
                path_string(&text_path),
            ],
            Some(OfficeTxPlan {
                tx_id,
                operation: "office.docx.rewrite_save_as".to_string(),
                source_path: Some(input_path.replace('\\', "/")),
                destination_path: Some(output_path.replace('\\', "/")),
                write_kind: "artifact".to_string(),
                source_before_exists: true,
                destination_before_exists: destination_backup.0,
                source_backup_ref: None,
                destination_backup_ref: destination_backup.1,
            }),
        )
    }

    pub fn preview_rewrite(&self, input_path: &str, text: &str) -> io::Result<CapabilityReceipt> {
        let input = self.resolve_path(input_path)?;
        let text_path = self.write_text_input("preview_rewrite", text)?;
        self.run_worker_receipt(
            "office.docx.rewrite_preview",
            vec![
                "preview-rewrite".to_string(),
                "--input".to_string(),
                path_string(&input),
                "--text-file".to_string(),
                path_string(&text_path),
            ],
            None,
        )
    }

    pub fn preview_in_place_rewrite(
        &self,
        input_path: &str,
        text: &str,
    ) -> io::Result<CapabilityReceipt> {
        let input = self.resolve_path(input_path)?;
        let text_path = self.write_text_input("preview_in_place", text)?;
        let mut receipt = self.run_worker_receipt(
            "office.docx.rewrite_in_place_preview",
            vec![
                "preview-in-place-rewrite".to_string(),
                "--input".to_string(),
                path_string(&input),
                "--text-file".to_string(),
                path_string(&text_path),
            ],
            None,
        )?;
        if let Some(data) = receipt.data.as_object_mut() {
            data.insert(
                "proposed_actions".to_string(),
                json!(["office.docx.rewrite_in_place"]),
            );
            data.insert(
                "target_paths".to_string(),
                json!([input_path.replace('\\', "/")]),
            );
            data.insert("requires_approval".to_string(), json!(true));
        }
        Ok(receipt)
    }

    pub fn rewrite_in_place(&self, input_path: &str, text: &str) -> io::Result<CapabilityReceipt> {
        self.rewrite_in_place_with_approval(input_path, text, None)
    }

    pub fn rewrite_in_place_with_approval(
        &self,
        input_path: &str,
        text: &str,
        _approval_id: Option<&str>,
    ) -> io::Result<CapabilityReceipt> {
        let input = self.resolve_path(input_path)?;
        let text_path = self.write_text_input("rewrite_in_place", text)?;
        let tx_id = new_tx_id("office_rewrite_in_place");
        let source_backup = self.backup_workspace_path(&tx_id, "source", &input)?;
        let receipt = self.run_worker_receipt(
            "office.docx.rewrite_in_place",
            vec![
                "rewrite-in-place".to_string(),
                "--input".to_string(),
                path_string(&input),
                "--text-file".to_string(),
                path_string(&text_path),
            ],
            Some(OfficeTxPlan {
                tx_id,
                operation: "office.docx.rewrite_in_place".to_string(),
                source_path: Some(input_path.replace('\\', "/")),
                destination_path: None,
                write_kind: "source_mutation".to_string(),
                source_before_exists: source_backup.0,
                destination_before_exists: false,
                source_backup_ref: source_backup.1,
                destination_backup_ref: None,
            }),
        )?;
        Ok(receipt)
    }

    pub fn diff_summary(
        &self,
        before_path: &str,
        after_path: &str,
    ) -> io::Result<CapabilityReceipt> {
        let before = self.resolve_path(before_path)?;
        let after = self.resolve_path(after_path)?;
        self.run_worker_receipt(
            "office.docx.diff_summary",
            vec![
                "diff-summary".to_string(),
                "--before".to_string(),
                path_string(&before),
                "--after".to_string(),
                path_string(&after),
            ],
            None,
        )
    }

    pub fn validate_docx(&self, input_path: &str) -> io::Result<CapabilityReceipt> {
        let input = self.resolve_path(input_path)?;
        self.run_worker_receipt(
            "office.docx.validate",
            vec![
                "validate".to_string(),
                "--input".to_string(),
                path_string(&input),
            ],
            None,
        )
    }

    fn run_worker_receipt(
        &self,
        capability_id: &str,
        worker_args: Vec<String>,
        tx_plan: Option<OfficeTxPlan>,
    ) -> io::Result<CapabilityReceipt> {
        if !self
            .token
            .capabilities
            .iter()
            .any(|item| item == capability_id)
        {
            return Ok(self.blocked_receipt(capability_id, &format!("{capability_id} not granted")));
        }
        let mut command = Command::new("dotnet");
        command.arg("run");
        command.arg("--project");
        command.arg(&self.worker_project);
        command.arg("--no-build");
        command.arg("--");
        command.args(&worker_args);
        command.current_dir(self.guard.root());
        command.env("DOTNET_CLI_TELEMETRY_OPTOUT", "1");
        suppress_child_window(&mut command);
        let output = command.output()?;
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let worker_receipt_ref = self.truth.write_blob(
            &format!(
                "office_receipts/{}_{}.json",
                safe_blob_name(capability_id),
                now_ms()
            ),
            stdout.as_bytes(),
        )?;
        let parsed = serde_json::from_str::<OfficeWorkerReceipt>(&stdout);
        let (status, worker_receipt_value) = match parsed {
            Ok(worker_receipt) => {
                let value = json!({
                    "capability_id": worker_receipt.capability_id,
                    "status": worker_receipt.status,
                    "data": worker_receipt.data,
                });
                let status = if output.status.success()
                    && value.get("status").and_then(Value::as_str) == Some("success")
                {
                    "success"
                } else {
                    "failed"
                };
                (status.to_string(), value)
            }
            Err(err) => (
                "failed".to_string(),
                json!({
                    "parse_error": err.to_string(),
                    "stdout": stdout,
                    "stderr": stderr,
                    "exit_code": output.status.code(),
                }),
            ),
        };

        let tx_ref = if let Some(plan) = &tx_plan {
            Some(self.record_tx_plan(plan)?)
        } else {
            None
        };
        let receipt = CapabilityReceipt {
            capability_id: capability_id.to_string(),
            job_id: self.token.job_id.clone(),
            pid: self.token.pid.clone(),
            status,
            data: json!({
                "worker_project": self.worker_project.display().to_string(),
                "worker_args": worker_args,
                "worker_receipt_ref": worker_receipt_ref,
                "worker_receipt": worker_receipt_value,
                "tx_id": tx_plan.as_ref().map(|plan| plan.tx_id.clone()),
                "tx_ref": tx_ref,
                "stdout_bytes": output.stdout.len(),
                "stderr_bytes": output.stderr.len(),
                "exit_code": output.status.code(),
            }),
        };
        if self.emit_events {
            self.truth.append_event(
                Some(&self.token.pid),
                "office_receipt",
                to_json_value(&receipt)?,
            )?;
            self.truth.append_event(
                Some(&self.token.pid),
                "capability_receipt",
                to_json_value(&receipt)?,
            )?;
        }
        Ok(receipt)
    }

    fn write_text_input(&self, name: &str, text: &str) -> io::Result<PathBuf> {
        let input_ref = self.truth.write_blob(
            &format!("office_inputs/{}_{}.txt", safe_blob_name(name), now_ms()),
            text.as_bytes(),
        )?;
        self.truth.resolve_blob_ref(&input_ref)
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

    fn record_tx_plan(&self, plan: &OfficeTxPlan) -> io::Result<String> {
        let record = OsTxRecord {
            tx_id: plan.tx_id.clone(),
            operation: plan.operation.clone(),
            source_path: plan.source_path.clone(),
            destination_path: plan.destination_path.clone(),
            write_kind: Some(plan.write_kind.clone()),
            source_before_exists: plan.source_before_exists,
            destination_before_exists: plan.destination_before_exists,
            source_backup_ref: plan.source_backup_ref.clone(),
            destination_backup_ref: plan.destination_backup_ref.clone(),
        };
        let tx_ref = self.truth.write_blob(
            &format!("tx/{}.json", safe_blob_name(&record.tx_id)),
            &serde_json::to_vec(&record).map_err(json_err)?,
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

    fn resolve_path(&self, relative_path: &str) -> io::Result<PathBuf> {
        self.guard
            .resolve_workspace_path(relative_path)
            .map_err(|err| io::Error::new(io::ErrorKind::PermissionDenied, err))
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
                "office_blocked",
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

    fn read_source_set(&self, source_set_ref: &str) -> io::Result<SourceSet> {
        let path = self.truth.resolve_blob_ref(source_set_ref)?;
        serde_json::from_str(&fs::read_to_string(path)?).map_err(json_err)
    }

    fn docx_paths_from_source_set(
        &self,
        source_set: &SourceSet,
    ) -> io::Result<Vec<(String, PathBuf)>> {
        source_set
            .files
            .iter()
            .filter(|file| file.extension.eq_ignore_ascii_case(".docx"))
            .map(|file| {
                self.resolve_path(&file.path)
                    .map(|path| (file.path.clone(), path))
            })
            .collect::<io::Result<Vec<_>>>()
    }
}

#[derive(Clone, Debug)]
struct OfficeTxPlan {
    tx_id: String,
    operation: String,
    source_path: Option<String>,
    destination_path: Option<String>,
    write_kind: String,
    source_before_exists: bool,
    destination_before_exists: bool,
    source_backup_ref: Option<String>,
    destination_backup_ref: Option<String>,
}

fn normalize_batch_raw_documents(root: &Path, source_set_ref: &str, worker_data: Value) -> Value {
    let documents = worker_data
        .get("documents")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|mut item| {
            if let Some(map) = item.as_object_mut() {
                if let Some(path) = map.get("source_path").and_then(Value::as_str) {
                    let relative = Path::new(path)
                        .strip_prefix(root)
                        .ok()
                        .map(|value| value.display().to_string().replace('\\', "/"))
                        .unwrap_or_else(|| path.replace('\\', "/"));
                    map.insert("source_path".to_string(), json!(relative));
                }
                map.insert("raw_text_ref".to_string(), Value::Null);
            }
            item
        })
        .collect::<Vec<_>>();
    json!({
        "source_set_ref": source_set_ref,
        "derivation_type": "raw",
        "is_lossless": false,
        "documents": documents,
        "total_files": worker_data.get("total_files").cloned().unwrap_or(Value::Null),
        "succeeded_files": worker_data.get("succeeded_files").cloned().unwrap_or(Value::Null),
        "failed_files": worker_data.get("failed_files").cloned().unwrap_or(Value::Null),
        "coverage_ratio": worker_data.get("coverage_ratio").cloned().unwrap_or(Value::Null),
        "errors": worker_data.get("errors").cloned().unwrap_or_else(|| json!([])),
    })
}
