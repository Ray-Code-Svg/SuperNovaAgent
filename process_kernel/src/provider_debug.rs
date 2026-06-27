use std::fs::OpenOptions;
use std::io::{self, Write};

use serde_json::{json, Map, Value};

use crate::{now_ms, ProcessTruthStore};

const PROVIDER_DEBUG_ENV: &[&str] = &[
    "SUPERNOVA_PROVIDER_DEBUG",
    "SUPERNOVA_PROVIDER_NATIVE_DEBUG",
];

const PROVIDER_DEBUG_TRACE_NAME: &str = "provider_debug/provider_native_debug.jsonl";

pub fn provider_debug_enabled() -> bool {
    PROVIDER_DEBUG_ENV.iter().any(|name| {
        std::env::var(name)
            .ok()
            .map(|value| {
                matches!(
                    value.trim().to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on" | "native" | "provider_native"
                )
            })
            .unwrap_or(false)
    })
}

pub fn append_provider_native_debug(
    truth: &ProcessTruthStore,
    phase: &str,
    payload: Value,
) -> io::Result<Option<String>> {
    if !provider_debug_enabled() {
        return Ok(None);
    }
    let mut record = match payload {
        Value::Object(map) => map,
        other => {
            let mut map = Map::new();
            map.insert("diagnostic".to_string(), other);
            map
        }
    };
    record.insert("event".to_string(), json!("provider_native_debug"));
    record.insert("phase".to_string(), json!(phase));
    record.insert("timestamp_ms".to_string(), json!(now_ms()));
    record.insert("job_id".to_string(), json!(truth.job_id()));
    record.insert(
        "trace_ref".to_string(),
        json!(format!(
            "blob://{}/{}",
            truth.job_id(),
            PROVIDER_DEBUG_TRACE_NAME
        )),
    );

    let trace_path = truth
        .state_root()
        .join("blobs")
        .join(truth.job_id())
        .join(PROVIDER_DEBUG_TRACE_NAME);
    if let Some(parent) = trace_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&trace_path)?;
    let line = serde_json::to_string(&Value::Object(record))
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    writeln!(file, "{line}")?;
    Ok(Some(format!(
        "blob://{}/{}",
        truth.job_id(),
        PROVIDER_DEBUG_TRACE_NAME
    )))
}

pub fn json_shape(value: &Value) -> Value {
    match value {
        Value::Null => json!({"type": "null"}),
        Value::Bool(_) => json!({"type": "bool"}),
        Value::Number(_) => json!({"type": "number"}),
        Value::String(text) => {
            json!({
                "type": "string",
                "len": text.chars().count(),
                "class": classify_string(text),
                "sample": safe_sample(text),
            })
        }
        Value::Array(items) => {
            let sample = items.iter().take(5).map(json_shape).collect::<Vec<_>>();
            json!({
                "type": "array",
                "len": items.len(),
                "sample": sample,
            })
        }
        Value::Object(map) => {
            let mut fields = Map::new();
            for (key, item) in map {
                fields.insert(key.clone(), json_shape(item));
            }
            json!({
                "type": "object",
                "keys": map.keys().cloned().collect::<Vec<_>>(),
                "fields": fields,
            })
        }
    }
}

pub fn argument_shape(arguments: &Value) -> Value {
    json_shape(arguments)
}

pub fn classify_string(value: &str) -> &'static str {
    let trimmed = value.trim();
    if trimmed.starts_with("blob://") {
        if trimmed.contains("/source_sets/") {
            "source_set_blob_ref"
        } else if trimmed.contains("/datasets/") {
            "dataset_blob_ref"
        } else if trimmed.contains("/raw_tool_results/") {
            "raw_tool_result_blob_ref"
        } else if trimmed.contains("/provider_tool_results/") {
            "provider_tool_result_blob_ref"
        } else {
            "blob_ref"
        }
    } else if trimmed.starts_with("dataset://") {
        "dataset_ref"
    } else if trimmed.starts_with("artifact://") {
        "artifact_ref"
    } else if trimmed.starts_with('/') || trimmed.starts_with('\\') {
        "rooted_or_internal_path"
    } else if trimmed.contains("://") {
        "uri_like"
    } else if trimmed.contains("..") {
        "path_with_parent_segment"
    } else if trimmed.contains('/') || trimmed.contains('\\') || trimmed.contains('.') {
        "workspace_path_or_filename"
    } else {
        "text"
    }
}

fn safe_sample(value: &str) -> String {
    let trimmed = value.trim();
    let mut sample = trimmed.chars().take(160).collect::<String>();
    if trimmed.chars().count() > 160 {
        sample.push_str("...");
    }
    sample
}
