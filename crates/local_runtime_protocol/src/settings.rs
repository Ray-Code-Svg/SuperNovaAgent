use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct AppSettings {
    pub provider_api: ProviderApiSettings,
    pub data_paths: DataPathSettings,
    pub appearance: AppearanceSettings,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct AppearanceSettings {
    pub language: DisplayLanguage,
    pub theme: DisplayTheme,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub enum DisplayLanguage {
    #[serde(rename = "zh-CN")]
    ZhCn,
    #[serde(rename = "en-US")]
    EnUs,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DisplayTheme {
    Light,
    Dark,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct ProviderApiSettings {
    pub providers: Vec<ProviderApiRecord>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct ProviderApiRecord {
    pub provider: String,
    pub api_base_url: Option<String>,
    pub api_key_configured: bool,
    pub credential_ref: Option<String>,
    pub validation_status: Option<String>,
    pub token_usage_summary: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct ProviderApiUpdateRequest {
    pub provider: String,
    pub api_base_url: Option<String>,
    pub api_key: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct ProviderApiTestRequest {
    pub provider: String,
    pub live_check: Option<bool>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct ProviderApiTestResult {
    pub provider: String,
    pub status: String,
    pub message: String,
    pub api_base_url: Option<String>,
    pub api_key_configured: bool,
    pub credential_ref: Option<String>,
    pub live_check_performed: bool,
    pub checked_by: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct DataPathSettings {
    pub app_config_root: String,
    pub app_state_root: String,
    pub workspace_registry_path: String,
}
