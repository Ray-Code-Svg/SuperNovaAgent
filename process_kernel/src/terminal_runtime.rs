use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{self, Read};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{
    child_process::suppress_child_window, file_fingerprint, json_err, now_ms, to_json_value,
    CapabilityReceipt, CapabilityToken, ProcessTruthStore, WorkspaceGuard, RUNTIME_DIR_NAME,
};
#[derive(Clone, Debug)]
pub struct TerminalRuntime {
    guard: WorkspaceGuard,
    truth: ProcessTruthStore,
    token: CapabilityToken,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TerminalApproval {
    pub approval_id: Option<String>,
    pub preview_id: Option<String>,
}

impl TerminalApproval {
    pub fn none() -> Self {
        Self {
            approval_id: None,
            preview_id: None,
        }
    }

    pub fn approved(approval_id: impl Into<String>) -> Self {
        Self {
            approval_id: Some(approval_id.into()),
            preview_id: None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceEntryState {
    pub kind: String,
    pub size_bytes: u64,
    pub fingerprint: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceDiff {
    pub added_files: Vec<String>,
    pub removed_files: Vec<String>,
    pub changed_files: Vec<String>,
    pub added_dirs: Vec<String>,
    pub removed_dirs: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TerminalServiceHealthCheck {
    pub kind: String,
    pub url: Option<String>,
    pub port: Option<u16>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TerminalServiceRecord {
    pub service_id: String,
    pub job_id: String,
    pub pid: u32,
    pub argv: Vec<String>,
    pub cwd: String,
    pub status: String,
    pub health_status: String,
    pub health_check: Option<TerminalServiceHealthCheck>,
    pub expected_ports: Vec<u16>,
    pub stdout_ref: String,
    pub stderr_ref: String,
    pub started_at_ms: u128,
    pub updated_at_ms: u128,
    pub stop_reason: Option<String>,
    pub exit_observed: bool,
}

impl TerminalRuntime {
    pub fn new(guard: WorkspaceGuard, truth: ProcessTruthStore, token: CapabilityToken) -> Self {
        Self {
            guard,
            truth,
            token,
        }
    }

    pub fn run_command(&self, argv: Vec<String>, timeout_ms: u64) -> io::Result<CapabilityReceipt> {
        self.run_command_with_approval(argv, timeout_ms, TerminalApproval::none())
    }

    pub fn run_command_with_approval(
        &self,
        argv: Vec<String>,
        timeout_ms: u64,
        approval: TerminalApproval,
    ) -> io::Result<CapabilityReceipt> {
        self.run_command_internal(argv, timeout_ms, approval, "direct", None)
    }

    pub fn run_powershell(
        &self,
        script: &str,
        timeout_ms: u64,
        approval: TerminalApproval,
    ) -> io::Result<CapabilityReceipt> {
        let shell = powershell_executable();
        let argv = vec![
            shell,
            "-NoProfile".to_string(),
            "-NonInteractive".to_string(),
            "-ExecutionPolicy".to_string(),
            "Bypass".to_string(),
            "-Command".to_string(),
            script.to_string(),
        ];
        self.run_command_internal(
            argv,
            timeout_ms,
            approval,
            "powershell",
            Some(script.to_string()),
        )
    }

    fn run_command_internal(
        &self,
        argv: Vec<String>,
        timeout_ms: u64,
        approval: TerminalApproval,
        shell_kind: &str,
        script: Option<String>,
    ) -> io::Result<CapabilityReceipt> {
        if !self
            .token
            .capabilities
            .iter()
            .any(|item| item == "terminal.run_command")
        {
            return Ok(self.blocked_receipt("terminal.run_command not granted"));
        }
        if argv.is_empty() {
            return Ok(self.blocked_receipt("empty argv"));
        }
        if timeout_ms == 0 {
            return Ok(self.blocked_receipt("timeout_ms must be provided and greater than zero"));
        }
        if service_command_detected(&argv) {
            return Ok(self.blocked_receipt_with_data(
                "terminal.run_command",
                "command_blocked",
                json!({
                    "reason": "service-class commands must use terminal.start_service",
                    "reason_code": "service_command_requires_start_service",
                    "recoverable": true,
                    "required_capability": "terminal.start_service",
                    "argv": argv,
                    "timeout_ms": timeout_ms,
                }),
            ));
        }
        let mutation_detected = is_mutation_command(&argv);
        if mutation_detected {
            if let Some(reason) = terminal_workspace_boundary_violation(&argv, script.as_deref()) {
                return Ok(self.hard_blocked_receipt(
                    &reason,
                    &argv,
                    shell_kind,
                    script.as_deref(),
                ));
            }
        }
        let before_snapshot = snapshot_workspace(self.guard.root())?;
        let before_snapshot_ref = self.truth.write_blob(
            "terminal_before_snapshot.json",
            &serde_json::to_vec(&before_snapshot).map_err(json_err)?,
        )?;
        let started = Instant::now();
        let mut command = Command::new(&argv[0]);
        command.args(&argv[1..]);
        command.current_dir(self.guard.root());
        command.env_clear();
        let fixed_env = fixed_terminal_env();
        command.envs(fixed_env.iter());
        command.stdout(Stdio::piped());
        command.stderr(Stdio::piped());
        suppress_child_window(&mut command);
        let mut child = command.spawn()?;
        let child_pid = child.id();
        let mut timed_out = false;
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break status;
            }
            if started.elapsed() > Duration::from_millis(timeout_ms) {
                timed_out = true;
                let _ = child.kill();
                break child.wait()?;
            }
            thread::sleep(Duration::from_millis(10));
        };
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        if let Some(mut out) = child.stdout.take() {
            out.read_to_end(&mut stdout)?;
        }
        if let Some(mut err) = child.stderr.take() {
            err.read_to_end(&mut stderr)?;
        }
        let stdout_ref = self.truth.write_blob("terminal_stdout.txt", &stdout)?;
        let stderr_ref = self.truth.write_blob("terminal_stderr.txt", &stderr)?;
        let after_snapshot = snapshot_workspace(self.guard.root())?;
        let after_snapshot_ref = self.truth.write_blob(
            "terminal_after_snapshot.json",
            &serde_json::to_vec(&after_snapshot).map_err(json_err)?,
        )?;
        let workspace_diff = diff_workspace_snapshots(&before_snapshot, &after_snapshot);
        let workspace_diff_ref = self.truth.write_blob(
            "terminal_workspace_diff.json",
            &serde_json::to_vec(&workspace_diff).map_err(json_err)?,
        )?;
        let duration_ms = started.elapsed().as_millis();
        let shell_version = shell_version_for(shell_kind, &argv);
        let argv_for_receipt = argv.clone();
        let argv_for_tree = argv.clone();
        let receipt = CapabilityReceipt {
            capability_id: "terminal.run_command".to_string(),
            job_id: self.token.job_id.clone(),
            pid: self.token.pid.clone(),
            status: if status.success() && !timed_out {
                "success"
            } else {
                "failed"
            }
            .to_string(),
            data: json!({
                "cwd": self.guard.root().display().to_string(),
                "argv": argv_for_receipt,
                "shell_kind": shell_kind,
                "shell_version": shell_version,
                "script": script,
                "pid": child_pid,
                "process_tree": [
                    {
                        "pid": child_pid,
                        "ppid": Value::Null,
                        "argv": argv_for_tree,
                    }
                ],
                "exit_code": status.code(),
                "timed_out": timed_out,
                "timeout_ms": timeout_ms,
                "duration_ms": duration_ms,
                "env": fixed_env,
                "stdout_ref": stdout_ref,
                "stderr_ref": stderr_ref,
                "stdout_bytes": stdout.len(),
                "stderr_bytes": stderr.len(),
                "before_snapshot_ref": before_snapshot_ref.clone(),
                "after_snapshot_ref": after_snapshot_ref,
                "workspace_diff_ref": workspace_diff_ref.clone(),
                "before_diff_ref": before_snapshot_ref,
                "after_diff_ref": workspace_diff_ref,
                "resource_usage": {
                    "wall_time_ms": duration_ms,
                    "stdout_bytes": stdout.len(),
                    "stderr_bytes": stderr.len(),
                },
                "mutation_detected": mutation_detected,
                "approval_required": mutation_detected,
                "approval_id": approval.approval_id,
                "preview_id": approval.preview_id,
            }),
        };
        self.truth.append_event(
            Some(&self.token.pid),
            "command_receipt",
            to_json_value(&receipt)?,
        )?;
        self.truth.append_event(
            Some(&self.token.pid),
            "capability_receipt",
            to_json_value(&receipt)?,
        )?;
        Ok(receipt)
    }

    pub fn start_service(
        &self,
        service_id: &str,
        argv: Vec<String>,
        startup_timeout_ms: u64,
        health_check: Option<TerminalServiceHealthCheck>,
        expected_ports: Vec<u16>,
    ) -> io::Result<CapabilityReceipt> {
        if !self
            .token
            .capabilities
            .iter()
            .any(|item| item == "terminal.start_service")
        {
            return Ok(self.blocked_receipt_with_data(
                "terminal.start_service",
                "terminal_service_blocked",
                json!({"reason": "terminal.start_service not granted"}),
            ));
        }
        let service_id = validate_service_id(service_id)?;
        if argv.is_empty() {
            return Ok(self.blocked_receipt_with_data(
                "terminal.start_service",
                "terminal_service_blocked",
                json!({"reason": "empty argv", "service_id": service_id}),
            ));
        }
        if startup_timeout_ms == 0 {
            return Ok(self.blocked_receipt_with_data(
                "terminal.start_service",
                "terminal_service_blocked",
                json!({
                    "reason": "startup_timeout_ms must be provided and greater than zero",
                    "service_id": service_id,
                    "recoverable": true,
                }),
            ));
        }
        if let Some(existing) = read_service_record(&self.truth, &service_id)? {
            if existing.status == "running" && process_is_running(existing.pid) {
                return Ok(self.blocked_receipt_with_data(
                    "terminal.start_service",
                    "terminal_service_blocked",
                    json!({
                        "reason": "terminal service is already running",
                        "service_id": service_id,
                        "pid": existing.pid,
                        "recoverable": true,
                        "suggested_capability": "terminal.service_status",
                    }),
                ));
            }
        }

        if let Some(reason) = terminal_workspace_boundary_violation(&argv, None) {
            if is_mutation_command(&argv) {
                return Ok(self.blocked_receipt_with_data(
                    "terminal.start_service",
                    "terminal_service_blocked",
                    json!({
                        "reason": reason,
                        "service_id": service_id,
                        "argv": argv,
                        "hard_block": true,
                        "non_overridable_policy": "workspace_boundary",
                    }),
                ));
            }
        }

        let stdout_ref = self
            .truth
            .write_blob(&format!("terminal_services/{service_id}/stdout.log"), b"")?;
        let stderr_ref = self
            .truth
            .write_blob(&format!("terminal_services/{service_id}/stderr.log"), b"")?;
        let stdout_path = self.truth.resolve_blob_ref(&stdout_ref)?;
        let stderr_path = self.truth.resolve_blob_ref(&stderr_ref)?;
        let stdout_file = File::create(&stdout_path)?;
        let stderr_file = File::create(&stderr_path)?;

        let started_at_ms = now_ms();
        let started = Instant::now();
        let mut command = Command::new(&argv[0]);
        command.args(&argv[1..]);
        command.current_dir(self.guard.root());
        command.env_clear();
        let fixed_env = fixed_terminal_env();
        command.envs(fixed_env.iter());
        command.stdout(Stdio::from(stdout_file));
        command.stderr(Stdio::from(stderr_file));
        suppress_child_window(&mut command);
        let mut child = command.spawn()?;
        let child_pid = child.id();

        let ready = loop {
            if let Some(_status) = child.try_wait()? {
                let mut record = TerminalServiceRecord {
                    service_id: service_id.clone(),
                    job_id: self.token.job_id.clone(),
                    pid: child_pid,
                    argv: argv.clone(),
                    cwd: self.guard.root().display().to_string(),
                    status: "failed".to_string(),
                    health_status: "process_exited_before_ready".to_string(),
                    health_check: health_check.clone(),
                    expected_ports: expected_ports.clone(),
                    stdout_ref: stdout_ref.clone(),
                    stderr_ref: stderr_ref.clone(),
                    started_at_ms,
                    updated_at_ms: now_ms(),
                    stop_reason: None,
                    exit_observed: true,
                };
                write_service_record(&self.truth, &record)?;
                return self.service_receipt(
                    "terminal.start_service",
                    "failed",
                    &mut record,
                    json!({
                        "reason": "process exited before service became ready",
                        "startup_timeout_ms": startup_timeout_ms,
                    }),
                );
            }
            if service_health_ready(&health_check, &expected_ports) {
                break true;
            }
            if health_check.is_none() && expected_ports.is_empty() {
                break true;
            }
            if started.elapsed() > Duration::from_millis(startup_timeout_ms) {
                let _ = child.kill();
                let _ = child.wait();
                break false;
            }
            thread::sleep(Duration::from_millis(100));
        };

        let mut record = TerminalServiceRecord {
            service_id: service_id.clone(),
            job_id: self.token.job_id.clone(),
            pid: child_pid,
            argv: argv.clone(),
            cwd: self.guard.root().display().to_string(),
            status: if ready { "running" } else { "failed" }.to_string(),
            health_status: if ready {
                "ready".to_string()
            } else {
                "startup_timeout".to_string()
            },
            health_check,
            expected_ports,
            stdout_ref,
            stderr_ref,
            started_at_ms,
            updated_at_ms: now_ms(),
            stop_reason: None,
            exit_observed: !ready,
        };
        write_service_record(&self.truth, &record)?;
        self.service_receipt(
            "terminal.start_service",
            if ready { "success" } else { "failed" },
            &mut record,
            json!({
                "startup_timeout_ms": startup_timeout_ms,
                "duration_ms": started.elapsed().as_millis(),
                "env": fixed_env,
            }),
        )
    }

    pub fn stop_service(
        &self,
        service_id: &str,
        reason: Option<&str>,
    ) -> io::Result<CapabilityReceipt> {
        if !self
            .token
            .capabilities
            .iter()
            .any(|item| item == "terminal.stop_service")
        {
            return Ok(self.blocked_receipt_with_data(
                "terminal.stop_service",
                "terminal_service_blocked",
                json!({"reason": "terminal.stop_service not granted"}),
            ));
        }
        let service_id = validate_service_id(service_id)?;
        let Some(mut record) = read_service_record(&self.truth, &service_id)? else {
            return Ok(self.blocked_receipt_with_data(
                "terminal.stop_service",
                "terminal_service_blocked",
                json!({
                    "reason": "terminal service not found",
                    "service_id": service_id,
                    "recoverable": true,
                }),
            ));
        };
        stop_service_record(
            &self.truth,
            &self.token.pid,
            &mut record,
            reason.unwrap_or("user_requested"),
        )
    }

    pub fn service_status(&self, service_id: &str) -> io::Result<CapabilityReceipt> {
        if !self
            .token
            .capabilities
            .iter()
            .any(|item| item == "terminal.service_status")
        {
            return Ok(self.blocked_receipt_with_data(
                "terminal.service_status",
                "terminal_service_blocked",
                json!({"reason": "terminal.service_status not granted"}),
            ));
        }
        let service_id = validate_service_id(service_id)?;
        let Some(mut record) = read_service_record(&self.truth, &service_id)? else {
            return Ok(self.blocked_receipt_with_data(
                "terminal.service_status",
                "terminal_service_blocked",
                json!({
                    "reason": "terminal service not found",
                    "service_id": service_id,
                    "recoverable": true,
                }),
            ));
        };
        refresh_service_record_status(&self.truth, &mut record)?;
        self.service_receipt("terminal.service_status", "success", &mut record, json!({}))
    }

    fn service_receipt(
        &self,
        capability_id: &str,
        status: &str,
        record: &mut TerminalServiceRecord,
        extra: Value,
    ) -> io::Result<CapabilityReceipt> {
        record.updated_at_ms = now_ms();
        write_service_record(&self.truth, record)?;
        let mut data = to_json_value(record)?;
        if let Some(object) = data.as_object_mut() {
            if let Some(extra_object) = extra.as_object() {
                for (key, value) in extra_object {
                    object.insert(key.clone(), value.clone());
                }
            }
            object.insert(
                "log_tail".to_string(),
                service_log_tail(&self.truth, record),
            );
        }
        let receipt = CapabilityReceipt {
            capability_id: capability_id.to_string(),
            job_id: self.token.job_id.clone(),
            pid: self.token.pid.clone(),
            status: status.to_string(),
            data,
        };
        self.truth.append_event(
            Some(&self.token.pid),
            terminal_service_event_type(capability_id),
            to_json_value(&receipt)?,
        )?;
        self.truth.append_event(
            Some(&self.token.pid),
            "capability_receipt",
            to_json_value(&receipt)?,
        )?;
        Ok(receipt)
    }

    fn blocked_receipt(&self, reason: &str) -> CapabilityReceipt {
        self.blocked_receipt_with_data(
            "terminal.run_command",
            "command_blocked",
            json!({"reason": reason}),
        )
    }

    fn blocked_receipt_with_data(
        &self,
        capability_id: &str,
        event_type: &str,
        data: Value,
    ) -> CapabilityReceipt {
        let receipt = CapabilityReceipt {
            capability_id: capability_id.to_string(),
            job_id: self.token.job_id.clone(),
            pid: self.token.pid.clone(),
            status: "blocked".to_string(),
            data,
        };
        let serialized = to_json_value(&receipt).unwrap_or_else(|_| {
            json!({
                "capability_id": capability_id,
                "status": "blocked",
            })
        });
        let _ = self
            .truth
            .append_event(Some(&self.token.pid), event_type, serialized.clone());
        let _ = self.truth.append_event(
            Some(&self.token.pid),
            if capability_id == "terminal.run_command" {
                "command_receipt"
            } else {
                "terminal_service_receipt"
            },
            serialized.clone(),
        );
        let _ = self
            .truth
            .append_event(Some(&self.token.pid), "capability_receipt", serialized);
        receipt
    }

    fn hard_blocked_receipt(
        &self,
        reason: &str,
        argv: &[String],
        shell_kind: &str,
        script: Option<&str>,
    ) -> CapabilityReceipt {
        let receipt = CapabilityReceipt {
            capability_id: "terminal.run_command".to_string(),
            job_id: self.token.job_id.clone(),
            pid: self.token.pid.clone(),
            status: "blocked".to_string(),
            data: json!({
                "reason": reason,
                "cwd": self.guard.root().display().to_string(),
                "argv": argv,
                "shell_kind": shell_kind,
                "script": script,
                "mutation_detected": true,
                "approval_required": false,
                "approval_allowed": false,
                "hard_block": true,
                "non_overridable_policy": "workspace_boundary",
            }),
        };
        let _ = self.truth.append_event(
            Some(&self.token.pid),
            "command_blocked",
            to_json_value(&receipt).unwrap_or_else(|_| json!({"reason": reason})),
        );
        let _ = self.truth.append_event(
            Some(&self.token.pid),
            "command_receipt",
            to_json_value(&receipt).unwrap_or_else(|_| json!({"reason": reason})),
        );
        let _ = self.truth.append_event(
            Some(&self.token.pid),
            "capability_receipt",
            to_json_value(&receipt).unwrap_or_else(|_| json!({"reason": reason})),
        );
        receipt
    }
}

pub fn stop_terminal_services_for_job(
    truth: &ProcessTruthStore,
    pid: &str,
    reason: &str,
) -> io::Result<Vec<CapabilityReceipt>> {
    let mut receipts = Vec::new();
    let dir = service_registry_dir(truth);
    if !dir.exists() {
        return Ok(receipts);
    }
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let mut record: TerminalServiceRecord =
            serde_json::from_slice(&fs::read(&path)?).map_err(json_err)?;
        if record.status == "running" {
            receipts.push(stop_service_record(truth, pid, &mut record, reason)?);
        }
    }
    Ok(receipts)
}

fn validate_service_id(service_id: &str) -> io::Result<String> {
    let trimmed = service_id.trim();
    if trimmed.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "service_id missing",
        ));
    }
    if trimmed.len() > 96
        || !trimmed
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "service_id may contain only ASCII letters, digits, '.', '_' or '-'",
        ));
    }
    Ok(trimmed.to_string())
}

fn service_registry_dir(truth: &ProcessTruthStore) -> PathBuf {
    truth
        .state_root()
        .join("terminal_services")
        .join(truth.job_id())
}

fn service_registry_path(truth: &ProcessTruthStore, service_id: &str) -> PathBuf {
    service_registry_dir(truth).join(format!("{service_id}.json"))
}

fn read_service_record(
    truth: &ProcessTruthStore,
    service_id: &str,
) -> io::Result<Option<TerminalServiceRecord>> {
    let path = service_registry_path(truth, service_id);
    if !path.exists() {
        return Ok(None);
    }
    serde_json::from_slice(&fs::read(path)?)
        .map(Some)
        .map_err(json_err)
}

fn write_service_record(
    truth: &ProcessTruthStore,
    record: &TerminalServiceRecord,
) -> io::Result<()> {
    let dir = service_registry_dir(truth);
    fs::create_dir_all(&dir)?;
    let path = service_registry_path(truth, &record.service_id);
    fs::write(path, serde_json::to_vec_pretty(record).map_err(json_err)?)?;
    Ok(())
}

fn refresh_service_record_status(
    truth: &ProcessTruthStore,
    record: &mut TerminalServiceRecord,
) -> io::Result<()> {
    if record.status == "running" && !process_is_running(record.pid) {
        record.status = "exited".to_string();
        record.health_status = "process_exited".to_string();
        record.exit_observed = true;
        record.updated_at_ms = now_ms();
        write_service_record(truth, record)?;
    }
    Ok(())
}

fn stop_service_record(
    truth: &ProcessTruthStore,
    pid: &str,
    record: &mut TerminalServiceRecord,
    reason: &str,
) -> io::Result<CapabilityReceipt> {
    let was_running = record.status == "running" && process_is_running(record.pid);
    let stopped = if was_running {
        stop_process_tree(record.pid)
    } else {
        true
    };
    record.status = if stopped { "stopped" } else { "failed" }.to_string();
    record.health_status = if stopped {
        "stopped".to_string()
    } else {
        "stop_failed".to_string()
    };
    record.stop_reason = Some(reason.to_string());
    record.updated_at_ms = now_ms();
    write_service_record(truth, record)?;
    let mut data = to_json_value(record)?;
    if let Some(object) = data.as_object_mut() {
        object.insert("reason".to_string(), json!(reason));
        object.insert("was_running".to_string(), json!(was_running));
        object.insert("stop_signal_sent".to_string(), json!(stopped));
        object.insert("log_tail".to_string(), service_log_tail(truth, record));
    }
    let receipt = CapabilityReceipt {
        capability_id: "terminal.stop_service".to_string(),
        job_id: truth.job_id().to_string(),
        pid: pid.to_string(),
        status: if stopped { "success" } else { "failed" }.to_string(),
        data,
    };
    truth.append_event(
        Some(pid),
        "terminal_service_stopped",
        to_json_value(&receipt)?,
    )?;
    truth.append_event(Some(pid), "capability_receipt", to_json_value(&receipt)?)?;
    Ok(receipt)
}

fn terminal_service_event_type(capability_id: &str) -> &'static str {
    match capability_id {
        "terminal.start_service" => "terminal_service_started",
        "terminal.stop_service" => "terminal_service_stopped",
        "terminal.service_status" => "terminal_service_status",
        _ => "terminal_service_receipt",
    }
}

fn service_health_ready(
    health_check: &Option<TerminalServiceHealthCheck>,
    expected_ports: &[u16],
) -> bool {
    if expected_ports.iter().any(|port| tcp_port_ready(*port)) {
        return true;
    }
    let Some(health_check) = health_check else {
        return expected_ports.is_empty();
    };
    match health_check.kind.as_str() {
        "http" => health_check.url.as_deref().is_some_and(http_health_ready),
        "tcp" => health_check.port.is_some_and(tcp_port_ready),
        _ => false,
    }
}

fn http_health_ready(url: &str) -> bool {
    match ureq::get(url).timeout(Duration::from_millis(1_000)).call() {
        Ok(response) => response.status() < 500,
        Err(ureq::Error::Status(status, _)) => status < 500,
        Err(_) => false,
    }
}

fn tcp_port_ready(port: u16) -> bool {
    let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);
    TcpStream::connect_timeout(&addr, Duration::from_millis(250)).is_ok()
}

fn service_log_tail(truth: &ProcessTruthStore, record: &TerminalServiceRecord) -> Value {
    json!({
        "stdout": log_tail_for_ref(truth, &record.stdout_ref),
        "stderr": log_tail_for_ref(truth, &record.stderr_ref),
    })
}

fn log_tail_for_ref(truth: &ProcessTruthStore, blob_ref: &str) -> String {
    let Ok(path) = truth.resolve_blob_ref(blob_ref) else {
        return String::new();
    };
    let Ok(bytes) = fs::read(path) else {
        return String::new();
    };
    let text = String::from_utf8_lossy(&bytes);
    let tail = text
        .lines()
        .rev()
        .take(20)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("\n");
    tail.chars()
        .rev()
        .take(4_000)
        .collect::<String>()
        .chars()
        .rev()
        .collect()
}

fn service_command_detected(argv: &[String]) -> bool {
    let normalized = argv
        .iter()
        .map(|item| item.to_ascii_lowercase())
        .collect::<Vec<_>>();
    if normalized.is_empty() {
        return false;
    }
    let joined = normalized.join(" ");
    joined.contains("uvicorn")
        || joined.contains("streamlit run")
        || joined.contains("flask run")
        || joined.contains("python -m http.server")
        || joined.contains("python3 -m http.server")
        || joined.contains("fastapi dev")
        || joined.contains("vite --host")
        || joined.contains("vite --open")
        || joined.contains("npm run dev")
        || joined.contains("npm.cmd run dev")
        || joined.contains("pnpm dev")
        || joined.contains("yarn dev")
        || joined.contains("next dev")
}

fn process_is_running(pid: u32) -> bool {
    #[cfg(windows)]
    {
        let filter = format!("PID eq {pid}");
        let Ok(output) = Command::new("tasklist")
            .args(["/FI", filter.as_str(), "/FO", "CSV", "/NH"])
            .output()
        else {
            return false;
        };
        let stdout = String::from_utf8_lossy(&output.stdout);
        output.status.success() && stdout.contains(&pid.to_string()) && !stdout.contains("INFO:")
    }
    #[cfg(not(windows))]
    {
        Command::new("sh")
            .arg("-c")
            .arg(format!("kill -0 {pid}"))
            .status()
            .is_ok_and(|status| status.success())
    }
}

fn stop_process_tree(pid: u32) -> bool {
    #[cfg(windows)]
    {
        Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .status()
            .is_ok_and(|status| status.success())
    }
    #[cfg(not(windows))]
    {
        let _ = Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .status();
        for _ in 0..20 {
            if !process_is_running(pid) {
                return true;
            }
            thread::sleep(Duration::from_millis(50));
        }
        Command::new("kill")
            .args(["-KILL", &pid.to_string()])
            .status()
            .is_ok_and(|status| status.success())
    }
}

fn snapshot_workspace(root: &Path) -> io::Result<BTreeMap<String, WorkspaceEntryState>> {
    let mut snapshot = BTreeMap::new();
    snapshot_workspace_inner(root, root, &mut snapshot)?;
    Ok(snapshot)
}

fn snapshot_workspace_inner(
    root: &Path,
    current: &Path,
    snapshot: &mut BTreeMap<String, WorkspaceEntryState>,
) -> io::Result<()> {
    let mut children = Vec::new();
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        if name.to_string_lossy() == RUNTIME_DIR_NAME {
            continue;
        }
        children.push(path);
    }
    children.sort();
    for path in children {
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .display()
            .to_string()
            .replace('\\', "/");
        if path.is_dir() {
            snapshot.insert(
                rel,
                WorkspaceEntryState {
                    kind: "dir".to_string(),
                    size_bytes: 0,
                    fingerprint: "dir".to_string(),
                },
            );
            snapshot_workspace_inner(root, &path, snapshot)?;
        } else if path.is_file() {
            let metadata = path.metadata()?;
            snapshot.insert(
                rel,
                WorkspaceEntryState {
                    kind: "file".to_string(),
                    size_bytes: metadata.len(),
                    fingerprint: file_fingerprint(&path)?,
                },
            );
        }
    }
    Ok(())
}

fn diff_workspace_snapshots(
    before: &BTreeMap<String, WorkspaceEntryState>,
    after: &BTreeMap<String, WorkspaceEntryState>,
) -> WorkspaceDiff {
    let before_keys: BTreeSet<String> = before.keys().cloned().collect();
    let after_keys: BTreeSet<String> = after.keys().cloned().collect();
    let added = after_keys.difference(&before_keys);
    let removed = before_keys.difference(&after_keys);
    let mut diff = WorkspaceDiff {
        added_files: Vec::new(),
        removed_files: Vec::new(),
        changed_files: Vec::new(),
        added_dirs: Vec::new(),
        removed_dirs: Vec::new(),
    };
    for path in added {
        if after.get(path).map(|item| item.kind.as_str()) == Some("dir") {
            diff.added_dirs.push(path.clone());
        } else {
            diff.added_files.push(path.clone());
        }
    }
    for path in removed {
        if before.get(path).map(|item| item.kind.as_str()) == Some("dir") {
            diff.removed_dirs.push(path.clone());
        } else {
            diff.removed_files.push(path.clone());
        }
    }
    for path in before_keys.intersection(&after_keys) {
        if let (Some(old), Some(new)) = (before.get(path), after.get(path)) {
            if old.kind == "file" && new.kind == "file" && old != new {
                diff.changed_files.push(path.clone());
            }
        }
    }
    diff
}

fn is_mutation_command(argv: &[String]) -> bool {
    let joined = argv.join(" ").to_ascii_lowercase();
    let patterns = [
        "remove-item",
        " rm ",
        " del ",
        "erase ",
        "move-item",
        "copy-item",
        "rename-item",
        "new-item",
        "set-content",
        "add-content",
        "out-file",
        "mkdir",
        "rmdir",
        "touch ",
        " > ",
        ">>",
    ];
    patterns.iter().any(|pattern| joined.contains(pattern))
}

pub fn terminal_command_mutation_detected(argv: &[String]) -> bool {
    is_mutation_command(argv)
}

fn terminal_workspace_boundary_violation(argv: &[String], script: Option<&str>) -> Option<String> {
    let joined = match script {
        Some(script) => format!("{} {}", argv.join(" "), script),
        None => argv.join(" "),
    };
    let lower = joined.to_ascii_lowercase();
    if lower.contains("..\\") || lower.contains("../") || lower.contains(" .. ") {
        return Some(
            "terminal mutation references a parent directory and cannot be approved".to_string(),
        );
    }
    if lower.contains(" > /")
        || lower.contains(" >/")
        || lower.contains(" >> /")
        || lower.contains(">>/")
        || lower.contains(" > ~")
        || lower.contains(" > %userprofile%")
        || lower.contains(" > $home")
    {
        return Some(
            "terminal mutation writes outside the workspace boundary and cannot be approved"
                .to_string(),
        );
    }
    for arg in argv.iter().skip(1) {
        if looks_like_absolute_or_parent_path(arg) {
            return Some(
                "terminal mutation contains a non-workspace-scoped path and cannot be approved"
                    .to_string(),
            );
        }
    }
    None
}

fn looks_like_absolute_or_parent_path(value: &str) -> bool {
    let trimmed = value.trim_matches('"').trim_matches('\'');
    if trimmed.contains("../") || trimmed.contains("..\\") {
        return true;
    }
    if trimmed.starts_with('/') || trimmed.starts_with('~') {
        return true;
    }
    let bytes = trimmed.as_bytes();
    bytes.len() >= 3 && bytes[1] == b':' && (bytes[2] == b'\\' || bytes[2] == b'/')
}

fn powershell_executable() -> String {
    if cfg!(windows) {
        "powershell.exe".to_string()
    } else {
        "pwsh".to_string()
    }
}

fn fixed_terminal_env() -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();
    for key in [
        "APPDATA",
        "COMSPEC",
        "HOMEDRIVE",
        "HOMEPATH",
        "LOCALAPPDATA",
        "NUMBER_OF_PROCESSORS",
        "OS",
        "PATH",
        "PATHEXT",
        "PROCESSOR_ARCHITECTURE",
        "ProgramData",
        "PSModulePath",
        "SystemDrive",
        "SystemRoot",
        "TEMP",
        "TMP",
        "USERPROFILE",
        "WINDIR",
    ] {
        if let Ok(value) = std::env::var(key) {
            env.insert(key.to_string(), value);
        }
    }
    env.insert("NO_COLOR".to_string(), "1".to_string());
    env.insert("SUPERNOVA_TERMINAL_WRAPPER".to_string(), "1".to_string());
    env
}

fn shell_version_for(shell_kind: &str, argv: &[String]) -> Value {
    if shell_kind != "powershell" {
        return json!({
            "kind": shell_kind,
            "program": argv.first().cloned().unwrap_or_default(),
            "version": Value::Null,
        });
    }
    let shell = argv.first().cloned().unwrap_or_else(powershell_executable);
    let mut command = Command::new(&shell);
    command
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "$PSVersionTable.PSVersion.ToString()",
        ])
        .env_clear()
        .envs(fixed_terminal_env().iter());
    suppress_child_window(&mut command);
    let output = command.output();
    match output {
        Ok(output) => json!({
            "kind": "powershell",
            "program": shell,
            "version": String::from_utf8_lossy(&output.stdout).trim(),
            "exit_code": output.status.code(),
        }),
        Err(err) => json!({
            "kind": "powershell",
            "program": shell,
            "version": Value::Null,
            "error": err.to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::create_agent_job_with_state_root;
    use std::path::PathBuf;

    fn temp_paths(name: &str) -> (PathBuf, PathBuf) {
        let base = std::env::temp_dir().join(format!("supernova_terminal_{name}_{}", now_ms()));
        let workspace = base.join("workspace");
        let state = base.join("state");
        fs::create_dir_all(&workspace).unwrap();
        fs::create_dir_all(&state).unwrap();
        (workspace, state)
    }

    fn service_command() -> Vec<String> {
        if cfg!(windows) {
            vec![
                "powershell.exe".to_string(),
                "-NoProfile".to_string(),
                "-NonInteractive".to_string(),
                "-Command".to_string(),
                "Start-Sleep -Seconds 30".to_string(),
            ]
        } else {
            vec!["sh".to_string(), "-c".to_string(), "sleep 30".to_string()]
        }
    }

    fn token(
        job_id: String,
        pid: String,
        workspace: &Path,
        capabilities: Vec<&str>,
    ) -> CapabilityToken {
        CapabilityToken {
            token_id: "token_terminal_service".to_string(),
            job_id,
            pid,
            workspace_root: workspace.display().to_string(),
            capabilities: capabilities.into_iter().map(str::to_string).collect(),
            permissions: vec!["terminal:execute".to_string()],
        }
    }

    #[test]
    fn run_command_rejects_service_class_command() {
        let (workspace, state) = temp_paths("reject_service");
        let (job, process, truth) =
            create_agent_job_with_state_root(&workspace, &state, "Reject service command").unwrap();
        let runtime = TerminalRuntime::new(
            WorkspaceGuard::new(&workspace).unwrap(),
            truth,
            token(
                job.job_id,
                process.pid,
                &workspace,
                vec!["terminal.run_command"],
            ),
        );

        let receipt = runtime
            .run_command(
                vec![
                    "python".to_string(),
                    "-m".to_string(),
                    "http.server".to_string(),
                    "8000".to_string(),
                ],
                1_000,
            )
            .unwrap();

        assert_eq!(receipt.status, "blocked");
        assert_eq!(
            receipt.data["reason_code"],
            "service_command_requires_start_service"
        );
        assert_eq!(
            receipt.data["required_capability"],
            "terminal.start_service"
        );
    }

    #[test]
    fn service_lifecycle_start_status_stop_records_receipts() {
        let (workspace, state) = temp_paths("service_lifecycle");
        let (job, process, truth) =
            create_agent_job_with_state_root(&workspace, &state, "Service lifecycle").unwrap();
        let runtime = TerminalRuntime::new(
            WorkspaceGuard::new(&workspace).unwrap(),
            truth.clone(),
            token(
                job.job_id,
                process.pid,
                &workspace,
                vec![
                    "terminal.start_service",
                    "terminal.stop_service",
                    "terminal.service_status",
                ],
            ),
        );

        let start = runtime
            .start_service("dev_server", service_command(), 2_000, None, vec![])
            .unwrap();
        assert_eq!(start.status, "success");
        assert_eq!(start.data["service_id"], "dev_server");
        assert_eq!(start.data["status"], "running");

        let status = runtime.service_status("dev_server").unwrap();
        assert_eq!(status.status, "success");
        assert_eq!(status.data["service_id"], "dev_server");

        let stop = runtime
            .stop_service("dev_server", Some("test cleanup"))
            .unwrap();
        assert_eq!(stop.status, "success");
        assert_eq!(stop.data["status"], "stopped");

        let events = truth.read_events().unwrap();
        assert!(events
            .iter()
            .any(|event| event.event_type == "terminal_service_started"));
        assert!(events
            .iter()
            .any(|event| event.event_type == "terminal_service_stopped"));
    }

    #[test]
    fn root_process_cancel_job_stops_running_services() {
        let (workspace, state) = temp_paths("job_stop");
        let (job, process, truth) =
            create_agent_job_with_state_root(&workspace, &state, "Stop services").unwrap();
        let job_id = job.job_id.clone();
        let runtime = TerminalRuntime::new(
            WorkspaceGuard::new(&workspace).unwrap(),
            truth.clone(),
            token(
                job.job_id.clone(),
                process.pid.clone(),
                &workspace,
                vec!["terminal.start_service"],
            ),
        );
        let start = runtime
            .start_service("job_service", service_command(), 2_000, None, vec![])
            .unwrap();
        assert_eq!(start.status, "success");

        crate::RootAgentProcessController::new_with_state_root(&workspace, &state)
            .unwrap()
            .cancel_job(&job_id, "job cancelled")
            .unwrap();

        let record = read_service_record(&truth, "job_service")
            .unwrap()
            .expect("service record exists");
        assert_eq!(record.status, "stopped");
        let events = truth.read_events().unwrap();
        assert!(events
            .iter()
            .any(|event| event.event_type == "terminal_service_stopped"));
        assert!(events
            .iter()
            .any(|event| event.event_type == "job_cancelled"));
    }
}
