use std::io;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::client_env_runtime::{ClientEnvRuntime, ClientEnvScanOptions};
use crate::model_config::estimate_text_tokens_conservative;
use crate::office_runtime::OfficeRuntime;
use crate::os_runtime::OsRuntime;
use crate::reasoning::NextActionDecision;
use crate::{
    json_err, now_ms, safe_blob_name, to_json_value, ArtifactRuntime, CapabilityReceipt,
    CapabilityToken, DataRuntime, ProcessTruthStore, WorkspaceGuard,
};

#[derive(Clone, Debug)]
pub struct ReadOnlyCapabilityExecutor {
    guard: WorkspaceGuard,
    truth: ProcessTruthStore,
    token: CapabilityToken,
    runtime_id: String,
    emit_capability_receipt_event: bool,
}

impl ReadOnlyCapabilityExecutor {
    pub fn new(
        guard: WorkspaceGuard,
        truth: ProcessTruthStore,
        token: CapabilityToken,
        runtime_id: impl Into<String>,
    ) -> Self {
        Self {
            guard,
            truth,
            token,
            runtime_id: runtime_id.into(),
            emit_capability_receipt_event: true,
        }
    }

    pub fn without_process_truth_receipt_events(mut self) -> Self {
        self.emit_capability_receipt_event = false;
        self
    }

    pub fn execute(&self, decision: &NextActionDecision) -> io::Result<CapabilityReceipt> {
        match decision.capability_id.as_str() {
            "os.list_tree" => self.os_runtime().list_tree(depth_arg(decision, 4)),
            "os.workspace_inventory" => self
                .os_runtime()
                .workspace_inventory(depth_arg(decision, 4)),
            "os.stat_path" => self
                .os_runtime()
                .stat_path(&path_arg(decision, "path")?),
            "os.read_file" => self
                .os_runtime()
                .read_file(&path_arg(decision, "path")?),
            "os.hash_path" => self
                .os_runtime()
                .hash_path(&path_arg(decision, "path")?),
            "os.diff" => self.os_runtime().diff_files(
                &path_arg(decision, "left_path")?,
                &path_arg(decision, "right_path")?,
            ),
            "os.verify_artifact" => self
                .os_runtime()
                .verify_artifact(&path_arg(decision, "path")?),
            "source_set.create" => self.data_runtime().create_source_set(
                decision
                    .output_spec
                    .get("root_path")
                    .and_then(Value::as_str)
                    .unwrap_or("."),
                &string_array_arg(decision, "include_extensions"),
                &string_array_arg(decision, "include_globs"),
                &string_array_arg(decision, "exclude_globs"),
                depth_arg(decision, 4),
            ),
            "source_set.read_page" => self.data_runtime().read_source_set_page(
                &string_arg(decision, "source_set_ref")?,
                usize_arg(decision, "offset", 0),
                usize_arg(decision, "limit", 100),
            ),
            "source_set.coverage_verify" => self
                .artifact_runtime()
                .source_set_coverage_verify(&string_arg(decision, "source_set_ref")?),
            "workspace.batch_hash" => self
                .data_runtime()
                .batch_hash(&string_arg(decision, "source_set_ref")?),
            "workspace.find_duplicates" => self
                .data_runtime()
                .find_duplicates(&string_arg(decision, "source_set_ref")?),
            "workspace.recent_changes" | "workspace.recent_changes_snapshot" => {
                self.data_runtime().recent_changes(
                    &string_arg(decision, "source_set_ref")?,
                    decision
                        .output_spec
                        .get("days")
                        .and_then(Value::as_u64)
                        .unwrap_or(7),
                )
            }
            "dataset.read_page" => self.read_dataset_page(decision),
            "dataset.coverage_verify" => self
                .artifact_runtime()
                .dataset_coverage_verify(&string_arg(decision, "dataset_ref")?),
            "artifact.inspect" => self.inspect_artifact(decision, "artifact.inspect"),
            "artifact.audit_readonly" => self.inspect_artifact(decision, "artifact.audit_readonly"),
            "client_env.scan_overview" => self
                .client_env_runtime()
                .scan_overview(ClientEnvScanOptions::from_value(&decision.output_spec)?),
            "client_env.scan_device" => self
                .client_env_runtime()
                .scan_device(ClientEnvScanOptions::from_value(&decision.output_spec)?),
            "client_env.scan_storage" => self
                .client_env_runtime()
                .scan_storage(ClientEnvScanOptions::from_value(&decision.output_spec)?),
            "client_env.scan_network" => self
                .client_env_runtime()
                .scan_network(ClientEnvScanOptions::from_value(&decision.output_spec)?),
            "client_env.scan_runtimes" => self
                .client_env_runtime()
                .scan_runtimes(ClientEnvScanOptions::from_value(&decision.output_spec)?),
            "client_env.read_snapshot" => self.client_env_runtime().read_snapshot(
                &string_arg(decision, "snapshot_ref").or_else(|_| string_arg(decision, "ref"))?,
                usize_arg(decision, "offset", 0),
                usize_arg(decision, "limit", 20),
            ),
            "client_env.request_sensitive_disclosure" => self
                .client_env_runtime()
                .request_sensitive_disclosure(
                    string_array_arg(decision, "requested_fields"),
                    decision
                        .output_spec
                        .get("reason")
                        .and_then(Value::as_str)
                        .unwrap_or(&decision.reason),
                ),
            "process.read_ref" => self.read_typed_ref(decision),
            "tool.result.page" => self.page_tool_result(decision),
            "tool.result.search" => self.search_tool_result(decision),
            "tool.result.inspect_schema" => self.inspect_tool_result_schema(decision),
            "process.query_events" => self.query_process_events(decision),
            "office.inspect_workbook" => self.inspect_workbook(decision),
            "office.workbook.read_text" => self.office_runtime().read_workbook_text(
                &path_arg(decision, "input_path").or_else(|_| path_arg(decision, "path"))?,
                decision.output_spec.get("sheet").and_then(Value::as_str),
                usize_arg(decision, "max_rows", 200),
            ),
            "office.workbook.read_cells" => self.office_runtime().read_workbook_cells(
                &path_arg(decision, "input_path").or_else(|_| path_arg(decision, "path"))?,
                decision.output_spec.get("sheet").and_then(Value::as_str),
                usize_arg(decision, "max_rows", 200),
            ),
            "office.docx.read_text" => self
                .office_runtime()
                .read_text(&path_arg(decision, "input_path").or_else(|_| path_arg(decision, "path"))?),
            "office.docx.batch_read_text" => self
                .office_runtime()
                .batch_read_text(&string_arg(decision, "source_set_ref")?),
            "office.docx.batch_extract_metadata" => self
                .office_runtime()
                .batch_extract_metadata(&string_arg(decision, "source_set_ref")?),
            "office.docx.batch_validate" => self
                .office_runtime()
                .batch_validate(&string_arg(decision, "source_set_ref")?),
            "document.pdf.extract_text" => self.extract_pdf_text(decision),
            "office.docx.diff_summary" => self.office_runtime().diff_summary(
                &path_arg(decision, "before_path")?,
                &path_arg(decision, "after_path")?,
            ),
            "office.docx.validate" => self
                .office_runtime()
                .validate_docx(&path_arg(decision, "input_path").or_else(|_| path_arg(decision, "path"))?),
            other => Ok(self.process_capability_receipt(
                other,
                "blocked",
                json!({
                    "reason": "capability is not read-only or is not supported by ReadOnlyCapabilityExecutor",
                    "capability_id": other,
                    "no_workspace_mutation": true,
                }),
            )?),
        }
    }

    pub fn provider_tool_result_from_receipt(
        &self,
        turn_id: &str,
        receipt: &CapabilityReceipt,
    ) -> io::Result<Value> {
        let receipt_ref = self.truth.write_blob(
            &format!(
                "provider_tool_results/{}/{}_{}_receipt.json",
                safe_blob_name(&self.runtime_id),
                safe_blob_name(turn_id),
                now_ms()
            ),
            &serde_json::to_vec_pretty(receipt).map_err(json_err)?,
        )?;
        let mut result = json!({
            "status": receipt.status,
            "receipt_status": receipt.status,
            "capability_id": receipt.capability_id,
            "receipt_ref": receipt_ref,
            "receipt": to_json_value(receipt)?,
            "read_only_executor": true,
        });
        if let Some(object) = result.as_object_mut() {
            for key in [
                "content_ref",
                "content_preview",
                "char_count",
                "row_count",
                "sheet_count",
                "dataset_ref",
                "raw_result_ref",
                "page_ref",
                "cited_refs",
            ] {
                if let Some(value) = receipt.data.get(key) {
                    object.insert(key.to_string(), value.clone());
                }
            }
        }
        Ok(result)
    }

    fn os_runtime(&self) -> OsRuntime {
        let runtime = OsRuntime::new(self.guard.clone(), self.truth.clone(), self.token.clone());
        if self.emit_capability_receipt_event {
            runtime
        } else {
            runtime.without_process_truth_events()
        }
    }

    fn office_runtime(&self) -> OfficeRuntime {
        let runtime = OfficeRuntime::new(
            self.guard.clone(),
            self.truth.clone(),
            self.token.clone(),
            office_worker_project(),
        );
        if self.emit_capability_receipt_event {
            runtime
        } else {
            runtime.without_process_truth_events()
        }
    }

    fn data_runtime(&self) -> DataRuntime {
        let runtime = DataRuntime::new(self.guard.clone(), self.truth.clone(), self.token.clone());
        if self.emit_capability_receipt_event {
            runtime
        } else {
            runtime.without_process_truth_events()
        }
    }

    fn artifact_runtime(&self) -> ArtifactRuntime {
        let runtime =
            ArtifactRuntime::new(self.guard.clone(), self.truth.clone(), self.token.clone());
        if self.emit_capability_receipt_event {
            runtime
        } else {
            runtime.without_process_truth_events()
        }
    }

    fn client_env_runtime(&self) -> ClientEnvRuntime {
        let runtime =
            ClientEnvRuntime::new(self.guard.clone(), self.truth.clone(), self.token.clone());
        if self.emit_capability_receipt_event {
            runtime
        } else {
            runtime.without_process_truth_events()
        }
    }

    fn read_dataset_page(&self, decision: &NextActionDecision) -> io::Result<CapabilityReceipt> {
        let dataset_ref = string_arg(decision, "dataset_ref")?;
        let offset = usize_arg(decision, "offset", 0);
        let limit = usize_arg(decision, "limit", 100).clamp(1, 1000);
        let dataset = match self.data_runtime().read_dataset(&dataset_ref) {
            Ok(dataset) => dataset,
            Err(err) => return self.read_raw_blob_text_page(&dataset_ref, offset, limit, err),
        };
        let rows = dataset
            .records
            .iter()
            .skip(offset)
            .take(limit)
            .cloned()
            .collect::<Vec<_>>();
        let page_ref = self.truth.write_blob(
            &format!(
                "datasets/{}_page_{}_{}.json",
                safe_blob_name(&dataset.dataset_id),
                offset,
                now_ms()
            ),
            &serde_json::to_vec_pretty(&rows).map_err(json_err)?,
        )?;
        self.process_capability_receipt(
            "dataset.read_page",
            "success",
            json!({
                "dataset_ref": dataset_ref,
                "dataset_id": dataset.dataset_id,
                "schema": dataset.schema,
                "offset": offset,
                "limit": limit,
                "returned": rows.len(),
                "total": dataset.row_count,
                "page_ref": page_ref,
                "rows": rows,
            }),
        )
    }

    fn read_raw_blob_text_page(
        &self,
        dataset_ref: &str,
        offset: usize,
        limit: usize,
        dataset_parse_error: io::Error,
    ) -> io::Result<CapabilityReceipt> {
        let blob_path = match self.truth.resolve_blob_ref(dataset_ref) {
            Ok(path) => path,
            Err(_) => return Err(dataset_parse_error),
        };
        let bytes = match std::fs::read(&blob_path) {
            Ok(bytes) => bytes,
            Err(_) => return Err(dataset_parse_error),
        };
        let text = String::from_utf8_lossy(&bytes);
        let lines = text
            .lines()
            .skip(offset)
            .take(limit)
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        let text_page = lines.join("\n");
        let total_lines = text.lines().count();
        let page_ref = self.truth.write_blob(
            &format!(
                "datasets/{}_raw_text_page_{}_{}.json",
                safe_blob_name(dataset_ref),
                offset,
                now_ms()
            ),
            &serde_json::to_vec_pretty(&json!({
                "dataset_ref": dataset_ref,
                "offset": offset,
                "limit": limit,
                "lines": lines,
            }))
            .map_err(json_err)?,
        )?;
        self.process_capability_receipt(
            "dataset.read_page",
            "success",
            json!({
                "dataset_ref": dataset_ref,
                "offset": offset,
                "limit": limit,
                "returned": lines.len(),
                "total": total_lines,
                "page_ref": page_ref,
                "lines": lines,
                "text": text_page,
                "raw_blob_text_fallback": true,
                "original_dataset_parse_error": dataset_parse_error.to_string(),
                "no_workspace_mutation": true,
            }),
        )
    }

    fn inspect_artifact(
        &self,
        decision: &NextActionDecision,
        capability_id: &str,
    ) -> io::Result<CapabilityReceipt> {
        let raw_path = path_arg(decision, "path")?;
        let path = self.resolve_artifact_input_path(&raw_path)?;
        let metadata = std::fs::metadata(&path)?;
        let max_preview_bytes = usize_arg(decision, "max_preview_bytes", 8192).min(65_536);
        let preview = if metadata.is_file() && max_preview_bytes > 0 {
            std::fs::read(&path).ok().map(|bytes| {
                String::from_utf8_lossy(&bytes[..bytes.len().min(max_preview_bytes)]).into_owned()
            })
        } else {
            None
        };
        let relative_path = path
            .strip_prefix(self.guard.root())
            .ok()
            .map(|item| item.display().to_string().replace('\\', "/"))
            .unwrap_or_else(|| raw_path.replace('\\', "/"));
        self.process_capability_receipt(
            capability_id,
            "success",
            json!({
                "path": relative_path,
                "exists": true,
                "is_file": metadata.is_file(),
                "is_dir": metadata.is_dir(),
                "size_bytes": metadata.len(),
                "preview": preview,
                "no_workspace_mutation": true,
            }),
        )
    }

    fn inspect_workbook(&self, decision: &NextActionDecision) -> io::Result<CapabilityReceipt> {
        let raw_path = path_arg(decision, "path").or_else(|_| path_arg(decision, "input_path"))?;
        let path = self.resolve_artifact_input_path(&raw_path)?;
        let metadata = std::fs::metadata(&path)?;
        let bytes = std::fs::read(&path).unwrap_or_default();
        let is_zip_container = bytes.starts_with(b"PK\x03\x04");
        let zip_entries = if is_zip_container {
            zip_central_directory_entries(&bytes).unwrap_or_default()
        } else {
            Vec::new()
        };
        let worksheet_entries = zip_entries
            .iter()
            .filter(|entry| entry.starts_with("xl/worksheets/") && entry.ends_with(".xml"))
            .cloned()
            .collect::<Vec<_>>();
        let relative_path = path
            .strip_prefix(self.guard.root())
            .ok()
            .map(|item| item.display().to_string().replace('\\', "/"))
            .unwrap_or_else(|| raw_path.replace('\\', "/"));
        self.process_capability_receipt(
            "office.inspect_workbook",
            "success",
            json!({
                "path": relative_path,
                "size_bytes": metadata.len(),
                "is_zip_container": is_zip_container,
                "extension": path.extension().and_then(|item| item.to_str()).unwrap_or(""),
                "inspection_kind": "workbook_openxml_container_metadata",
                "entry_count": zip_entries.len(),
                "entries": zip_entries.iter().take(200).cloned().collect::<Vec<_>>(),
                "workbook_xml_present": zip_entries.iter().any(|entry| entry == "xl/workbook.xml"),
                "shared_strings_present": zip_entries.iter().any(|entry| entry == "xl/sharedStrings.xml"),
                "styles_present": zip_entries.iter().any(|entry| entry == "xl/styles.xml"),
                "worksheet_count": worksheet_entries.len(),
                "worksheet_entries": worksheet_entries,
                "no_workspace_mutation": true,
            }),
        )
    }

    fn extract_pdf_text(&self, decision: &NextActionDecision) -> io::Result<CapabilityReceipt> {
        let raw_path = path_arg(decision, "input_path").or_else(|_| path_arg(decision, "path"))?;
        let path = self.resolve_artifact_input_path(&raw_path)?;
        let bytes = std::fs::read(&path)?;
        let extracted = extract_pdf_text_layer(&bytes);
        let text = extracted.text;
        let relative_path = path
            .strip_prefix(self.guard.root())
            .ok()
            .map(|item| item.display().to_string().replace('\\', "/"))
            .unwrap_or_else(|| raw_path.replace('\\', "/"));
        if text.trim().is_empty() {
            return self.process_capability_receipt(
                "document.pdf.extract_text",
                "failed",
                json!({
                    "input_path": relative_path,
                    "reason": "pdf_text_layer_empty_or_unsupported",
                    "extractor": extracted.extractor,
                    "extractor_error": extracted.error,
                    "ocr_performed": false,
                    "no_workspace_mutation": true,
                }),
            );
        }
        let content_ref = self.truth.write_blob(
            &format!(
                "pdf_text/{}_{}.txt",
                safe_blob_name(&relative_path),
                now_ms()
            ),
            text.as_bytes(),
        )?;
        self.process_capability_receipt(
            "document.pdf.extract_text",
            "success",
            json!({
                "input_path": relative_path,
                "content_ref": content_ref,
                "content_preview": text.chars().take(8192).collect::<String>(),
                "char_count": text.chars().count(),
                "extractor": extracted.extractor,
                "extractor_error": extracted.error,
                "ocr_performed": false,
                "no_workspace_mutation": true,
            }),
        )
    }

    fn resolve_artifact_input_path(&self, raw_path: &str) -> io::Result<PathBuf> {
        let normalized = raw_path
            .strip_prefix("artifact://")
            .or_else(|| raw_path.strip_prefix("artifact_ref://"))
            .unwrap_or(raw_path);
        self.guard
            .resolve_workspace_path(normalized)
            .map_err(|err| io::Error::new(io::ErrorKind::PermissionDenied, err))
    }

    fn read_typed_ref(&self, decision: &NextActionDecision) -> io::Result<CapabilityReceipt> {
        let target_ref = ref_arg(decision)?;
        let Ok((content, source_kind)) = self.read_ref_text(&target_ref) else {
            return Ok(self.process_capability_receipt(
                "process.read_ref",
                "blocked",
                json!({
                    "ref": target_ref,
                    "reason": "unsupported ref scheme for process.read_ref",
                }),
            )?);
        };
        let content_ref = self.truth.write_blob(
            &format!(
                "process_reads/{}_{}.txt",
                safe_blob_name(&self.runtime_id),
                now_ms()
            ),
            content.as_bytes(),
        )?;
        self.process_capability_receipt(
            "process.read_ref",
            "success",
            json!({
                "ref": target_ref,
                "source_kind": source_kind,
                "content_ref": content_ref,
                "content_preview": content.chars().take(8192).collect::<String>(),
                "bytes": content.len(),
                "tokens_estimated": estimate_text_tokens_conservative(&content),
            }),
        )
    }

    fn page_tool_result(&self, decision: &NextActionDecision) -> io::Result<CapabilityReceipt> {
        let target_ref = raw_result_ref_arg(decision)?;
        let (content, source_kind) = self.read_ref_text(&target_ref)?;
        let offset = decision
            .output_spec
            .get("offset")
            .and_then(Value::as_u64)
            .unwrap_or(0) as usize;
        let limit = decision
            .output_spec
            .get("limit_bytes")
            .or_else(|| decision.output_spec.get("limit"))
            .and_then(Value::as_u64)
            .unwrap_or(16 * 1024) as usize;
        let total_chars = content.chars().count();
        let page = content.chars().skip(offset).take(limit).collect::<String>();
        let page_ref = self.truth.write_blob(
            &format!(
                "tool_result_pages/{}_{}_{}.txt",
                safe_blob_name(&self.runtime_id),
                offset,
                now_ms()
            ),
            page.as_bytes(),
        )?;
        self.process_capability_receipt(
            "tool.result.page",
            "success",
            json!({
                "ref": target_ref,
                "source_kind": source_kind,
                "offset": offset,
                "limit": limit,
                "total_chars": total_chars,
                "has_more": offset + page.chars().count() < total_chars,
                "page_ref": page_ref,
                "page": page,
            }),
        )
    }

    fn search_tool_result(&self, decision: &NextActionDecision) -> io::Result<CapabilityReceipt> {
        let target_ref = raw_result_ref_arg(decision)?;
        let query = string_arg(decision, "query")?;
        let max_matches = decision
            .output_spec
            .get("max_matches")
            .and_then(Value::as_u64)
            .unwrap_or(20) as usize;
        let (content, source_kind) = self.read_ref_text(&target_ref)?;
        let mut matches = Vec::new();
        for (index, line) in content.lines().enumerate() {
            if line.contains(&query) {
                matches.push(json!({
                    "line_number": index + 1,
                    "line": line,
                }));
                if matches.len() >= max_matches {
                    break;
                }
            }
        }
        self.process_capability_receipt(
            "tool.result.search",
            "success",
            json!({
                "ref": target_ref,
                "source_kind": source_kind,
                "query": query,
                "match_count": matches.len(),
                "matches": matches,
            }),
        )
    }

    fn inspect_tool_result_schema(
        &self,
        decision: &NextActionDecision,
    ) -> io::Result<CapabilityReceipt> {
        let target_ref = raw_result_ref_arg(decision)?;
        let (content, source_kind) = self.read_ref_text(&target_ref)?;
        let parsed = serde_json::from_str::<Value>(&content).ok();
        let schema = parsed
            .as_ref()
            .map(inspect_json_shape)
            .unwrap_or_else(|| json!({"type": "text", "bytes": content.len()}));
        self.process_capability_receipt(
            "tool.result.inspect_schema",
            "success",
            json!({
                "ref": target_ref,
                "source_kind": source_kind,
                "schema": schema,
            }),
        )
    }

    fn query_process_events(&self, decision: &NextActionDecision) -> io::Result<CapabilityReceipt> {
        let limit = decision
            .output_spec
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(80) as usize;
        let event_type_filter = decision
            .output_spec
            .get("event_type")
            .and_then(Value::as_str)
            .map(str::to_string);
        let mut events = self.truth.read_events()?;
        if let Some(filter) = event_type_filter.as_deref() {
            events.retain(|event| event.event_type == filter);
        }
        let start = events.len().saturating_sub(limit);
        let selected = events[start..].to_vec();
        let events_ref = self.truth.write_blob(
            &format!(
                "process_reads/{}_events_{}.json",
                safe_blob_name(&self.runtime_id),
                now_ms()
            ),
            &serde_json::to_vec_pretty(&selected).map_err(json_err)?,
        )?;
        self.process_capability_receipt(
            "process.query_events",
            "success",
            json!({
                "events_ref": events_ref,
                "event_count": selected.len(),
                "event_type": event_type_filter,
            }),
        )
    }

    fn read_ref_text(&self, target_ref: &str) -> io::Result<(String, &'static str)> {
        if target_ref.starts_with("blob://") {
            Ok((
                std::fs::read_to_string(self.truth.resolve_blob_ref(target_ref)?)?,
                "blob",
            ))
        } else if let Some(path) = target_ref.strip_prefix("artifact_ref://") {
            let artifact_path = self
                .guard
                .resolve_workspace_path(path)
                .map_err(|err| io::Error::new(io::ErrorKind::PermissionDenied, err))?;
            Ok((std::fs::read_to_string(artifact_path)?, "artifact"))
        } else if let Some(path) = target_ref.strip_prefix("artifact://") {
            let artifact_path = self
                .guard
                .resolve_workspace_path(path)
                .map_err(|err| io::Error::new(io::ErrorKind::PermissionDenied, err))?;
            Ok((std::fs::read_to_string(artifact_path)?, "artifact"))
        } else if target_ref.starts_with("chat_blob://") {
            Ok((
                crate::ChatTruthStore::new_with_state_root(
                    self.truth.workspace_root(),
                    self.truth.state_root(),
                )?
                .read_chat_blob_text(target_ref)?,
                "chat_blob",
            ))
        } else if target_ref.starts_with("chat://")
            || target_ref.starts_with("chat_thread://")
            || target_ref.starts_with("chat_turn://")
            || target_ref.starts_with("chat_")
        {
            Ok((
                crate::ChatTruthStore::new_with_state_root(
                    self.truth.workspace_root(),
                    self.truth.state_root(),
                )?
                .read_chat_ref_text(target_ref)?,
                "chat_truth",
            ))
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "unsupported ref scheme",
            ))
        }
    }

    fn process_capability_receipt(
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
        if self.emit_capability_receipt_event {
            self.truth.append_event(
                Some(&self.token.pid),
                "capability_receipt",
                to_json_value(&receipt)?,
            )?;
        }
        Ok(receipt)
    }
}

fn string_arg(decision: &NextActionDecision, key: &str) -> io::Result<String> {
    decision
        .output_spec
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(ToString::to_string)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, format!("{key} missing")))
}

fn path_arg(decision: &NextActionDecision, key: &str) -> io::Result<String> {
    string_arg(decision, key)
}

fn string_array_arg(decision: &NextActionDecision, key: &str) -> Vec<String> {
    decision
        .output_spec
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn usize_arg(decision: &NextActionDecision, key: &str, default_value: usize) -> usize {
    decision
        .output_spec
        .get(key)
        .and_then(Value::as_u64)
        .map(|value| value as usize)
        .unwrap_or(default_value)
}

fn ref_arg(decision: &NextActionDecision) -> io::Result<String> {
    decision
        .output_spec
        .get("ref")
        .and_then(Value::as_str)
        .or_else(|| decision.output_spec.get("path").and_then(Value::as_str))
        .or_else(|| decision.input_refs.first().map(String::as_str))
        .filter(|value| !value.trim().is_empty())
        .map(ToString::to_string)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "ref missing"))
}

fn raw_result_ref_arg(decision: &NextActionDecision) -> io::Result<String> {
    for key in ["ref", "raw_result_ref", "receipt_ref", "path"] {
        if let Some(value) = decision
            .output_spec
            .get(key)
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
        {
            return Ok(value.to_string());
        }
    }
    if let Some(value) = decision.input_refs.first() {
        return Ok(value.clone());
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidInput,
        "raw result ref missing",
    ))
}

fn depth_arg(decision: &NextActionDecision, default_depth: usize) -> usize {
    decision
        .output_spec
        .get("max_depth")
        .and_then(Value::as_u64)
        .unwrap_or(default_depth as u64)
        .clamp(1, 128) as usize
}

fn inspect_json_shape(value: &Value) -> Value {
    match value {
        Value::Null => json!({"type": "null"}),
        Value::Bool(_) => json!({"type": "boolean"}),
        Value::Number(_) => json!({"type": "number"}),
        Value::String(text) => json!({"type": "string", "chars": text.chars().count()}),
        Value::Array(items) => json!({
            "type": "array",
            "len": items.len(),
            "first": items.first().map(inspect_json_shape),
        }),
        Value::Object(map) => json!({
            "type": "object",
            "keys": map.keys().cloned().collect::<Vec<_>>(),
        }),
    }
}

fn zip_central_directory_entries(bytes: &[u8]) -> io::Result<Vec<String>> {
    let Some(eocd) = find_zip_eocd(bytes) else {
        return Ok(Vec::new());
    };
    if eocd + 22 > bytes.len() {
        return Ok(Vec::new());
    }
    let entry_count = read_u16_le(bytes, eocd + 10).unwrap_or(0) as usize;
    let directory_size = read_u32_le(bytes, eocd + 12).unwrap_or(0) as usize;
    let directory_offset = read_u32_le(bytes, eocd + 16).unwrap_or(0) as usize;
    let directory_end = directory_offset
        .saturating_add(directory_size)
        .min(bytes.len());
    let mut offset = directory_offset;
    let mut entries = Vec::new();
    while offset + 46 <= directory_end && entries.len() < entry_count.max(1_000) {
        if read_u32_le(bytes, offset) != Some(0x0201_4b50) {
            break;
        }
        let name_len = read_u16_le(bytes, offset + 28).unwrap_or(0) as usize;
        let extra_len = read_u16_le(bytes, offset + 30).unwrap_or(0) as usize;
        let comment_len = read_u16_le(bytes, offset + 32).unwrap_or(0) as usize;
        let name_start = offset + 46;
        let name_end = name_start.saturating_add(name_len);
        if name_end > bytes.len() {
            break;
        }
        let name = String::from_utf8_lossy(&bytes[name_start..name_end])
            .replace('\\', "/")
            .trim_matches('/')
            .to_string();
        if !name.is_empty() {
            entries.push(name);
        }
        offset = name_end
            .saturating_add(extra_len)
            .saturating_add(comment_len);
    }
    Ok(entries)
}

fn find_zip_eocd(bytes: &[u8]) -> Option<usize> {
    let min_len = 22;
    if bytes.len() < min_len {
        return None;
    }
    let search_start = bytes.len().saturating_sub(65_557);
    (search_start..=bytes.len() - min_len)
        .rev()
        .find(|&idx| read_u32_le(bytes, idx) == Some(0x0605_4b50))
}

fn read_u16_le(bytes: &[u8], offset: usize) -> Option<u16> {
    let slice = bytes.get(offset..offset + 2)?;
    Some(u16::from_le_bytes([slice[0], slice[1]]))
}

fn read_u32_le(bytes: &[u8], offset: usize) -> Option<u32> {
    let slice = bytes.get(offset..offset + 4)?;
    Some(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

struct PdfTextExtraction {
    text: String,
    extractor: &'static str,
    error: Option<String>,
}

fn extract_pdf_text_layer(bytes: &[u8]) -> PdfTextExtraction {
    let primary = std::panic::catch_unwind(|| pdf_extract::extract_text_from_mem(bytes));
    match primary {
        Ok(Ok(text)) if !text.trim().is_empty() => PdfTextExtraction {
            text,
            extractor: "pdf_extract_text_layer",
            error: None,
        },
        Ok(Ok(_)) => {
            let literal = extract_pdf_literal_text(bytes);
            PdfTextExtraction {
                text: literal,
                extractor: "pdf_literal_fallback_after_empty_text_layer",
                error: Some("pdf_extract returned empty text".to_string()),
            }
        }
        Ok(Err(err)) => {
            let literal = extract_pdf_literal_text(bytes);
            PdfTextExtraction {
                text: literal,
                extractor: "pdf_literal_fallback_after_extract_error",
                error: Some(err.to_string()),
            }
        }
        Err(_) => {
            let literal = extract_pdf_literal_text(bytes);
            PdfTextExtraction {
                text: literal,
                extractor: "pdf_literal_fallback_after_extract_panic",
                error: Some("pdf_extract panicked while reading PDF".to_string()),
            }
        }
    }
}

fn extract_pdf_literal_text(bytes: &[u8]) -> String {
    let source = String::from_utf8_lossy(bytes);
    let mut out = Vec::<String>::new();
    let mut chars = source.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '(' {
            let mut value = String::new();
            let mut escaped = false;
            while let Some(next) = chars.next() {
                if escaped {
                    value.push(match next {
                        'n' => '\n',
                        'r' => '\r',
                        't' => '\t',
                        'b' => '\u{0008}',
                        'f' => '\u{000c}',
                        other => other,
                    });
                    escaped = false;
                } else if next == '\\' {
                    escaped = true;
                } else if next == ')' {
                    break;
                } else {
                    value.push(next);
                }
            }
            let trimmed = value.trim();
            if looks_like_pdf_text(trimmed) {
                out.push(trimmed.to_string());
            }
        } else if ch == '<' && chars.peek() != Some(&'<') {
            let mut hex = String::new();
            while let Some(next) = chars.next() {
                if next == '>' {
                    break;
                }
                if next.is_ascii_hexdigit() {
                    hex.push(next);
                }
            }
            if hex.len() >= 4 && hex.len() % 2 == 0 {
                let decoded = hex
                    .as_bytes()
                    .chunks(2)
                    .filter_map(|pair| std::str::from_utf8(pair).ok())
                    .filter_map(|pair| u8::from_str_radix(pair, 16).ok())
                    .collect::<Vec<_>>();
                let value = String::from_utf8_lossy(&decoded).trim().to_string();
                if looks_like_pdf_text(&value) {
                    out.push(value);
                }
            }
        }
    }
    out.join("\n")
}

fn looks_like_pdf_text(value: &str) -> bool {
    let meaningful = value
        .chars()
        .filter(|ch| ch.is_alphanumeric() || ch.is_ascii_punctuation())
        .count();
    meaningful >= 2 && !value.starts_with('/') && !value.contains('\u{0000}')
}

fn office_worker_project() -> PathBuf {
    std::env::var("SUPERNOVA_OFFICE_WORKER_PROJECT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .join("office_worker")
                .join("SuperNova.OfficeWorker")
                .join("SuperNova.OfficeWorker.csproj")
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dataset_read_page_can_page_raw_read_file_blob_refs() {
        let workspace =
            std::env::temp_dir().join(format!("supernova_readonly_raw_blob_{}", crate::now_ms()));
        std::fs::create_dir_all(&workspace).unwrap();
        let (job, process, truth) =
            crate::create_agent_job(&workspace, "Read raw blob as dataset page").unwrap();
        let token = CapabilityToken {
            token_id: "token_test".to_string(),
            job_id: job.job_id,
            pid: process.pid,
            workspace_root: workspace.display().to_string(),
            capabilities: vec!["dataset.read_page".to_string()],
            permissions: vec!["workspace:read".to_string()],
        };
        let executor = ReadOnlyCapabilityExecutor::new(
            WorkspaceGuard::new(&workspace).unwrap(),
            truth.clone(),
            token,
            "test_readonly",
        );
        let raw_ref = truth
            .write_blob("datasets/readme.md", b"line one\nline two\nline three")
            .unwrap();
        let mut decision = crate::reasoning::decision(
            crate::reasoning::TaskAgentDecisionKind::RunCapability,
            "dataset.read_page",
            "page raw text",
        );
        decision.output_spec = json!({
            "dataset_ref": raw_ref,
            "offset": 1,
            "limit": 1,
        });

        let receipt = executor.execute(&decision).unwrap();

        assert_eq!(receipt.status, "success");
        assert_eq!(receipt.data["raw_blob_text_fallback"], true);
        assert_eq!(receipt.data["text"], "line two");
        assert_eq!(receipt.data["returned"], 1);
    }

    #[test]
    fn process_read_ref_reads_chat_turn_refs_from_chat_truth() {
        let workspace =
            std::env::temp_dir().join(format!("supernova_readonly_chat_ref_{}", crate::now_ms()));
        std::fs::create_dir_all(&workspace).unwrap();
        let (job, process, truth) =
            crate::create_agent_job(&workspace, "Read chat turn ref").unwrap();
        let chat_truth =
            crate::ChatTruthStore::new_with_state_root(&workspace, truth.state_root()).unwrap();
        let thread = chat_truth
            .create_thread("container_1", Some("prior chat".to_string()))
            .unwrap();
        let turn_id = "chat_turn_test_read_ref";
        let user_ref = chat_truth
            .write_chat_blob(
                &thread.chat_thread_id,
                "turns/user.txt",
                b"prior user question",
            )
            .unwrap();
        let assistant_ref = chat_truth
            .write_chat_blob(
                &thread.chat_thread_id,
                "turns/assistant.txt",
                b"prior assistant answer",
            )
            .unwrap();
        chat_truth
            .append_event(
                &thread.chat_thread_id,
                &thread.container_id,
                "chat_user_message_recorded",
                json!({"turn_id": turn_id, "message_ref": user_ref}),
                None,
            )
            .unwrap();
        chat_truth
            .append_event(
                &thread.chat_thread_id,
                &thread.container_id,
                "chat_assistant_answered",
                json!({"turn_id": turn_id, "assistant_content_ref": assistant_ref}),
                None,
            )
            .unwrap();
        let token = CapabilityToken {
            token_id: "token_test".to_string(),
            job_id: job.job_id,
            pid: process.pid,
            workspace_root: workspace.display().to_string(),
            capabilities: vec!["process.read_ref".to_string()],
            permissions: vec!["workspace:read".to_string()],
        };
        let executor = ReadOnlyCapabilityExecutor::new(
            WorkspaceGuard::new(&workspace).unwrap(),
            truth,
            token,
            "test_readonly",
        );
        let mut decision = crate::reasoning::decision(
            crate::reasoning::TaskAgentDecisionKind::RunCapability,
            "process.read_ref",
            "read prior chat",
        );
        decision.output_spec = json!({ "ref": turn_id });

        let receipt = executor.execute(&decision).unwrap();

        assert_eq!(receipt.status, "success");
        assert_eq!(receipt.data["source_kind"], "chat_truth");
        assert!(receipt.data["content_preview"]
            .as_str()
            .unwrap()
            .contains("prior user question"));
        assert!(receipt.data["content_preview"]
            .as_str()
            .unwrap()
            .contains("prior assistant answer"));
    }

    #[test]
    fn zip_central_directory_entries_reads_openxml_sheet_names() {
        let name = b"xl/worksheets/sheet1.xml";
        let mut central = vec![0_u8; 46];
        central[0..4].copy_from_slice(&0x0201_4b50_u32.to_le_bytes());
        central[28..30].copy_from_slice(&(name.len() as u16).to_le_bytes());
        central.extend_from_slice(name);
        let central_len = central.len() as u32;
        let mut eocd = vec![0_u8; 22];
        eocd[0..4].copy_from_slice(&0x0605_4b50_u32.to_le_bytes());
        eocd[8..10].copy_from_slice(&1_u16.to_le_bytes());
        eocd[10..12].copy_from_slice(&1_u16.to_le_bytes());
        eocd[12..16].copy_from_slice(&central_len.to_le_bytes());
        eocd[16..20].copy_from_slice(&0_u32.to_le_bytes());
        central.extend_from_slice(&eocd);

        let entries = zip_central_directory_entries(&central).unwrap();
        assert_eq!(entries, vec!["xl/worksheets/sheet1.xml"]);
    }

    #[test]
    fn pdf_literal_text_extractor_reads_text_layer_strings() {
        let bytes =
            b"%PDF-1.4\n1 0 obj <<>> stream\nBT (Hello PDF) Tj <576f726c64> Tj ET\nendstream";
        let text = extract_pdf_literal_text(bytes);

        assert!(text.contains("Hello PDF"));
        assert!(text.contains("World"));
        let extracted = extract_pdf_text_layer(bytes);
        assert!(extracted.text.contains("Hello PDF"));
        assert!(extracted.text.contains("World"));
        assert!(extracted.extractor.starts_with("pdf_literal_fallback"));
    }
}
