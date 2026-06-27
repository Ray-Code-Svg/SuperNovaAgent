use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use local_runtime_protocol::{ContainerMessage, MessageLane, MessageRole, MessageType};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;

use crate::state::message_feed::{
    message_cursor_event_id, normalize_message_page_limit, sort_messages_for_display_page,
};
use crate::state::product_db::ProductDb;
use crate::state::workspace_registry::now_ms;

const SHARD_BUSY_TIMEOUT_MS: u64 = 5_000;
const MAX_CONTAINER_AGGREGATED_SHARDS: usize = 64;
const MESSAGE_SELECT_COLUMNS: &str = "message_id, workspace_uid, container_id, lane, role, message_type, status, title, body_text, body_json, card_json, chat_thread_id, task_id, job_id, source_kind, source_ref, source_seq, created_at_ms, updated_at_ms, sort_key";
const MESSAGE_CURSOR_SQL: &str = "MAX((CAST(substr(sort_key, 1, 20) AS INTEGER) * 1000000) + CAST(substr(sort_key, 22, 10) AS INTEGER), (updated_at_ms * 1000000) + MAX(0, MIN(CAST(COALESCE(json_extract(body_json, '$._feed_cursor_seq'), source_seq, 0) AS INTEGER), 999999)))";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectionShardRecord {
    pub shard_id: String,
    pub workspace_uid: String,
    pub container_id: String,
    pub shard_kind: String,
    pub chat_thread_id: Option<String>,
    pub task_id: Option<String>,
    pub job_id: Option<String>,
    pub relative_db_path: String,
    pub status: String,
    pub created_at_ms: u128,
    pub updated_at_ms: u128,
}

#[derive(Clone, Debug)]
pub struct ProjectionShardManager {
    workspace_state_root: PathBuf,
}

#[derive(Clone, Debug)]
pub struct ProjectionShardDb {
    pub db_path: PathBuf,
}

impl ProjectionShardManager {
    pub fn new(workspace_state_root: impl AsRef<Path>) -> Self {
        Self {
            workspace_state_root: workspace_state_root.as_ref().to_path_buf(),
        }
    }

    pub fn for_product_db(db: &ProductDb) -> Self {
        let root = db
            .db_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        Self::new(root)
    }

    pub fn task_job_record(
        &self,
        workspace_uid: &str,
        container_id: &str,
        task_id: &str,
        job_id: &str,
    ) -> ProjectionShardRecord {
        let relative = format!("projection_shards/task/{}.sqlite3", safe_shard_name(job_id));
        let now = now_ms() as u128;
        ProjectionShardRecord {
            shard_id: format!("task_job:{job_id}"),
            workspace_uid: workspace_uid.to_string(),
            container_id: container_id.to_string(),
            shard_kind: "task_job".to_string(),
            chat_thread_id: None,
            task_id: Some(task_id.to_string()),
            job_id: Some(job_id.to_string()),
            relative_db_path: relative,
            status: "active".to_string(),
            created_at_ms: now,
            updated_at_ms: now,
        }
    }

    pub fn chat_thread_record(
        &self,
        workspace_uid: &str,
        container_id: &str,
        chat_thread_id: &str,
    ) -> ProjectionShardRecord {
        let relative = format!(
            "projection_shards/chat/{}.sqlite3",
            safe_shard_name(chat_thread_id)
        );
        let now = now_ms() as u128;
        ProjectionShardRecord {
            shard_id: format!("chat_thread:{chat_thread_id}"),
            workspace_uid: workspace_uid.to_string(),
            container_id: container_id.to_string(),
            shard_kind: "chat_thread".to_string(),
            chat_thread_id: Some(chat_thread_id.to_string()),
            task_id: None,
            job_id: None,
            relative_db_path: relative,
            status: "active".to_string(),
            created_at_ms: now,
            updated_at_ms: now,
        }
    }

    pub fn open_shard(
        &self,
        record: &ProjectionShardRecord,
    ) -> rusqlite::Result<ProjectionShardDb> {
        let db_path = self.workspace_state_root.join(&record.relative_db_path);
        ProjectionShardDb::open(db_path)
    }

    pub fn open_existing_shard(
        &self,
        record: &ProjectionShardRecord,
    ) -> rusqlite::Result<Option<ProjectionShardDb>> {
        let db_path = self.workspace_state_root.join(&record.relative_db_path);
        if !db_path.exists() {
            return Ok(None);
        }
        ProjectionShardDb::open(db_path).map(Some)
    }
}

impl ProjectionShardDb {
    pub fn open(db_path: impl AsRef<Path>) -> rusqlite::Result<Self> {
        if let Some(parent) = db_path.as_ref().parent() {
            std::fs::create_dir_all(parent).map_err(io_to_sqlite)?;
        }
        let db_path = db_path.as_ref().to_path_buf();
        let conn = open_shard_connection(&db_path)?;
        init_projection_shard_db(&conn)?;
        Ok(Self { db_path })
    }

    fn connect(&self) -> rusqlite::Result<Connection> {
        open_shard_connection(&self.db_path)
    }

    pub fn append_message(
        &self,
        mut message: ContainerMessage,
    ) -> rusqlite::Result<ContainerMessage> {
        let now = now_ms() as u128;
        if message.created_at_ms == 0 {
            message.created_at_ms = now;
        }
        let conn = self.connect()?;
        if message.updated_at_ms == 0 {
            message.updated_at_ms = now;
        }
        if let Some((created_at_ms, updated_at_ms, sort_key)) = conn
            .query_row(
                "SELECT created_at_ms, updated_at_ms, sort_key FROM messages WHERE message_id=?1",
                params![&message.message_id],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)? as u128,
                        row.get::<_, i64>(1)? as u128,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()?
        {
            message.created_at_ms = created_at_ms.min(message.created_at_ms);
            message.updated_at_ms = updated_at_ms.max(message.updated_at_ms);
            message.sort_key = sort_key;
        }
        conn.execute(
            r#"
            INSERT OR REPLACE INTO messages(
              message_id, workspace_uid, container_id, lane, role, message_type, status,
              title, body_text, body_json, card_json, chat_thread_id, task_id, job_id,
              source_kind, source_ref, source_seq, created_at_ms, updated_at_ms, sort_key
            ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)
            "#,
            params![
                &message.message_id,
                &message.workspace_uid,
                &message.container_id,
                lane_str(&message.lane),
                role_str(&message.role),
                message_type_str(&message.message_type),
                &message.status,
                &message.title,
                &message.body_text,
                serde_json::to_string(&message.body_json).unwrap_or_else(|_| "{}".into()),
                serde_json::to_string(&message.card_json).unwrap_or_else(|_| "{}".into()),
                &message.chat_thread_id,
                &message.task_id,
                &message.job_id,
                &message.source_kind,
                &message.source_ref,
                message.source_seq,
                message.created_at_ms as i64,
                message.updated_at_ms as i64,
                &message.sort_key,
            ],
        )?;
        Ok(message)
    }

    pub fn list_messages_page(
        &self,
        after_event_id: Option<i64>,
        limit: Option<usize>,
    ) -> rusqlite::Result<Vec<ContainerMessage>> {
        self.list_messages_page_for_lane(None, after_event_id, limit)
    }

    pub fn list_messages_page_for_lane(
        &self,
        lane: Option<&MessageLane>,
        after_event_id: Option<i64>,
        limit: Option<usize>,
    ) -> rusqlite::Result<Vec<ContainerMessage>> {
        let bounded_limit = normalize_message_page_limit(limit);
        let conn = self.connect()?;
        if let Some(after_event_id) = after_event_id {
            if let Some(lane) = lane {
                let sql = format!(
                    "SELECT {MESSAGE_SELECT_COLUMNS} FROM messages WHERE lane=?1 AND {MESSAGE_CURSOR_SQL} > ?2 ORDER BY sort_key ASC LIMIT ?3"
                );
                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt.query_map(
                    params![lane_str(lane), after_event_id, bounded_limit as i64],
                    row_to_message,
                )?;
                return rows.collect();
            }
            let sql = format!(
                "SELECT {MESSAGE_SELECT_COLUMNS} FROM messages WHERE {MESSAGE_CURSOR_SQL} > ?1 ORDER BY sort_key ASC LIMIT ?2"
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(
                params![after_event_id, bounded_limit as i64],
                row_to_message,
            )?;
            return rows.collect();
        }
        if let Some(lane) = lane {
            let sql = format!(
                "SELECT {MESSAGE_SELECT_COLUMNS} FROM (SELECT {MESSAGE_SELECT_COLUMNS} FROM messages WHERE lane=?1 ORDER BY sort_key DESC LIMIT ?2) ORDER BY sort_key ASC"
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(
                params![lane_str(lane), bounded_limit as i64],
                row_to_message,
            )?;
            return rows.collect();
        }
        let sql = format!(
            "SELECT {MESSAGE_SELECT_COLUMNS} FROM (SELECT {MESSAGE_SELECT_COLUMNS} FROM messages ORDER BY sort_key DESC LIMIT ?1) ORDER BY sort_key ASC"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params![bounded_limit as i64], row_to_message)?;
        rows.collect()
    }

    pub fn upsert_run_state(
        &self,
        run_id: &str,
        status: &str,
        updated_at_ms: u128,
    ) -> rusqlite::Result<()> {
        let conn = self.connect()?;
        conn.execute(
            "INSERT INTO run_state(run_id, status, updated_at_ms)
             VALUES(?1, ?2, ?3)
             ON CONFLICT(run_id) DO UPDATE SET status=excluded.status, updated_at_ms=excluded.updated_at_ms",
            params![run_id, status, updated_at_ms as i64],
        )?;
        Ok(())
    }

    pub fn append_runtime_event(
        &self,
        event_id: &str,
        event_type: &str,
        payload: Value,
    ) -> rusqlite::Result<()> {
        let mut conn = self.connect()?;
        let tx = conn.transaction()?;
        let exists: Option<String> = tx
            .query_row(
                "SELECT event_id FROM runtime_events WHERE event_id=?1",
                params![event_id],
                |row| row.get(0),
            )
            .optional()?;
        if exists.is_some() {
            tx.commit()?;
            return Ok(());
        }
        let cursor_seq: i64 = tx.query_row(
            "SELECT COALESCE(MAX(cursor_seq), 0) + 1 FROM runtime_events",
            [],
            |row| row.get(0),
        )?;
        tx.execute(
            "INSERT INTO runtime_events(event_id, event_type, payload_json, created_at_ms, cursor_seq)
             VALUES(?1, ?2, ?3, ?4, ?5)",
            params![
                event_id,
                event_type,
                serde_json::to_string(&payload).unwrap_or_else(|_| "{}".into()),
                now_ms() as i64,
                cursor_seq,
            ],
        )?;
        tx.commit()
    }
}

impl ProductDb {
    pub fn upsert_projection_shard(
        &self,
        record: &ProjectionShardRecord,
    ) -> rusqlite::Result<ProjectionShardRecord> {
        let now = now_ms() as u128;
        let conn = self.connect()?;
        conn.execute(
            "INSERT INTO projection_shards(
              shard_id, workspace_uid, container_id, shard_kind, chat_thread_id, task_id,
              job_id, relative_db_path, status, created_at_ms, updated_at_ms
            ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
            ON CONFLICT(shard_id) DO UPDATE SET
              workspace_uid=excluded.workspace_uid,
              container_id=excluded.container_id,
              shard_kind=excluded.shard_kind,
              chat_thread_id=excluded.chat_thread_id,
              task_id=excluded.task_id,
              job_id=excluded.job_id,
              relative_db_path=excluded.relative_db_path,
              status=excluded.status,
              updated_at_ms=excluded.updated_at_ms",
            params![
                &record.shard_id,
                &record.workspace_uid,
                &record.container_id,
                &record.shard_kind,
                &record.chat_thread_id,
                &record.task_id,
                &record.job_id,
                &record.relative_db_path,
                &record.status,
                record.created_at_ms as i64,
                now as i64,
            ],
        )?;
        self.get_projection_shard(&record.shard_id)
    }

    pub fn get_projection_shard(&self, shard_id: &str) -> rusqlite::Result<ProjectionShardRecord> {
        let conn = self.connect()?;
        conn.query_row(
            "SELECT shard_id, workspace_uid, container_id, shard_kind, chat_thread_id, task_id,
              job_id, relative_db_path, status, created_at_ms, updated_at_ms
             FROM projection_shards WHERE shard_id=?1",
            params![shard_id],
            row_to_shard_record,
        )
    }

    pub fn projection_shard_for_task_job(
        &self,
        job_id: &str,
    ) -> rusqlite::Result<Option<ProjectionShardRecord>> {
        let conn = self.connect()?;
        conn.query_row(
            "SELECT shard_id, workspace_uid, container_id, shard_kind, chat_thread_id, task_id,
              job_id, relative_db_path, status, created_at_ms, updated_at_ms
             FROM projection_shards WHERE job_id=?1 ORDER BY updated_at_ms DESC LIMIT 1",
            params![job_id],
            row_to_shard_record,
        )
        .optional()
    }

    pub fn projection_shard_for_chat_thread(
        &self,
        chat_thread_id: &str,
    ) -> rusqlite::Result<Option<ProjectionShardRecord>> {
        let conn = self.connect()?;
        conn.query_row(
            "SELECT shard_id, workspace_uid, container_id, shard_kind, chat_thread_id, task_id,
              job_id, relative_db_path, status, created_at_ms, updated_at_ms
             FROM projection_shards WHERE chat_thread_id=?1 ORDER BY updated_at_ms DESC LIMIT 1",
            params![chat_thread_id],
            row_to_shard_record,
        )
        .optional()
    }

    pub fn list_container_projection_shards(
        &self,
        container_id: &str,
    ) -> rusqlite::Result<Vec<ProjectionShardRecord>> {
        let conn = self.connect()?;
        let mut stmt = conn.prepare(
            "SELECT shard_id, workspace_uid, container_id, shard_kind, chat_thread_id, task_id,
              job_id, relative_db_path, status, created_at_ms, updated_at_ms
             FROM projection_shards WHERE container_id=?1 ORDER BY updated_at_ms DESC, shard_id DESC",
        )?;
        let rows = stmt.query_map(params![container_id], row_to_shard_record)?;
        rows.collect()
    }

    pub fn ensure_chat_projection_shard(
        &self,
        container_id: &str,
        chat_thread_id: &str,
    ) -> rusqlite::Result<ProjectionShardDb> {
        let manager = ProjectionShardManager::for_product_db(self);
        let record = manager.chat_thread_record(&self.workspace_uid, container_id, chat_thread_id);
        let record = self.upsert_projection_shard(&record)?;
        manager.open_shard(&record)
    }

    pub fn ensure_task_projection_shard(
        &self,
        container_id: &str,
        task_id: &str,
        job_id: &str,
    ) -> rusqlite::Result<ProjectionShardDb> {
        let manager = ProjectionShardManager::for_product_db(self);
        let record = manager.task_job_record(&self.workspace_uid, container_id, task_id, job_id);
        let record = self.upsert_projection_shard(&record)?;
        manager.open_shard(&record)
    }

    pub fn append_chat_projection_message(
        &self,
        chat_thread_id: &str,
        message: ContainerMessage,
    ) -> rusqlite::Result<ContainerMessage> {
        let record = self.get_chat_thread(chat_thread_id)?;
        let shard = self.ensure_chat_projection_shard(&record.container_id, chat_thread_id)?;
        shard.append_message(message)
    }

    pub fn append_task_projection_message(
        &self,
        task_id: &str,
        job_id: &str,
        container_id: &str,
        message: ContainerMessage,
    ) -> rusqlite::Result<ContainerMessage> {
        let shard = self.ensure_task_projection_shard(container_id, task_id, job_id)?;
        shard.append_message(message)
    }

    pub fn list_projected_container_messages_page(
        &self,
        container_id: &str,
        after_event_id: Option<i64>,
        limit: Option<usize>,
    ) -> rusqlite::Result<Vec<ContainerMessage>> {
        self.list_projected_container_messages_page_for_lane(
            container_id,
            None,
            after_event_id,
            limit,
        )
    }

    pub fn list_projected_container_messages_page_for_lane(
        &self,
        container_id: &str,
        lane: Option<&MessageLane>,
        after_event_id: Option<i64>,
        limit: Option<usize>,
    ) -> rusqlite::Result<Vec<ContainerMessage>> {
        let mut candidates = if let Some(lane) = lane {
            self.list_container_messages_page_for_lane(container_id, lane, after_event_id, limit)?
        } else {
            self.list_container_messages_page(container_id, after_event_id, limit)?
        };
        let manager = ProjectionShardManager::for_product_db(self);
        for record in self
            .list_container_projection_shards(container_id)?
            .into_iter()
            .filter(|record| record.status == "active")
            .take(MAX_CONTAINER_AGGREGATED_SHARDS)
        {
            let Some(shard) = manager.open_existing_shard(&record)? else {
                continue;
            };
            candidates.extend(
                shard
                    .list_messages_page_for_lane(lane, after_event_id, limit)?
                    .into_iter()
                    .filter(|message| {
                        message.container_id == container_id
                            && lane
                                .map(|expected| message.lane == *expected)
                                .unwrap_or(true)
                    }),
            );
        }
        Ok(merge_projected_messages(candidates, after_event_id, limit))
    }

    pub fn list_projected_chat_messages_page(
        &self,
        chat_thread_id: &str,
        after_event_id: Option<i64>,
        limit: Option<usize>,
    ) -> rusqlite::Result<Vec<ContainerMessage>> {
        let mut candidates = self.list_chat_messages_page(chat_thread_id, after_event_id, limit)?;
        if let Some(record) = self.projection_shard_for_chat_thread(chat_thread_id)? {
            if record.status == "active" {
                let manager = ProjectionShardManager::for_product_db(self);
                if let Some(shard) = manager.open_existing_shard(&record)? {
                    candidates.extend(
                        shard
                            .list_messages_page(after_event_id, limit)?
                            .into_iter()
                            .filter(|message| {
                                message.chat_thread_id.as_deref() == Some(chat_thread_id)
                            }),
                    );
                }
            }
        }
        Ok(merge_projected_messages(candidates, after_event_id, limit))
    }

    pub fn list_projected_task_messages_page(
        &self,
        task_id: &str,
        after_event_id: Option<i64>,
        limit: Option<usize>,
    ) -> rusqlite::Result<Vec<ContainerMessage>> {
        let mut candidates = self.list_task_messages_page(task_id, after_event_id, limit)?;
        if let Ok(task) = self.get_task(task_id) {
            if let Some(job_id) = task.job_id.as_deref() {
                if let Some(record) = self.projection_shard_for_task_job(job_id)? {
                    if record.status == "active" {
                        let manager = ProjectionShardManager::for_product_db(self);
                        if let Some(shard) = manager.open_existing_shard(&record)? {
                            candidates.extend(
                                shard
                                    .list_messages_page(after_event_id, limit)?
                                    .into_iter()
                                    .filter(|message| message.task_id.as_deref() == Some(task_id)),
                            );
                        }
                    }
                }
            }
        }
        Ok(merge_projected_messages(candidates, after_event_id, limit))
    }
}

fn merge_projected_messages(
    candidates: Vec<ContainerMessage>,
    after_event_id: Option<i64>,
    limit: Option<usize>,
) -> Vec<ContainerMessage> {
    let bounded_limit = normalize_message_page_limit(limit);
    let mut deduped: HashMap<String, ContainerMessage> = HashMap::new();
    for message in candidates {
        if after_event_id
            .map(|after| message_cursor_event_id(&message) <= after)
            .unwrap_or(false)
        {
            continue;
        }
        match deduped.get(&message.message_id) {
            Some(existing)
                if message_cursor_event_id(existing) >= message_cursor_event_id(&message) => {}
            _ => {
                deduped.insert(message.message_id.clone(), message);
            }
        }
    }
    let mut messages = deduped.into_values().collect::<Vec<_>>();
    sort_messages_for_display_page(&mut messages);
    if after_event_id.is_some() {
        messages.into_iter().take(bounded_limit).collect()
    } else if messages.len() > bounded_limit {
        messages[messages.len() - bounded_limit..].to_vec()
    } else {
        messages
    }
}

fn open_shard_connection(path: &Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open(path)?;
    conn.busy_timeout(Duration::from_millis(SHARD_BUSY_TIMEOUT_MS))?;
    conn.execute_batch(
        r#"
        PRAGMA journal_mode = WAL;
        PRAGMA synchronous = NORMAL;
        "#,
    )?;
    Ok(conn)
}

fn init_projection_shard_db(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS messages(
          message_id TEXT PRIMARY KEY,
          workspace_uid TEXT NOT NULL,
          container_id TEXT NOT NULL,
          lane TEXT NOT NULL,
          role TEXT NOT NULL,
          message_type TEXT NOT NULL,
          status TEXT NOT NULL,
          title TEXT,
          body_text TEXT,
          body_json TEXT NOT NULL,
          card_json TEXT NOT NULL,
          chat_thread_id TEXT,
          task_id TEXT,
          job_id TEXT,
          source_kind TEXT NOT NULL,
          source_ref TEXT NOT NULL,
          source_seq INTEGER,
          created_at_ms INTEGER NOT NULL,
          updated_at_ms INTEGER NOT NULL,
          sort_key TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_shard_messages_sort ON messages(sort_key);
        CREATE INDEX IF NOT EXISTS idx_shard_messages_chat ON messages(chat_thread_id, sort_key);
        CREATE INDEX IF NOT EXISTS idx_shard_messages_task ON messages(task_id, sort_key);

        CREATE TABLE IF NOT EXISTS runtime_events(
          event_id TEXT PRIMARY KEY,
          event_type TEXT NOT NULL,
          payload_json TEXT NOT NULL,
          created_at_ms INTEGER NOT NULL,
          cursor_seq INTEGER NOT NULL UNIQUE
        );
        CREATE INDEX IF NOT EXISTS idx_shard_runtime_events_cursor ON runtime_events(cursor_seq);

        CREATE TABLE IF NOT EXISTS run_state(
          run_id TEXT PRIMARY KEY,
          status TEXT NOT NULL,
          updated_at_ms INTEGER NOT NULL
        );
        "#,
    )
}

fn row_to_shard_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<ProjectionShardRecord> {
    Ok(ProjectionShardRecord {
        shard_id: row.get(0)?,
        workspace_uid: row.get(1)?,
        container_id: row.get(2)?,
        shard_kind: row.get(3)?,
        chat_thread_id: row.get(4)?,
        task_id: row.get(5)?,
        job_id: row.get(6)?,
        relative_db_path: row.get(7)?,
        status: row.get(8)?,
        created_at_ms: row.get::<_, i64>(9)? as u128,
        updated_at_ms: row.get::<_, i64>(10)? as u128,
    })
}

fn row_to_message(row: &rusqlite::Row<'_>) -> rusqlite::Result<ContainerMessage> {
    Ok(ContainerMessage {
        message_id: row.get(0)?,
        workspace_uid: row.get(1)?,
        container_id: row.get(2)?,
        lane: parse_lane(row.get::<_, String>(3)?.as_str()),
        role: parse_role(row.get::<_, String>(4)?.as_str()),
        message_type: parse_message_type(row.get::<_, String>(5)?.as_str()),
        status: row.get(6)?,
        title: row.get(7)?,
        body_text: row.get(8)?,
        body_json: parse_json(row.get::<_, String>(9)?),
        card_json: parse_json(row.get::<_, String>(10)?),
        chat_thread_id: row.get(11)?,
        task_id: row.get(12)?,
        job_id: row.get(13)?,
        source_kind: row.get(14)?,
        source_ref: row.get(15)?,
        source_seq: row.get(16)?,
        created_at_ms: row.get::<_, i64>(17)? as u128,
        updated_at_ms: row.get::<_, i64>(18)? as u128,
        sort_key: row.get(19)?,
    })
}

fn parse_json(raw: String) -> Value {
    serde_json::from_str(&raw).unwrap_or(Value::Object(Default::default()))
}

fn lane_str(value: &MessageLane) -> &'static str {
    match value {
        MessageLane::Chat => "chat",
        MessageLane::Task => "task",
        MessageLane::Runtime => "runtime",
    }
}

fn role_str(value: &MessageRole) -> &'static str {
    match value {
        MessageRole::User => "user",
        MessageRole::Assistant => "assistant",
        MessageRole::Agent => "agent",
        MessageRole::Tool => "tool",
        MessageRole::System => "system",
    }
}

fn message_type_str(value: &MessageType) -> &'static str {
    match value {
        MessageType::Text => "text",
        MessageType::Reasoning => "reasoning",
        MessageType::ToolCall => "tool_call",
        MessageType::ToolResult => "tool_result",
        MessageType::Approval => "approval",
        MessageType::Artifact => "artifact",
        MessageType::Phase => "phase",
        MessageType::Error => "error",
    }
}

fn parse_lane(value: &str) -> MessageLane {
    match value {
        "task" => MessageLane::Task,
        "runtime" => MessageLane::Runtime,
        _ => MessageLane::Chat,
    }
}

fn parse_role(value: &str) -> MessageRole {
    match value {
        "assistant" => MessageRole::Assistant,
        "agent" => MessageRole::Agent,
        "tool" => MessageRole::Tool,
        "system" => MessageRole::System,
        _ => MessageRole::User,
    }
}

fn parse_message_type(value: &str) -> MessageType {
    match value {
        "reasoning" => MessageType::Reasoning,
        "tool_call" => MessageType::ToolCall,
        "tool_result" => MessageType::ToolResult,
        "approval" => MessageType::Approval,
        "artifact" => MessageType::Artifact,
        "phase" => MessageType::Phase,
        "error" => MessageType::Error,
        _ => MessageType::Text,
    }
}

fn safe_shard_name(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        "shard".to_string()
    } else {
        out
    }
}

fn io_to_sqlite(err: std::io::Error) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(Box::new(err))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::message_feed::new_message;
    use local_runtime_protocol::{ChatThreadRecord, ContainerBadges, TaskRecord};

    fn temp_root(name: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("supernova_projection_shards_{name}_{}", now_ms()));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn fixed_message(
        message_id: &str,
        workspace_uid: &str,
        container_id: &str,
        lane: MessageLane,
        text: &str,
        sort_millis: u128,
    ) -> ContainerMessage {
        let mut message = new_message(
            workspace_uid,
            container_id,
            lane,
            MessageRole::Assistant,
            MessageType::Text,
            Some(text.to_string()),
            None,
        );
        message.message_id = message_id.to_string();
        message.source_ref = message_id.to_string();
        message.created_at_ms = sort_millis;
        message.updated_at_ms = sort_millis;
        message.sort_key = format!("{sort_millis:020}_{:010}", 1);
        message
    }

    fn body_texts(messages: &[ContainerMessage]) -> Vec<String> {
        messages
            .iter()
            .map(|message| message.body_text.clone().unwrap_or_default())
            .collect()
    }

    #[test]
    fn projection_shard_index_round_trips_task_and_chat_records() {
        let root = temp_root("index_round_trip");
        let db = ProductDb::open(&root, "workspace_shards".into()).unwrap();
        let manager = ProjectionShardManager::new(&root);
        let task_record =
            manager.task_job_record("workspace_shards", "container_1", "task_1", "job_1");
        let chat_record = manager.chat_thread_record("workspace_shards", "container_1", "chat_1");

        db.upsert_projection_shard(&task_record).unwrap();
        db.upsert_projection_shard(&chat_record).unwrap();

        assert_eq!(
            db.projection_shard_for_task_job("job_1")
                .unwrap()
                .unwrap()
                .relative_db_path,
            "projection_shards/task/job_1.sqlite3"
        );
        assert_eq!(
            db.projection_shard_for_chat_thread("chat_1")
                .unwrap()
                .unwrap()
                .relative_db_path,
            "projection_shards/chat/chat_1.sqlite3"
        );
        assert_eq!(
            db.list_container_projection_shards("container_1")
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn projection_shard_messages_are_isolated_by_db_file() {
        let root = temp_root("message_isolation");
        let manager = ProjectionShardManager::new(&root);
        let task_record =
            manager.task_job_record("workspace_shards", "container_1", "task_1", "job_1");
        let chat_record = manager.chat_thread_record("workspace_shards", "container_2", "chat_1");
        let task_shard = manager.open_shard(&task_record).unwrap();
        let chat_shard = manager.open_shard(&chat_record).unwrap();
        let mut task_message = new_message(
            "workspace_shards",
            "container_1",
            MessageLane::Task,
            MessageRole::Agent,
            MessageType::Text,
            Some("task done".into()),
            None,
        );
        task_message.task_id = Some("task_1".into());
        task_message.job_id = Some("job_1".into());
        let chat_message = new_message(
            "workspace_shards",
            "container_2",
            MessageLane::Chat,
            MessageRole::Assistant,
            MessageType::Text,
            Some("chat done".into()),
            Some("chat_1".into()),
        );

        task_shard.append_message(task_message).unwrap();
        chat_shard.append_message(chat_message).unwrap();

        let task_page = task_shard.list_messages_page(None, Some(20)).unwrap();
        let chat_page = chat_shard.list_messages_page(None, Some(20)).unwrap();
        assert_eq!(task_page.len(), 1);
        assert_eq!(task_page[0].container_id, "container_1");
        assert_eq!(chat_page.len(), 1);
        assert_eq!(chat_page[0].container_id, "container_2");
        assert_ne!(task_shard.db_path, chat_shard.db_path);

        let cursor = crate::state::message_feed::message_cursor_event_id(&task_page[0]);
        assert!(task_shard
            .list_messages_page(Some(cursor), Some(20))
            .unwrap()
            .is_empty());
    }

    #[test]
    fn projected_container_page_merges_workspace_and_shard_messages() {
        let root = temp_root("container_merge");
        let db = ProductDb::open(&root, "workspace_shards".into()).unwrap();
        let manager = ProjectionShardManager::new(&root);
        let workspace_message = fixed_message(
            "workspace_msg",
            "workspace_shards",
            "container_1",
            MessageLane::Chat,
            "workspace old",
            1_000,
        );
        db.append_message(workspace_message.clone()).unwrap();

        let record = manager.chat_thread_record("workspace_shards", "container_1", "chat_1");
        db.upsert_projection_shard(&record).unwrap();
        let shard = manager.open_shard(&record).unwrap();
        let mut shard_message = fixed_message(
            "shard_msg",
            "workspace_shards",
            "container_1",
            MessageLane::Chat,
            "shard new",
            2_000,
        );
        shard_message.chat_thread_id = Some("chat_1".into());
        shard.append_message(shard_message).unwrap();

        let page = db
            .list_projected_container_messages_page("container_1", None, Some(10))
            .unwrap();
        assert_eq!(body_texts(&page), vec!["workspace old", "shard new"]);

        let incremental = db
            .list_projected_container_messages_page(
                "container_1",
                Some(message_cursor_event_id(&workspace_message)),
                Some(10),
            )
            .unwrap();
        assert_eq!(body_texts(&incremental), vec!["shard new"]);
    }

    #[test]
    fn projected_container_page_filters_by_lane_before_limit() {
        let root = temp_root("container_lane_filter");
        let db = ProductDb::open(&root, "workspace_shards".into()).unwrap();
        let manager = ProjectionShardManager::new(&root);

        let chat_record = manager.chat_thread_record("workspace_shards", "container_1", "chat_1");
        db.upsert_projection_shard(&chat_record).unwrap();
        let chat_shard = manager.open_shard(&chat_record).unwrap();
        for index in 0..2 {
            let mut message = fixed_message(
                &format!("chat_msg_{index}"),
                "workspace_shards",
                "container_1",
                MessageLane::Chat,
                &format!("chat {index}"),
                1_000 + index,
            );
            message.chat_thread_id = Some("chat_1".into());
            chat_shard.append_message(message).unwrap();
        }

        let task_record =
            manager.task_job_record("workspace_shards", "container_1", "task_1", "job_1");
        db.upsert_projection_shard(&task_record).unwrap();
        let task_shard = manager.open_shard(&task_record).unwrap();
        for index in 0..10 {
            let mut message = fixed_message(
                &format!("task_msg_{index}"),
                "workspace_shards",
                "container_1",
                MessageLane::Task,
                &format!("task {index}"),
                2_000 + index,
            );
            message.task_id = Some("task_1".into());
            message.job_id = Some("job_1".into());
            task_shard.append_message(message).unwrap();
        }

        let page = db
            .list_projected_container_messages_page_for_lane(
                "container_1",
                Some(&MessageLane::Chat),
                None,
                Some(2),
            )
            .unwrap();

        assert_eq!(body_texts(&page), vec!["chat 0", "chat 1"]);
        assert!(page.iter().all(|message| message.lane == MessageLane::Chat));
    }

    #[test]
    fn projected_chat_and_task_pages_read_target_shards() {
        let root = temp_root("scoped_pages");
        let db = ProductDb::open(&root, "workspace_shards".into()).unwrap();
        let manager = ProjectionShardManager::new(&root);
        db.upsert_chat_thread(&ChatThreadRecord {
            chat_thread_id: "chat_1".into(),
            container_id: "container_1".into(),
            title: "Chat".into(),
            created_at_ms: 1,
            updated_at_ms: 1,
        })
        .unwrap();
        db.upsert_task(&TaskRecord {
            task_id: "task_1".into(),
            container_id: "container_1".into(),
            job_id: Some("job_1".into()),
            title: "Task".into(),
            goal: "Do task".into(),
            status: "running".into(),
            badges: ContainerBadges::default(),
            created_at_ms: 1,
            updated_at_ms: 1,
        })
        .unwrap();

        let chat_record = manager.chat_thread_record("workspace_shards", "container_1", "chat_1");
        db.upsert_projection_shard(&chat_record).unwrap();
        let mut chat_message = fixed_message(
            "chat_msg",
            "workspace_shards",
            "container_1",
            MessageLane::Chat,
            "chat shard",
            1_000,
        );
        chat_message.chat_thread_id = Some("chat_1".into());
        manager
            .open_shard(&chat_record)
            .unwrap()
            .append_message(chat_message)
            .unwrap();

        let task_record =
            manager.task_job_record("workspace_shards", "container_1", "task_1", "job_1");
        db.upsert_projection_shard(&task_record).unwrap();
        let mut task_message = fixed_message(
            "task_msg",
            "workspace_shards",
            "container_1",
            MessageLane::Task,
            "task shard",
            2_000,
        );
        task_message.task_id = Some("task_1".into());
        task_message.job_id = Some("job_1".into());
        manager
            .open_shard(&task_record)
            .unwrap()
            .append_message(task_message)
            .unwrap();

        assert_eq!(
            body_texts(
                &db.list_projected_chat_messages_page("chat_1", None, Some(10))
                    .unwrap()
            ),
            vec!["chat shard"]
        );
        assert_eq!(
            body_texts(
                &db.list_projected_task_messages_page("task_1", None, Some(10))
                    .unwrap()
            ),
            vec!["task shard"]
        );
    }

    #[test]
    fn projected_pages_dedupe_by_message_id_with_newest_cursor() {
        let root = temp_root("dedupe");
        let db = ProductDb::open(&root, "workspace_shards".into()).unwrap();
        let manager = ProjectionShardManager::new(&root);
        let workspace_message = fixed_message(
            "same_msg",
            "workspace_shards",
            "container_1",
            MessageLane::Chat,
            "old body",
            1_000,
        );
        db.append_message(workspace_message).unwrap();

        let record = manager.chat_thread_record("workspace_shards", "container_1", "chat_1");
        db.upsert_projection_shard(&record).unwrap();
        let mut shard_message = fixed_message(
            "same_msg",
            "workspace_shards",
            "container_1",
            MessageLane::Chat,
            "new body",
            1_000,
        );
        shard_message.chat_thread_id = Some("chat_1".into());
        shard_message.updated_at_ms = 2_000;
        manager
            .open_shard(&record)
            .unwrap()
            .append_message(shard_message)
            .unwrap();

        let page = db
            .list_projected_container_messages_page("container_1", None, Some(10))
            .unwrap();
        assert_eq!(body_texts(&page), vec!["new body"]);
    }
}
