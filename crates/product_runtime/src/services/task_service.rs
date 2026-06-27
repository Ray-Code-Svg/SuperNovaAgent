use std::collections::BTreeMap;
use std::io;
use std::path::Path;
use std::sync::{Arc, Mutex};

use axum::response::sse::Event;
use local_runtime_protocol::{
    ApprovalRecord, ArtifactRecord, ContainerBadges, ContainerMessage, ForceCloseResult,
    MessageLane, MessageRole, MessageType, TaskApprovalActionResult, TaskDetail,
    TaskDraftArtifactRecord, TaskReceiptRecord, TaskRecord, TaskStreamPayload, TaskStreamRequest,
    TaskUserInputRequest,
};
use serde_json::{json, Value};
use supernova_process_kernel::{
    ModelStreamDelta, ModelStreamDeltaKind, ModelStreamSink, ProcessEvent,
};

use crate::http::sse;
use crate::kernel::event_projection::{
    process_event_to_message, task_result_to_record, timeline_task_to_record,
};
use crate::kernel::KernelBridge;
use crate::services::context_pack_service::ContextPackService;
use crate::services::run_manager::{ProcessWorkerEvent, RunManager, TaskWorkerRequest};
use crate::services::settings_service::SettingsService;
use crate::state::message_feed::{advance_message_cursor, message_cursor_event_id, new_message};
use crate::state::product_db::ProductDb;
use crate::state::projection_shards::ProjectionShardDb;
use crate::state::workspace_registry::now_ms;

#[derive(Clone)]
pub struct TaskService {
    db: ProductDb,
    kernel: KernelBridge,
    context_pack: ContextPackService,
    run_manager: RunManager,
    settings: SettingsService,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TaskProjectionRepairReport {
    pub tasks_scanned: usize,
    pub tasks_with_truth: usize,
    pub messages_projected: usize,
    pub tasks_reconciled: usize,
    pub runs_repaired: usize,
    pub database_locked_messages_downgraded: usize,
}

impl TaskService {
    pub fn new(
        db: ProductDb,
        kernel: KernelBridge,
        context_pack: ContextPackService,
        run_manager: RunManager,
        settings: SettingsService,
    ) -> Self {
        Self {
            db,
            kernel,
            context_pack,
            run_manager,
            settings,
        }
    }

    pub fn list(&self, container_id: &str) -> rusqlite::Result<Vec<TaskRecord>> {
        if let Ok(items) = self.kernel.list_container_tasks(container_id, 500) {
            for item in items {
                let record = timeline_task_to_record(container_id, item);
                let record = self.merge_existing_task_record(record)?;
                let _ = self.db.upsert_task(&record)?;
            }
        }
        let mut tasks = self.db.list_tasks(container_id)?;
        for task in &mut tasks {
            *task = self.hydrate_task(task)?;
            let mut projection = project_task_detail(task, &self.task_process_events(task));
            self.sync_task_draft_artifacts(task, &mut projection)?;
            task.badges = projection.badges(&task.status);
        }
        Ok(tasks)
    }

    pub fn get(&self, task_id: &str) -> rusqlite::Result<TaskDetail> {
        let task = self.db.get_task(task_id)?;
        let mut task = self.hydrate_task(&task)?;
        let events = self.task_process_events(&task);
        let mut projection = project_task_detail(&task, &events);
        self.sync_task_draft_artifacts(&task, &mut projection)?;
        task.badges = projection.badges(&task.status);
        Ok(TaskDetail {
            messages: self
                .db
                .list_projected_task_messages_page(task_id, None, None)?,
            task,
            artifacts: projection.artifacts,
            approvals: projection.approvals,
            receipts: projection.receipts,
            selected_output_dir: projection.selected_output_dir,
            destination_fulfilled: projection.destination_fulfilled,
        })
    }

    pub fn messages(&self, task_id: &str) -> rusqlite::Result<Vec<ContainerMessage>> {
        if let Ok(task) = self.db.get_task(task_id) {
            let _ = self.hydrate_task(&task)?;
        }
        self.db
            .list_projected_task_messages_page(task_id, None, None)
    }

    pub fn messages_page(
        &self,
        task_id: &str,
        after_event_id: Option<i64>,
        limit: Option<usize>,
    ) -> rusqlite::Result<Vec<ContainerMessage>> {
        self.db
            .list_projected_task_messages_page(task_id, after_event_id, limit)
    }

    pub fn events_stream(
        &self,
        task_id: &str,
        after_event_id: Option<i64>,
        limit: Option<usize>,
    ) -> rusqlite::Result<Vec<Event>> {
        let messages = self.messages_page(task_id, after_event_id, limit)?;
        Ok(messages
            .into_iter()
            .map(|message| {
                sse_message(
                    event_type_for_message(&message),
                    &self.db.workspace_uid,
                    Some(message),
                )
            })
            .collect())
    }

    pub fn submit_user_input(
        &self,
        task_id: &str,
        request: TaskUserInputRequest,
    ) -> rusqlite::Result<TaskApprovalActionResult> {
        let input = request.input.trim();
        if input.is_empty() {
            return Err(rusqlite::Error::InvalidParameterName(
                "clarification input cannot be empty".into(),
            ));
        }
        self.run_user_input_action(task_id, "user_input", None, input.to_string())
    }

    pub fn force_close(
        &self,
        task_id: &str,
        reason: Option<String>,
    ) -> rusqlite::Result<ForceCloseResult> {
        let task = self.hydrate_task(&self.db.get_task(task_id)?)?;
        let job_id = task_job_id(&task)?;
        if task_status_is_closed(&task.status) || !task_status_can_force_close(&task.status) {
            self.complete_task_runtime_phase(&job_id, &task.status)?;
            return Ok(ForceCloseResult {
                action: "force_close".into(),
                status: task.status,
                messages: self
                    .db
                    .list_projected_task_messages_page(task_id, None, None)?,
            });
        }
        let reason = reason
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "User forced this task closed from Workbench v2.".to_string());
        self.kernel
            .cancel_job(&job_id, &reason)
            .map_err(io_to_sqlite)?;
        let _ = self.run_manager.request_cancel_task_run(task_id)?;
        self.complete_task_runtime_phase(&job_id, "cancelled")?;
        let task = self.hydrate_task(&task)?;
        Ok(ForceCloseResult {
            action: "force_close".into(),
            status: task.status,
            messages: self
                .db
                .list_projected_task_messages_page(task_id, None, None)?,
        })
    }

    pub fn hydrate_container(&self, container_id: &str) -> rusqlite::Result<()> {
        for task in self.list(container_id)? {
            self.hydrate_task(&task)?;
        }
        Ok(())
    }

    pub fn repair_container_stale_task_runs(&self, container_id: &str) -> rusqlite::Result<usize> {
        let mut repaired = 0;
        for task in self.db.list_tasks(container_id)? {
            let Some(job_id) = task.job_id.as_deref() else {
                continue;
            };
            let Ok(process_events) = self.kernel.read_process_events(job_id) else {
                continue;
            };
            if process_events.is_empty() {
                continue;
            }
            repaired += self.repair_stale_task_runs_from_events(&task, &process_events)?;
        }
        Ok(repaired)
    }

    pub fn repair_workspace_projection(&self) -> rusqlite::Result<TaskProjectionRepairReport> {
        let mut report = TaskProjectionRepairReport::default();
        for task in self.db.list_all_tasks()? {
            report.tasks_scanned += 1;
            let Some(job_id) = task.job_id.as_deref() else {
                continue;
            };
            let Ok(process_events) = self.kernel.read_process_events(job_id) else {
                continue;
            };
            if process_events.is_empty() {
                continue;
            }
            report.tasks_with_truth += 1;
            let before_count = self
                .db
                .list_projected_task_messages_page(&task.task_id, None, None)?
                .len();
            for process_event in &process_events {
                if let Some(message) = process_event_to_message(
                    &self.db.workspace_uid,
                    &task.container_id,
                    job_id,
                    process_event,
                ) {
                    let _ = self.append_task_projection_message(
                        &task.task_id,
                        job_id,
                        &task.container_id,
                        message,
                    )?;
                }
            }
            let after_count = self
                .db
                .list_projected_task_messages_page(&task.task_id, None, None)?
                .len();
            report.messages_projected += after_count.saturating_sub(before_count);

            let reconciled = self.reconcile_task_record(&task, &process_events)?;
            if reconciled.status != task.status || reconciled.updated_at_ms != task.updated_at_ms {
                report.tasks_reconciled += 1;
            }
            if let Some((terminal_status, _)) = task_status_from_events(&process_events) {
                self.complete_task_runtime_phase(&task.task_id, &terminal_status)?;
                let terminal_error_message = if matches!(
                    terminal_status.as_str(),
                    "failed" | "blocked" | "interrupted"
                ) {
                    task_terminal_error_message(&process_events)
                } else {
                    None
                };
                report.runs_repaired += self.db.repair_database_locked_task_runs(
                    &task.task_id,
                    job_id,
                    &terminal_status,
                    terminal_error_message.as_deref(),
                )?;
                if task_status_is_closed(&terminal_status) {
                    report.runs_repaired += self.db.repair_stale_active_task_runs(
                        &task.task_id,
                        job_id,
                        &terminal_status,
                        terminal_error_message.as_deref(),
                    )?;
                }
                if task_status_is_closed(&terminal_status) {
                    report.database_locked_messages_downgraded += self
                        .db
                        .downgrade_database_locked_task_errors(&task.container_id)?;
                }
            }
        }
        Ok(report)
    }

    pub fn start_stream(
        &self,
        container_id: &str,
        request: TaskStreamRequest,
    ) -> rusqlite::Result<Vec<Event>> {
        let response_language = self
            .settings
            .appearance_settings()
            .map_err(io_to_sqlite)?
            .language;
        let mut events = Vec::new();
        let mut pre_job_message_ids = Vec::new();
        let artifact_target = request.artifact_target.clone();
        let run = self.run_manager.start_task_run(container_id)?;
        let run_id = run.run_id.clone();
        let user = new_message(
            &self.db.workspace_uid,
            container_id,
            MessageLane::Task,
            MessageRole::User,
            MessageType::Text,
            Some(request.goal.clone()),
            None,
        );
        let user = self.db.append_message(user)?;
        pre_job_message_ids.push(user.message_id.clone());
        let mut last_cursor_event_id = Some(message_cursor_event_id(&user));
        events.push(sse_message(
            "task.started",
            &self.db.workspace_uid,
            Some(user),
        ));
        if let Some(guidance) = request.source_guidance.as_ref() {
            if !guidance.selected_sources.is_empty() {
                let label = if guidance.selected_sources.len() == 1 {
                    guidance.selected_sources[0]
                        .label
                        .clone()
                        .unwrap_or_else(|| guidance.selected_sources[0].ref_id.clone())
                } else {
                    format!("{} reference sources", guidance.selected_sources.len())
                };
                let mut source_message = new_message(
                    &self.db.workspace_uid,
                    container_id,
                    MessageLane::Task,
                    MessageRole::System,
                    MessageType::Phase,
                    Some(format!(
                        "Reference source guidance attached: {label}. Contents are not preloaded; Agent must inspect sources when needed."
                    )),
                    None,
                );
                source_message.title = Some("Reference sources selected".into());
                source_message.body_json =
                    serde_json::to_value(guidance).unwrap_or_else(|_| json!({}));
                source_message.source_ref = "source_guidance".into();
                let source_message = self.db.append_message(source_message)?;
                pre_job_message_ids.push(source_message.message_id.clone());
                last_cursor_event_id = Some(message_cursor_event_id(&source_message));
                events.push(sse_message(
                    "task.phase",
                    &self.db.workspace_uid,
                    Some(source_message),
                ));
            }
        }
        if let Some(destination) = request.artifact_destination.as_ref() {
            if !destination.selected_output_dir.trim().is_empty() {
                let mut destination_message = new_message(
                    &self.db.workspace_uid,
                    container_id,
                    MessageLane::Task,
                    MessageRole::System,
                    MessageType::Phase,
                    Some(format!(
                        "Output destination guidance attached: {}",
                        destination.selected_output_dir
                    )),
                    None,
                );
                destination_message.title = Some("Output destination selected".into());
                destination_message.body_json =
                    serde_json::to_value(destination).unwrap_or_else(|_| json!({}));
                destination_message.source_ref = "artifact_destination_guidance".into();
                let destination_message = self.db.append_message(destination_message)?;
                pre_job_message_ids.push(destination_message.message_id.clone());
                last_cursor_event_id = Some(message_cursor_event_id(&destination_message));
                events.push(sse_message(
                    "task.phase",
                    &self.db.workspace_uid,
                    Some(destination_message),
                ));
            }
        }
        if let Some(target) = artifact_target.as_ref() {
            let mut target_message = new_message(
                &self.db.workspace_uid,
                container_id,
                MessageLane::Task,
                MessageRole::System,
                MessageType::Phase,
                Some(format!(
                    "Artifact target: {} / {} / {}",
                    target.target_dir.as_deref().unwrap_or("runtime default"),
                    target.artifact_type,
                    target.save_strategy
                )),
                None,
            );
            target_message.title = Some("Artifact target selected".into());
            target_message.body_json = serde_json::to_value(target).unwrap_or_else(|_| json!({}));
            target_message.source_ref = "artifact_target".into();
            let target_message = self.db.append_message(target_message)?;
            pre_job_message_ids.push(target_message.message_id.clone());
            last_cursor_event_id = Some(message_cursor_event_id(&target_message));
            events.push(sse_message(
                "task.phase",
                &self.db.workspace_uid,
                Some(target_message),
            ));
        }
        let stream_sink = Arc::new(ProductTaskStreamSink::new(
            self.db.workspace_uid.clone(),
            container_id.to_string(),
        ));
        let materialized_context_pack_id = self
            .context_pack
            .materialize_for_request(container_id, None)?
            .map(|pack| pack.context_pack_id);
        let effective_auto_approve = effective_task_auto_approve(&request);
        let started_db = self.db.clone();
        let started_container_id = container_id.to_string();
        let started_goal = request.goal.clone();
        let started_message_ids = pre_job_message_ids.clone();
        let started_stream_sink = stream_sink.clone();
        let started_run_manager = self.run_manager.clone();
        let started_run_id = run_id.clone();
        let task_result = if self.run_manager.process_worker_enabled() {
            let worker_db = started_db.clone();
            let worker_container_id = started_container_id.clone();
            let worker_goal = started_goal.clone();
            let worker_message_ids = started_message_ids.clone();
            let worker_stream_sink = started_stream_sink.clone();
            let worker_run_manager = started_run_manager.clone();
            let worker_run_id = started_run_id.clone();
            self.run_manager.run_task_in_process_worker(
                &run_id,
                TaskWorkerRequest {
                    container_id: container_id.to_string(),
                    goal: request.goal.clone(),
                    context_pack_id: materialized_context_pack_id,
                    source_guidance: request.source_guidance.clone(),
                    artifact_destination: request.artifact_destination.clone(),
                    model_config: request.model_config,
                    auto_approve: effective_auto_approve,
                    response_language,
                },
                move |event| {
                    match event {
                        ProcessWorkerEvent::TaskStarted { job_id, .. } => bind_started_task(
                            &worker_db,
                            &worker_run_manager,
                            &worker_stream_sink,
                            &worker_run_id,
                            &worker_container_id,
                            &worker_goal,
                            &worker_message_ids,
                            &job_id,
                        )?,
                        ProcessWorkerEvent::ModelStreamDelta(delta) => {
                            worker_stream_sink.on_model_stream_delta(delta);
                        }
                        ProcessWorkerEvent::WorkerStarted { .. }
                        | ProcessWorkerEvent::Heartbeat => {}
                    }
                    Ok(())
                },
            )
        } else {
            self.kernel
                .start_task_in_container_with_started_and_stream_sink_and_response_language(
                    container_id,
                    &request.goal,
                    materialized_context_pack_id,
                    request.source_guidance.clone(),
                    request.artifact_destination.clone(),
                    request.model_config,
                    effective_auto_approve,
                    move |job_id, _root_pid| {
                        bind_started_task(
                            &started_db,
                            &started_run_manager,
                            &started_stream_sink,
                            &started_run_id,
                            &started_container_id,
                            &started_goal,
                            &started_message_ids,
                            job_id,
                        )
                    },
                    Some(stream_sink),
                    response_language,
                )
        };
        match task_result {
            Ok(result) => {
                let process_events = self
                    .kernel
                    .read_process_events(&result.job_id)
                    .unwrap_or_default();
                let final_status = task_status_from_events(&process_events)
                    .map(|(status, _)| status)
                    .unwrap_or_else(|| result.status.clone());
                self.complete_streamed_task_reasoning(&result.job_id)?;
                self.complete_task_runtime_phase(&result.job_id, &final_status)?;
                let _ = self.run_manager.complete_run(&run_id, &final_status)?;
                let mut record = task_result_to_record(container_id, &request.goal, &result);
                record.status = final_status.clone();
                let record = self.merge_existing_task_record(record)?;
                let task = self.db.upsert_task(&record)?;
                for process_event in &process_events {
                    if let Some(message) = process_event_to_message(
                        &self.db.workspace_uid,
                        container_id,
                        &result.job_id,
                        process_event,
                    ) {
                        let message = self.append_task_projection_message(
                            &result.job_id,
                            &result.job_id,
                            container_id,
                            message,
                        )?;
                        last_cursor_event_id = Some(message_cursor_event_id(&message));
                        events.push(sse_message(
                            event_type_for_message(&message),
                            &self.db.workspace_uid,
                            Some(message),
                        ));
                    }
                }
                events.push(sse_payload(
                    "task.complete",
                    &self.db.workspace_uid,
                    last_cursor_event_id,
                    json!({"task": task, "status": final_status}),
                ));
            }
            Err(err) => {
                let _ = self.run_manager.fail_run(&run_id, &err.to_string())?;
                let mut error = new_message(
                    &self.db.workspace_uid,
                    container_id,
                    MessageLane::Task,
                    MessageRole::System,
                    MessageType::Error,
                    Some(err.to_string()),
                    None,
                );
                error.source_kind = "kernel_bridge".into();
                error.source_ref = "task_start".into();
                let error = self.db.append_message(error)?;
                last_cursor_event_id = Some(message_cursor_event_id(&error));
                events.push(sse_message(
                    "task.error",
                    &self.db.workspace_uid,
                    Some(error),
                ));
                events.push(sse_payload(
                    "task.complete",
                    &self.db.workspace_uid,
                    last_cursor_event_id,
                    json!({"status": "failed"}),
                ));
            }
        }
        Ok(events)
    }

    fn complete_streamed_task_reasoning(&self, task_id: &str) -> rusqlite::Result<()> {
        for mut message in self
            .db
            .list_projected_task_messages_page(task_id, None, None)?
        {
            if message.source_kind != "model_stream"
                || message.message_type != MessageType::Reasoning
                || message.status == "completed"
            {
                continue;
            }
            message.status = "completed".into();
            message.title = Some("Task reasoning".into());
            message.body_json = json!({
                "schema": "supernova_task_model_stream.v1",
                "completed": true,
                "source_ref": message.source_ref,
                "execution_fact_boundary": "Displayed model reasoning stream only; Task execution remains governed by validated decisions, tool calls, and Kernel receipts.",
            });
            advance_message_cursor(&mut message);
            let job_id = message
                .job_id
                .clone()
                .unwrap_or_else(|| task_id.to_string());
            let container_id = message.container_id.clone();
            let _ =
                self.append_task_projection_message(task_id, &job_id, &container_id, message)?;
        }
        Ok(())
    }

    fn complete_task_runtime_phase(&self, task_id: &str, status: &str) -> rusqlite::Result<()> {
        for mut message in self.db.list_task_messages(task_id)? {
            if message.source_kind != "product_runtime_runtime"
                || message.source_ref != "task_runtime_running"
            {
                continue;
            }
            message.status = if status == "running" {
                "streaming".into()
            } else {
                status.to_string()
            };
            message.title = Some(
                match status {
                    "completed" => "TaskRuntime completed",
                    "waiting_approval" => "TaskRuntime waiting approval",
                    "failed" | "interrupted" | "blocked" | "cancelled" | "canceled" => {
                        "TaskRuntime stopped"
                    }
                    _ => "TaskRuntime status updated",
                }
                .into(),
            );
            message.body_json = json!({
                "schema": "supernova_task_runtime_phase.v1",
                "status": status,
                "source_ref": message.source_ref,
            });
            advance_message_cursor(&mut message);
            let _ = self.db.append_message(message)?;
        }
        Ok(())
    }

    fn merge_existing_task_record(&self, mut record: TaskRecord) -> rusqlite::Result<TaskRecord> {
        if let Ok(existing) = self.db.get_task(&record.task_id) {
            record.created_at_ms = existing.created_at_ms.min(record.created_at_ms);
            record.updated_at_ms = existing.updated_at_ms.max(record.updated_at_ms);
            if record.goal.trim().is_empty() {
                record.goal = existing.goal;
            }
            if record.title.trim().is_empty() || record.title == "Task" {
                record.title = existing.title;
            }
            if record.job_id.is_none() {
                record.job_id = existing.job_id;
            }
        }
        Ok(record)
    }

    fn hydrate_task(&self, task: &TaskRecord) -> rusqlite::Result<TaskRecord> {
        let Some(job_id) = task.job_id.as_deref() else {
            return Ok(task.clone());
        };
        if let Ok(process_events) = self.kernel.read_process_events(job_id) {
            for process_event in &process_events {
                if let Some(message) = process_event_to_message(
                    &self.db.workspace_uid,
                    &task.container_id,
                    job_id,
                    process_event,
                ) {
                    let _ = self.append_task_projection_message(
                        &task.task_id,
                        job_id,
                        &task.container_id,
                        message,
                    )?;
                }
            }
            let reconciled = self.reconcile_task_record(task, &process_events)?;
            self.repair_stale_task_runs_from_events(&reconciled, &process_events)?;
            return Ok(reconciled);
        }
        Ok(task.clone())
    }

    fn repair_stale_task_runs_from_events(
        &self,
        task: &TaskRecord,
        events: &[ProcessEvent],
    ) -> rusqlite::Result<usize> {
        let Some(job_id) = task.job_id.as_deref() else {
            return Ok(0);
        };
        let Some((terminal_status, _)) = task_status_from_events(events) else {
            return Ok(0);
        };
        if !task_status_is_closed(&terminal_status) {
            return Ok(0);
        }
        let terminal_error_message = if matches!(
            terminal_status.as_str(),
            "failed" | "blocked" | "interrupted"
        ) {
            task_terminal_error_message(events)
        } else {
            None
        };
        self.db.repair_stale_active_task_runs(
            &task.task_id,
            job_id,
            &terminal_status,
            terminal_error_message.as_deref(),
        )
    }

    fn append_task_projection_message(
        &self,
        task_id: &str,
        job_id: &str,
        container_id: &str,
        message: ContainerMessage,
    ) -> rusqlite::Result<ContainerMessage> {
        match message.source_kind.as_str() {
            "process_truth" | "model_stream" => {
                self.db
                    .append_task_projection_message(task_id, job_id, container_id, message)
            }
            _ => self.db.append_message(message),
        }
    }

    fn reconcile_task_record(
        &self,
        task: &TaskRecord,
        events: &[ProcessEvent],
    ) -> rusqlite::Result<TaskRecord> {
        let Some((status, updated_at_ms)) = task_status_from_events(events) else {
            return Ok(task.clone());
        };
        if task.status == status && task.updated_at_ms >= updated_at_ms {
            return Ok(task.clone());
        }
        let mut updated = task.clone();
        updated.status = status;
        updated.updated_at_ms = updated.updated_at_ms.max(updated_at_ms);
        self.db.upsert_task(&updated)
    }

    fn task_process_events(&self, task: &TaskRecord) -> Vec<ProcessEvent> {
        task.job_id
            .as_deref()
            .and_then(|job_id| self.kernel.read_process_events(job_id).ok())
            .unwrap_or_default()
    }

    fn sync_task_draft_artifacts(
        &self,
        task: &TaskRecord,
        projection: &mut TaskDetailProjection,
    ) -> rusqlite::Result<()> {
        if projection.approvals.is_empty() {
            self.db
                .delete_task_draft_artifacts_for_task(&task.task_id)?;
            return Ok(());
        }

        if task_status_is_closed(&task.status) {
            self.db
                .delete_task_draft_artifacts_for_task(&task.task_id)?;
            for approval in &mut projection.approvals {
                approval.draft_artifact = None;
            }
            return Ok(());
        }

        for approval in &mut projection.approvals {
            if approval.status != "pending" {
                self.db
                    .delete_task_draft_artifact(&task.task_id, &approval.approval_id)?;
                approval.draft_artifact = None;
                continue;
            }

            if let Some(mut draft) =
                draft_artifact_from_approval(&self.db.workspace_uid, &task.container_id, approval)
            {
                if let Some(existing) = self
                    .db
                    .get_task_draft_artifact(&task.task_id, &approval.approval_id)?
                {
                    draft.created_at_ms = existing.created_at_ms;
                    if existing.status == "edited" && existing.preview_ref == draft.preview_ref {
                        draft.status = existing.status;
                        draft.content_text = existing.content_text;
                    }
                }
                approval.draft_artifact = Some(self.db.upsert_task_draft_artifact(&draft)?);
            } else {
                approval.draft_artifact = self
                    .db
                    .get_task_draft_artifact(&task.task_id, &approval.approval_id)?;
            }
        }

        Ok(())
    }

    fn run_user_input_action(
        &self,
        task_id: &str,
        action: &str,
        approval_id: Option<String>,
        user_input: String,
    ) -> rusqlite::Result<TaskApprovalActionResult> {
        let task = self.db.get_task(task_id)?;
        let job_id = task_job_id(&task)?;
        let _ = approval_id;
        let result = self
            .kernel
            .submit_user_input(&job_id, &user_input)
            .map_err(io_to_sqlite)?;
        self.finish_task_action(task, action, result)
    }

    fn finish_task_action(
        &self,
        previous: TaskRecord,
        action: &str,
        result: supernova_process_kernel::TaskAgentRunResult,
    ) -> rusqlite::Result<TaskApprovalActionResult> {
        let process_events = self
            .kernel
            .read_process_events(&result.job_id)
            .unwrap_or_default();
        let final_status = task_status_from_events(&process_events)
            .map(|(status, _)| status)
            .unwrap_or_else(|| result.status.clone());
        self.complete_streamed_task_reasoning(&result.job_id)?;
        self.complete_task_runtime_phase(&result.job_id, &final_status)?;
        let mut record = task_result_to_record(&previous.container_id, &previous.goal, &result);
        record.status = final_status.clone();
        let record = self.merge_existing_task_record(record)?;
        let task = self.db.upsert_task(&record)?;
        let task = self.hydrate_task(&task)?;
        let messages = self
            .db
            .list_projected_task_messages_page(&task.task_id, None, None)?;
        Ok(TaskApprovalActionResult {
            action: action.into(),
            status: final_status,
            task,
            messages,
        })
    }
}

#[allow(clippy::too_many_arguments)]
fn bind_started_task(
    db: &ProductDb,
    run_manager: &RunManager,
    stream_sink: &ProductTaskStreamSink,
    run_id: &str,
    container_id: &str,
    goal: &str,
    message_ids: &[String],
    job_id: &str,
) -> io::Result<()> {
    let now = now_ms() as u128;
    let record = TaskRecord {
        task_id: job_id.to_string(),
        container_id: container_id.to_string(),
        job_id: Some(job_id.to_string()),
        title: goal.chars().take(80).collect(),
        goal: goal.to_string(),
        status: "running".into(),
        badges: Default::default(),
        created_at_ms: now,
        updated_at_ms: now,
    };
    let mut running = new_message(
        &db.workspace_uid,
        container_id,
        MessageLane::Task,
        MessageRole::System,
        MessageType::Phase,
        Some("Kernel TaskRuntime started and is processing this task.".into()),
        None,
    );
    running.status = "streaming".into();
    running.title = Some("TaskRuntime running".into());
    running.task_id = Some(job_id.to_string());
    running.job_id = Some(job_id.to_string());
    running.source_kind = "product_runtime_runtime".into();
    running.source_ref = "task_runtime_running".into();
    db.upsert_task(&record)
        .and_then(|_| db.bind_messages_to_task(message_ids, job_id, job_id))
        .and_then(|_| run_manager.bind_task_run(run_id, job_id, job_id))
        .and_then(|_| db.append_message(running).map(|_| ()))
        .map_err(sqlite_to_io)?;
    let projection_shard = db
        .ensure_task_projection_shard(container_id, job_id, job_id)
        .map_err(sqlite_to_io)?;
    stream_sink.bind_job(job_id, projection_shard);
    Ok(())
}

#[derive(Default)]
struct TaskDetailProjection {
    artifacts: Vec<ArtifactRecord>,
    approvals: Vec<ApprovalRecord>,
    receipts: Vec<TaskReceiptRecord>,
    selected_output_dir: Option<String>,
    destination_fulfilled: Option<bool>,
}

impl TaskDetailProjection {
    fn badges(&self, status: &str) -> ContainerBadges {
        ContainerBadges {
            running: u32::from(status == "running"),
            blocked: u32::from(status.contains("blocked") || status == "failed"),
            approval: 0,
            artifact_ready: self
                .artifacts
                .iter()
                .filter(|item| artifact_counts_as_ready(item))
                .count() as u32,
            ..Default::default()
        }
    }
}

fn project_task_detail(task: &TaskRecord, events: &[ProcessEvent]) -> TaskDetailProjection {
    let mut approvals = BTreeMap::<String, ApprovalRecord>::new();
    let mut artifacts = BTreeMap::<String, ArtifactRecord>::new();
    let mut receipts = BTreeMap::<String, TaskReceiptRecord>::new();
    let mut selected_output_dir = None;

    for event in events {
        match event.event_type.as_str() {
            "task_artifact_destination_guidance_attached" => {
                if let Some(path) = event
                    .data
                    .get("selected_output_dir")
                    .and_then(Value::as_str)
                    .map(normalize_workspace_path)
                    .filter(|value| !value.is_empty())
                {
                    selected_output_dir = Some(path);
                }
            }
            "preview_tx_created" | "preview_created" => {}
            "preview_tx_approved"
            | "approval_token_issued"
            | "approval_token_consumed"
            | "approval_token_used"
            | "preview_tx_applied"
            | "preview_tx_closed" => {
                update_approval_status(&mut approvals, event);
            }
            "user_input_received" => {
                for approval in approvals.values_mut() {
                    if approval.status == "pending" {
                        approval.status = "resolved_by_user_input".to_string();
                        approval.resolved_at_ms = Some(event.timestamp_ms);
                    }
                }
            }
            "job_waiting_user" => {
                close_pending_approvals(&mut approvals, event, "resolved_by_user_input");
            }
            "job_completed" | "task_completed" | "process_completed" => {
                close_pending_approvals(&mut approvals, event, "closed");
            }
            "job_cancelled" => {
                close_pending_approvals(&mut approvals, event, "cancelled");
            }
            "job_interrupted_by_model_protocol_error" => {
                close_pending_approvals(&mut approvals, event, "interrupted");
            }
            value if value.contains("failed") || value.contains("blocked") => {
                close_pending_approvals(&mut approvals, event, "blocked");
            }
            _ => {}
        }

        if let Some(record) = receipt_from_event(task, event) {
            receipts.insert(record.receipt_id.clone(), record);
        }

        for record in artifacts_from_event(task, event) {
            let key = record
                .path
                .clone()
                .unwrap_or_else(|| record.artifact_id.clone());
            artifacts
                .entry(key)
                .and_modify(|existing| merge_artifact(existing, &record))
                .or_insert(record);
        }
    }

    let mut artifacts = artifacts.into_values().collect::<Vec<_>>();
    artifacts.sort_by_key(|item| item.created_at_ms);
    let mut approvals = approvals.into_values().collect::<Vec<_>>();
    approvals.sort_by_key(|item| item.created_at_ms);
    let mut receipts = receipts.into_values().collect::<Vec<_>>();
    receipts.sort_by_key(|item| item.created_at_ms);
    let destination_fulfilled = selected_output_dir.as_ref().map(|target_dir| {
        artifacts.iter().any(|artifact| {
            artifact.verified && artifact_path_matches_dir(&artifact.path, target_dir)
        })
    });

    TaskDetailProjection {
        artifacts,
        approvals,
        receipts,
        selected_output_dir,
        destination_fulfilled,
    }
}

fn close_pending_approvals(
    approvals: &mut BTreeMap<String, ApprovalRecord>,
    event: &ProcessEvent,
    status: &str,
) {
    for approval in approvals.values_mut() {
        if approval.status == "pending" {
            approval.status = status.to_string();
            approval.resolved_at_ms = Some(event.timestamp_ms);
        }
    }
}

pub(crate) fn task_status_from_events(events: &[ProcessEvent]) -> Option<(String, u128)> {
    let mut status: Option<(String, u128)> = None;
    for event in events {
        let next = match event.event_type.as_str() {
            "job_status_changed" => event
                .data
                .get("status")
                .and_then(Value::as_str)
                .map(normalize_task_status),
            "job_completed" => Some("completed".to_string()),
            "provider_tool_call_waiting_approval" => None,
            "job_waiting_user" => Some("waiting_user".to_string()),
            "job_cancelled" => Some("cancelled".to_string()),
            "job_failed" => Some("failed".to_string()),
            "job_blocked" => Some("blocked".to_string()),
            "job_interrupted_by_model_protocol_error" => Some("interrupted".to_string()),
            _ => None,
        };
        if let Some(next) = next {
            if status
                .as_ref()
                .map(|(current, _)| task_status_is_closed(current.as_str()))
                .unwrap_or(false)
            {
                continue;
            }
            status = Some((next, event.timestamp_ms));
        }
    }
    status
}

fn task_terminal_error_message(events: &[ProcessEvent]) -> Option<String> {
    events.iter().rev().find_map(|event| {
        if matches!(
            event.event_type.as_str(),
            "job_failed" | "job_blocked" | "job_interrupted_by_model_protocol_error"
        ) {
            projection_error_message(&event.data)
        } else {
            None
        }
    })
}

fn projection_error_message(value: &Value) -> Option<String> {
    value
        .get("message")
        .and_then(Value::as_str)
        .or_else(|| value.get("reason").and_then(Value::as_str))
        .or_else(|| value.pointer("/error/message").and_then(Value::as_str))
        .or_else(|| value.get("error").and_then(Value::as_str))
        .map(str::trim)
        .filter(|message| !message.is_empty())
        .map(ToString::to_string)
}

fn normalize_task_status(status: &str) -> String {
    if status == "waiting_approval" {
        "running".to_string()
    } else {
        status.to_string()
    }
}

fn update_approval_status(approvals: &mut BTreeMap<String, ApprovalRecord>, event: &ProcessEvent) {
    let preview_id = event
        .data
        .get("preview_id")
        .or_else(|| event.data.get("tx_id"))
        .and_then(Value::as_str)
        .map(ToString::to_string);
    let Some(preview_id) = preview_id else {
        return;
    };
    let Some(record) = approvals.get_mut(&preview_id) else {
        return;
    };
    record.status = match event.event_type.as_str() {
        "preview_tx_closed" => event
            .data
            .get("status")
            .and_then(Value::as_str)
            .map(|value| {
                if value == "applied" {
                    "approved".to_string()
                } else {
                    value.to_string()
                }
            })
            .unwrap_or_else(|| "closed".to_string()),
        "preview_tx_approved"
        | "approval_token_issued"
        | "approval_token_consumed"
        | "approval_token_used"
        | "preview_tx_applied" => "approved".to_string(),
        _ => record.status.clone(),
    };
    record.resolved_at_ms = Some(event.timestamp_ms);
}

fn draft_artifact_from_approval(
    workspace_uid: &str,
    container_id: &str,
    approval: &ApprovalRecord,
) -> Option<TaskDraftArtifactRecord> {
    let content_text = approval_preview_markdown(&approval.preview)?;
    Some(TaskDraftArtifactRecord {
        draft_id: format!(
            "draft_{}_{}",
            safe_projection_id(&approval.task_id),
            safe_projection_id(&approval.approval_id)
        ),
        workspace_uid: workspace_uid.to_string(),
        container_id: container_id.to_string(),
        task_id: approval.task_id.clone(),
        approval_id: approval.approval_id.clone(),
        preview_ref: approval.preview_ref.clone(),
        operation: approval.operation.clone(),
        status: "pending".to_string(),
        content_format: "markdown".to_string(),
        content_text,
        created_at_ms: approval.created_at_ms,
        updated_at_ms: approval.resolved_at_ms.unwrap_or(approval.created_at_ms),
    })
}

fn approval_preview_markdown(value: &Value) -> Option<String> {
    preview_operation_draft_content(value)
        .or_else(|| {
            value
                .get("preview")
                .and_then(preview_operation_draft_content)
        })
        .or_else(|| provider_preview_markdown(value).map(clean_preview_markdown))
        .or_else(|| string_field(value, "content").map(clean_preview_markdown))
        .or_else(|| string_field(value, "text").map(clean_preview_markdown))
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn provider_preview_markdown(value: &Value) -> Option<String> {
    string_field(value, "human_preview_markdown").or_else(|| {
        value
            .get("preview")
            .and_then(|preview| string_field(preview, "human_preview_markdown"))
    })
}

fn preview_operation_draft_content(value: &Value) -> Option<String> {
    value
        .get("executable_operations")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|operation| operation.get("arguments"))
        .find_map(operation_argument_text)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn operation_argument_text(arguments: &Value) -> Option<String> {
    ["content", "text", "markdown", "draft_content", "body"]
        .iter()
        .find_map(|field| string_field(arguments, field))
}

fn clean_preview_markdown(value: String) -> String {
    extract_draft_content_from_preview_markdown(&value).unwrap_or(value)
}

fn extract_draft_content_from_preview_markdown(markdown: &str) -> Option<String> {
    let normalized = markdown.replace("\r\n", "\n");
    let mut offset = 0usize;
    for line in normalized.split_inclusive('\n') {
        if line.trim().eq_ignore_ascii_case("## Draft content") {
            let after_heading = normalized[offset + line.len()..].trim();
            if after_heading.is_empty() {
                return None;
            }
            return strip_fenced_block(after_heading)
                .or_else(|| Some(after_heading.to_string()))
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty());
        }
        offset += line.len();
    }
    None
}

fn strip_fenced_block(value: &str) -> Option<String> {
    let mut lines = value.lines();
    let first = lines.next()?.trim_start();
    let fence = if first.starts_with("```") {
        "```"
    } else if first.starts_with("~~~") {
        "~~~"
    } else {
        return None;
    };

    let mut content = Vec::new();
    for line in lines {
        if line.trim_start().starts_with(fence) {
            return Some(content.join("\n"));
        }
        content.push(line);
    }
    None
}

fn string_field(value: &Value, field: &str) -> Option<String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(ToString::to_string)
}

fn safe_projection_id(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

pub(crate) fn task_status_is_closed(status: &str) -> bool {
    matches!(
        status,
        "completed" | "failed" | "blocked" | "interrupted" | "cancelled" | "canceled"
    )
}

fn task_status_can_force_close(status: &str) -> bool {
    matches!(status, "running" | "waiting_user" | "pending")
}

fn receipt_from_event(task: &TaskRecord, event: &ProcessEvent) -> Option<TaskReceiptRecord> {
    if !is_receipt_event(event.event_type.as_str()) {
        return None;
    }
    let capability_id = event
        .data
        .get("capability_id")
        .and_then(Value::as_str)
        .map(ToString::to_string);
    let status = event
        .data
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("recorded")
        .to_string();
    let receipt_data = event.data.get("data").unwrap_or(&event.data);
    let artifact_paths = artifact_paths_from_value(receipt_data);
    let capability_label = capability_id
        .as_deref()
        .unwrap_or(event.event_type.as_str());
    let summary = if artifact_paths.is_empty() {
        Some(format!("{capability_label}: {status}"))
    } else {
        Some(format!(
            "{capability_label}: {status}; {} path(s)",
            artifact_paths.len()
        ))
    };
    Some(TaskReceiptRecord {
        receipt_id: format!("receipt_{}_{}", task.task_id, event.event_id),
        task_id: task.task_id.clone(),
        capability_id,
        status,
        kind: event.event_type.clone(),
        receipt_ref: Some(format!(
            "process_event://{}/{}",
            task.task_id, event.event_id
        )),
        artifact_paths,
        summary,
        created_at_ms: event.timestamp_ms,
    })
}

fn is_receipt_event(event_type: &str) -> bool {
    matches!(
        event_type,
        "capability_receipt" | "verify_event" | "artifact_model_audit_receipt" | "tx_rollback"
    )
}

fn artifacts_from_event(task: &TaskRecord, event: &ProcessEvent) -> Vec<ArtifactRecord> {
    let capability_id = event
        .data
        .get("capability_id")
        .and_then(Value::as_str)
        .map(ToString::to_string);
    let receipt_status = event
        .data
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("recorded");
    let receipt_data = event.data.get("data").unwrap_or(&event.data);
    let paths = artifact_paths_from_value(receipt_data);
    if paths.is_empty() {
        return Vec::new();
    }
    let verified =
        receipt_status == "success" && is_verification_capability(capability_id.as_deref());
    let status = if verified {
        "verified"
    } else if receipt_status == "success" && is_artifact_write_capability(capability_id.as_deref())
        || event.event_type == "job_completed"
        || event.event_type == "completion_statement_recorded"
    {
        "ready"
    } else {
        receipt_status
    };
    paths
        .into_iter()
        .enumerate()
        .map(|(index, path)| ArtifactRecord {
            artifact_id: format!("artifact_{}_{}_{}", task.task_id, event.event_id, index),
            container_id: task.container_id.clone(),
            task_id: Some(task.task_id.clone()),
            title: artifact_title(&path),
            artifact_type: artifact_type(&path, capability_id.as_deref()),
            path: Some(path),
            status: status.to_string(),
            capability_id: capability_id.clone(),
            receipt_ref: Some(format!(
                "process_event://{}/{}",
                task.task_id, event.event_id
            )),
            verified,
            kind: Some(event.event_type.clone()),
            created_at_ms: event.timestamp_ms,
        })
        .collect()
}

fn merge_artifact(existing: &mut ArtifactRecord, next: &ArtifactRecord) {
    if next.verified {
        existing.verified = true;
        existing.status = "verified".to_string();
    } else if next.status == "ready" && existing.status != "verified" {
        existing.status = "ready".to_string();
    } else if existing.status != "verified" {
        existing.status = next.status.clone();
    }
    if existing.capability_id.is_none() {
        existing.capability_id = next.capability_id.clone();
    }
    if next.receipt_ref.is_some() {
        existing.receipt_ref = next.receipt_ref.clone();
    }
    if next.kind.is_some() {
        existing.kind = next.kind.clone();
    }
}

fn artifact_paths_from_value(value: &Value) -> Vec<String> {
    let mut paths = Vec::new();
    collect_path_fields(value, &mut paths);
    paths.sort();
    paths.dedup();
    paths
}

fn collect_path_fields(value: &Value, paths: &mut Vec<String>) {
    const SCALAR_KEYS: &[&str] = &[
        "artifact_path",
        "archive_path",
        "path",
        "output_path",
        "destination_path",
        "destination_zip_path",
        "manifest_path",
        "checksums_path",
        "perf_notes_path",
        "workspace_path",
        "destination_dir",
    ];
    const ARRAY_KEYS: &[&str] = &[
        "artifacts",
        "artifact_paths",
        "all_artifacts",
        "archive_paths",
        "claimed_artifacts",
        "paths",
        "output_paths",
        "files",
        "created_files",
        "written_files",
        "verified_paths",
        "included_artifacts",
    ];

    for key in SCALAR_KEYS {
        if let Some(path) = value
            .get(*key)
            .and_then(Value::as_str)
            .map(normalize_workspace_path)
            .filter(|item| !item.is_empty())
        {
            paths.push(path);
        }
    }

    for key in ARRAY_KEYS {
        let Some(items) = value.get(*key).and_then(Value::as_array) else {
            continue;
        };
        for item in items {
            if let Some(path) = item
                .as_str()
                .map(normalize_workspace_path)
                .filter(|value| !value.is_empty())
            {
                paths.push(path);
            } else if item.is_object() {
                collect_path_fields(item, paths);
            }
        }
    }
}

fn artifact_counts_as_ready(artifact: &ArtifactRecord) -> bool {
    artifact.verified || artifact.status == "ready" || artifact.status == "verified"
}

fn is_artifact_write_capability(capability_id: Option<&str>) -> bool {
    matches!(
        capability_id,
        Some("os.write_artifact")
            | Some("os.zip")
            | Some("artifact.copy_source_set")
            | Some("package.build_zip")
            | Some("office.docx.create")
            | Some("office.docx.rewrite_save_as")
            | Some("office.docx.rewrite_in_place")
            | Some("workspace.perf_inventory")
            | Some("dataset.export_csv")
            | Some("dataset.export_markdown")
            | Some("model.generate_artifact")
    )
}

fn is_verification_capability(capability_id: Option<&str>) -> bool {
    matches!(
        capability_id,
        Some("os.verify_artifact")
            | Some("artifact.verify_typed")
            | Some("artifact.audit_quality")
            | Some("model.audit_artifact_quality")
    )
}

fn artifact_title(path: &str) -> String {
    Path::new(path)
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or(path)
        .to_string()
}

fn artifact_type(path: &str, capability_id: Option<&str>) -> String {
    if matches!(capability_id, Some("os.zip") | Some("package.build_zip")) {
        return "zip".to_string();
    }
    if matches!(
        capability_id,
        Some("office.docx.create")
            | Some("office.docx.rewrite_save_as")
            | Some("office.docx.rewrite_in_place")
    ) {
        return "docx".to_string();
    }
    Path::new(path)
        .extension()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or("artifact")
        .to_ascii_lowercase()
}

fn artifact_path_matches_dir(path: &Option<String>, target_dir: &str) -> bool {
    let Some(path) = path.as_deref().map(normalize_workspace_path) else {
        return false;
    };
    let target = normalize_workspace_path(target_dir)
        .trim_end_matches('/')
        .to_string();
    if target.is_empty() || target == "." {
        return true;
    }
    path == target || path.starts_with(&format!("{target}/"))
}

fn normalize_workspace_path(value: impl AsRef<str>) -> String {
    value
        .as_ref()
        .trim()
        .replace('\\', "/")
        .trim_start_matches("./")
        .trim_matches('/')
        .to_string()
}

pub(crate) fn effective_task_auto_approve(_request: &TaskStreamRequest) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::PathBuf;

    use crate::app_paths::AppPaths;
    use crate::kernel::KernelBridge;
    use crate::services::settings_service::SettingsService;
    use supernova_process_kernel::ProcessTruthStore;

    fn temp_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("supernova_task_service_{name}_{}", now_ms()));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn test_service(name: &str) -> TaskService {
        let workspace_root = temp_root(&format!("{name}_workspace"));
        let state_root = temp_root(&format!("{name}_state"));
        let provider_root = temp_root(&format!("{name}_provider"));
        let db = ProductDb::open(&state_root, format!("workspace_{name}")).unwrap();
        let kernel = KernelBridge::new(workspace_root.clone(), state_root.clone(), provider_root);
        let context_pack =
            ContextPackService::new(db.clone(), kernel.clone(), workspace_root.clone());
        let run_manager = RunManager::new(db.clone());
        let settings = test_settings(name, &kernel);
        TaskService::new(db, kernel, context_pack, run_manager, settings)
    }

    fn test_settings(name: &str, kernel: &KernelBridge) -> SettingsService {
        SettingsService::new(
            AppPaths {
                app_config_root: temp_root(&format!("{name}_config")),
                app_state_root: temp_root(&format!("{name}_app_state")),
            },
            kernel.clone(),
        )
    }

    #[test]
    fn task_auto_approve_request_is_clamped_by_service_policy() {
        let request = TaskStreamRequest {
            goal: "do work".into(),
            session_id: None,
            context_pack_id: None,
            source_guidance: None,
            model_config: None,
            artifact_destination: None,
            artifact_target: None,
            auto_approve: true,
        };

        assert!(!effective_task_auto_approve(&request));
    }

    fn task_record() -> TaskRecord {
        TaskRecord {
            task_id: "job_test".into(),
            container_id: "container_test".into(),
            job_id: Some("job_test".into()),
            title: "Test task".into(),
            goal: "write report".into(),
            status: "completed".into(),
            badges: Default::default(),
            created_at_ms: 1,
            updated_at_ms: 1,
        }
    }

    #[test]
    fn task_model_stream_sink_projects_reasoning_chunks_as_single_live_message() {
        let service = test_service("model_stream_sink");
        service.db.upsert_task(&task_record()).unwrap();
        let sink =
            ProductTaskStreamSink::new(service.db.workspace_uid.clone(), "container_test".into());
        sink.on_model_stream_delta(ModelStreamDelta {
            model_call_id: "task-call-1".into(),
            operation: supernova_process_kernel::ModelOperation::DecideNextAction,
            kind: ModelStreamDeltaKind::Reasoning,
            sequence: 1,
            delta: "Plan".into(),
        });
        let projection_shard = service
            .db
            .ensure_task_projection_shard("container_test", "job_test", "job_test")
            .unwrap();
        sink.bind_job("job_test", projection_shard);
        sink.on_model_stream_delta(ModelStreamDelta {
            model_call_id: "task-call-1".into(),
            operation: supernova_process_kernel::ModelOperation::DecideNextAction,
            kind: ModelStreamDeltaKind::Reasoning,
            sequence: 2,
            delta: " next".into(),
        });

        let messages = service
            .db
            .list_projected_task_messages_page("job_test", None, Some(20))
            .unwrap();
        let stream_messages: Vec<_> = messages
            .iter()
            .filter(|message| message.source_kind == "model_stream")
            .collect();
        assert_eq!(stream_messages.len(), 1);
        assert!(service
            .db
            .list_task_messages("job_test")
            .unwrap()
            .iter()
            .all(|message| message.source_kind != "model_stream"));
        let stream_message = stream_messages[0];
        assert_eq!(stream_message.body_text.as_deref(), Some("Plan next"));
        assert_eq!(stream_message.message_type, MessageType::Reasoning);
        assert_eq!(stream_message.status, "streaming");
        assert_eq!(stream_message.source_seq, Some(2));
        assert_eq!(
            event_type_for_message(stream_message),
            "task.reasoning.delta"
        );
        let streaming_cursor = message_cursor_event_id(stream_message);

        service
            .complete_streamed_task_reasoning("job_test")
            .unwrap();

        let completed_messages = service
            .db
            .list_projected_task_messages_page("job_test", None, Some(20))
            .unwrap();
        let completed = completed_messages
            .iter()
            .find(|message| message.source_kind == "model_stream")
            .unwrap();
        assert_eq!(completed.status, "completed");
        assert_eq!(completed.title.as_deref(), Some("Task reasoning"));
        assert!(
            message_cursor_event_id(completed) > streaming_cursor,
            "completed stream replacement must advance the message-feed cursor so SSE clients receive it"
        );
    }

    #[test]
    fn task_assistant_text_message_uses_task_message_event_type() {
        let mut message = new_message(
            "workspace_test",
            "container_test",
            MessageLane::Task,
            MessageRole::Assistant,
            MessageType::Text,
            Some("I need one more tool result.".into()),
            None,
        );
        message.title = Some("Model message".into());

        assert_eq!(event_type_for_message(&message), "task.message");
    }

    #[test]
    fn task_status_projection_keeps_user_cancelled_status_final() {
        let events = vec![
            event(1, "job_status_changed", json!({"status": "running"})),
            event(2, "job_cancelled", json!({"reason": "用户强制关闭"})),
            event(3, "job_completed", json!({})),
        ];

        let (status, timestamp_ms) = task_status_from_events(&events).unwrap();
        assert_eq!(status, "cancelled");
        assert_eq!(timestamp_ms, 2);
    }

    #[test]
    fn task_status_projection_does_not_downgrade_completed_with_later_cancel() {
        let events = vec![
            event(1, "job_status_changed", json!({"status": "running"})),
            event(2, "job_completed", json!({})),
            event(3, "job_cancelled", json!({"reason": "late force close"})),
        ];

        let (status, timestamp_ms) = task_status_from_events(&events).unwrap();
        assert_eq!(status, "completed");
        assert_eq!(timestamp_ms, 2);
    }

    #[test]
    fn force_close_completed_task_is_noop_and_closes_runtime_phase_as_completed() {
        let service = test_service("force_close_completed_noop");
        let mut task = task_record();
        task.status = "completed".into();
        service.db.upsert_task(&task).unwrap();
        service.append_runtime_phase(&task, "streaming");

        let result = service
            .force_close(&task.task_id, Some("late user click".into()))
            .unwrap();

        assert_eq!(result.status, "completed");
        assert_eq!(
            service.db.get_task(&task.task_id).unwrap().status,
            "completed"
        );
        let runtime = service.runtime_phase_message(&task.task_id);
        assert_eq!(runtime.status, "completed");
        assert_eq!(runtime.title.as_deref(), Some("TaskRuntime completed"));
    }

    #[test]
    fn force_close_running_task_cancels_and_closes_runtime_phase() {
        let service = test_service("force_close_running");
        let mut task = task_record();
        task.status = "running".into();
        service.db.upsert_task(&task).unwrap();
        service.append_runtime_phase(&task, "streaming");

        let result = service
            .force_close(&task.task_id, Some("user requested stop".into()))
            .unwrap();

        assert_eq!(result.status, "cancelled");
        assert_eq!(
            service.db.get_task(&task.task_id).unwrap().status,
            "cancelled"
        );
        let runtime = service.runtime_phase_message(&task.task_id);
        assert_eq!(runtime.status, "cancelled");
        assert_eq!(runtime.title.as_deref(), Some("TaskRuntime stopped"));
    }

    #[test]
    fn force_close_waiting_user_task_cancels_and_closes_runtime_phase() {
        let service = test_service("force_close_waiting_user");
        let mut task = task_record();
        task.status = "waiting_user".into();
        service.db.upsert_task(&task).unwrap();
        service.append_runtime_phase(&task, "streaming");

        let result = service
            .force_close(&task.task_id, Some("user requested stop".into()))
            .unwrap();

        assert_eq!(result.status, "cancelled");
        assert_eq!(
            service.db.get_task(&task.task_id).unwrap().status,
            "cancelled"
        );
        assert_eq!(
            service.runtime_phase_message(&task.task_id).status,
            "cancelled"
        );
    }

    #[test]
    fn repair_workspace_projection_recovers_database_locked_task_split_brain() {
        let workspace_root = temp_root("repair_split_brain_workspace");
        let state_root = temp_root("repair_split_brain_state");
        let provider_root = temp_root("repair_split_brain_provider");
        let db = ProductDb::open(&state_root, "workspace_repair_split_brain".into()).unwrap();
        let kernel = KernelBridge::new(workspace_root.clone(), state_root.clone(), provider_root);
        let context_pack =
            ContextPackService::new(db.clone(), kernel.clone(), workspace_root.clone());
        let run_manager = RunManager::new(db.clone());
        let settings = test_settings("repair_split_brain", &kernel);
        let service = TaskService::new(db, kernel, context_pack, run_manager, settings);

        let mut task = task_record();
        task.status = "running".into();
        service.db.upsert_task(&task).unwrap();
        service.append_runtime_phase(&task, "streaming");
        let run = service
            .run_manager
            .start_task_run(&task.container_id)
            .unwrap();
        service
            .run_manager
            .bind_task_run(&run.run_id, &task.task_id, task.job_id.as_deref().unwrap())
            .unwrap();
        service
            .run_manager
            .fail_run(&run.run_id, "database is locked")
            .unwrap();
        let mut lock_error = new_message(
            &service.db.workspace_uid,
            &task.container_id,
            MessageLane::Task,
            MessageRole::System,
            MessageType::Error,
            Some("database is locked".into()),
            None,
        );
        lock_error.source_kind = "kernel_bridge".into();
        lock_error.source_ref = "task_start".into();
        service.db.append_message(lock_error).unwrap();

        let truth =
            ProcessTruthStore::new_with_state_root(&workspace_root, &state_root, &task.task_id)
                .unwrap();
        truth
            .append_event(
                Some("root"),
                "job_status_changed",
                json!({"status": "running"}),
            )
            .unwrap();
        truth
            .append_event(
                Some("root"),
                "completion_statement_recorded",
                json!({"message": "done"}),
            )
            .unwrap();
        truth
            .append_event(Some("root"), "job_completed", json!({}))
            .unwrap();

        let report = service.repair_workspace_projection().unwrap();

        assert_eq!(report.tasks_scanned, 1);
        assert_eq!(report.tasks_with_truth, 1);
        assert_eq!(report.tasks_reconciled, 1);
        assert_eq!(report.runs_repaired, 1);
        assert_eq!(report.database_locked_messages_downgraded, 1);
        assert_eq!(
            service.db.get_task(&task.task_id).unwrap().status,
            "completed"
        );
        let repaired_run = service.db.get_run(&run.run_id).unwrap();
        assert_eq!(repaired_run.status, "completed");
        assert!(repaired_run.error_message.is_none());
        assert_eq!(
            service.runtime_phase_message(&task.task_id).status,
            "completed"
        );
        let messages = service
            .db
            .list_projected_task_messages_page(&task.task_id, None, Some(20))
            .unwrap();
        assert!(messages.iter().any(|message| {
            message.source_kind == "process_truth"
                && message.title.as_deref() == Some("Task completed")
        }));
        let container_messages = service
            .db
            .list_container_messages(&task.container_id)
            .unwrap();
        let recovered = container_messages
            .iter()
            .find(|message| {
                message.source_kind == "kernel_bridge" && message.source_ref == "task_start"
            })
            .unwrap();
        assert_eq!(recovered.message_type, MessageType::Phase);
        assert_eq!(
            recovered.body_json["status"].as_str(),
            Some("projection_error_recovered")
        );
    }

    #[test]
    fn hydrate_task_repairs_stale_running_run_from_process_truth() {
        let workspace_root = temp_root("repair_stale_running_workspace");
        let state_root = temp_root("repair_stale_running_state");
        let provider_root = temp_root("repair_stale_running_provider");
        let db = ProductDb::open(&state_root, "workspace_repair_stale_running".into()).unwrap();
        let kernel = KernelBridge::new(workspace_root.clone(), state_root.clone(), provider_root);
        let context_pack =
            ContextPackService::new(db.clone(), kernel.clone(), workspace_root.clone());
        let run_manager = RunManager::new(db.clone());
        let settings = test_settings("repair_stale_running", &kernel);
        let service = TaskService::new(db, kernel, context_pack, run_manager, settings);
        let mut task = task_record();
        task.status = "running".into();
        service.db.upsert_task(&task).unwrap();
        service.append_runtime_phase(&task, "streaming");
        let run = service
            .run_manager
            .start_task_run(&task.container_id)
            .unwrap();
        service
            .run_manager
            .bind_task_run(&run.run_id, &task.task_id, task.job_id.as_deref().unwrap())
            .unwrap();

        let truth =
            ProcessTruthStore::new_with_state_root(&workspace_root, &state_root, &task.task_id)
                .unwrap();
        truth
            .append_event(
                Some("root"),
                "job_status_changed",
                json!({"status": "running"}),
            )
            .unwrap();
        truth
            .append_event(Some("root"), "job_completed", json!({}))
            .unwrap();

        let detail = service.get(&task.task_id).unwrap();

        assert_eq!(detail.task.status, "completed");
        assert_eq!(service.db.get_run(&run.run_id).unwrap().status, "completed");
    }

    #[test]
    fn repair_container_stale_task_runs_uses_process_truth_for_run_page() {
        let workspace_root = temp_root("repair_container_stale_workspace");
        let state_root = temp_root("repair_container_stale_state");
        let provider_root = temp_root("repair_container_stale_provider");
        let db = ProductDb::open(&state_root, "workspace_repair_container_stale".into()).unwrap();
        let kernel = KernelBridge::new(workspace_root.clone(), state_root.clone(), provider_root);
        let context_pack =
            ContextPackService::new(db.clone(), kernel.clone(), workspace_root.clone());
        let run_manager = RunManager::new(db.clone());
        let settings = test_settings("repair_container_stale", &kernel);
        let service = TaskService::new(db, kernel, context_pack, run_manager, settings);
        let mut task = task_record();
        task.container_id = "container_target".into();
        task.task_id = "job_target".into();
        task.job_id = Some("job_target".into());
        task.status = "running".into();
        service.db.upsert_task(&task).unwrap();
        let target_run = service
            .run_manager
            .start_task_run(&task.container_id)
            .unwrap();
        service
            .run_manager
            .bind_task_run(
                &target_run.run_id,
                &task.task_id,
                task.job_id.as_deref().unwrap(),
            )
            .unwrap();
        let unrelated_run = service
            .run_manager
            .start_task_run("container_other")
            .unwrap();

        let truth =
            ProcessTruthStore::new_with_state_root(&workspace_root, &state_root, &task.task_id)
                .unwrap();
        truth
            .append_event(
                Some("root"),
                "job_status_changed",
                json!({"status": "running"}),
            )
            .unwrap();
        truth
            .append_event(Some("root"), "job_completed", json!({}))
            .unwrap();

        let repaired = service
            .repair_container_stale_task_runs(&task.container_id)
            .unwrap();

        assert_eq!(repaired, 1);
        assert_eq!(
            service.db.get_run(&target_run.run_id).unwrap().status,
            "completed"
        );
        assert_eq!(
            service.db.get_run(&unrelated_run.run_id).unwrap().status,
            "running"
        );
    }

    fn event(event_id: u64, event_type: &str, data: Value) -> ProcessEvent {
        ProcessEvent {
            schema_version: "process_truth.event.v1".into(),
            event_id,
            timestamp_ms: event_id as u128,
            job_id: "job_test".into(),
            pid: Some("root".into()),
            event_type: event_type.into(),
            data,
        }
    }

    trait TaskServiceTestExt {
        fn append_runtime_phase(&self, task: &TaskRecord, status: &str);
        fn runtime_phase_message(&self, task_id: &str) -> ContainerMessage;
    }

    impl TaskServiceTestExt for TaskService {
        fn append_runtime_phase(&self, task: &TaskRecord, status: &str) {
            let mut message = new_message(
                &self.db.workspace_uid,
                &task.container_id,
                MessageLane::Task,
                MessageRole::System,
                MessageType::Phase,
                Some("Kernel TaskRuntime started and is processing this task.".into()),
                None,
            );
            message.status = status.into();
            message.title = Some("TaskRuntime running".into());
            message.task_id = Some(task.task_id.clone());
            message.job_id = task.job_id.clone();
            message.source_kind = "product_runtime_runtime".into();
            message.source_ref = "task_runtime_running".into();
            self.db.append_message(message).unwrap();
        }

        fn runtime_phase_message(&self, task_id: &str) -> ContainerMessage {
            self.db
                .list_task_messages(task_id)
                .unwrap()
                .into_iter()
                .find(|message| {
                    message.source_kind == "product_runtime_runtime"
                        && message.source_ref == "task_runtime_running"
                })
                .unwrap()
        }
    }

    #[test]
    fn task_detail_projection_reconciles_selected_output_with_verified_artifacts() {
        let task = task_record();
        let events = vec![
            event(
                1,
                "task_artifact_destination_guidance_attached",
                json!({"selected_output_dir": "reports"}),
            ),
            event(
                2,
                "capability_receipt",
                json!({
                    "capability_id": "os.write_artifact",
                    "status": "success",
                    "data": {"artifact_path": "reports/out.md"}
                }),
            ),
            event(
                3,
                "verify_event",
                json!({
                    "capability_id": "os.verify_artifact",
                    "status": "success",
                    "data": {"artifact_path": "reports/out.md", "exists": true}
                }),
            ),
        ];

        let projection = project_task_detail(&task, &events);

        assert_eq!(projection.selected_output_dir.as_deref(), Some("reports"));
        assert_eq!(projection.destination_fulfilled, Some(true));
        assert_eq!(projection.artifacts.len(), 1);
        assert!(projection.artifacts[0].verified);
        assert_eq!(projection.badges(&task.status).artifact_ready, 1);
    }

    #[test]
    fn task_detail_projection_tracks_approval_resolution() {
        let task = task_record();
        let events = vec![
            event(
                1,
                "preview_tx_created",
                json!({
                    "preview_id": "preview_1",
                    "preview_ref": "blob://job_test/previews/preview_1.md",
                    "executable_operations": [{"capability_id": "os.write_artifact"}]
                }),
            ),
            event(
                2,
                "preview_tx_approved",
                json!({
                    "preview_id": "preview_1",
                    "approval_token_id": "approval_1",
                    "status": "approved"
                }),
            ),
        ];

        let projection = project_task_detail(&task, &events);

        assert!(projection.approvals.is_empty());
        assert_eq!(projection.badges(&task.status).approval, 0);
    }

    #[test]
    fn task_detail_projection_closes_pending_approval_when_waiting_user() {
        let mut task = task_record();
        task.status = "waiting_user".into();
        let events = vec![
            event(
                1,
                "preview_tx_created",
                json!({
                    "preview_id": "preview_1",
                    "preview_ref": "blob://job_test/previews/preview_1.md",
                    "executable_operations": [{"capability_id": "os.delete_path"}]
                }),
            ),
            event(
                2,
                "job_waiting_user",
                json!({
                    "question": "Which file should be deleted?",
                    "status": "waiting_user",
                }),
            ),
        ];

        let projection = project_task_detail(&task, &events);

        assert!(projection.approvals.is_empty());
        assert_eq!(projection.badges(&task.status).approval, 0);
    }

    #[test]
    fn pending_approval_preview_syncs_product_db_draft_artifact() {
        let service = test_service("draft_projection");
        let mut task = task_record();
        task.status = "waiting_approval".into();
        service.db.upsert_task(&task).unwrap();
        let events = vec![event(
            1,
            "preview_tx_created",
            json!({
                "preview_id": "preview_1",
                "preview_ref": "blob://job_test/previews/preview_1.md",
                "human_preview_markdown": "# Draft\n\nReview me.",
                "executable_operations": [{"capability_id": "os.write_artifact"}]
            }),
        )];
        let mut projection = project_task_detail(&task, &events);

        service
            .sync_task_draft_artifacts(&task, &mut projection)
            .unwrap();

        assert!(projection.approvals.is_empty());
        assert!(service
            .db
            .get_task_draft_artifact("job_test", "preview_1")
            .unwrap()
            .is_none());
    }

    #[test]
    fn pending_approval_prefers_executable_operation_content_for_draft_artifact() {
        let service = test_service("draft_operation_content");
        let mut task = task_record();
        task.status = "waiting_approval".into();
        service.db.upsert_task(&task).unwrap();
        let events = vec![event(
            1,
            "preview_tx_created",
            json!({
                "preview_id": "preview_1",
                "preview_ref": "blob://job_test/previews/preview_1.md",
                "human_preview_markdown": "# Provider Native Write Preview\n\n## Draft content\n\n```text\n# Wrapped\n```",
                "executable_operations": [{
                    "capability_id": "os.write_artifact",
                    "arguments": {
                        "path": "artifacts/report.md",
                        "content": "# Report\n\n## 摘要\n正文。"
                    }
                }]
            }),
        )];
        let mut projection = project_task_detail(&task, &events);

        service
            .sync_task_draft_artifacts(&task, &mut projection)
            .unwrap();

        assert!(projection.approvals.is_empty());
    }

    #[test]
    fn pending_approval_unwraps_fenced_draft_content_from_human_preview() {
        let service = test_service("draft_fenced_preview");
        let mut task = task_record();
        task.status = "waiting_approval".into();
        service.db.upsert_task(&task).unwrap();
        let events = vec![event(
            1,
            "preview_tx_created",
            json!({
                "preview_id": "preview_1",
                "human_preview_markdown": "# Provider Native Write Preview\n\n## Draft content\n\n```text\n# Report\n\n## 摘要\n正文。\n```",
                "executable_operations": [{"capability_id": "os.write_artifact"}]
            }),
        )];
        let mut projection = project_task_detail(&task, &events);

        service
            .sync_task_draft_artifacts(&task, &mut projection)
            .unwrap();

        assert!(projection.approvals.is_empty());
    }

    #[test]
    fn resolved_approval_deletes_product_db_draft_artifact() {
        let service = test_service("draft_cleanup");
        let mut task = task_record();
        task.status = "waiting_approval".into();
        let draft = TaskDraftArtifactRecord {
            draft_id: "draft_job_test_preview_1".into(),
            workspace_uid: service.db.workspace_uid.clone(),
            container_id: task.container_id.clone(),
            task_id: task.task_id.clone(),
            approval_id: "preview_1".into(),
            preview_ref: Some("blob://preview".into()),
            operation: Some("os.write_artifact".into()),
            status: "pending".into(),
            content_format: "markdown".into(),
            content_text: "# Draft".into(),
            created_at_ms: 1,
            updated_at_ms: 1,
        };
        service.db.upsert_task_draft_artifact(&draft).unwrap();
        let events = vec![
            event(
                1,
                "preview_tx_created",
                json!({
                    "preview_id": "preview_1",
                    "human_preview_markdown": "# Draft",
                    "executable_operations": [{"capability_id": "os.write_artifact"}]
                }),
            ),
            event(2, "preview_tx_approved", json!({"preview_id": "preview_1"})),
        ];
        let mut projection = project_task_detail(&task, &events);

        service
            .sync_task_draft_artifacts(&task, &mut projection)
            .unwrap();

        assert!(projection.approvals.is_empty());
        assert!(service
            .db
            .get_task_draft_artifact("job_test", "preview_1")
            .unwrap()
            .is_none());
    }

    #[test]
    fn task_detail_projection_counts_completed_claimed_artifacts_as_ready() {
        let task = task_record();
        let events = vec![
            event(
                1,
                "completion_statement_recorded",
                json!({
                    "claimed_artifacts": ["reports/final.md"],
                    "completion_statement": "done"
                }),
            ),
            event(
                2,
                "job_completed",
                json!({
                    "artifacts": ["reports/final.md"],
                    "all_artifacts": ["reports/final.md"],
                    "claimed_artifacts": ["reports/final.md"]
                }),
            ),
        ];

        let projection = project_task_detail(&task, &events);

        assert_eq!(projection.artifacts.len(), 1);
        assert_eq!(
            projection.artifacts[0].path.as_deref(),
            Some("reports/final.md")
        );
        assert_eq!(projection.artifacts[0].status, "ready");
        assert!(!projection.artifacts[0].verified);
        assert_eq!(projection.badges(&task.status).artifact_ready, 1);
    }

    #[test]
    fn task_status_reconciliation_prefers_process_truth_terminal_status() {
        let service = test_service("status_reconcile");
        let mut task = task_record();
        task.status = "waiting_approval".into();
        service.db.upsert_task(&task).unwrap();
        let events = vec![
            event(
                1,
                "job_status_changed",
                json!({"status": "waiting_approval"}),
            ),
            event(2, "job_status_changed", json!({"status": "completed"})),
            event(
                3,
                "job_completed",
                json!({"artifacts": ["reports/final.md"]}),
            ),
        ];

        let reconciled = service.reconcile_task_record(&task, &events).unwrap();

        assert_eq!(reconciled.status, "completed");
        assert_eq!(
            service.db.get_task(&task.task_id).unwrap().status,
            "completed"
        );
    }

    #[test]
    fn task_detail_projection_expands_package_multi_artifacts_and_receipts() {
        let task = task_record();
        let events = vec![
            event(
                1,
                "capability_receipt",
                json!({
                    "capability_id": "package.build_zip",
                    "status": "success",
                    "data": {
                        "artifact_path": "deliverables/bundle.zip",
                        "artifacts": [
                            "deliverables/bundle.zip",
                            "PACK_MANIFEST.md",
                            "SHA256SUMS.txt",
                            "PERF_NOTES.json"
                        ],
                        "manifest_path": "PACK_MANIFEST.md",
                        "checksums_path": "SHA256SUMS.txt",
                        "perf_notes_path": "PERF_NOTES.json"
                    }
                }),
            ),
            event(
                2,
                "capability_receipt",
                json!({
                    "capability_id": "artifact.verify_typed",
                    "status": "success",
                    "data": {"artifact_path": "deliverables/bundle.zip"}
                }),
            ),
        ];

        let projection = project_task_detail(&task, &events);

        assert_eq!(projection.artifacts.len(), 4);
        assert_eq!(projection.receipts.len(), 2);
        assert!(projection
            .artifacts
            .iter()
            .any(
                |artifact| artifact.path.as_deref() == Some("deliverables/bundle.zip")
                    && artifact.verified
            ));
        assert!(projection
            .receipts
            .iter()
            .any(
                |receipt| receipt.capability_id.as_deref() == Some("package.build_zip")
                    && receipt.artifact_paths.len() == 4
            ));
    }

    #[test]
    fn task_detail_projection_records_rollback_receipts_without_artifact() {
        let task = task_record();
        let events = vec![event(
            1,
            "capability_receipt",
            json!({
                "capability_id": "os.rollback_tx",
                "status": "success",
                "data": {
                    "tx_id": "tx_123",
                    "rolled_back_operation": "office.docx.rewrite_in_place"
                }
            }),
        )];

        let projection = project_task_detail(&task, &events);

        assert!(projection.artifacts.is_empty());
        assert_eq!(projection.receipts.len(), 1);
        assert_eq!(
            projection.receipts[0].capability_id.as_deref(),
            Some("os.rollback_tx")
        );
        assert_eq!(projection.receipts[0].status, "success");
    }
}

fn task_job_id(task: &TaskRecord) -> rusqlite::Result<String> {
    task.job_id.clone().ok_or_else(|| {
        rusqlite::Error::InvalidParameterName("task has no job_id for approval action".into())
    })
}

fn io_to_sqlite(err: std::io::Error) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(Box::new(err))
}

fn sqlite_to_io(err: rusqlite::Error) -> io::Error {
    io::Error::new(io::ErrorKind::Other, err)
}

#[derive(Debug)]
struct ProductTaskStreamSink {
    workspace_uid: String,
    container_id: String,
    state: Mutex<ProductTaskStreamState>,
}

#[derive(Debug, Default)]
struct ProductTaskStreamState {
    task_id: Option<String>,
    projection_shard: Option<ProjectionShardDb>,
    messages: BTreeMap<String, ProductTaskStreamMessageState>,
    pending_deltas: Vec<ModelStreamDelta>,
}

#[derive(Debug)]
struct ProductTaskStreamMessageState {
    message_id: String,
    created_at_ms: u128,
    body_text: String,
}

impl ProductTaskStreamSink {
    fn new(workspace_uid: String, container_id: String) -> Self {
        Self {
            workspace_uid,
            container_id,
            state: Mutex::new(ProductTaskStreamState::default()),
        }
    }

    fn bind_job(&self, job_id: &str, projection_shard: ProjectionShardDb) {
        let mut state = self.state.lock().unwrap_or_else(|err| err.into_inner());
        state.task_id = Some(job_id.to_string());
        state.projection_shard = Some(projection_shard);
        let pending = std::mem::take(&mut state.pending_deltas);
        drop(state);
        for delta in pending {
            self.project_delta(delta);
        }
    }

    fn project_delta(&self, delta: ModelStreamDelta) {
        if delta.kind != ModelStreamDeltaKind::Reasoning || delta.delta.is_empty() {
            return;
        }
        let mut state = self.state.lock().unwrap_or_else(|err| err.into_inner());
        let Some(task_id) = state.task_id.clone() else {
            state.pending_deltas.push(delta);
            return;
        };
        let Some(projection_shard) = state.projection_shard.clone() else {
            state.pending_deltas.push(delta);
            return;
        };
        let message = state
            .messages
            .entry(delta.model_call_id.clone())
            .or_insert_with(|| ProductTaskStreamMessageState {
                message_id: format!(
                    "task_stream_{}_{}",
                    safe_message_component(&task_id),
                    safe_message_component(&delta.model_call_id)
                ),
                created_at_ms: now_ms() as u128,
                body_text: String::new(),
            });
        message.body_text.push_str(&delta.delta);
        let mut container_message = new_message(
            &self.workspace_uid,
            &self.container_id,
            MessageLane::Task,
            MessageRole::Agent,
            MessageType::Reasoning,
            Some(message.body_text.clone()),
            None,
        );
        container_message.message_id = message.message_id.clone();
        container_message.created_at_ms = message.created_at_ms;
        container_message.status = "streaming".into();
        container_message.title = Some("Task reasoning streaming".into());
        container_message.task_id = Some(task_id.clone());
        container_message.job_id = Some(task_id);
        container_message.source_kind = "model_stream".into();
        container_message.source_ref = delta.model_call_id.clone();
        container_message.source_seq = Some(i64::from(delta.sequence));
        container_message.body_json = json!({
            "schema": "supernova_task_model_stream.v1",
            "model_call_id": delta.model_call_id,
            "operation": delta.operation.as_str(),
            "stream_kind": "reasoning",
            "delta_sequence": delta.sequence,
            "execution_fact_boundary": "Displayed model reasoning stream only; Task execution remains governed by validated decisions, tool calls, and Kernel receipts.",
        });
        let _ = projection_shard.append_message(container_message);
    }
}

impl ModelStreamSink for ProductTaskStreamSink {
    fn on_model_stream_delta(&self, delta: ModelStreamDelta) {
        self.project_delta(delta);
    }
}

fn safe_message_component(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn event_type_for_message(message: &ContainerMessage) -> &'static str {
    if message.message_type == MessageType::Reasoning {
        return "task.reasoning.delta";
    }
    if message.role == MessageRole::Assistant
        && message.message_type == MessageType::Text
        && message.title.as_deref() == Some("Model message")
    {
        return "task.message";
    }
    match message.message_type {
        MessageType::ToolCall => "task.tool.call",
        MessageType::ToolResult => "task.tool.result",
        MessageType::Approval => "task.approval.required",
        MessageType::Artifact => "task.artifact.ready",
        MessageType::Error => "task.error",
        MessageType::Phase => "task.phase",
        _ => "task.phase",
    }
}

fn sse_message(
    event_type: &'static str,
    workspace_uid: &str,
    message: Option<ContainerMessage>,
) -> Event {
    let payload = serde_json::to_value(TaskStreamPayload {
        phase: message.as_ref().and_then(|value| value.body_text.clone()),
        message: message.clone(),
        approval: None,
        artifact: None,
        receipt: None,
    })
    .unwrap_or_else(|_| json!({}));
    sse::protocol_message_event(event_type, workspace_uid, payload, message.as_ref())
}

fn sse_payload(
    event_type: &'static str,
    workspace_uid: &str,
    after_event_id: Option<i64>,
    payload: Value,
) -> Event {
    sse::protocol_payload_event(
        event_type,
        workspace_uid,
        local_runtime_protocol::Cursor {
            kind: "message_feed".into(),
            after: None,
            after_event_id,
        },
        payload,
    )
}
