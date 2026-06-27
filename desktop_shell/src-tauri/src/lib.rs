use std::env;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use product_runtime::{start_product_runtime, ProductRuntimeConfig, ProductRuntimeHandle};
use serde::Serialize;
use serde_json::json;
use tauri::{AppHandle, Manager, WebviewWindow};

const SHELL_SCHEMA_VERSION: &str = "supernova_tauri_shell.v2";
const SHELL_LAYER: &str = "tauri_desktop_shell";
const FRONTEND_MODE: &str = "static_tauri_frontend";
const RUNTIME_KIND: &str = "rust_product_runtime";
const DEFAULT_BIND_HOST: &str = "127.0.0.1";
const DEFAULT_RUNTIME_PORT: u16 = 8765;

#[derive(Default)]
struct ProductRuntimeState {
    runtime: Mutex<Option<ProductRuntimeHandle>>,
    close_to_tray_enabled: Mutex<bool>,
    explicit_quit_requested: Mutex<bool>,
}

#[derive(Clone, Debug, Serialize)]
struct ShellRuntimeDescriptor {
    kind: &'static str,
    base_url: String,
    runtime_token: String,
    actual_port: u16,
    workspace_id: String,
    status: &'static str,
}

#[derive(Clone, Debug, Serialize)]
struct ShellRuntimeDiagnosticsDescriptor {
    kind: &'static str,
    base_url: String,
    actual_port: u16,
    workspace_id: String,
    status: &'static str,
}

#[derive(Clone, Debug, Serialize)]
struct ShellBootstrap {
    schema_version: &'static str,
    shell_layer: &'static str,
    frontend_mode: &'static str,
    workspace_root: String,
    runtime: Option<ShellRuntimeDescriptor>,
}

#[derive(Clone, Debug, Serialize)]
struct RuntimeStatus {
    schema_version: &'static str,
    shell_layer: &'static str,
    frontend_mode: &'static str,
    runtime: Option<ShellRuntimeDescriptor>,
}

#[derive(Clone, Debug, Serialize)]
struct RuntimeShutdownResult {
    schema_version: &'static str,
    shell_layer: &'static str,
    status: &'static str,
}

#[derive(Clone, Debug, Serialize)]
struct WorkspaceDialogResult {
    schema_version: &'static str,
    shell_layer: &'static str,
    workspace_root: Option<String>,
    status: &'static str,
}

#[derive(Clone, Debug, Serialize)]
struct DiagnosticsExportResult {
    schema_version: &'static str,
    shell_layer: &'static str,
    status: &'static str,
    export_path: String,
}

#[tauri::command]
async fn shell_bootstrap(
    state: tauri::State<'_, ProductRuntimeState>,
) -> Result<ShellBootstrap, String> {
    let runtime = runtime_descriptor(&state)?;
    Ok(ShellBootstrap {
        schema_version: SHELL_SCHEMA_VERSION,
        shell_layer: SHELL_LAYER,
        frontend_mode: FRONTEND_MODE,
        workspace_root: workspace_root().to_string_lossy().to_string(),
        runtime,
    })
}

#[tauri::command]
async fn runtime_ensure(
    state: tauri::State<'_, ProductRuntimeState>,
) -> Result<RuntimeStatus, String> {
    ensure_runtime(&state).await?;
    runtime_status(state).await
}

#[tauri::command]
async fn runtime_status(
    state: tauri::State<'_, ProductRuntimeState>,
) -> Result<RuntimeStatus, String> {
    Ok(RuntimeStatus {
        schema_version: SHELL_SCHEMA_VERSION,
        shell_layer: SHELL_LAYER,
        frontend_mode: FRONTEND_MODE,
        runtime: runtime_descriptor(&state)?,
    })
}

#[tauri::command]
async fn runtime_shutdown(
    state: tauri::State<'_, ProductRuntimeState>,
) -> Result<RuntimeShutdownResult, String> {
    let handle = {
        let mut runtime = state
            .runtime
            .lock()
            .map_err(|_| "runtime state mutex poisoned".to_string())?;
        runtime.take()
    };
    if let Some(handle) = handle {
        handle.shutdown().await;
    }
    Ok(RuntimeShutdownResult {
        schema_version: SHELL_SCHEMA_VERSION,
        shell_layer: SHELL_LAYER,
        status: "stopped",
    })
}

#[tauri::command]
async fn workspace_choose_dialog() -> Result<WorkspaceDialogResult, String> {
    if let Some(workspace_root) = pick_workspace_folder()? {
        return Ok(WorkspaceDialogResult {
            schema_version: SHELL_SCHEMA_VERSION,
            shell_layer: SHELL_LAYER,
            workspace_root: Some(workspace_root.to_string_lossy().to_string()),
            status: "selected",
        });
    }
    Ok(WorkspaceDialogResult {
        schema_version: SHELL_SCHEMA_VERSION,
        shell_layer: SHELL_LAYER,
        workspace_root: None,
        status: "cancelled",
    })
}

#[tauri::command]
async fn window_minimize(window: WebviewWindow) -> Result<(), String> {
    window.minimize().map_err(|err| err.to_string())
}

#[tauri::command]
async fn window_maximize(window: WebviewWindow) -> Result<(), String> {
    if window.is_maximized().map_err(|err| err.to_string())? {
        window.unmaximize().map_err(|err| err.to_string())
    } else {
        window.maximize().map_err(|err| err.to_string())
    }
}

#[tauri::command]
async fn window_close_to_tray(
    window: WebviewWindow,
    state: tauri::State<'_, ProductRuntimeState>,
) -> Result<(), String> {
    {
        let mut enabled = state
            .close_to_tray_enabled
            .lock()
            .map_err(|_| "tray state mutex poisoned".to_string())?;
        *enabled = true;
    }
    window.hide().map_err(|err| err.to_string())
}

#[tauri::command]
async fn app_quit(
    app: AppHandle,
    state: tauri::State<'_, ProductRuntimeState>,
) -> Result<(), String> {
    {
        let mut quit = state
            .explicit_quit_requested
            .lock()
            .map_err(|_| "quit state mutex poisoned".to_string())?;
        *quit = true;
    }
    let handle = {
        let mut runtime = state
            .runtime
            .lock()
            .map_err(|_| "runtime state mutex poisoned".to_string())?;
        runtime.take()
    };
    if let Some(handle) = handle {
        handle.shutdown().await;
    }
    app.exit(0);
    Ok(())
}

#[tauri::command]
async fn diagnostics_export(
    state: tauri::State<'_, ProductRuntimeState>,
) -> Result<DiagnosticsExportResult, String> {
    let diagnostics_root = workspace_root().join("reports").join("desktop_shell");
    fs::create_dir_all(&diagnostics_root).map_err(|err| err.to_string())?;
    let export_path = diagnostics_root.join("desktop_runtime_diagnostics_v2.json");
    let payload = json!({
        "schema_version": SHELL_SCHEMA_VERSION,
        "shell_layer": SHELL_LAYER,
        "frontend_mode": FRONTEND_MODE,
        "runtime": runtime_diagnostics_descriptor(&state)?,
    });
    fs::write(
        &export_path,
        serde_json::to_vec_pretty(&payload).map_err(|err| err.to_string())?,
    )
    .map_err(|err| err.to_string())?;
    Ok(DiagnosticsExportResult {
        schema_version: SHELL_SCHEMA_VERSION,
        shell_layer: SHELL_LAYER,
        status: "exported",
        export_path: export_path.to_string_lossy().to_string(),
    })
}

async fn ensure_runtime(state: &ProductRuntimeState) -> Result<(), String> {
    if state
        .runtime
        .lock()
        .map_err(|_| "runtime state mutex poisoned".to_string())?
        .is_some()
    {
        return Ok(());
    }

    let config = ProductRuntimeConfig {
        bind_host: DEFAULT_BIND_HOST.into(),
        preferred_port: DEFAULT_RUNTIME_PORT,
        app_config_root: None,
        app_state_root: None,
        initial_workspace: workspace_root(),
        register_initial_workspace: false,
        runtime_token: None,
        allowed_origins: Vec::new(),
    };
    let handle = match start_product_runtime(config).await {
        Ok(handle) => handle,
        Err(_) => start_product_runtime(ProductRuntimeConfig {
            bind_host: DEFAULT_BIND_HOST.into(),
            preferred_port: 0,
            app_config_root: None,
            app_state_root: None,
            initial_workspace: workspace_root(),
            register_initial_workspace: false,
            runtime_token: None,
            allowed_origins: Vec::new(),
        })
        .await
        .map_err(|err| err.to_string())?,
    };
    let mut runtime = state
        .runtime
        .lock()
        .map_err(|_| "runtime state mutex poisoned".to_string())?;
    *runtime = Some(handle);
    Ok(())
}

fn runtime_descriptor(
    state: &ProductRuntimeState,
) -> Result<Option<ShellRuntimeDescriptor>, String> {
    let runtime = state
        .runtime
        .lock()
        .map_err(|_| "runtime state mutex poisoned".to_string())?;
    Ok(runtime.as_ref().map(|handle| {
        let diagnostics = handle.diagnostics_snapshot();
        ShellRuntimeDescriptor {
            kind: RUNTIME_KIND,
            base_url: diagnostics.base_url,
            runtime_token: handle.runtime_token().to_string(),
            actual_port: diagnostics.actual_port,
            workspace_id: diagnostics.workspace_id,
            status: diagnostics.status,
        }
    }))
}

fn runtime_diagnostics_descriptor(
    state: &ProductRuntimeState,
) -> Result<Option<ShellRuntimeDiagnosticsDescriptor>, String> {
    let runtime = state
        .runtime
        .lock()
        .map_err(|_| "runtime state mutex poisoned".to_string())?;
    Ok(runtime.as_ref().map(|handle| {
        let diagnostics = handle.diagnostics_snapshot();
        ShellRuntimeDiagnosticsDescriptor {
            kind: RUNTIME_KIND,
            base_url: diagnostics.base_url,
            actual_port: diagnostics.actual_port,
            workspace_id: diagnostics.workspace_id,
            status: diagnostics.status,
        }
    }))
}

fn workspace_root() -> PathBuf {
    env::var("SUPERNOVA_WORKSPACE_ROOT")
        .map(PathBuf::from)
        .ok()
        .or_else(|| env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
}

#[cfg(target_os = "windows")]
fn pick_workspace_folder() -> Result<Option<PathBuf>, String> {
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_INPROC_SERVER,
        COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::UI::Shell::{
        FileOpenDialog, IFileOpenDialog, FOS_FORCEFILESYSTEM, FOS_NOCHANGEDIR, FOS_PICKFOLDERS,
        SIGDN_FILESYSPATH,
    };

    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED)
            .ok()
            .map_err(|err| format!("failed to initialize COM for workspace dialog: {err}"))?;
        let dialog_result = (|| -> Result<Option<PathBuf>, String> {
            let dialog: IFileOpenDialog =
                CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)
                    .map_err(|err| format!("failed to create workspace dialog: {err}"))?;
            dialog
                .SetOptions(FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM | FOS_NOCHANGEDIR)
                .map_err(|err| format!("failed to configure workspace dialog: {err}"))?;
            if dialog.Show(None).is_err() {
                return Ok(None);
            }
            let item = dialog
                .GetResult()
                .map_err(|err| format!("failed to read selected workspace: {err}"))?;
            let raw = item
                .GetDisplayName(SIGDN_FILESYSPATH)
                .map_err(|err| format!("failed to read selected workspace path: {err}"))?;
            let path = raw
                .to_string()
                .map(PathBuf::from)
                .map_err(|err| format!("failed to decode selected workspace path: {err}"))?;
            CoTaskMemFree(Some(raw.0.cast()));
            Ok(Some(path))
        })();
        CoUninitialize();
        dialog_result
    }
}

#[cfg(not(target_os = "windows"))]
fn pick_workspace_folder() -> Result<Option<PathBuf>, String> {
    Ok(None)
}

pub fn run() {
    tauri::Builder::default()
        .manage(ProductRuntimeState::default())
        .invoke_handler(tauri::generate_handler![
            shell_bootstrap,
            runtime_ensure,
            runtime_status,
            runtime_shutdown,
            workspace_choose_dialog,
            window_minimize,
            window_maximize,
            window_close_to_tray,
            app_quit,
            diagnostics_export
        ])
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let state = app.state::<ProductRuntimeState>();
            tauri::async_runtime::block_on(async { ensure_runtime(&state).await })?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let state = window.state::<ProductRuntimeState>();
                let explicit_quit = state
                    .explicit_quit_requested
                    .lock()
                    .map(|value| *value)
                    .unwrap_or(false);
                let close_to_tray = state
                    .close_to_tray_enabled
                    .lock()
                    .map(|value| *value)
                    .unwrap_or(true);
                if close_to_tray && !explicit_quit {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("failed to run SuperNova desktop shell");
}
