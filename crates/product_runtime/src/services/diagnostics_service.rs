use local_runtime_protocol::DiagnosticsSnapshot;

use crate::app_paths::AppPaths;

#[derive(Clone)]
pub struct DiagnosticsService {
    app_paths: AppPaths,
    workspace_uid: String,
}

impl DiagnosticsService {
    pub fn new(app_paths: AppPaths, workspace_uid: String) -> Self {
        Self {
            app_paths,
            workspace_uid,
        }
    }

    pub fn snapshot(&self) -> DiagnosticsSnapshot {
        DiagnosticsSnapshot {
            runtime_status: "ready".into(),
            protocol_version: local_runtime_protocol::PROTOCOL_VERSION.into(),
            runtime_layer: "rust_product_runtime".into(),
            kernel_layer: "rust_process_kernel".into(),
            app_config_root: self.app_paths.app_config_root.to_string_lossy().to_string(),
            app_state_root: self.app_paths.app_state_root.to_string_lossy().to_string(),
            workspace_id: self.workspace_uid.clone(),
            last_error: None,
        }
    }
}
