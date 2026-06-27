# Container-first Desktop UI Browser Regression Checklist

Last updated: 2026-06-07

This checklist is the Browser Plugin validation baseline for the Container-first desktop UX. It is intended for `desktop_shell/ui` local development and should be run through the Codex in-app Browser after meaningful UI changes.

## Start Target

Run the frontend dev server:

```powershell
cd desktop_shell/ui
npm.cmd run dev
```

Open with the in-app Browser:

```text
http://127.0.0.1:5173/
```

Recommended local checks:

```powershell
cd desktop_shell/ui
npm.cmd run typecheck
npm.cmd run build
npm.cmd run test
cargo check --manifest-path ..\src-tauri\Cargo.toml
```

## Browser Plugin Evidence

For each UI development round, capture:

- initial screenshot at `1440x900`
- responsive screenshots at `1366x768`, `1280x720`, and one narrow desktop/mobile-like width
- console error scan
- interaction trace notes for Container switch, Workspace switch, and reload
- local/session storage keys used for UI state

The Browser Plugin should validate visual behavior and state transitions. It must not bypass the desktop API boundary or read `.supernova_v2` files directly.

## Default State

- Top bar shows workspace/runtime/diagnostics.
- Container rail is visible on the left.
- No global Front Agent chat panel exists outside the active Container.
- Active Container shows a mode switch: `AgentChat | Agent TASK`.
- Workspace Task History is history/search/navigation only.
- No `New Task` entry appears in Workspace Task History.
- Active Task Inspector is empty or closed until a task is selected.
- No page-level horizontal scrolling appears.

## Container Rail

- Create a Container.
- The new Container appears in the rail without changing workspace identity.
- Badges render for `running`, `approval`, `blocked`, `unread`, and `artifact ready` states from Product Runtime projections or an explicit seeded runtime state.
- Pin/archive/search controls do not resize rows or hide badges.
- Clicking a background Container switches the active Container and loads its snapshot.

Failure signals:

- Badge text overlaps title or controls.
- Background stream logs appear in the active Container view.
- Container selection changes workspace task history filters unexpectedly.

## Container State Isolation

- Select Container A.
- Enter an AgentChat draft, choose a context pack, select a task, and open an inspector tab.
- Switch to Container B.
- Verify B does not inherit A's chat draft, task launcher draft, selected task, stream buffer, context pack, or inspector tab.
- Switch back to A.
- Verify A's local drafts and UI focus restore from `workspace_id + container_id` UI state.

Failure signals:

- Selected task leaks between Containers.
- Context pack from one Container is used to start another Container task.
- Reload depends on component memory instead of Protocol snapshots.

## AgentChat Mode

- Chat thread renders inside the active Container only.
- Markdown and code blocks render without layout shift.
- Stable LaTeX blocks render only when complete; incomplete streaming formula text remains readable.
- A `needs_task` response shows a task suggestion card.
- Clicking `Start in Agent TASK` switches the same Container to Agent TASK, pre-fills the suggested goal, and carries the current context pack.

Failure signals:

- User must reselect the same context pack after `needs_task`.
- Half-rendered LaTeX causes flicker or broken layout.
- Chat stream from a background Container interrupts the active Container.

## Agent TASK Mode

- New Task launch exists only inside active Container Agent TASK mode.
- Basic launch fields are visible first: goal, task intent, source scope, target artifact/output.
- Advanced config is collapsed by default: model, thinking, token budget, strict tools.
- Task intent supports `read-only`, `artifact write`, and `may mutate workspace`.
- The mutation conflict note appears only for `may mutate workspace`; it is not shown for read-only tasks.
- Starting a task calls the Container task route and updates the Container task queue.
- Active task stream is phase-oriented: planning, tool/capability, preview, approval, artifact, verify, complete.
- Selecting an active task subscribes to `GET /api/v1/tasks/{task_id}/phase-stream`; switching Container aborts that foreground stream.
- Phase messages render through the safe message renderer and do not expose raw HTML.

Failure signals:

- New Task remains in Workspace Task History.
- Advanced model settings dominate the default view.
- Generic conflict warning appears for every task intent.

## Workspace Task History

- Open Workspace Task History.
- Search/filter task history.
- Select a historical task from another Container.
- The UI navigates to that Container and selects the task there.
- No launch form or `New Task` button appears in history.

Failure signals:

- History selection opens a stale task in the wrong Container.
- History panel becomes a second task launcher.

## Background Containers

- Start or simulate a running task in Container B.
- Stay focused on Container A.
- Verify Container B updates only rail badges, summary, and optional non-interruptive toast.
- Click the badge/toast.
- Verify focus moves to Container B and the latest snapshot/stream cursor is loaded.

Failure signals:

- Background logs are inserted into Container A.
- A background Container's `task.phase` stream remains visible after switching away from it.
- Toast steals focus or opens inspector automatically.

## Workspace Switch And Reload

- Switch from Workspace A to Workspace B.
- Verify active Container from A is cleared.
- Verify B restores its last active Container when present.
- Refresh the page.
- Verify product state reloads from Protocol snapshots and then event cursor increments.
- Verify UI drafts restore only for the matching workspace/container key.

Failure signals:

- Task/run/job ids from Workspace A remain visible in Workspace B.
- Refresh loses product state that exists in Protocol snapshots.
- UI uses local storage as a fact source.

## Responsive Checks

Validate at:

- `1440x900`
- `1366x768`
- `1280x720`
- `390x844`

Expected:

- Container rail remains usable.
- Mode switch and primary controls remain reachable.
- Inspector has its own scroll region.
- Long task/container names truncate or wrap without pushing controls out of bounds.
- Buttons do not resize fixed-format toolbars.

## Console And Network

Treat these as regressions:

- React runtime errors.
- Unhandled promise rejections.
- 404/500 calls for product routes used by the visible UI.
- Frontend filtering workspace tasks as a substitute for `GET /api/v1/containers/{container_id}/tasks`.
- Raw HTML rendering in chat/task Markdown.
