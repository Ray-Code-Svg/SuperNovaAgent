use std::env;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::process;

use serde::Serialize;
use serde_json::json;
use serde_json::Value;
use supernova_process_kernel::{
    create_agent_job, create_agent_job_with_state_root, default_capability_registry,
    default_model_provider_from_env, CapabilityToken, ChatTurnRequest, ContainerTimelineItemKind,
    ContextPack, ContextWindowControlConfig, KernelApi, ModelAction, ModelBudget,
    ModelFailurePolicy, ModelInvocationConfig, ModelOperation, ModelRuntime, OsRuntime,
    ProcessTruthStore, WorkspaceGuard,
};

fn main() {
    if let Err(err) = run() {
        let payload = json!({
            "ok": false,
            "error": {
                "code": "KERNEL_CLI_ERROR",
                "message": err.to_string(),
            }
        });
        println!("{}", serde_json::to_string(&payload).unwrap());
        process::exit(1);
    }
}

fn run() -> io::Result<()> {
    let mut args = env::args().skip(1).collect::<Vec<_>>();
    if args.is_empty() {
        return Err(invalid("missing command"));
    }
    let command = args.remove(0);
    match command.as_str() {
        "manifest" => {
            let workspace = take_value(&mut args, "--workspace")?;
            let _ = kernel_api(&mut args, &workspace)?;
            print_ok(json!({"capabilities": default_capability_registry()}))
        }
        "start-job" => {
            let workspace = take_value(&mut args, "--workspace")?;
            let goal = take_value(&mut args, "--goal")?;
            let max_turns = take_optional_usize(&mut args, "--max-turns")?;
            let model_config = take_optional_model_config(&mut args)?;
            let api = kernel_api(&mut args, &workspace)?;
            print_ok(api.start_job_with_config(&goal, max_turns, model_config)?)
        }
        "start-container-task" => {
            let workspace = take_value(&mut args, "--workspace")?;
            let container_id = take_value(&mut args, "--container-id")?;
            let goal = take_value(&mut args, "--goal")?;
            let max_turns = take_optional_usize(&mut args, "--max-turns")?;
            let model_config = take_optional_model_config(&mut args)?;
            let context_pack_id = take_optional_value(&mut args, "--context-pack-id");
            let auto_approve = take_flag(&mut args, "--auto-approve");
            let api = kernel_api(&mut args, &workspace)?;
            print_ok(api.start_task_in_container_with_options(
                &container_id,
                &goal,
                max_turns,
                model_config,
                context_pack_id,
                auto_approve,
            )?)
        }
        "create-container" => {
            let workspace = take_value(&mut args, "--workspace")?;
            let title = take_optional_value(&mut args, "--title");
            let default_model_config = take_optional_model_config_arg(&mut args)?;
            let context_policy = take_optional_context_policy(&mut args)?;
            let api = kernel_api(&mut args, &workspace)?;
            print_ok(api.create_container(title, default_model_config, context_policy)?)
        }
        "list-containers" => {
            let workspace = take_value(&mut args, "--workspace")?;
            let api = kernel_api(&mut args, &workspace)?;
            print_ok(json!({"items": api.list_containers()?}))
        }
        "get-container" => {
            let workspace = take_value(&mut args, "--workspace")?;
            let container_id = take_value(&mut args, "--container-id")?;
            let api = kernel_api(&mut args, &workspace)?;
            print_ok(api.get_container(&container_id)?)
        }
        "archive-container" => {
            let workspace = take_value(&mut args, "--workspace")?;
            let container_id = take_value(&mut args, "--container-id")?;
            let api = kernel_api(&mut args, &workspace)?;
            api.archive_container(&container_id)?;
            print_ok(json!({"container_id": container_id, "status": "archived"}))
        }
        "update-container" => {
            let workspace = take_value(&mut args, "--workspace")?;
            let container_id = take_value(&mut args, "--container-id")?;
            let title = take_optional_value(&mut args, "--title");
            let status = take_optional_value(&mut args, "--status");
            let default_model_config = take_optional_model_config_arg(&mut args)?;
            let context_policy = take_optional_context_policy(&mut args)?;
            let api = kernel_api(&mut args, &workspace)?;
            print_ok(api.update_container(
                &container_id,
                title,
                status,
                default_model_config,
                context_policy,
            )?)
        }
        "append-container-timeline" => {
            let workspace = take_value(&mut args, "--workspace")?;
            let container_id = take_value(&mut args, "--container-id")?;
            let item_kind = take_value(&mut args, "--item-kind")?;
            let ref_id = take_value(&mut args, "--ref-id")?;
            let status =
                take_optional_value(&mut args, "--status").unwrap_or_else(|| "active".to_string());
            let title = take_optional_value(&mut args, "--title");
            let summary_ref = take_optional_value(&mut args, "--summary-ref");
            let api = kernel_api(&mut args, &workspace)?;
            print_ok(api.append_container_timeline_item(
                &container_id,
                ContainerTimelineItemKind::from_str(&item_kind),
                &ref_id,
                &status,
                title,
                summary_ref,
            )?)
        }
        "container-timeline" => {
            let workspace = take_value(&mut args, "--workspace")?;
            let container_id = take_value(&mut args, "--container-id")?;
            let limit = take_optional_value(&mut args, "--limit")
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(200);
            let api = kernel_api(&mut args, &workspace)?;
            print_ok(
                json!({"container_id": container_id, "items": api.list_container_timeline(&container_id, limit)?}),
            )
        }
        "container-tasks" => {
            let workspace = take_value(&mut args, "--workspace")?;
            let container_id = take_value(&mut args, "--container-id")?;
            let limit = take_optional_value(&mut args, "--limit")
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(200);
            let api = kernel_api(&mut args, &workspace)?;
            print_ok(
                json!({"container_id": container_id, "items": api.list_container_tasks(&container_id, limit)?}),
            )
        }
        "upsert-context-pack" => {
            let workspace = take_value(&mut args, "--workspace")?;
            let raw = take_value(&mut args, "--context-pack-json")?;
            let pack = serde_json::from_str::<ContextPack>(&raw)
                .map_err(|err| invalid(format!("invalid --context-pack-json: {err}")))?;
            let api = kernel_api(&mut args, &workspace)?;
            print_ok(api.upsert_context_pack(pack)?)
        }
        "get-context-pack" => {
            let workspace = take_value(&mut args, "--workspace")?;
            let context_pack_id = take_value(&mut args, "--context-pack-id")?;
            let api = kernel_api(&mut args, &workspace)?;
            print_ok(api.get_context_pack(&context_pack_id)?)
        }
        "latest-context-pack" => {
            let workspace = take_value(&mut args, "--workspace")?;
            let container_id = take_value(&mut args, "--container-id")?;
            let api = kernel_api(&mut args, &workspace)?;
            print_ok(
                json!({"container_id": container_id, "context_pack": api.latest_context_pack(&container_id)?}),
            )
        }
        "estimate-context-pack" => {
            let workspace = take_value(&mut args, "--workspace")?;
            let raw = take_value(&mut args, "--context-pack-json")?;
            let pack = serde_json::from_str::<ContextPack>(&raw)
                .map_err(|err| invalid(format!("invalid --context-pack-json: {err}")))?;
            let api = kernel_api(&mut args, &workspace)?;
            print_ok(api.estimate_context_pack(&pack)?)
        }
        "bind-container-memory" => {
            let workspace = take_value(&mut args, "--workspace")?;
            let container_id = take_value(&mut args, "--container-id")?;
            let memory_ref = take_value(&mut args, "--memory-ref")?;
            let include_mode = take_optional_value(&mut args, "--include-mode")
                .unwrap_or_else(|| "summary".to_string());
            let priority = take_optional_value(&mut args, "--priority")
                .and_then(|value| value.parse::<u8>().ok())
                .unwrap_or(50);
            let api = kernel_api(&mut args, &workspace)?;
            print_ok(api.bind_memory(&container_id, &memory_ref, &include_mode, priority)?)
        }
        "list-container-memories" => {
            let workspace = take_value(&mut args, "--workspace")?;
            let container_id = take_value(&mut args, "--container-id")?;
            let api = kernel_api(&mut args, &workspace)?;
            print_ok(
                json!({"container_id": container_id, "items": api.list_memory_bindings(&container_id)?}),
            )
        }
        "unbind-container-memory" => {
            let workspace = take_value(&mut args, "--workspace")?;
            let binding_id = take_value(&mut args, "--binding-id")?;
            let api = kernel_api(&mut args, &workspace)?;
            api.unbind_memory(&binding_id)?;
            print_ok(json!({"binding_id": binding_id, "status": "unbound"}))
        }
        "task-context-window" => {
            let workspace = take_value(&mut args, "--workspace")?;
            let job_id = take_value(&mut args, "--job-id")?;
            let api = kernel_api(&mut args, &workspace)?;
            print_ok(api.task_context_window(&job_id)?)
        }
        "chat-context-window" => {
            let workspace = take_value(&mut args, "--workspace")?;
            let chat_thread_id = take_value(&mut args, "--chat-thread-id")?;
            let api = kernel_api(&mut args, &workspace)?;
            print_ok(api.chat_context_window(&chat_thread_id)?)
        }
        "compact-chat-thread" => {
            let workspace = take_value(&mut args, "--workspace")?;
            let chat_thread_id = take_value(&mut args, "--chat-thread-id")?;
            let api = kernel_api(&mut args, &workspace)?;
            print_ok(api.compact_chat_thread(&chat_thread_id)?)
        }
        "compact-task-context" => {
            let workspace = take_value(&mut args, "--workspace")?;
            let job_id = take_value(&mut args, "--job-id")?;
            let api = kernel_api(&mut args, &workspace)?;
            print_ok(api.compact_task_context(&job_id)?)
        }
        "compact-container-context" => {
            let workspace = take_value(&mut args, "--workspace")?;
            let container_id = take_value(&mut args, "--container-id")?;
            let target_runtime = take_optional_value(&mut args, "--target-runtime");
            let api = kernel_api(&mut args, &workspace)?;
            print_ok(api.compact_container_context(&container_id, target_runtime)?)
        }
        "create-chat-thread" => {
            let workspace = take_value(&mut args, "--workspace")?;
            let container_id = take_value(&mut args, "--container-id")?;
            let title = take_optional_value(&mut args, "--title");
            let api = kernel_api(&mut args, &workspace)?;
            print_ok(api.create_chat_thread(&container_id, title)?)
        }
        "list-chat-threads" => {
            let workspace = take_value(&mut args, "--workspace")?;
            let container_id = take_value(&mut args, "--container-id")?;
            let api = kernel_api(&mut args, &workspace)?;
            print_ok(
                json!({"container_id": container_id, "items": api.list_chat_threads(&container_id)?}),
            )
        }
        "chat-events" => {
            let workspace = take_value(&mut args, "--workspace")?;
            let chat_thread_id = take_value(&mut args, "--chat-thread-id")?;
            let api = kernel_api(&mut args, &workspace)?;
            print_ok(
                json!({"chat_thread_id": chat_thread_id, "events": api.read_chat_events(&chat_thread_id)?}),
            )
        }
        "start-chat-turn" => {
            let workspace = take_value(&mut args, "--workspace")?;
            let container_id = take_value(&mut args, "--container-id")?;
            let message = take_value(&mut args, "--message")?;
            let chat_thread_id = take_optional_value(&mut args, "--chat-thread-id");
            let context_pack = take_optional_context_pack_arg(&mut args)?;
            let model_config_override = take_optional_model_config_arg(&mut args)?;
            let api = kernel_api(&mut args, &workspace)?;
            print_ok(api.start_chat_turn(ChatTurnRequest {
                container_id,
                chat_thread_id,
                message,
                context_pack,
                source_guidance: None,
                model_config_override,
            })?)
        }
        "resume-job" => {
            let workspace = take_value(&mut args, "--workspace")?;
            let job_id = take_value(&mut args, "--job-id")?;
            let max_turns = take_optional_usize(&mut args, "--max-turns")?;
            let api = kernel_api(&mut args, &workspace)?;
            print_ok(api.resume_job_with_max_turns(&job_id, max_turns)?)
        }
        "approve-preview" => {
            let workspace = take_value(&mut args, "--workspace")?;
            let job_id = take_value(&mut args, "--job-id")?;
            let max_turns = take_optional_usize(&mut args, "--max-turns")?;
            let note =
                take_optional_value(&mut args, "--note").unwrap_or_else(|| "approved".to_string());
            let api = kernel_api(&mut args, &workspace)?;
            print_ok(api.approve_preview_with_max_turns(&job_id, &note, max_turns)?)
        }
        "approve-client-env-disclosure" => {
            let workspace = take_value(&mut args, "--workspace")?;
            let job_id = take_value(&mut args, "--job-id")?;
            let request_id = take_value(&mut args, "--request-id")?;
            let allowed_fields = take_string_list(&mut args, "--allowed-fields")?;
            let note =
                take_optional_value(&mut args, "--note").unwrap_or_else(|| "approved".to_string());
            let api = kernel_api(&mut args, &workspace)?;
            print_ok(api.approve_client_env_disclosure(
                &job_id,
                &request_id,
                allowed_fields,
                &note,
            )?)
        }
        "reject-client-env-disclosure" => {
            let workspace = take_value(&mut args, "--workspace")?;
            let job_id = take_value(&mut args, "--job-id")?;
            let request_id = take_value(&mut args, "--request-id")?;
            let reason = take_optional_value(&mut args, "--reason")
                .unwrap_or_else(|| "rejected".to_string());
            let api = kernel_api(&mut args, &workspace)?;
            print_ok(api.reject_client_env_disclosure(&job_id, &request_id, &reason)?)
        }
        "submit-user-input" => {
            let workspace = take_value(&mut args, "--workspace")?;
            let job_id = take_value(&mut args, "--job-id")?;
            let input = take_value(&mut args, "--input")?;
            let max_turns = take_optional_usize(&mut args, "--max-turns")?;
            let api = kernel_api(&mut args, &workspace)?;
            print_ok(api.submit_user_input_with_max_turns(&job_id, &input, max_turns)?)
        }
        "cancel-job" => {
            let workspace = take_value(&mut args, "--workspace")?;
            let job_id = take_value(&mut args, "--job-id")?;
            let reason = take_optional_value(&mut args, "--reason")
                .unwrap_or_else(|| "cancelled".to_string());
            let api = kernel_api(&mut args, &workspace)?;
            api.cancel_job(&job_id, &reason)?;
            print_ok(json!({"job_id": job_id, "status": "cancelled"}))
        }
        "status" => {
            let workspace = take_value(&mut args, "--workspace")?;
            let job_id = take_value(&mut args, "--job-id")?;
            let api = kernel_api(&mut args, &workspace)?;
            print_ok(api.get_job_status(&workspace, &job_id)?)
        }
        "events" => {
            let workspace = take_value(&mut args, "--workspace")?;
            let job_id = take_value(&mut args, "--job-id")?;
            let after = take_optional_value(&mut args, "--after")
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(0);
            let limit = take_optional_value(&mut args, "--limit")
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(400);
            let api = kernel_api(&mut args, &workspace)?;
            print_ok(json!({
                "job_id": job_id,
                "events": api.stream_job_events(&workspace, &job_id, after, limit)?,
            }))
        }
        "replay-job" => {
            let workspace = take_value(&mut args, "--workspace")?;
            let job_id = take_value(&mut args, "--job-id")?;
            let api = kernel_api(&mut args, &workspace)?;
            print_ok(api.replay_job(&workspace, &job_id)?)
        }
        "export-process-truth" => {
            let workspace = take_value(&mut args, "--workspace")?;
            let job_id = take_value(&mut args, "--job-id")?;
            let output = take_optional_value(&mut args, "--output").map(PathBuf::from);
            let truth = process_truth_store(&mut args, &workspace, &job_id)?;
            let path = truth.export_jsonl(output.unwrap_or_else(|| truth.export_path()))?;
            print_ok(json!({
                "job_id": job_id,
                "path": path.display().to_string(),
                "events": truth.read_events()?,
            }))
        }
        "rollback-tx" => {
            let workspace = take_value(&mut args, "--workspace")?;
            let job_id = take_value(&mut args, "--job-id")?;
            let tx_id = take_value(&mut args, "--tx-id")?;
            let truth = process_truth_store(&mut args, &workspace, &job_id)?;
            let root_pid = truth
                .registry_snapshot()?
                .processes
                .iter()
                .find(|process| process.process_type == "root_agent_process")
                .map(|process| process.pid.clone())
                .unwrap_or_else(|| format!("pid_rollback_{}", now_ms_local()));
            let token = CapabilityToken {
                token_id: format!("token_rollback_{root_pid}"),
                job_id: job_id.clone(),
                pid: root_pid,
                workspace_root: workspace.clone(),
                capabilities: vec!["os.rollback_tx".to_string()],
                permissions: vec!["fs:write".to_string()],
            };
            let runtime = OsRuntime::new(WorkspaceGuard::new(&workspace)?, truth, token);
            let receipt = runtime.rollback_tx(&tx_id)?;
            print_ok(json!({
                "job_id": job_id,
                "tx_id": tx_id,
                "status": receipt.status,
                "receipt": receipt,
            }))
        }
        "render-entity-reply" => {
            let workspace = take_value(&mut args, "--workspace")?;
            let session_id =
                take_optional_value(&mut args, "--session-id").unwrap_or_else(|| "".to_string());
            let operation =
                take_optional_value(&mut args, "--operation").unwrap_or_else(|| "turn".to_string());
            let next_action = take_optional_value(&mut args, "--next-action")
                .unwrap_or_else(|| "complete".to_string());
            let payload_raw =
                take_optional_value(&mut args, "--payload").unwrap_or_else(|| "{}".to_string());
            let payload: Value = serde_json::from_str(&payload_raw)
                .unwrap_or_else(|_| json!({"raw_payload": payload_raw}));
            let (job, process, truth) = create_agent_job_from_args(
                &mut args,
                &workspace,
                "Super Agent Entity reply render",
            )?;
            truth.append_event(
                Some(&process.pid),
                "entity_model_render_requested",
                json!({
                    "session_id": session_id,
                    "operation": operation,
                    "next_action": next_action,
                }),
            )?;
            let instruction_ref = truth.write_blob(
                "entity_render/instruction.txt",
                b"You are the SuperNova Super Agent Entity. Return one concise Chinese user-facing reply. Do not claim direct workspace mutation. Mention kernel/task state only when grounded in the provided context.",
            )?;
            let payload_ref = truth.write_blob(
                "entity_render/context.json",
                &serde_json::to_vec_pretty(&payload).map_err(json_err)?,
            )?;
            let provider = default_model_provider_from_env();
            let action = ModelAction {
                action_id: format!("entity_render_{}", now_ms_local()),
                job_id: job.job_id.clone(),
                pid: process.pid.clone(),
                reasoning_step_id: "entity_render".to_string(),
                operation: ModelOperation::RenderEntityReply,
                instruction_ref,
                input_refs: vec![payload_ref],
                preference_snapshot_ref: None,
                output_schema: json!({"type": "string"}),
                provider: provider.provider_name().to_string(),
                model: provider.model_name_for_operation(&ModelOperation::RenderEntityReply),
                budget: ModelBudget {
                    max_input_bytes: 64 * 1024,
                    max_output_tokens: 512,
                    timeout_ms: 15_000,
                    max_retries: 0,
                },
                failure_policy: ModelFailurePolicy::OptionalVisible,
                required: false,
            };
            let token = CapabilityToken {
                token_id: format!("token_entity_render_{}", process.pid),
                job_id: job.job_id.clone(),
                pid: process.pid.clone(),
                workspace_root: workspace.clone(),
                capabilities: vec![
                    "model.invoke".to_string(),
                    "model.render_entity_reply".to_string(),
                ],
                permissions: vec!["model:invoke".to_string()],
            };
            let model_config = take_optional_model_config(&mut args)?;
            let receipt = ModelRuntime::new(truth.clone(), token, provider)
                .with_model_invocation_config(model_config, None)
                .render_entity_reply(action)?;
            let reply = match receipt.output_ref.as_deref() {
                Some(output_ref) if receipt.status == "success" => {
                    let path = truth.resolve_blob_ref(output_ref)?;
                    fs::read_to_string(path).unwrap_or_default()
                }
                _ => "".to_string(),
            };
            truth.append_event(
                Some(&process.pid),
                "entity_model_render_completed",
                json!({
                    "session_id": session_id,
                    "operation": operation,
                    "next_action": next_action,
                    "model_call_id": receipt.model_call_id.clone(),
                    "status": receipt.status.clone(),
                    "output_ref": receipt.output_ref.clone(),
                    "ledger_ref": receipt.ledger_ref.clone(),
                }),
            )?;
            if receipt.status == "success" {
                truth.update_job_status("completed")?;
                truth.append_event(
                    Some(&process.pid),
                    "job_completed",
                    json!({"reason": "entity reply rendered", "artifacts": []}),
                )?;
            }
            print_ok(json!({
                "job_id": job.job_id,
                "root_pid": process.pid,
                "process_truth_path": truth.export_path().display().to_string(),
                "reply": reply.trim(),
                "receipt": receipt,
            }))
        }
        other => Err(invalid(format!("unknown command: {other}"))),
    }
}

fn print_ok<T: Serialize>(data: T) -> io::Result<()> {
    let payload = json!({"ok": true, "data": data});
    println!("{}", serde_json::to_string(&payload).map_err(json_err)?);
    Ok(())
}

fn kernel_api(args: &mut Vec<String>, workspace: &str) -> io::Result<KernelApi> {
    if let Some(state_root) = take_state_root(args) {
        KernelApi::new_with_state_root(workspace, state_root)
    } else {
        KernelApi::new(workspace)
    }
}

fn process_truth_store(
    args: &mut Vec<String>,
    workspace: &str,
    job_id: &str,
) -> io::Result<ProcessTruthStore> {
    if let Some(state_root) = take_state_root(args) {
        ProcessTruthStore::new_with_state_root(workspace, state_root, job_id)
    } else {
        ProcessTruthStore::new(workspace, job_id)
    }
}

fn create_agent_job_from_args(
    args: &mut Vec<String>,
    workspace: &str,
    user_goal: &str,
) -> io::Result<(
    supernova_process_kernel::AgentJob,
    supernova_process_kernel::AgentProcess,
    ProcessTruthStore,
)> {
    if let Some(state_root) = take_state_root(args) {
        create_agent_job_with_state_root(workspace, state_root, user_goal)
    } else {
        create_agent_job(workspace, user_goal)
    }
}

fn take_state_root(args: &mut Vec<String>) -> Option<PathBuf> {
    take_optional_value(args, "--state-root")
        .map(PathBuf::from)
        .or_else(|| env::var_os("SUPERNOVA_RUNTIME_STATE_ROOT").map(PathBuf::from))
}

fn take_value(args: &mut Vec<String>, flag: &str) -> io::Result<String> {
    take_optional_value(args, flag).ok_or_else(|| invalid(format!("missing {flag}")))
}

fn take_optional_value(args: &mut Vec<String>, flag: &str) -> Option<String> {
    let index = args.iter().position(|item| item == flag)?;
    args.remove(index);
    if index >= args.len() {
        return None;
    }
    Some(args.remove(index))
}

fn take_flag(args: &mut Vec<String>, flag: &str) -> bool {
    if let Some(index) = args.iter().position(|item| item == flag) {
        args.remove(index);
        true
    } else {
        false
    }
}

fn take_string_list(args: &mut Vec<String>, flag: &str) -> io::Result<Vec<String>> {
    let raw = take_value(args, flag)?;
    Ok(raw
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_string)
        .collect())
}

fn take_optional_usize(args: &mut Vec<String>, flag: &str) -> io::Result<Option<usize>> {
    let Some(value) = take_optional_value(args, flag) else {
        return Ok(None);
    };
    value
        .parse::<usize>()
        .map(Some)
        .map_err(|_| invalid(format!("{flag} must be a non-negative integer")))
}

fn take_optional_model_config(args: &mut Vec<String>) -> io::Result<ModelInvocationConfig> {
    let Some(raw) = take_optional_value(args, "--model-config-json") else {
        return Ok(ModelInvocationConfig::from_env());
    };
    serde_json::from_str::<ModelInvocationConfig>(&raw)
        .map_err(|err| invalid(format!("invalid --model-config-json: {err}")))
}

fn take_optional_model_config_arg(
    args: &mut Vec<String>,
) -> io::Result<Option<ModelInvocationConfig>> {
    let Some(raw) = take_optional_value(args, "--model-config-json") else {
        return Ok(None);
    };
    serde_json::from_str::<ModelInvocationConfig>(&raw)
        .map(Some)
        .map_err(|err| invalid(format!("invalid --model-config-json: {err}")))
}

fn take_optional_context_policy(
    args: &mut Vec<String>,
) -> io::Result<Option<ContextWindowControlConfig>> {
    let Some(raw) = take_optional_value(args, "--context-policy-json") else {
        return Ok(None);
    };
    serde_json::from_str::<ContextWindowControlConfig>(&raw)
        .map(Some)
        .map_err(|err| invalid(format!("invalid --context-policy-json: {err}")))
}

fn take_optional_context_pack_arg(args: &mut Vec<String>) -> io::Result<Option<ContextPack>> {
    let Some(raw) = take_optional_value(args, "--context-pack-json") else {
        return Ok(None);
    };
    serde_json::from_str::<ContextPack>(&raw)
        .map(Some)
        .map_err(|err| invalid(format!("invalid --context-pack-json: {err}")))
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

fn json_err(err: serde_json::Error) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, err)
}

fn now_ms_local() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_else(|_| std::time::Duration::from_millis(0))
        .as_millis()
}
