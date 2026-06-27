use std::collections::BTreeSet;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;

use crate::agent_container::{
    AgentContainer, AgentContainerStatus, ContainerTimelineItem, ContainerTimelineItemKind,
    MemoryBinding,
};
use crate::context_pack::{
    ContextPack, ContextPackAutoPolicy, ContextPackIncludeMode, ContextPackItem,
    ContextPackItemKind,
};
use crate::context_window::ContextWindowControlConfig;
use crate::{json_err, now_ms, ModelInvocationConfig, WorkspaceGuard, RUNTIME_DIR_NAME};

static CONTAINER_ID_COUNTER: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug)]
pub struct ContainerStore {
    workspace_root: PathBuf,
    state_root: PathBuf,
    db_path: PathBuf,
}

impl ContainerStore {
    pub fn new(workspace_root: impl AsRef<Path>) -> io::Result<Self> {
        let guard = WorkspaceGuard::new(workspace_root)?;
        Self::new_with_state_root(guard.root(), guard.root().join(RUNTIME_DIR_NAME))
    }

    pub fn new_with_state_root(
        workspace_root: impl AsRef<Path>,
        state_root: impl AsRef<Path>,
    ) -> io::Result<Self> {
        let guard = WorkspaceGuard::new(workspace_root)?;
        std::fs::create_dir_all(state_root.as_ref())?;
        let state_root = state_root.as_ref().canonicalize()?;
        let state_dir = state_root.join("state");
        std::fs::create_dir_all(&state_dir)?;
        let store = Self {
            workspace_root: guard.root().to_path_buf(),
            state_root,
            db_path: state_dir.join("containers.sqlite3"),
        };
        store.init_schema()?;
        Ok(store)
    }

    pub fn path(&self) -> &Path {
        &self.db_path
    }

    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    pub fn state_root(&self) -> &Path {
        &self.state_root
    }

    pub fn write_container_blob(
        &self,
        container_id: &str,
        relative_path: &str,
        bytes: &[u8],
    ) -> io::Result<String> {
        if relative_path.contains("..")
            || relative_path.starts_with('/')
            || relative_path.starts_with('\\')
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "container blob relative_path must stay inside the container blob directory",
            ));
        }
        let blob_root = self
            .state_root
            .join("blobs")
            .join("container")
            .join(container_id);
        let path = blob_root.join(relative_path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, bytes)?;
        Ok(format!(
            "container_blob://{}/{}",
            container_id,
            relative_path.replace('\\', "/")
        ))
    }

    pub fn create_container(
        &self,
        title: Option<String>,
        default_model_config: Option<ModelInvocationConfig>,
        context_policy: Option<ContextWindowControlConfig>,
    ) -> io::Result<AgentContainer> {
        let now = now_ms_i64();
        let container = AgentContainer {
            container_id: next_id("container"),
            title,
            workspace_root: self.workspace_root.clone(),
            created_at_ms: now,
            updated_at_ms: now,
            status: AgentContainerStatus::Active,
            default_model_config: default_model_config
                .unwrap_or_else(ModelInvocationConfig::from_env),
            context_policy: context_policy.unwrap_or_default(),
        };
        let conn = self.connect()?;
        conn.execute(
            r#"
            INSERT INTO containers(
                container_id, title, workspace_root, status, default_model_config_json,
                context_policy_json, created_at_ms, updated_at_ms
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
            params![
                container.container_id,
                container.title,
                container.workspace_root.display().to_string(),
                container.status.as_str(),
                serde_json::to_string(&container.default_model_config).map_err(json_err)?,
                serde_json::to_string(&container.context_policy).map_err(json_err)?,
                container.created_at_ms,
                container.updated_at_ms,
            ],
        )
        .map_err(sql_err)?;
        Ok(container)
    }

    pub fn get_container(&self, container_id: &str) -> io::Result<AgentContainer> {
        let conn = self.connect()?;
        conn.query_row(
            r#"
            SELECT container_id, title, workspace_root, status, default_model_config_json,
                   context_policy_json, created_at_ms, updated_at_ms
            FROM containers
            WHERE container_id = ?1
            "#,
            params![container_id],
            row_to_container,
        )
        .map_err(sql_err)
    }

    pub fn list_containers(&self) -> io::Result<Vec<AgentContainer>> {
        let conn = self.connect()?;
        let mut stmt = conn
            .prepare(
                r#"
                SELECT container_id, title, workspace_root, status, default_model_config_json,
                       context_policy_json, created_at_ms, updated_at_ms
                FROM containers
                ORDER BY updated_at_ms DESC, created_at_ms DESC
                "#,
            )
            .map_err(sql_err)?;
        let rows = stmt.query_map([], row_to_container).map_err(sql_err)?;
        collect_rows(rows)
    }

    pub fn archive_container(&self, container_id: &str) -> io::Result<()> {
        let conn = self.connect()?;
        conn.execute(
            "UPDATE containers SET status = ?1, updated_at_ms = ?2 WHERE container_id = ?3",
            params![
                AgentContainerStatus::Archived.as_str(),
                now_ms_i64(),
                container_id
            ],
        )
        .map_err(sql_err)?;
        Ok(())
    }

    pub fn update_container(
        &self,
        container_id: &str,
        title: Option<String>,
        status: Option<AgentContainerStatus>,
        default_model_config: Option<ModelInvocationConfig>,
        context_policy: Option<ContextWindowControlConfig>,
    ) -> io::Result<AgentContainer> {
        let mut container = self.get_container(container_id)?;
        if title.is_some() {
            container.title = title;
        }
        if let Some(status) = status {
            container.status = status;
        }
        if let Some(config) = default_model_config {
            container.default_model_config = config;
        }
        if let Some(policy) = context_policy {
            container.context_policy = policy;
        }
        container.updated_at_ms = now_ms_i64();
        let conn = self.connect()?;
        conn.execute(
            r#"
            UPDATE containers
            SET title = ?1,
                status = ?2,
                default_model_config_json = ?3,
                context_policy_json = ?4,
                updated_at_ms = ?5
            WHERE container_id = ?6
            "#,
            params![
                container.title.clone(),
                container.status.as_str(),
                serde_json::to_string(&container.default_model_config).map_err(json_err)?,
                serde_json::to_string(&container.context_policy).map_err(json_err)?,
                container.updated_at_ms,
                container_id,
            ],
        )
        .map_err(sql_err)?;
        Ok(container)
    }

    pub fn append_timeline_item(
        &self,
        container_id: &str,
        item_kind: ContainerTimelineItemKind,
        ref_id: impl Into<String>,
        status: impl Into<String>,
        title: Option<String>,
        summary_ref: Option<String>,
    ) -> io::Result<ContainerTimelineItem> {
        let now = now_ms_i64();
        let item = ContainerTimelineItem {
            container_id: container_id.to_string(),
            item_id: next_id("timeline"),
            item_kind,
            title,
            status: status.into(),
            created_at_ms: now,
            updated_at_ms: now,
            ref_id: ref_id.into(),
            summary_ref,
        };
        let conn = self.connect()?;
        conn.execute(
            r#"
            INSERT INTO container_timeline(
                item_id, container_id, item_kind, ref_id, status, title,
                summary_ref, created_at_ms, updated_at_ms
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            "#,
            params![
                item.item_id,
                item.container_id,
                item.item_kind.as_str(),
                item.ref_id,
                item.status,
                item.title,
                item.summary_ref,
                item.created_at_ms,
                item.updated_at_ms,
            ],
        )
        .map_err(sql_err)?;
        self.touch_container(container_id)?;
        Ok(item)
    }

    pub fn upsert_timeline_item(
        &self,
        container_id: &str,
        item_kind: ContainerTimelineItemKind,
        ref_id: impl Into<String>,
        status: impl Into<String>,
        title: Option<String>,
        summary_ref: Option<String>,
    ) -> io::Result<ContainerTimelineItem> {
        let ref_id = ref_id.into();
        let status = status.into();
        let now = now_ms_i64();
        let conn = self.connect()?;
        let existing = conn
            .query_row(
                r#"
                SELECT item_id, container_id, item_kind, ref_id, status, title,
                       summary_ref, created_at_ms, updated_at_ms
                FROM container_timeline
                WHERE container_id = ?1 AND item_kind = ?2 AND ref_id = ?3
                ORDER BY created_at_ms ASC, item_id ASC
                LIMIT 1
                "#,
                params![container_id, item_kind.as_str(), ref_id],
                row_to_timeline_item,
            )
            .optional()
            .map_err(sql_err)?;
        if let Some(mut item) = existing {
            item.status = status;
            if title.is_some() {
                item.title = title;
            }
            if summary_ref.is_some() {
                item.summary_ref = summary_ref;
            }
            item.updated_at_ms = now;
            conn.execute(
                r#"
                UPDATE container_timeline
                SET status = ?1,
                    title = ?2,
                    summary_ref = ?3,
                    updated_at_ms = ?4
                WHERE item_id = ?5
                "#,
                params![
                    item.status,
                    item.title,
                    item.summary_ref,
                    item.updated_at_ms,
                    item.item_id,
                ],
            )
            .map_err(sql_err)?;
            self.touch_container(container_id)?;
            return Ok(item);
        }

        let item = ContainerTimelineItem {
            container_id: container_id.to_string(),
            item_id: next_id("timeline"),
            item_kind,
            title,
            status,
            created_at_ms: now,
            updated_at_ms: now,
            ref_id,
            summary_ref,
        };
        conn.execute(
            r#"
            INSERT INTO container_timeline(
                item_id, container_id, item_kind, ref_id, status, title,
                summary_ref, created_at_ms, updated_at_ms
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            "#,
            params![
                item.item_id,
                item.container_id,
                item.item_kind.as_str(),
                item.ref_id,
                item.status,
                item.title,
                item.summary_ref,
                item.created_at_ms,
                item.updated_at_ms,
            ],
        )
        .map_err(sql_err)?;
        self.touch_container(container_id)?;
        Ok(item)
    }

    pub fn list_timeline(
        &self,
        container_id: &str,
        limit: usize,
    ) -> io::Result<Vec<ContainerTimelineItem>> {
        let conn = self.connect()?;
        let mut stmt = conn
            .prepare(
                r#"
                SELECT item_id, container_id, item_kind, ref_id, status, title,
                       summary_ref, created_at_ms, updated_at_ms
                FROM container_timeline
                WHERE container_id = ?1
                ORDER BY created_at_ms ASC, item_id ASC
                LIMIT ?2
                "#,
            )
            .map_err(sql_err)?;
        let rows = stmt
            .query_map(params![container_id, limit as i64], row_to_timeline_item)
            .map_err(sql_err)?;
        collect_rows(rows)
    }

    pub fn upsert_context_pack(&self, mut pack: ContextPack) -> io::Result<ContextPack> {
        if pack.context_pack_id.trim().is_empty() {
            pack.context_pack_id = next_id("context_pack");
        }
        let now = now_ms_i64();
        let conn = self.connect()?;
        conn.execute(
            r#"
            INSERT INTO context_packs(
                context_pack_id, container_id, selected_items_json, excluded_items_json,
                auto_policy_json, summary_ref, estimated_tokens, created_at_ms, updated_at_ms
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            ON CONFLICT(context_pack_id) DO UPDATE SET
                selected_items_json = excluded.selected_items_json,
                excluded_items_json = excluded.excluded_items_json,
                auto_policy_json = excluded.auto_policy_json,
                summary_ref = excluded.summary_ref,
                estimated_tokens = excluded.estimated_tokens,
                updated_at_ms = excluded.updated_at_ms
            "#,
            params![
                pack.context_pack_id,
                pack.container_id,
                serde_json::to_string(&pack.selected_items).map_err(json_err)?,
                serde_json::to_string(&pack.excluded_items).map_err(json_err)?,
                serde_json::to_string(&pack.auto_policy).map_err(json_err)?,
                pack.summary_ref,
                pack.estimated_tokens.map(|value| value as i64),
                now,
                now,
            ],
        )
        .map_err(sql_err)?;
        self.touch_container(&pack.container_id)?;
        Ok(pack)
    }

    pub fn materialize_context_pack_auto_items(
        &self,
        mut pack: ContextPack,
    ) -> io::Result<ContextPack> {
        let excluded_refs = pack
            .excluded_items
            .iter()
            .map(|item| item.ref_id.clone())
            .collect::<BTreeSet<_>>();
        let mut selected_refs = pack
            .selected_items
            .iter()
            .map(|item| item.ref_id.clone())
            .collect::<BTreeSet<_>>();
        let timeline = self.list_timeline(&pack.container_id, 1_000)?;
        let mut recent_chat_turns = 0_usize;
        let mut recent_tasks = 0_usize;
        for item in timeline.iter().rev() {
            if excluded_refs.contains(&item.ref_id) || selected_refs.contains(&item.ref_id) {
                continue;
            }
            let item_kind = match item.item_kind {
                ContainerTimelineItemKind::ChatTurn | ContainerTimelineItemKind::ChatThread
                    if recent_chat_turns < pack.auto_policy.include_recent_chat_turns =>
                {
                    recent_chat_turns += 1;
                    Some(ContextPackItemKind::ChatTurn)
                }
                ContainerTimelineItemKind::TaskRun
                    if recent_tasks < pack.auto_policy.include_recent_tasks =>
                {
                    recent_tasks += 1;
                    Some(ContextPackItemKind::TaskRun)
                }
                ContainerTimelineItemKind::Artifact => Some(ContextPackItemKind::Artifact),
                ContainerTimelineItemKind::ContextCompaction => {
                    Some(ContextPackItemKind::ContainerSummary)
                }
                _ => None,
            };
            let Some(item_kind) = item_kind else {
                continue;
            };
            let include_mode = if pack.auto_policy.prefer_summaries && item.summary_ref.is_some() {
                ContextPackIncludeMode::Summary
            } else {
                ContextPackIncludeMode::RefOnly
            };
            selected_refs.insert(item.ref_id.clone());
            pack.selected_items.push(ContextPackItem {
                item_kind,
                ref_id: item.ref_id.clone(),
                label: item.title.clone(),
                include_mode,
                priority: 50,
            });
        }
        Ok(pack)
    }

    pub fn get_context_pack(&self, context_pack_id: &str) -> io::Result<ContextPack> {
        let conn = self.connect()?;
        conn.query_row(
            r#"
            SELECT context_pack_id, container_id, selected_items_json, excluded_items_json,
                   auto_policy_json, summary_ref, estimated_tokens
            FROM context_packs
            WHERE context_pack_id = ?1
            "#,
            params![context_pack_id],
            row_to_context_pack,
        )
        .map_err(sql_err)
    }

    pub fn latest_context_pack_for_container(
        &self,
        container_id: &str,
    ) -> io::Result<Option<ContextPack>> {
        let conn = self.connect()?;
        let mut stmt = conn
            .prepare(
                r#"
                SELECT context_pack_id, container_id, selected_items_json, excluded_items_json,
                       auto_policy_json, summary_ref, estimated_tokens
                FROM context_packs
                WHERE container_id = ?1
                ORDER BY updated_at_ms DESC, created_at_ms DESC
                LIMIT 1
                "#,
            )
            .map_err(sql_err)?;
        let mut rows = stmt.query(params![container_id]).map_err(sql_err)?;
        if let Some(row) = rows.next().map_err(sql_err)? {
            row_to_context_pack(row).map(Some).map_err(sql_err)
        } else {
            Ok(None)
        }
    }

    pub fn bind_memory(
        &self,
        container_id: &str,
        memory_ref: impl Into<String>,
        include_mode: impl Into<String>,
        priority: u8,
    ) -> io::Result<MemoryBinding> {
        let now = now_ms_i64();
        let binding = MemoryBinding {
            binding_id: next_id("memory_binding"),
            container_id: container_id.to_string(),
            memory_ref: memory_ref.into(),
            include_mode: include_mode.into(),
            priority,
            created_at_ms: now,
            updated_at_ms: now,
        };
        let conn = self.connect()?;
        conn.execute(
            r#"
            INSERT INTO memory_bindings(
                binding_id, container_id, memory_ref, include_mode, priority,
                created_at_ms, updated_at_ms
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            "#,
            params![
                binding.binding_id,
                binding.container_id,
                binding.memory_ref,
                binding.include_mode,
                binding.priority as i64,
                binding.created_at_ms,
                binding.updated_at_ms,
            ],
        )
        .map_err(sql_err)?;
        self.touch_container(container_id)?;
        Ok(binding)
    }

    pub fn list_memory_bindings(&self, container_id: &str) -> io::Result<Vec<MemoryBinding>> {
        let conn = self.connect()?;
        let mut stmt = conn
            .prepare(
                r#"
                SELECT binding_id, container_id, memory_ref, include_mode, priority,
                       created_at_ms, updated_at_ms
                FROM memory_bindings
                WHERE container_id = ?1
                ORDER BY priority DESC, created_at_ms ASC
                "#,
            )
            .map_err(sql_err)?;
        let rows = stmt
            .query_map(params![container_id], row_to_memory_binding)
            .map_err(sql_err)?;
        collect_rows(rows)
    }

    pub fn unbind_memory(&self, binding_id: &str) -> io::Result<()> {
        let conn = self.connect()?;
        conn.execute(
            "DELETE FROM memory_bindings WHERE binding_id = ?1",
            params![binding_id],
        )
        .map_err(sql_err)?;
        Ok(())
    }

    fn touch_container(&self, container_id: &str) -> io::Result<()> {
        let conn = self.connect()?;
        conn.execute(
            "UPDATE containers SET updated_at_ms = ?1 WHERE container_id = ?2",
            params![now_ms_i64(), container_id],
        )
        .map_err(sql_err)?;
        Ok(())
    }

    fn connect(&self) -> io::Result<Connection> {
        Connection::open(&self.db_path).map_err(sql_err)
    }

    fn init_schema(&self) -> io::Result<()> {
        let conn = self.connect()?;
        conn.execute_batch(
            r#"
            PRAGMA journal_mode = WAL;
            CREATE TABLE IF NOT EXISTS containers(
              container_id text primary key,
              title text,
              workspace_root text not null,
              status text not null,
              default_model_config_json text not null,
              context_policy_json text not null,
              created_at_ms integer not null,
              updated_at_ms integer not null
            );
            CREATE TABLE IF NOT EXISTS container_timeline(
              item_id text primary key,
              container_id text not null,
              item_kind text not null,
              ref_id text not null,
              status text not null,
              title text,
              summary_ref text,
              created_at_ms integer not null,
              updated_at_ms integer not null
            );
            CREATE TABLE IF NOT EXISTS context_packs(
              context_pack_id text primary key,
              container_id text not null,
              selected_items_json text not null,
              excluded_items_json text not null,
              auto_policy_json text not null,
              summary_ref text,
              estimated_tokens integer,
              created_at_ms integer not null,
              updated_at_ms integer not null
            );
            CREATE TABLE IF NOT EXISTS memory_bindings(
              binding_id text primary key,
              container_id text not null,
              memory_ref text not null,
              include_mode text not null,
              priority integer not null,
              created_at_ms integer not null,
              updated_at_ms integer not null
            );
            "#,
        )
        .map_err(sql_err)?;
        Ok(())
    }
}

fn row_to_container(row: &rusqlite::Row<'_>) -> rusqlite::Result<AgentContainer> {
    let model_config_raw: String = row.get(4)?;
    let context_policy_raw: String = row.get(5)?;
    Ok(AgentContainer {
        container_id: row.get(0)?,
        title: row.get(1)?,
        workspace_root: PathBuf::from(row.get::<_, String>(2)?),
        status: AgentContainerStatus::from_str(&row.get::<_, String>(3)?),
        default_model_config: serde_json::from_str(&model_config_raw).map_err(row_json_err)?,
        context_policy: serde_json::from_str(&context_policy_raw).map_err(row_json_err)?,
        created_at_ms: row.get(6)?,
        updated_at_ms: row.get(7)?,
    })
}

fn row_to_timeline_item(row: &rusqlite::Row<'_>) -> rusqlite::Result<ContainerTimelineItem> {
    Ok(ContainerTimelineItem {
        item_id: row.get(0)?,
        container_id: row.get(1)?,
        item_kind: ContainerTimelineItemKind::from_str(&row.get::<_, String>(2)?),
        ref_id: row.get(3)?,
        status: row.get(4)?,
        title: row.get(5)?,
        summary_ref: row.get(6)?,
        created_at_ms: row.get(7)?,
        updated_at_ms: row.get(8)?,
    })
}

fn row_to_context_pack(row: &rusqlite::Row<'_>) -> rusqlite::Result<ContextPack> {
    let selected_raw: String = row.get(2)?;
    let excluded_raw: String = row.get(3)?;
    let auto_policy_raw: String = row.get(4)?;
    let estimated: Option<i64> = row.get(6)?;
    Ok(ContextPack {
        context_pack_id: row.get(0)?,
        container_id: row.get(1)?,
        selected_items: serde_json::from_str::<Vec<ContextPackItem>>(&selected_raw)
            .map_err(row_json_err)?,
        excluded_items: serde_json::from_str::<Vec<ContextPackItem>>(&excluded_raw)
            .map_err(row_json_err)?,
        auto_policy: serde_json::from_str::<ContextPackAutoPolicy>(&auto_policy_raw)
            .map_err(row_json_err)?,
        summary_ref: row.get(5)?,
        estimated_tokens: estimated.map(|value| value as u64),
    })
}

fn row_to_memory_binding(row: &rusqlite::Row<'_>) -> rusqlite::Result<MemoryBinding> {
    Ok(MemoryBinding {
        binding_id: row.get(0)?,
        container_id: row.get(1)?,
        memory_ref: row.get(2)?,
        include_mode: row.get(3)?,
        priority: row.get::<_, i64>(4)? as u8,
        created_at_ms: row.get(5)?,
        updated_at_ms: row.get(6)?,
    })
}

fn collect_rows<T, I>(rows: I) -> io::Result<Vec<T>>
where
    I: IntoIterator<Item = rusqlite::Result<T>>,
{
    let mut values = Vec::new();
    for row in rows {
        values.push(row.map_err(sql_err)?);
    }
    Ok(values)
}

fn next_id(prefix: &str) -> String {
    format!(
        "{}_{}_{}_{}",
        prefix,
        now_ms(),
        std::process::id(),
        CONTAINER_ID_COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

fn now_ms_i64() -> i64 {
    now_ms().min(i64::MAX as u128) as i64
}

fn row_json_err(err: serde_json::Error) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(err))
}

fn sql_err(err: rusqlite::Error) -> io::Error {
    io::Error::new(io::ErrorKind::Other, err)
}

#[allow(dead_code)]
fn _value_to_string(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context_pack::{ContextPackIncludeMode, ContextPackItem, ContextPackItemKind};

    fn temp_workspace(name: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("supernova_container_store_{}_{}", name, now_ms()));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn container_store_uses_external_state_root_without_workspace_state() {
        let workspace = temp_workspace("external_state_workspace");
        let state_root = std::env::temp_dir().join(format!(
            "supernova_container_store_external_state_{}",
            now_ms()
        ));
        let store = ContainerStore::new_with_state_root(&workspace, &state_root).unwrap();
        let state_root = state_root.canonicalize().unwrap();
        let container = store
            .create_container(Some("External".to_string()), None, None)
            .unwrap();
        let blob_ref = store
            .write_container_blob(&container.container_id, "notes/context.json", b"{}")
            .unwrap();

        assert!(store.path().starts_with(state_root.join("state")));
        assert!(state_root.join("state").join("containers.sqlite3").exists());
        assert!(state_root
            .join("blobs")
            .join("container")
            .join(&container.container_id)
            .join("notes")
            .join("context.json")
            .exists());
        assert!(blob_ref.starts_with("container_blob://"));
        assert!(!workspace.join(crate::RUNTIME_DIR_NAME).exists());
    }

    #[test]
    fn container_store_persists_timeline_context_pack_and_memory() {
        let workspace = temp_workspace("basic");
        let store = ContainerStore::new(&workspace).unwrap();
        let container = store
            .create_container(Some("Research".to_string()), None, None)
            .unwrap();
        assert!(container.container_id.starts_with("container_"));

        store
            .append_timeline_item(
                &container.container_id,
                ContainerTimelineItemKind::ChatThread,
                "chat://thread1",
                "active",
                Some("Chat".to_string()),
                None,
            )
            .unwrap();
        store
            .append_timeline_item(
                &container.container_id,
                ContainerTimelineItemKind::TaskRun,
                "job_1",
                "completed",
                Some("Task".to_string()),
                Some("blob://summary".to_string()),
            )
            .unwrap();
        let timeline = store.list_timeline(&container.container_id, 10).unwrap();
        assert_eq!(timeline.len(), 2);
        assert_eq!(timeline[0].item_kind, ContainerTimelineItemKind::ChatThread);
        assert_eq!(timeline[1].item_kind, ContainerTimelineItemKind::TaskRun);
        let task_timeline_id = timeline[1].item_id.clone();
        let task_created_at_ms = timeline[1].created_at_ms;
        let updated = store
            .upsert_timeline_item(
                &container.container_id,
                ContainerTimelineItemKind::TaskRun,
                "job_1",
                "running",
                Some("Task updated".to_string()),
                None,
            )
            .unwrap();
        assert_eq!(updated.item_id, task_timeline_id);
        assert_eq!(updated.created_at_ms, task_created_at_ms);
        assert_eq!(updated.status, "running");
        assert_eq!(
            store
                .list_timeline(&container.container_id, 10)
                .unwrap()
                .len(),
            2
        );

        let mut pack = ContextPack::empty("", &container.container_id);
        pack.selected_items.push(ContextPackItem {
            item_kind: ContextPackItemKind::TaskArtifact,
            ref_id: "artifact://report".to_string(),
            label: Some("Report".to_string()),
            include_mode: ContextPackIncludeMode::Summary,
            priority: 80,
        });
        let pack = store.upsert_context_pack(pack).unwrap();
        let loaded = store.get_context_pack(&pack.context_pack_id).unwrap();
        assert!(loaded
            .selected_items
            .iter()
            .any(|item| item.ref_id == "artifact://report"
                && item.include_mode == ContextPackIncludeMode::Summary));
        assert!(!loaded
            .selected_items
            .iter()
            .any(|item| item.ref_id == "chat://thread1"));
        assert!(!loaded
            .selected_items
            .iter()
            .any(|item| item.ref_id == "job_1"));
        let materialized = store
            .materialize_context_pack_auto_items(loaded.clone())
            .unwrap();
        assert!(materialized
            .selected_items
            .iter()
            .any(|item| item.ref_id == "chat://thread1"
                && item.item_kind == ContextPackItemKind::ChatTurn));
        assert!(materialized
            .selected_items
            .iter()
            .any(|item| item.ref_id == "job_1"
                && item.item_kind == ContextPackItemKind::TaskRun
                && item.include_mode == ContextPackIncludeMode::Summary));

        let binding = store
            .bind_memory(&container.container_id, "memory://note", "summary", 42)
            .unwrap();
        assert_eq!(
            store.list_memory_bindings(&container.container_id).unwrap()[0].binding_id,
            binding.binding_id
        );
    }
}
