# SuperNova Local Runtime Protocol v1

Last updated: 2026-06-07

This document records the desktop UI integration contract for the product-level local runtime protocol. It is intentionally above `process_kernel`: the UI consumes product objects and never calls Kernel capabilities, `ProcessAction`, capability tokens, or workspace files directly.

## Runtime Boundary

Live HTTP mode uses the versioned API root:

```text
/api/v1/...
```

Workbench v2 client code must use the generated protocol client and the v2 runtime adapter under:

```text
desktop_shell/ui/src/protocol/generated/
desktop_shell/ui/src/workbench_v2/protocol/
```

The retired Workbench v1 protocol facade under `desktop_shell/ui/src/api/`, `supernovaClient.ts`, and `native/shellClient.ts` is no longer part of the product path.

## Product Objects

The protocol exposes product-level resources:

- Runtime: `GET /api/v1/runtime/meta`, `GET /api/v1/runtime/events`
- Workspaces: activate/list recent workspaces through Product Runtime/Tauri shell adapters
- Containers: `GET|POST /api/v1/containers`
- Container timeline: `GET /api/v1/containers/{container_id}/timeline`
- Container tasks: `GET|POST /api/v1/containers/{container_id}/tasks`
- Chat turns: `POST /api/v1/chat/threads/{chat_thread_id}/turns`, `POST /api/v1/chat/threads/{chat_thread_id}/turns/stream`
- Tasks: `GET /api/v1/tasks`, `GET /api/v1/tasks/{task_id}`, `GET /api/v1/tasks/{task_id}/events/stream`
- Task controls: approve, reject, edit, rollback, artifacts, ProcessTruth export
- Approvals: `GET /api/v1/approvals`
- Client Env disclosures: list, approve, reject
- Settings: provider API and model config through redacted DTOs

Task identity is client-facing `task_id`; `job_id` remains available for audit/detail views only.

## Container Task Projection

`GET /api/v1/containers/{container_id}/tasks` returns a Container-scoped task portfolio:

```json
{
  "workspace_id": "C:\\workspace",
  "container_id": "container_...",
  "projection": "container_task_portfolio",
  "projection_source": "run_checkpoint_container_index + process_truth_replay",
  "count": 1,
  "items": [
    {
      "task_id": "run_...",
      "run_id": "run_...",
      "job_id": "job_...",
      "container_id": "container_...",
      "status": "waiting_approval",
      "available_actions": {
        "approve": true,
        "reject": true,
        "edit": true
      }
    }
  ]
}
```

The desktop UI must use this route for the active Container task queue. It must not fetch `GET /api/v1/tasks` and filter workspace task history as a fallback.

## Envelope

Non-streaming responses are wrapped:

```json
{
  "protocol_version": "supernova.local_runtime.v1",
  "schema_version": "supernova.protocol.response.v1",
  "request_id": "req_...",
  "workspace_id": "C:\\workspace",
  "resource": "task",
  "data": {}
}
```

Errors are wrapped with `schema_version=supernova.protocol.error.v1` and a structured `error` object.

## SSE

Protocol SSE payloads use `schema_version=supernova.protocol.event.v1` and include `event_id`, `cursor`, `workspace_id`, `event_type`, and resource ids. Workbench v2 consumes this shape through:

```text
desktop_shell/ui/src/protocol/generated/client.ts
desktop_shell/ui/src/workbench_v2/protocol/runtimeClient.ts
```

A task stream event should carry task/job/container ids, a cursor, and a payload such as:

```json
{
  "event_type": "task.message",
  "task_id": "run_...",
  "job_id": "job_...",
  "container_id": "container_...",
  "cursor": {
    "kind": "task_message_stream",
    "after_event_id": 42,
    "source": "message_feed"
  },
  "payload": {
    "status": "active",
    "message": "workspace.inspect: success",
    "source_event_id": 42,
    "source_event_type": "capability_receipt"
  }
}
```

Foreground task streams are scoped to the active Container/task. Background Containers consume runtime event summaries for badges and must not insert their logs into the current focus surface. Reconnect must use `cursor.after_event_id` rather than rebuilding state from kernel files.

## Shell Boundary

The active Tauri boundary is `desktop_shell/src-tauri`. It may expose lifecycle, window, AppData, workspace activation, and runtime bootstrap operations, but product resources belong in Local Runtime Protocol HTTP/SSE routes. Workbench v2 must not call Kernel capabilities, read `.supernova_v2`, or use a second local settings source for provider/model/task facts.
