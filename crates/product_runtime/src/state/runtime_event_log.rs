use local_runtime_protocol::Cursor;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;

use crate::state::message_feed::normalize_message_page_limit;
use crate::state::product_db::ProductDb;
use crate::state::workspace_registry::now_ms;

#[derive(Clone, Debug, PartialEq)]
pub struct RuntimeEventRecord {
    pub event_id: String,
    pub workspace_uid: String,
    pub partition_key: String,
    pub container_id: Option<String>,
    pub run_id: Option<String>,
    pub task_id: Option<String>,
    pub chat_thread_id: Option<String>,
    pub event_type: String,
    pub payload: Value,
    pub created_at_ms: u128,
    pub cursor_seq: i64,
    pub projection_status: String,
}

impl ProductDb {
    pub fn append_runtime_event(
        &self,
        event_id: &str,
        partition_key: &str,
        container_id: Option<&str>,
        run_id: Option<&str>,
        task_id: Option<&str>,
        chat_thread_id: Option<&str>,
        event_type: &str,
        payload: Value,
    ) -> rusqlite::Result<RuntimeEventRecord> {
        let mut conn = self.connect()?;
        let tx = conn.transaction()?;
        if let Some(existing) = get_runtime_event_tx(&tx, event_id)? {
            tx.commit()?;
            return Ok(existing);
        }
        let cursor_seq: i64 = tx.query_row(
            "SELECT COALESCE(MAX(cursor_seq), 0) + 1 FROM runtime_event_log",
            [],
            |row| row.get(0),
        )?;
        let created_at_ms = now_ms();
        tx.execute(
            "INSERT INTO runtime_event_log(
              event_id, workspace_uid, partition_key, container_id, run_id, task_id,
              chat_thread_id, event_type, payload_json, created_at_ms, cursor_seq, projection_status
            ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 'pending')",
            params![
                event_id,
                &self.workspace_uid,
                partition_key,
                container_id,
                run_id,
                task_id,
                chat_thread_id,
                event_type,
                serde_json::to_string(&payload).unwrap_or_else(|_| "{}".into()),
                created_at_ms,
                cursor_seq,
            ],
        )?;
        let inserted = get_runtime_event_tx(&tx, event_id)?
            .ok_or_else(|| rusqlite::Error::QueryReturnedNoRows)?;
        tx.commit()?;
        Ok(inserted)
    }

    pub fn list_runtime_events_page(
        &self,
        after_event_id: Option<i64>,
        limit: Option<usize>,
    ) -> rusqlite::Result<Vec<RuntimeEventRecord>> {
        let bounded_limit = normalize_message_page_limit(limit);
        let conn = self.connect()?;
        if let Some(after_event_id) = after_event_id {
            let mut stmt = conn.prepare(
                "SELECT event_id, workspace_uid, partition_key, container_id, run_id, task_id,
                  chat_thread_id, event_type, payload_json, created_at_ms, cursor_seq, projection_status
                 FROM runtime_event_log
                 WHERE cursor_seq > ?1
                 ORDER BY cursor_seq ASC
                 LIMIT ?2",
            )?;
            let rows = stmt.query_map(
                params![after_event_id, bounded_limit as i64],
                row_to_runtime_event,
            )?;
            return rows.collect();
        }

        let mut stmt = conn.prepare(
            "SELECT event_id, workspace_uid, partition_key, container_id, run_id, task_id,
              chat_thread_id, event_type, payload_json, created_at_ms, cursor_seq, projection_status
             FROM (
               SELECT event_id, workspace_uid, partition_key, container_id, run_id, task_id,
                 chat_thread_id, event_type, payload_json, created_at_ms, cursor_seq, projection_status
               FROM runtime_event_log
               ORDER BY cursor_seq DESC
               LIMIT ?1
             )
             ORDER BY cursor_seq ASC",
        )?;
        let rows = stmt.query_map(params![bounded_limit as i64], row_to_runtime_event)?;
        rows.collect()
    }
}

pub fn cursor_for_runtime_events(events: &[RuntimeEventRecord]) -> Option<Cursor> {
    events.last().map(|event| Cursor {
        kind: "runtime_event_log".into(),
        after: Some(event.cursor_seq.to_string()),
        after_event_id: Some(event.cursor_seq),
    })
}

fn get_runtime_event_tx(
    conn: &Connection,
    event_id: &str,
) -> rusqlite::Result<Option<RuntimeEventRecord>> {
    conn.query_row(
        "SELECT event_id, workspace_uid, partition_key, container_id, run_id, task_id,
          chat_thread_id, event_type, payload_json, created_at_ms, cursor_seq, projection_status
         FROM runtime_event_log
         WHERE event_id=?1",
        params![event_id],
        row_to_runtime_event,
    )
    .optional()
}

fn row_to_runtime_event(row: &rusqlite::Row<'_>) -> rusqlite::Result<RuntimeEventRecord> {
    Ok(RuntimeEventRecord {
        event_id: row.get(0)?,
        workspace_uid: row.get(1)?,
        partition_key: row.get(2)?,
        container_id: row.get(3)?,
        run_id: row.get(4)?,
        task_id: row.get(5)?,
        chat_thread_id: row.get(6)?,
        event_type: row.get(7)?,
        payload: serde_json::from_str(&row.get::<_, String>(8)?)
            .unwrap_or(Value::Object(Default::default())),
        created_at_ms: row.get::<_, i64>(9)? as u128,
        cursor_seq: row.get(10)?,
        projection_status: row.get(11)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use serde_json::json;
    use std::path::PathBuf;

    fn temp_root(name: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("supernova_runtime_event_log_{name}_{}", now_ms()));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn test_db(name: &str) -> ProductDb {
        ProductDb::open(&temp_root(name), format!("workspace_{name}")).unwrap()
    }

    #[test]
    fn runtime_event_log_cursor_is_monotonic_and_incremental() {
        let db = test_db("cursor_incremental");
        let first = db
            .append_runtime_event(
                "event_1",
                "workspace/container_a",
                Some("container_a"),
                Some("run_1"),
                None,
                Some("chat_1"),
                "run.started",
                json!({"status": "running"}),
            )
            .unwrap();
        let second = db
            .append_runtime_event(
                "event_2",
                "workspace/container_a",
                Some("container_a"),
                Some("run_1"),
                None,
                Some("chat_1"),
                "run.completed",
                json!({"status": "completed"}),
            )
            .unwrap();

        assert!(second.cursor_seq > first.cursor_seq);
        let page = db
            .list_runtime_events_page(Some(first.cursor_seq), Some(10))
            .unwrap();
        assert_eq!(page.len(), 1);
        assert_eq!(page[0].event_id, "event_2");
        assert_eq!(page[0].payload["status"], "completed");
    }

    #[test]
    fn runtime_event_log_append_is_idempotent_by_event_id() {
        let db = test_db("idempotent");
        let first = db
            .append_runtime_event(
                "event_1",
                "workspace/container_a",
                Some("container_a"),
                None,
                None,
                None,
                "run.started",
                json!({"status": "running"}),
            )
            .unwrap();
        let replayed = db
            .append_runtime_event(
                "event_1",
                "workspace/container_a",
                Some("container_a"),
                None,
                None,
                None,
                "run.started",
                json!({"status": "running"}),
            )
            .unwrap();

        assert_eq!(first.cursor_seq, replayed.cursor_seq);
        assert_eq!(
            db.list_runtime_events_page(None, Some(10)).unwrap().len(),
            1
        );
    }
}
