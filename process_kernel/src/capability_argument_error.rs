use std::io;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{now_ms, safe_blob_name, ProcessTruthStore};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CapabilityInvalidField {
    pub field: String,
    pub issue: String,
    pub expected: Value,
    pub received_type: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CapabilityArgumentError {
    pub schema_error: bool,
    pub capability_id: String,
    pub error_message: String,
    pub raw_arguments_ref: String,
    pub invalid_fields: Vec<CapabilityInvalidField>,
    pub required_arguments: Value,
    pub minimal_valid_example: Value,
    pub recoverable_by_task_agent: bool,
}

impl CapabilityArgumentError {
    pub fn to_receipt_data(&self, runtime_id: &str, turn_id: &str, decision_id: &str) -> Value {
        json!({
            "runtime_id": runtime_id,
            "turn_id": turn_id,
            "decision_id": decision_id,
            "capability_id": self.capability_id,
            "error": self.error_message,
            "schema_error": self.schema_error,
            "raw_arguments_ref": self.raw_arguments_ref,
            "invalid_fields": self.invalid_fields,
            "required_arguments": self.required_arguments,
            "minimal_valid_example": self.minimal_valid_example,
            "recoverable_by_task_agent": self.recoverable_by_task_agent,
            "normalized_arguments": Value::Null,
            "runtime_note": "recoverable syscall argument error; TaskAgent must inspect invalid_fields/minimal_valid_example and retry with corrected arguments if the task still requires this capability",
        })
    }
}

pub fn build_capability_argument_error(
    truth: &ProcessTruthStore,
    runtime_id: &str,
    capability_id: &str,
    raw_arguments: &Value,
    error_message: &str,
) -> io::Result<CapabilityArgumentError> {
    let raw_arguments_ref = truth.write_blob(
        &format!(
            "capability_argument_errors/{}_{}_{}_args.json",
            safe_blob_name(runtime_id),
            safe_blob_name(capability_id),
            now_ms()
        ),
        &serde_json::to_vec_pretty(raw_arguments).map_err(crate::json_err)?,
    )?;
    Ok(CapabilityArgumentError {
        schema_error: true,
        capability_id: capability_id.to_string(),
        error_message: error_message.to_string(),
        raw_arguments_ref,
        invalid_fields: invalid_fields_for(capability_id, raw_arguments, error_message),
        required_arguments: required_arguments_for(capability_id),
        minimal_valid_example: minimal_valid_example_for(capability_id),
        recoverable_by_task_agent: true,
    })
}

pub fn invalid_write_kind_argument_error(
    truth: &ProcessTruthStore,
    runtime_id: &str,
    raw_arguments: &Value,
    write_kind: &str,
) -> io::Result<CapabilityArgumentError> {
    let mut error = build_capability_argument_error(
        truth,
        runtime_id,
        "os.write_file",
        raw_arguments,
        "write_kind must be one of artifact, source_mutation, or temp_dataset",
    )?;
    error.invalid_fields = vec![CapabilityInvalidField {
        field: "write_kind".to_string(),
        issue: "invalid_enum_value".to_string(),
        expected: json!(["artifact", "source_mutation", "temp_dataset"]),
        received_type: format!("string:{write_kind}"),
    }];
    Ok(error)
}

fn invalid_fields_for(
    capability_id: &str,
    raw_arguments: &Value,
    error_message: &str,
) -> Vec<CapabilityInvalidField> {
    let mut fields = Vec::new();
    for field in required_field_names(capability_id) {
        if !has_required_field(raw_arguments, field) {
            fields.push(CapabilityInvalidField {
                field: field.to_string(),
                issue: required_field_issue(field).to_string(),
                expected: expected_value_for_field(field),
                received_type: received_type(raw_arguments.get(field)),
            });
        }
    }
    if fields.is_empty() && content_required(capability_id) && !has_any_content(raw_arguments) {
        fields.push(CapabilityInvalidField {
            field: "content|text|content_ref|text_ref".to_string(),
            issue: "missing_content_payload".to_string(),
            expected: json!("one of content, text, content_ref, or text_ref"),
            received_type: "missing".to_string(),
        });
    }
    if fields.is_empty() {
        if let Some(field) = missing_field_from_error(error_message) {
            fields.push(CapabilityInvalidField {
                field: field.to_string(),
                issue: "missing_or_not_string".to_string(),
                expected: expected_value_for_field(field),
                received_type: received_type(raw_arguments.get(field)),
            });
        }
    }
    if fields.is_empty() && error_message.contains("expected value at line 1 column 1") {
        if source_set_ref_capability(capability_id) {
            fields.push(CapabilityInvalidField {
                field: "source_set_ref".to_string(),
                issue: "referenced_source_set_blob_is_empty_or_not_valid_json".to_string(),
                expected: json!("blob://... source set JSON created by source_set.create"),
                received_type: received_type(raw_arguments.get("source_set_ref")),
            });
        } else {
            fields.push(CapabilityInvalidField {
                field: "arguments".to_string(),
                issue: "raw_argument_or_referenced_blob_is_empty_or_invalid_json".to_string(),
                expected: json!("valid JSON arguments or refs produced by prior receipts"),
                received_type: json_value_kind(raw_arguments).to_string(),
            });
        }
    }
    if fields.is_empty() {
        fields.push(CapabilityInvalidField {
            field: "arguments".to_string(),
            issue: "capability_argument_error".to_string(),
            expected: required_arguments_for(capability_id),
            received_type: json_value_kind(raw_arguments).to_string(),
        });
    }
    fields
}

fn required_field_names(capability_id: &str) -> &'static [&'static str] {
    match capability_id {
        "source_set.read_page"
        | "workspace.batch_hash"
        | "workspace.find_duplicates"
        | "workspace.recent_changes"
        | "workspace.recent_changes_snapshot"
        | "workspace.tree_index"
        | "workspace.perf_inventory"
        | "office.docx.batch_read_text"
        | "office.docx.batch_extract_metadata"
        | "office.docx.batch_validate"
        | "package.build_zip" => &["source_set_ref"],
        "dataset.export_csv" | "dataset.export_markdown" | "dataset.coverage_verify" => {
            &["dataset_ref"]
        }
        "artifact.copy_source_set" => &["source_set_ref", "destination_dir"],
        "os.write_file" => &["path", "write_kind"],
        "os.write_artifact"
        | "os.write_temp_dataset"
        | "os.write_source_mutation_preview"
        | "os.write_source_mutation_apply" => &["path"],
        "os.stat_path" | "os.read_file" | "os.hash_path" | "os.delete_path"
        | "os.verify_artifact" => &["path"],
        "os.copy_path" | "os.move_path" | "os.rename_path" => &["source_path", "destination_path"],
        "os.diff" => &["left_path", "right_path"],
        "os.zip" => &["destination_zip_path"],
        "os.unzip" => &["archive_path", "destination_dir"],
        "office.docx.read_text" | "office.docx.validate" => &["input_path"],
        "office.docx.create" => &["output_path"],
        "office.docx.rewrite_save_as" => &["input_path", "output_path"],
        "office.docx.rewrite_preview"
        | "office.docx.rewrite_in_place_preview"
        | "office.docx.rewrite_in_place" => &["input_path"],
        "office.docx.diff_summary" => &["before_path", "after_path"],
        "workspace.apply_organize_tx" => &["organize_plan_ref"],
        "workspace.rename_batch_apply" => &["rename_plan_ref"],
        "process.preview.create" | "process.request_preview" => &["operations"],
        "terminal.run_command" => &["argv", "timeout_ms"],
        "terminal.start_service" => &["service_id", "argv", "startup_timeout_ms"],
        "terminal.stop_service" | "terminal.service_status" => &["service_id"],
        _ => &[],
    }
}

fn content_required(capability_id: &str) -> bool {
    matches!(
        capability_id,
        "os.write_file"
            | "os.write_artifact"
            | "os.write_temp_dataset"
            | "os.write_source_mutation_preview"
            | "os.write_source_mutation_apply"
            | "office.docx.create"
            | "office.docx.rewrite_save_as"
            | "office.docx.rewrite_preview"
            | "office.docx.rewrite_in_place_preview"
            | "office.docx.rewrite_in_place"
    )
}

fn source_set_ref_capability(capability_id: &str) -> bool {
    required_field_names(capability_id)
        .iter()
        .any(|field| *field == "source_set_ref")
}

fn required_arguments_for(capability_id: &str) -> Value {
    match capability_id {
        "source_set.create" => json!({
            "root_path": "workspace directory; defaults to . when omitted",
            "include_extensions": "optional array of extensions such as .docx or .md",
            "include_globs": "optional array of workspace-relative glob filters",
            "exclude_globs": "optional array of workspace-relative exclude filters",
            "max_depth": "optional traversal depth"
        }),
        "source_set.read_page" => json!({
            "source_set_ref": "blob://... source set created by source_set.create",
            "offset": "optional non-negative integer",
            "limit": "optional positive integer"
        }),
        "workspace.find_duplicates" => json!({
            "source_set_ref": "blob://... source set created by source_set.create"
        }),
        "workspace.recent_changes" | "workspace.recent_changes_snapshot" => json!({
            "source_set_ref": "blob://... source set created by source_set.create",
            "days": "optional number of days; default 7"
        }),
        "workspace.tree_index" => json!({
            "source_set_ref": "blob://... source set created by source_set.create",
            "tree_path": "optional output path such as TREE.md"
        }),
        "workspace.perf_inventory" => json!({
            "source_set_ref": "blob://... source set created by source_set.create",
            "output_path": "optional output path such as PERF_NOTES.json"
        }),
        "process.preview.create" | "process.request_preview" => json!({
            "operations": [{
                "capability_id": "canonical registered capability id, e.g. package.build_zip",
                "arguments": "capability arguments object",
                "target_paths": ["workspace-relative target path"],
                "human_description": "natural-language preview text for the user",
                "rollback_policy": "optional rollback policy"
            }]
        }),
        "os.write_file" => json!({
            "path": "workspace-relative output path",
            "content_or_text_or_ref": "one of content, text, content_ref, or text_ref",
            "write_kind": "artifact, source_mutation, or temp_dataset"
        }),
        "os.write_artifact" => json!({
            "path": "workspace-relative user artifact output path",
            "content_or_text_or_ref": "one of content, text, content_ref, or text_ref"
        }),
        "os.write_temp_dataset" => json!({
            "path": "workspace-relative temporary dataset output path",
            "content_or_text_or_ref": "one of content, text, content_ref, or text_ref"
        }),
        "os.write_source_mutation_preview" | "os.write_source_mutation_apply" => json!({
            "path": "workspace-relative source file path",
            "content_or_text_or_ref": "one of content, text, content_ref, or text_ref"
        }),
        "office.docx.read_text" | "office.docx.validate" => json!({
            "input_path": "workspace-relative .docx path"
        }),
        "office.docx.create" => json!({
            "output_path": "workspace-relative .docx output path",
            "content_or_text_or_ref": "one of content, text, content_ref, or text_ref"
        }),
        "office.docx.rewrite_save_as" => json!({
            "input_path": "workspace-relative source .docx path",
            "output_path": "workspace-relative target .docx path",
            "content_or_text_or_ref": "one of content, text, content_ref, or text_ref containing the already rewritten document body"
        }),
        "office.docx.rewrite_preview"
        | "office.docx.rewrite_in_place_preview"
        | "office.docx.rewrite_in_place" => json!({
            "input_path": "workspace-relative source .docx path",
            "content_or_text_or_ref": "one of content, text, content_ref, or text_ref containing the already rewritten document body"
        }),
        "office.docx.batch_read_text"
        | "office.docx.batch_extract_metadata"
        | "office.docx.batch_validate" => json!({
            "source_set_ref": "blob://... source set containing DOCX files"
        }),
        _ => {
            let fields = required_field_names(capability_id);
            if fields.is_empty() {
                json!({"schema_note": "inspect the capability descriptor for accepted arguments"})
            } else {
                let mut object = serde_json::Map::new();
                for field in fields {
                    object.insert((*field).to_string(), expected_value_for_field(field));
                }
                Value::Object(object)
            }
        }
    }
}

fn minimal_valid_example_for(capability_id: &str) -> Value {
    match capability_id {
        "source_set.create" => json!({"root_path": "."}),
        "source_set.read_page" => {
            json!({"source_set_ref": "blob://<job_id>/source_sets/<id>.json", "offset": 0, "limit": 100})
        }
        "workspace.find_duplicates" => {
            json!({"source_set_ref": "blob://<job_id>/source_sets/<id>.json"})
        }
        "workspace.recent_changes" | "workspace.recent_changes_snapshot" => {
            json!({"source_set_ref": "blob://<job_id>/source_sets/<id>.json", "days": 7})
        }
        "workspace.tree_index" => {
            json!({"source_set_ref": "blob://<job_id>/source_sets/<id>.json", "tree_path": "TREE.md"})
        }
        "workspace.perf_inventory" => {
            json!({"source_set_ref": "blob://<job_id>/source_sets/<id>.json", "output_path": "PERF_NOTES.json"})
        }
        "process.preview.create" | "process.request_preview" => json!({
            "operations": [{
                "capability_id": "package.build_zip",
                "arguments": {
                    "source_set_ref": "blob://<job_id>/source_sets/<id>.json",
                    "archive_path": "deliverable.zip"
                },
                "target_paths": ["deliverable.zip", "PACK_MANIFEST.md", "SHA256SUMS.txt"],
                "human_description": "Create the approved deliverable package.",
                "rollback_policy": "delete_created_artifacts"
            }]
        }),
        "os.write_file" => {
            json!({"path": "OUTPUT.md", "content": "user-visible artifact text", "write_kind": "artifact"})
        }
        "os.write_artifact" => {
            json!({"path": "OUTPUT.md", "content": "user-visible artifact text"})
        }
        "os.write_temp_dataset" => {
            json!({"path": "tmp/intermediate.json", "content": "{\"rows\":[]}"})
        }
        "os.write_source_mutation_preview" => {
            json!({"path": "reports/source.md", "content": "replacement text"})
        }
        "os.write_source_mutation_apply" => {
            json!({"path": "reports/source.md", "content": "replacement text"})
        }
        "office.docx.read_text" | "office.docx.validate" => {
            json!({"input_path": "documents/example.docx"})
        }
        "office.docx.create" => {
            json!({"output_path": "deliverables/output.docx", "content_ref": "blob://<job_id>/model_outputs/<id>.txt"})
        }
        "office.docx.rewrite_save_as" => {
            json!({"input_path": "drafts/source.docx", "output_path": "deliverables/output.docx", "text_ref": "blob://<job_id>/model_outputs/<id>.txt"})
        }
        "office.docx.rewrite_preview"
        | "office.docx.rewrite_in_place_preview"
        | "office.docx.rewrite_in_place" => {
            json!({"input_path": "drafts/source.docx", "text_ref": "blob://<job_id>/model_outputs/<id>.txt"})
        }
        "office.docx.batch_read_text"
        | "office.docx.batch_extract_metadata"
        | "office.docx.batch_validate" => {
            json!({"source_set_ref": "blob://<job_id>/source_sets/<id>.json"})
        }
        _ => {
            let mut object = serde_json::Map::new();
            for field in required_field_names(capability_id) {
                object.insert((*field).to_string(), expected_value_for_field(field));
            }
            Value::Object(object)
        }
    }
}

fn expected_value_for_field(field: &str) -> Value {
    match field {
        "source_set_ref" => json!("blob://... source set created by source_set.create"),
        "dataset_ref" => json!("blob://... dataset created by a dataset-producing capability"),
        "path"
        | "artifact_path"
        | "input_path"
        | "output_path"
        | "source_path"
        | "destination_path"
        | "left_path"
        | "right_path"
        | "before_path"
        | "after_path"
        | "tree_path"
        | "destination_zip_path"
        | "archive_path"
        | "destination_dir" => {
            json!("workspace-relative path string")
        }
        "write_kind" => json!(["artifact", "source_mutation", "temp_dataset"]),
        "argv" => json!("array of command argv strings"),
        "operations" => json!("array of executable preview operation objects"),
        "organize_plan_ref" => json!("blob://... organize plan ref"),
        "rename_plan_ref" => json!("blob://... rename plan ref"),
        _ => json!("required argument"),
    }
}

fn has_non_empty_string(arguments: &Value, field: &str) -> bool {
    arguments
        .get(field)
        .and_then(Value::as_str)
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
}

fn has_required_field(arguments: &Value, field: &str) -> bool {
    match field {
        "operations" | "argv" => arguments
            .get(field)
            .and_then(Value::as_array)
            .map(|items| !items.is_empty())
            .unwrap_or(false),
        _ => has_non_empty_string(arguments, field),
    }
}

fn required_field_issue(field: &str) -> &'static str {
    match field {
        "operations" | "argv" => "missing_or_not_non_empty_array",
        _ => "missing_or_not_string",
    }
}

fn has_any_content(arguments: &Value) -> bool {
    ["content", "text", "content_ref", "text_ref"]
        .iter()
        .any(|field| has_non_empty_string(arguments, field))
}

fn received_type(value: Option<&Value>) -> String {
    match value {
        None => "missing".to_string(),
        Some(value) => json_value_kind(value).to_string(),
    }
}

fn json_value_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn missing_field_from_error(error_message: &str) -> Option<&str> {
    error_message
        .strip_suffix(" missing")
        .map(str::trim)
        .filter(|field| !field.is_empty() && !field.contains(' '))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_set_ref_json_parse_failure_points_to_ref_field() {
        let fields = invalid_fields_for(
            "workspace.find_duplicates",
            &json!({"source_set_ref": "blob://job/source_sets/empty.json"}),
            "expected value at line 1 column 1",
        );
        assert_eq!(fields[0].field, "source_set_ref");
        assert_eq!(
            fields[0].issue,
            "referenced_source_set_blob_is_empty_or_not_valid_json"
        );
    }

    #[test]
    fn docx_rewrite_missing_content_uses_content_group() {
        let fields = invalid_fields_for(
            "office.docx.rewrite_save_as",
            &json!({"input_path": "a.docx", "output_path": "b.docx"}),
            "content/text/content_ref/text_ref missing",
        );
        assert_eq!(fields[0].field, "content|text|content_ref|text_ref");
    }

    #[test]
    fn write_file_missing_write_kind_points_to_explicit_field() {
        let fields = invalid_fields_for(
            "os.write_file",
            &json!({"path": "REPORT.md", "content": "# Report"}),
            "write_kind missing",
        );
        assert!(fields
            .iter()
            .any(|field| field.field == "write_kind" && field.issue == "missing_or_not_string"));
    }

    #[test]
    fn preview_create_requires_operations_array() {
        let fields = invalid_fields_for(
            "process.preview.create",
            &json!({"preview_markdown": "# Preview"}),
            "preview requires executable operations",
        );
        assert_eq!(fields[0].field, "operations");
        assert_eq!(fields[0].issue, "missing_or_not_non_empty_array");
    }
}
