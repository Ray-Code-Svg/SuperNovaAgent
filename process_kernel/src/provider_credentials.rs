use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::deepseek_provider::{
    deepseek_base_url_default, deepseek_provider_from_resolved_credential,
    default_model_provider_from_env, DEEPSEEK_OFFICIAL_BASE_URL,
};
use crate::model_runtime::{MissingModelProvider, ModelProvider};

const PROVIDER_PROFILE_SCHEMA_VERSION: &str = "supernova_provider_profiles.v1";
const PROVIDER_CREDENTIAL_REF_PREFIX: &str = "kernel_credential://provider/";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderProfileRecord {
    pub provider_id: String,
    pub api_base_url: Option<String>,
    pub credential_ref: Option<String>,
    pub enabled: bool,
    pub created_at_unix_ms: u128,
    pub updated_at_unix_ms: u128,
    pub validation_status: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderTestReceipt {
    pub provider_id: String,
    pub status: String,
    pub message: String,
    pub api_base_url: Option<String>,
    pub credential_ref: Option<String>,
    pub credential_resolved: bool,
    pub live_check_performed: bool,
    pub checked_by: String,
}

#[derive(Clone, Debug)]
pub struct ProviderCredentialStore {
    root: PathBuf,
}

impl ProviderCredentialStore {
    pub fn new(root: impl AsRef<Path>) -> Self {
        Self {
            root: root.as_ref().to_path_buf(),
        }
    }

    pub fn list_profiles(&self) -> io::Result<Vec<ProviderProfileRecord>> {
        let mut store = self.load_store()?;
        ensure_default_profiles(&mut store);
        Ok(store.providers.into_values().collect())
    }

    pub fn read_profile(&self, provider_id: &str) -> io::Result<Option<ProviderProfileRecord>> {
        let mut store = self.load_store()?;
        ensure_default_profiles(&mut store);
        Ok(store
            .providers
            .get(&normalize_provider_id(provider_id))
            .cloned())
    }

    pub fn save_provider_profile(
        &self,
        provider_id: &str,
        api_base_url: Option<String>,
        api_key: Option<String>,
    ) -> io::Result<ProviderProfileRecord> {
        let provider_id = normalize_provider_id(provider_id);
        if provider_id == "deepseek"
            && api_base_url
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty())
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "DeepSeek API base URL is fixed and cannot be changed.",
            ));
        }
        let mut store = self.load_store()?;
        ensure_default_profiles(&mut store);
        let now = crate::now_ms();
        let mut record =
            store
                .providers
                .remove(&provider_id)
                .unwrap_or_else(|| ProviderProfileRecord {
                    provider_id: provider_id.clone(),
                    api_base_url: None,
                    credential_ref: None,
                    enabled: true,
                    created_at_unix_ms: now,
                    updated_at_unix_ms: now,
                    validation_status: None,
                });
        if provider_id == "deepseek" {
            record.api_base_url = Some(deepseek_base_url_default());
        }
        if let Some(base_url) = api_base_url {
            record.api_base_url = optional_trimmed(base_url);
        }
        if let Some(key) = api_key {
            let trimmed = key.trim().to_string();
            if trimmed.is_empty() {
                if let Some(old_ref) = record.credential_ref.take() {
                    self.delete_secret(&old_ref)?;
                }
                record.validation_status = Some("credential_cleared".to_string());
            } else {
                if let Some(old_ref) = record.credential_ref.take() {
                    self.delete_secret(&old_ref)?;
                }
                let credential_ref = self.write_secret(&provider_id, &trimmed)?;
                record.credential_ref = Some(credential_ref);
                record.validation_status = Some("credential_stored".to_string());
            }
        }
        record.updated_at_unix_ms = now;
        store.providers.insert(provider_id, record.clone());
        self.save_store(&store)?;
        Ok(record)
    }

    pub fn delete_provider_profile(&self, provider_id: &str) -> io::Result<()> {
        let provider_id = normalize_provider_id(provider_id);
        let mut store = self.load_store()?;
        if let Some(record) = store.providers.remove(&provider_id) {
            if let Some(credential_ref) = record.credential_ref {
                self.delete_secret(&credential_ref)?;
            }
        }
        ensure_default_profiles(&mut store);
        self.save_store(&store)
    }

    pub fn resolve_secret(&self, credential_ref: &str) -> io::Result<String> {
        let encrypted = fs::read(self.secret_path(credential_ref)?)?;
        unprotect_secret(&encrypted)
    }

    pub fn test_provider(
        &self,
        provider_id: &str,
        live_check: bool,
    ) -> io::Result<ProviderTestReceipt> {
        let provider_id = normalize_provider_id(provider_id);
        let profile = self.read_profile(&provider_id)?;
        let Some(profile) = profile else {
            return Ok(ProviderTestReceipt {
                provider_id,
                status: "missing_provider".to_string(),
                message: "Provider profile is not configured in the Kernel credential store."
                    .to_string(),
                api_base_url: None,
                credential_ref: None,
                credential_resolved: false,
                live_check_performed: false,
                checked_by: "kernel_provider_credential_store".to_string(),
            });
        };
        let api_base_url = effective_base_url(&profile);
        let Some(credential_ref) = profile.credential_ref.clone() else {
            return Ok(ProviderTestReceipt {
                provider_id,
                status: "missing_api_key".to_string(),
                message: "Provider API key is not configured in the Kernel credential store."
                    .to_string(),
                api_base_url,
                credential_ref: None,
                credential_resolved: false,
                live_check_performed: false,
                checked_by: "kernel_provider_credential_store".to_string(),
            });
        };
        let secret = match self.resolve_secret(&credential_ref) {
            Ok(secret) if !secret.trim().is_empty() => secret,
            Ok(_) => {
                return Ok(ProviderTestReceipt {
                    provider_id,
                    status: "missing_api_key".to_string(),
                    message: "Resolved provider API key is empty.".to_string(),
                    api_base_url,
                    credential_ref: Some(credential_ref),
                    credential_resolved: false,
                    live_check_performed: false,
                    checked_by: "kernel_provider_credential_store".to_string(),
                })
            }
            Err(err) => {
                return Ok(ProviderTestReceipt {
                    provider_id,
                    status: "credential_resolution_failed".to_string(),
                    message: format!("Kernel credential resolution failed: {err}"),
                    api_base_url,
                    credential_ref: Some(credential_ref),
                    credential_resolved: false,
                    live_check_performed: false,
                    checked_by: "kernel_provider_credential_store".to_string(),
                })
            }
        };
        if !live_check {
            return Ok(ProviderTestReceipt {
                provider_id,
                status: "credential_resolved".to_string(),
                message: "Kernel credential_ref resolved successfully; live provider check was not requested."
                    .to_string(),
                api_base_url,
                credential_ref: Some(credential_ref),
                credential_resolved: true,
                live_check_performed: false,
                checked_by: "kernel_provider_credential_store".to_string(),
            });
        }
        let live = live_provider_check(&provider_id, api_base_url.as_deref(), &secret);
        Ok(ProviderTestReceipt {
            provider_id,
            status: live.0,
            message: live.1,
            api_base_url,
            credential_ref: Some(credential_ref),
            credential_resolved: true,
            live_check_performed: true,
            checked_by: "kernel_provider_credential_store+provider_http_check".to_string(),
        })
    }

    pub fn model_provider_or_env(&self) -> Arc<dyn ModelProvider> {
        if std::env::var("SUPERNOVA_DETERMINISTIC_PROVIDER_JSON").is_ok() {
            return default_model_provider_from_env();
        }
        match self.read_profile("deepseek") {
            Ok(Some(profile)) if profile.enabled && profile.credential_ref.is_some() => {
                let credential_ref = profile.credential_ref.clone().unwrap_or_default();
                match self.resolve_secret(&credential_ref) {
                    Ok(secret) if !secret.trim().is_empty() => {
                        Arc::new(deepseek_provider_from_resolved_credential(
                            secret,
                            effective_base_url(&profile).unwrap_or_else(deepseek_base_url_default),
                        ))
                    }
                    Ok(_) => Arc::new(MissingModelProvider::new(
                        "deepseek",
                        "credential-empty",
                        "Kernel provider credential_ref resolved to an empty secret",
                    )),
                    Err(err) => Arc::new(MissingModelProvider::new(
                        "deepseek",
                        "credential-resolution-failed",
                        format!("Kernel provider credential_ref could not be resolved: {err}"),
                    )),
                }
            }
            _ => default_model_provider_from_env(),
        }
    }

    fn profiles_path(&self) -> PathBuf {
        self.root.join("provider_profiles.json")
    }

    fn secrets_dir(&self) -> PathBuf {
        self.root.join("secrets")
    }

    fn load_store(&self) -> io::Result<ProviderProfileStore> {
        let path = self.profiles_path();
        if !path.exists() {
            return Ok(ProviderProfileStore::default());
        }
        let raw = fs::read_to_string(path)?;
        serde_json::from_str::<ProviderProfileStore>(&raw)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }

    fn save_store(&self, store: &ProviderProfileStore) -> io::Result<()> {
        fs::create_dir_all(&self.root)?;
        let payload = serde_json::to_vec_pretty(store)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        fs::write(self.profiles_path(), payload)
    }

    fn write_secret(&self, provider_id: &str, secret: &str) -> io::Result<String> {
        fs::create_dir_all(self.secrets_dir())?;
        let credential_ref = format!(
            "{PROVIDER_CREDENTIAL_REF_PREFIX}{}/{}",
            provider_id,
            crate::now_ms()
        );
        let protected = protect_secret(secret)?;
        fs::write(self.secret_path(&credential_ref)?, protected)?;
        Ok(credential_ref)
    }

    fn delete_secret(&self, credential_ref: &str) -> io::Result<()> {
        let path = self.secret_path(credential_ref)?;
        if path.exists() {
            fs::remove_file(path)?;
        }
        Ok(())
    }

    fn secret_path(&self, credential_ref: &str) -> io::Result<PathBuf> {
        if !credential_ref.starts_with(PROVIDER_CREDENTIAL_REF_PREFIX) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "provider credential_ref has unsupported scheme",
            ));
        }
        Ok(self.secrets_dir().join(format!(
            "{}.dpapi",
            sanitize_ref_for_filename(credential_ref)
        )))
    }
}

pub fn model_provider_from_profile_root_or_env(root: impl AsRef<Path>) -> Arc<dyn ModelProvider> {
    ProviderCredentialStore::new(root).model_provider_or_env()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct ProviderProfileStore {
    schema_version: String,
    #[serde(default)]
    providers: BTreeMap<String, ProviderProfileRecord>,
}

impl Default for ProviderProfileStore {
    fn default() -> Self {
        Self {
            schema_version: PROVIDER_PROFILE_SCHEMA_VERSION.to_string(),
            providers: BTreeMap::new(),
        }
    }
}

fn ensure_default_profiles(store: &mut ProviderProfileStore) {
    store
        .providers
        .entry("deepseek".to_string())
        .or_insert_with(|| {
            let now = crate::now_ms();
            ProviderProfileRecord {
                provider_id: "deepseek".to_string(),
                api_base_url: Some(deepseek_base_url_default()),
                credential_ref: None,
                enabled: true,
                created_at_unix_ms: now,
                updated_at_unix_ms: now,
                validation_status: None,
            }
        });
    if let Some(record) = store.providers.get_mut("deepseek") {
        record.api_base_url = Some(deepseek_base_url_default());
    }
}

fn effective_base_url(profile: &ProviderProfileRecord) -> Option<String> {
    if profile.provider_id == "deepseek" {
        return Some(deepseek_base_url_default());
    }
    profile
        .api_base_url
        .as_ref()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn live_provider_check(
    provider_id: &str,
    api_base_url: Option<&str>,
    api_key: &str,
) -> (String, String) {
    if provider_id != "deepseek" {
        return (
            "unsupported_provider".to_string(),
            "Live provider check is only implemented for deepseek.".to_string(),
        );
    }
    let Some(base_url) = api_base_url else {
        return (
            "invalid_base_url".to_string(),
            "Provider API base URL is missing.".to_string(),
        );
    };
    let normalized_base_url = base_url.trim().trim_end_matches('/');
    if normalized_base_url != DEEPSEEK_OFFICIAL_BASE_URL {
        return (
            "invalid_base_url".to_string(),
            "DeepSeek live checks only allow the official provider endpoint.".to_string(),
        );
    }
    let url = if normalized_base_url.ends_with("/models") {
        normalized_base_url.to_string()
    } else {
        format!("{normalized_base_url}/models")
    };
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(15))
        .build();
    match agent
        .get(&url)
        .set("Authorization", &format!("Bearer {api_key}"))
        .set("Accept", "application/json")
        .call()
    {
        Ok(_) => (
            "ready".to_string(),
            "Provider credential resolved and live provider check succeeded.".to_string(),
        ),
        Err(ureq::Error::Status(code, response)) => {
            let body = response
                .into_string()
                .unwrap_or_else(|_| "<unreadable response body>".to_string());
            (
                format!("provider_http_{code}"),
                format!("Provider credential resolved, but live provider check returned HTTP {code}: {body}"),
            )
        }
        Err(ureq::Error::Transport(err)) => (
            "provider_transport_error".to_string(),
            format!("Provider credential resolved, but live provider check failed: {err}"),
        ),
    }
}

fn normalize_provider_id(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        "deepseek".to_string()
    } else {
        trimmed.to_ascii_lowercase()
    }
}

fn optional_trimmed(value: String) -> Option<String> {
    let trimmed = value.trim().to_string();
    (!trimmed.is_empty()).then_some(trimmed)
}

fn sanitize_ref_for_filename(value: &str) -> String {
    value
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect()
}

#[cfg(windows)]
fn protect_secret(secret: &str) -> io::Result<Vec<u8>> {
    dpapi::protect(secret.as_bytes())
}

#[cfg(windows)]
fn unprotect_secret(bytes: &[u8]) -> io::Result<String> {
    let unprotected = dpapi::unprotect(bytes)?;
    String::from_utf8(unprotected).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
}

#[cfg(not(windows))]
fn protect_secret(_secret: &str) -> io::Result<Vec<u8>> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "Kernel provider secure-secret storage requires Windows DPAPI in the commercial desktop path",
    ))
}

#[cfg(not(windows))]
fn unprotect_secret(_bytes: &[u8]) -> io::Result<String> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "Kernel provider secure-secret storage requires Windows DPAPI in the commercial desktop path",
    ))
}

#[cfg(windows)]
mod dpapi {
    use std::ffi::c_void;
    use std::io;
    use std::ptr;
    use std::slice;

    #[repr(C)]
    struct DataBlob {
        cb_data: u32,
        pb_data: *mut u8,
    }

    #[link(name = "Crypt32")]
    extern "system" {
        fn CryptProtectData(
            pDataIn: *mut DataBlob,
            szDataDescr: *const u16,
            pOptionalEntropy: *mut DataBlob,
            pvReserved: *mut c_void,
            pPromptStruct: *mut c_void,
            dwFlags: u32,
            pDataOut: *mut DataBlob,
        ) -> i32;

        fn CryptUnprotectData(
            pDataIn: *mut DataBlob,
            ppszDataDescr: *mut *mut u16,
            pOptionalEntropy: *mut DataBlob,
            pvReserved: *mut c_void,
            pPromptStruct: *mut c_void,
            dwFlags: u32,
            pDataOut: *mut DataBlob,
        ) -> i32;
    }

    #[link(name = "Kernel32")]
    extern "system" {
        fn LocalFree(hMem: *mut c_void) -> *mut c_void;
    }

    pub fn protect(bytes: &[u8]) -> io::Result<Vec<u8>> {
        let mut input = DataBlob {
            cb_data: bytes.len() as u32,
            pb_data: bytes.as_ptr() as *mut u8,
        };
        let mut output = DataBlob {
            cb_data: 0,
            pb_data: ptr::null_mut(),
        };
        let ok = unsafe {
            CryptProtectData(
                &mut input,
                ptr::null(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                0,
                &mut output,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        copy_and_free(output)
    }

    pub fn unprotect(bytes: &[u8]) -> io::Result<Vec<u8>> {
        let mut input = DataBlob {
            cb_data: bytes.len() as u32,
            pb_data: bytes.as_ptr() as *mut u8,
        };
        let mut output = DataBlob {
            cb_data: 0,
            pb_data: ptr::null_mut(),
        };
        let ok = unsafe {
            CryptUnprotectData(
                &mut input,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                0,
                &mut output,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        copy_and_free(output)
    }

    fn copy_and_free(blob: DataBlob) -> io::Result<Vec<u8>> {
        if blob.pb_data.is_null() {
            return Ok(Vec::new());
        }
        let bytes = unsafe { slice::from_raw_parts(blob.pb_data, blob.cb_data as usize).to_vec() };
        unsafe {
            LocalFree(blob.pb_data as *mut c_void);
        }
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "supernova_provider_credentials_{name}_{}",
            crate::now_ms()
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    #[cfg(windows)]
    fn provider_profile_stores_secret_as_credential_ref() {
        let store = ProviderCredentialStore::new(temp_root("stores_secret"));
        let profile = store
            .save_provider_profile("deepseek", None, Some("secret-key".to_string()))
            .unwrap();
        let credential_ref = profile.credential_ref.expect("credential ref");
        assert!(credential_ref.starts_with(PROVIDER_CREDENTIAL_REF_PREFIX));
        assert_eq!(store.resolve_secret(&credential_ref).unwrap(), "secret-key");
        let raw_profiles = fs::read_to_string(store.profiles_path()).unwrap();
        assert!(!raw_profiles.contains("secret-key"));
    }

    #[test]
    fn deepseek_profile_rejects_caller_supplied_base_url() {
        let store = ProviderCredentialStore::new(temp_root("rejects_base_url"));
        let error = store
            .save_provider_profile(
                "deepseek",
                Some("https://attacker.example".to_string()),
                None,
            )
            .expect_err("DeepSeek base URL must not be writable");
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn deepseek_effective_base_url_ignores_stored_malicious_url() {
        let profile = ProviderProfileRecord {
            provider_id: "deepseek".to_string(),
            api_base_url: Some("https://attacker.example".to_string()),
            credential_ref: None,
            enabled: true,
            created_at_unix_ms: 1,
            updated_at_unix_ms: 1,
            validation_status: None,
        };

        assert_eq!(
            effective_base_url(&profile),
            Some(DEEPSEEK_OFFICIAL_BASE_URL.to_string())
        );
    }

    #[test]
    fn deepseek_live_check_rejects_non_official_endpoint() {
        let (status, message) =
            live_provider_check("deepseek", Some("https://attacker.example"), "secret-key");

        assert_eq!(status, "invalid_base_url");
        assert!(message.contains("official provider endpoint"));
    }

    #[test]
    #[cfg(windows)]
    fn provider_test_can_resolve_without_live_check() {
        let store = ProviderCredentialStore::new(temp_root("test_resolve"));
        store
            .save_provider_profile("deepseek", None, Some("secret-key".to_string()))
            .unwrap();
        let receipt = store.test_provider("deepseek", false).unwrap();
        assert_eq!(receipt.status, "credential_resolved");
        assert!(receipt.credential_resolved);
        assert!(!receipt.live_check_performed);
        assert!(receipt.credential_ref.is_some());
    }
}
