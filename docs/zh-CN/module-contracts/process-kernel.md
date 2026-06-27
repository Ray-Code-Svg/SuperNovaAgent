# Module Contract: Process Kernel

[English](../../module-contracts/process-kernel.md) | 中文

## 职责

`process_kernel/` 是执行事实层。它负责 Chat 与 TASK 的 runtime loop，把 provider intent 映射为 registered capability，执行 policy-controlled capability，记录 receipts，并通过 `KernelApi` 向 Product Runtime 暴露受控能力。

## 关键文件

| 领域 | 文件 |
| --- | --- |
| API boundary | `process_kernel/src/kernel_api.rs` |
| TASK loop | `process_kernel/src/root_process.rs`, `process_kernel/src/task_agent.rs`, `process_kernel/src/task_agent_runtime.rs` |
| Chat loop | `process_kernel/src/chat_runtime.rs`, `process_kernel/src/chat_truth.rs` |
| Model/provider | `process_kernel/src/model_runtime.rs`, `process_kernel/src/deepseek_provider.rs`, `process_kernel/src/provider_tool*.rs` |
| Capability runtime | `process_kernel/src/capability_kernel.rs`, `os_runtime.rs`, `terminal_runtime.rs`, `office_runtime.rs`, `artifact_runtime.rs` |
| Observation/replay | `process_kernel/src/observation.rs`, `process_kernel/src/closure_gate.rs`, `process_kernel/src/lib.rs` |

## 输入

- Chat turn request。
- TASK goal、resume request、approval decision、artifact request。
- workspace/container context。
- provider configuration handle。
- Product Runtime 传入的 stream sink 与 projection sync 请求。

## 输出

- `ChatTruth` events。
- `ProcessTruth` events。
- capability receipts。
- model transcript evidence。
- task status / closure evidence。
- artifact metadata。
- Product Runtime 可消费的 timeline/projection snapshot。

## Source Of Truth

| 事实类型 | 事实源 |
| --- | --- |
| TASK 是否执行 | `ProcessTruth`、capability receipt、closure/replay state。 |
| Chat 是否回答 | `ChatTruth`、provider transcript、ChatRuntime control decision。 |
| 工具是否真正执行 | capability receipt，而不是 provider tool call 文本。 |
| artifact 是否可信 | artifact receipt、typed verification、source set evidence。 |

## 不变量

- model output 不等于 execution truth。
- provider-native tool call 只是 intent，不能跳过 Kernel capability contract。
- ChatRuntime 保持 read-only boundary；需要 mutation 的请求应进入 TASK。
- TASK closure 必须基于 runtime fact 与 evidence，而不是 UI 显示状态。
- Product Runtime projection 不能反向改写 Kernel truth。
- 公开文档只说明职责和边界，不展开内部安全控制实现。

## 禁止越界

- 不让 UI 直接读写 Kernel truth 文件。
- 不把 Product DB status 当成 TASK 完成证据。
- 不用 mock/fallback 冒充 provider/API/capability 成功。
- 不在公开材料中列出内部安全检查清单、payload、header 或绕过路径。

## 验证入口

```powershell
cargo check --workspace
```

更细的验证应按变更范围选择 Kernel unit tests、Product Runtime integration tests、real API smoke、installed-app replay。历史 `reports/*` 只能证明历史运行，不证明当前 worktree 刚刚通过。

