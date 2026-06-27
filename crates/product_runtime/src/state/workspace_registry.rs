use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use local_runtime_protocol::WorkspaceRecord;
use rusqlite::{params, Connection};

use crate::app_paths::workspace_uid;
use crate::state::migrations::init_workspace_registry;

#[derive(Clone)]
pub struct WorkspaceRegistry {
    db_path: PathBuf,
}

impl WorkspaceRegistry {
    pub fn open(app_config_root: &Path) -> rusqlite::Result<Self> {
        let dir = app_config_root.join("config");
        std::fs::create_dir_all(&dir)
            .map_err(|err| rusqlite::Error::ToSqlConversionFailure(Box::new(err)))?;
        let db_path = dir.join("workspace_registry.sqlite3");
        let conn = Connection::open(&db_path)?;
        init_workspace_registry(&conn)?;
        Ok(Self { db_path })
    }

    pub fn register(
        &self,
        workspace_root: &Path,
        display_name: Option<String>,
    ) -> rusqlite::Result<WorkspaceRecord> {
        let uid = workspace_uid(workspace_root);
        let root = workspace_root.to_string_lossy().to_string();
        let name = display_name.unwrap_or_else(|| {
            workspace_root
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("Workspace")
                .to_string()
        });
        let now = now_ms();
        let conn = Connection::open(&self.db_path)?;
        conn.execute(
            r#"
            INSERT INTO workspaces(workspace_uid, workspace_root, display_name, created_at_ms, last_opened_at_ms, archived)
            VALUES(?1, ?2, ?3, ?4, ?4, 0)
            ON CONFLICT(workspace_root) DO UPDATE SET
              display_name=excluded.display_name,
              last_opened_at_ms=excluded.last_opened_at_ms,
              archived=0
            "#,
            params![uid, root, name, now],
        )?;
        self.get_by_uid(&uid)
    }

    pub fn list(&self) -> rusqlite::Result<Vec<WorkspaceRecord>> {
        let conn = Connection::open(&self.db_path)?;
        let mut stmt = conn.prepare(
            "SELECT workspace_uid, workspace_root, display_name, created_at_ms, last_opened_at_ms, archived FROM workspaces WHERE archived=0 ORDER BY created_at_ms ASC, workspace_uid ASC",
        )?;
        let rows = stmt.query_map([], row_to_workspace)?;
        rows.collect()
    }

    pub fn archive(&self, uid: &str) -> rusqlite::Result<WorkspaceRecord> {
        let conn = Connection::open(&self.db_path)?;
        conn.execute(
            "UPDATE workspaces SET archived=1 WHERE workspace_uid=?1",
            params![uid],
        )?;
        self.get_by_uid(uid)
    }

    pub fn archive_matching_roots(&self, roots: &[PathBuf]) -> rusqlite::Result<usize> {
        if roots.is_empty() {
            return Ok(0);
        }
        let target_roots = roots
            .iter()
            .map(|root| comparable_workspace_root(&root.to_string_lossy()))
            .collect::<HashSet<_>>();
        let conn = Connection::open(&self.db_path)?;
        let rows = {
            let mut stmt = conn
                .prepare("SELECT workspace_uid, workspace_root FROM workspaces WHERE archived=0")?;
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        let mut archived = 0;
        for (workspace_uid, workspace_root) in rows {
            if target_roots.contains(&comparable_workspace_root(&workspace_root)) {
                archived += conn.execute(
                    "UPDATE workspaces SET archived=1 WHERE workspace_uid=?1",
                    params![workspace_uid],
                )?;
            }
        }
        Ok(archived)
    }

    pub fn get_by_uid(&self, uid: &str) -> rusqlite::Result<WorkspaceRecord> {
        let conn = Connection::open(&self.db_path)?;
        conn.query_row(
            "SELECT workspace_uid, workspace_root, display_name, created_at_ms, last_opened_at_ms, archived FROM workspaces WHERE workspace_uid=?1",
            params![uid],
            row_to_workspace,
        )
    }
}

fn row_to_workspace(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkspaceRecord> {
    Ok(WorkspaceRecord {
        workspace_uid: row.get(0)?,
        workspace_root: row.get(1)?,
        display_name: row.get(2)?,
        created_at_ms: row.get::<_, i64>(3)? as u128,
        last_opened_at_ms: row.get::<_, i64>(4)? as u128,
        archived: row.get::<_, i64>(5)? != 0,
    })
}

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn comparable_workspace_root(value: &str) -> String {
    let mut root = value.replace('\\', "/");
    if let Some(stripped) = root.strip_prefix("//?/") {
        root = stripped.to_string();
    }
    while root.ends_with('/') && root.len() > 3 {
        root.pop();
    }
    #[cfg(windows)]
    {
        root = root.to_lowercase();
    }
    root
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(name: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("supernova_workspace_registry_{name}_{}", now_ms()));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn archive_hides_workspace_from_default_list() {
        let config_root = temp_root("config");
        let workspace_a = temp_root("a");
        let workspace_b = temp_root("b");
        let registry = WorkspaceRegistry::open(&config_root).unwrap();
        let archived = registry.register(&workspace_a, Some("A".into())).unwrap();
        let visible = registry.register(&workspace_b, Some("B".into())).unwrap();

        let archived_record = registry.archive(&archived.workspace_uid).unwrap();
        let listed = registry.list().unwrap();
        let listed_uids = listed
            .iter()
            .map(|workspace| workspace.workspace_uid.as_str())
            .collect::<Vec<_>>();

        assert!(archived_record.archived);
        assert!(listed_uids.contains(&visible.workspace_uid.as_str()));
        assert!(!listed_uids.contains(&archived.workspace_uid.as_str()));
        assert!(
            registry
                .get_by_uid(&archived.workspace_uid)
                .unwrap()
                .archived
        );
    }

    #[test]
    fn list_order_is_stable_after_workspace_reopen() {
        let config_root = temp_root("stable_order_config");
        let workspace_a = temp_root("stable_order_a");
        let workspace_b = temp_root("stable_order_b");
        let registry = WorkspaceRegistry::open(&config_root).unwrap();
        let first = registry.register(&workspace_a, Some("A".into())).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let second = registry.register(&workspace_b, Some("B".into())).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        registry.register(&workspace_a, Some("A".into())).unwrap();

        let listed = registry.list().unwrap();
        assert_eq!(listed[0].workspace_uid, first.workspace_uid);
        assert_eq!(listed[1].workspace_uid, second.workspace_uid);
    }

    #[test]
    fn archive_matching_roots_matches_windows_extended_path_prefix() {
        let config_root = temp_root("archive_matching_config");
        let registry = WorkspaceRegistry::open(&config_root).unwrap();
        let root = PathBuf::from(r"\\?\C:\Users\86188\AppData\Local\SuperNova");
        let archived = registry.register(&root, Some("SuperNova".into())).unwrap();
        let visible = registry
            .register(
                &PathBuf::from(r"C:\Users\86188\Desktop\UserWorkspace"),
                Some("UserWorkspace".into()),
            )
            .unwrap();

        let archived_count = registry
            .archive_matching_roots(&[PathBuf::from(r"C:\Users\86188\AppData\Local\SuperNova")])
            .unwrap();
        let listed = registry.list().unwrap();
        let listed_uids = listed
            .iter()
            .map(|workspace| workspace.workspace_uid.as_str())
            .collect::<Vec<_>>();

        assert_eq!(archived_count, 1);
        assert!(!listed_uids.contains(&archived.workspace_uid.as_str()));
        assert!(listed_uids.contains(&visible.workspace_uid.as_str()));
    }
}
