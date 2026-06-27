# Container-first Desktop UI Architecture

Last updated: 2026-06-01

This document records the desktop UI architecture for the Container-first UX line. The UI stays above Local Runtime Protocol v1. It consumes product projections and never calls `process_kernel`, `ProcessAction`, capability tokens, kernel CLI commands, or `.supernova_v2` files directly.

## Product Rule

```text
Workspace aggregates attention.
Container carries concurrent context.
AgentChat and Agent TASK are two modes inside the active Container.
Workspace Task History is navigation and search only. It is not a New Task entry.
```

## Screen Layout

- Top bar: workspace identity, runtime attachment, diagnostics, global task/history entry.
- Left rail: Container list, create Container, pin/archive/search, and badges for `running`, `approval`, `blocked`, `unread`, and `artifact ready`.
- Center: active Container surface with mode switch `AgentChat | Agent TASK`.
- Right inspector: active task state, preview/approval, artifacts, Truth/receipts/audit, client-env disclosure, config.
- Workspace task history: history/search/filter/cross-container navigation only. It must not contain `New Task`.

## State Layers

Workspace product state comes from Protocol v1 and runtime projections:

- `workspace_id`
- runtime identity and diagnostics
- Container list
- workspace-level task history
- protocol/event cursor state

Container product state also comes from Protocol v1:

- active chat thread and ChatTruth cursor
- context pack
- Container task queue
- approvals and client-env disclosure projections
- timeline
- stream summary and background badge counters

Client UI state is local-only and may be cached by `workspace_id + container_id`:

- active Container id
- active Container mode
- chat draft
- task launcher draft
- expanded panels
- inspector tab
- selected task in the active Container
- recent active Container per workspace

Client UI state is never a fact source. A refresh must rebuild product state from Protocol snapshots and then replay event increments by cursor.

## Switching Semantics

Container switch:

- Save local drafts and panel focus for the current Container.
- Stop foreground stream rendering for the old Container view.
- Do not cancel background ChatRuntime or TaskRuntime execution.
- Load the target Container snapshot: chat threads, context pack, task queue, approvals, disclosures, and timeline.
- Restore only that Container's cached UI state.
- Never leak selected task, chat draft, launcher draft, context pack, or stream buffer across Containers.

Workspace switch:

- Save current workspace UI state.
- Attach the new workspace runtime through the shell/protocol boundary.
- Clear runtime-bound projections from the previous workspace.
- Load runtime meta and Containers for the new workspace.
- Restore that workspace's last active Container when it still exists; otherwise show Container selection/create state.
- Do not reuse task/run/job/tx/context ids across workspaces.

Refresh/reload:

- Load `GET /api/v1/runtime/meta`.
- Load `GET /api/v1/containers`.
- Restore active Container id from client UI state when valid.
- Load the active Container snapshot.
- Load workspace task history.
- Resume event ingestion from the last durable cursor when available.

## Component Direction

Target workbench structure:

```tsx
<WorkbenchShell>
  <Topbar />
  <ContainerRail />
  <ActiveContainerWorkspace>
    <ContainerContextStrip />
    <ContainerModeSwitch />
    {mode === "agent_chat" ? <AgentChatMode /> : <AgentTaskMode />}
  </ActiveContainerWorkspace>
  <ActiveTaskInspector />
  <WorkspaceTaskHistory />
  <BackgroundContainerToast />
</WorkbenchShell>
```

Recommended component ownership:

- `ContainerRail.tsx`: Container list, create action, badges, pin/archive/search.
- `ActiveContainerWorkspace.tsx`: active Container snapshot loading and mode layout.
- `AgentChatMode.tsx`: chat thread, Markdown/code/stable LaTeX rendering, `needs_task` card.
- `AgentTaskMode.tsx`: task launcher, Container task queue, active task phase stream.
- `TaskLaunchConfig.tsx`: Basic fields first; model/thinking/token/strict-tools settings collapsed.
- `AgentStreamPanel.tsx`: active Container only, phase-oriented task stream.
- `WorkspaceTaskHistory.tsx`: read-only history/search/filter/navigation.
- `BackgroundContainerToast.tsx`: non-interruptive background state notifications.

Existing inspector tabs may remain, but they must be keyed by active Container/task identity and clear on workspace switch.

## Protocol Contract

The desktop UI should prefer these Protocol v1 resources:

- `GET /api/v1/runtime/meta`
- `GET|POST /api/v1/containers`
- `GET /api/v1/containers/{container_id}/timeline`
- `GET /api/v1/containers/{container_id}/tasks`
- `POST /api/v1/containers/{container_id}/tasks`
- `GET|POST /api/v1/containers/{container_id}/context-pack`
- `GET /api/v1/approvals?container_id=...`
- `GET /api/v1/client-env/disclosures`
- `GET /api/v1/tasks`
- `GET /api/v1/tasks/{task_id}`

`GET /api/v1/containers/{container_id}/tasks` is a Container-scoped task portfolio. The frontend must not fetch workspace tasks and filter them as a fallback.

## Stream UX

Active Container:

- AgentChat renders message stream with Markdown, code blocks, and stable KaTeX blocks.
- Agent TASK renders phases: planning, tool/capability, preview, approval, artifact, verify, complete.
- Audit details stay collapsed in Truth/Receipt panels by default.

Background Containers:

- Only update badges, summaries, and non-interruptive toasts.
- Never insert background stream logs into the active Container view.
- A user click on the badge or toast switches focus to that Container.

## Diagnostics

Diagnostics should expose:

- runtime transport and API base URL
- protocol version and request id
- workspace id and active Container id
- event cursor and stream state
- selected task identity
- task/history counts
- last structured error

These diagnostics belong in the inspector/config surface. They should not compete with the primary AgentChat or Agent TASK workflow.
