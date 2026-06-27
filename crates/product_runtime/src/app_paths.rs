use std::env;
use std::path::{Path, PathBuf};

pub const APP_NAME: &str = "SuperNova";
pub const APP_HOME_WORKSPACE_ID: &str = "ws_app_home";

#[derive(Clone, Debug)]
pub struct AppPaths {
    pub app_config_root: PathBuf,
    pub app_state_root: PathBuf,
}

impl AppPaths {
    pub fn resolve(config_override: Option<PathBuf>, state_override: Option<PathBuf>) -> Self {
        let app_config_root = config_override.unwrap_or_else(default_config_root);
        let app_state_root = state_override.unwrap_or_else(default_state_root);
        Self {
            app_config_root,
            app_state_root,
        }
    }

    pub fn workspace_state_root(&self, workspace_uid: &str) -> PathBuf {
        self.app_state_root
            .join("state")
            .join("workspaces")
            .join(workspace_uid)
    }

    pub fn app_home_state_root(&self) -> PathBuf {
        self.app_state_root.join("state").join("app_home")
    }
}

pub fn workspace_uid(workspace_root: impl AsRef<Path>) -> String {
    let normalized = workspace_root
        .as_ref()
        .to_string_lossy()
        .replace('\\', "/")
        .to_lowercase();
    format!("ws_{}", fnv1a64(&normalized))
}

fn default_config_root() -> PathBuf {
    if let Ok(value) = env::var("SUPERNOVA_APP_CONFIG_ROOT") {
        return PathBuf::from(value);
    }
    env::var("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
        .join(APP_NAME)
}

fn default_state_root() -> PathBuf {
    if let Ok(value) = env::var("SUPERNOVA_APP_STATE_ROOT") {
        return PathBuf::from(value);
    }
    env::var("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
        .join(APP_NAME)
}

fn fnv1a64(value: &str) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}
