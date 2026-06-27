use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use local_runtime_protocol::{
    ChatThreadRecord, ContainerBadges, ContainerRecord, ContainerStatus, ContextPack,
    ContextPackAutoPolicy, ModelConfig, TaskDraftArtifactRecord, TaskRecord,
};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;

use crate::state::migrations::init_product_db;
use crate::state::workspace_registry::now_ms;

static ID_COUNTER: AtomicU64 = AtomicU64::new(1);
const PRODUCT_DB_BUSY_TIMEOUT_MS: u64 = 5_000;

#[derive(Clone, Debug)]
pub struct ProductDb {
    pub db_path: PathBuf,
    pub workspace_uid: String,
}

fn open_product_db_connection(path: &Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open(path)?;
    conn.busy_timeout(Duration::from_millis(PRODUCT_DB_BUSY_TIMEOUT_MS))?;
    conn.execute_batch(
        r#"
        PRAGMA journal_mode = WAL;
        PRAGMA synchronous = NORMAL;
        "#,
    )?;
    Ok(conn)
}

impl ProductDb {
    pub fn open(workspace_state_root: &Path, workspace_uid: String) -> rusqlite::Result<Self> {
        std::fs::create_dir_all(workspace_state_root)
            .map_err(|err| rusqlite::Error::ToSqlConversionFailure(Box::new(err)))?;
        let db_path = workspace_state_root.join("product.sqlite3");
        let conn = open_product_db_connection(&db_path)?;
        init_product_db(&conn)?;
        Ok(Self {
            db_path,
            workspace_uid,
        })
    }

    pub fn open_existing(
        workspace_state_root: &Path,
        workspace_uid: String,
    ) -> rusqlite::Result<Option<Self>> {
        let db_path = workspace_state_root.join("product.sqlite3");
        if !db_path.exists() {
            return Ok(None);
        }
        let conn = open_product_db_connection(&db_path)?;
        init_product_db(&conn)?;
        Ok(Some(Self {
            db_path,
            workspace_uid,
        }))
    }

    pub(crate) fn connect(&self) -> rusqlite::Result<Connection> {
        open_product_db_connection(&self.db_path)
    }

    pub fn list_container_projections(
        &self,
        include_archived: bool,
    ) -> rusqlite::Result<Vec<ContainerRecord>> {
        let conn = self.connect()?;
        let sql = if include_archived {
            "SELECT container_id, workspace_uid, title, status, created_at_ms, updated_at_ms, last_active_at_ms, default_model_config_json, context_policy_json FROM container_projection ORDER BY created_at_ms ASC, container_id ASC"
        } else {
            "SELECT container_id, workspace_uid, title, status, created_at_ms, updated_at_ms, last_active_at_ms, default_model_config_json, context_policy_json FROM container_projection WHERE status != 'archived' AND status != 'deleted' ORDER BY created_at_ms ASC, container_id ASC"
        };
        let mut stmt = conn.prepare(sql)?;
        let rows = stmt.query_map([], row_to_container)?;
        rows.collect()
    }

    pub fn list_archived_container_projections(&self) -> rusqlite::Result<Vec<ContainerRecord>> {
        let conn = self.connect()?;
        let mut stmt = conn.prepare(
            "SELECT container_id, workspace_uid, title, status, created_at_ms, updated_at_ms, last_active_at_ms, default_model_config_json, context_policy_json FROM container_projection WHERE status='archived' ORDER BY updated_at_ms DESC",
        )?;
        let rows = stmt.query_map([], row_to_container)?;
        rows.collect()
    }

    pub fn get_container_projection(
        &self,
        container_id: &str,
    ) -> rusqlite::Result<ContainerRecord> {
        let conn = self.connect()?;
        conn.query_row(
            "SELECT container_id, workspace_uid, title, status, created_at_ms, updated_at_ms, last_active_at_ms, default_model_config_json, context_policy_json FROM container_projection WHERE container_id=?1",
            params![container_id],
            row_to_container,
        )
    }

    pub fn upsert_container_projection(
        &self,
        container: &ContainerRecord,
    ) -> rusqlite::Result<ContainerRecord> {
        let conn = self.connect()?;
        conn.execute(
            "INSERT INTO container_projection(container_id, workspace_uid, title, status, created_at_ms, updated_at_ms, last_active_at_ms, default_model_config_json, context_policy_json)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(container_id) DO UPDATE SET
               workspace_uid=excluded.workspace_uid,
               title=excluded.title,
               status=excluded.status,
               created_at_ms=excluded.created_at_ms,
               updated_at_ms=excluded.updated_at_ms,
               default_model_config_json=excluded.default_model_config_json,
               context_policy_json=excluded.context_policy_json",
            params![
                &container.container_id,
                &container.workspace_uid,
                &container.title,
                status_to_str(&container.status),
                container.created_at_ms as i64,
                container.updated_at_ms as i64,
                container.last_active_at_ms as i64,
                container
                    .default_model_config
                    .as_ref()
                    .map(|value| serde_json::to_string(value).unwrap_or_else(|_| "{}".into())),
                container
                    .context_policy
                    .as_ref()
                    .map(|value| serde_json::to_string(value).unwrap_or_else(|_| "{}".into())),
            ],
        )?;
        if matches!(
            container.status,
            ContainerStatus::Archived | ContainerStatus::Deleted
        ) {
            self.reconcile_active_after_hidden(&conn, &container.container_id)?;
        }
        self.get_container_projection(&container.container_id)
    }

    pub fn set_active_container_projection(
        &self,
        container_id: &str,
    ) -> rusqlite::Result<ContainerRecord> {
        let now = now_ms();
        let conn = self.connect()?;
        let changed = conn.execute(
            "UPDATE container_projection SET last_active_at_ms=?2, updated_at_ms=?2 WHERE container_id=?1 AND status != 'archived' AND status != 'deleted'",
            params![container_id, now],
        )?;
        if changed == 0 {
            return self.get_container_projection(container_id);
        }
        conn.execute(
            "INSERT INTO active_container(workspace_uid, container_id, updated_at_ms) VALUES(?1, ?2, ?3) ON CONFLICT(workspace_uid) DO UPDATE SET container_id=excluded.container_id, updated_at_ms=excluded.updated_at_ms",
            params![self.workspace_uid, container_id, now],
        )?;
        self.get_container_projection(container_id)
    }

    fn reconcile_active_after_hidden(
        &self,
        conn: &Connection,
        container_id: &str,
    ) -> rusqlite::Result<()> {
        let active: Option<String> = conn
            .query_row(
                "SELECT container_id FROM active_container WHERE workspace_uid=?1",
                params![self.workspace_uid],
                |row| row.get(0),
            )
            .optional()?;
        if active.as_deref() != Some(container_id) {
            return Ok(());
        }
        let next: Option<String> = conn
            .query_row(
                "SELECT container_id FROM container_projection WHERE status != 'archived' AND status != 'deleted' ORDER BY last_active_at_ms DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        let now = now_ms();
        if let Some(next_container_id) = next {
            conn.execute(
                "INSERT INTO active_container(workspace_uid, container_id, updated_at_ms) VALUES(?1, ?2, ?3) ON CONFLICT(workspace_uid) DO UPDATE SET container_id=excluded.container_id, updated_at_ms=excluded.updated_at_ms",
                params![self.workspace_uid, next_container_id, now],
            )?;
        } else {
            conn.execute(
                "DELETE FROM active_container WHERE workspace_uid=?1",
                params![self.workspace_uid],
            )?;
        }
        Ok(())
    }

    pub fn active_container_projection_id(&self) -> rusqlite::Result<Option<String>> {
        let conn = self.connect()?;
        conn.query_row(
            "SELECT container_id FROM active_container WHERE workspace_uid=?1",
            params![self.workspace_uid],
            |row| row.get(0),
        )
        .optional()
    }

    pub fn create_chat_thread(
        &self,
        container_id: &str,
        title: Option<String>,
    ) -> rusqlite::Result<ChatThreadRecord> {
        let id = next_id("chat");
        let now = now_ms();
        let title = title.unwrap_or_else(|| "Workbench thread".to_string());
        let conn = self.connect()?;
        conn.execute(
            "INSERT INTO chat_threads(chat_thread_id, container_id, title, created_at_ms, updated_at_ms) VALUES(?1, ?2, ?3, ?4, ?4)",
            params![id, container_id, title, now],
        )?;
        self.get_chat_thread(&id)
    }

    pub fn list_chat_threads(&self, container_id: &str) -> rusqlite::Result<Vec<ChatThreadRecord>> {
        let conn = self.connect()?;
        let mut stmt = conn.prepare(
            "SELECT chat_thread_id, container_id, title, created_at_ms, updated_at_ms FROM chat_threads WHERE container_id=?1 ORDER BY updated_at_ms DESC",
        )?;
        let rows = stmt.query_map(params![container_id], row_to_chat_thread)?;
        rows.collect()
    }

    pub fn list_all_chat_threads(&self) -> rusqlite::Result<Vec<ChatThreadRecord>> {
        let conn = self.connect()?;
        let mut stmt = conn.prepare(
            "SELECT chat_thread_id, container_id, title, created_at_ms, updated_at_ms FROM chat_threads ORDER BY updated_at_ms DESC, chat_thread_id DESC",
        )?;
        let rows = stmt.query_map([], row_to_chat_thread)?;
        rows.collect()
    }

    pub fn upsert_chat_thread(
        &self,
        thread: &ChatThreadRecord,
    ) -> rusqlite::Result<ChatThreadRecord> {
        let conn = self.connect()?;
        conn.execute(
            "INSERT OR REPLACE INTO chat_threads(chat_thread_id, container_id, title, created_at_ms, updated_at_ms) VALUES(?1, ?2, ?3, ?4, ?5)",
            params![
                &thread.chat_thread_id,
                &thread.container_id,
                &thread.title,
                thread.created_at_ms as i64,
                thread.updated_at_ms as i64,
            ],
        )?;
        self.get_chat_thread(&thread.chat_thread_id)
    }

    pub fn get_chat_thread(&self, chat_thread_id: &str) -> rusqlite::Result<ChatThreadRecord> {
        let conn = self.connect()?;
        conn.query_row(
            "SELECT chat_thread_id, container_id, title, created_at_ms, updated_at_ms FROM chat_threads WHERE chat_thread_id=?1",
            params![chat_thread_id],
            row_to_chat_thread,
        )
    }

    pub fn list_tasks(&self, container_id: &str) -> rusqlite::Result<Vec<TaskRecord>> {
        let conn = self.connect()?;
        let mut stmt = conn.prepare(
            "SELECT task_id, container_id, job_id, title, goal, status, created_at_ms, updated_at_ms FROM tasks WHERE container_id=?1 ORDER BY updated_at_ms DESC",
        )?;
        let rows = stmt.query_map(params![container_id], row_to_task)?;
        rows.collect()
    }

    pub fn list_all_tasks(&self) -> rusqlite::Result<Vec<TaskRecord>> {
        let conn = self.connect()?;
        let mut stmt = conn.prepare(
            "SELECT task_id, container_id, job_id, title, goal, status, created_at_ms, updated_at_ms FROM tasks ORDER BY updated_at_ms DESC, task_id DESC",
        )?;
        let rows = stmt.query_map([], row_to_task)?;
        rows.collect()
    }

    pub fn upsert_task(&self, task: &TaskRecord) -> rusqlite::Result<TaskRecord> {
        let conn = self.connect()?;
        conn.execute(
            "INSERT OR REPLACE INTO tasks(task_id, container_id, job_id, title, goal, status, created_at_ms, updated_at_ms) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                &task.task_id,
                &task.container_id,
                &task.job_id,
                &task.title,
                &task.goal,
                &task.status,
                task.created_at_ms as i64,
                task.updated_at_ms as i64,
            ],
        )?;
        self.get_task(&task.task_id)
    }

    pub fn bind_messages_to_task(
        &self,
        message_ids: &[String],
        task_id: &str,
        job_id: &str,
    ) -> rusqlite::Result<()> {
        if message_ids.is_empty() {
            return Ok(());
        }
        let mut conn = self.connect()?;
        let tx = conn.transaction()?;
        for message_id in message_ids {
            tx.execute(
                "UPDATE messages SET task_id=?2, job_id=?3 WHERE message_id=?1",
                params![message_id, task_id, job_id],
            )?;
        }
        tx.commit()
    }

    pub fn get_task(&self, task_id: &str) -> rusqlite::Result<TaskRecord> {
        let conn = self.connect()?;
        conn.query_row(
            "SELECT task_id, container_id, job_id, title, goal, status, created_at_ms, updated_at_ms FROM tasks WHERE task_id=?1",
            params![task_id],
            row_to_task,
        )
    }

    pub fn list_task_messages(
        &self,
        task_id: &str,
    ) -> rusqlite::Result<Vec<local_runtime_protocol::ContainerMessage>> {
        let conn = self.connect()?;
        let mut stmt = conn.prepare(
            "SELECT message_id, workspace_uid, container_id, lane, role, message_type, status, title, body_text, body_json, card_json, chat_thread_id, task_id, job_id, source_kind, source_ref, source_seq, created_at_ms, updated_at_ms, sort_key FROM messages WHERE task_id=?1 ORDER BY sort_key ASC",
        )?;
        let rows = stmt.query_map(
            params![task_id],
            crate::state::message_feed::row_to_message_public,
        )?;
        rows.collect()
    }

    pub fn upsert_task_draft_artifact(
        &self,
        draft: &TaskDraftArtifactRecord,
    ) -> rusqlite::Result<TaskDraftArtifactRecord> {
        let conn = self.connect()?;
        conn.execute(
            "INSERT INTO task_draft_artifacts(
              draft_id, workspace_uid, container_id, task_id, approval_id, preview_ref,
              operation, status, content_format, content_text, created_at_ms, updated_at_ms
            )
            VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
            ON CONFLICT(task_id, approval_id) DO UPDATE SET
              draft_id=excluded.draft_id,
              workspace_uid=excluded.workspace_uid,
              container_id=excluded.container_id,
              preview_ref=excluded.preview_ref,
              operation=excluded.operation,
              status=excluded.status,
              content_format=excluded.content_format,
              content_text=excluded.content_text,
              updated_at_ms=excluded.updated_at_ms",
            params![
                &draft.draft_id,
                &draft.workspace_uid,
                &draft.container_id,
                &draft.task_id,
                &draft.approval_id,
                &draft.preview_ref,
                &draft.operation,
                &draft.status,
                &draft.content_format,
                &draft.content_text,
                draft.created_at_ms as i64,
                draft.updated_at_ms as i64,
            ],
        )?;
        self.get_task_draft_artifact(&draft.task_id, &draft.approval_id)?
            .ok_or_else(|| rusqlite::Error::QueryReturnedNoRows)
    }

    pub fn list_task_draft_artifacts(
        &self,
        task_id: &str,
    ) -> rusqlite::Result<Vec<TaskDraftArtifactRecord>> {
        let conn = self.connect()?;
        let mut stmt = conn.prepare(
            "SELECT draft_id, workspace_uid, container_id, task_id, approval_id, preview_ref, operation, status, content_format, content_text, created_at_ms, updated_at_ms
             FROM task_draft_artifacts WHERE task_id=?1 ORDER BY updated_at_ms DESC",
        )?;
        let rows = stmt.query_map(params![task_id], row_to_task_draft_artifact)?;
        rows.collect()
    }

    pub fn get_task_draft_artifact(
        &self,
        task_id: &str,
        approval_id: &str,
    ) -> rusqlite::Result<Option<TaskDraftArtifactRecord>> {
        let conn = self.connect()?;
        conn.query_row(
            "SELECT draft_id, workspace_uid, container_id, task_id, approval_id, preview_ref, operation, status, content_format, content_text, created_at_ms, updated_at_ms
             FROM task_draft_artifacts WHERE task_id=?1 AND approval_id=?2",
            params![task_id, approval_id],
            row_to_task_draft_artifact,
        )
        .optional()
    }

    pub fn update_task_draft_artifact_content(
        &self,
        task_id: &str,
        approval_id: &str,
        content_text: &str,
    ) -> rusqlite::Result<Option<TaskDraftArtifactRecord>> {
        let conn = self.connect()?;
        let changed = conn.execute(
            "UPDATE task_draft_artifacts SET content_text=?3, status='edited', updated_at_ms=?4 WHERE task_id=?1 AND approval_id=?2",
            params![task_id, approval_id, content_text, now_ms() as i64],
        )?;
        if changed == 0 {
            return Ok(None);
        }
        self.get_task_draft_artifact(task_id, approval_id)
    }

    pub fn delete_task_draft_artifact(
        &self,
        task_id: &str,
        approval_id: &str,
    ) -> rusqlite::Result<()> {
        let conn = self.connect()?;
        conn.execute(
            "DELETE FROM task_draft_artifacts WHERE task_id=?1 AND approval_id=?2",
            params![task_id, approval_id],
        )?;
        Ok(())
    }

    pub fn delete_task_draft_artifacts_for_task(&self, task_id: &str) -> rusqlite::Result<()> {
        let conn = self.connect()?;
        conn.execute(
            "DELETE FROM task_draft_artifacts WHERE task_id=?1",
            params![task_id],
        )?;
        Ok(())
    }

    pub fn get_context_pack(&self, container_id: &str) -> rusqlite::Result<Option<ContextPack>> {
        let conn = self.connect()?;
        conn.query_row(
            "SELECT context_pack_id, container_id, selected_items_json, excluded_items_json, auto_policy_json, summary_ref, estimated_tokens FROM context_packs WHERE container_id=?1 ORDER BY rowid DESC LIMIT 1",
            params![container_id],
            row_to_context_pack,
        )
        .optional()
    }

    pub fn save_context_pack(&self, pack: &ContextPack) -> rusqlite::Result<ContextPack> {
        let context_pack_id = if pack.context_pack_id.is_empty() {
            next_id("context_pack")
        } else {
            pack.context_pack_id.clone()
        };
        let conn = self.connect()?;
        conn.execute(
            "INSERT OR REPLACE INTO context_packs(context_pack_id, container_id, selected_items_json, excluded_items_json, auto_policy_json, summary_ref, estimated_tokens) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                &context_pack_id,
                &pack.container_id,
                serde_json::to_string(&pack.selected_items).unwrap_or_else(|_| "[]".into()),
                serde_json::to_string(&pack.excluded_items).unwrap_or_else(|_| "[]".into()),
                serde_json::to_string(&pack.auto_policy).unwrap_or_else(|_| "{}".into()),
                &pack.summary_ref,
                pack.estimated_tokens.map(|value| value as i64),
            ],
        )?;
        Ok(ContextPack {
            context_pack_id,
            container_id: pack.container_id.clone(),
            selected_items: pack.selected_items.clone(),
            excluded_items: pack.excluded_items.clone(),
            auto_policy: pack.auto_policy.clone(),
            summary_ref: pack.summary_ref.clone(),
            estimated_tokens: pack.estimated_tokens,
        })
    }

    pub fn get_model_config(&self) -> rusqlite::Result<Option<ModelConfig>> {
        let conn = self.connect()?;
        conn.query_row(
            "SELECT config_json FROM model_config_profiles WHERE profile_id='active'",
            [],
            |row| {
                let raw: String = row.get(0)?;
                serde_json::from_str(&raw).map_err(|err| {
                    rusqlite::Error::FromSqlConversionFailure(
                        0,
                        rusqlite::types::Type::Text,
                        Box::new(err),
                    )
                })
            },
        )
        .optional()
    }

    pub fn save_model_config(&self, config: &ModelConfig) -> rusqlite::Result<ModelConfig> {
        let conn = self.connect()?;
        conn.execute(
            "INSERT INTO model_config_profiles(profile_id, config_json, updated_at_ms) VALUES('active', ?1, ?2) ON CONFLICT(profile_id) DO UPDATE SET config_json=excluded.config_json, updated_at_ms=excluded.updated_at_ms",
            params![
                serde_json::to_string(config).unwrap_or_else(|_| "{}".into()),
                now_ms(),
            ],
        )?;
        Ok(config.clone())
    }
}

fn row_to_container(row: &rusqlite::Row<'_>) -> rusqlite::Result<ContainerRecord> {
    Ok(ContainerRecord {
        container_id: row.get(0)?,
        workspace_uid: row.get(1)?,
        title: row.get(2)?,
        status: match row.get::<_, String>(3)?.as_str() {
            "running" => ContainerStatus::Running,
            "approval" => ContainerStatus::Approval,
            "blocked" => ContainerStatus::Blocked,
            "archived" => ContainerStatus::Archived,
            "deleted" => ContainerStatus::Deleted,
            _ => ContainerStatus::Active,
        },
        badges: ContainerBadges::default(),
        created_at_ms: row.get::<_, i64>(4)? as u128,
        updated_at_ms: row.get::<_, i64>(5)? as u128,
        last_active_at_ms: row.get::<_, i64>(6)? as u128,
        default_model_config: row_json(row.get::<_, Option<String>>(7)?),
        context_policy: row_json(row.get::<_, Option<String>>(8)?),
    })
}

fn status_to_str(status: &ContainerStatus) -> &'static str {
    match status {
        ContainerStatus::Active => "active",
        ContainerStatus::Running => "running",
        ContainerStatus::Approval => "approval",
        ContainerStatus::Blocked => "blocked",
        ContainerStatus::Archived => "archived",
        ContainerStatus::Deleted => "deleted",
    }
}

fn row_to_chat_thread(row: &rusqlite::Row<'_>) -> rusqlite::Result<ChatThreadRecord> {
    Ok(ChatThreadRecord {
        chat_thread_id: row.get(0)?,
        container_id: row.get(1)?,
        title: row.get(2)?,
        created_at_ms: row.get::<_, i64>(3)? as u128,
        updated_at_ms: row.get::<_, i64>(4)? as u128,
    })
}

fn row_to_task(row: &rusqlite::Row<'_>) -> rusqlite::Result<TaskRecord> {
    Ok(TaskRecord {
        task_id: row.get(0)?,
        container_id: row.get(1)?,
        job_id: row.get(2)?,
        title: row.get(3)?,
        goal: row.get(4)?,
        status: row.get(5)?,
        badges: ContainerBadges::default(),
        created_at_ms: row.get::<_, i64>(6)? as u128,
        updated_at_ms: row.get::<_, i64>(7)? as u128,
    })
}

fn row_to_task_draft_artifact(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<TaskDraftArtifactRecord> {
    Ok(TaskDraftArtifactRecord {
        draft_id: row.get(0)?,
        workspace_uid: row.get(1)?,
        container_id: row.get(2)?,
        task_id: row.get(3)?,
        approval_id: row.get(4)?,
        preview_ref: row.get(5)?,
        operation: row.get(6)?,
        status: row.get(7)?,
        content_format: row.get(8)?,
        content_text: row.get(9)?,
        created_at_ms: row.get::<_, i64>(10)? as u128,
        updated_at_ms: row.get::<_, i64>(11)? as u128,
    })
}

fn row_to_context_pack(row: &rusqlite::Row<'_>) -> rusqlite::Result<ContextPack> {
    Ok(ContextPack {
        context_pack_id: row.get(0)?,
        container_id: row.get(1)?,
        selected_items: serde_json::from_str(&row.get::<_, String>(2)?).unwrap_or_default(),
        excluded_items: serde_json::from_str(&row.get::<_, String>(3)?).unwrap_or_default(),
        auto_policy: serde_json::from_str(&row.get::<_, String>(4)?).unwrap_or(
            ContextPackAutoPolicy {
                include_recent_chat_turns: 6,
                include_recent_tasks: 3,
                prefer_summaries: true,
            },
        ),
        summary_ref: row.get(5)?,
        estimated_tokens: row.get::<_, Option<i64>>(6)?.map(|value| value as u64),
    })
}

fn row_json(raw: Option<String>) -> Option<Value> {
    raw.and_then(|value| serde_json::from_str(&value).ok())
}

pub fn next_id(prefix: &str) -> String {
    let seq = ID_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{}_{}_{}", prefix, now_ms(), seq)
}

#[cfg(test)]
mod tests {
    use super::*;

    use rusqlite::OptionalExtension;

    use crate::state::message_feed::new_message;
    use crate::state::migrations::init_product_db;
    use local_runtime_protocol::{MessageLane, MessageRole, MessageType};

    fn temp_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("supernova_product_db_{name}_{}", now_ms()));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn table_exists(conn: &Connection, name: &str) -> bool {
        conn.query_row(
            "SELECT name FROM sqlite_master WHERE type='table' AND name=?1",
            [name],
            |_| Ok(()),
        )
        .optional()
        .unwrap()
        .is_some()
    }

    #[test]
    fn new_product_db_uses_container_projection_table() {
        let root = temp_root("projection_shape");
        let db = ProductDb::open(&root, "workspace_projection".into()).unwrap();
        let conn = Connection::open(db.db_path).unwrap();

        assert!(table_exists(&conn, "container_projection"));
        assert!(!table_exists(&conn, "containers"));
    }

    #[test]
    fn product_db_connections_use_wal_and_busy_timeout() {
        let root = temp_root("connection_pragmas");
        let db = ProductDb::open(&root, "workspace_pragmas".into()).unwrap();
        let conn = db.connect().unwrap();

        let journal_mode: String = conn
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .unwrap();
        let busy_timeout: i64 = conn
            .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
            .unwrap();

        assert_eq!(journal_mode.to_ascii_lowercase(), "wal");
        assert_eq!(busy_timeout, PRODUCT_DB_BUSY_TIMEOUT_MS as i64);
    }

    #[test]
    fn legacy_containers_table_migrates_to_projection_table() {
        let root = temp_root("legacy_container_migration");
        let db_path = root.join("product.sqlite3");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE containers(
              container_id TEXT PRIMARY KEY,
              workspace_uid TEXT NOT NULL,
              title TEXT NOT NULL,
              status TEXT NOT NULL,
              created_at_ms INTEGER NOT NULL,
              updated_at_ms INTEGER NOT NULL,
              last_active_at_ms INTEGER NOT NULL,
              default_model_config_json TEXT,
              context_policy_json TEXT
            );
            INSERT INTO containers(
              container_id,
              workspace_uid,
              title,
              status,
              created_at_ms,
              updated_at_ms,
              last_active_at_ms,
              default_model_config_json,
              context_policy_json
            ) VALUES(
              'container_legacy',
              'workspace_legacy',
              'Legacy Container',
              'active',
              1,
              2,
              3,
              NULL,
              NULL
            );
            "#,
        )
        .unwrap();

        init_product_db(&conn).unwrap();

        assert!(table_exists(&conn, "container_projection"));
        assert!(!table_exists(&conn, "containers"));
        assert!(table_exists(&conn, "containers_legacy_pre_projection_v1"));

        let title: String = conn
            .query_row(
                "SELECT title FROM container_projection WHERE container_id='container_legacy'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(title, "Legacy Container");
    }

    #[test]
    fn bind_messages_to_task_makes_pre_job_messages_replayable_by_task() {
        let root = temp_root("bind_task_messages");
        let db = ProductDb::open(&root, "workspace_messages".into()).unwrap();
        let message = db
            .append_message(new_message(
                "workspace_messages",
                "container_1",
                MessageLane::Task,
                MessageRole::User,
                MessageType::Text,
                Some("start long task".into()),
                None,
            ))
            .unwrap();

        db.bind_messages_to_task(&[message.message_id], "job_1", "job_1")
            .unwrap();

        let task_messages = db.list_task_messages("job_1").unwrap();
        assert_eq!(task_messages.len(), 1);
        assert_eq!(task_messages[0].task_id.as_deref(), Some("job_1"));
        assert_eq!(task_messages[0].job_id.as_deref(), Some("job_1"));
        assert_eq!(
            task_messages[0].body_text.as_deref(),
            Some("start long task")
        );
    }

    #[test]
    fn task_draft_artifact_crud_round_trips_preview_content() {
        let root = temp_root("task_draft_artifacts");
        let db = ProductDb::open(&root, "workspace_drafts".into()).unwrap();
        let draft = TaskDraftArtifactRecord {
            draft_id: "draft_job_1_preview_1".into(),
            workspace_uid: "workspace_drafts".into(),
            container_id: "container_1".into(),
            task_id: "job_1".into(),
            approval_id: "preview_1".into(),
            preview_ref: Some("blob://preview".into()),
            operation: Some("os.write_artifact".into()),
            status: "pending".into(),
            content_format: "markdown".into(),
            content_text: "# Draft".into(),
            created_at_ms: 10,
            updated_at_ms: 10,
        };

        let saved = db.upsert_task_draft_artifact(&draft).unwrap();
        assert_eq!(saved.content_text, "# Draft");

        let updated = db
            .update_task_draft_artifact_content("job_1", "preview_1", "# Edited")
            .unwrap()
            .unwrap();
        assert_eq!(updated.status, "edited");
        assert_eq!(updated.content_text, "# Edited");

        let listed = db.list_task_draft_artifacts("job_1").unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].approval_id, "preview_1");

        db.delete_task_draft_artifact("job_1", "preview_1").unwrap();
        assert!(db
            .get_task_draft_artifact("job_1", "preview_1")
            .unwrap()
            .is_none());
    }

    #[test]
    fn container_projection_list_order_is_stable_after_activation_update() {
        let root = temp_root("container_stable_order");
        let db = ProductDb::open(&root, "workspace_order".into()).unwrap();
        let first = ContainerRecord {
            container_id: "container_a".into(),
            workspace_uid: "workspace_order".into(),
            title: "A".into(),
            status: ContainerStatus::Active,
            badges: Default::default(),
            created_at_ms: 1,
            updated_at_ms: 1,
            last_active_at_ms: 1,
            default_model_config: None,
            context_policy: None,
        };
        let second = ContainerRecord {
            container_id: "container_b".into(),
            workspace_uid: "workspace_order".into(),
            title: "B".into(),
            status: ContainerStatus::Active,
            badges: Default::default(),
            created_at_ms: 2,
            updated_at_ms: 2,
            last_active_at_ms: 2,
            default_model_config: None,
            context_policy: None,
        };
        db.upsert_container_projection(&first).unwrap();
        db.upsert_container_projection(&second).unwrap();
        db.set_active_container_projection(&second.container_id)
            .unwrap();

        let listed = db.list_container_projections(false).unwrap();
        assert_eq!(listed[0].container_id, first.container_id);
        assert_eq!(listed[1].container_id, second.container_id);
    }
}
