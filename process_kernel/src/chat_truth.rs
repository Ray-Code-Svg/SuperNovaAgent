use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{json_err, now_ms, WorkspaceGuard, RUNTIME_DIR_NAME};

static CHAT_ID_COUNTER: AtomicU64 = AtomicU64::new(1);

pub const CHAT_TRUTH_SCHEMA_VERSION: &str = "supernova_chat_truth.v1";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatThread {
    pub chat_thread_id: String,
    pub container_id: String,
    pub title: Option<String>,
    pub status: String,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ChatEvent {
    pub event_id: String,
    pub chat_thread_id: String,
    pub container_id: String,
    pub event_seq: u64,
    pub event_type: String,
    pub payload: Value,
    pub blob_ref: Option<String>,
    pub created_at_ms: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatProviderTranscript {
    pub transcript_id: String,
    pub chat_thread_id: String,
    pub provider: String,
    pub model: String,
    pub messages_ref: String,
    pub summary_ref: Option<String>,
    pub live_suffix_ref: Option<String>,
    pub compacted_until_seq: Option<u64>,
    pub updated_at_ms: i64,
}

#[derive(Clone, Debug)]
pub struct ChatTruthStore {
    state_root: PathBuf,
    db_path: PathBuf,
}

impl ChatTruthStore {
    pub fn new(workspace_root: impl AsRef<Path>) -> io::Result<Self> {
        let guard = WorkspaceGuard::new(workspace_root)?;
        Self::new_with_state_root(guard.root(), guard.root().join(RUNTIME_DIR_NAME))
    }

    pub fn new_with_state_root(
        workspace_root: impl AsRef<Path>,
        state_root: impl AsRef<Path>,
    ) -> io::Result<Self> {
        let _guard = WorkspaceGuard::new(workspace_root)?;
        std::fs::create_dir_all(state_root.as_ref())?;
        let state_root = state_root.as_ref().canonicalize()?;
        let state_dir = state_root.join("state");
        std::fs::create_dir_all(&state_dir)?;
        let store = Self {
            state_root,
            db_path: state_dir.join("chat_truth.sqlite3"),
        };
        store.init_schema()?;
        Ok(store)
    }

    pub fn path(&self) -> &Path {
        &self.db_path
    }

    pub fn create_thread(
        &self,
        container_id: impl Into<String>,
        title: Option<String>,
    ) -> io::Result<ChatThread> {
        let now = now_ms_i64();
        let thread = ChatThread {
            chat_thread_id: next_id("chat"),
            container_id: container_id.into(),
            title,
            status: "active".to_string(),
            created_at_ms: now,
            updated_at_ms: now,
        };
        let conn = self.connect()?;
        conn.execute(
            r#"
            INSERT INTO chat_threads(
                chat_thread_id, container_id, title, status, created_at_ms, updated_at_ms
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            "#,
            params![
                thread.chat_thread_id,
                thread.container_id,
                thread.title,
                thread.status,
                thread.created_at_ms,
                thread.updated_at_ms,
            ],
        )
        .map_err(sql_err)?;
        Ok(thread)
    }

    pub fn get_thread(&self, chat_thread_id: &str) -> io::Result<ChatThread> {
        let conn = self.connect()?;
        conn.query_row(
            r#"
            SELECT chat_thread_id, container_id, title, status, created_at_ms, updated_at_ms
            FROM chat_threads
            WHERE chat_thread_id = ?1
            "#,
            params![chat_thread_id],
            row_to_thread,
        )
        .map_err(sql_err)
    }

    pub fn list_threads_for_container(&self, container_id: &str) -> io::Result<Vec<ChatThread>> {
        let conn = self.connect()?;
        let mut stmt = conn
            .prepare(
                r#"
                SELECT chat_thread_id, container_id, title, status, created_at_ms, updated_at_ms
                FROM chat_threads
                WHERE container_id = ?1
                ORDER BY updated_at_ms DESC, created_at_ms DESC
                "#,
            )
            .map_err(sql_err)?;
        let rows = stmt
            .query_map(params![container_id], row_to_thread)
            .map_err(sql_err)?;
        let mut values = Vec::new();
        for row in rows {
            values.push(row.map_err(sql_err)?);
        }
        Ok(values)
    }

    pub fn append_event(
        &self,
        chat_thread_id: &str,
        container_id: &str,
        event_type: &str,
        payload: Value,
        blob_ref: Option<String>,
    ) -> io::Result<ChatEvent> {
        let conn = self.connect()?;
        let next_seq = conn
            .query_row(
                "SELECT COALESCE(MAX(event_seq), 0) + 1 FROM chat_events WHERE chat_thread_id = ?1",
                params![chat_thread_id],
                |row| row.get::<_, i64>(0),
            )
            .map_err(sql_err)? as u64;
        let event = ChatEvent {
            event_id: next_id("chat_event"),
            chat_thread_id: chat_thread_id.to_string(),
            container_id: container_id.to_string(),
            event_seq: next_seq,
            event_type: event_type.to_string(),
            payload,
            blob_ref,
            created_at_ms: now_ms_i64(),
        };
        conn.execute(
            r#"
            INSERT INTO chat_events(
                event_id, chat_thread_id, container_id, event_seq, event_type,
                payload_json, blob_ref, created_at_ms
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
            params![
                event.event_id,
                event.chat_thread_id,
                event.container_id,
                event.event_seq as i64,
                event.event_type,
                serde_json::to_string(&event.payload).map_err(json_err)?,
                event.blob_ref,
                event.created_at_ms,
            ],
        )
        .map_err(sql_err)?;
        conn.execute(
            "UPDATE chat_threads SET updated_at_ms = ?1 WHERE chat_thread_id = ?2",
            params![event.created_at_ms, chat_thread_id],
        )
        .map_err(sql_err)?;
        Ok(event)
    }

    pub fn read_events(&self, chat_thread_id: &str) -> io::Result<Vec<ChatEvent>> {
        let conn = self.connect()?;
        let mut stmt = conn
            .prepare(
                r#"
                SELECT event_id, chat_thread_id, container_id, event_seq, event_type,
                       payload_json, blob_ref, created_at_ms
                FROM chat_events
                WHERE chat_thread_id = ?1
                ORDER BY event_seq ASC
                "#,
            )
            .map_err(sql_err)?;
        let rows = stmt
            .query_map(params![chat_thread_id], row_to_event)
            .map_err(sql_err)?;
        let mut values = Vec::new();
        for row in rows {
            values.push(row.map_err(sql_err)?);
        }
        Ok(values)
    }

    pub fn upsert_provider_transcript(
        &self,
        transcript: ChatProviderTranscript,
    ) -> io::Result<ChatProviderTranscript> {
        let conn = self.connect()?;
        conn.execute(
            r#"
            INSERT INTO chat_provider_transcript(
                transcript_id, chat_thread_id, provider, model, messages_ref, summary_ref,
                live_suffix_ref, compacted_until_seq, updated_at_ms
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            ON CONFLICT(transcript_id) DO UPDATE SET
                provider = excluded.provider,
                model = excluded.model,
                messages_ref = excluded.messages_ref,
                summary_ref = excluded.summary_ref,
                live_suffix_ref = excluded.live_suffix_ref,
                compacted_until_seq = excluded.compacted_until_seq,
                updated_at_ms = excluded.updated_at_ms
            "#,
            params![
                transcript.transcript_id,
                transcript.chat_thread_id,
                transcript.provider,
                transcript.model,
                transcript.messages_ref,
                transcript.summary_ref,
                transcript.live_suffix_ref,
                transcript.compacted_until_seq.map(|value| value as i64),
                transcript.updated_at_ms,
            ],
        )
        .map_err(sql_err)?;
        Ok(transcript)
    }

    pub fn list_provider_transcripts(
        &self,
        chat_thread_id: &str,
    ) -> io::Result<Vec<ChatProviderTranscript>> {
        let conn = self.connect()?;
        let mut stmt = conn
            .prepare(
                r#"
                SELECT transcript_id, chat_thread_id, provider, model, messages_ref, summary_ref,
                       live_suffix_ref, compacted_until_seq, updated_at_ms
                FROM chat_provider_transcript
                WHERE chat_thread_id = ?1
                ORDER BY updated_at_ms DESC
                "#,
            )
            .map_err(sql_err)?;
        let rows = stmt
            .query_map(params![chat_thread_id], row_to_provider_transcript)
            .map_err(sql_err)?;
        let mut values = Vec::new();
        for row in rows {
            values.push(row.map_err(sql_err)?);
        }
        Ok(values)
    }

    pub fn write_chat_blob(
        &self,
        chat_thread_id: &str,
        name: &str,
        content: &[u8],
    ) -> io::Result<String> {
        let safe_name = checked_relative_path(name)?;
        let blob_root = self
            .state_root
            .join("blobs")
            .join("chat")
            .join(chat_thread_id);
        let path = blob_root.join(&safe_name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, content)?;
        Ok(format!(
            "chat_blob://{}/{}",
            chat_thread_id,
            safe_name.to_string_lossy().replace('\\', "/")
        ))
    }

    pub fn read_chat_blob_text(&self, blob_ref: &str) -> io::Result<String> {
        let (chat_thread_id, relative) = parse_chat_blob_ref(blob_ref)?;
        let blob_root = self
            .state_root
            .join("blobs")
            .join("chat")
            .join(chat_thread_id);
        let path = blob_root.join(checked_relative_path(relative)?);
        std::fs::read_to_string(path)
    }

    pub fn read_chat_ref_text(&self, target_ref: &str) -> io::Result<String> {
        let target_ref = target_ref.trim();
        if target_ref.starts_with("chat_blob://") {
            return self.read_chat_blob_text(target_ref);
        }

        let payload = match parse_chat_ref(target_ref)? {
            ChatRefTarget::Thread { chat_thread_id } => {
                self.render_chat_thread_ref(target_ref, &chat_thread_id)?
            }
            ChatRefTarget::Turn {
                chat_thread_id,
                turn_id,
            } => self.render_chat_turn_ref(target_ref, chat_thread_id.as_deref(), &turn_id)?,
        };
        serde_json::to_string_pretty(&payload).map_err(json_err)
    }

    fn render_chat_thread_ref(&self, target_ref: &str, chat_thread_id: &str) -> io::Result<Value> {
        let thread = self.get_thread(chat_thread_id)?;
        let events = self.read_events(chat_thread_id)?;
        Ok(json!({
            "schema": "supernova_chat_truth_ref_read.v1",
            "ref": target_ref,
            "resolution": "chat_thread",
            "chat_thread": thread,
            "event_count": events.len(),
            "messages": self.messages_from_events(&events),
            "events": render_chat_events(&events),
        }))
    }

    fn render_chat_turn_ref(
        &self,
        target_ref: &str,
        chat_thread_id: Option<&str>,
        turn_id: &str,
    ) -> io::Result<Value> {
        let (thread, events) = if let Some(chat_thread_id) = chat_thread_id {
            let thread = self.get_thread(chat_thread_id)?;
            let events = self
                .read_events(chat_thread_id)?
                .into_iter()
                .filter(|event| event_turn_id(event).as_deref() == Some(turn_id))
                .collect::<Vec<_>>();
            (thread, events)
        } else {
            self.read_events_for_turn(turn_id)?
        };
        if events.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "chat turn ref has no ChatTruth events",
            ));
        }
        Ok(json!({
            "schema": "supernova_chat_truth_ref_read.v1",
            "ref": target_ref,
            "resolution": "chat_turn",
            "chat_thread": thread,
            "turn_id": turn_id,
            "event_count": events.len(),
            "messages": self.messages_from_events(&events),
            "events": render_chat_events(&events),
        }))
    }

    fn read_events_for_turn(&self, turn_id: &str) -> io::Result<(ChatThread, Vec<ChatEvent>)> {
        let conn = self.connect()?;
        let mut stmt = conn
            .prepare(
                r#"
                SELECT event_id, chat_thread_id, container_id, event_seq, event_type,
                       payload_json, blob_ref, created_at_ms
                FROM chat_events
                WHERE payload_json LIKE ?1
                ORDER BY chat_thread_id ASC, event_seq ASC
                "#,
            )
            .map_err(sql_err)?;
        let like_pattern = format!("%{turn_id}%");
        let rows = stmt
            .query_map(params![like_pattern], row_to_event)
            .map_err(sql_err)?;
        let mut events = Vec::new();
        for row in rows {
            let event = row.map_err(sql_err)?;
            if event_turn_id(&event).as_deref() == Some(turn_id) {
                events.push(event);
            }
        }
        let chat_thread_id = events
            .first()
            .map(|event| event.chat_thread_id.clone())
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "chat turn ref not found"))?;
        let thread = self.get_thread(&chat_thread_id)?;
        Ok((thread, events))
    }

    fn messages_from_events(&self, events: &[ChatEvent]) -> Vec<Value> {
        events
            .iter()
            .filter_map(|event| self.message_from_event(event))
            .collect()
    }

    fn message_from_event(&self, event: &ChatEvent) -> Option<Value> {
        let turn_id = event_turn_id(event);
        match event.event_type.as_str() {
            "chat_user_message_recorded" => {
                let content_ref = event.payload.get("message_ref").and_then(Value::as_str);
                Some(json!({
                    "role": "user",
                    "turn_id": turn_id,
                    "event_seq": event.event_seq,
                    "content_ref": content_ref,
                    "content": content_ref.and_then(|value| self.read_chat_blob_text(value).ok()),
                }))
            }
            "chat_assistant_answered" => {
                let content_ref = event
                    .payload
                    .get("assistant_content_ref")
                    .and_then(Value::as_str);
                Some(json!({
                    "role": "assistant",
                    "turn_id": turn_id,
                    "event_seq": event.event_seq,
                    "content_ref": content_ref,
                    "content": content_ref
                        .and_then(|value| self.read_chat_blob_text(value).ok())
                        .or_else(|| event.payload.get("content").and_then(Value::as_str).map(ToString::to_string)),
                }))
            }
            "chat_clarification_requested" => Some(json!({
                "role": "assistant",
                "turn_id": turn_id,
                "event_seq": event.event_seq,
                "content": event.payload.get("question").and_then(Value::as_str),
                "message_kind": "clarification",
            })),
            "chat_needs_task_suggested" => Some(json!({
                "role": "assistant",
                "turn_id": turn_id,
                "event_seq": event.event_seq,
                "content": event.payload.get("suggested_task"),
                "message_kind": "needs_task",
            })),
            _ => None,
        }
    }

    fn connect(&self) -> io::Result<Connection> {
        Connection::open(&self.db_path).map_err(sql_err)
    }

    fn init_schema(&self) -> io::Result<()> {
        let conn = self.connect()?;
        conn.execute_batch(
            r#"
            PRAGMA journal_mode = WAL;
            CREATE TABLE IF NOT EXISTS chat_threads(
              chat_thread_id text primary key,
              container_id text not null,
              title text,
              status text not null,
              created_at_ms integer not null,
              updated_at_ms integer not null
            );
            CREATE TABLE IF NOT EXISTS chat_events(
              event_id text primary key,
              chat_thread_id text not null,
              container_id text not null,
              event_seq integer not null,
              event_type text not null,
              payload_json text not null,
              blob_ref text,
              created_at_ms integer not null
            );
            CREATE TABLE IF NOT EXISTS chat_provider_transcript(
              transcript_id text primary key,
              chat_thread_id text not null,
              provider text not null,
              model text not null,
              messages_ref text not null,
              summary_ref text,
              live_suffix_ref text,
              compacted_until_seq integer,
              updated_at_ms integer not null
            );
            "#,
        )
        .map_err(sql_err)?;
        Ok(())
    }
}

fn row_to_thread(row: &rusqlite::Row<'_>) -> rusqlite::Result<ChatThread> {
    Ok(ChatThread {
        chat_thread_id: row.get(0)?,
        container_id: row.get(1)?,
        title: row.get(2)?,
        status: row.get(3)?,
        created_at_ms: row.get(4)?,
        updated_at_ms: row.get(5)?,
    })
}

fn row_to_event(row: &rusqlite::Row<'_>) -> rusqlite::Result<ChatEvent> {
    let payload_raw: String = row.get(5)?;
    Ok(ChatEvent {
        event_id: row.get(0)?,
        chat_thread_id: row.get(1)?,
        container_id: row.get(2)?,
        event_seq: row.get::<_, i64>(3)? as u64,
        event_type: row.get(4)?,
        payload: serde_json::from_str(&payload_raw).map_err(row_json_err)?,
        blob_ref: row.get(6)?,
        created_at_ms: row.get(7)?,
    })
}

fn row_to_provider_transcript(row: &rusqlite::Row<'_>) -> rusqlite::Result<ChatProviderTranscript> {
    let compacted_until_seq: Option<i64> = row.get(7)?;
    Ok(ChatProviderTranscript {
        transcript_id: row.get(0)?,
        chat_thread_id: row.get(1)?,
        provider: row.get(2)?,
        model: row.get(3)?,
        messages_ref: row.get(4)?,
        summary_ref: row.get(5)?,
        live_suffix_ref: row.get(6)?,
        compacted_until_seq: compacted_until_seq.map(|value| value as u64),
        updated_at_ms: row.get(8)?,
    })
}

fn checked_relative_path(value: &str) -> io::Result<PathBuf> {
    let path = Path::new(value);
    if path.as_os_str().is_empty() || path.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "chat blob path must be a non-empty relative path",
        ));
    }
    let mut clean = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => clean.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::Prefix(_) | Component::RootDir => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "chat blob path must stay inside chat blob root",
                ));
            }
        }
    }
    Ok(clean)
}

enum ChatRefTarget {
    Thread {
        chat_thread_id: String,
    },
    Turn {
        chat_thread_id: Option<String>,
        turn_id: String,
    },
}

fn parse_chat_ref(target_ref: &str) -> io::Result<ChatRefTarget> {
    if let Some(chat_thread_id) = target_ref.strip_prefix("chat://") {
        return non_empty_ref(chat_thread_id, "chat thread ref")
            .map(|chat_thread_id| ChatRefTarget::Thread { chat_thread_id });
    }
    if let Some(chat_thread_id) = target_ref.strip_prefix("chat_thread://") {
        return non_empty_ref(chat_thread_id, "chat thread ref")
            .map(|chat_thread_id| ChatRefTarget::Thread { chat_thread_id });
    }
    if let Some(raw) = target_ref.strip_prefix("chat_turn://") {
        let raw = raw.trim_matches('/');
        if let Some((chat_thread_id, turn_id)) = raw.split_once('/') {
            return Ok(ChatRefTarget::Turn {
                chat_thread_id: Some(non_empty_ref(chat_thread_id, "chat thread ref")?),
                turn_id: non_empty_ref(turn_id, "chat turn ref")?,
            });
        }
        return Ok(ChatRefTarget::Turn {
            chat_thread_id: None,
            turn_id: non_empty_ref(raw, "chat turn ref")?,
        });
    }
    if target_ref.starts_with("chat_turn_") {
        return Ok(ChatRefTarget::Turn {
            chat_thread_id: None,
            turn_id: target_ref.to_string(),
        });
    }
    if target_ref.starts_with("chat_") {
        return Ok(ChatRefTarget::Thread {
            chat_thread_id: target_ref.to_string(),
        });
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidInput,
        "unsupported chat ref scheme",
    ))
}

fn parse_chat_blob_ref(blob_ref: &str) -> io::Result<(&str, &str)> {
    let raw = blob_ref.strip_prefix("chat_blob://").ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "chat blob ref must use chat_blob://",
        )
    })?;
    let (chat_thread_id, relative) = raw.split_once('/').ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "chat blob ref must include thread and path",
        )
    })?;
    if chat_thread_id.is_empty() || relative.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "chat blob ref must include non-empty thread and path",
        ));
    }
    Ok((chat_thread_id, relative))
}

fn non_empty_ref(value: &str, kind: &str) -> io::Result<String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{kind} is empty"),
        ));
    }
    Ok(value.to_string())
}

fn event_turn_id(event: &ChatEvent) -> Option<String> {
    event
        .payload
        .get("turn_id")
        .and_then(Value::as_str)
        .map(ToString::to_string)
}

fn render_chat_events(events: &[ChatEvent]) -> Vec<Value> {
    events
        .iter()
        .map(|event| {
            json!({
                "event_id": event.event_id.clone(),
                "event_seq": event.event_seq,
                "event_type": event.event_type.clone(),
                "payload": event.payload.clone(),
                "blob_ref": event.blob_ref.clone(),
                "created_at_ms": event.created_at_ms,
            })
        })
        .collect()
}

fn next_id(prefix: &str) -> String {
    format!(
        "{}_{}_{}_{}",
        prefix,
        now_ms(),
        std::process::id(),
        CHAT_ID_COUNTER.fetch_add(1, Ordering::Relaxed)
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn temp_workspace(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("supernova_chat_truth_{}_{}", name, now_ms()));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn chat_truth_uses_external_state_root_without_workspace_state() {
        let workspace = temp_workspace("external_state_workspace");
        let state_root =
            std::env::temp_dir().join(format!("supernova_chat_truth_external_state_{}", now_ms()));
        let store = ChatTruthStore::new_with_state_root(&workspace, &state_root).unwrap();
        let state_root = state_root.canonicalize().unwrap();
        let thread = store.create_thread("container_1", None).unwrap();
        let blob_ref = store
            .write_chat_blob(&thread.chat_thread_id, "turns/user.txt", b"hello")
            .unwrap();
        store
            .append_event(
                &thread.chat_thread_id,
                &thread.container_id,
                "chat_user_message_recorded",
                json!({"blob_ref": blob_ref}),
                None,
            )
            .unwrap();

        assert!(store.path().starts_with(state_root.join("state")));
        assert!(state_root.join("state").join("chat_truth.sqlite3").exists());
        assert!(state_root
            .join("blobs")
            .join("chat")
            .join(&thread.chat_thread_id)
            .join("turns")
            .join("user.txt")
            .exists());
        assert_eq!(store.read_events(&thread.chat_thread_id).unwrap().len(), 1);
        assert!(!workspace.join(crate::RUNTIME_DIR_NAME).exists());
    }

    #[test]
    fn chat_truth_appends_and_replays_without_process_truth() {
        let workspace = temp_workspace("basic");
        let store = ChatTruthStore::new(&workspace).unwrap();
        let thread = store
            .create_thread("container_1", Some("Research chat".to_string()))
            .unwrap();
        let blob_ref = store
            .write_chat_blob(&thread.chat_thread_id, "turns/user.txt", b"hello")
            .unwrap();
        store
            .append_event(
                &thread.chat_thread_id,
                &thread.container_id,
                "chat_turn_started",
                json!({"message_ref": blob_ref}),
                None,
            )
            .unwrap();
        store
            .append_event(
                &thread.chat_thread_id,
                &thread.container_id,
                "chat_assistant_answered",
                json!({"content": "answer"}),
                None,
            )
            .unwrap();

        let events = store.read_events(&thread.chat_thread_id).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].event_seq, 1);
        assert_eq!(events[1].event_seq, 2);
        assert!(!(workspace.join(".supernova_v2").join("process_truth")).exists());
    }
}
