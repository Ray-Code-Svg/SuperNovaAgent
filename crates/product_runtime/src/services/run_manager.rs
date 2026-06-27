use std::collections::HashMap;
use std::io::{self, BufRead, Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use local_runtime_protocol::{
    ArtifactDestinationGuidance, ContextPack, DisplayLanguage, ModelConfig, RunRecord,
    SourceGuidance,
};
use serde_json::{json, Value};
use supernova_process_kernel::{ChatTurnResult, ModelStreamDelta, TaskAgentRunResult};

use crate::kernel_worker::{KernelWorkerCommand, KernelWorkerEvent, KERNEL_WORKER_ARG};
use crate::state::product_db::ProductDb;
use crate::state::projection_shards::{ProjectionShardDb, ProjectionShardManager};
use crate::state::run_registry::NewRun;

static WORKER_EVENT_COUNTER: AtomicU64 = AtomicU64::new(1);

#[derive(Clone)]
pub struct InProcessKernelWorker {
    worker_id: String,
}

impl InProcessKernelWorker {
    pub fn current() -> Self {
        Self {
            worker_id: format!("in_process:{}", std::process::id()),
        }
    }

    pub fn worker_id(&self) -> &str {
        &self.worker_id
    }
}

impl ProcessKernelWorker {
    pub fn current(
        workspace_root: PathBuf,
        state_root: PathBuf,
        provider_profile_root: PathBuf,
    ) -> io::Result<Self> {
        Ok(Self {
            executable_path: std::env::current_exe()?,
            workspace_root,
            state_root,
            provider_profile_root,
        })
    }
}

#[derive(Clone)]
pub struct RunManager {
    db: ProductDb,
    worker: InProcessKernelWorker,
    process_worker: Option<ProcessKernelWorker>,
    active_processes: Arc<Mutex<HashMap<String, ActiveProcess>>>,
}

#[derive(Clone)]
pub struct ProcessKernelWorker {
    executable_path: PathBuf,
    workspace_root: PathBuf,
    state_root: PathBuf,
    provider_profile_root: PathBuf,
}

#[derive(Clone)]
struct ActiveProcess {
    child: Arc<Mutex<Child>>,
}

#[derive(Clone, Debug)]
pub struct TaskWorkerRequest {
    pub container_id: String,
    pub goal: String,
    pub context_pack_id: Option<String>,
    pub source_guidance: Option<SourceGuidance>,
    pub artifact_destination: Option<ArtifactDestinationGuidance>,
    pub model_config: Option<ModelConfig>,
    pub auto_approve: bool,
    pub response_language: DisplayLanguage,
}

#[derive(Clone, Debug)]
pub struct ChatWorkerRequest {
    pub container_id: String,
    pub chat_thread_id: Option<String>,
    pub message: String,
    pub context_pack: Option<ContextPack>,
    pub source_guidance: Option<SourceGuidance>,
    pub model_config: Option<ModelConfig>,
    pub response_language: DisplayLanguage,
}

#[derive(Clone, Debug)]
pub enum ProcessWorkerEvent {
    WorkerStarted { pid: u32 },
    Heartbeat,
    TaskStarted { job_id: String, root_pid: String },
    ModelStreamDelta(ModelStreamDelta),
}

#[derive(Debug)]
enum WorkerTerminalResult {
    Task(TaskAgentRunResult),
    Chat(ChatTurnResult),
    Probe,
}

impl RunManager {
    pub fn new(db: ProductDb) -> Self {
        Self {
            db,
            worker: InProcessKernelWorker::current(),
            process_worker: None,
            active_processes: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn with_process_worker(
        db: ProductDb,
        workspace_root: PathBuf,
        state_root: PathBuf,
        provider_profile_root: PathBuf,
    ) -> Self {
        let process_worker =
            ProcessKernelWorker::current(workspace_root, state_root, provider_profile_root).ok();
        Self {
            db,
            worker: InProcessKernelWorker::current(),
            process_worker,
            active_processes: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn with_process_worker_executable(
        db: ProductDb,
        executable_path: PathBuf,
        workspace_root: PathBuf,
        state_root: PathBuf,
        provider_profile_root: PathBuf,
    ) -> Self {
        Self {
            db,
            worker: InProcessKernelWorker::current(),
            process_worker: Some(ProcessKernelWorker {
                executable_path,
                workspace_root,
                state_root,
                provider_profile_root,
            }),
            active_processes: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn list_runs(&self, container_id: Option<&str>) -> rusqlite::Result<Vec<RunRecord>> {
        self.db.list_runs(container_id)
    }

    pub fn process_worker_enabled(&self) -> bool {
        self.process_worker.is_some()
    }

    pub fn start_chat_run(
        &self,
        container_id: &str,
        chat_thread_id: &str,
    ) -> rusqlite::Result<RunRecord> {
        let run = self.db.insert_run(NewRun {
            container_id: container_id.to_string(),
            run_kind: "chat".into(),
            chat_thread_id: Some(chat_thread_id.to_string()),
            task_id: None,
            job_id: None,
            worker_id: self.initial_worker_id(),
        })?;
        self.mark_running(&run.run_id)
    }

    pub fn start_task_run(&self, container_id: &str) -> rusqlite::Result<RunRecord> {
        let run = self.db.insert_run(NewRun {
            container_id: container_id.to_string(),
            run_kind: "task".into(),
            chat_thread_id: None,
            task_id: None,
            job_id: None,
            worker_id: self.initial_worker_id(),
        })?;
        self.mark_running(&run.run_id)
    }

    fn initial_worker_id(&self) -> String {
        if self.process_worker.is_some() {
            "process:pending".into()
        } else {
            self.worker.worker_id().to_string()
        }
    }

    pub fn mark_running(&self, run_id: &str) -> rusqlite::Result<RunRecord> {
        let run = retry_sqlite_busy(|| self.db.update_run_status(run_id, "running", None))?;
        let _ = retry_sqlite_busy(|| self.db.heartbeat_run(run_id))?;
        self.append_run_event(&run, "run.running")
    }

    pub fn bind_task_run(
        &self,
        run_id: &str,
        task_id: &str,
        job_id: &str,
    ) -> rusqlite::Result<RunRecord> {
        let run = retry_sqlite_busy(|| self.db.bind_run_task(run_id, task_id, job_id))?;
        self.append_run_event(&run, "run.bound")
    }

    pub fn complete_run(&self, run_id: &str, status: &str) -> rusqlite::Result<RunRecord> {
        let status = run_terminal_status(status);
        let run = retry_sqlite_busy(|| self.db.update_run_status(run_id, status, None))?;
        let event_type = if status == "cancelled" {
            "run.cancelled"
        } else {
            "run.completed"
        };
        self.append_run_event(&run, event_type)
    }

    pub fn fail_run(&self, run_id: &str, error: &str) -> rusqlite::Result<RunRecord> {
        let run = retry_sqlite_busy(|| self.db.update_run_status(run_id, "failed", Some(error)))?;
        self.append_run_event(&run, "run.failed")
    }

    pub fn request_cancel_task_run(&self, task_id: &str) -> rusqlite::Result<Vec<RunRecord>> {
        let runs = retry_sqlite_busy(|| self.db.request_cancel_task_runs(task_id))?;
        for run in &runs {
            let _ = self.kill_run_process(&run.run_id);
            let _ = self.append_run_event(run, "run.cancel_requested")?;
        }
        Ok(runs)
    }

    pub fn request_cancel_chat_run(
        &self,
        chat_thread_id: &str,
    ) -> rusqlite::Result<Vec<RunRecord>> {
        let runs = retry_sqlite_busy(|| self.db.request_cancel_chat_runs(chat_thread_id))?;
        for run in &runs {
            let _ = self.kill_run_process(&run.run_id);
            let _ = self.append_run_event(run, "run.cancel_requested")?;
        }
        Ok(runs)
    }

    pub fn run_task_in_process_worker<F>(
        &self,
        run_id: &str,
        request: TaskWorkerRequest,
        on_event: F,
    ) -> io::Result<TaskAgentRunResult>
    where
        F: FnMut(ProcessWorkerEvent) -> io::Result<()>,
    {
        let Some(worker) = self.process_worker.clone() else {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "process kernel worker is not configured",
            ));
        };
        let terminal = self.run_worker_command(
            run_id,
            KernelWorkerCommand::StartTask {
                run_id: run_id.to_string(),
                workspace_root: worker.workspace_root.to_string_lossy().to_string(),
                state_root: worker.state_root.to_string_lossy().to_string(),
                provider_profile_root: worker.provider_profile_root.to_string_lossy().to_string(),
                container_id: request.container_id,
                goal: request.goal,
                context_pack_id: request.context_pack_id,
                source_guidance: request.source_guidance,
                artifact_destination: request.artifact_destination,
                model_config: request.model_config,
                auto_approve: request.auto_approve,
                response_language: request.response_language,
            },
            on_event,
        )?;
        match terminal {
            WorkerTerminalResult::Task(result) => Ok(result),
            other => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("worker returned unexpected terminal event: {other:?}"),
            )),
        }
    }

    pub fn run_chat_in_process_worker<F>(
        &self,
        run_id: &str,
        request: ChatWorkerRequest,
        on_event: F,
    ) -> io::Result<ChatTurnResult>
    where
        F: FnMut(ProcessWorkerEvent) -> io::Result<()>,
    {
        let Some(worker) = self.process_worker.clone() else {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "process kernel worker is not configured",
            ));
        };
        let terminal = self.run_worker_command(
            run_id,
            KernelWorkerCommand::StartChat {
                run_id: run_id.to_string(),
                workspace_root: worker.workspace_root.to_string_lossy().to_string(),
                state_root: worker.state_root.to_string_lossy().to_string(),
                provider_profile_root: worker.provider_profile_root.to_string_lossy().to_string(),
                container_id: request.container_id,
                chat_thread_id: request.chat_thread_id,
                message: request.message,
                context_pack: request.context_pack,
                source_guidance: request.source_guidance,
                model_config: request.model_config,
                response_language: request.response_language,
            },
            on_event,
        )?;
        match terminal {
            WorkerTerminalResult::Chat(result) => Ok(result),
            other => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("worker returned unexpected terminal event: {other:?}"),
            )),
        }
    }

    pub fn run_probe_sleep_in_process_worker(
        &self,
        run_id: &str,
        duration_ms: u64,
    ) -> io::Result<()> {
        let terminal = self.run_worker_command(
            run_id,
            KernelWorkerCommand::ProbeSleep {
                run_id: run_id.to_string(),
                duration_ms,
            },
            |_| Ok(()),
        )?;
        match terminal {
            WorkerTerminalResult::Probe => {
                let _ = self.complete_run(run_id, "completed").map_err(sqlite_err)?;
                Ok(())
            }
            other => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("probe returned unexpected terminal event: {other:?}"),
            )),
        }
    }

    pub fn run_probe_terminal_then_sleep_in_process_worker(
        &self,
        run_id: &str,
        duration_ms: u64,
    ) -> io::Result<()> {
        let terminal = self.run_worker_command(
            run_id,
            KernelWorkerCommand::ProbeTerminalThenSleep {
                run_id: run_id.to_string(),
                duration_ms,
            },
            |_| Ok(()),
        )?;
        match terminal {
            WorkerTerminalResult::Probe => {
                let _ = self.complete_run(run_id, "completed").map_err(sqlite_err)?;
                Ok(())
            }
            other => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("probe returned unexpected terminal event: {other:?}"),
            )),
        }
    }

    pub fn run_probe_crash_in_process_worker(&self, run_id: &str) -> io::Result<()> {
        let _ = self.run_worker_command(
            run_id,
            KernelWorkerCommand::ProbeCrash {
                run_id: run_id.to_string(),
            },
            |_| Ok(()),
        )?;
        Ok(())
    }

    fn run_worker_command<F>(
        &self,
        run_id: &str,
        command: KernelWorkerCommand,
        mut on_event: F,
    ) -> io::Result<WorkerTerminalResult>
    where
        F: FnMut(ProcessWorkerEvent) -> io::Result<()>,
    {
        let worker = self.process_worker.clone().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::Unsupported,
                "process kernel worker is not configured",
            )
        })?;
        let mut child = Command::new(&worker.executable_path)
            .arg(KERNEL_WORKER_ARG)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let pid = child.id();
        retry_sqlite_busy(|| self.db.update_run_worker(run_id, &format!("process:{pid}")))
            .map_err(sqlite_err)?;
        let mut stdin = child.stdin.take().ok_or_else(|| {
            io::Error::new(io::ErrorKind::BrokenPipe, "worker stdin was not available")
        })?;
        serde_json::to_writer(&mut stdin, &command).map_err(json_err)?;
        stdin.write_all(b"\n")?;
        drop(stdin);

        let stdout = child.stdout.take().ok_or_else(|| {
            io::Error::new(io::ErrorKind::BrokenPipe, "worker stdout was not available")
        })?;
        let stderr = child.stderr.take();
        let stderr_handle = stderr.map(|mut stderr| {
            std::thread::spawn(move || {
                let mut text = String::new();
                let _ = stderr.read_to_string(&mut text);
                text
            })
        });
        let child = Arc::new(Mutex::new(child));
        self.active_processes
            .lock()
            .map_err(|_| io::Error::new(io::ErrorKind::Other, "active process lock poisoned"))?
            .insert(
                run_id.to_string(),
                ActiveProcess {
                    child: child.clone(),
                },
            );

        let mut terminal: Option<WorkerTerminalResult> = None;
        let mut worker_error: Option<String> = None;
        on_event(ProcessWorkerEvent::WorkerStarted { pid })?;
        for line in io::BufReader::new(stdout).lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let event = serde_json::from_str::<KernelWorkerEvent>(&line).map_err(json_err)?;
            if let Err(err) = self.append_worker_event(run_id, &event) {
                if !is_sqlite_lock_error(&err) {
                    return Err(sqlite_err(err));
                }
            }
            match event.event_type.as_str() {
                "worker.started" => {}
                "worker.heartbeat" => {
                    if let Err(err) = retry_sqlite_busy(|| self.db.heartbeat_run(run_id)) {
                        if !is_sqlite_lock_error(&err) {
                            return Err(sqlite_err(err));
                        }
                    }
                    on_event(ProcessWorkerEvent::Heartbeat)?;
                }
                "task.started" => {
                    let job_id = event
                        .payload
                        .get("job_id")
                        .and_then(Value::as_str)
                        .ok_or_else(|| {
                            io::Error::new(
                                io::ErrorKind::InvalidData,
                                "task.started missing job_id",
                            )
                        })?
                        .to_string();
                    let root_pid = event
                        .payload
                        .get("root_pid")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    on_event(ProcessWorkerEvent::TaskStarted { job_id, root_pid })?;
                }
                "model.stream_delta" => {
                    let delta = serde_json::from_value::<ModelStreamDelta>(event.payload)
                        .map_err(json_err)?;
                    on_event(ProcessWorkerEvent::ModelStreamDelta(delta))?;
                }
                "task.completed" => {
                    terminal = Some(WorkerTerminalResult::Task(
                        serde_json::from_value::<TaskAgentRunResult>(event.payload)
                            .map_err(json_err)?,
                    ));
                    break;
                }
                "chat.completed" => {
                    terminal = Some(WorkerTerminalResult::Chat(
                        serde_json::from_value::<ChatTurnResult>(event.payload)
                            .map_err(json_err)?,
                    ));
                    break;
                }
                "probe.completed" => {
                    terminal = Some(WorkerTerminalResult::Probe);
                    break;
                }
                "worker.failed" => {
                    worker_error = Some(
                        event
                            .payload
                            .get("error")
                            .and_then(Value::as_str)
                            .unwrap_or("worker failed")
                            .to_string(),
                    );
                }
                _ => {}
            }
        }

        if let Some(terminal) = terminal {
            let child_exited = finish_child_after_terminal(&child).unwrap_or(false);
            self.active_processes
                .lock()
                .map_err(|_| io::Error::new(io::ErrorKind::Other, "active process lock poisoned"))?
                .remove(run_id);
            if child_exited {
                append_finished_worker_stderr(&self.db, run_id, stderr_handle);
            }
            let run = retry_sqlite_busy(|| self.db.get_run(run_id)).map_err(sqlite_err)?;
            if run.cancel_requested {
                let _ = self.complete_run(run_id, "cancelled").map_err(sqlite_err)?;
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    format!("worker {run_id} completed after cancellation was requested"),
                ));
            }
            return Ok(terminal);
        }

        let status = child
            .lock()
            .map_err(|_| io::Error::new(io::ErrorKind::Other, "worker child lock poisoned"))?
            .wait()?;
        self.active_processes
            .lock()
            .map_err(|_| io::Error::new(io::ErrorKind::Other, "active process lock poisoned"))?
            .remove(run_id);
        append_finished_worker_stderr(&self.db, run_id, stderr_handle);
        if status.success() {
            let run = retry_sqlite_busy(|| self.db.get_run(run_id)).map_err(sqlite_err)?;
            if run.cancel_requested {
                let _ = self.complete_run(run_id, "cancelled").map_err(sqlite_err)?;
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    format!("worker {run_id} completed after cancellation was requested"),
                ));
            }
            terminal.ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "worker exited successfully without a terminal event",
                )
            })
        } else {
            let run = retry_sqlite_busy(|| self.db.get_run(run_id)).map_err(sqlite_err)?;
            if run.cancel_requested {
                let _ = self.complete_run(run_id, "cancelled").map_err(sqlite_err)?;
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    format!("worker {run_id} was cancelled"),
                ));
            }
            let message = worker_error.unwrap_or_else(|| {
                format!(
                    "worker process exited with status {}",
                    status
                        .code()
                        .map(|code| code.to_string())
                        .unwrap_or_else(|| "terminated".into())
                )
            });
            let _ = self.fail_run(run_id, &message).map_err(sqlite_err)?;
            Err(io::Error::new(io::ErrorKind::Other, message))
        }
    }

    fn kill_run_process(&self, run_id: &str) -> io::Result<bool> {
        let active = self
            .active_processes
            .lock()
            .map_err(|_| io::Error::new(io::ErrorKind::Other, "active process lock poisoned"))?
            .get(run_id)
            .cloned();
        let Some(active) = active else {
            return Ok(false);
        };
        let mut child = active
            .child
            .lock()
            .map_err(|_| io::Error::new(io::ErrorKind::Other, "worker child lock poisoned"))?;
        let pid = child.id();
        match child.kill() {
            Ok(()) => {}
            Err(err) if err.kind() == io::ErrorKind::InvalidInput => {}
            Err(err) => return Err(err),
        }
        if wait_for_child_exit(&mut child, Duration::from_millis(1500))? {
            return Ok(true);
        }
        #[cfg(windows)]
        {
            let _ = Command::new("taskkill")
                .args(["/PID", &pid.to_string(), "/T", "/F"])
                .status();
            if wait_for_child_exit(&mut child, Duration::from_millis(1500))? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn append_worker_event(&self, run_id: &str, event: &KernelWorkerEvent) -> rusqlite::Result<()> {
        let run = self.db.get_run(run_id).ok();
        let event_id = format!("{}:{run_id}:{}", event.event_type, next_worker_event_seq());
        if let Some(run) = run.as_ref() {
            if let Some(shard) = self.projection_shard_for_run(run)? {
                shard.append_runtime_event(&event_id, &event.event_type, event.payload.clone())?;
                return Ok(());
            }
        }
        let _ = self.db.append_runtime_event(
            &event_id,
            run.as_ref()
                .map(|record| format!("{}/{}", record.workspace_uid, record.container_id))
                .as_deref()
                .unwrap_or("worker/event"),
            run.as_ref().map(|record| record.container_id.as_str()),
            Some(run_id),
            run.as_ref().and_then(|record| record.task_id.as_deref()),
            run.as_ref()
                .and_then(|record| record.chat_thread_id.as_deref()),
            &event.event_type,
            event.payload.clone(),
        )?;
        Ok(())
    }

    fn append_run_event(&self, run: &RunRecord, event_type: &str) -> rusqlite::Result<RunRecord> {
        let payload = serde_json::to_value(run).unwrap_or_else(|_| json!({}));
        if let Err(err) = retry_sqlite_busy(|| {
            self.db.append_runtime_event(
                &format!("{event_type}:{}:{}", run.run_id, run.updated_at_ms),
                &format!("{}/{}", run.workspace_uid, run.container_id),
                Some(&run.container_id),
                Some(&run.run_id),
                run.task_id.as_deref(),
                run.chat_thread_id.as_deref(),
                event_type,
                payload.clone(),
            )
        }) {
            if !is_sqlite_lock_error(&err) {
                return Err(err);
            }
        }
        Ok(run.clone())
    }

    fn projection_shard_for_run(
        &self,
        run: &RunRecord,
    ) -> rusqlite::Result<Option<ProjectionShardDb>> {
        let manager = ProjectionShardManager::for_product_db(&self.db);
        if let Some(chat_thread_id) = run.chat_thread_id.as_deref() {
            if let Some(record) = self.db.projection_shard_for_chat_thread(chat_thread_id)? {
                return manager.open_existing_shard(&record);
            }
        }
        if let Some(job_id) = run.job_id.as_deref() {
            if let Some(record) = self.db.projection_shard_for_task_job(job_id)? {
                return manager.open_existing_shard(&record);
            }
        }
        Ok(None)
    }
}

fn run_terminal_status(status: &str) -> &str {
    match status {
        "completed" => "completed",
        "cancelled" | "canceled" => "cancelled",
        "blocked" => "blocked",
        "waiting_approval" | "waiting_user" | "running" => status,
        _ => "failed",
    }
}

fn wait_for_child_exit(child: &mut Child, timeout: Duration) -> io::Result<bool> {
    let started = Instant::now();
    while started.elapsed() < timeout {
        if child.try_wait()?.is_some() {
            return Ok(true);
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    Ok(child.try_wait()?.is_some())
}

fn finish_child_after_terminal(child: &Arc<Mutex<Child>>) -> io::Result<bool> {
    let mut child = child
        .lock()
        .map_err(|_| io::Error::new(io::ErrorKind::Other, "worker child lock poisoned"))?;
    if wait_for_child_exit(&mut child, Duration::from_millis(50))? {
        return Ok(true);
    }
    match child.kill() {
        Ok(()) => {}
        Err(err) if err.kind() == io::ErrorKind::InvalidInput => return Ok(true),
        Err(err) => return Err(err),
    }
    wait_for_child_exit(&mut child, Duration::from_millis(500))
}

fn append_finished_worker_stderr(
    db: &ProductDb,
    run_id: &str,
    stderr_handle: Option<std::thread::JoinHandle<String>>,
) {
    let Some(handle) = stderr_handle else {
        return;
    };
    let stderr_text = handle.join().unwrap_or_default();
    if stderr_text.trim().is_empty() {
        return;
    }
    let _ = db.append_runtime_event(
        &format!("worker.stderr:{run_id}:{}", next_worker_event_seq()),
        "worker/stderr",
        None,
        Some(run_id),
        None,
        None,
        "worker.stderr",
        json!({"stderr": stderr_text}),
    );
}

fn next_worker_event_seq() -> u64 {
    WORKER_EVENT_COUNTER.fetch_add(1, Ordering::Relaxed)
}

fn sqlite_err(err: rusqlite::Error) -> io::Error {
    io::Error::new(io::ErrorKind::Other, err)
}

fn retry_sqlite_busy<T, F>(mut op: F) -> rusqlite::Result<T>
where
    F: FnMut() -> rusqlite::Result<T>,
{
    let mut delay = Duration::from_millis(25);
    let mut last_busy = None;
    for _ in 0..8 {
        match op() {
            Ok(value) => return Ok(value),
            Err(err) if is_sqlite_lock_error(&err) => {
                last_busy = Some(err);
                std::thread::sleep(delay);
                delay = (delay * 2).min(Duration::from_millis(400));
            }
            Err(err) => return Err(err),
        }
    }
    Err(last_busy.unwrap_or_else(|| {
        rusqlite::Error::InvalidParameterName("sqlite busy retry exhausted".into())
    }))
}

fn is_sqlite_lock_error(err: &rusqlite::Error) -> bool {
    matches!(
        err,
        rusqlite::Error::SqliteFailure(error, _)
            if matches!(
                error.code,
                rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
            )
    )
}

fn json_err(err: serde_json::Error) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, err)
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::PathBuf;

    use crate::state::workspace_registry::now_ms;

    fn temp_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("supernova_run_manager_{name}_{}", now_ms()));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn test_manager(name: &str) -> RunManager {
        let db = ProductDb::open(&temp_root(name), format!("workspace_{name}")).unwrap();
        RunManager::new(db)
    }

    #[test]
    fn run_manager_records_in_process_worker_identity() {
        let manager = test_manager("worker_identity");
        let run = manager.start_chat_run("container_a", "chat_a").unwrap();

        assert_eq!(run.run_kind, "chat");
        assert_eq!(run.status, "running");
        assert!(run.worker_id.starts_with("in_process:"));
        assert_eq!(manager.list_runs(Some("container_a")).unwrap().len(), 1);
    }

    #[test]
    fn run_manager_cancel_request_does_not_close_unrelated_runs() {
        let manager = test_manager("cancel_scope");
        let first = manager.start_task_run("container_a").unwrap();
        let second = manager.start_task_run("container_b").unwrap();
        manager
            .bind_task_run(&first.run_id, "task_a", "task_a")
            .unwrap();
        manager
            .bind_task_run(&second.run_id, "task_b", "task_b")
            .unwrap();

        let cancelled = manager.request_cancel_task_run("task_a").unwrap();

        assert_eq!(cancelled.len(), 1);
        assert_eq!(cancelled[0].run_id, first.run_id);
        assert!(cancelled[0].cancel_requested);
        let second = manager.list_runs(Some("container_b")).unwrap().remove(0);
        assert_eq!(second.status, "running");
    }
}
