use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use serde::Serialize;
use tokio::sync::oneshot;
use uuid::Uuid;

use crate::app_paths::{workspace_uid, AppPaths, APP_HOME_WORKSPACE_ID};
use crate::app_state::ProductRuntimeState;
use crate::http::router::build_router;
use crate::services::Services;
use crate::state::workspace_registry::WorkspaceRegistry;

#[derive(Clone, Debug)]
pub struct ProductRuntimeConfig {
    pub bind_host: String,
    pub preferred_port: u16,
    pub app_config_root: Option<PathBuf>,
    pub app_state_root: Option<PathBuf>,
    pub initial_workspace: PathBuf,
    pub register_initial_workspace: bool,
    pub runtime_token: Option<String>,
    pub allowed_origins: Vec<String>,
}

pub struct ProductRuntimeHandle {
    pub base_url: String,
    pub actual_port: u16,
    state: ProductRuntimeState,
    shutdown_tx: Option<oneshot::Sender<()>>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ProductRuntimeDiagnostics {
    pub runtime_layer: &'static str,
    pub status: &'static str,
    pub base_url: String,
    pub workspace_id: String,
    pub actual_port: u16,
}

impl ProductRuntimeHandle {
    pub fn runtime_token(&self) -> &str {
        self.state.runtime_token()
    }

    pub fn diagnostics_snapshot(&self) -> ProductRuntimeDiagnostics {
        ProductRuntimeDiagnostics {
            runtime_layer: "rust_product_runtime",
            status: "ready",
            base_url: self.base_url.clone(),
            workspace_id: self.state.workspace_uid(),
            actual_port: self.actual_port,
        }
    }

    pub async fn shutdown(mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
}

pub async fn start_product_runtime(
    config: ProductRuntimeConfig,
) -> Result<ProductRuntimeHandle, Box<dyn std::error::Error + Send + Sync>> {
    let app_paths = AppPaths::resolve(config.app_config_root, config.app_state_root);
    std::fs::create_dir_all(app_paths.app_config_root.join("config"))?;
    std::fs::create_dir_all(app_paths.app_state_root.join("state").join("workspaces"))?;
    let runtime_token = normalize_runtime_token(config.runtime_token)?;
    let requested_initial_workspace = config.initial_workspace.clone();
    let initial_workspace = config
        .initial_workspace
        .canonicalize()
        .unwrap_or(config.initial_workspace);
    if !config.register_initial_workspace {
        archive_desktop_bootstrap_workspaces(
            &app_paths,
            &requested_initial_workspace,
            &initial_workspace,
        )?;
    }
    let (workspace_root, workspace_id, workspace_state_root) = if config.register_initial_workspace
    {
        let workspace_id = workspace_uid(&initial_workspace);
        (
            initial_workspace,
            workspace_id.clone(),
            app_paths.workspace_state_root(&workspace_id),
        )
    } else {
        let app_home_root = app_paths.app_home_state_root();
        (
            app_home_root.clone(),
            APP_HOME_WORKSPACE_ID.to_string(),
            app_home_root,
        )
    };
    let services = Arc::new(Services::open(
        app_paths.clone(),
        workspace_root.clone(),
        workspace_id.clone(),
        workspace_state_root,
        config.register_initial_workspace,
    )?);
    Services::spawn_projection_repair(Arc::clone(&services));
    let state = ProductRuntimeState::new(
        app_paths,
        workspace_root,
        workspace_id,
        services,
        runtime_token,
    );
    let listener =
        tokio::net::TcpListener::bind(format!("{}:{}", config.bind_host, config.preferred_port))
            .await?;
    let addr = listener.local_addr()?;
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let router = build_router(state.clone(), config.allowed_origins);
    tokio::spawn(async move {
        let _ = axum::serve(listener, router)
            .with_graceful_shutdown(async move {
                let _ = shutdown_rx.await;
            })
            .await;
    });
    Ok(ProductRuntimeHandle {
        base_url: format!("http://{}", normalize_addr(addr)),
        actual_port: addr.port(),
        state,
        shutdown_tx: Some(shutdown_tx),
    })
}

fn normalize_addr(addr: SocketAddr) -> String {
    match addr {
        SocketAddr::V4(v4) if v4.ip().is_unspecified() => format!("127.0.0.1:{}", v4.port()),
        _ => addr.to_string(),
    }
}

fn normalize_runtime_token(
    configured: Option<String>,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let token = configured
        .or_else(|| std::env::var("SUPERNOVA_RUNTIME_TOKEN").ok())
        .unwrap_or_else(generate_runtime_token);
    let trimmed = token.trim().to_string();
    if trimmed.is_empty() {
        return Err("runtime token must not be empty".into());
    }
    Ok(trimmed)
}

fn generate_runtime_token() -> String {
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}

fn archive_desktop_bootstrap_workspaces(
    app_paths: &AppPaths,
    requested_initial_workspace: &PathBuf,
    initial_workspace: &PathBuf,
) -> rusqlite::Result<()> {
    let registry = WorkspaceRegistry::open(&app_paths.app_config_root)?;
    let mut roots = vec![
        requested_initial_workspace.clone(),
        initial_workspace.clone(),
        app_paths.app_state_root.clone(),
    ];
    if let Ok(canonical_state_root) = app_paths.app_state_root.canonicalize() {
        roots.push(canonical_state_root);
    }
    registry.archive_matching_roots(&roots)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::workspace_registry::now_ms;

    fn temp_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("supernova_bootstrap_{name}_{}", now_ms()));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[tokio::test]
    async fn desktop_bootstrap_uses_app_home_state_and_archives_legacy_workspace() {
        let config_root = temp_root("config");
        let state_root = temp_root("state");
        let legacy_bootstrap_root = state_root.join("SuperNova");
        std::fs::create_dir_all(&legacy_bootstrap_root).unwrap();
        let legacy_workspace_uid = workspace_uid(&legacy_bootstrap_root);
        let registry = WorkspaceRegistry::open(&config_root).unwrap();
        registry
            .register(&legacy_bootstrap_root, Some("SuperNova".into()))
            .unwrap();

        let handle = start_product_runtime(ProductRuntimeConfig {
            bind_host: "127.0.0.1".into(),
            preferred_port: 0,
            app_config_root: Some(config_root.clone()),
            app_state_root: Some(state_root.clone()),
            initial_workspace: legacy_bootstrap_root,
            register_initial_workspace: false,
            runtime_token: Some("test-runtime-token".into()),
            allowed_origins: Vec::new(),
        })
        .await
        .unwrap();

        assert_eq!(
            handle.diagnostics_snapshot().workspace_id,
            APP_HOME_WORKSPACE_ID
        );
        handle.shutdown().await;

        let listed = WorkspaceRegistry::open(&config_root)
            .unwrap()
            .list()
            .unwrap();
        assert!(listed.is_empty());
        assert!(state_root
            .join("state")
            .join("app_home")
            .join("product.sqlite3")
            .exists());
        assert!(!state_root
            .join("state")
            .join("workspaces")
            .join(legacy_workspace_uid)
            .join("product.sqlite3")
            .exists());
    }
}
