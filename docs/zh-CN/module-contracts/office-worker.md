# Module Contract: Office Worker

[English](../../module-contracts/office-worker.md) | 中文

## 职责

`office_worker/` 是由 Kernel `OfficeRuntime` 调用的 .NET subprocess，用于 Office 文档读写、转换、验证等底层操作。它不是模型 runtime，不做 rewrite 规划，不自行理解用户目标。

## 关键文件

| 领域 | 文件 |
| --- | --- |
| Worker entry | `office_worker/SuperNova.OfficeWorker/Program.cs` |
| Capability metadata | `office_worker/SuperNova.OfficeWorker/capability_manifest.json` |
| Project | `office_worker/SuperNova.OfficeWorker/SuperNova.OfficeWorker.csproj` |
| Kernel caller | `process_kernel/src/office_runtime.rs` |

## 输入

- Kernel Office Runtime 发来的结构化文档操作请求。
- 文件路径、目标路径、操作参数、已由模型或上游 runtime 准备好的内容。

## 输出

- 结构化 JSON result。
- 文档文件的 deterministic operation result。
- 可被 Kernel 记录的 evidence。

## Source Of Truth

Office Worker 的 stdout/result 不是单独的产品事实源。它必须回到 Kernel `OfficeRuntime`，由 Kernel 记录 receipt 与 source evidence 后，才进入 TASK truth 和 Product Runtime projection。

## 不变量

- Office Worker 不调用 provider/model。
- Office Worker 不接受自然语言 rewrite instruction 后自行改写。
- Office Worker 不决定 TASK 是否完成。
- Office Worker 不绕过 workspace/document operation boundary。
- 文档产物必须能被 Kernel 侧 evidence 和 artifact flow 追踪。

