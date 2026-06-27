use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::net::{TcpListener, ToSocketAddrs};
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{
    child_process::suppress_child_window, json_err, now_ms, safe_blob_name, to_json_value,
    CapabilityReceipt, CapabilityToken, ProcessTruthStore, WorkspaceGuard,
};

pub const CLIENT_ENV_SNAPSHOT_SCHEMA_VERSION: &str = "supernova_client_env_snapshot.v1";
pub const CLIENT_LOCALE_CONTEXT_SCHEMA_VERSION: &str = "supernova_client_locale_context.v1";
pub const CLIENT_ENV_ORIGIN: &str = "kernel_host_local";

const DEFAULT_DETAIL_LEVEL: &str = "summary";
const DEFAULT_MAX_ITEMS: u32 = 100;
const DISCLOSURE_TTL_MS: u128 = 10 * 60 * 1000;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClientLocaleContext {
    pub schema_version: String,
    pub captured_at_unix_ms: u128,
    pub origin: String,
    pub os_family: String,
    pub timezone_id: Option<String>,
    pub utc_offset_minutes: Option<i32>,
    pub locale: Option<String>,
    pub current_local_datetime: String,
    pub sensitivity: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClientEnvScanOptions {
    #[serde(default)]
    pub sections: Vec<String>,
    #[serde(default = "default_detail_level")]
    pub detail_level: String,
    #[serde(default)]
    pub include_sensitive_fields: bool,
    #[serde(default)]
    pub authorization_id: Option<String>,
    #[serde(default)]
    pub max_items: Option<u32>,
    #[serde(default)]
    pub reason: Option<String>,
}

impl Default for ClientEnvScanOptions {
    fn default() -> Self {
        Self {
            sections: Vec::new(),
            detail_level: DEFAULT_DETAIL_LEVEL.to_string(),
            include_sensitive_fields: false,
            authorization_id: None,
            max_items: Some(DEFAULT_MAX_ITEMS),
            reason: None,
        }
    }
}

impl ClientEnvScanOptions {
    pub fn from_value(value: &Value) -> io::Result<Self> {
        let mut options = if value.is_null() {
            Self::default()
        } else {
            serde_json::from_value::<Self>(value.clone()).map_err(json_err)?
        };
        if options.detail_level.trim().is_empty() {
            options.detail_level = DEFAULT_DETAIL_LEVEL.to_string();
        }
        options.detail_level = match options.detail_level.as_str() {
            "summary" | "standard" => options.detail_level,
            _ => DEFAULT_DETAIL_LEVEL.to_string(),
        };
        options.sections = options
            .sections
            .into_iter()
            .filter_map(|item| normalize_section_id(&item))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        options.max_items = Some(
            options
                .max_items
                .unwrap_or(DEFAULT_MAX_ITEMS)
                .clamp(1, 1000),
        );
        Ok(options)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ClientEnvSnapshot {
    pub schema_version: String,
    pub snapshot_id: String,
    pub captured_at_unix_ms: u128,
    pub origin: String,
    pub sections: Vec<ClientEnvSection>,
    pub redaction: ClientEnvRedactionReport,
    pub sensitive_fields_available: Vec<String>,
    pub sensitive_fields_returned: Vec<String>,
    pub unsupported_fields: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ClientEnvSection {
    pub section_id: String,
    pub status: String,
    pub facts: Value,
    pub unavailable_fields: Vec<String>,
    pub collector_warnings: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClientEnvRedactionReport {
    pub policy: String,
    pub sensitive_fields_redacted: bool,
    pub redacted_field_count: u32,
    pub requires_authorization_for: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClientEnvDisclosureRequest {
    pub request_id: String,
    pub requested_fields: Vec<String>,
    pub reason: String,
    pub created_at_unix_ms: u128,
    pub expires_at_unix_ms: u128,
    pub status: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClientEnvDisclosureToken {
    pub authorization_id: String,
    pub request_id: String,
    pub allowed_fields: Vec<String>,
    pub expires_at_unix_ms: u128,
    pub user_approved: bool,
}

#[derive(Clone, Debug)]
pub struct ClientEnvRuntime {
    guard: WorkspaceGuard,
    truth: ProcessTruthStore,
    token: CapabilityToken,
    emit_capability_receipt_event: bool,
}

impl ClientEnvRuntime {
    pub fn new(guard: WorkspaceGuard, truth: ProcessTruthStore, token: CapabilityToken) -> Self {
        Self {
            guard,
            truth,
            token,
            emit_capability_receipt_event: true,
        }
    }

    pub fn without_process_truth_events(mut self) -> Self {
        self.emit_capability_receipt_event = false;
        self
    }

    pub fn capture_locale_context() -> ClientLocaleContext {
        let captured_at_unix_ms = now_ms();
        let os_family = os_family();
        let locale = capture_locale();
        let timezone_id = capture_timezone_id();
        let current_local_datetime = capture_current_local_datetime()
            .unwrap_or_else(|| unix_ms_as_utc_like(captured_at_unix_ms));
        let utc_offset_minutes = capture_utc_offset_minutes();
        ClientLocaleContext {
            schema_version: CLIENT_LOCALE_CONTEXT_SCHEMA_VERSION.to_string(),
            captured_at_unix_ms,
            origin: CLIENT_ENV_ORIGIN.to_string(),
            os_family,
            timezone_id,
            utc_offset_minutes,
            locale,
            current_local_datetime,
            sensitivity: "low".to_string(),
        }
    }

    pub fn scan_overview(&self, options: ClientEnvScanOptions) -> io::Result<CapabilityReceipt> {
        let mut normalized = options;
        if normalized.sections.is_empty() {
            normalized.sections = vec![
                "device".to_string(),
                "storage".to_string(),
                "network".to_string(),
                "runtimes".to_string(),
            ];
        }
        self.scan("client_env.scan_overview", normalized)
    }

    pub fn scan_device(&self, options: ClientEnvScanOptions) -> io::Result<CapabilityReceipt> {
        self.scan(
            "client_env.scan_device",
            with_forced_section(options, "device"),
        )
    }

    pub fn scan_storage(&self, options: ClientEnvScanOptions) -> io::Result<CapabilityReceipt> {
        self.scan(
            "client_env.scan_storage",
            with_forced_section(options, "storage"),
        )
    }

    pub fn scan_network(&self, options: ClientEnvScanOptions) -> io::Result<CapabilityReceipt> {
        self.scan(
            "client_env.scan_network",
            with_forced_section(options, "network"),
        )
    }

    pub fn scan_runtimes(&self, options: ClientEnvScanOptions) -> io::Result<CapabilityReceipt> {
        self.scan(
            "client_env.scan_runtimes",
            with_forced_section(options, "runtimes"),
        )
    }

    pub fn read_snapshot(
        &self,
        snapshot_ref: &str,
        offset: usize,
        limit: usize,
    ) -> io::Result<CapabilityReceipt> {
        let path = self.truth.resolve_blob_ref(snapshot_ref)?;
        let snapshot =
            serde_json::from_slice::<ClientEnvSnapshot>(&fs::read(path)?).map_err(json_err)?;
        let limit = limit.clamp(1, 50);
        let total = snapshot.sections.len();
        let sections = snapshot
            .sections
            .iter()
            .skip(offset)
            .take(limit)
            .cloned()
            .collect::<Vec<_>>();
        self.process_capability_receipt(
            "client_env.read_snapshot",
            "success",
            json!({
                "schema_version": CLIENT_ENV_SNAPSHOT_SCHEMA_VERSION,
                "snapshot_ref": snapshot_ref,
                "snapshot_id": snapshot.snapshot_id,
                "offset": offset,
                "limit": limit,
                "returned": sections.len(),
                "total": total,
                "sections": sections,
                "redaction": snapshot.redaction,
                "sensitive_fields_available": snapshot.sensitive_fields_available,
                "sensitive_fields_returned": snapshot.sensitive_fields_returned,
                "no_workspace_mutation": true,
            }),
        )
    }

    pub fn request_sensitive_disclosure(
        &self,
        requested_fields: Vec<String>,
        reason: impl Into<String>,
    ) -> io::Result<CapabilityReceipt> {
        let requested_fields = sanitize_requested_fields(requested_fields);
        let now = now_ms();
        let request = ClientEnvDisclosureRequest {
            request_id: format!("client_env_disclosure_{}_{}", self.token.pid, now),
            requested_fields: if requested_fields.is_empty() {
                sensitive_fields_for_sections(&["network".to_string()])
            } else {
                requested_fields
            },
            reason: {
                let reason = reason.into();
                if reason.trim().is_empty() {
                    "Client environment sensitive field disclosure requested.".to_string()
                } else {
                    reason
                }
            },
            created_at_unix_ms: now,
            expires_at_unix_ms: now.saturating_add(DISCLOSURE_TTL_MS),
            status: "pending".to_string(),
        };
        if self.emit_capability_receipt_event {
            self.truth.append_event(
                Some(&self.token.pid),
                "client_env_disclosure_requested",
                to_json_value(&request)?,
            )?;
        }
        self.process_capability_receipt(
            "client_env.request_sensitive_disclosure",
            "blocked",
            json!({
                "status": "blocked",
                "requires_explicit_user_authorization": true,
                "disclosure_request_id": request.request_id,
                "requested_fields": request.requested_fields,
                "reason": request.reason,
                "created_at_unix_ms": request.created_at_unix_ms,
                "expires_at_unix_ms": request.expires_at_unix_ms,
                "no_sensitive_values_returned": true,
                "no_workspace_mutation": true,
            }),
        )
    }

    pub fn approve_sensitive_disclosure(
        truth: &ProcessTruthStore,
        pid: &str,
        request_id: &str,
        allowed_fields: Vec<String>,
        note: &str,
    ) -> io::Result<ClientEnvDisclosureToken> {
        let now = now_ms();
        let request = pending_disclosure_request_for_approval(truth, request_id, now)?;
        let allowed_fields = sanitize_requested_fields(allowed_fields);
        if allowed_fields.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "client env disclosure approval requires at least one allowed field",
            ));
        }
        let requested_fields = request
            .requested_fields
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        let out_of_scope = allowed_fields
            .iter()
            .filter(|field| !requested_fields.contains(*field))
            .cloned()
            .collect::<Vec<_>>();
        if !out_of_scope.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "client env disclosure approval contains fields outside request scope: {}",
                    out_of_scope.join(",")
                ),
            ));
        }
        let token = ClientEnvDisclosureToken {
            authorization_id: format!("client_env_auth_{}_{}", safe_blob_name(request_id), now),
            request_id: request_id.to_string(),
            allowed_fields,
            expires_at_unix_ms: now.saturating_add(DISCLOSURE_TTL_MS),
            user_approved: true,
        };
        truth.append_event(
            Some(pid),
            "client_env_disclosure_approved",
            json!({
                "authorization_id": token.authorization_id,
                "request_id": token.request_id,
                "allowed_fields": token.allowed_fields,
                "expires_at_unix_ms": token.expires_at_unix_ms,
                "user_approved": true,
                "note": note,
            }),
        )?;
        Ok(token)
    }

    pub fn reject_sensitive_disclosure(
        truth: &ProcessTruthStore,
        pid: &str,
        request_id: &str,
        reason: &str,
    ) -> io::Result<()> {
        truth.append_event(
            Some(pid),
            "client_env_disclosure_rejected",
            json!({
                "request_id": request_id,
                "reason": reason,
                "user_approved": false,
            }),
        )?;
        Ok(())
    }

    fn scan(
        &self,
        capability_id: &str,
        options: ClientEnvScanOptions,
    ) -> io::Result<CapabilityReceipt> {
        let sections = if options.sections.is_empty() {
            vec!["device".to_string()]
        } else {
            options.sections.clone()
        };
        let requested_sensitive_fields = sensitive_fields_for_sections(&sections);
        let authorized_fields = if options.include_sensitive_fields {
            match self.authorized_fields(
                options.authorization_id.as_deref(),
                &requested_sensitive_fields,
            )? {
                Ok(fields) => fields,
                Err(block_data) => {
                    return self.process_capability_receipt(capability_id, "blocked", block_data);
                }
            }
        } else {
            BTreeSet::new()
        };

        let mut snapshot_sections = Vec::new();
        for section in &sections {
            match section.as_str() {
                "device" => snapshot_sections.push(self.collect_device_section()),
                "storage" => snapshot_sections.push(self.collect_storage_section()),
                "network" => {
                    snapshot_sections.push(self.collect_network_section(&authorized_fields))
                }
                "runtimes" => snapshot_sections.push(self.collect_runtimes_section()),
                other => snapshot_sections.push(ClientEnvSection {
                    section_id: other.to_string(),
                    status: "unavailable".to_string(),
                    facts: json!({}),
                    unavailable_fields: vec![other.to_string()],
                    collector_warnings: vec!["unsupported client environment section".to_string()],
                }),
            }
        }

        let sensitive_fields_returned = authorized_fields
            .iter()
            .filter(|field| requested_sensitive_fields.contains(*field))
            .cloned()
            .collect::<Vec<_>>();
        let redacted_count = requested_sensitive_fields
            .iter()
            .filter(|field| !sensitive_fields_returned.contains(*field))
            .count() as u32;
        let snapshot_id = format!("client_env_snapshot_{}_{}", self.token.pid, now_ms());
        let snapshot = ClientEnvSnapshot {
            schema_version: CLIENT_ENV_SNAPSHOT_SCHEMA_VERSION.to_string(),
            snapshot_id: snapshot_id.clone(),
            captured_at_unix_ms: now_ms(),
            origin: CLIENT_ENV_ORIGIN.to_string(),
            sections: snapshot_sections,
            redaction: ClientEnvRedactionReport {
                policy: "standard".to_string(),
                sensitive_fields_redacted: redacted_count > 0,
                redacted_field_count: redacted_count,
                requires_authorization_for: requested_sensitive_fields
                    .iter()
                    .filter(|field| !sensitive_fields_returned.contains(*field))
                    .cloned()
                    .collect(),
            },
            sensitive_fields_available: requested_sensitive_fields.clone(),
            sensitive_fields_returned,
            unsupported_fields: Vec::new(),
            warnings: Vec::new(),
        };
        let snapshot_ref = self.truth.write_blob(
            &format!(
                "client_env/{}_{}.json",
                safe_blob_name(&snapshot.snapshot_id),
                now_ms()
            ),
            &serde_json::to_vec_pretty(&snapshot).map_err(json_err)?,
        )?;
        if self.emit_capability_receipt_event {
            self.truth.append_event(
                Some(&self.token.pid),
                "client_env_snapshot_recorded",
                json!({
                    "capability_id": capability_id,
                    "snapshot_id": snapshot.snapshot_id,
                    "snapshot_ref": snapshot_ref,
                    "sections": sections,
                    "sensitive_fields_returned": snapshot.sensitive_fields_returned,
                    "redaction": snapshot.redaction,
                }),
            )?;
            if !snapshot.sensitive_fields_returned.is_empty() {
                self.truth.append_event(
                    Some(&self.token.pid),
                    "client_env_sensitive_fields_returned",
                    json!({
                        "capability_id": capability_id,
                        "snapshot_id": snapshot.snapshot_id,
                        "snapshot_ref": snapshot_ref,
                        "returned_fields": snapshot.sensitive_fields_returned,
                    }),
                )?;
            }
        }
        self.process_capability_receipt(
            capability_id,
            "success",
            json!({
                "schema_version": CLIENT_ENV_SNAPSHOT_SCHEMA_VERSION,
                "snapshot_id": snapshot_id,
                "snapshot_ref": snapshot_ref,
                "origin": CLIENT_ENV_ORIGIN,
                "sections": snapshot.sections,
                "redaction": snapshot.redaction,
                "sensitive_fields_available": snapshot.sensitive_fields_available,
                "sensitive_fields_returned": snapshot.sensitive_fields_returned,
                "no_workspace_mutation": true,
            }),
        )
    }

    fn authorized_fields(
        &self,
        authorization_id: Option<&str>,
        requested_fields: &[String],
    ) -> io::Result<Result<BTreeSet<String>, Value>> {
        let Some(authorization_id) = authorization_id.filter(|value| !value.trim().is_empty())
        else {
            return Ok(Err(self.sensitive_block_data(
                "missing_authorization_id",
                requested_fields,
                None,
            )));
        };
        let now = now_ms();
        let events = self.truth.read_events()?;
        let approval = events.iter().rev().find(|event| {
            event.event_type == "client_env_disclosure_approved"
                && event
                    .data
                    .get("authorization_id")
                    .and_then(Value::as_str)
                    .is_some_and(|value| value == authorization_id)
        });
        let Some(approval) = approval else {
            return Ok(Err(self.sensitive_block_data(
                "authorization_not_found",
                requested_fields,
                Some(authorization_id),
            )));
        };
        let expires_at = approval
            .data
            .get("expires_at_unix_ms")
            .and_then(Value::as_u64)
            .map(u128::from)
            .unwrap_or(0);
        if expires_at < now {
            return Ok(Err(self.sensitive_block_data(
                "authorization_expired",
                requested_fields,
                Some(authorization_id),
            )));
        }
        let allowed = approval
            .data
            .get("allowed_fields")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect::<BTreeSet<_>>()
            })
            .unwrap_or_default();
        let requested = requested_fields.iter().cloned().collect::<BTreeSet<_>>();
        let authorized = requested
            .intersection(&allowed)
            .cloned()
            .collect::<BTreeSet<_>>();
        if authorized.is_empty() {
            return Ok(Err(self.sensitive_block_data(
                "authorization_scope_does_not_cover_requested_fields",
                requested_fields,
                Some(authorization_id),
            )));
        }
        Ok(Ok(authorized))
    }

    fn sensitive_block_data(
        &self,
        reason: &str,
        requested_fields: &[String],
        authorization_id: Option<&str>,
    ) -> Value {
        json!({
            "status": "blocked",
            "reason": reason,
            "requires_explicit_user_authorization": true,
            "authorization_id": authorization_id,
            "requested_fields": requested_fields,
            "no_sensitive_values_returned": true,
            "no_workspace_mutation": true,
        })
    }

    fn collect_device_section(&self) -> ClientEnvSection {
        let mut facts = json!({
            "os_family": os_family(),
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
            "cpu_logical_cores": std::thread::available_parallelism().ok().map(|value| value.get()),
            "locale_context": Self::capture_locale_context(),
        });
        let mut unavailable_fields = Vec::new();
        let mut warnings = Vec::new();
        let memory_bucket = memory_total_bytes().map(bucket_bytes);
        if let Some(bucket) = memory_bucket {
            facts["memory_total_bucket"] = json!(bucket);
        } else {
            unavailable_fields.push("device.memory_total_bucket".to_string());
            warnings.push("total memory unavailable through safe collector".to_string());
        }
        ClientEnvSection {
            section_id: "device".to_string(),
            status: if unavailable_fields.is_empty() {
                "success".to_string()
            } else {
                "partial".to_string()
            },
            facts,
            unavailable_fields,
            collector_warnings: warnings,
        }
    }

    fn collect_storage_section(&self) -> ClientEnvSection {
        let mut facts = json!({
            "workspace_volume": "current_workspace_volume",
            "workspace_root_relation": "kernel_workspace_root",
        });
        let mut unavailable_fields = Vec::new();
        let mut warnings = Vec::new();
        if let Some((capacity, free)) = storage_capacity_for_workspace(self.guard.root()) {
            facts["capacity_bucket"] = json!(bucket_bytes(capacity));
            facts["free_space_bucket"] = json!(bucket_bytes(free));
            facts["filesystem_kind"] = json!("local_or_mounted");
        } else {
            unavailable_fields.extend([
                "storage.capacity_bucket".to_string(),
                "storage.free_space_bucket".to_string(),
            ]);
            warnings
                .push("workspace volume capacity unavailable through safe collector".to_string());
        }
        ClientEnvSection {
            section_id: "storage".to_string(),
            status: if unavailable_fields.is_empty() {
                "success".to_string()
            } else {
                "partial".to_string()
            },
            facts,
            unavailable_fields,
            collector_warnings: warnings,
        }
    }

    fn collect_network_section(&self, authorized_fields: &BTreeSet<String>) -> ClientEnvSection {
        let mut facts = json!({
            "loopback_bind_available": loopback_bind_available(),
            "localhost_resolution_available": localhost_resolution_available(),
            "sensitive_fields_redacted": true,
        });
        let mut warnings = Vec::new();
        if authorized_fields.contains("network.local_ip") {
            facts["local_ip"] = json!(collect_local_ips());
        }
        if authorized_fields.contains("network.mac_address") {
            facts["mac_address"] = json!(collect_mac_addresses());
        }
        if authorized_fields.contains("network.local_ip")
            || authorized_fields.contains("network.mac_address")
        {
            facts["sensitive_fields_redacted"] = json!(false);
            warnings
                .push("sensitive network fields returned under explicit authorization".to_string());
        }
        ClientEnvSection {
            section_id: "network".to_string(),
            status: "success".to_string(),
            facts,
            unavailable_fields: Vec::new(),
            collector_warnings: warnings,
        }
    }

    fn collect_runtimes_section(&self) -> ClientEnvSection {
        let runtime_specs = [
            ("python", "python", &["--version"][..]),
            ("python3", "python3", &["--version"][..]),
            ("node", "node", &["--version"][..]),
            ("npm", "npm", &["--version"][..]),
            ("rustc", "rustc", &["--version"][..]),
            ("cargo", "cargo", &["--version"][..]),
            ("dotnet", "dotnet", &["--version"][..]),
        ];
        let mut facts = serde_json::Map::new();
        for (runtime_id, program, args) in runtime_specs {
            facts.insert(runtime_id.to_string(), runtime_readiness(program, args));
        }
        facts.insert("office_worker".to_string(), office_worker_readiness());
        facts.insert(
            "kernel_cli".to_string(),
            json!({
                "available": true,
                "version": null,
                "readiness": "available",
                "source": "current_process_kernel_runtime",
            }),
        );
        ClientEnvSection {
            section_id: "runtimes".to_string(),
            status: "success".to_string(),
            facts: Value::Object(facts),
            unavailable_fields: Vec::new(),
            collector_warnings: Vec::new(),
        }
    }

    fn process_capability_receipt(
        &self,
        capability_id: &str,
        status: &str,
        data: Value,
    ) -> io::Result<CapabilityReceipt> {
        let receipt = CapabilityReceipt {
            capability_id: capability_id.to_string(),
            job_id: self.token.job_id.clone(),
            pid: self.token.pid.clone(),
            status: status.to_string(),
            data,
        };
        if self.emit_capability_receipt_event {
            self.truth.append_event(
                Some(&self.token.pid),
                "capability_receipt",
                to_json_value(&receipt)?,
            )?;
        }
        Ok(receipt)
    }
}

fn default_detail_level() -> String {
    DEFAULT_DETAIL_LEVEL.to_string()
}

fn with_forced_section(mut options: ClientEnvScanOptions, section: &str) -> ClientEnvScanOptions {
    options.sections = vec![section.to_string()];
    options
}

fn normalize_section_id(value: &str) -> Option<String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "device" | "storage" | "network" | "runtimes" => Some(value.trim().to_ascii_lowercase()),
        _ => None,
    }
}

fn sensitive_fields_for_sections(sections: &[String]) -> Vec<String> {
    let mut fields = BTreeSet::new();
    for section in sections {
        if section == "network" {
            fields.insert("network.local_ip".to_string());
            fields.insert("network.mac_address".to_string());
        }
    }
    fields.into_iter().collect()
}

fn sanitize_requested_fields(fields: Vec<String>) -> Vec<String> {
    let allowed = [
        "network.local_ip",
        "network.mac_address",
        "network.ssid",
        "network.proxy_url",
        "device.username",
        "device.hostname",
        "env.path",
        "env.vars",
    ]
    .into_iter()
    .collect::<BTreeSet<_>>();
    fields
        .into_iter()
        .map(|field| field.trim().to_ascii_lowercase())
        .filter(|field| allowed.contains(field.as_str()))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn pending_disclosure_request_for_approval(
    truth: &ProcessTruthStore,
    request_id: &str,
    now: u128,
) -> io::Result<ClientEnvDisclosureRequest> {
    if request_id.trim().is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "client env disclosure request_id is required",
        ));
    }
    let events = truth.read_events()?;
    let Some((request_index, request_event)) =
        events.iter().enumerate().rev().find(|(_, event)| {
            event.event_type == "client_env_disclosure_requested"
                && disclosure_event_request_id(&event.data) == Some(request_id)
        })
    else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("client env disclosure request not found: {request_id}"),
        ));
    };
    let request: ClientEnvDisclosureRequest =
        serde_json::from_value(request_event.data.clone()).map_err(json_err)?;
    if request.status != "pending" {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "client env disclosure request is not pending: {}",
                request.status
            ),
        ));
    }
    if request.expires_at_unix_ms < now {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "client env disclosure request expired",
        ));
    }
    if let Some(resolved) = events.iter().skip(request_index + 1).rev().find(|event| {
        matches!(
            event.event_type.as_str(),
            "client_env_disclosure_approved" | "client_env_disclosure_rejected"
        ) && disclosure_event_request_id(&event.data) == Some(request_id)
    }) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "client env disclosure request already resolved by {}",
                resolved.event_type
            ),
        ));
    }
    Ok(request)
}

fn disclosure_event_request_id(data: &Value) -> Option<&str> {
    data.get("request_id")
        .and_then(Value::as_str)
        .or_else(|| data.get("disclosure_request_id").and_then(Value::as_str))
}

fn os_family() -> String {
    match std::env::consts::OS {
        "windows" => "windows",
        "macos" => "macos",
        "linux" => "linux",
        _ => "unknown",
    }
    .to_string()
}

fn capture_locale() -> Option<String> {
    #[cfg(windows)]
    {
        if let Some(value) = run_fixed_command(
            "powershell.exe",
            &[
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "[System.Globalization.CultureInfo]::CurrentCulture.Name",
            ],
            1500,
            256,
        ) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    for key in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Ok(value) = std::env::var(key) {
            let normalized = value.split('.').next().unwrap_or(&value).replace('_', "-");
            if !normalized.trim().is_empty() && normalized != "C" {
                return Some(normalized);
            }
        }
    }
    None
}

fn capture_timezone_id() -> Option<String> {
    if let Ok(value) = std::env::var("TZ") {
        if !value.trim().is_empty() {
            return Some(value);
        }
    }
    #[cfg(windows)]
    {
        if let Some(value) = run_fixed_command("tzutil.exe", &["/g"], 1500, 256) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    #[cfg(not(windows))]
    {
        if let Ok(target) = fs::read_link("/etc/localtime") {
            let text = target.display().to_string();
            if let Some((_, zone)) = text.split_once("zoneinfo/") {
                if !zone.trim().is_empty() {
                    return Some(zone.to_string());
                }
            }
        }
        if let Some(value) = run_fixed_command("date", &["+%Z"], 1500, 128) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

fn capture_current_local_datetime() -> Option<String> {
    #[cfg(windows)]
    {
        if let Some(value) = run_fixed_command(
            "powershell.exe",
            &[
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Get-Date -Format o",
            ],
            1500,
            128,
        ) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    #[cfg(not(windows))]
    {
        if let Some(value) = run_fixed_command("date", &["+%Y-%m-%dT%H:%M:%S%z"], 1500, 128) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

fn capture_utc_offset_minutes() -> Option<i32> {
    #[cfg(windows)]
    {
        if let Some(value) = run_fixed_command(
            "powershell.exe",
            &[
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "[int]([TimeZoneInfo]::Local.GetUtcOffset((Get-Date)).TotalMinutes)",
            ],
            1500,
            64,
        ) {
            return value.trim().parse::<i32>().ok();
        }
    }
    #[cfg(not(windows))]
    {
        if let Some(value) = run_fixed_command("date", &["+%z"], 1500, 64) {
            return parse_offset(value.trim());
        }
    }
    None
}

#[cfg(not(windows))]
fn parse_offset(value: &str) -> Option<i32> {
    if value.len() != 5 {
        return None;
    }
    let sign = match &value[0..1] {
        "+" => 1,
        "-" => -1,
        _ => return None,
    };
    let hours = value[1..3].parse::<i32>().ok()?;
    let minutes = value[3..5].parse::<i32>().ok()?;
    Some(sign * (hours * 60 + minutes))
}

fn unix_ms_as_utc_like(ms: u128) -> String {
    format!("{ms}ms_since_unix_epoch")
}

fn memory_total_bytes() -> Option<u64> {
    #[cfg(windows)]
    {
        let output = run_fixed_command(
            "powershell.exe",
            &[
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "(Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory",
            ],
            2000,
            128,
        )?;
        return output.trim().parse::<u64>().ok();
    }
    #[cfg(target_os = "linux")]
    {
        let raw = fs::read_to_string("/proc/meminfo").ok()?;
        for line in raw.lines() {
            if let Some(rest) = line.strip_prefix("MemTotal:") {
                let kb = rest
                    .split_whitespace()
                    .next()
                    .and_then(|value| value.parse::<u64>().ok())?;
                return Some(kb.saturating_mul(1024));
            }
        }
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        None
    }
}

fn storage_capacity_for_workspace(path: &Path) -> Option<(u64, u64)> {
    #[cfg(windows)]
    {
        let root = path
            .components()
            .next()?
            .as_os_str()
            .to_string_lossy()
            .to_string();
        let drive = root.trim_end_matches('\\').trim_end_matches(':');
        if drive.is_empty() {
            return None;
        }
        let script = format!(
            "$d=Get-PSDrive -Name '{}' -ErrorAction SilentlyContinue; if ($d) {{ [string]$d.Used + ',' + [string]$d.Free }}",
            drive.replace('\'', "")
        );
        let output = run_fixed_command(
            "powershell.exe",
            &["-NoProfile", "-NonInteractive", "-Command", &script],
            2000,
            128,
        )?;
        let (used, free) = output.trim().split_once(',')?;
        let used = used.trim().parse::<u64>().ok()?;
        let free = free.trim().parse::<u64>().ok()?;
        return Some((used.saturating_add(free), free));
    }
    #[cfg(not(windows))]
    {
        let path_text = path.display().to_string();
        let output = run_fixed_command("df", &["-kP", &path_text], 2000, 512)?;
        let line = output.lines().nth(1)?;
        let parts = line.split_whitespace().collect::<Vec<_>>();
        if parts.len() < 5 {
            return None;
        }
        let total = parts[1].parse::<u64>().ok()?.saturating_mul(1024);
        let free = parts[3].parse::<u64>().ok()?.saturating_mul(1024);
        return Some((total, free));
    }
    #[allow(unreachable_code)]
    None
}

fn bucket_bytes(bytes: u64) -> String {
    const GIB: u64 = 1024 * 1024 * 1024;
    match bytes / GIB {
        0..=7 => "<8GB".to_string(),
        8..=15 => "8-16GB".to_string(),
        16..=31 => "16-32GB".to_string(),
        32..=63 => "32-64GB".to_string(),
        64..=127 => "64-128GB".to_string(),
        _ => "128GB+".to_string(),
    }
}

fn loopback_bind_available() -> bool {
    TcpListener::bind("127.0.0.1:0").is_ok()
}

fn localhost_resolution_available() -> bool {
    ("localhost", 0).to_socket_addrs().is_ok()
}

fn runtime_readiness(program: &str, args: &[&str]) -> Value {
    let output = run_fixed_command(program, args, 2500, 512);
    match output {
        Some(text) => json!({
            "available": true,
            "version": first_non_empty_line(&text),
            "readiness": "available",
        }),
        None => json!({
            "available": false,
            "version": null,
            "readiness": "unavailable",
        }),
    }
}

fn office_worker_readiness() -> Value {
    let default = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("office_worker")
        .join("SuperNova.OfficeWorker")
        .join("SuperNova.OfficeWorker.csproj");
    let path = std::env::var("SUPERNOVA_OFFICE_WORKER_PROJECT")
        .map(std::path::PathBuf::from)
        .unwrap_or(default);
    json!({
        "available": path.is_file(),
        "version": null,
        "readiness": if path.is_file() { "available" } else { "unavailable" },
    })
}

fn first_non_empty_line(value: &str) -> Option<String> {
    value
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_string)
}

fn collect_local_ips() -> Vec<String> {
    #[cfg(windows)]
    {
        let Some(output) = run_fixed_command("ipconfig.exe", &[], 2500, 8192) else {
            return Vec::new();
        };
        return output
            .lines()
            .filter_map(|line| {
                if !line.contains("IPv4") {
                    return None;
                }
                let (_, value) = line.rsplit_once(':')?;
                let ip = value.trim().trim_end_matches("(Preferred)").trim();
                if ip.is_empty() {
                    None
                } else {
                    Some(ip.to_string())
                }
            })
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
    }
    #[cfg(not(windows))]
    {
        let Some(output) = run_fixed_command("hostname", &["-I"], 1500, 2048) else {
            return Vec::new();
        };
        return output
            .split_whitespace()
            .filter(|value| !value.trim().is_empty())
            .map(str::to_string)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
    }
}

fn collect_mac_addresses() -> Vec<String> {
    #[cfg(windows)]
    {
        let Some(output) = run_fixed_command("getmac.exe", &["/fo", "csv", "/nh"], 2500, 4096)
        else {
            return Vec::new();
        };
        return output
            .lines()
            .filter_map(|line| line.split(',').next())
            .map(|value| value.trim().trim_matches('"').to_string())
            .filter(|value| !value.is_empty() && value != "N/A")
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
    }
    #[cfg(target_os = "linux")]
    {
        let mut values = BTreeSet::new();
        if let Ok(entries) = fs::read_dir("/sys/class/net") {
            for entry in entries.flatten() {
                let Ok(name) = entry.file_name().into_string() else {
                    continue;
                };
                if name == "lo" {
                    continue;
                }
                if let Ok(raw) = fs::read_to_string(entry.path().join("address")) {
                    let value = raw.trim();
                    if !value.is_empty() && value != "00:00:00:00:00:00" {
                        values.insert(value.to_string());
                    }
                }
            }
        }
        return values.into_iter().collect();
    }
    #[cfg(all(not(windows), not(target_os = "linux")))]
    {
        Vec::new()
    }
}

fn run_fixed_command(
    program: &str,
    args: &[&str],
    timeout_ms: u64,
    max_output_bytes: usize,
) -> Option<String> {
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("PYTHONDONTWRITEBYTECODE", "1");
    suppress_child_window(&mut command);
    let mut child = command.spawn().ok()?;
    let deadline = Duration::from_millis(timeout_ms.max(1));
    let started = std::time::Instant::now();
    loop {
        if child.try_wait().ok().flatten().is_some() {
            break;
        }
        if started.elapsed() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        thread::sleep(Duration::from_millis(20));
    }
    let output = child.wait_with_output().ok()?;
    if !output.status.success() {
        return None;
    }
    let mut bytes = output.stdout;
    if bytes.is_empty() {
        bytes = output.stderr;
    }
    if bytes.len() > max_output_bytes {
        bytes.truncate(max_output_bytes);
    }
    Some(String::from_utf8_lossy(&bytes).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::create_agent_job;

    fn temp_workspace(name: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("supernova_client_env_{}_{}", name, now_ms()));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn runtime(name: &str) -> ClientEnvRuntime {
        let workspace = temp_workspace(name);
        let (job, process, truth) = create_agent_job(&workspace, "client env test").unwrap();
        let token = CapabilityToken {
            token_id: format!("token_{}", process.pid),
            job_id: job.job_id,
            pid: process.pid,
            workspace_root: workspace.display().to_string(),
            capabilities: vec!["client_env.scan_overview".to_string()],
            permissions: vec!["client_env:read".to_string()],
        };
        ClientEnvRuntime::new(WorkspaceGuard::new(workspace).unwrap(), truth, token)
    }

    #[test]
    fn locale_context_is_low_sensitive() {
        let context = ClientEnvRuntime::capture_locale_context();
        assert_eq!(context.schema_version, CLIENT_LOCALE_CONTEXT_SCHEMA_VERSION);
        assert_eq!(context.origin, CLIENT_ENV_ORIGIN);
        assert_eq!(context.sensitivity, "low");
        assert!(!context.current_local_datetime.trim().is_empty());
    }

    #[test]
    fn default_snapshot_redacts_sensitive_network_fields() {
        let runtime = runtime("redacts_sensitive");
        let receipt = runtime
            .scan_network(ClientEnvScanOptions::default())
            .unwrap();
        assert_eq!(receipt.status, "success");
        let data = receipt.data;
        assert!(data["sensitive_fields_returned"]
            .as_array()
            .unwrap()
            .is_empty());
        let section = &data["sections"].as_array().unwrap()[0];
        assert!(section["facts"].get("local_ip").is_none());
        assert!(section["facts"].get("mac_address").is_none());
        assert_eq!(
            data["redaction"]["requires_authorization_for"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn sensitive_scan_without_authorization_blocks() {
        let runtime = runtime("sensitive_blocks");
        let receipt = runtime
            .scan_network(ClientEnvScanOptions {
                include_sensitive_fields: true,
                ..ClientEnvScanOptions::default()
            })
            .unwrap();
        assert_eq!(receipt.status, "blocked");
        assert_eq!(
            receipt.data["requires_explicit_user_authorization"].as_bool(),
            Some(true)
        );
        assert_eq!(
            receipt.data["no_sensitive_values_returned"].as_bool(),
            Some(true)
        );
    }

    #[test]
    fn authorized_scan_returns_only_scoped_fields() {
        let runtime = runtime("authorized_scope");
        let request = runtime
            .request_sensitive_disclosure(
                vec![
                    "network.local_ip".to_string(),
                    "network.mac_address".to_string(),
                ],
                "test",
            )
            .unwrap();
        let request_id = request.data["disclosure_request_id"].as_str().unwrap();
        let token = ClientEnvRuntime::approve_sensitive_disclosure(
            &runtime.truth,
            &runtime.token.pid,
            request_id,
            vec!["network.local_ip".to_string()],
            "approved for test",
        )
        .unwrap();
        let receipt = runtime
            .scan_network(ClientEnvScanOptions {
                include_sensitive_fields: true,
                authorization_id: Some(token.authorization_id),
                ..ClientEnvScanOptions::default()
            })
            .unwrap();
        assert_eq!(receipt.status, "success");
        let returned = receipt.data["sensitive_fields_returned"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();
        assert_eq!(returned, vec!["network.local_ip"]);
        let section = &receipt.data["sections"].as_array().unwrap()[0];
        assert!(section["facts"].get("mac_address").is_none());
    }

    #[test]
    fn disclosure_approval_requires_pending_request_and_non_empty_subset_scope() {
        let runtime = runtime("approval_scope_validation");

        let missing = ClientEnvRuntime::approve_sensitive_disclosure(
            &runtime.truth,
            &runtime.token.pid,
            "missing_request",
            vec!["network.local_ip".to_string()],
            "should fail",
        )
        .unwrap_err();
        assert_eq!(missing.kind(), io::ErrorKind::InvalidInput);
        assert!(missing.to_string().contains("request not found"));

        let request = runtime
            .request_sensitive_disclosure(vec!["network.local_ip".to_string()], "test")
            .unwrap();
        let request_id = request.data["disclosure_request_id"].as_str().unwrap();

        let empty_scope = ClientEnvRuntime::approve_sensitive_disclosure(
            &runtime.truth,
            &runtime.token.pid,
            request_id,
            vec!["unknown.field".to_string()],
            "should fail",
        )
        .unwrap_err();
        assert_eq!(empty_scope.kind(), io::ErrorKind::InvalidInput);
        assert!(empty_scope
            .to_string()
            .contains("at least one allowed field"));

        let out_of_scope = ClientEnvRuntime::approve_sensitive_disclosure(
            &runtime.truth,
            &runtime.token.pid,
            request_id,
            vec!["network.mac_address".to_string()],
            "should fail",
        )
        .unwrap_err();
        assert_eq!(out_of_scope.kind(), io::ErrorKind::InvalidInput);
        assert!(out_of_scope.to_string().contains("outside request scope"));

        let token = ClientEnvRuntime::approve_sensitive_disclosure(
            &runtime.truth,
            &runtime.token.pid,
            request_id,
            vec!["network.local_ip".to_string()],
            "approved for test",
        )
        .unwrap();
        assert_eq!(token.allowed_fields, vec!["network.local_ip".to_string()]);

        let duplicate = ClientEnvRuntime::approve_sensitive_disclosure(
            &runtime.truth,
            &runtime.token.pid,
            request_id,
            vec!["network.local_ip".to_string()],
            "should fail",
        )
        .unwrap_err();
        assert_eq!(duplicate.kind(), io::ErrorKind::InvalidInput);
        assert!(duplicate.to_string().contains("already resolved"));
    }

    #[test]
    fn disclosure_approval_rejects_expired_pending_request() {
        let runtime = runtime("approval_expired");
        let now = now_ms();
        let request_id = format!("expired_request_{now}");
        runtime
            .truth
            .append_event(
                Some(&runtime.token.pid),
                "client_env_disclosure_requested",
                json!({
                    "request_id": request_id,
                    "requested_fields": ["network.local_ip"],
                    "reason": "expired test",
                    "created_at_unix_ms": now.saturating_sub(DISCLOSURE_TTL_MS + 10),
                    "expires_at_unix_ms": now.saturating_sub(1),
                    "status": "pending",
                }),
            )
            .unwrap();

        let err = ClientEnvRuntime::approve_sensitive_disclosure(
            &runtime.truth,
            &runtime.token.pid,
            &request_id,
            vec!["network.local_ip".to_string()],
            "should fail",
        )
        .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        assert!(err.to_string().contains("request expired"));
    }

    #[test]
    fn read_snapshot_returns_structured_page() {
        let runtime = runtime("read_snapshot");
        let receipt = runtime
            .scan_overview(ClientEnvScanOptions::default())
            .unwrap();
        let snapshot_ref = receipt.data["snapshot_ref"].as_str().unwrap();
        let page = runtime.read_snapshot(snapshot_ref, 0, 2).unwrap();
        assert_eq!(page.status, "success");
        assert_eq!(page.data["returned"].as_u64(), Some(2));
        assert_eq!(page.data["snapshot_ref"].as_str(), Some(snapshot_ref));
    }
}
