use std::path::PathBuf;

use local_runtime_protocol::ArtifactTargetOption;

use crate::app_paths::AppPaths;

#[derive(Clone)]
pub struct ArtifactService {
    app_paths: AppPaths,
    workspace_root: PathBuf,
    workspace_uid: String,
}

impl ArtifactService {
    pub fn new(app_paths: AppPaths, workspace_root: PathBuf, workspace_uid: String) -> Self {
        Self {
            app_paths,
            workspace_root,
            workspace_uid,
        }
    }

    pub fn target_options(&self, _container_id: &str) -> Vec<ArtifactTargetOption> {
        vec![
            ArtifactTargetOption {
                target_id: "workspace_artifacts".into(),
                label: "Workspace artifacts".into(),
                target_dir: self
                    .workspace_root
                    .join("artifacts")
                    .to_string_lossy()
                    .to_string(),
                artifact_types: vec!["text".into(), "report".into(), "package".into()],
                save_strategies: vec!["ask_before_overwrite".into(), "create_revision".into()],
                user_visible: true,
            },
            ArtifactTargetOption {
                target_id: "local_appdata_blobs".into(),
                label: "Local app data blobs".into(),
                target_dir: self
                    .app_paths
                    .workspace_state_root(&self.workspace_uid)
                    .join("blobs")
                    .to_string_lossy()
                    .to_string(),
                artifact_types: vec!["preview".into(), "receipt".into(), "internal_blob".into()],
                save_strategies: vec!["internal_only".into()],
                user_visible: false,
            },
        ]
    }
}
