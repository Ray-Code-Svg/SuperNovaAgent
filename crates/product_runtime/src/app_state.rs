use std::path::PathBuf;
use std::sync::Arc;
use std::sync::RwLock;
use std::time::Instant;

use crate::app_paths::AppPaths;
use crate::services::Services;

#[derive(Clone)]
pub struct ProductRuntimeState {
    pub app_paths: AppPaths,
    pub started_at: Instant,
    active: Arc<RwLock<ActiveWorkspace>>,
    runtime_token: Arc<str>,
}

#[derive(Clone)]
struct ActiveWorkspace {
    workspace_root: PathBuf,
    workspace_uid: String,
    services: Arc<Services>,
}

impl ProductRuntimeState {
    pub fn new(
        app_paths: AppPaths,
        workspace_root: PathBuf,
        workspace_uid: String,
        services: Arc<Services>,
        runtime_token: String,
    ) -> Self {
        Self {
            app_paths,
            started_at: Instant::now(),
            active: Arc::new(RwLock::new(ActiveWorkspace {
                workspace_root,
                workspace_uid,
                services,
            })),
            runtime_token: Arc::from(runtime_token),
        }
    }

    pub fn runtime_token(&self) -> &str {
        &self.runtime_token
    }

    pub fn workspace_root(&self) -> PathBuf {
        self.active
            .read()
            .expect("active workspace lock should not be poisoned")
            .workspace_root
            .clone()
    }

    pub fn workspace_uid(&self) -> String {
        self.active
            .read()
            .expect("active workspace lock should not be poisoned")
            .workspace_uid
            .clone()
    }

    pub fn services(&self) -> Arc<Services> {
        self.active
            .read()
            .expect("active workspace lock should not be poisoned")
            .services
            .clone()
    }

    pub fn rebind_workspace(
        &self,
        workspace_root: PathBuf,
        workspace_uid: String,
    ) -> rusqlite::Result<Option<String>> {
        let services = Arc::new(Services::open(
            self.app_paths.clone(),
            workspace_root.clone(),
            workspace_uid.clone(),
            self.app_paths.workspace_state_root(&workspace_uid),
            true,
        )?);
        let recent_active_container_id = services.container.active_container_id()?;
        Services::spawn_projection_repair(Arc::clone(&services));
        let mut active = self
            .active
            .write()
            .expect("active workspace lock should not be poisoned");
        *active = ActiveWorkspace {
            workspace_root,
            workspace_uid,
            services,
        };
        Ok(recent_active_container_id)
    }
}
