# Module Contract: Workbench v2

[中文](../zh-CN/module-contracts/workbench-v2.md) | English

`desktop_shell/ui/src/workbench_v2/` is the React/Tauri product surface. It consumes the Local Runtime Protocol and renders workspace, container, Chat, TASK, approval, artifact, and settings projections.

## Key Files

| Area | Files |
| --- | --- |
| App shell | `main/WorkbenchV2.tsx`, `main/FluentAppShell.tsx`, `main/RuntimeStatusBar.tsx` |
| Protocol client | `protocol/runtimeClient.ts`, `protocol/*Queries.ts` |
| Chat | `chat/*`, `main/streamMessages.ts` |
| TASK | `task/*`, `layout/TaskStatusRail.tsx` |
| UI state | `state/*` |
| Settings/onboarding | `settings/*`, `onboarding/*`, `i18n/i18n.ts` |

## Invariants

- Workbench is not the execution truth source.
- UI-local state should remain scoped to display, selection, drafts, and preferences.
- Runtime state must come through Product Runtime and protocol DTOs.
- Chat and TASK projections must remain distinguishable.

