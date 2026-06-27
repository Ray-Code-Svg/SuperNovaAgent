use std::io;
use std::path::Path;

use serde_json::{json, Value};

use crate::agent_container::{AgentContainer, ContainerTimelineItem, ContainerTimelineItemKind};
use crate::container_store::ContainerStore;
use crate::context_compaction::{
    ContextCheckpointReceipt, ContextCompactionInput, ContextCompactionReceipt,
    ProviderTranscriptReplacement,
};
use crate::context_pack::ContextPack;
use crate::context_pack::{ContextPackIncludeMode, ContextPackItem, ContextPackItemKind};
use crate::context_window::{
    ContextScope, ContextWindowEstimate, ContextWindowEvent, ContextWindowRequestParts,
    ContextWindowScopeAdapter,
};
use crate::{json_err, now_ms, ChatTruthStore, MemoryBinding, ProcessTruthStore};

#[derive(Clone, Debug)]
pub struct ContainerContextWindowAdapter {
    store: ContainerStore,
    container: AgentContainer,
    timeline: Vec<ContainerTimelineItem>,
    memories: Vec<MemoryBinding>,
    context_pack: Option<ContextPack>,
    target_runtime: String,
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::context_pack::ContextPackAutoPolicy;

    use super::*;

    #[test]
    fn context_pack_visible_payload_resolves_task_run_process_truth() {
        let workspace = temp_workspace_root("context_pack_visible_payload_task_run");
        let state_root = workspace.join(".supernova_v2");
        let (job, process, truth) =
            crate::create_agent_job_with_state_root(&workspace, &state_root, "Summarize").unwrap();
        truth
            .append_event(
                Some(&process.pid),
                "job_completed",
                json!({
                    "status": "completed",
                    "artifacts": ["out.md"],
                }),
            )
            .unwrap();
        let pack = ContextPack {
            context_pack_id: "context_pack_1".to_string(),
            container_id: "container_1".to_string(),
            selected_items: vec![ContextPackItem {
                item_kind: ContextPackItemKind::TaskRun,
                ref_id: job.job_id.clone(),
                label: Some("Summarize".to_string()),
                include_mode: ContextPackIncludeMode::Summary,
                priority: 50,
            }],
            excluded_items: Vec::new(),
            auto_policy: ContextPackAutoPolicy::default(),
            summary_ref: None,
            estimated_tokens: None,
        };

        let payload = build_context_pack_visible_payload(&workspace, &state_root, &pack).unwrap();
        let resolved = &payload["resolved_items"][0];
        assert_eq!(resolved["resolution"], "process_truth_replay");
        assert_eq!(resolved["task"]["status"], "completed");
        assert_eq!(resolved["task"]["artifact_refs"][0], "out.md");

        fs::remove_dir_all(workspace).unwrap();
    }

    #[test]
    fn context_pack_visible_payload_marks_chat_turn_refs_readable() {
        let workspace = temp_workspace_root("context_pack_visible_payload_chat_turn");
        let state_root = workspace.join(".supernova_v2");
        let chat_truth =
            crate::ChatTruthStore::new_with_state_root(&workspace, &state_root).unwrap();
        let thread = chat_truth
            .create_thread("container_1", Some("Prior chat".to_string()))
            .unwrap();
        let turn_id = "chat_turn_context_payload";
        let user_ref = chat_truth
            .write_chat_blob(
                &thread.chat_thread_id,
                "turns/user.txt",
                b"context question",
            )
            .unwrap();
        let assistant_ref = chat_truth
            .write_chat_blob(
                &thread.chat_thread_id,
                "turns/assistant.txt",
                b"context answer",
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
        let pack = ContextPack {
            context_pack_id: "context_pack_1".to_string(),
            container_id: "container_1".to_string(),
            selected_items: vec![ContextPackItem {
                item_kind: ContextPackItemKind::ChatTurn,
                ref_id: turn_id.to_string(),
                label: Some("Prior chat".to_string()),
                include_mode: ContextPackIncludeMode::RefOnly,
                priority: 50,
            }],
            excluded_items: Vec::new(),
            auto_policy: ContextPackAutoPolicy::default(),
            summary_ref: None,
            estimated_tokens: None,
        };

        let payload = build_context_pack_visible_payload(&workspace, &state_root, &pack).unwrap();
        let resolved = &payload["resolved_items"][0];
        assert_eq!(resolved["resolution"], "chat_truth_ref_readable");
        assert_eq!(resolved["read_with"]["capability_id"], "process.read_ref");
        assert_eq!(resolved["read_with"]["ref"], turn_id);
        assert!(resolved["content_preview"]
            .as_str()
            .unwrap()
            .contains("context answer"));

        fs::remove_dir_all(workspace).unwrap();
    }

    fn temp_workspace_root(name: &str) -> std::path::PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("supernova_{name}_{unique}"));
        fs::create_dir_all(&root).unwrap();
        root
    }
}

impl ContainerContextWindowAdapter {
    pub fn new(
        store: ContainerStore,
        container: AgentContainer,
        timeline: Vec<ContainerTimelineItem>,
        memories: Vec<MemoryBinding>,
        context_pack: Option<ContextPack>,
        target_runtime: impl Into<String>,
    ) -> Self {
        Self {
            store,
            container,
            timeline,
            memories,
            context_pack,
            target_runtime: target_runtime.into(),
        }
    }

    pub fn compaction_payload(&self) -> Value {
        let resolved_context_pack = match self
            .context_pack
            .as_ref()
            .map(|pack| {
                build_context_pack_visible_payload(
                    self.store.workspace_root(),
                    self.store.state_root(),
                    pack,
                )
            })
            .transpose()
        {
            Ok(Some(payload)) => payload,
            Ok(None) | Err(_) => Value::Null,
        };
        json!({
            "schema": "supernova_container_context_compaction_input.v1",
            "container_id": self.container.container_id.clone(),
            "timeline_items": self.timeline.clone(),
            "selected_memory_refs": self.memories.clone(),
            "context_pack": self.context_pack.clone(),
            "resolved_context_pack": resolved_context_pack,
            "target_runtime": self.target_runtime.clone(),
            "required_output": {
                "schema": "supernova_container_context_summary.v1",
                "container_id": self.container.container_id.clone(),
                "summary": "<provider-visible container context summary>",
                "important_decisions": [],
                "active_goals": [],
                "artifact_index": [],
                "source_refs": [],
                "task_refs": [],
                "chat_refs": [],
                "memory_refs": [],
                "known_constraints": [],
                "open_questions": []
            },
            "fact_boundary": "Container summaries are input context only. They do not replace ChatTruth or ProcessTruth facts.",
        })
    }
}

pub fn build_context_pack_visible_payload(
    workspace_root: impl AsRef<Path>,
    state_root: impl AsRef<Path>,
    pack: &ContextPack,
) -> io::Result<Value> {
    let workspace_root = workspace_root.as_ref();
    let state_root = state_root.as_ref();
    let resolved_items = pack
        .selected_items
        .iter()
        .map(|item| resolve_context_pack_item(workspace_root, state_root, item))
        .collect::<Vec<_>>();
    Ok(json!({
        "schema": "supernova_context_pack_visible_payload.v1",
        "container_id": pack.container_id.clone(),
        "context_pack_id": pack.context_pack_id.clone(),
        "selected_item_count": pack.selected_items.len(),
        "excluded_item_count": pack.excluded_items.len(),
        "auto_policy": pack.auto_policy.clone(),
        "summary_ref": pack.summary_ref.clone(),
        "estimated_tokens": pack.estimated_tokens,
        "selected_items": pack.selected_items.clone(),
        "resolved_items": resolved_items,
        "fact_boundary": "Context pack items are provider-visible input context only. They do not replace ChatTruth, ProcessTruth, receipts, or Kernel policy.",
    }))
}

fn resolve_context_pack_item(
    workspace_root: &Path,
    state_root: &Path,
    item: &ContextPackItem,
) -> Value {
    match &item.item_kind {
        ContextPackItemKind::TaskRun => {
            resolve_task_run_context_item(workspace_root, state_root, item)
        }
        ContextPackItemKind::ChatThread => {
            resolve_chat_thread_context_item(workspace_root, state_root, item)
        }
        ContextPackItemKind::ChatTurn => {
            resolve_chat_turn_context_item(workspace_root, state_root, item)
        }
        _ => json!({
            "item_kind": &item.item_kind,
            "ref_id": item.ref_id.clone(),
            "label": item.label.clone(),
            "include_mode": include_mode_label(&item.include_mode),
            "priority": item.priority,
            "resolution": "ref_only",
        }),
    }
}

fn resolve_task_run_context_item(
    workspace_root: &Path,
    state_root: &Path,
    item: &ContextPackItem,
) -> Value {
    let truth =
        match ProcessTruthStore::new_with_state_root(workspace_root, state_root, &item.ref_id) {
            Ok(truth) => truth,
            Err(err) => {
                return unresolved_context_item(item, "process_truth_open_failed", err);
            }
        };
    let replay = match truth.replay() {
        Ok(replay) => replay,
        Err(err) => {
            return unresolved_context_item(item, "process_truth_replay_failed", err);
        }
    };
    let events = truth.read_events().unwrap_or_default();
    let terminal_event = events
        .iter()
        .rev()
        .find(|event| {
            matches!(
                event.event_type.as_str(),
                "job_completed"
                    | "job_blocked"
                    | "job_failed"
                    | "job_interrupted_by_model_protocol_error"
                    | "completion_statement_recorded"
                    | "process.complete"
            )
        })
        .map(|event| {
            json!({
                "event_id": event.event_id,
                "event_type": event.event_type.clone(),
                "data": event.data.clone(),
            })
        });
    json!({
        "item_kind": &item.item_kind,
        "ref_id": item.ref_id.clone(),
        "label": item.label.clone(),
        "include_mode": include_mode_label(&item.include_mode),
        "priority": item.priority,
        "resolution": "process_truth_replay",
        "task": {
            "job_id": replay.job_id,
            "status": replay.status,
            "event_count": replay.event_count,
            "artifact_refs": replay.artifact_refs,
            "artifact_provenance": replay.artifact_provenance,
            "terminal_event": terminal_event,
        }
    })
}

fn resolve_chat_thread_context_item(
    workspace_root: &Path,
    state_root: &Path,
    item: &ContextPackItem,
) -> Value {
    let chat_truth = match ChatTruthStore::new_with_state_root(workspace_root, state_root) {
        Ok(chat_truth) => chat_truth,
        Err(err) => {
            return unresolved_context_item(item, "chat_truth_open_failed", err);
        }
    };
    let thread = chat_truth.get_thread(&item.ref_id).ok();
    let events = chat_truth.read_events(&item.ref_id).unwrap_or_default();
    let recent_events = events
        .iter()
        .rev()
        .take(16)
        .map(|event| {
            json!({
                "event_seq": event.event_seq,
                "event_type": event.event_type.clone(),
                "payload": event.payload.clone(),
                "blob_ref": event.blob_ref.clone(),
            })
        })
        .collect::<Vec<_>>();
    json!({
        "item_kind": &item.item_kind,
        "ref_id": item.ref_id.clone(),
        "label": item.label.clone(),
        "include_mode": include_mode_label(&item.include_mode),
        "priority": item.priority,
        "resolution": "chat_truth_recent_events",
        "chat_thread": thread,
        "recent_events_newest_first": recent_events,
    })
}

fn resolve_chat_turn_context_item(
    workspace_root: &Path,
    state_root: &Path,
    item: &ContextPackItem,
) -> Value {
    let chat_truth = match ChatTruthStore::new_with_state_root(workspace_root, state_root) {
        Ok(chat_truth) => chat_truth,
        Err(err) => {
            return unresolved_context_item(item, "chat_truth_open_failed", err);
        }
    };
    match chat_truth.read_chat_ref_text(&item.ref_id) {
        Ok(content) => json!({
            "item_kind": &item.item_kind,
            "ref_id": item.ref_id.clone(),
            "label": item.label.clone(),
            "include_mode": include_mode_label(&item.include_mode),
            "priority": item.priority,
            "resolution": "chat_truth_ref_readable",
            "read_with": {
                "capability_id": "process.read_ref",
                "ref": item.ref_id.clone(),
            },
            "content_preview": content.chars().take(2048).collect::<String>(),
        }),
        Err(err) => unresolved_context_item(item, "chat_truth_ref_unreadable", err),
    }
}

fn unresolved_context_item(
    item: &ContextPackItem,
    reason: &str,
    error: impl std::fmt::Display,
) -> Value {
    json!({
        "item_kind": &item.item_kind,
        "ref_id": item.ref_id.clone(),
        "label": item.label.clone(),
        "include_mode": include_mode_label(&item.include_mode),
        "priority": item.priority,
        "resolution": "unresolved",
        "reason": reason,
        "error": error.to_string(),
    })
}

fn include_mode_label(mode: &ContextPackIncludeMode) -> &'static str {
    match mode {
        ContextPackIncludeMode::Full => "full",
        ContextPackIncludeMode::Summary => "summary",
        ContextPackIncludeMode::MetadataOnly => "metadata_only",
        ContextPackIncludeMode::RefOnly => "ref_only",
    }
}

impl ContextWindowScopeAdapter for ContainerContextWindowAdapter {
    fn scope(&self) -> ContextScope {
        ContextScope::Container
    }

    fn build_visible_request_parts(&self) -> io::Result<ContextWindowRequestParts> {
        Ok(ContextWindowRequestParts {
            provider: "local_estimator".to_string(),
            model: "container_context".to_string(),
            context_window_tokens: 128_000,
            context_pack_payload: self.compaction_payload(),
            reserved_output_tokens: Some(0),
            reserved_reasoning_tokens: Some(0),
            ..ContextWindowRequestParts::default()
        })
    }

    fn build_compaction_input(
        &self,
        estimate: &ContextWindowEstimate,
    ) -> io::Result<ContextCompactionInput> {
        let selected_refs = self
            .timeline
            .iter()
            .map(|item| item.ref_id.clone())
            .chain(self.memories.iter().map(|item| item.memory_ref.clone()))
            .chain(
                self.context_pack
                    .as_ref()
                    .and_then(|pack| pack.summary_ref.clone())
                    .into_iter(),
            )
            .collect::<Vec<_>>();
        Ok(ContextCompactionInput {
            schema: "supernova_container_context_compaction_input.v1".to_string(),
            scope: self.scope(),
            estimate: estimate.clone(),
            visible_context_ref: self
                .context_pack
                .as_ref()
                .map(|pack| format!("context_pack://{}", pack.context_pack_id)),
            selected_refs,
            live_suffix_refs: self
                .timeline
                .iter()
                .rev()
                .take(16)
                .map(|item| item.ref_id.clone())
                .collect::<Vec<_>>(),
            target_summary_tokens: self.container.context_policy.max_summary_tokens,
            payload: self.compaction_payload(),
        })
    }

    fn run_pre_compaction_checkpoint(
        &mut self,
        estimate: &ContextWindowEstimate,
    ) -> io::Result<ContextCheckpointReceipt> {
        let checkpoint_id = format!("container_context_checkpoint_{}", now_ms());
        let payload = json!({
            "schema": "supernova_container_context_pre_compaction_checkpoint.v1",
            "checkpoint_id": checkpoint_id,
            "scope": self.scope(),
            "estimate": estimate,
            "container": self.container.clone(),
            "timeline_items": self.timeline.clone(),
            "selected_memory_refs": self.memories.clone(),
            "context_pack": self.context_pack.clone(),
            "target_runtime": self.target_runtime.clone(),
        });
        let checkpoint_ref = self.store.write_container_blob(
            &self.container.container_id,
            &format!("compactions/{checkpoint_id}.json"),
            &serde_json::to_vec_pretty(&payload).map_err(json_err)?,
        )?;
        Ok(ContextCheckpointReceipt {
            checkpoint_id,
            scope: self.scope(),
            checkpoint_ref,
            created_at_ms: now_ms() as i64,
        })
    }

    fn replace_visible_context(
        &mut self,
        compaction: ContextCompactionReceipt,
    ) -> io::Result<ProviderTranscriptReplacement> {
        if let Some(summary_ref) = compaction.summary_ref.clone() {
            if let Some(mut pack) = self.context_pack.clone() {
                pack.summary_ref = Some(summary_ref.clone());
                self.store.upsert_context_pack(pack)?;
            }
            self.store.append_timeline_item(
                &self.container.container_id,
                ContainerTimelineItemKind::ContextCompaction,
                compaction.compaction_id.clone(),
                compaction.status.clone(),
                Some("Container context compaction".to_string()),
                Some(summary_ref.clone()),
            )?;
        }
        Ok(ProviderTranscriptReplacement {
            old_transcript_ref: compaction
                .live_suffix_ref
                .clone()
                .unwrap_or_else(|| "container_context://pre_compaction".to_string()),
            new_transcript_ref: compaction
                .summary_ref
                .clone()
                .unwrap_or_else(|| "container_context://unchanged".to_string()),
            summary_ref: compaction.summary_ref,
            live_suffix_ref: compaction.live_suffix_ref,
            compacted_until_message_index: compaction.compacted_until_message_index,
        })
    }

    fn append_context_event(&mut self, event: ContextWindowEvent) -> io::Result<()> {
        self.store.write_container_blob(
            &self.container.container_id,
            &format!(
                "context_window_events/{}_{}.json",
                crate::safe_blob_name(&event.event_type),
                now_ms()
            ),
            &serde_json::to_vec_pretty(&event).map_err(json_err)?,
        )?;
        Ok(())
    }
}
