use rusqlite::{Connection, OptionalExtension};

pub fn init_product_db(conn: &Connection) -> rusqlite::Result<()> {
    let has_legacy_containers = table_exists(conn, "containers")?;
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS container_projection(
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

        CREATE TABLE IF NOT EXISTS active_container(
          workspace_uid TEXT PRIMARY KEY,
          container_id TEXT NOT NULL,
          updated_at_ms INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS chat_threads(
          chat_thread_id TEXT PRIMARY KEY,
          container_id TEXT NOT NULL,
          title TEXT NOT NULL,
          created_at_ms INTEGER NOT NULL,
          updated_at_ms INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS tasks(
          task_id TEXT PRIMARY KEY,
          container_id TEXT NOT NULL,
          job_id TEXT,
          title TEXT NOT NULL,
          goal TEXT NOT NULL,
          status TEXT NOT NULL,
          created_at_ms INTEGER NOT NULL,
          updated_at_ms INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS context_packs(
          context_pack_id TEXT PRIMARY KEY,
          container_id TEXT NOT NULL,
          selected_items_json TEXT NOT NULL,
          excluded_items_json TEXT NOT NULL,
          auto_policy_json TEXT NOT NULL,
          summary_ref TEXT,
          estimated_tokens INTEGER
        );

        CREATE TABLE IF NOT EXISTS task_draft_artifacts(
          draft_id TEXT PRIMARY KEY,
          workspace_uid TEXT NOT NULL,
          container_id TEXT NOT NULL,
          task_id TEXT NOT NULL,
          approval_id TEXT NOT NULL,
          preview_ref TEXT,
          operation TEXT,
          status TEXT NOT NULL,
          content_format TEXT NOT NULL,
          content_text TEXT NOT NULL,
          created_at_ms INTEGER NOT NULL,
          updated_at_ms INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_task_draft_artifacts_task ON task_draft_artifacts(task_id);
        CREATE UNIQUE INDEX IF NOT EXISTS idx_task_draft_artifacts_approval ON task_draft_artifacts(task_id, approval_id);

        CREATE TABLE IF NOT EXISTS model_config_profiles(
          profile_id TEXT PRIMARY KEY,
          config_json TEXT NOT NULL,
          updated_at_ms INTEGER NOT NULL
        );

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
        CREATE INDEX IF NOT EXISTS idx_messages_container_sort ON messages(container_id, sort_key);
        CREATE INDEX IF NOT EXISTS idx_messages_chat_sort ON messages(chat_thread_id, sort_key);
        CREATE INDEX IF NOT EXISTS idx_messages_task_sort ON messages(task_id, sort_key);

        CREATE TABLE IF NOT EXISTS runtime_event_log(
          event_id TEXT PRIMARY KEY,
          workspace_uid TEXT NOT NULL,
          partition_key TEXT NOT NULL,
          container_id TEXT,
          run_id TEXT,
          task_id TEXT,
          chat_thread_id TEXT,
          event_type TEXT NOT NULL,
          payload_json TEXT NOT NULL,
          created_at_ms INTEGER NOT NULL,
          cursor_seq INTEGER NOT NULL UNIQUE,
          projection_status TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_runtime_event_log_cursor ON runtime_event_log(cursor_seq);
        CREATE INDEX IF NOT EXISTS idx_runtime_event_log_partition_cursor ON runtime_event_log(partition_key, cursor_seq);
        CREATE INDEX IF NOT EXISTS idx_runtime_event_log_container_cursor ON runtime_event_log(container_id, cursor_seq);
        CREATE INDEX IF NOT EXISTS idx_runtime_event_log_run_cursor ON runtime_event_log(run_id, cursor_seq);

        CREATE TABLE IF NOT EXISTS run_registry(
          run_id TEXT PRIMARY KEY,
          workspace_uid TEXT NOT NULL,
          container_id TEXT NOT NULL,
          run_kind TEXT NOT NULL,
          chat_thread_id TEXT,
          task_id TEXT,
          job_id TEXT,
          worker_id TEXT NOT NULL,
          status TEXT NOT NULL,
          cancel_requested INTEGER NOT NULL DEFAULT 0,
          heartbeat_at_ms INTEGER,
          started_at_ms INTEGER NOT NULL,
          updated_at_ms INTEGER NOT NULL,
          error_message TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_run_registry_container_status ON run_registry(container_id, status, updated_at_ms);
        CREATE INDEX IF NOT EXISTS idx_run_registry_task ON run_registry(task_id);
        CREATE INDEX IF NOT EXISTS idx_run_registry_chat_thread ON run_registry(chat_thread_id);
        CREATE INDEX IF NOT EXISTS idx_run_registry_job ON run_registry(job_id);

        CREATE TABLE IF NOT EXISTS projection_shards(
          shard_id TEXT PRIMARY KEY,
          workspace_uid TEXT NOT NULL,
          container_id TEXT NOT NULL,
          shard_kind TEXT NOT NULL,
          chat_thread_id TEXT,
          task_id TEXT,
          job_id TEXT,
          relative_db_path TEXT NOT NULL,
          status TEXT NOT NULL,
          created_at_ms INTEGER NOT NULL,
          updated_at_ms INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_projection_shards_container ON projection_shards(container_id, updated_at_ms);
        CREATE INDEX IF NOT EXISTS idx_projection_shards_chat_thread ON projection_shards(chat_thread_id);
        CREATE INDEX IF NOT EXISTS idx_projection_shards_task ON projection_shards(task_id);
        CREATE INDEX IF NOT EXISTS idx_projection_shards_job ON projection_shards(job_id);
        "#,
    )?;
    if has_legacy_containers {
        migrate_legacy_containers(conn)?;
    }
    Ok(())
}

pub fn init_workspace_registry(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS workspaces(
          workspace_uid TEXT PRIMARY KEY,
          workspace_root TEXT NOT NULL UNIQUE,
          display_name TEXT NOT NULL,
          created_at_ms INTEGER NOT NULL,
          last_opened_at_ms INTEGER NOT NULL,
          archived INTEGER NOT NULL DEFAULT 0
        );
        "#,
    )
}

fn table_exists(conn: &Connection, table_name: &str) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT name FROM sqlite_master WHERE type='table' AND name=?1",
        [table_name],
        |_| Ok(()),
    )
    .optional()
    .map(|value| value.is_some())
}

fn migrate_legacy_containers(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute(
        r#"
        INSERT OR REPLACE INTO container_projection(
          container_id,
          workspace_uid,
          title,
          status,
          created_at_ms,
          updated_at_ms,
          last_active_at_ms,
          default_model_config_json,
          context_policy_json
        )
        SELECT
          container_id,
          workspace_uid,
          title,
          status,
          created_at_ms,
          updated_at_ms,
          last_active_at_ms,
          default_model_config_json,
          context_policy_json
        FROM containers
        "#,
        [],
    )?;
    if !table_exists(conn, "containers_legacy_pre_projection_v1")? {
        conn.execute(
            "ALTER TABLE containers RENAME TO containers_legacy_pre_projection_v1",
            [],
        )?;
    }
    Ok(())
}
