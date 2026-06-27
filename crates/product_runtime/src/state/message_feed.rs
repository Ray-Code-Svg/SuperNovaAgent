use std::sync::atomic::{AtomicU64, Ordering};

use local_runtime_protocol::{ContainerMessage, Cursor, MessageLane, MessageRole, MessageType};
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};

use crate::state::product_db::ProductDb;
use crate::state::workspace_registry::now_ms;

static MESSAGE_COUNTER: AtomicU64 = AtomicU64::new(1);
const DEFAULT_MESSAGE_PAGE_LIMIT: usize = 1_000;
const MAX_MESSAGE_PAGE_LIMIT: usize = 1_000;
const MESSAGE_SELECT_COLUMNS: &str = "message_id, workspace_uid, container_id, lane, role, message_type, status, title, body_text, body_json, card_json, chat_thread_id, task_id, job_id, source_kind, source_ref, source_seq, created_at_ms, updated_at_ms, sort_key";
const MESSAGE_CURSOR_SQL: &str = "MAX((CAST(substr(sort_key, 1, 20) AS INTEGER) * 1000000) + CAST(substr(sort_key, 22, 10) AS INTEGER), (updated_at_ms * 1000000) + MAX(0, MIN(CAST(COALESCE(json_extract(body_json, '$._feed_cursor_seq'), source_seq, 0) AS INTEGER), 999999)))";

impl ProductDb {
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

    pub fn list_container_messages(
        &self,
        container_id: &str,
    ) -> rusqlite::Result<Vec<ContainerMessage>> {
        let conn = self.connect()?;
        let mut stmt = conn.prepare(
            "SELECT message_id, workspace_uid, container_id, lane, role, message_type, status, title, body_text, body_json, card_json, chat_thread_id, task_id, job_id, source_kind, source_ref, source_seq, created_at_ms, updated_at_ms, sort_key FROM messages WHERE container_id=?1 ORDER BY sort_key ASC",
        )?;
        let rows = stmt.query_map(params![container_id], row_to_message_public)?;
        rows.collect()
    }

    pub fn list_chat_messages(
        &self,
        chat_thread_id: &str,
    ) -> rusqlite::Result<Vec<ContainerMessage>> {
        let conn = self.connect()?;
        let mut stmt = conn.prepare(
            "SELECT message_id, workspace_uid, container_id, lane, role, message_type, status, title, body_text, body_json, card_json, chat_thread_id, task_id, job_id, source_kind, source_ref, source_seq, created_at_ms, updated_at_ms, sort_key FROM messages WHERE chat_thread_id=?1 ORDER BY sort_key ASC",
        )?;
        let rows = stmt.query_map(params![chat_thread_id], row_to_message_public)?;
        rows.collect()
    }

    pub fn list_container_messages_page(
        &self,
        container_id: &str,
        after_event_id: Option<i64>,
        limit: Option<usize>,
    ) -> rusqlite::Result<Vec<ContainerMessage>> {
        self.list_messages_page("container_id", container_id, None, after_event_id, limit)
    }

    pub fn list_container_messages_page_for_lane(
        &self,
        container_id: &str,
        lane: &MessageLane,
        after_event_id: Option<i64>,
        limit: Option<usize>,
    ) -> rusqlite::Result<Vec<ContainerMessage>> {
        self.list_messages_page(
            "container_id",
            container_id,
            Some(lane),
            after_event_id,
            limit,
        )
    }

    pub fn list_chat_messages_page(
        &self,
        chat_thread_id: &str,
        after_event_id: Option<i64>,
        limit: Option<usize>,
    ) -> rusqlite::Result<Vec<ContainerMessage>> {
        self.list_messages_page(
            "chat_thread_id",
            chat_thread_id,
            None,
            after_event_id,
            limit,
        )
    }

    pub fn list_task_messages_page(
        &self,
        task_id: &str,
        after_event_id: Option<i64>,
        limit: Option<usize>,
    ) -> rusqlite::Result<Vec<ContainerMessage>> {
        self.list_messages_page("task_id", task_id, None, after_event_id, limit)
    }

    pub fn list_runtime_messages_page(
        &self,
        after_event_id: Option<i64>,
        limit: Option<usize>,
    ) -> rusqlite::Result<Vec<ContainerMessage>> {
        let bounded_limit = normalize_message_page_limit(limit);
        let conn = self.connect()?;
        if let Some(after_event_id) = after_event_id {
            let sql = format!(
                "SELECT {MESSAGE_SELECT_COLUMNS} FROM messages WHERE {MESSAGE_CURSOR_SQL} > ?1 ORDER BY sort_key ASC LIMIT ?2"
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(
                params![after_event_id, bounded_limit as i64],
                row_to_message_public,
            )?;
            return rows.collect();
        }

        let sql = format!(
            "SELECT {MESSAGE_SELECT_COLUMNS} FROM (SELECT {MESSAGE_SELECT_COLUMNS} FROM messages ORDER BY sort_key DESC LIMIT ?1) ORDER BY sort_key ASC"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params![bounded_limit as i64], row_to_message_public)?;
        rows.collect()
    }

    pub fn downgrade_database_locked_task_errors(
        &self,
        container_id: &str,
    ) -> rusqlite::Result<usize> {
        self.downgrade_database_locked_kernel_errors(container_id, "task", "task_start")
    }

    pub fn downgrade_database_locked_chat_errors(
        &self,
        container_id: &str,
    ) -> rusqlite::Result<usize> {
        self.downgrade_database_locked_kernel_errors(container_id, "chat", "chat_turn")
    }

    fn downgrade_database_locked_kernel_errors(
        &self,
        container_id: &str,
        lane: &str,
        source_ref: &str,
    ) -> rusqlite::Result<usize> {
        let now = now_ms() as u128;
        let seq = MESSAGE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let body_json = json!({
            "schema": "supernova_projection_repair.v1",
            "status": "projection_error_recovered",
            "original_error_class": "database_is_locked",
            "truth_source": if lane == "task" { "process_truth" } else { "chat_truth" },
            "source_kind": "kernel_bridge",
            "source_ref": source_ref,
            "recovered_at_ms": now,
            "_feed_cursor_seq": seq as i64,
        });
        let conn = self.connect()?;
        conn.execute(
            "UPDATE messages
             SET message_type='phase',
                 status='completed',
                 title='Projection error recovered',
                 body_text='Recovered local projection write failure from Kernel truth.',
                 body_json=?4,
                 updated_at_ms=?5
             WHERE container_id=?1
               AND lane=?2
               AND source_kind='kernel_bridge'
               AND source_ref=?3
               AND message_type='error'
               AND lower(COALESCE(body_text, '')) LIKE '%database is locked%'",
            params![
                container_id,
                lane,
                source_ref,
                serde_json::to_string(&body_json).unwrap_or_else(|_| "{}".into()),
                now as i64,
            ],
        )
    }

    fn list_messages_page(
        &self,
        scope_column: &str,
        scope_value: &str,
        lane: Option<&MessageLane>,
        after_event_id: Option<i64>,
        limit: Option<usize>,
    ) -> rusqlite::Result<Vec<ContainerMessage>> {
        let scope_column = match scope_column {
            "container_id" => "container_id",
            "chat_thread_id" => "chat_thread_id",
            "task_id" => "task_id",
            _ => {
                return Err(rusqlite::Error::InvalidParameterName(format!(
                    "unsupported message page scope: {scope_column}"
                )))
            }
        };
        let bounded_limit = normalize_message_page_limit(limit);
        let conn = self.connect()?;
        if let Some(after_event_id) = after_event_id {
            if let Some(lane) = lane {
                let sql = format!(
                    "SELECT {MESSAGE_SELECT_COLUMNS} FROM messages WHERE {scope_column}=?1 AND lane=?2 AND {MESSAGE_CURSOR_SQL} > ?3 ORDER BY sort_key ASC LIMIT ?4"
                );
                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt.query_map(
                    params![
                        scope_value,
                        lane_str(lane),
                        after_event_id,
                        bounded_limit as i64,
                    ],
                    row_to_message_public,
                )?;
                return rows.collect();
            }
            let sql = format!(
                "SELECT {MESSAGE_SELECT_COLUMNS} FROM messages WHERE {scope_column}=?1 AND {MESSAGE_CURSOR_SQL} > ?2 ORDER BY sort_key ASC LIMIT ?3"
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(
                params![scope_value, after_event_id, bounded_limit as i64],
                row_to_message_public,
            )?;
            return rows.collect();
        }

        if let Some(lane) = lane {
            let sql = format!(
                "SELECT {MESSAGE_SELECT_COLUMNS} FROM (SELECT {MESSAGE_SELECT_COLUMNS} FROM messages WHERE {scope_column}=?1 AND lane=?2 ORDER BY sort_key DESC LIMIT ?3) ORDER BY sort_key ASC"
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(
                params![scope_value, lane_str(lane), bounded_limit as i64],
                row_to_message_public,
            )?;
            return rows.collect();
        }

        let sql = format!(
            "SELECT {MESSAGE_SELECT_COLUMNS} FROM (SELECT {MESSAGE_SELECT_COLUMNS} FROM messages WHERE {scope_column}=?1 ORDER BY sort_key DESC LIMIT ?2) ORDER BY sort_key ASC"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(
            params![scope_value, bounded_limit as i64],
            row_to_message_public,
        )?;
        rows.collect()
    }
}

pub fn filter_messages_after(
    messages: Vec<ContainerMessage>,
    after_event_id: Option<i64>,
    limit: Option<usize>,
) -> Vec<ContainerMessage> {
    let bounded_limit = normalize_message_page_limit(limit);
    let filtered = messages
        .into_iter()
        .filter(|message| {
            after_event_id
                .map(|after| message_cursor_event_id(message) > after)
                .unwrap_or(true)
        })
        .collect::<Vec<_>>();
    if after_event_id.is_some() || filtered.len() <= bounded_limit {
        return filtered.into_iter().take(bounded_limit).collect();
    }
    filtered[filtered.len() - bounded_limit..].to_vec()
}

pub fn normalize_message_page_limit(limit: Option<usize>) -> usize {
    limit
        .unwrap_or(DEFAULT_MESSAGE_PAGE_LIMIT)
        .min(MAX_MESSAGE_PAGE_LIMIT)
}

pub fn page_cursor_for_messages(
    kind: impl Into<String>,
    messages: &[ContainerMessage],
) -> Option<Cursor> {
    messages.last().map(|message| Cursor {
        kind: kind.into(),
        after: Some(message.sort_key.clone()),
        after_event_id: Some(message_cursor_event_id(message)),
    })
}

pub fn message_cursor_event_id(message: &ContainerMessage) -> i64 {
    let sort_cursor = sort_key_to_cursor(&message.sort_key);
    let updated_cursor = {
        let updated_at = i64::try_from(message.updated_at_ms).unwrap_or(i64::MAX / 1_000_000);
        let seq = feed_cursor_seq(message)
            .or(message.source_seq)
            .unwrap_or(0)
            .clamp(0, 999_999);
        Some(updated_at.saturating_mul(1_000_000).saturating_add(seq))
    };
    sort_cursor
        .into_iter()
        .chain(updated_cursor)
        .max()
        .unwrap_or_else(|| {
            let created_at = i64::try_from(message.created_at_ms).unwrap_or(i64::MAX / 1_000_000);
            created_at.saturating_mul(1_000_000)
        })
}

pub fn sort_messages_for_display_page(messages: &mut [ContainerMessage]) {
    messages.sort_by(|left, right| {
        left.sort_key
            .cmp(&right.sort_key)
            .then_with(|| message_cursor_event_id(left).cmp(&message_cursor_event_id(right)))
            .then_with(|| left.message_id.cmp(&right.message_id))
    });
}

pub fn advance_message_cursor(message: &mut ContainerMessage) {
    let now = now_ms() as u128;
    let seq = MESSAGE_COUNTER.fetch_add(1, Ordering::Relaxed);
    message.updated_at_ms = now;
    if let Some(body) = message.body_json.as_object_mut() {
        body.insert("_feed_cursor_seq".into(), Value::from(seq as i64));
    }
}

fn feed_cursor_seq(message: &ContainerMessage) -> Option<i64> {
    message
        .body_json
        .get("_feed_cursor_seq")
        .and_then(Value::as_i64)
}

fn sort_key_to_cursor(sort_key: &str) -> Option<i64> {
    let mut parts = sort_key.split('_');
    let millis = parts.next()?.parse::<i64>().ok()?;
    let seq = parts
        .next()
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(0)
        .clamp(0, 999_999);
    Some(millis.saturating_mul(1_000_000).saturating_add(seq))
}

pub(crate) fn row_to_message_public(row: &rusqlite::Row<'_>) -> rusqlite::Result<ContainerMessage> {
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

pub fn new_message(
    workspace_uid: &str,
    container_id: &str,
    lane: MessageLane,
    role: MessageRole,
    message_type: MessageType,
    body_text: Option<String>,
    chat_thread_id: Option<String>,
) -> ContainerMessage {
    let now = now_ms() as u128;
    let seq = MESSAGE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let id = format!("msg_{now}_{seq}");
    ContainerMessage {
        message_id: id.clone(),
        workspace_uid: workspace_uid.to_string(),
        container_id: container_id.to_string(),
        lane,
        role,
        message_type,
        status: "completed".into(),
        title: None,
        body_text,
        body_json: Value::Object(Default::default()),
        card_json: Value::Object(Default::default()),
        chat_thread_id,
        task_id: None,
        job_id: None,
        source_kind: "product_runtime".into(),
        source_ref: id,
        source_seq: None,
        created_at_ms: now,
        updated_at_ms: now,
        sort_key: format!("{now:020}_{seq:010}"),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::PathBuf;

    fn temp_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("supernova_message_feed_{name}_{}", now_ms()));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn test_db(name: &str) -> ProductDb {
        ProductDb::open(&temp_root(name), format!("workspace_{name}")).unwrap()
    }

    #[test]
    fn message_cursor_uses_sort_key_millis_and_sequence() {
        let mut message = new_message(
            "ws",
            "container",
            MessageLane::Chat,
            MessageRole::User,
            MessageType::Text,
            Some("hello".into()),
            Some("chat".into()),
        );
        message.sort_key = "00000000000000001234_0000000042".into();
        message.updated_at_ms = 1;
        assert_eq!(message_cursor_event_id(&message), 1_234_000_042);
    }

    #[test]
    fn list_container_messages_page_without_cursor_reads_latest_window_in_display_order() {
        let db = test_db("container_latest_page");
        for index in 0..5 {
            let mut message = new_message(
                &db.workspace_uid,
                "container",
                MessageLane::Task,
                MessageRole::Agent,
                MessageType::Phase,
                Some(format!("phase {index}")),
                None,
            );
            message.sort_key = format!("{:020}_{:010}", 10 + index, index);
            message.created_at_ms = (10 + index) as u128;
            message.updated_at_ms = (10 + index) as u128;
            db.append_message(message).unwrap();
        }

        let page = db
            .list_container_messages_page("container", None, Some(2))
            .unwrap();

        assert_eq!(page.len(), 2);
        assert_eq!(page[0].body_text.as_deref(), Some("phase 3"));
        assert_eq!(page[1].body_text.as_deref(), Some("phase 4"));
    }

    #[test]
    fn list_container_messages_page_with_cursor_reads_only_incremental_window() {
        let db = test_db("container_cursor_page");
        let mut inserted = Vec::new();
        for index in 0..5 {
            let mut message = new_message(
                &db.workspace_uid,
                "container",
                MessageLane::Task,
                MessageRole::Agent,
                MessageType::Phase,
                Some(format!("phase {index}")),
                None,
            );
            message.sort_key = format!("{:020}_{:010}", 10 + index, index);
            message.created_at_ms = (10 + index) as u128;
            message.updated_at_ms = (10 + index) as u128;
            inserted.push(db.append_message(message).unwrap());
        }
        let after = message_cursor_event_id(&inserted[1]);

        let page = db
            .list_container_messages_page("container", Some(after), Some(2))
            .unwrap();

        assert_eq!(page.len(), 2);
        assert_eq!(page[0].body_text.as_deref(), Some("phase 2"));
        assert_eq!(page[1].body_text.as_deref(), Some("phase 3"));
    }

    #[test]
    fn list_messages_page_after_cursor_returns_stream_replacement_updates() {
        let db = test_db("stream_replacement_page");
        let mut first = new_message(
            &db.workspace_uid,
            "container",
            MessageLane::Chat,
            MessageRole::Assistant,
            MessageType::Text,
            Some("Hel".into()),
            Some("chat".into()),
        );
        first.message_id = "stream_message".into();
        first.sort_key = "00000000000000000010_0000000001".into();
        first.created_at_ms = 10;
        first.updated_at_ms = 10;
        first.source_kind = "model_stream".into();
        first.source_ref = "call_1".into();
        let _ = db.append_message(first.clone()).unwrap();

        let mut second = new_message(
            &db.workspace_uid,
            "container",
            MessageLane::Chat,
            MessageRole::User,
            MessageType::Text,
            Some("later".into()),
            Some("chat".into()),
        );
        second.sort_key = "00000000000000000020_0000000001".into();
        second.created_at_ms = 20;
        second.updated_at_ms = 20;
        let second = db.append_message(second).unwrap();
        let after = message_cursor_event_id(&second);

        first.body_text = Some("Hello".into());
        advance_message_cursor(&mut first);
        let _ = db.append_message(first).unwrap();

        let page = db
            .list_chat_messages_page("chat", Some(after), Some(10))
            .unwrap();

        assert_eq!(page.len(), 1);
        assert_eq!(page[0].message_id, "stream_message");
        assert_eq!(page[0].body_text.as_deref(), Some("Hello"));
    }

    #[test]
    fn list_runtime_messages_page_reads_global_incremental_window_without_container_scan() {
        let db = test_db("runtime_page");
        let mut inserted = Vec::new();
        for (index, container_id) in ["container_a", "container_b", "container_a"]
            .into_iter()
            .enumerate()
        {
            let mut message = new_message(
                &db.workspace_uid,
                container_id,
                MessageLane::Chat,
                MessageRole::Assistant,
                MessageType::Text,
                Some(format!("message {index}")),
                Some(format!("chat_{index}")),
            );
            message.sort_key = format!("{:020}_{:010}", 10 + index as i64, index);
            message.created_at_ms = (10 + index) as u128;
            message.updated_at_ms = (10 + index) as u128;
            inserted.push(db.append_message(message).unwrap());
        }
        let after = message_cursor_event_id(&inserted[0]);

        let page = db
            .list_runtime_messages_page(Some(after), Some(10))
            .unwrap();

        assert_eq!(page.len(), 2);
        assert_eq!(page[0].container_id, "container_b");
        assert_eq!(page[1].container_id, "container_a");
    }

    #[test]
    fn advance_message_cursor_does_not_change_display_sort_key() {
        let mut message = new_message(
            "ws",
            "container",
            MessageLane::Task,
            MessageRole::Agent,
            MessageType::Reasoning,
            Some("streaming".into()),
            None,
        );
        message.sort_key = "00000000000000001000_0000000001".into();
        let sort_key = message.sort_key.clone();
        let before = message_cursor_event_id(&message);
        advance_message_cursor(&mut message);
        assert_eq!(message.sort_key, sort_key);
        assert!(
            message_cursor_event_id(&message) > before,
            "completed stream replacement must advance the SSE cursor without moving display order"
        );
    }

    #[test]
    fn filter_messages_after_applies_cursor_and_limit() {
        let messages = (0..4)
            .map(|index| {
                let mut message = new_message(
                    "ws",
                    "container",
                    MessageLane::Task,
                    MessageRole::Agent,
                    MessageType::Phase,
                    Some(format!("phase {index}")),
                    None,
                );
                message.sort_key = format!("{:020}_{:010}", 10 + index, index);
                message.updated_at_ms = 10 + index;
                message
            })
            .collect::<Vec<_>>();
        let after = message_cursor_event_id(&messages[1]);
        let filtered = filter_messages_after(messages, Some(after), Some(1));
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].body_text.as_deref(), Some("phase 2"));
    }

    #[test]
    fn filter_messages_after_without_cursor_returns_latest_window_in_display_order() {
        let messages = messages_with_sort_keys(0..5);

        let filtered = filter_messages_after(messages, None, Some(2));

        assert_eq!(filtered.len(), 2);
        assert_eq!(filtered[0].body_text.as_deref(), Some("phase 3"));
        assert_eq!(filtered[1].body_text.as_deref(), Some("phase 4"));
    }

    #[test]
    fn filter_messages_after_without_cursor_keeps_all_messages_under_limit() {
        let messages = messages_with_sort_keys(0..3);

        let filtered = filter_messages_after(messages, None, Some(5));

        assert_eq!(filtered.len(), 3);
        assert_eq!(filtered[0].body_text.as_deref(), Some("phase 0"));
        assert_eq!(filtered[2].body_text.as_deref(), Some("phase 2"));
    }

    #[test]
    fn filter_messages_after_with_cursor_keeps_incremental_window_from_cursor() {
        let messages = messages_with_sort_keys(0..5);
        let after = message_cursor_event_id(&messages[1]);

        let filtered = filter_messages_after(messages, Some(after), Some(2));

        assert_eq!(filtered.len(), 2);
        assert_eq!(filtered[0].body_text.as_deref(), Some("phase 2"));
        assert_eq!(filtered[1].body_text.as_deref(), Some("phase 3"));
    }

    fn messages_with_sort_keys(range: std::ops::Range<i64>) -> Vec<ContainerMessage> {
        range
            .map(|index| {
                let mut message = new_message(
                    "ws",
                    "container",
                    MessageLane::Task,
                    MessageRole::Agent,
                    MessageType::Phase,
                    Some(format!("phase {index}")),
                    None,
                );
                message.sort_key = format!("{:020}_{:010}", 10 + index, index);
                message.created_at_ms = (10 + index) as u128;
                message.updated_at_ms = (10 + index) as u128;
                message
            })
            .collect()
    }
}
