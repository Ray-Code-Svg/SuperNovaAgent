use std::collections::{BTreeMap, BTreeSet};
use std::io;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::model_config::{ModelInvocationConfig, TaskAgentDecisionProtocol, ToolChoicePolicy};
use crate::model_runtime::{ModelOperation, ProviderToolCall};
use crate::reasoning::{decision, NextActionDecision, TaskAgentDecisionKind};
use crate::CapabilityDescriptor;

pub const PROVIDER_TOOL_PHASE6_FULL_COVERAGE: &str = "phase6_full_schema_coverage";

pub const PHASE7_STRICT_COMPATIBLE_READONLY_PROVIDER_TOOL_CAPABILITIES: &[&str] = &[
    "os.stat_path",
    "os.read_file",
    "data.csv.read_dataset",
    "office.workbook.read_cells",
    "office.workbook.read_text",
    "document.pdf.extract_text",
    "client_env.scan_overview",
    "process.read_ref",
];

pub const CHAT_RUNTIME_PROVIDER_TOOL_CAPABILITIES: &[&str] = &[
    "chat.answer",
    "chat.clarify",
    "chat.needs_task",
    "os.list_tree",
    "os.workspace_inventory",
    "os.stat_path",
    "os.read_file",
    "os.hash_path",
    "os.diff",
    "os.verify_artifact",
    "source_set.create",
    "source_set.read_page",
    "source_set.coverage_verify",
    "workspace.batch_hash",
    "workspace.find_duplicates",
    "workspace.recent_changes",
    "workspace.recent_changes_snapshot",
    "dataset.read_page",
    "data.csv.read_dataset",
    "dataset.coverage_verify",
    "artifact.inspect",
    "artifact.audit_readonly",
    "client_env.scan_overview",
    "client_env.scan_device",
    "client_env.scan_storage",
    "client_env.scan_network",
    "client_env.scan_runtimes",
    "client_env.read_snapshot",
    "client_env.request_sensitive_disclosure",
    "process.read_ref",
    "tool.result.page",
    "tool.result.search",
    "tool.result.inspect_schema",
    "office.inspect_workbook",
    "office.workbook.read_cells",
    "office.workbook.read_text",
    "office.docx.read_text",
    "document.pdf.extract_text",
    "office.docx.batch_read_text",
    "office.docx.batch_extract_metadata",
    "office.docx.batch_validate",
    "office.docx.diff_summary",
    "office.docx.validate",
];

pub const PHASE4_PROVIDER_TOOL_CAPABILITIES: &[&str] = &[
    "os.list_tree",
    "os.workspace_inventory",
    "os.stat_path",
    "os.read_file",
    "process.read_ref",
    "process.query_events",
    "process.toolset.select",
    "process.clarify",
    "process.fail",
    "process.complete",
];

pub const PHASE5_PREVIEW_PROVIDER_TOOL_CAPABILITIES: &[&str] = &[];

pub const PHASE5_MUTATION_APPLY_PROVIDER_TOOL_CAPABILITIES: &[&str] = &[
    "os.move_path",
    "os.rename_path",
    "os.delete_path",
    "office.docx.rewrite_in_place",
];

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ProviderToolDefinition {
    #[serde(rename = "type")]
    pub tool_type: String,
    pub function: ProviderToolFunction,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ProviderToolFunction {
    pub name: String,
    pub description: String,
    pub parameters: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strict: Option<bool>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderToolBinding {
    pub provider_tool_name: String,
    pub capability_id: String,
    pub phase: String,
    pub read_only_or_process_control: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ProviderToolRegistry {
    pub tools: Vec<ProviderToolDefinition>,
    pub bindings: BTreeMap<String, ProviderToolBinding>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderToolProtocolError {
    pub error_code: String,
    pub message: String,
    pub provider_tool_name: Option<String>,
    pub provider_tool_call_id: Option<String>,
    pub capability_id: Option<String>,
}

impl ProviderToolRegistry {
    pub fn phase4_readonly(
        registry: &[CapabilityDescriptor],
        config: &ModelInvocationConfig,
    ) -> Self {
        Self::from_capability_ids(
            registry,
            config,
            PHASE4_PROVIDER_TOOL_CAPABILITIES,
            "phase4_readonly_mvp",
        )
    }

    pub fn phase5_approval_aware(
        registry: &[CapabilityDescriptor],
        config: &ModelInvocationConfig,
    ) -> Self {
        let mut capability_ids = PHASE4_PROVIDER_TOOL_CAPABILITIES.to_vec();
        capability_ids.extend(PHASE5_PREVIEW_PROVIDER_TOOL_CAPABILITIES);
        capability_ids.extend(PHASE5_MUTATION_APPLY_PROVIDER_TOOL_CAPABILITIES);
        Self::from_capability_ids(
            registry,
            config,
            &capability_ids,
            "phase5_approval_aware_mutation_mvp",
        )
    }

    pub fn phase6_full_coverage(
        registry: &[CapabilityDescriptor],
        config: &ModelInvocationConfig,
    ) -> Self {
        Self::from_capability_descriptors(
            registry,
            config,
            &registry
                .iter()
                .filter(|descriptor| {
                    provider_tool_capability_is_task_runtime_exposable(&descriptor.capability_id)
                })
                .collect::<Vec<_>>(),
            PROVIDER_TOOL_PHASE6_FULL_COVERAGE,
            None,
        )
    }

    pub fn phase6_schema_coverage(
        registry: &[CapabilityDescriptor],
        config: &ModelInvocationConfig,
    ) -> Self {
        Self::from_capability_descriptors(
            registry,
            config,
            &registry
                .iter()
                .filter(|descriptor| {
                    provider_tool_capability_is_exposable(&descriptor.capability_id)
                })
                .collect::<Vec<_>>(),
            PROVIDER_TOOL_PHASE6_FULL_COVERAGE,
            None,
        )
    }

    pub fn phase6_selected(
        registry: &[CapabilityDescriptor],
        config: &ModelInvocationConfig,
        capability_ids: &[String],
        phase: &str,
    ) -> Self {
        let selected = capability_ids
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        Self::from_capability_ids_with_limit(registry, config, &selected, phase, None)
    }

    pub fn chat_runtime_readonly(
        registry: &[CapabilityDescriptor],
        config: &ModelInvocationConfig,
    ) -> Self {
        Self::from_capability_ids_with_limit(
            registry,
            config,
            CHAT_RUNTIME_PROVIDER_TOOL_CAPABILITIES,
            "chat_runtime_readonly",
            Some(config.tool_calling.max_provider_tools_per_request),
        )
    }

    fn from_capability_ids(
        registry: &[CapabilityDescriptor],
        config: &ModelInvocationConfig,
        capability_ids: &[&str],
        phase: &str,
    ) -> Self {
        Self::from_capability_ids_with_limit(
            registry,
            config,
            capability_ids,
            phase,
            Some(config.tool_calling.max_provider_tools_per_request),
        )
    }

    fn from_capability_ids_with_limit(
        registry: &[CapabilityDescriptor],
        config: &ModelInvocationConfig,
        capability_ids: &[&str],
        phase: &str,
        limit: Option<usize>,
    ) -> Self {
        let selected = capability_ids.iter().copied().collect::<BTreeSet<_>>();
        let descriptors = registry
            .iter()
            .filter(|descriptor| selected.contains(descriptor.capability_id.as_str()))
            .collect::<Vec<_>>();
        Self::from_capability_descriptors(registry, config, &descriptors, phase, limit)
    }

    fn from_capability_descriptors(
        registry: &[CapabilityDescriptor],
        config: &ModelInvocationConfig,
        descriptors: &[&CapabilityDescriptor],
        phase: &str,
        limit: Option<usize>,
    ) -> Self {
        let known = registry
            .iter()
            .map(|descriptor| descriptor.capability_id.as_str())
            .collect::<BTreeSet<_>>();
        let mut tools = Vec::new();
        let mut bindings = BTreeMap::new();
        for descriptor in descriptors {
            let capability_id = descriptor.capability_id.as_str();
            if !known.contains(capability_id)
                || !provider_tool_capability_is_exposable(capability_id)
            {
                continue;
            }
            if limit.is_some_and(|max| tools.len() >= max) {
                break;
            }
            let provider_tool_name = provider_tool_name_for_capability(capability_id);
            let parameters = provider_tool_parameters_for_descriptor(descriptor);
            let strict_parameters = provider_tool_strict_parameters_for_descriptor(
                descriptor,
                &parameters,
                config.tool_calling.strict_mode,
            );
            let strict = strict_parameters.is_some().then_some(true);
            let binding = ProviderToolBinding {
                provider_tool_name: provider_tool_name.clone(),
                capability_id: capability_id.to_string(),
                phase: phase.to_string(),
                read_only_or_process_control: !provider_tool_is_mutation_apply_capability(
                    capability_id,
                ),
            };
            tools.push(ProviderToolDefinition {
                tool_type: "function".to_string(),
                function: ProviderToolFunction {
                    name: provider_tool_name.clone(),
                    description: provider_tool_description(descriptor),
                    parameters: strict_parameters.unwrap_or(parameters),
                    strict,
                },
            });
            bindings.insert(provider_tool_name, binding);
        }
        Self { tools, bindings }
    }

    pub fn binding_for_tool_name(&self, name: &str) -> Option<&ProviderToolBinding> {
        self.bindings.get(name)
    }

    pub fn decision_for_tool_call(
        &self,
        call: &ProviderToolCall,
    ) -> Result<NextActionDecision, ProviderToolProtocolError> {
        let function_name = provider_tool_call_name(call)?;
        let Some(binding) = self.binding_for_tool_name(&function_name) else {
            return Err(unregistered_tool_error(call, &function_name));
        };
        let arguments = provider_tool_call_arguments(call)?;
        let reason = arguments
            .get("reason")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("DeepSeek provider-native tool call requested this capability.");
        let kind = match binding.capability_id.as_str() {
            "process.request_preview" => TaskAgentDecisionKind::RequestPreview,
            "process.clarify" => TaskAgentDecisionKind::Clarify,
            "process.complete" => TaskAgentDecisionKind::Complete,
            "process.fail" => TaskAgentDecisionKind::Fail,
            _ => TaskAgentDecisionKind::RunCapability,
        };
        let mut next = decision(kind, &binding.capability_id, reason);
        next.output_spec = arguments;
        Ok(next)
    }
}

pub fn provider_native_tool_calls_enabled(config: &ModelInvocationConfig) -> bool {
    config.tool_calling.enabled
        && matches!(config.decision_protocol, TaskAgentDecisionProtocol::ProviderNativeToolCalls)
}

pub fn provider_native_tool_request_enabled(
    config: &ModelInvocationConfig,
    operation: &ModelOperation,
) -> bool {
    provider_native_tool_calls_enabled(config)
        && matches!(
            operation,
            ModelOperation::DecideNextAction | ModelOperation::ChatTurn
        )
}

pub fn provider_tool_choice_value(config: &ModelInvocationConfig) -> Option<Value> {
    if config.thinking.mode.is_effectively_enabled() {
        return None;
    }
    match config.tool_calling.tool_choice {
        ToolChoicePolicy::None => Some(json!("none")),
        ToolChoicePolicy::Auto => Some(json!("auto")),
        ToolChoicePolicy::Required => Some(json!("required")),
    }
}

pub fn provider_tool_name_for_capability(capability_id: &str) -> String {
    format!(
        "cap_{}",
        capability_id
            .chars()
            .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
            .collect::<String>()
    )
}

pub fn provider_tool_capability_is_exposable(capability_id: &str) -> bool {
    !capability_id.starts_with("model.")
}

pub fn provider_tool_capability_is_task_runtime_exposable(capability_id: &str) -> bool {
    provider_tool_capability_is_exposable(capability_id)
        && !capability_id.starts_with("chat.")
        && capability_id != "process.approval.record"
        && capability_id != "process.request_preview"
        && capability_id != "process.preview.create"
        && capability_id != "process.pending_approvals"
        && capability_id != "workspace.rename_batch_preview"
        && capability_id != "os.write_source_mutation_preview"
        && capability_id != "office.docx.rewrite_in_place_preview"
}

pub fn provider_tool_domain(capability_id: &str) -> String {
    if capability_id == "process.fork_child" {
        return "child_process".to_string();
    }
    capability_id
        .split_once('.')
        .map(|(head, _)| head.to_string())
        .unwrap_or_else(|| "process".to_string())
}

pub fn provider_tool_is_mutation_apply_capability(capability_id: &str) -> bool {
    PHASE5_MUTATION_APPLY_PROVIDER_TOOL_CAPABILITIES.contains(&capability_id)
        || matches!(
            capability_id,
            "os.write_file"
                | "os.write_source_mutation_apply"
                | "os.move_path"
                | "os.rename_path"
                | "os.delete_path"
                | "os.rollback_tx"
                | "workspace.apply_organize_tx"
                | "workspace.rename_batch_apply"
                | "office.docx.rewrite_in_place"
        )
}

pub fn provider_tool_requires_explicit_approval_id(capability_id: &str) -> bool {
    let _ = capability_id;
    false
}

pub fn provider_tool_strict_compatible(capability_id: &str) -> bool {
    PHASE7_STRICT_COMPATIBLE_READONLY_PROVIDER_TOOL_CAPABILITIES.contains(&capability_id)
        && !provider_tool_is_mutation_apply_capability(capability_id)
}

pub fn provider_tool_is_preview_capability(capability_id: &str) -> bool {
    PHASE5_PREVIEW_PROVIDER_TOOL_CAPABILITIES.contains(&capability_id)
}

pub fn provider_tool_call_name(
    call: &ProviderToolCall,
) -> Result<String, ProviderToolProtocolError> {
    call.function
        .get("name")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .ok_or_else(|| ProviderToolProtocolError {
            error_code: "PROVIDER_TOOL_FUNCTION_NAME_MISSING".to_string(),
            message: "provider tool_call.function.name is missing".to_string(),
            provider_tool_name: None,
            provider_tool_call_id: Some(call.id.clone()),
            capability_id: None,
        })
}

pub fn provider_tool_call_arguments(
    call: &ProviderToolCall,
) -> Result<Value, ProviderToolProtocolError> {
    let Some(arguments) = call.function.get("arguments") else {
        return Ok(json!({}));
    };
    match arguments {
        Value::Object(_) => Ok(arguments.clone()),
        Value::String(text) if text.trim().is_empty() => Ok(json!({})),
        Value::String(text) => {
            serde_json::from_str::<Value>(text).map_err(|err| ProviderToolProtocolError {
                error_code: "PROVIDER_TOOL_ARGUMENTS_JSON_INVALID".to_string(),
                message: format!("provider tool_call.function.arguments is not valid JSON: {err}"),
                provider_tool_name: call
                    .function
                    .get("name")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                provider_tool_call_id: Some(call.id.clone()),
                capability_id: None,
            })
        }
        _ => Err(ProviderToolProtocolError {
            error_code: "PROVIDER_TOOL_ARGUMENTS_TYPE_INVALID".to_string(),
            message: "provider tool_call.function.arguments must be a JSON string or object"
                .to_string(),
            provider_tool_name: call
                .function
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_string),
            provider_tool_call_id: Some(call.id.clone()),
            capability_id: None,
        }),
    }
}

pub fn protocol_error_to_io(error: ProviderToolProtocolError) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error.message)
}

fn unregistered_tool_error(
    call: &ProviderToolCall,
    function_name: &str,
) -> ProviderToolProtocolError {
    if let Some(model_capability) = forbidden_model_capability(function_name) {
        return ProviderToolProtocolError {
            error_code: "PROVIDER_TOOL_MODEL_CAPABILITY_FORBIDDEN".to_string(),
            message: format!(
                "provider-native tool call attempted forbidden model capability: {model_capability}"
            ),
            provider_tool_name: Some(function_name.to_string()),
            provider_tool_call_id: Some(call.id.clone()),
            capability_id: Some(model_capability),
        };
    }
    ProviderToolProtocolError {
        error_code: "PROVIDER_TOOL_FUNCTION_UNREGISTERED".to_string(),
        message: format!("provider-native tool function is not registered: {function_name}"),
        provider_tool_name: Some(function_name.to_string()),
        provider_tool_call_id: Some(call.id.clone()),
        capability_id: None,
    }
}

fn forbidden_model_capability(function_name: &str) -> Option<String> {
    function_name
        .strip_prefix("cap_model_")
        .map(|tail| format!("model.{}", tail.replace('_', ".")))
}

fn provider_tool_description(descriptor: &CapabilityDescriptor) -> String {
    let capability_id = descriptor.capability_id.as_str();
    let specific = match capability_id {
        "os.list_tree" => Some("List workspace tree entries without reading file contents."),
        "os.workspace_inventory" => Some("Return a bounded workspace inventory for planning."),
        "os.stat_path" => Some("Inspect metadata for one workspace path."),
        "os.read_file" => Some(
            "Read one workspace file through the Process Kernel. Do not use this for directories; use os.list_tree or os.stat_path for directories, and source_set.create/source_set.read_page for project-scale source inspection.",
        ),
        "client_env.scan_overview" => Some(
            "Inspect a sanitized low-sensitive summary of the local desktop environment. Sensitive fields are redacted unless explicitly authorized.",
        ),
        "client_env.scan_device" => Some(
            "Inspect sanitized local OS/device facts such as OS family, architecture, CPU count, memory bucket, locale, and timezone.",
        ),
        "client_env.scan_storage" => Some(
            "Inspect sanitized workspace-volume storage readiness with bucketed capacity/free-space facts.",
        ),
        "client_env.scan_network" => Some(
            "Inspect sanitized local network readiness. Local IP and MAC require explicit client-env disclosure authorization.",
        ),
        "client_env.scan_runtimes" => Some(
            "Inspect local runtime readiness for Python, Node, npm, Rust, Cargo, .NET, Office worker, and Kernel CLI. Missing runtimes return available=false.",
        ),
        "client_env.read_snapshot" => Some("Read a paged ClientEnv snapshot ref."),
        "client_env.request_sensitive_disclosure" => Some(
            "Request explicit user authorization for sensitive local environment fields. This never returns sensitive values.",
        ),
        "process.read_ref" => Some("Read a typed ProcessTruth, artifact, chat blob, chat thread, or chat turn ref."),
        "process.query_events" => Some("Query recent ProcessTruth events."),
        "process.toolset.select" => {
            Some("Select provider tool groups to expose in the next request.")
        }
        "process.pending_approvals" => Some("Inspect legacy pending approval state for diagnostics."),
        "process.request_preview" => Some("Disabled compatibility control. It records a no-preview receipt and does not pause execution."),
        "process.preview.create" => Some("Disabled compatibility control. It records a no-preview receipt and does not create an approval transaction."),
        "process.clarify" => Some("Ask the user for missing task information."),
        "process.fail" => Some("Fail the task with explicit evidence and reason."),
        "process.complete" => Some("Complete the task through the Kernel completion gate."),
        "terminal.run_command" => Some(
            "Run one bounded foreground command. You must provide timeout_ms yourself. Do not use this for servers or dev services; use terminal.start_service instead.",
        ),
        "terminal.start_service" => Some(
            "Start a long-running terminal service such as uvicorn, streamlit, vite, or a dev server. Provide a stable service_id, argv, explicit startup_timeout_ms, and health_check or expected_ports when possible.",
        ),
        "terminal.stop_service" => Some("Stop a previously started terminal service by service_id."),
        "terminal.service_status" => {
            Some("Inspect the current status and recent logs for a terminal service by service_id.")
        },
        "workspace.rename_batch_preview" => Some("Internal workspace rename preview capability."),
        "os.write_source_mutation_preview" => Some("Internal source-file mutation preview capability."),
        "office.docx.rewrite_in_place_preview" => Some("Internal DOCX rewrite preview capability."),
        "os.write_artifact" => Some(
            "Request a text artifact write. Use only for markdown, csv, json, or txt user artifacts; never create .zip/.docx or claim workspace mutations with this tool. The Kernel executes directly under workspace boundaries and receipts.",
        ),
        "os.write_temp_dataset" => Some(
            "Request a temporary dataset write. The Kernel executes directly under workspace boundaries and receipts.",
        ),
        "os.copy_path" => Some("Request a workspace copy through the Kernel."),
        "os.move_path" => Some("Request a workspace move through the Kernel."),
        "os.rename_path" => Some("Request a workspace rename through the Kernel."),
        "os.delete_path" => Some("Request a workspace delete through the Kernel."),
        "office.docx.rewrite_in_place" => {
            Some("Request an in-place DOCX rewrite through the Kernel.")
        }
        "package.build_zip" => Some("Request a package build through the Kernel."),
        _ => None,
    };
    specific.map(str::to_string).unwrap_or_else(|| {
        format!(
            "SuperNova Process Kernel capability `{}`. Input contract: {}. Approval policy: {}.",
            descriptor.capability_id, descriptor.input_schema, descriptor.approval_policy
        )
    })
}

fn object_parameters(
    required: &[&str],
    properties: &[(&str, Value)],
    additional_properties: bool,
) -> Value {
    let mut property_map = serde_json::Map::new();
    for (name, schema) in properties {
        property_map.insert((*name).to_string(), schema.clone());
    }
    json!({
        "type": "object",
        "required": required,
        "properties": property_map,
        "additionalProperties": additional_properties
    })
}

fn object_parameters_with_any_of(
    required: &[&str],
    properties: &[(&str, Value)],
    any_of: Value,
    additional_properties: bool,
) -> Value {
    let mut schema = object_parameters(required, properties, additional_properties);
    if let Some(object) = schema.as_object_mut() {
        object.insert("anyOf".to_string(), any_of);
    }
    schema
}

fn string_parameter(description: &str) -> Value {
    json!({
        "type": "string",
        "description": description
    })
}

fn boolean_parameter(description: &str) -> Value {
    json!({
        "type": "boolean",
        "description": description
    })
}

fn integer_parameter(description: &str, minimum: u64, maximum: Option<u64>) -> Value {
    let mut schema = serde_json::Map::new();
    schema.insert("type".to_string(), json!("integer"));
    schema.insert("minimum".to_string(), json!(minimum));
    if let Some(maximum) = maximum {
        schema.insert("maximum".to_string(), json!(maximum));
    }
    schema.insert("description".to_string(), json!(description));
    Value::Object(schema)
}

fn string_array_parameter(description: &str) -> Value {
    json!({
        "type": "array",
        "items": {"type": "string"},
        "description": description
    })
}

fn string_enum_parameter(description: &str, values: &[&str]) -> Value {
    json!({
        "type": "string",
        "enum": values,
        "description": description
    })
}

fn json_object_parameter(description: &str) -> Value {
    json!({
        "type": "object",
        "description": description
    })
}

fn reason_parameter() -> Value {
    string_parameter("Short reason for requesting this capability.")
}

fn content_any_of() -> Value {
    json!([
        {"required": ["content"]},
        {"required": ["text"]},
        {"required": ["content_ref"]},
        {"required": ["text_ref"]}
    ])
}

fn content_properties() -> [(&'static str, Value); 4] {
    [
        (
            "content",
            string_parameter("Literal text or bytes-as-text content for the capability."),
        ),
        ("text", string_parameter("Alias for literal text content.")),
        (
            "content_ref",
            string_parameter("Blob ref containing content, text, or rewritten_text."),
        ),
        (
            "text_ref",
            string_parameter("Blob ref containing text content."),
        ),
    ]
}

fn rename_mapping_parameter() -> Value {
    json!({
        "type": "object",
        "required": ["source_path", "destination_path"],
        "properties": {
            "source_path": {
                "type": "string",
                "description": "Workspace-relative path to rename from."
            },
            "destination_path": {
                "type": "string",
                "description": "Workspace-relative path to rename to."
            },
            "reason": {
                "type": "string",
                "description": "Optional reason for this individual rename."
            }
        },
        "additionalProperties": false
    })
}

pub fn provider_tool_parameters_for_descriptor(descriptor: &CapabilityDescriptor) -> Value {
    let capability_id = descriptor.capability_id.as_str();
    match capability_id {
        "os.list_tree" | "os.workspace_inventory" => object_parameters(
            &[],
            &[
                (
                    "max_depth",
                    integer_parameter("Maximum directory depth to inspect.", 1, Some(64)),
                ),
                ("reason", reason_parameter()),
            ],
            false,
        ),
        "os.stat_path" | "os.read_file" | "os.hash_path" | "os.verify_artifact" => {
            object_parameters(
                &["path"],
                &[
                    ("path", string_parameter("Workspace-relative path.")),
                    ("reason", reason_parameter()),
                ],
                false,
            )
        }
        "os.diff" => object_parameters(
            &["left_path", "right_path"],
            &[
                (
                    "left_path",
                    string_parameter("Left workspace-relative path."),
                ),
                (
                    "right_path",
                    string_parameter("Right workspace-relative path."),
                ),
                ("reason", reason_parameter()),
            ],
            false,
        ),
        "os.zip" => object_parameters(
            &["source_paths", "destination_zip_path"],
            &[
                (
                    "source_paths",
                    string_array_parameter("Workspace-relative files or directories to archive."),
                ),
                (
                    "destination_zip_path",
                    string_parameter("Workspace-relative destination zip path."),
                ),
                ("reason", reason_parameter()),
            ],
            false,
        ),
        "os.unzip" => object_parameters(
            &["archive_path", "destination_dir"],
            &[
                (
                    "archive_path",
                    string_parameter("Workspace-relative archive path."),
                ),
                (
                    "destination_dir",
                    string_parameter("Workspace-relative destination directory."),
                ),
                ("reason", reason_parameter()),
            ],
            false,
        ),
        "os.rollback_tx" => object_parameters(
            &["tx_id"],
            &[
                (
                    "tx_id",
                    string_parameter("Kernel transaction id to roll back."),
                ),
                ("reason", reason_parameter()),
            ],
            false,
        ),
        "client_env.scan_overview"
        | "client_env.scan_device"
        | "client_env.scan_storage"
        | "client_env.scan_network"
        | "client_env.scan_runtimes" => object_parameters(
            &[],
            &[
                (
                    "sections",
                    string_array_parameter(
                        "Optional section ids: device, storage, network, runtimes.",
                    ),
                ),
                (
                    "detail_level",
                    string_enum_parameter(
                        "Detail level for sanitized facts.",
                        &["summary", "standard"],
                    ),
                ),
                (
                    "include_sensitive_fields",
                    boolean_parameter("Set true only with a valid client-env authorization_id."),
                ),
                (
                    "authorization_id",
                    string_parameter(
                        "Client-env disclosure authorization id for sensitive fields.",
                    ),
                ),
                (
                    "max_items",
                    integer_parameter("Maximum bounded items to return.", 1, Some(1000)),
                ),
                ("reason", reason_parameter()),
            ],
            false,
        ),
        "client_env.read_snapshot" => object_parameters(
            &["snapshot_ref"],
            &[
                (
                    "snapshot_ref",
                    string_parameter("ClientEnv snapshot blob ref."),
                ),
                ("ref", string_parameter("Alias for snapshot_ref.")),
                (
                    "offset",
                    integer_parameter("Section offset.", 0, Some(10_000)),
                ),
                (
                    "limit",
                    integer_parameter("Maximum sections to return.", 1, Some(50)),
                ),
                ("reason", reason_parameter()),
            ],
            false,
        ),
        "client_env.request_sensitive_disclosure" => object_parameters(
            &["requested_fields", "reason"],
            &[
                (
                    "requested_fields",
                    string_array_parameter(
                        "Sensitive field ids, for example network.local_ip or network.mac_address.",
                    ),
                ),
                (
                    "reason",
                    string_parameter("Why the sensitive local environment fields are needed."),
                ),
            ],
            false,
        ),
        "process.read_ref" => object_parameters(
            &["ref"],
            &[
                (
                    "ref",
                    string_parameter("Typed ref, blob ref, artifact ref, or receipt ref."),
                ),
                ("reason", reason_parameter()),
            ],
            false,
        ),
        "chat.answer" => object_parameters(
            &["content"],
            &[
                (
                    "content",
                    string_parameter("Final assistant answer for this chat turn."),
                ),
                (
                    "cited_refs",
                    string_array_parameter("Optional refs used to ground the answer."),
                ),
                ("reason", reason_parameter()),
            ],
            false,
        ),
        "chat.clarify" => object_parameters(
            &["question"],
            &[
                (
                    "question",
                    string_parameter("Question to ask the user before answering."),
                ),
                (
                    "missing_fact",
                    string_parameter("Specific missing fact that blocks an answer."),
                ),
                ("reason", reason_parameter()),
            ],
            false,
        ),
        "chat.needs_task" => object_parameters(
            &["goal"],
            &[
                (
                    "goal",
                    string_parameter(
                        "Task goal that should be submitted to TaskRuntime if the user accepts.",
                    ),
                ),
                (
                    "context_pack_id",
                    string_parameter("Optional ContextPack id to pass to the task."),
                ),
                ("reason", reason_parameter()),
            ],
            false,
        ),
        "tool.result.page" => object_parameters_with_any_of(
            &[],
            &[
                (
                    "ref",
                    string_parameter(
                        "Ref to page; prefer receipt_ref from a previous tool result.",
                    ),
                ),
                (
                    "raw_result_ref",
                    string_parameter("Raw result ref, blob ref, artifact ref, or receipt ref."),
                ),
                (
                    "receipt_ref",
                    string_parameter("receipt_ref returned by a previous provider tool result."),
                ),
                (
                    "path",
                    string_parameter("Workspace path or artifact path alias."),
                ),
                (
                    "input_refs",
                    string_array_parameter("Fallback refs when no direct ref field is available."),
                ),
                (
                    "offset",
                    integer_parameter("Character offset for the page.", 0, None),
                ),
                (
                    "limit_bytes",
                    integer_parameter("Maximum characters/bytes to return.", 1, Some(200_000)),
                ),
                (
                    "limit",
                    integer_parameter("Alias for limit_bytes.", 1, Some(200_000)),
                ),
                ("reason", reason_parameter()),
            ],
            json!([
                {"required": ["ref"]},
                {"required": ["raw_result_ref"]},
                {"required": ["receipt_ref"]},
                {"required": ["path"]},
                {"required": ["input_refs"]}
            ]),
            false,
        ),
        "tool.result.search" => object_parameters_with_any_of(
            &["query"],
            &[
                (
                    "ref",
                    string_parameter(
                        "Ref to search; prefer receipt_ref from a previous tool result.",
                    ),
                ),
                (
                    "raw_result_ref",
                    string_parameter("Raw result ref, blob ref, artifact ref, or receipt ref."),
                ),
                (
                    "receipt_ref",
                    string_parameter("receipt_ref returned by a previous provider tool result."),
                ),
                (
                    "path",
                    string_parameter("Workspace path or artifact path alias."),
                ),
                (
                    "input_refs",
                    string_array_parameter("Fallback refs when no direct ref field is available."),
                ),
                ("query", string_parameter("Case-sensitive text query.")),
                (
                    "max_matches",
                    integer_parameter("Maximum number of matching lines to return.", 1, Some(200)),
                ),
                ("reason", reason_parameter()),
            ],
            json!([
                {"required": ["ref"]},
                {"required": ["raw_result_ref"]},
                {"required": ["receipt_ref"]},
                {"required": ["path"]},
                {"required": ["input_refs"]}
            ]),
            false,
        ),
        "tool.result.inspect_schema" => object_parameters_with_any_of(
            &[],
            &[
                (
                    "ref",
                    string_parameter(
                        "Ref to inspect; prefer receipt_ref from a previous tool result.",
                    ),
                ),
                (
                    "raw_result_ref",
                    string_parameter("Raw result ref, blob ref, artifact ref, or receipt ref."),
                ),
                (
                    "receipt_ref",
                    string_parameter("receipt_ref returned by a previous provider tool result."),
                ),
                (
                    "path",
                    string_parameter("Workspace path or artifact path alias."),
                ),
                (
                    "input_refs",
                    string_array_parameter("Fallback refs when no direct ref field is available."),
                ),
                ("reason", reason_parameter()),
            ],
            json!([
                {"required": ["ref"]},
                {"required": ["raw_result_ref"]},
                {"required": ["receipt_ref"]},
                {"required": ["path"]},
                {"required": ["input_refs"]}
            ]),
            false,
        ),
        "process.query_events" => object_parameters(
            &[],
            &[
                (
                    "event_type",
                    string_parameter("Optional ProcessTruth event type filter."),
                ),
                (
                    "limit",
                    integer_parameter("Maximum ProcessTruth events to return.", 1, Some(200)),
                ),
                ("reason", reason_parameter()),
            ],
            false,
        ),
        "process.toolset.select" => object_parameters(
            &["requested_groups", "next_intent", "reason"],
            &[
                (
                    "requested_groups",
                    json!({
                        "type": "array",
                        "items": {
                            "type": "string",
                            "enum": [
                                "source_set_batch",
                                "dataset_ops",
                                "artifact_write",
                                "artifact_quality",
                                "mutation_preview",
                                "mutation_apply",
                                "office_docx",
                                "package_release",
                                "terminal_fallback",
                                "process_structure",
                                "rollback_recovery"
                            ]
                        },
                        "maxItems": 4,
                        "description": "Provider tool groups requested for the next model request."
                    }),
                ),
                (
                    "required_capabilities",
                    json!({
                        "type": "array",
                        "items": {"type": "string"},
                        "maxItems": 8,
                        "description": "Optional exact SuperNova capability ids requested for the next model request."
                    }),
                ),
                (
                    "next_intent",
                    json!({
                        "type": "string",
                        "maxLength": 300,
                        "description": "Brief description of the next task step that requires these groups."
                    }),
                ),
                (
                    "ttl_model_calls",
                    json!({
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 6,
                        "description": "How many future model calls should keep this selection active."
                    }),
                ),
                ("reason", reason_parameter()),
            ],
            false,
        ),
        "process.fork_child" => object_parameters(
            &["kind"],
            &[
                (
                    "kind",
                    string_enum_parameter(
                        "Child process kind to fork.",
                        &[
                            "source_discovery",
                            "corpus_extraction",
                            "synthesis",
                            "mutation_preview",
                            "commit",
                            "verify",
                            "artifact_audit",
                        ],
                    ),
                ),
                (
                    "input_refs",
                    string_array_parameter("Typed refs to pass into the child process."),
                ),
                (
                    "capabilities",
                    string_array_parameter("Capability ids to grant to the child process."),
                ),
                (
                    "budget_ms",
                    integer_parameter("Optional child process budget in milliseconds.", 1, None),
                ),
                ("reason", reason_parameter()),
            ],
            false,
        ),
        "process.approval.record" => object_parameters(
            &["approval_note"],
            &[
                (
                    "preview_id",
                    string_parameter("Preview id that the host/user approved."),
                ),
                (
                    "tx_id",
                    string_parameter("Preview transaction id that the host/user approved."),
                ),
                (
                    "approval_note",
                    string_parameter(
                        "Host/user approval note. The model must not invent approval.",
                    ),
                ),
                ("reason", reason_parameter()),
            ],
            false,
        ),
        "process.pending_approvals" => {
            object_parameters(&[], &[("reason", reason_parameter())], false)
        }
        "process.clarify" => object_parameters(
            &[],
            &[
                ("question", string_parameter("Question to ask the user.")),
                (
                    "missing_fact",
                    string_parameter("Specific missing fact that blocks progress."),
                ),
                ("reason", reason_parameter()),
            ],
            false,
        ),
        "process.fail" => object_parameters(
            &[],
            &[
                ("error_code", string_parameter("Stable failure code.")),
                ("reason", reason_parameter()),
            ],
            false,
        ),
        "process.complete" => object_parameters(
            &["completion_statement"],
            &[
                (
                    "completion_statement",
                    string_parameter("User-facing completion statement grounded in receipts."),
                ),
                (
                    "claimed_artifacts",
                    string_array_parameter("Workspace artifacts claimed as final deliverables."),
                ),
                (
                    "key_sources",
                    string_array_parameter("Key source refs or paths used to complete the task."),
                ),
                (
                    "known_limitations",
                    string_array_parameter("Known limitations that remain true at completion."),
                ),
                (
                    "user_review_notes",
                    string_array_parameter("Notes useful for human acceptance review."),
                ),
                ("reason", reason_parameter()),
            ],
            false,
        ),
        "source_set.create" => object_parameters(
            &[],
            &[
                (
                    "root_path",
                    string_parameter("Workspace-relative root path; defaults to workspace root."),
                ),
                (
                    "include_extensions",
                    string_array_parameter("File extensions to include, such as .md or .rs."),
                ),
                (
                    "include_globs",
                    string_array_parameter("Glob-like include patterns."),
                ),
                (
                    "exclude_globs",
                    string_array_parameter("Glob-like exclude patterns."),
                ),
                (
                    "max_depth",
                    integer_parameter("Maximum directory depth to index.", 1, Some(128)),
                ),
                ("reason", reason_parameter()),
            ],
            false,
        ),
        "source_set.read_page" => object_parameters(
            &["source_set_ref"],
            &[
                (
                    "source_set_ref",
                    string_parameter("SourceSetRef returned by source_set.create."),
                ),
                (
                    "offset",
                    integer_parameter("Item offset for the page.", 0, None),
                ),
                (
                    "limit",
                    integer_parameter("Maximum source-set items to return.", 1, Some(1_000)),
                ),
                ("reason", reason_parameter()),
            ],
            false,
        ),
        "dataset.read_page" => object_parameters(
            &["dataset_ref"],
            &[
                (
                    "dataset_ref",
                    string_parameter("DataSetRef to page through."),
                ),
                (
                    "offset",
                    integer_parameter("Row offset for the page.", 0, None),
                ),
                (
                    "limit",
                    integer_parameter("Maximum dataset rows to return.", 1, Some(1_000)),
                ),
                ("reason", reason_parameter()),
            ],
            false,
        ),
        "source_set.coverage_verify" | "workspace.batch_hash" | "workspace.find_duplicates" => {
            object_parameters(
                &["source_set_ref"],
                &[
                    (
                        "source_set_ref",
                        string_parameter("SourceSetRef to process."),
                    ),
                    ("reason", reason_parameter()),
                ],
                false,
            )
        }
        "workspace.recent_changes" | "workspace.recent_changes_snapshot" => object_parameters(
            &["source_set_ref"],
            &[
                (
                    "source_set_ref",
                    string_parameter("SourceSetRef to inspect."),
                ),
                (
                    "days",
                    integer_parameter("Lookback window in days.", 1, Some(3650)),
                ),
                ("reason", reason_parameter()),
            ],
            false,
        ),
        "workspace.plan_organize" => object_parameters(
            &["source_set_ref"],
            &[
                (
                    "source_set_ref",
                    string_parameter("SourceSetRef to plan organization for."),
                ),
                (
                    "destination_root",
                    string_parameter("Workspace-relative destination root for the plan."),
                ),
                ("reason", reason_parameter()),
            ],
            false,
        ),
        "workspace.apply_organize_tx" => object_parameters(
            &["organize_plan_ref"],
            &[
                (
                    "organize_plan_ref",
                    string_parameter(
                        "Workspace organize plan ref returned by workspace.plan_organize.",
                    ),
                ),
                ("reason", reason_parameter()),
            ],
            false,
        ),
        "workspace.rename_batch_preview" => object_parameters(
            &["mappings"],
            &[
                (
                    "mappings",
                    json!({
                        "type": "array",
                        "items": rename_mapping_parameter(),
                        "description": "Rename mappings to preview."
                    }),
                ),
                (
                    "target_paths",
                    string_array_parameter("Workspace paths affected by the preview."),
                ),
                ("reason", reason_parameter()),
            ],
            false,
        ),
        "workspace.rename_batch_apply" => object_parameters(
            &["rename_plan_ref"],
            &[
                (
                    "rename_plan_ref",
                    string_parameter("Rename plan ref returned by workspace.rename_batch_preview."),
                ),
                ("reason", reason_parameter()),
            ],
            false,
        ),
        "workspace.tree_index" => object_parameters(
            &["source_set_ref"],
            &[
                ("source_set_ref", string_parameter("SourceSetRef to index.")),
                (
                    "tree_path",
                    string_parameter("Optional workspace-relative output path for the tree index."),
                ),
                ("reason", reason_parameter()),
            ],
            false,
        ),
        "workspace.perf_inventory" => object_parameters(
            &["source_set_ref"],
            &[
                (
                    "source_set_ref",
                    string_parameter("SourceSetRef to summarize for perf inventory."),
                ),
                (
                    "output_path",
                    string_parameter("Optional workspace-relative output artifact path."),
                ),
                ("reason", reason_parameter()),
            ],
            false,
        ),
        "dataset.export_csv" | "dataset.export_markdown" => object_parameters(
            &["dataset_ref", "output_path"],
            &[
                ("dataset_ref", string_parameter("DataSetRef to export.")),
                (
                    "output_path",
                    string_parameter("Workspace-relative output artifact path."),
                ),
                ("path", string_parameter("Alias for output_path.")),
                (
                    "title",
                    string_parameter("Optional markdown title; ignored for CSV export."),
                ),
                ("reason", reason_parameter()),
            ],
            false,
        ),
        "dataset.coverage_verify" => object_parameters(
            &["dataset_ref"],
            &[
                ("dataset_ref", string_parameter("DataSetRef to verify.")),
                ("reason", reason_parameter()),
            ],
            false,
        ),
        "artifact.inspect" | "artifact.audit_readonly" => object_parameters(
            &["path"],
            &[
                (
                    "path",
                    string_parameter("Workspace-relative artifact path or artifact:// ref."),
                ),
                (
                    "max_preview_bytes",
                    integer_parameter("Maximum text bytes to preview.", 0, Some(65_536)),
                ),
                ("reason", reason_parameter()),
            ],
            false,
        ),
        "artifact.copy_source_set" => object_parameters(
            &["source_set_ref", "destination_dir"],
            &[
                (
                    "source_set_ref",
                    string_parameter("SourceSetRef to copy into artifacts."),
                ),
                (
                    "destination_dir",
                    string_parameter("Workspace-relative destination directory."),
                ),
                ("reason", reason_parameter()),
            ],
            false,
        ),
        "artifact.verify_coverage" | "artifact.source_coverage_verify" => object_parameters(
            &["artifact_path"],
            &[
                (
                    "artifact_path",
                    string_parameter("Workspace-relative artifact path to verify."),
                ),
                ("path", string_parameter("Alias for artifact_path.")),
                (
                    "source_set_ref",
                    string_parameter("Optional SourceSetRef expected to ground the artifact."),
                ),
                (
                    "dataset_ref",
                    string_parameter("Optional DataSetRef expected to ground the artifact."),
                ),
                (
                    "coverage_contract",
                    json_object_parameter("Optional structured coverage contract."),
                ),
                (
                    "contract",
                    json_object_parameter("Alias for coverage_contract."),
                ),
                ("reason", reason_parameter()),
            ],
            false,
        ),
        "artifact.verify_typed" => object_parameters(
            &["artifact_path"],
            &[
                (
                    "artifact_path",
                    string_parameter("Workspace-relative artifact path to type-check."),
                ),
                ("path", string_parameter("Alias for artifact_path.")),
                ("reason", reason_parameter()),
            ],
            false,
        ),
        "artifact.audit_quality" => object_parameters(
            &["artifact_path"],
            &[
                (
                    "artifact_path",
                    string_parameter("Workspace-relative artifact path to audit."),
                ),
                ("path", string_parameter("Alias for artifact_path.")),
                (
                    "minimum_chars",
                    integer_parameter(
                        "Minimum text length expected for non-empty artifacts.",
                        1,
                        None,
                    ),
                ),
                (
                    "require_source_refs",
                    boolean_parameter("Whether the artifact must contain source references."),
                ),
                ("reason", reason_parameter()),
            ],
            false,
        ),
        "process.request_preview" | "process.preview.create" => object_parameters(
            &["operations", "preview_markdown"],
            &[
                (
                    "preview_markdown",
                    string_parameter("Human-readable preview markdown to show before approval."),
                ),
                (
                    "risk_level",
                    string_parameter("Low, medium, or high risk label."),
                ),
                (
                    "operations",
                    json!({
                        "type": "array",
                        "items": {
                            "type": "object",
                            "required": ["capability_id", "target_paths"],
                            "properties": {
                                "capability_id": {"type": "string"},
                                "arguments": {"type": "object"},
                                "target_paths": {"type": "array", "items": {"type": "string"}},
                                "human_description": {"type": "string"},
                                "rollback_policy": {"type": "string"}
                            },
                            "additionalProperties": false
                        },
                        "description": "Executable operations requested for approval."
                    }),
                ),
                ("reason", reason_parameter()),
            ],
            false,
        ),
        "os.write_source_mutation_preview" => {
            let mut properties = vec![
                (
                    "path",
                    string_parameter("Workspace-relative source path to preview mutating."),
                ),
                ("reason", reason_parameter()),
            ];
            properties.extend(content_properties());
            object_parameters_with_any_of(&["path"], &properties, content_any_of(), false)
        }
        "office.docx.rewrite_in_place_preview" | "office.docx.rewrite_preview" => {
            let mut properties = vec![
                (
                    "input_path",
                    string_parameter("Workspace-relative DOCX path to preview rewriting."),
                ),
                ("reason", reason_parameter()),
            ];
            properties.extend(content_properties());
            object_parameters_with_any_of(&["input_path"], &properties, content_any_of(), false)
        }
        "os.write_artifact" | "os.write_temp_dataset" => {
            let mut properties = vec![
                (
                    "path",
                    string_parameter(
                        "Workspace-relative path to write through the Kernel. For os.write_artifact this must be a text artifact path such as .md, .csv, .json, or .txt; .zip and .docx require package/office capabilities.",
                    ),
                ),
                ("reason", reason_parameter()),
            ];
            properties.extend(content_properties());
            object_parameters_with_any_of(&["path"], &properties, content_any_of(), false)
        }
        "os.write_source_mutation_apply" => {
            let mut properties = vec![
                (
                    "path",
                    string_parameter("Workspace-relative path to write through the Kernel."),
                ),
                ("reason", reason_parameter()),
            ];
            properties.extend(content_properties());
            object_parameters_with_any_of(&["path"], &properties, content_any_of(), false)
        }
        "os.write_file" => {
            let mut properties = vec![
                (
                    "path",
                    string_parameter("Workspace-relative path to write through the Kernel."),
                ),
                (
                    "write_kind",
                    json!({
                        "type": "string",
                        "enum": ["artifact", "source_mutation", "temp_dataset"],
                        "description": "Compatibility write kind. Prefer explicit write capabilities when possible."
                    }),
                ),
                ("reason", reason_parameter()),
            ];
            properties.extend(content_properties());
            object_parameters_with_any_of(
                &["path", "write_kind"],
                &properties,
                content_any_of(),
                false,
            )
        }
        "os.copy_path" => object_parameters(
            &["source_path", "destination_path"],
            &[
                (
                    "source_path",
                    string_parameter("Workspace-relative source path."),
                ),
                (
                    "destination_path",
                    string_parameter("Workspace-relative destination path."),
                ),
                ("reason", reason_parameter()),
            ],
            false,
        ),
        "os.move_path" | "os.rename_path" => object_parameters(
            &["source_path", "destination_path"],
            &[
                (
                    "source_path",
                    string_parameter("Workspace-relative source path."),
                ),
                (
                    "destination_path",
                    string_parameter("Workspace-relative destination path."),
                ),
                ("reason", reason_parameter()),
            ],
            false,
        ),
        "os.delete_path" => object_parameters(
            &["path"],
            &[
                (
                    "path",
                    string_parameter("Workspace-relative path to delete."),
                ),
                ("reason", reason_parameter()),
            ],
            false,
        ),
        "office.inspect_workbook" => object_parameters(
            &["path"],
            &[
                (
                    "path",
                    string_parameter("Workspace-relative workbook path or artifact:// ref."),
                ),
                ("reason", reason_parameter()),
            ],
            false,
        ),
        "data.csv.read_dataset" => object_parameters(
            &["input_path"],
            &[
                (
                    "input_path",
                    string_parameter("Workspace-relative CSV path."),
                ),
                (
                    "has_header",
                    boolean_parameter("Whether the first row contains column names."),
                ),
                (
                    "max_rows",
                    integer_parameter("Maximum rows to ingest.", 1, Some(100_000)),
                ),
                ("reason", reason_parameter()),
            ],
            false,
        ),
        "office.workbook.read_text" | "office.workbook.read_cells" => object_parameters(
            &["input_path"],
            &[
                (
                    "input_path",
                    string_parameter("Workspace-relative XLSX path."),
                ),
                ("sheet", string_parameter("Optional worksheet name.")),
                (
                    "max_rows",
                    integer_parameter("Maximum rows per sheet.", 1, Some(10_000)),
                ),
                ("reason", reason_parameter()),
            ],
            false,
        ),
        "document.pdf.extract_text" => object_parameters(
            &["input_path"],
            &[
                (
                    "input_path",
                    string_parameter("Workspace-relative PDF path."),
                ),
                ("reason", reason_parameter()),
            ],
            false,
        ),
        "office.docx.read_text" | "office.docx.validate" => object_parameters(
            &["input_path"],
            &[
                (
                    "input_path",
                    string_parameter("Workspace-relative DOCX path."),
                ),
                ("reason", reason_parameter()),
            ],
            false,
        ),
        "office.docx.batch_read_text"
        | "office.docx.batch_extract_metadata"
        | "office.docx.batch_validate" => object_parameters(
            &["source_set_ref"],
            &[
                (
                    "source_set_ref",
                    string_parameter("SourceSetRef containing DOCX files."),
                ),
                ("reason", reason_parameter()),
            ],
            false,
        ),
        "office.docx.create" => {
            let mut properties = vec![
                (
                    "output_path",
                    string_parameter("Workspace-relative DOCX output path."),
                ),
                ("path", string_parameter("Alias for output_path.")),
                ("title", string_parameter("Optional DOCX title.")),
                ("reason", reason_parameter()),
            ];
            properties.extend(content_properties());
            object_parameters_with_any_of(&["output_path"], &properties, content_any_of(), false)
        }
        "office.docx.rewrite_save_as" => {
            let mut properties = vec![
                (
                    "input_path",
                    string_parameter("Workspace-relative source DOCX path."),
                ),
                (
                    "output_path",
                    string_parameter("Workspace-relative destination DOCX path."),
                ),
                ("reason", reason_parameter()),
            ];
            properties.extend(content_properties());
            object_parameters_with_any_of(
                &["input_path", "output_path"],
                &properties,
                content_any_of(),
                false,
            )
        }
        "office.docx.rewrite_in_place" => {
            let mut properties = vec![
                (
                    "input_path",
                    string_parameter("Workspace-relative DOCX path to rewrite in place."),
                ),
                ("reason", reason_parameter()),
            ];
            properties.extend(content_properties());
            object_parameters_with_any_of(&["input_path"], &properties, content_any_of(), false)
        }
        "office.docx.diff_summary" => object_parameters(
            &["before_path", "after_path"],
            &[
                (
                    "before_path",
                    string_parameter("Workspace-relative DOCX path before change."),
                ),
                (
                    "after_path",
                    string_parameter("Workspace-relative DOCX path after change."),
                ),
                ("reason", reason_parameter()),
            ],
            false,
        ),
        "package.build_zip" => object_parameters(
            &["source_set_ref", "destination_zip_path"],
            &[
                (
                    "source_set_ref",
                    string_parameter("SourceSetRef to package."),
                ),
                (
                    "destination_zip_path",
                    string_parameter("Workspace-relative destination zip path."),
                ),
                (
                    "manifest_path",
                    string_parameter("Optional workspace-relative manifest output path."),
                ),
                (
                    "checksums_path",
                    string_parameter("Optional workspace-relative checksums output path."),
                ),
                (
                    "perf_notes_path",
                    string_parameter("Optional workspace-relative performance notes output path."),
                ),
                (
                    "exclude_globs",
                    string_array_parameter("Glob-like package exclusions."),
                ),
                ("reason", reason_parameter()),
            ],
            false,
        ),
        "terminal.run_command" => object_parameters(
            &["argv", "timeout_ms"],
            &[
                (
                    "argv",
                    json!({
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "Command argv list. Do not pass a shell string."
                    }),
                ),
                (
                    "timeout_ms",
                    integer_parameter("Required command timeout in milliseconds. Choose a bounded value for this foreground command.", 1, Some(600_000)),
                ),
                (
                    "cwd",
                    string_parameter("Optional workspace-relative working directory."),
                ),
                (
                    "target_paths",
                    string_array_parameter("Workspace paths expected to be affected."),
                ),
                ("reason", reason_parameter()),
            ],
            false,
        ),
        "terminal.start_service" => object_parameters(
            &["service_id", "argv", "startup_timeout_ms"],
            &[
                (
                    "service_id",
                    string_parameter("Stable ASCII service identifier used for later status and stop calls."),
                ),
                (
                    "argv",
                    json!({
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "Service command argv list. Do not pass a shell string."
                    }),
                ),
                (
                    "startup_timeout_ms",
                    integer_parameter("Required startup readiness timeout in milliseconds.", 1, Some(600_000)),
                ),
                (
                    "health_check",
                    json!({
                        "type": "object",
                        "description": "Optional readiness check: {\"kind\":\"http\",\"url\":\"http://127.0.0.1:8000/\"} or {\"kind\":\"tcp\",\"port\":8000}.",
                        "properties": {
                            "kind": {"type": "string"},
                            "url": {"type": "string"},
                            "port": {"type": "integer", "minimum": 1, "maximum": 65535}
                        },
                        "additionalProperties": false
                    }),
                ),
                (
                    "expected_ports",
                    json!({
                        "type": "array",
                        "items": {"type": "integer", "minimum": 1, "maximum": 65535},
                        "description": "Optional localhost ports expected to become reachable."
                    }),
                ),
                ("reason", reason_parameter()),
            ],
            false,
        ),
        "terminal.stop_service" => object_parameters(
            &["service_id"],
            &[
                (
                    "service_id",
                    string_parameter("Service identifier returned or chosen for terminal.start_service."),
                ),
                ("reason", reason_parameter()),
            ],
            false,
        ),
        "terminal.service_status" => object_parameters(
            &["service_id"],
            &[(
                "service_id",
                string_parameter("Service identifier returned or chosen for terminal.start_service."),
            )],
            false,
        ),
        _ => generic_provider_tool_parameters(descriptor),
    }
}

fn provider_tool_strict_parameters_for_descriptor(
    descriptor: &CapabilityDescriptor,
    parameters: &Value,
    strict_mode_enabled: bool,
) -> Option<Value> {
    if !strict_mode_enabled || !provider_tool_strict_compatible(&descriptor.capability_id) {
        return None;
    }
    let strict_parameters = deepseek_strict_schema(parameters)?;
    provider_tool_schema_is_strict_compatible(&strict_parameters).then_some(strict_parameters)
}

pub fn provider_tool_schema_is_strict_compatible(schema: &Value) -> bool {
    deepseek_strict_schema(schema).is_some()
}

fn deepseek_strict_schema(schema: &Value) -> Option<Value> {
    let object = schema.as_object()?;
    let schema_type = object.get("type").and_then(Value::as_str);
    match schema_type {
        Some("object") => {
            if object
                .get("additionalProperties")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                return None;
            }
            let properties = object
                .get("properties")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            let mut strict_properties = serde_json::Map::new();
            let mut required = Vec::<String>::new();
            for (name, value) in properties {
                strict_properties.insert(name.clone(), deepseek_strict_schema(&value)?);
                required.push(name);
            }
            Some(json!({
                "type": "object",
                "required": required,
                "properties": strict_properties,
                "additionalProperties": false,
            }))
        }
        Some("array") => Some(json!({
            "type": "array",
            "items": deepseek_strict_schema(object.get("items")?)?,
        })),
        Some("string") | Some("integer") | Some("number") | Some("boolean") | Some("null") => {
            let mut primitive = serde_json::Map::new();
            primitive.insert("type".to_string(), Value::String(schema_type?.to_string()));
            if let Some(description) = object.get("description").and_then(Value::as_str) {
                primitive.insert(
                    "description".to_string(),
                    Value::String(description.to_string()),
                );
            }
            if let Some(enum_values) = object.get("enum").and_then(Value::as_array) {
                primitive.insert("enum".to_string(), Value::Array(enum_values.clone()));
            }
            Some(Value::Object(primitive))
        }
        _ => None,
    }
}

fn generic_provider_tool_parameters(descriptor: &CapabilityDescriptor) -> Value {
    let input = descriptor.input_schema.to_ascii_lowercase();
    let target = descriptor.target_path_schema.to_ascii_lowercase();
    let mut properties = serde_json::Map::new();
    properties.insert("reason".to_string(), json!({"type": "string"}));
    properties.insert(
        "raw_arguments".to_string(),
        json!({
            "type": "object",
            "description": "Optional capability-specific structured arguments when no stricter schema is available."
        }),
    );
    let mut required = Vec::<String>::new();
    let mut add = |name: &str, schema: Value, is_required: bool| {
        properties.insert(name.to_string(), schema);
        if is_required && !required.iter().any(|item| item == name) {
            required.push(name.to_string());
        }
    };
    if input.contains("sourcesetref")
        || input.contains("source_set")
        || target.contains("source_set")
    {
        add("source_set_ref", json!({"type": "string"}), false);
    }
    if input.contains("datasetref") || input.contains("data_set") || target.contains("dataset") {
        add("dataset_ref", json!({"type": "string"}), false);
    }
    if input.contains("artifactpath") || input.contains("artifact") {
        add("artifact_path", json!({"type": "string"}), false);
    }
    if input.contains("docxref")
        || input.contains("docx")
        || descriptor.capability_id.contains(".docx.")
    {
        add("input_path", json!({"type": "string"}), false);
    }
    if target.contains("output_path") || input.contains("outputpath") {
        add("output_path", json!({"type": "string"}), false);
    }
    if target.contains("destination") || input.contains("destination") {
        add("destination_path", json!({"type": "string"}), false);
        add("destination_dir", json!({"type": "string"}), false);
    }
    if target.contains("path") || input.contains("workspacepath") || input.contains("artifactpath")
    {
        add("path", json!({"type": "string"}), false);
    }
    if input.contains("txid") || input.contains("tx_id") {
        add("tx_id", json!({"type": "string"}), false);
    }
    if input.contains("rawresultref")
        || input.contains("typedref")
        || input.contains("receiptref")
        || input.contains("ref")
    {
        add("ref", json!({"type": "string"}), false);
        add("raw_result_ref", json!({"type": "string"}), false);
        add("receipt_ref", json!({"type": "string"}), false);
    }
    if input.contains("offset") {
        add("offset", json!({"type": "integer", "minimum": 0}), false);
    }
    if input.contains("limit") {
        add("limit", json!({"type": "integer", "minimum": 1}), false);
        add(
            "limit_bytes",
            json!({"type": "integer", "minimum": 1}),
            false,
        );
    }
    if input.contains("days") {
        add("days", json!({"type": "integer", "minimum": 1}), false);
    }
    json!({
        "type": "object",
        "required": required,
        "properties": properties,
        "additionalProperties": true,
        "x-supernova-input_schema": provider_visible_input_schema(&descriptor.input_schema),
        "x-supernova-target_path_schema": descriptor.target_path_schema,
        "x-supernova-approval_policy": descriptor.approval_policy,
    })
}

fn provider_visible_input_schema(input_schema: &str) -> String {
    input_schema
        .replace(" + approval_id", "")
        .replace("approval_id + ", "")
        .replace(" + ApprovalToken", "")
        .replace("ApprovalToken + ", "")
}
