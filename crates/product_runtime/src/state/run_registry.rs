use std::sync::atomic::{AtomicU64, Ordering};

use local_runtime_protocol::RunRecord;
use rusqlite::{params, OptionalExtension};

use crate::state::product_db::ProductDb;
use crate::state::workspace_registry::now_ms;

static RUN_COUNTER: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewRun {
    pub container_id: String,
    pub run_kind: String,
    pub chat_thread_id: Option<String>,
    pub task_id: Option<String>,
    pub job_id: Option<String>,
    pub worker_id: String,
}

impl ProductDb {
    pub fn insert_run(&self, run: NewRun) -> rusqlite::Result<RunRecord> {
        let run_id = next_run_id();
        let now = now_ms() as u128;
        let conn = self.connect()?;
        conn.execute(
            "INSERT INTO run_registry(
              run_id, workspace_uid, container_id, run_kind, chat_thread_id, task_id,
              job_id, worker_id, status, cancel_requested, heartbeat_at_ms,
              started_at_ms, updated_at_ms, error_message
            ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'queued', 0, NULL, ?9, ?9, NULL)",
            params![
                &run_id,
                &self.workspace_uid,
                &run.container_id,
                &run.run_kind,
                &run.chat_thread_id,
                &run.task_id,
                &run.job_id,
                &run.worker_id,
                now as i64,
            ],
        )?;
        self.get_run(&run_id)
    }

    pub fn get_run(&self, run_id: &str) -> rusqlite::Result<RunRecord> {
        let conn = self.connect()?;
        conn.query_row(
            run_select_sql("WHERE run_id=?1").as_str(),
            params![run_id],
            row_to_run,
        )
    }

    pub fn list_runs(&self, container_id: Option<&str>) -> rusqlite::Result<Vec<RunRecord>> {
        let conn = self.connect()?;
        if let Some(container_id) = container_id {
            let mut stmt = conn.prepare(
                run_select_sql("WHERE container_id=?1 ORDER BY updated_at_ms DESC, run_id DESC")
                    .as_str(),
            )?;
            let rows = stmt.query_map(params![container_id], row_to_run)?;
            return rows.collect();
        }
        let mut stmt =
            conn.prepare(run_select_sql("ORDER BY updated_at_ms DESC, run_id DESC").as_str())?;
        let rows = stmt.query_map([], row_to_run)?;
        rows.collect()
    }

    pub fn update_run_status(
        &self,
        run_id: &str,
        status: &str,
        error_message: Option<&str>,
    ) -> rusqlite::Result<RunRecord> {
        let now = now_ms() as u128;
        let conn = self.connect()?;
        conn.execute(
            "UPDATE run_registry
             SET status=?2, updated_at_ms=?3, error_message=?4
             WHERE run_id=?1",
            params![run_id, status, now as i64, error_message],
        )?;
        self.get_run(run_id)
    }

    pub fn update_run_worker(&self, run_id: &str, worker_id: &str) -> rusqlite::Result<RunRecord> {
        let now = now_ms() as u128;
        let conn = self.connect()?;
        conn.execute(
            "UPDATE run_registry
             SET worker_id=?2, updated_at_ms=?3
             WHERE run_id=?1",
            params![run_id, worker_id, now as i64],
        )?;
        self.get_run(run_id)
    }

    pub fn heartbeat_run(&self, run_id: &str) -> rusqlite::Result<Option<RunRecord>> {
        let now = now_ms() as u128;
        let conn = self.connect()?;
        let changed = conn.execute(
            "UPDATE run_registry
             SET heartbeat_at_ms=?2, updated_at_ms=?2
             WHERE run_id=?1",
            params![run_id, now as i64],
        )?;
        if changed == 0 {
            return Ok(None);
        }
        self.get_run(run_id).map(Some)
    }

    pub fn bind_run_task(
        &self,
        run_id: &str,
        task_id: &str,
        job_id: &str,
    ) -> rusqlite::Result<RunRecord> {
        let now = now_ms() as u128;
        let conn = self.connect()?;
        conn.execute(
            "UPDATE run_registry
             SET task_id=?2, job_id=?3, updated_at_ms=?4
             WHERE run_id=?1",
            params![run_id, task_id, job_id, now as i64],
        )?;
        self.get_run(run_id)
    }

    pub fn request_cancel_run(&self, run_id: &str) -> rusqlite::Result<Option<RunRecord>> {
        let now = now_ms() as u128;
        let conn = self.connect()?;
        let changed = conn.execute(
            "UPDATE run_registry
             SET cancel_requested=1, status='cancel_requested', updated_at_ms=?2
             WHERE run_id=?1 AND status NOT IN ('completed', 'failed', 'blocked', 'cancelled')",
            params![run_id, now as i64],
        )?;
        if changed == 0 {
            return Ok(conn
                .query_row(
                    run_select_sql("WHERE run_id=?1").as_str(),
                    params![run_id],
                    row_to_run,
                )
                .optional()?);
        }
        self.get_run(run_id).map(Some)
    }

    pub fn request_cancel_task_runs(&self, task_id: &str) -> rusqlite::Result<Vec<RunRecord>> {
        self.request_cancel_runs_by("task_id", task_id)
    }

    pub fn request_cancel_chat_runs(
        &self,
        chat_thread_id: &str,
    ) -> rusqlite::Result<Vec<RunRecord>> {
        self.request_cancel_runs_by("chat_thread_id", chat_thread_id)
    }

    fn request_cancel_runs_by(
        &self,
        column: &str,
        value: &str,
    ) -> rusqlite::Result<Vec<RunRecord>> {
        let now = now_ms() as u128;
        let sql = format!(
            "UPDATE run_registry
             SET cancel_requested=1, status='cancel_requested', updated_at_ms=?2
             WHERE {column}=?1 AND status NOT IN ('completed', 'failed', 'blocked', 'cancelled')"
        );
        let conn = self.connect()?;
        conn.execute(sql.as_str(), params![value, now as i64])?;
        let select_sql = format!(
            "{} WHERE {column}=?1 ORDER BY updated_at_ms DESC, run_id DESC",
            RUN_SELECT_COLUMNS
        );
        let mut stmt = conn.prepare(&select_sql)?;
        let rows = stmt.query_map(params![value], row_to_run)?;
        rows.collect()
    }

    pub fn repair_database_locked_task_runs(
        &self,
        task_id: &str,
        job_id: &str,
        terminal_status: &str,
        terminal_error_message: Option<&str>,
    ) -> rusqlite::Result<usize> {
        let now = now_ms() as u128;
        let conn = self.connect()?;
        conn.execute(
            "UPDATE run_registry
             SET status=?3, cancel_requested=0, updated_at_ms=?4, error_message=?5
             WHERE (task_id=?1 OR job_id=?2)
               AND status='failed'
               AND lower(COALESCE(error_message, '')) LIKE '%database is locked%'",
            params![
                task_id,
                job_id,
                terminal_status,
                now as i64,
                terminal_error_message,
            ],
        )
    }

    pub fn repair_stale_active_task_runs(
        &self,
        task_id: &str,
        job_id: &str,
        terminal_status: &str,
        terminal_error_message: Option<&str>,
    ) -> rusqlite::Result<usize> {
        let now = now_ms() as u128;
        let conn = self.connect()?;
        conn.execute(
            "UPDATE run_registry
             SET status=?3, cancel_requested=0, updated_at_ms=?4, error_message=?5
             WHERE (task_id=?1 OR job_id=?2)
               AND status IN ('queued', 'running', 'waiting_approval', 'waiting_user', 'cancel_requested')",
            params![
                task_id,
                job_id,
                terminal_status,
                now as i64,
                terminal_error_message,
            ],
        )
    }

    pub fn repair_database_locked_chat_runs(
        &self,
        chat_thread_id: &str,
        terminal_status: &str,
        terminal_error_message: Option<&str>,
    ) -> rusqlite::Result<usize> {
        let now = now_ms() as u128;
        let conn = self.connect()?;
        conn.execute(
            "UPDATE run_registry
             SET status=?2, cancel_requested=0, updated_at_ms=?3, error_message=?4
             WHERE chat_thread_id=?1
               AND status='failed'
               AND lower(COALESCE(error_message, '')) LIKE '%database is locked%'",
            params![
                chat_thread_id,
                terminal_status,
                now as i64,
                terminal_error_message,
            ],
        )
    }
}

fn next_run_id() -> String {
    format!(
        "run_{}_{}",
        now_ms(),
        RUN_COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

const RUN_SELECT_COLUMNS: &str = "SELECT run_id, workspace_uid, container_id, run_kind, chat_thread_id, task_id, job_id, worker_id, status, cancel_requested, heartbeat_at_ms, started_at_ms, updated_at_ms, error_message FROM run_registry";

fn run_select_sql(clause: &str) -> String {
    format!("{RUN_SELECT_COLUMNS} {clause}")
}

fn row_to_run(row: &rusqlite::Row<'_>) -> rusqlite::Result<RunRecord> {
    Ok(RunRecord {
        run_id: row.get(0)?,
        workspace_uid: row.get(1)?,
        container_id: row.get(2)?,
        run_kind: row.get(3)?,
        chat_thread_id: row.get(4)?,
        task_id: row.get(5)?,
        job_id: row.get(6)?,
        worker_id: row.get(7)?,
        status: row.get(8)?,
        cancel_requested: row.get::<_, i64>(9)? != 0,
        heartbeat_at_ms: row.get::<_, Option<i64>>(10)?.map(|value| value as u128),
        started_at_ms: row.get::<_, i64>(11)? as u128,
        updated_at_ms: row.get::<_, i64>(12)? as u128,
        error_message: row.get(13)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::PathBuf;

    fn temp_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("supernova_run_registry_{name}_{}", now_ms()));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn test_db(name: &str) -> ProductDb {
        ProductDb::open(&temp_root(name), format!("workspace_{name}")).unwrap()
    }

    #[test]
    fn run_registry_records_lifecycle_and_container_page() {
        let db = test_db("lifecycle");
        let run = db
            .insert_run(NewRun {
                container_id: "container_a".into(),
                run_kind: "task".into(),
                chat_thread_id: None,
                task_id: None,
                job_id: None,
                worker_id: "in_process:1".into(),
            })
            .unwrap();

        assert_eq!(run.status, "queued");
        let running = db.update_run_status(&run.run_id, "running", None).unwrap();
        assert_eq!(running.status, "running");
        assert!(db
            .heartbeat_run(&run.run_id)
            .unwrap()
            .unwrap()
            .heartbeat_at_ms
            .is_some());
        let bound = db.bind_run_task(&run.run_id, "job_1", "job_1").unwrap();
        assert_eq!(bound.task_id.as_deref(), Some("job_1"));
        let completed = db
            .update_run_status(&run.run_id, "completed", None)
            .unwrap();
        assert_eq!(completed.status, "completed");

        let page = db.list_runs(Some("container_a")).unwrap();
        assert_eq!(page.len(), 1);
        assert_eq!(page[0].run_id, run.run_id);
    }

    #[test]
    fn cancelling_task_run_is_scoped_to_matching_task_id() {
        let db = test_db("cancel_scope");
        let first = db
            .insert_run(NewRun {
                container_id: "container_a".into(),
                run_kind: "task".into(),
                chat_thread_id: None,
                task_id: Some("task_a".into()),
                job_id: Some("task_a".into()),
                worker_id: "in_process:1".into(),
            })
            .unwrap();
        let second = db
            .insert_run(NewRun {
                container_id: "container_b".into(),
                run_kind: "task".into(),
                chat_thread_id: None,
                task_id: Some("task_b".into()),
                job_id: Some("task_b".into()),
                worker_id: "in_process:1".into(),
            })
            .unwrap();
        db.update_run_status(&first.run_id, "running", None)
            .unwrap();
        db.update_run_status(&second.run_id, "running", None)
            .unwrap();

        let cancelled = db.request_cancel_task_runs("task_a").unwrap();

        assert_eq!(cancelled.len(), 1);
        assert_eq!(cancelled[0].run_id, first.run_id);
        assert!(cancelled[0].cancel_requested);
        assert_eq!(db.get_run(&second.run_id).unwrap().status, "running");
    }

    #[test]
    fn repair_stale_active_task_runs_uses_terminal_truth_without_touching_unrelated_runs() {
        let db = test_db("repair_stale_active");
        let stale = db
            .insert_run(NewRun {
                container_id: "container_a".into(),
                run_kind: "task".into(),
                chat_thread_id: None,
                task_id: Some("task_a".into()),
                job_id: Some("job_a".into()),
                worker_id: "process:1".into(),
            })
            .unwrap();
        let unrelated = db
            .insert_run(NewRun {
                container_id: "container_a".into(),
                run_kind: "task".into(),
                chat_thread_id: None,
                task_id: Some("task_b".into()),
                job_id: Some("job_b".into()),
                worker_id: "process:2".into(),
            })
            .unwrap();
        db.update_run_status(&stale.run_id, "running", None)
            .unwrap();
        db.update_run_status(&unrelated.run_id, "running", None)
            .unwrap();

        let repaired = db
            .repair_stale_active_task_runs("task_a", "job_a", "completed", None)
            .unwrap();

        assert_eq!(repaired, 1);
        assert_eq!(db.get_run(&stale.run_id).unwrap().status, "completed");
        assert_eq!(db.get_run(&unrelated.run_id).unwrap().status, "running");
    }
}
