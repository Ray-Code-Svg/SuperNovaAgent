# Module Contract: Workbench v2

[English](../../module-contracts/workbench-v2.md) | 中文

## 职责

`desktop_shell/ui/src/workbench_v2/` 是 Tauri/React 桌面产品界面。它消费 Local Runtime Protocol，展示 workspace、container、Chat、TASK、approval、artifact、settings、runtime status 等产品投影。

Workbench v2 的核心边界是：它负责让用户看见和操作产品状态，但不自行制造执行事实。

## 关键文件

| 领域 | 文件 |
| --- | --- |
| App shell | `main/WorkbenchV2.tsx`, `main/FluentAppShell.tsx`, `main/RuntimeStatusBar.tsx`, `main/StartupScreen.tsx` |
| Protocol client | `protocol/runtimeClient.ts`, `protocol/*Queries.ts` |
| Chat surface | `chat/AgentChatSurface.tsx`, `chat/ChatMessageStream.tsx`, `main/streamMessages.ts` |
| TASK surface | `task/AgentTaskSurface.tsx`, `task/TaskMessageStream.tsx`, `task/ArtifactReadyCard.tsx`, `layout/TaskStatusRail.tsx` |
| Composer/flyouts | `composer/*`, `flyouts/*` |
| UI-local state | `state/uiStore.ts`, `state/containerUiStore.ts`, `state/workspaceUiStore.ts`, `state/windowScope.ts` |
| Settings/i18n/onboarding | `settings/*`, `i18n/i18n.ts`, `onboarding/*` |

## 输入

- Local Runtime Protocol generated client 返回的数据。
- SSE stream message。
- 用户输入：draft、mode、workspace/container selection、settings 操作。
- Tauri shell 提供的启动状态和本地窗口状态。

## 输出

- Chat/TASK message stream UI。
- task rail / run status。
- artifact card。
- settings and onboarding UI。
- protocol mutation/query request。

## Source Of Truth

| 状态 | Workbench 是否为 truth | 说明 |
| --- | --- | --- |
| draft、selected container、open flyout | 是，限 UI-local state | 不影响 Kernel truth。 |
| Chat answer | 否 | 来自 ChatTruth/Product Runtime projection。 |
| TASK status | 否 | 来自 Product Runtime projection，并应可追溯到 Kernel truth。 |
| artifact readiness | 否 | UI card 是投影，receipt/evidence 在 Kernel/runtime 层。 |
| provider setup display | 否 | UI 只展示配置状态，不公开敏感内容。 |

## 不变量

- UI 不直接读写 Kernel truth 文件。
- UI 不根据 streaming delta 自行判定任务完成。
- UI 不把 historical report 当成当前运行状态。
- UI state 只管理显示、选择、草稿、偏好，不承担 execution closure。
- 所有 runtime 操作必须通过 protocol client。

## 测试入口

```powershell
npm.cmd --prefix desktop_shell/ui run typecheck
npm.cmd --prefix desktop_shell/ui run test
```

需要验证真实桌面体验时，还要做 browser validation 或 installed-app replay；组件测试只能证明局部渲染和状态逻辑。
