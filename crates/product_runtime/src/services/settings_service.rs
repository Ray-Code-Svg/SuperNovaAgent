use std::{fs, io};

use local_runtime_protocol::{
    AppSettings, AppearanceSettings, DataPathSettings, DisplayLanguage, DisplayTheme,
    ProviderApiRecord, ProviderApiSettings, ProviderApiTestRequest, ProviderApiTestResult,
    ProviderApiUpdateRequest,
};
use supernova_process_kernel::{ProviderProfileRecord, ProviderTestReceipt};

use crate::app_paths::AppPaths;
use crate::kernel::KernelBridge;

#[derive(Clone)]
pub struct SettingsService {
    app_paths: AppPaths,
    kernel: KernelBridge,
}

impl SettingsService {
    pub fn new(app_paths: AppPaths, kernel: KernelBridge) -> Self {
        Self { app_paths, kernel }
    }

    pub fn get(&self) -> io::Result<AppSettings> {
        Ok(AppSettings {
            provider_api: self.provider_settings()?,
            appearance: self.appearance_settings()?,
            data_paths: DataPathSettings {
                app_config_root: self.app_paths.app_config_root.to_string_lossy().to_string(),
                app_state_root: self.app_paths.app_state_root.to_string_lossy().to_string(),
                workspace_registry_path: self
                    .app_paths
                    .app_config_root
                    .join("config")
                    .join("workspace_registry.sqlite3")
                    .to_string_lossy()
                    .to_string(),
            },
        })
    }

    pub fn update(&self, request: AppSettings) -> io::Result<AppSettings> {
        self.save_appearance_settings(&request.appearance)?;
        self.get()
    }

    pub fn provider_settings(&self) -> io::Result<ProviderApiSettings> {
        Ok(ProviderApiSettings {
            providers: self
                .kernel
                .list_provider_profiles()?
                .into_iter()
                .map(provider_record)
                .collect(),
        })
    }

    pub fn update_provider(
        &self,
        request: ProviderApiUpdateRequest,
    ) -> io::Result<ProviderApiSettings> {
        if request.provider.trim().eq_ignore_ascii_case("deepseek")
            && request
                .api_base_url
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty())
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "DeepSeek API base URL is fixed and cannot be changed.",
            ));
        }
        self.kernel
            .save_provider_profile(&request.provider, None, request.api_key)?;
        self.provider_settings()
    }

    pub fn test_provider(
        &self,
        request: ProviderApiTestRequest,
    ) -> io::Result<ProviderApiTestResult> {
        self.kernel
            .test_provider_profile(&request.provider, request.live_check.unwrap_or(true))
            .map(provider_test_result)
    }

    pub fn appearance_settings(&self) -> io::Result<AppearanceSettings> {
        let path = self.appearance_settings_path();
        if !path.exists() {
            return Ok(default_appearance_settings());
        }
        let raw = fs::read_to_string(path)?;
        let settings = serde_json::from_str::<AppearanceSettings>(&raw)
            .unwrap_or_else(|_| default_appearance_settings());
        Ok(settings)
    }

    fn save_appearance_settings(&self, settings: &AppearanceSettings) -> io::Result<()> {
        let path = self.appearance_settings_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let payload = serde_json::to_vec_pretty(settings)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        fs::write(path, payload)?;
        Ok(())
    }

    fn appearance_settings_path(&self) -> std::path::PathBuf {
        self.app_paths
            .app_config_root
            .join("config")
            .join("appearance_settings.json")
    }
}

fn default_appearance_settings() -> AppearanceSettings {
    AppearanceSettings {
        language: DisplayLanguage::EnUs,
        theme: DisplayTheme::Dark,
    }
}

fn provider_record(record: ProviderProfileRecord) -> ProviderApiRecord {
    ProviderApiRecord {
        provider: record.provider_id,
        api_base_url: record.api_base_url,
        api_key_configured: record.credential_ref.is_some(),
        credential_ref: record.credential_ref,
        validation_status: record.validation_status,
        token_usage_summary: None,
    }
}

fn provider_test_result(receipt: ProviderTestReceipt) -> ProviderApiTestResult {
    ProviderApiTestResult {
        provider: receipt.provider_id,
        status: receipt.status,
        message: receipt.message,
        api_base_url: receipt.api_base_url,
        api_key_configured: receipt.credential_ref.is_some() && receipt.credential_resolved,
        credential_ref: receipt.credential_ref,
        live_check_performed: receipt.live_check_performed,
        checked_by: receipt.checked_by,
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use super::*;
    use crate::kernel::KernelBridge;

    fn temp_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "supernova_product_settings_{name}_{}",
            now_ms_for_test()
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn test_service(name: &str) -> SettingsService {
        let root = temp_root(name);
        let workspace = root.join("workspace");
        let state = root.join("state");
        let config = root.join("config");
        fs::create_dir_all(&workspace).unwrap();
        fs::create_dir_all(&state).unwrap();
        fs::create_dir_all(&config).unwrap();
        let app_paths = AppPaths {
            app_config_root: config.clone(),
            app_state_root: state.clone(),
        };
        let kernel = KernelBridge::new(
            workspace,
            state.join("workspace_state"),
            config.join("kernel_provider_credentials"),
        );
        SettingsService::new(app_paths, kernel)
    }

    #[test]
    #[cfg(windows)]
    fn provider_update_uses_kernel_credential_ref_without_exposing_api_key() {
        let service = test_service("provider_update_kernel_ref");
        let settings = service
            .update_provider(ProviderApiUpdateRequest {
                provider: "deepseek".into(),
                api_base_url: None,
                api_key: Some("secret-key".into()),
            })
            .expect("provider settings should persist through Kernel store");
        let provider = settings
            .providers
            .iter()
            .find(|provider| provider.provider == "deepseek")
            .expect("deepseek provider record");
        assert!(provider.api_key_configured);
        assert!(provider.credential_ref.is_some());
        let profile_path = service
            .app_paths
            .app_config_root
            .join("kernel_provider_credentials")
            .join("provider_profiles.json");
        let raw_profiles = fs::read_to_string(profile_path).unwrap();
        assert!(!raw_profiles.contains("secret-key"));
        let old_settings_path = service
            .app_paths
            .app_config_root
            .join("config")
            .join("app_settings.json");
        assert!(!old_settings_path.exists());
    }

    #[test]
    #[cfg(windows)]
    fn provider_test_uses_kernel_credential_resolution() {
        let service = test_service("provider_test_kernel_ref");
        service
            .update_provider(ProviderApiUpdateRequest {
                provider: "deepseek".into(),
                api_base_url: None,
                api_key: Some("secret-key".into()),
            })
            .unwrap();
        let result = service
            .test_provider(ProviderApiTestRequest {
                provider: "deepseek".into(),
                live_check: Some(false),
            })
            .unwrap();
        assert_eq!(result.status, "credential_resolved");
        assert!(result.api_key_configured);
        assert!(result.credential_ref.is_some());
        assert!(!result.live_check_performed);
        assert_eq!(result.checked_by, "kernel_provider_credential_store");
    }

    #[test]
    fn provider_update_rejects_deepseek_base_url_changes() {
        let service = test_service("provider_update_rejects_base_url");
        let error = service
            .update_provider(ProviderApiUpdateRequest {
                provider: "deepseek".into(),
                api_base_url: Some("https://attacker.example".into()),
                api_key: None,
            })
            .expect_err("DeepSeek base URL must not be writable");
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn appearance_settings_default_and_persist() {
        let service = test_service("appearance_settings");
        let initial = service.get().unwrap();
        assert_eq!(initial.appearance.language, DisplayLanguage::EnUs);
        assert_eq!(initial.appearance.theme, DisplayTheme::Dark);

        let mut request = initial;
        request.appearance = AppearanceSettings {
            language: DisplayLanguage::ZhCn,
            theme: DisplayTheme::Dark,
        };

        let updated = service.update(request).unwrap();
        assert_eq!(updated.appearance.language, DisplayLanguage::ZhCn);
        assert_eq!(updated.appearance.theme, DisplayTheme::Dark);

        let reloaded = service.get().unwrap();
        assert_eq!(reloaded.appearance, updated.appearance);
    }

    fn now_ms_for_test() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis()
    }
}
