# Module Contract: Process Kernel

[中文](../zh-CN/module-contracts/process-kernel.md) | English

## Responsibility

`process_kernel/` owns execution truth. It runs Chat and TASK workflows, maps provider intent into registered capabilities, records receipts, and exposes Kernel-facing APIs through `KernelApi`.

## Key Files

| Area | Files |
| --- | --- |
| API boundary | `process_kernel/src/kernel_api.rs` |
| TASK loop | `process_kernel/src/root_process.rs`, `process_kernel/src/task_agent.rs`, `process_kernel/src/task_agent_runtime.rs` |
| Chat loop | `process_kernel/src/chat_runtime.rs`, `process_kernel/src/chat_truth.rs` |
| Provider path | `process_kernel/src/model_runtime.rs`, `process_kernel/src/deepseek_provider.rs`, `process_kernel/src/provider_tool*.rs` |
| Capabilities | `process_kernel/src/capability_kernel.rs`, `os_runtime.rs`, `terminal_runtime.rs`, `office_runtime.rs`, `artifact_runtime.rs` |
| State and replay | `process_kernel/src/lib.rs`, `process_kernel/src/observation.rs`, `process_kernel/src/closure_gate.rs` |

## Inputs And Outputs

| Direction | Contract |
| --- | --- |
| Input | Chat turns, TASK goals, resume/approval/artifact requests, model/provider configuration handles, workspace/container context. |
| Output | `ChatTruth`, `ProcessTruth`, capability receipts, task status, stream events, artifact metadata, product-facing timeline snapshots. |

## Invariants

- Model output is not execution truth by itself.
- TASK mutation must be represented by registered capability execution and receipts.
- Chat remains read-only; mutation intent is routed toward TASK semantics.
- Product Runtime projections never replace Kernel truth.
- Public documentation must not expose internal security-control details.

## Verification Entry Points

- `cargo check --workspace`
- Kernel unit tests under `process_kernel/src/*`
- Historical evidence under `reports/rc0_batch7_validation/*` can support history, not current-state claims.

