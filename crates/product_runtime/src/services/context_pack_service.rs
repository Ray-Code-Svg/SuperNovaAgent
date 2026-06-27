use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

use local_runtime_protocol::{ContextPack, ContextPackAutoPolicy, ContextPackEstimate};
use local_runtime_protocol::{ContextPackItem, SourceCandidate, SourceCandidateRequest};

use crate::kernel::KernelBridge;
use crate::state::product_db::ProductDb;
use crate::state::workspace_registry::now_ms;

#[derive(Clone)]
pub struct ContextPackService {
    db: ProductDb,
    kernel: KernelBridge,
    workspace_root: PathBuf,
}

impl ContextPackService {
    pub fn new(db: ProductDb, kernel: KernelBridge, workspace_root: PathBuf) -> Self {
        Self {
            db,
            kernel,
            workspace_root,
        }
    }

    pub fn current(&self, container_id: &str) -> rusqlite::Result<ContextPack> {
        Ok(self
            .db
            .get_context_pack(container_id)?
            .map(strip_materialized_auto_items)
            .unwrap_or_else(|| ContextPack {
                context_pack_id: String::new(),
                container_id: container_id.to_string(),
                selected_items: Vec::new(),
                excluded_items: Vec::new(),
                auto_policy: ContextPackAutoPolicy {
                    include_recent_chat_turns: 6,
                    include_recent_tasks: 3,
                    prefer_summaries: true,
                },
                summary_ref: None,
                estimated_tokens: Some(0),
            }))
    }

    pub fn save(&self, pack: ContextPack) -> rusqlite::Result<ContextPack> {
        let pack = strip_materialized_auto_items(pack);
        let mut pack = self
            .kernel
            .upsert_context_pack(pack)
            .map_err(|err| rusqlite::Error::ToSqlConversionFailure(Box::new(err)))?;
        let materialized = self
            .kernel
            .materialize_context_pack(pack.clone())
            .map_err(|err| rusqlite::Error::ToSqlConversionFailure(Box::new(err)))?;
        let (estimated_tokens, _, _) = self.estimate_values(&materialized);
        pack.estimated_tokens = Some(estimated_tokens);
        let pack = self
            .kernel
            .upsert_context_pack(pack)
            .map_err(|err| rusqlite::Error::ToSqlConversionFailure(Box::new(err)))?;
        self.db.save_context_pack(&pack)
    }

    pub fn estimate(&self, pack: ContextPack) -> ContextPackEstimate {
        let mut context_pack = strip_materialized_auto_items(pack);
        let materialized = self
            .kernel
            .materialize_context_pack(context_pack.clone())
            .unwrap_or_else(|_| context_pack.clone());
        let (estimated_tokens, context_window_tokens, usage_ratio) =
            self.estimate_values(&materialized);
        context_pack.estimated_tokens = Some(estimated_tokens);
        ContextPackEstimate {
            context_pack,
            estimated_tokens,
            context_window_tokens,
            usage_ratio,
        }
    }

    pub fn materialize_for_request(
        &self,
        container_id: &str,
        pack: Option<ContextPack>,
    ) -> rusqlite::Result<Option<ContextPack>> {
        let policy = match pack {
            Some(pack) => strip_materialized_auto_items(pack),
            None => self.current(container_id)?,
        };
        if policy.context_pack_id.trim().is_empty()
            && policy.selected_items.is_empty()
            && policy.excluded_items.is_empty()
            && policy.auto_policy.include_recent_chat_turns == 0
            && policy.auto_policy.include_recent_tasks == 0
        {
            return Ok(None);
        }
        let policy_id = if policy.context_pack_id.trim().is_empty() {
            "context_pack_policy".to_string()
        } else {
            policy.context_pack_id.clone()
        };
        let mut materialized = self
            .kernel
            .materialize_context_pack(policy)
            .map_err(|err| rusqlite::Error::ToSqlConversionFailure(Box::new(err)))?;
        materialized.context_pack_id = format!("{policy_id}_materialized_{}", now_ms());
        let (estimated_tokens, _, _) = self.estimate_values(&materialized);
        materialized.estimated_tokens = Some(estimated_tokens);
        let materialized = self
            .kernel
            .upsert_context_pack(materialized)
            .map_err(|err| rusqlite::Error::ToSqlConversionFailure(Box::new(err)))?;
        Ok(Some(materialized))
    }

    fn estimate_values(&self, pack: &ContextPack) -> (u64, u64, String) {
        let estimate = self.kernel.estimate_context_pack(pack).ok();
        extract_context_pack_estimate_values(estimate.as_ref())
    }

    pub fn source_candidates(
        &self,
        container_id: &str,
        request: SourceCandidateRequest,
    ) -> rusqlite::Result<Vec<SourceCandidate>> {
        let current = self.current(container_id)?;
        let selected_refs = current
            .selected_items
            .iter()
            .map(|item| (item.item_kind.clone(), item.ref_id.clone()))
            .collect::<HashSet<_>>();
        let mut items = Vec::new();
        self.workspace_source_candidates(&selected_refs, &request, &mut items);
        for thread in self.db.list_chat_threads(container_id)? {
            if !candidate_matches(
                request.q.as_deref(),
                &thread.chat_thread_id,
                Some(&thread.title),
            ) {
                continue;
            }
            push_candidate(
                &mut items,
                &selected_refs,
                "chat_thread",
                thread.chat_thread_id,
                Some(thread.title),
                "history",
                Some("Chat thread".into()),
                40,
            );
        }
        for task in self.db.list_tasks(container_id)? {
            if !candidate_matches(request.q.as_deref(), &task.task_id, Some(&task.title)) {
                continue;
            }
            push_candidate(
                &mut items,
                &selected_refs,
                "task_run",
                task.task_id,
                Some(task.title),
                "history",
                Some(task.status),
                30,
            );
        }
        let limit = normalized_limit(request.limit);
        items.truncate(limit);
        Ok(items)
    }

    fn workspace_source_candidates(
        &self,
        selected_refs: &HashSet<(String, String)>,
        request: &SourceCandidateRequest,
        items: &mut Vec<SourceCandidate>,
    ) {
        let query = request.q.as_deref();
        let limit = normalized_limit(request.limit);
        let mut scanned = 0usize;
        walk_workspace(
            &self.workspace_root,
            &self.workspace_root,
            query,
            limit,
            items,
            selected_refs,
            &mut scanned,
        );
    }
}

fn strip_materialized_auto_items(mut pack: ContextPack) -> ContextPack {
    pack.selected_items.retain(|item| {
        !(item.priority == 50
            && matches!(
                item.item_kind.as_str(),
                "chat_turn" | "task_run" | "artifact" | "container_summary"
            ))
    });
    pack
}

fn extract_context_pack_estimate_values(
    estimate: Option<&serde_json::Value>,
) -> (u64, u64, String) {
    let payload = estimate
        .and_then(|value| value.get("estimate"))
        .or(estimate);
    let estimated_tokens = payload
        .and_then(|value| value.get("estimated_tokens"))
        .or_else(|| payload.and_then(|value| value.get("estimated_input_tokens")))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let context_window_tokens = payload
        .and_then(|value| value.get("context_window_tokens"))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let usage_ratio = payload
        .and_then(|value| value.get("usage_ratio"))
        .and_then(serde_json::Value::as_f64)
        .map(|value| format!("{value:.3}"))
        .unwrap_or_else(|| {
            if context_window_tokens > 0 {
                format!(
                    "{:.3}",
                    estimated_tokens as f64 / context_window_tokens as f64
                )
            } else {
                "0".into()
            }
        });
    (estimated_tokens, context_window_tokens, usage_ratio)
}

fn walk_workspace(
    workspace_root: &Path,
    dir: &Path,
    query: Option<&str>,
    limit: usize,
    items: &mut Vec<SourceCandidate>,
    selected_refs: &HashSet<(String, String)>,
    scanned: &mut usize,
) {
    if items.len() >= limit || *scanned >= 20_000 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries = entries.flatten().collect::<Vec<_>>();
    entries.sort_by_key(|entry| {
        let path = entry.path();
        (
            !path.is_dir(),
            entry
                .file_name()
                .to_string_lossy()
                .to_string()
                .to_lowercase(),
        )
    });
    for entry in entries {
        if items.len() >= limit || *scanned >= 20_000 {
            return;
        }
        let path = entry.path();
        let Some(file_name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if is_hidden_or_internal(file_name) {
            continue;
        }
        *scanned += 1;
        let Ok(relative) = path.strip_prefix(workspace_root) else {
            continue;
        };
        let relative_label = relative.to_string_lossy().replace('\\', "/");
        if candidate_matches(query, &relative_label, Some(file_name)) {
            let detail = if path.is_dir() { "Directory" } else { "File" };
            push_candidate(
                items,
                selected_refs,
                "source_ref",
                format!("workspace://{}", sanitize_relative_source(relative)),
                Some(relative_label),
                if path.is_dir() {
                    "workspace_dir"
                } else {
                    "workspace_file"
                },
                Some(detail.into()),
                if path.is_dir() { 20 } else { 25 },
            );
        }
        if path.is_dir() {
            walk_workspace(
                workspace_root,
                &path,
                query,
                limit,
                items,
                selected_refs,
                scanned,
            );
        }
    }
}

fn push_candidate(
    items: &mut Vec<SourceCandidate>,
    selected_refs: &HashSet<(String, String)>,
    item_kind: &str,
    ref_id: String,
    label: Option<String>,
    source_kind: &str,
    detail: Option<String>,
    priority: u8,
) {
    let item = ContextPackItem {
        item_kind: item_kind.into(),
        ref_id,
        label,
        include_mode: "summary".into(),
        priority,
    };
    let selected = selected_refs.contains(&(item.item_kind.clone(), item.ref_id.clone()));
    items.push(SourceCandidate {
        item,
        source_kind: source_kind.into(),
        detail,
        selected,
    });
}

fn is_hidden_or_internal(file_name: &str) -> bool {
    let lower = file_name.to_lowercase();
    matches!(
        lower.as_str(),
        ".git"
            | ".supernova_v2"
            | "__pycache__"
            | "node_modules"
            | "target"
            | "dist"
            | "build"
            | "coverage"
            | "reports"
            | "artifacts"
            | "logs"
            | "log"
            | "tmp"
            | "temp"
            | "venv"
            | "%temp%"
    ) || file_name.starts_with('.')
        || lower.ends_with(".pyc")
        || lower.ends_with(".pyo")
}

fn sanitize_relative_source(relative: &Path) -> String {
    let mut clean = PathBuf::new();
    for component in relative.components() {
        if let Component::Normal(part) = component {
            clean.push(part);
        }
    }
    clean.to_string_lossy().replace('\\', "/")
}

fn candidate_matches(query: Option<&str>, ref_id: &str, label: Option<&str>) -> bool {
    let Some(query) = query.map(str::trim).filter(|value| !value.is_empty()) else {
        return true;
    };
    let query = query.to_lowercase();
    ref_id.to_lowercase().contains(&query)
        || label
            .map(|value| value.to_lowercase().contains(&query))
            .unwrap_or(false)
}

fn normalized_limit(limit: Option<usize>) -> usize {
    limit.unwrap_or(200).clamp(20, 500)
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;
    use serde_json::json;

    #[test]
    fn context_pack_estimate_parser_reads_nested_kernel_estimate() {
        let payload = json!({
            "container_id": "container_1",
            "context_pack_id": "context_pack_1",
            "estimate": {
                "estimated_input_tokens": 345,
                "context_window_tokens": 1000,
                "usage_ratio": 0.345
            }
        });

        assert_eq!(
            extract_context_pack_estimate_values(Some(&payload)),
            (345, 1000, "0.345".to_string())
        );
    }

    #[test]
    fn context_pack_estimate_parser_keeps_legacy_top_level_shape() {
        let payload = json!({
            "estimated_tokens": 120,
            "context_window_tokens": 600
        });

        assert_eq!(
            extract_context_pack_estimate_values(Some(&payload)),
            (120, 600, "0.200".to_string())
        );
    }

    #[test]
    fn materialize_for_request_uses_latest_timeline_without_polluting_policy() {
        let workspace_root = temp_workspace_root("context_policy_workspace");
        let state_root = temp_workspace_root("context_policy_state");
        let provider_root = temp_workspace_root("context_policy_provider");
        let db = ProductDb::open(&state_root, "workspace_context_policy".to_string()).unwrap();
        let kernel = KernelBridge::new(
            workspace_root.clone(),
            state_root.clone(),
            provider_root.clone(),
        );
        let container = kernel
            .create_container(Some("Context policy".into()), None, None)
            .unwrap();
        let service = ContextPackService::new(db, kernel.clone(), workspace_root.clone());
        let saved = service
            .save(ContextPack {
                context_pack_id: String::new(),
                container_id: container.container_id.clone(),
                selected_items: Vec::new(),
                excluded_items: Vec::new(),
                auto_policy: ContextPackAutoPolicy {
                    include_recent_chat_turns: 1,
                    include_recent_tasks: 0,
                    prefer_summaries: true,
                },
                summary_ref: None,
                estimated_tokens: None,
            })
            .unwrap();
        assert!(saved.selected_items.is_empty());

        let first = kernel
            .create_chat_thread(&container.container_id, Some("first".into()))
            .unwrap();
        let materialized = service
            .materialize_for_request(&container.container_id, None)
            .unwrap()
            .expect("request pack");
        assert_ne!(materialized.context_pack_id, saved.context_pack_id);
        assert!(materialized.context_pack_id.contains("_materialized_"));
        assert!(materialized
            .selected_items
            .iter()
            .any(|item| item.item_kind == "chat_turn" && item.ref_id == first.chat_thread_id));

        let current = service.current(&container.container_id).unwrap();
        assert_eq!(current.context_pack_id, saved.context_pack_id);
        assert!(current.selected_items.is_empty());

        let second = kernel
            .create_chat_thread(&container.container_id, Some("second".into()))
            .unwrap();
        let materialized = service
            .materialize_for_request(&container.container_id, None)
            .unwrap()
            .expect("request pack");
        assert!(materialized
            .selected_items
            .iter()
            .any(|item| item.item_kind == "chat_turn" && item.ref_id == second.chat_thread_id));
        assert!(!materialized
            .selected_items
            .iter()
            .any(|item| item.item_kind == "chat_turn" && item.ref_id == first.chat_thread_id));

        fs::remove_dir_all(workspace_root).unwrap();
        fs::remove_dir_all(state_root).unwrap();
        fs::remove_dir_all(provider_root).unwrap();
    }

    #[test]
    fn workspace_candidates_skip_internal_noise() {
        let root = temp_workspace_root("source_candidates_skip_internal_noise");
        fs::create_dir_all(root.join("visible")).unwrap();
        fs::write(root.join("visible").join("README.md"), "visible source").unwrap();
        fs::create_dir_all(root.join("__pycache__")).unwrap();
        fs::write(root.join("__pycache__").join("cached.pyc"), "cache").unwrap();
        fs::create_dir_all(root.join("%TEMP%")).unwrap();
        fs::write(root.join("%TEMP%").join("scratch.txt"), "scratch").unwrap();
        fs::create_dir_all(root.join("target")).unwrap();
        fs::write(root.join("target").join("debug.pdb"), "debug").unwrap();
        fs::write(root.join("module.pyc"), "compiled").unwrap();

        let mut items = Vec::new();
        let mut scanned = 0usize;
        walk_workspace(
            &root,
            &root,
            None,
            100,
            &mut items,
            &HashSet::new(),
            &mut scanned,
        );

        let labels = items
            .iter()
            .filter_map(|item| item.item.label.as_deref())
            .collect::<Vec<_>>();
        assert!(labels.contains(&"visible"));
        assert!(labels.contains(&"visible/README.md"));
        assert!(!labels.iter().any(|label| label.contains("__pycache__")));
        assert!(!labels.iter().any(|label| label.contains("%TEMP%")));
        assert!(!labels.iter().any(|label| label.contains("target")));
        assert!(!labels.iter().any(|label| label.ends_with(".pyc")));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn workspace_candidates_filter_by_query() {
        let root = temp_workspace_root("source_candidates_filter_by_query");
        fs::create_dir_all(root.join("visible")).unwrap();
        fs::write(root.join("visible").join("README.md"), "visible source").unwrap();
        fs::write(root.join("visible").join("notes.txt"), "notes").unwrap();

        let mut items = Vec::new();
        let mut scanned = 0usize;
        walk_workspace(
            &root,
            &root,
            Some("readme"),
            100,
            &mut items,
            &HashSet::new(),
            &mut scanned,
        );

        let labels = items
            .iter()
            .filter_map(|item| item.item.label.as_deref())
            .collect::<Vec<_>>();
        assert_eq!(labels, vec!["visible/README.md"]);

        fs::remove_dir_all(root).unwrap();
    }

    fn temp_workspace_root(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("supernova_{name}_{unique}"));
        fs::create_dir_all(&root).unwrap();
        root
    }
}
