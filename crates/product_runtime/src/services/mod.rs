pub mod artifact_service;
pub mod capability_manifest_service;
pub mod chat_service;
pub mod container_service;
pub mod context_pack_service;
pub mod diagnostics_service;
pub mod model_config_service;
pub mod run_manager;
pub mod settings_service;
pub mod task_service;
pub mod workspace_service;

use std::path::PathBuf;
use std::sync::Arc;

use crate::app_paths::AppPaths;
use crate::kernel::KernelBridge;
use crate::state::product_db::ProductDb;
use crate::state::workspace_registry::WorkspaceRegistry;

#[derive(Clone)]
pub struct Services {
    pub workspace: workspace_service::WorkspaceService,
    pub container: container_service::ContainerService,
    pub chat: chat_service::ChatService,
    pub task: task_service::TaskService,
    pub context_pack: context_pack_service::ContextPackService,
    pub model_config: model_config_service::ModelConfigService,
    pub artifact: artifact_service::ArtifactService,
    pub settings: settings_service::SettingsService,
    pub diagnostics: diagnostics_service::DiagnosticsService,
    pub capability_manifest: capability_manifest_service::CapabilityManifestService,
    pub run_manager: run_manager::RunManager,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WorkspaceProjectionRepairReport {
    pub chat: chat_service::ChatProjectionRepairReport,
    pub task: task_service::TaskProjectionRepairReport,
}

impl Services {
    pub fn open(
        app_paths: AppPaths,
        workspace_root: PathBuf,
        workspace_uid: String,
        workspace_state_root: PathBuf,
        register_workspace: bool,
    ) -> rusqlite::Result<Self> {
        let registry = WorkspaceRegistry::open(&app_paths.app_config_root)?;
        if register_workspace {
            registry.register(&workspace_root, None)?;
        }
        let product_db = ProductDb::open(&workspace_state_root, workspace_uid.clone())?;
        let provider_profile_root = app_paths
            .app_config_root
            .join("kernel_provider_credentials");
        let kernel = KernelBridge::new(
            workspace_root.clone(),
            workspace_state_root.clone(),
            provider_profile_root.clone(),
        );
        let context_pack = context_pack_service::ContextPackService::new(
            product_db.clone(),
            kernel.clone(),
            workspace_root.clone(),
        );
        let settings = settings_service::SettingsService::new(app_paths.clone(), kernel.clone());
        let run_manager = run_manager::RunManager::with_process_worker(
            product_db.clone(),
            workspace_root.clone(),
            workspace_state_root.clone(),
            provider_profile_root,
        );
        let chat = chat_service::ChatService::new(
            product_db.clone(),
            kernel.clone(),
            context_pack.clone(),
            run_manager.clone(),
            settings.clone(),
        );
        let task = task_service::TaskService::new(
            product_db.clone(),
            kernel.clone(),
            context_pack.clone(),
            run_manager.clone(),
            settings.clone(),
        );
        Ok(Self {
            workspace: workspace_service::WorkspaceService::new(registry),
            container: container_service::ContainerService::new(product_db.clone(), kernel.clone()),
            chat,
            task,
            context_pack,
            model_config: model_config_service::ModelConfigService::new(product_db.clone()),
            artifact: artifact_service::ArtifactService::new(
                app_paths.clone(),
                workspace_root.clone(),
                workspace_uid.clone(),
            ),
            settings,
            diagnostics: diagnostics_service::DiagnosticsService::new(app_paths, workspace_uid),
            capability_manifest: capability_manifest_service::CapabilityManifestService::new(),
            run_manager,
        })
    }

    pub fn repair_workspace_projection(&self) -> rusqlite::Result<WorkspaceProjectionRepairReport> {
        Ok(WorkspaceProjectionRepairReport {
            chat: self.chat.repair_workspace_projection()?,
            task: self.task.repair_workspace_projection()?,
        })
    }

    pub fn spawn_projection_repair(services: Arc<Self>) {
        let _ = std::thread::Builder::new()
            .name("supernova-projection-repair".into())
            .spawn(move || {
                let _ = services.repair_workspace_projection();
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_paths::workspace_uid;
    use crate::services::run_manager::RunManager;
    use crate::state::message_feed::new_message;
    use crate::state::workspace_registry::now_ms;
    use local_runtime_protocol::{
        ContainerBadges, MessageLane, MessageRole, MessageType, TaskRecord,
    };
    use serde_json::json;
    use supernova_process_kernel::ProcessTruthStore;

    fn temp_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("supernova_services_{name}_{}", now_ms()));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn opening_services_can_skip_bootstrap_workspace_registration() {
        let config_root = temp_root("skip_register_config");
        let state_root = temp_root("skip_register_state");
        let workspace_root = temp_root("skip_register_workspace");
        let app_paths = AppPaths {
            app_config_root: config_root.clone(),
            app_state_root: state_root,
        };
        let workspace_uid = workspace_uid(&workspace_root);

        let workspace_state_root = app_paths.workspace_state_root(&workspace_uid);
        Services::open(
            app_paths.clone(),
            workspace_root,
            workspace_uid,
            workspace_state_root,
            false,
        )
        .unwrap();

        let registry = WorkspaceRegistry::open(&config_root).unwrap();
        assert!(registry.list().unwrap().is_empty());
    }

    #[test]
    fn workspace_projection_repair_recovers_database_locked_task_run_from_process_truth() {
        let config_root = temp_root("startup_repair_config");
        let state_root = temp_root("startup_repair_state");
        let workspace_root = temp_root("startup_repair_workspace");
        let app_paths = AppPaths {
            app_config_root: config_root,
            app_state_root: state_root,
        };
        let workspace_uid = workspace_uid(&workspace_root);
        let workspace_state_root = app_paths.workspace_state_root(&workspace_uid);
        let db = ProductDb::open(&workspace_state_root, workspace_uid.clone()).unwrap();
        let task = TaskRecord {
            task_id: "job_startup_repair".into(),
            container_id: "container_startup".into(),
            job_id: Some("job_startup_repair".into()),
            title: "Startup repair".into(),
            goal: "repair projection".into(),
            status: "running".into(),
            badges: ContainerBadges::default(),
            created_at_ms: 1,
            updated_at_ms: 1,
        };
        db.upsert_task(&task).unwrap();
        let run_manager = RunManager::new(db.clone());
        let run = run_manager.start_task_run(&task.container_id).unwrap();
        run_manager
            .bind_task_run(&run.run_id, &task.task_id, task.job_id.as_deref().unwrap())
            .unwrap();
        run_manager
            .fail_run(&run.run_id, "database is locked")
            .unwrap();
        let mut lock_error = new_message(
            &db.workspace_uid,
            &task.container_id,
            MessageLane::Task,
            MessageRole::System,
            MessageType::Error,
            Some("database is locked".into()),
            None,
        );
        lock_error.source_kind = "kernel_bridge".into();
        lock_error.source_ref = "task_start".into();
        db.append_message(lock_error).unwrap();
        let truth = ProcessTruthStore::new_with_state_root(
            &workspace_root,
            &workspace_state_root,
            &task.task_id,
        )
        .unwrap();
        truth
            .append_event(
                Some("root"),
                "job_status_changed",
                json!({"status": "running"}),
            )
            .unwrap();
        truth
            .append_event(Some("root"), "job_completed", json!({}))
            .unwrap();

        let services = Services::open(
            app_paths,
            workspace_root,
            workspace_uid,
            workspace_state_root,
            false,
        )
        .unwrap();
        let report = services.repair_workspace_projection().unwrap();
        let repaired = services
            .run_manager
            .list_runs(Some(&task.container_id))
            .unwrap();
        let repaired_run = repaired
            .iter()
            .find(|item| item.run_id == run.run_id)
            .unwrap();

        assert_eq!(repaired_run.status, "completed");
        assert!(repaired_run.error_message.is_none());
        assert_eq!(report.task.runs_repaired, 1);
    }
}
