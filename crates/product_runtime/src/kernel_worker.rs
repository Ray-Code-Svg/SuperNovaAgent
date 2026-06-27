use std::io::{self, BufRead, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use local_runtime_protocol::{
    ArtifactDestinationGuidance, ContextPack, DisplayLanguage, ModelConfig, SourceGuidance,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use supernova_process_kernel::{ModelStreamDelta, ModelStreamSink};

use crate::kernel::KernelBridge;

pub const KERNEL_WORKER_ARG: &str = "--supernova-kernel-worker";
pub const KERNEL_WORKER_EVENT_SCHEMA: &str = "supernova.kernel_worker.event.v1";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum KernelWorkerCommand {
    StartTask {
        run_id: String,
        workspace_root: String,
        state_root: String,
        provider_profile_root: String,
        container_id: String,
        goal: String,
        context_pack_id: Option<String>,
        source_guidance: Option<SourceGuidance>,
        artifact_destination: Option<ArtifactDestinationGuidance>,
        model_config: Option<ModelConfig>,
        #[serde(default = "default_display_language")]
        response_language: DisplayLanguage,
        auto_approve: bool,
    },
    StartChat {
        run_id: String,
        workspace_root: String,
        state_root: String,
        provider_profile_root: String,
        container_id: String,
        chat_thread_id: Option<String>,
        message: String,
        context_pack: Option<ContextPack>,
        source_guidance: Option<SourceGuidance>,
        model_config: Option<ModelConfig>,
        #[serde(default = "default_display_language")]
        response_language: DisplayLanguage,
    },
    ProbeSleep {
        run_id: String,
        duration_ms: u64,
    },
    ProbeTerminalThenSleep {
        run_id: String,
        duration_ms: u64,
    },
    ProbeCrash {
        run_id: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct KernelWorkerEvent {
    pub schema_version: String,
    pub run_id: String,
    pub event_type: String,
    pub payload: Value,
}

impl KernelWorkerEvent {
    pub fn new(run_id: impl Into<String>, event_type: impl Into<String>, payload: Value) -> Self {
        Self {
            schema_version: KERNEL_WORKER_EVENT_SCHEMA.into(),
            run_id: run_id.into(),
            event_type: event_type.into(),
            payload,
        }
    }
}

pub fn run_kernel_worker_from_args_if_requested() -> bool {
    if !std::env::args().any(|arg| arg == KERNEL_WORKER_ARG) {
        return false;
    }
    let code = match run_kernel_worker_stdio() {
        Ok(code) => code,
        Err(err) => {
            let payload = KernelWorkerEvent::new(
                "unknown",
                "worker.failed",
                json!({"error": err.to_string()}),
            );
            println!(
                "{}",
                serde_json::to_string(&payload).unwrap_or_else(|_| {
                    "{\"schema_version\":\"supernova.kernel_worker.event.v1\",\"run_id\":\"unknown\",\"event_type\":\"worker.failed\",\"payload\":{\"error\":\"serialization failed\"}}".into()
                })
            );
            1
        }
    };
    std::process::exit(code);
}

pub fn run_kernel_worker_stdio() -> io::Result<i32> {
    let mut line = String::new();
    io::stdin().lock().read_line(&mut line)?;
    if line.trim().is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "kernel worker command JSONL was empty",
        ));
    }
    let command: KernelWorkerCommand = serde_json::from_str(&line)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidInput, err))?;
    let run_id = command.run_id().to_string();
    let writer = WorkerEventWriter::new(run_id.clone());
    writer.emit("worker.started", json!({"pid": std::process::id()}))?;
    let heartbeat = WorkerHeartbeat::start(writer.clone());
    let result = run_worker_command(command, writer.clone());
    heartbeat.stop();
    match result {
        Ok(()) => Ok(0),
        Err(err) => {
            let _ = writer.emit("worker.failed", json!({"error": err.to_string()}));
            Ok(1)
        }
    }
}

fn run_worker_command(command: KernelWorkerCommand, writer: WorkerEventWriter) -> io::Result<()> {
    match command {
        KernelWorkerCommand::StartTask {
            run_id: _,
            workspace_root,
            state_root,
            provider_profile_root,
            container_id,
            goal,
            context_pack_id,
            source_guidance,
            artifact_destination,
            model_config,
            response_language,
            auto_approve,
        } => {
            let bridge = KernelBridge::new(
                workspace_root.into(),
                state_root.into(),
                provider_profile_root.into(),
            );
            let started_writer = writer.clone();
            let stream_sink = Arc::new(WorkerStreamSink::new(writer.clone()));
            let result = bridge
                .start_task_in_container_with_started_and_stream_sink_and_response_language(
                    &container_id,
                    &goal,
                    context_pack_id,
                    source_guidance,
                    artifact_destination,
                    model_config,
                    auto_approve,
                    move |job_id, root_pid| {
                        started_writer.emit(
                            "task.started",
                            json!({"job_id": job_id, "root_pid": root_pid}),
                        )
                    },
                    Some(stream_sink),
                    response_language,
                )?;
            writer.emit(
                "task.completed",
                serde_json::to_value(result).map_err(json_err)?,
            )?;
            Ok(())
        }
        KernelWorkerCommand::StartChat {
            run_id: _,
            workspace_root,
            state_root,
            provider_profile_root,
            container_id,
            chat_thread_id,
            message,
            context_pack,
            source_guidance,
            model_config,
            response_language,
        } => {
            let bridge = KernelBridge::new(
                workspace_root.into(),
                state_root.into(),
                provider_profile_root.into(),
            );
            let stream_sink = Arc::new(WorkerStreamSink::new(writer.clone()));
            let result = bridge.start_chat_turn_with_stream_sink_and_response_language(
                &container_id,
                chat_thread_id,
                message,
                context_pack,
                source_guidance,
                model_config,
                stream_sink,
                response_language,
            )?;
            writer.emit(
                "chat.completed",
                serde_json::to_value(result).map_err(json_err)?,
            )?;
            Ok(())
        }
        KernelWorkerCommand::ProbeSleep {
            run_id: _,
            duration_ms,
        } => {
            thread::sleep(Duration::from_millis(duration_ms));
            writer.emit("probe.completed", json!({"status": "completed"}))?;
            Ok(())
        }
        KernelWorkerCommand::ProbeTerminalThenSleep {
            run_id: _,
            duration_ms,
        } => {
            writer.emit("probe.completed", json!({"status": "completed"}))?;
            thread::sleep(Duration::from_millis(duration_ms));
            Ok(())
        }
        KernelWorkerCommand::ProbeCrash { run_id: _ } => {
            writer.emit("probe.started", json!({"status": "crashing"}))?;
            std::process::exit(70);
        }
    }
}

impl KernelWorkerCommand {
    fn run_id(&self) -> &str {
        match self {
            KernelWorkerCommand::StartTask { run_id, .. }
            | KernelWorkerCommand::StartChat { run_id, .. }
            | KernelWorkerCommand::ProbeSleep { run_id, .. }
            | KernelWorkerCommand::ProbeTerminalThenSleep { run_id, .. }
            | KernelWorkerCommand::ProbeCrash { run_id } => run_id,
        }
    }
}

#[derive(Clone)]
struct WorkerEventWriter {
    run_id: String,
    stdout: Arc<Mutex<io::Stdout>>,
}

impl WorkerEventWriter {
    fn new(run_id: String) -> Self {
        Self {
            run_id,
            stdout: Arc::new(Mutex::new(io::stdout())),
        }
    }

    fn emit(&self, event_type: &str, payload: Value) -> io::Result<()> {
        let event = KernelWorkerEvent::new(self.run_id.clone(), event_type, payload);
        let mut stdout = self.stdout.lock().map_err(|_| {
            io::Error::new(io::ErrorKind::Other, "kernel worker stdout lock poisoned")
        })?;
        serde_json::to_writer(&mut *stdout, &event).map_err(json_err)?;
        stdout.write_all(b"\n")?;
        stdout.flush()
    }
}

struct WorkerHeartbeat {
    running: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl WorkerHeartbeat {
    fn start(writer: WorkerEventWriter) -> Self {
        let running = Arc::new(AtomicBool::new(true));
        let thread_running = running.clone();
        let handle = thread::spawn(move || {
            while thread_running.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_millis(1000));
                if thread_running.load(Ordering::Relaxed) {
                    let _ = writer.emit("worker.heartbeat", json!({"pid": std::process::id()}));
                }
            }
        });
        Self {
            running,
            handle: Some(handle),
        }
    }

    fn stop(mut self) {
        self.running.store(false, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

#[derive(Debug)]
struct WorkerStreamSink {
    writer: WorkerEventWriter,
}

impl WorkerStreamSink {
    fn new(writer: WorkerEventWriter) -> Self {
        Self { writer }
    }
}

impl ModelStreamSink for WorkerStreamSink {
    fn on_model_stream_delta(&self, delta: ModelStreamDelta) {
        let _ = self.writer.emit(
            "model.stream_delta",
            serde_json::to_value(delta).unwrap_or_else(|_| json!({})),
        );
    }
}

impl std::fmt::Debug for WorkerEventWriter {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("WorkerEventWriter")
            .field("run_id", &self.run_id)
            .finish_non_exhaustive()
    }
}

fn json_err(err: serde_json::Error) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, err)
}

fn default_display_language() -> DisplayLanguage {
    DisplayLanguage::EnUs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kernel_worker_commands_roundtrip_response_language() {
        let chat = KernelWorkerCommand::StartChat {
            run_id: "run_chat".into(),
            workspace_root: "workspace".into(),
            state_root: "state".into(),
            provider_profile_root: "provider".into(),
            container_id: "container".into(),
            chat_thread_id: Some("chat".into()),
            message: "hello".into(),
            context_pack: None,
            source_guidance: None,
            model_config: None,
            response_language: DisplayLanguage::ZhCn,
        };
        let task = KernelWorkerCommand::StartTask {
            run_id: "run_task".into(),
            workspace_root: "workspace".into(),
            state_root: "state".into(),
            provider_profile_root: "provider".into(),
            container_id: "container".into(),
            goal: "write report".into(),
            context_pack_id: None,
            source_guidance: None,
            artifact_destination: None,
            model_config: None,
            response_language: DisplayLanguage::EnUs,
            auto_approve: false,
        };

        let decoded_chat: KernelWorkerCommand =
            serde_json::from_str(&serde_json::to_string(&chat).unwrap()).unwrap();
        let decoded_task: KernelWorkerCommand =
            serde_json::from_str(&serde_json::to_string(&task).unwrap()).unwrap();

        match decoded_chat {
            KernelWorkerCommand::StartChat {
                response_language, ..
            } => assert_eq!(response_language, DisplayLanguage::ZhCn),
            other => panic!("unexpected command: {other:?}"),
        }
        match decoded_task {
            KernelWorkerCommand::StartTask {
                response_language, ..
            } => assert_eq!(response_language, DisplayLanguage::EnUs),
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn kernel_worker_commands_default_legacy_response_language_to_english() {
        let chat: KernelWorkerCommand = serde_json::from_value(json!({
            "command": "start_chat",
            "run_id": "run_chat",
            "workspace_root": "workspace",
            "state_root": "state",
            "provider_profile_root": "provider",
            "container_id": "container",
            "chat_thread_id": null,
            "message": "hello",
            "context_pack": null,
            "source_guidance": null,
            "model_config": null
        }))
        .unwrap();

        match chat {
            KernelWorkerCommand::StartChat {
                response_language, ..
            } => assert_eq!(response_language, DisplayLanguage::EnUs),
            other => panic!("unexpected command: {other:?}"),
        }
    }
}
