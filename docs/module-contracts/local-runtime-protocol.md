# Module Contract: Local Runtime Protocol

[中文](../zh-CN/module-contracts/local-runtime-protocol.md) | English

`crates/local_runtime_protocol/` defines the typed contract consumed by Workbench and produced by Product Runtime. It is a schema/DTO boundary, not an execution layer.

## Key Files

| Domain | Files |
| --- | --- |
| Envelope/errors | `envelope.rs`, `error.rs` |
| Runtime/stream | `runtime.rs`, `stream.rs`, `run.rs` |
| Product objects | `workspace.rs`, `container.rs`, `artifact.rs`, `task.rs`, `chat.rs` |
| Configuration | `settings.rs`, `model_config.rs`, `context_pack.rs`, `guidance.rs` |

## Invariants

- DTOs describe what the UI may consume.
- DTOs must not smuggle Kernel internals into the UI.
- Generated frontend clients are consumers of this contract.
- Protocol state remains a representation of product/runtime state, not Kernel truth itself.

