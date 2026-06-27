# Retired Live Adapter Contract

Last updated: 2026-06-07

The old `SuperNovaClient` / `runGoalStream` live adapter contract is retired with the Workbench v1 frontend and Python Web/CLI compatibility path.

Current desktop UI work must use:

```text
desktop_shell/ui/src/protocol/generated/
desktop_shell/ui/src/workbench_v2/protocol/runtimeClient.ts
crates/local_runtime_protocol/
crates/product_runtime/
```

The active streaming contract is Local Runtime Protocol v1 SSE with `ProtocolEvent` envelopes and `cursor.after_event_id` reconnect. New code must not add calls to the deleted `desktop_shell/ui/src/api` facade or `/api/run/stream`.
