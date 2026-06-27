# Module Contract: Product Runtime

[中文](../zh-CN/module-contracts/product-runtime.md) | English

`crates/product_runtime/` is the local product service between Workbench and Process Kernel. It owns HTTP/SSE routes, product database projections, run supervision, and the Kernel bridge. It does not replace Kernel execution truth.

## Key Files

| Area | Files |
| --- | --- |
| Startup | `main.rs`, `bootstrap.rs`, `app_state.rs` |
| HTTP/SSE | `http/router.rs`, `http/server.rs`, `http/routes/*`, `http/sse.rs` |
| Services | `services/chat_service.rs`, `task_service.rs`, `run_manager.rs`, `workspace_service.rs`, `container_service.rs` |
| Kernel bridge | `kernel/kernel_bridge.rs`, `kernel/event_projection.rs`, `kernel_worker.rs` |
| Product state | `state/product_db.rs`, `message_feed.rs`, `run_registry.rs`, `runtime_event_log.rs`, `projection_shards.rs` |

## Invariants

- Product Runtime exposes product read models and routes.
- Kernel truth remains authoritative for execution facts.
- Projections must be reconciled from Kernel/runtime events, not guessed by UI convenience.
- Public docs should describe local runtime responsibilities without exposing internal access-control details.

