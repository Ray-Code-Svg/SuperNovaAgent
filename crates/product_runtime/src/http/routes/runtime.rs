use axum::extract::{Query, State};
use axum::response::IntoResponse;
use axum::Json;
use local_runtime_protocol::{
    Cursor, ProtocolResponse, RuntimeHealth, RuntimeMeta, StreamOpenRequest, UiCapabilityManifest,
};
use serde_json::json;

use crate::app_state::ProductRuntimeState;
use crate::error::{RuntimeError, RuntimeResult};
use crate::http::sse;

pub async fn meta(State(state): State<ProductRuntimeState>) -> Json<ProtocolResponse<RuntimeMeta>> {
    let workspace_uid = state.workspace_uid();
    Json(ProtocolResponse::new(
        "req_runtime_meta",
        workspace_uid.clone(),
        "runtime.meta",
        RuntimeMeta::rust_product_runtime(
            state.workspace_root().to_string_lossy().to_string(),
            workspace_uid,
        ),
    ))
}

pub async fn health(
    State(state): State<ProductRuntimeState>,
) -> Json<ProtocolResponse<RuntimeHealth>> {
    let workspace_uid = state.workspace_uid();
    Json(ProtocolResponse::new(
        "req_runtime_health",
        workspace_uid.clone(),
        "runtime.health",
        RuntimeHealth {
            status: "ready".into(),
            runtime_layer: "rust_product_runtime".into(),
            workspace_id: workspace_uid,
            uptime_ms: state.started_at.elapsed().as_millis(),
        },
    ))
}

pub async fn capabilities(
    State(state): State<ProductRuntimeState>,
) -> Json<ProtocolResponse<UiCapabilityManifest>> {
    let workspace_uid = state.workspace_uid();
    let services = state.services();
    let model_config = services.model_config.descriptor();
    let manifest = services.capability_manifest.manifest(model_config);
    Json(ProtocolResponse::new(
        "req_runtime_capabilities",
        workspace_uid,
        "runtime.capabilities",
        manifest,
    ))
}

pub async fn events(
    State(state): State<ProductRuntimeState>,
    Query(query): Query<StreamOpenRequest>,
) -> RuntimeResult<impl IntoResponse> {
    let workspace_uid = state.workspace_uid();
    let services = state.services();
    let mut events = Vec::new();

    if query.after_event_id.unwrap_or(-1) < 0 {
        events.push(sse::protocol_payload_event(
            "runtime.ready",
            &workspace_uid,
            Cursor {
                kind: "runtime".into(),
                after: Some("ready".into()),
                after_event_id: Some(0),
            },
            json!({
                "summary": "Runtime ready",
                "message": null,
                "record": {
                    "status": "ready",
                    "runtime_layer": "rust_product_runtime",
                    "kernel_layer": "rust_process_kernel",
                    "workspace_id": workspace_uid,
                    "uptime_ms": state.started_at.elapsed().as_millis()
                }
            }),
        ));
    }

    let after_event_id = query.after_event_id.filter(|value| *value >= 0);
    let messages = services
        .container
        .runtime_messages_page(after_event_id, query.limit)
        .map_err(|err| RuntimeError::internal(workspace_uid.clone(), err.to_string()))?;
    for message in messages {
        let event_type = sse::event_type_for_message(&message);
        events.push(sse::protocol_message_event(
            event_type,
            &workspace_uid,
            json!({ "message": message.clone() }),
            Some(&message),
        ));
    }
    Ok(sse::event_batch(events))
}
