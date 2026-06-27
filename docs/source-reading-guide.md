# Source Reading Guide

[中文](zh-CN/source-reading-guide.md) | English

This guide maps the current SuperNova RC0 codebase for developers who want to understand the runtime from source. It describes the existing implementation only; it does not propose new architecture.

## At A Glance

| Question | Answer |
| --- | --- |
| Primary path | `desktop_shell/ui/src/workbench_v2 -> crates/product_runtime -> crates/local_runtime_protocol -> process_kernel` |
| Execution truth | `ProcessTruth` for TASK, `ChatTruth` for Chat. |
| Product projection | Product Runtime stores UI read models such as message feed, run registry, runtime event log, and projection shards. |
| Read first | Protocol DTOs, then Product Runtime services/routes, then Kernel APIs and truth stores, then Workbench rendering. |
| Security detail boundary | Public docs describe boundaries and responsibilities, not internal security controls or bypass-relevant implementation details. |

## Recommended Reading Order

1. Read `crates/local_runtime_protocol/src/*` to understand DTOs, envelopes, stream messages, and domain types.
2. Read `crates/product_runtime/src/http/routes/*` and `crates/product_runtime/src/services/*` to see how product requests are routed.
3. Read `crates/product_runtime/src/kernel/*` and `crates/product_runtime/src/kernel_worker.rs` to locate the bridge to the Kernel.
4. Read `process_kernel/src/kernel_api.rs`, `process_kernel/src/chat_runtime.rs`, `process_kernel/src/root_process.rs`, and `process_kernel/src/task_agent.rs`.
5. Read `process_kernel/src/chat_truth.rs` and the `ProcessTruthStore` exports in `process_kernel/src/lib.rs` to understand durable facts.
6. Read `crates/product_runtime/src/state/*` to understand UI projection and local product state.
7. Read `desktop_shell/ui/src/workbench_v2/*` to see how the Workbench consumes the protocol and renders Chat/TASK state.
8. Read `office_worker/*` only after Kernel `OfficeRuntime`; the worker is a document-processing subprocess, not a model runtime.

## Core Module Contracts

- [Process Kernel](module-contracts/process-kernel.md)
- [Product Runtime](module-contracts/product-runtime.md)
- [Local Runtime Protocol](module-contracts/local-runtime-protocol.md)
- [Workbench v2](module-contracts/workbench-v2.md)
- [Office Worker](module-contracts/office-worker.md)
- [State and Truth Boundaries](module-contracts/state-and-truth-boundaries.md)
- [Chat Request Lifecycle](module-contracts/request-lifecycle-chat.md)
- [TASK Request Lifecycle](module-contracts/request-lifecycle-task.md)

## What Not To Misread

| Surface | Common misread | Correct reading |
| --- | --- | --- |
| Workbench message stream | UI messages prove execution happened. | UI messages are projections of runtime state. |
| Product Runtime database | Product DB is the Kernel truth store. | Product DB is a product read/projection layer. |
| Provider tool call | A model tool call already mutated the workspace. | Provider tool calls are intent until Kernel policy and capability execution record receipts. |
| Historical reports | A prior report proves the current build. | Prior reports are historical evidence; current claims require fresh verification. |

