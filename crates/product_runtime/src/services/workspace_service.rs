use std::path::PathBuf;

use local_runtime_protocol::{
    ActivateWorkspaceRequest, CreateWorkspaceRequest, WorkspaceActivation, WorkspaceRecord,
};

use crate::state::workspace_registry::WorkspaceRegistry;

#[derive(Clone)]
pub struct WorkspaceService {
    registry: WorkspaceRegistry,
}

impl WorkspaceService {
    pub fn new(registry: WorkspaceRegistry) -> Self {
        Self { registry }
    }

    pub fn list(&self) -> rusqlite::Result<Vec<WorkspaceRecord>> {
        self.registry.list()
    }

    pub fn get(&self, workspace_uid: &str) -> rusqlite::Result<WorkspaceRecord> {
        self.registry.get_by_uid(workspace_uid)
    }

    pub fn create(&self, request: CreateWorkspaceRequest) -> rusqlite::Result<WorkspaceRecord> {
        let workspace_root = canonical_workspace_root(&request.workspace_root)?;
        self.registry
            .register(&workspace_root, request.display_name)
    }

    pub fn archive(&self, workspace_uid: &str) -> rusqlite::Result<WorkspaceRecord> {
        self.registry.archive(workspace_uid)
    }

    pub fn activate(
        &self,
        request: ActivateWorkspaceRequest,
    ) -> rusqlite::Result<WorkspaceActivation> {
        if request
            .workspace_root
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
        {
            return Err(bad_workspace_request(
                "workspace_root is not accepted by workspace activation; use workspace_uid.",
            ));
        }
        let workspace_uid = required_workspace_uid(request.workspace_uid)?;
        let workspace = self.registry.get_by_uid(&workspace_uid)?;
        Ok(WorkspaceActivation {
            workspace,
            recent_active_container_id: None,
        })
    }
}

fn canonical_workspace_root(raw: &str) -> rusqlite::Result<PathBuf> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(bad_workspace_request("workspace_root is required."));
    }
    let canonical = PathBuf::from(trimmed).canonicalize().map_err(|err| {
        bad_workspace_request(format!(
            "workspace_root must be an existing local directory: {err}"
        ))
    })?;
    if !canonical.is_dir() {
        return Err(bad_workspace_request(
            "workspace_root must be an existing local directory.",
        ));
    }
    Ok(canonical)
}

fn required_workspace_uid(value: Option<String>) -> rusqlite::Result<String> {
    value
        .map(|uid| uid.trim().to_string())
        .filter(|uid| !uid.is_empty())
        .ok_or_else(|| bad_workspace_request("workspace_uid is required for activation."))
}

fn bad_workspace_request(message: impl Into<String>) -> rusqlite::Error {
    rusqlite::Error::InvalidParameterName(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::workspace_registry::{now_ms, WorkspaceRegistry};

    fn temp_root(name: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("supernova_workspace_service_{name}_{}", now_ms()));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn service(name: &str) -> WorkspaceService {
        let config_root = temp_root(&format!("{name}_config"));
        WorkspaceService::new(WorkspaceRegistry::open(&config_root).unwrap())
    }

    #[test]
    fn create_canonicalizes_workspace_root() {
        let service = service("canonical_create");
        let workspace_root = temp_root("canonical_workspace");
        let nested = workspace_root.join(".");
        let created = service
            .create(CreateWorkspaceRequest {
                workspace_root: nested.to_string_lossy().to_string(),
                display_name: Some("Canonical".into()),
            })
            .unwrap();

        assert_eq!(
            PathBuf::from(created.workspace_root),
            workspace_root.canonicalize().unwrap()
        );
    }

    #[test]
    fn activate_requires_registered_workspace_uid_and_rejects_root_fallback() {
        let service = service("activate_requires_uid");
        let rejected = service
            .activate(ActivateWorkspaceRequest {
                workspace_uid: None,
                workspace_root: Some(temp_root("unexpected_root").to_string_lossy().to_string()),
            })
            .expect_err("activation must not register a root fallback");

        assert!(rejected
            .to_string()
            .contains("workspace_root is not accepted"));
        assert!(service.list().unwrap().is_empty());
    }

    #[test]
    fn activate_rejects_workspace_root_even_with_uid() {
        let service = service("activate_rejects_root_with_uid");
        let workspace = service
            .create(CreateWorkspaceRequest {
                workspace_root: temp_root("registered").to_string_lossy().to_string(),
                display_name: None,
            })
            .unwrap();

        let rejected = service
            .activate(ActivateWorkspaceRequest {
                workspace_uid: Some(workspace.workspace_uid),
                workspace_root: Some(
                    temp_root("unexpected_root_with_uid")
                        .to_string_lossy()
                        .to_string(),
                ),
            })
            .expect_err("activation must not accept workspace_root");

        assert!(rejected
            .to_string()
            .contains("workspace_root is not accepted"));
    }

    #[test]
    fn activate_registered_workspace_uid() {
        let service = service("activate_uid");
        let workspace = service
            .create(CreateWorkspaceRequest {
                workspace_root: temp_root("registered_uid").to_string_lossy().to_string(),
                display_name: None,
            })
            .unwrap();

        let activation = service
            .activate(ActivateWorkspaceRequest {
                workspace_uid: Some(workspace.workspace_uid.clone()),
                workspace_root: None,
            })
            .unwrap();

        assert_eq!(activation.workspace.workspace_uid, workspace.workspace_uid);
    }
}
