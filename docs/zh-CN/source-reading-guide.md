# 源码阅读指南

[English](../source-reading-guide.md) | 中文

本文是 SuperNova 当前 RC0 代码的反向学习地图。主体使用中文，源码路径、模块名、类型名、函数名和协议术语保留英文。本文只解释现状，不引入新架构设计，也不展开公开材料不应暴露的安全基线实现细节。

## 快速理解

| 问题 | 结论 |
| --- | --- |
| 当前主链路 | `desktop_shell/ui/src/workbench_v2 -> crates/product_runtime -> crates/local_runtime_protocol -> process_kernel` |
| TASK truth | `ProcessTruth`、capability receipt、Kernel replay。 |
| Chat truth | `ChatTruth`、provider transcript、read-only tool receipt。 |
| Product projection | Product Runtime 的 message feed、run registry、runtime event log、projection shards。 |
| UI 角色 | Workbench v2 是协议消费者和产品投影渲染层，不是执行事实源。 |
| 阅读原则 | 先读协议，再读 Product Runtime，再读 Kernel，最后读 UI 投影。 |

## 读代码顺序

1. 先读 `crates/local_runtime_protocol/src/*`。
   这里定义 Workbench 和 Product Runtime 之间的 DTO、envelope、stream event、task/chat/container/workspace 类型。它能告诉你 UI 能看到什么，也能告诉你 UI 不应该直接知道什么。

2. 再读 `crates/product_runtime/src/http/routes/*`。
   route 层是 HTTP/SSE 入口。重点看 `chat.rs`、`tasks.rs`、`containers.rs`、`runs.rs`、`settings.rs`、`model_config.rs`。这里能看到每类产品操作进入哪个 service。

3. 接着读 `crates/product_runtime/src/services/*`。
   service 层承担 product-facing orchestration：创建 workspace/container、提交 Chat/TASK、维护 run、写入 projection。它不能把 Product DB 当成 Kernel truth。

4. 再读 `crates/product_runtime/src/kernel/*` 和 `crates/product_runtime/src/kernel_worker.rs`。
   这里是 Product Runtime 与 Process Kernel 的桥。重点看 Kernel 调用如何被隔离在 worker/bridge 边界内。

5. 然后读 `process_kernel/src/kernel_api.rs`。
   `KernelApi` 是外部进入 Kernel 的主要 API 面。读它可以看到 Chat、TASK、resume、approval、artifact、timeline sync 等能力如何暴露给 Product Runtime。

6. 继续读 `process_kernel/src/chat_runtime.rs`、`process_kernel/src/root_process.rs`、`process_kernel/src/task_agent.rs`、`process_kernel/src/task_agent_runtime.rs`。
   这里是 Chat 和 TASK 的核心执行循环。重点区分 Chat 的 read-only contract 与 TASK 的 mutation-capable contract。

7. 再读 truth 相关代码。
   `process_kernel/src/chat_truth.rs` 和 `process_kernel/src/lib.rs` 中导出的 `ProcessTruthStore` 是理解事实层的入口。阅读时要把 event、receipt、replay state 与 Product Runtime projection 分开。

8. 最后读 Workbench v2。
   `desktop_shell/ui/src/workbench_v2/protocol/*` 是前端协议客户端；`main/*`、`chat/*`、`task/*`、`layout/*` 是产品界面与状态渲染；`state/*` 是 UI-local state，不是 runtime truth。

9. `office_worker/*` 放在最后读。
   Office Worker 是 Office 文件处理 subprocess。它不调用模型，不做任务规划，只对 Kernel Office Runtime 发来的文档操作请求返回结构化结果。

## 模块契约入口

- [Process Kernel](module-contracts/process-kernel.md)
- [Product Runtime](module-contracts/product-runtime.md)
- [Local Runtime Protocol](module-contracts/local-runtime-protocol.md)
- [Workbench v2](module-contracts/workbench-v2.md)
- [Office Worker](module-contracts/office-worker.md)
- [State and Truth Boundaries](module-contracts/state-and-truth-boundaries.md)
- [Chat Request Lifecycle](module-contracts/request-lifecycle-chat.md)
- [TASK Request Lifecycle](module-contracts/request-lifecycle-task.md)

## 不要误解的点

| 表面现象 | 错误理解 | 正确理解 |
| --- | --- | --- |
| UI 出现 assistant message | UI 已证明 Kernel 完成了任务 | UI message 是 projection；TASK 是否完成要看 Kernel truth 与 run reconciliation。 |
| Product DB 有 task/run 状态 | Product DB 是执行事实源 | Product DB 是产品读模型和监控层；执行事实仍在 Kernel truth。 |
| provider 返回 tool call | 工具已经执行 | provider tool call 只是 model intent；执行必须经过 Kernel capability contract 并产生 receipt。 |
| streaming delta 正常显示 | 任务一定成功 | streaming 是显示/传输层事实，不等价于 artifact、workspace mutation 或 task closure。 |
| historical report 通过 | 当前 worktree 已通过 | historical report 是历史证据；当前 claim 必须重新跑对应验证。 |

## 练习任务

| 练习 | 起点 | 目标 |
| --- | --- | --- |
| 追踪一次 Chat answer delta | `desktop_shell/ui/src/workbench_v2/main/streamMessages.ts` | 解释 Chat stream event 如何变成 UI message。 |
| 追踪一次 TASK submit | `desktop_shell/ui/src/workbench_v2/protocol/taskQueries.ts` | 解释 TASK 如何进入 Product Runtime route 和 Kernel worker。 |
| 追踪一次 artifact ready card | `desktop_shell/ui/src/workbench_v2/task/ArtifactReadyCard.tsx` | 区分 artifact projection 与 Kernel artifact receipt。 |
| 追踪一次 run status | `crates/product_runtime/src/state/run_registry.rs` | 解释 run state 与 Kernel task status 的关系。 |
| 追踪一次 Office document operation | `process_kernel/src/office_runtime.rs` | 解释 Kernel Office Runtime 与 `office_worker` 的边界。 |
