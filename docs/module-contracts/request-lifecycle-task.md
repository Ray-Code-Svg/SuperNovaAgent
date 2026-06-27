# TASK Request Lifecycle

[中文](../zh-CN/module-contracts/request-lifecycle-task.md) | English

```mermaid
sequenceDiagram
  participant UI as Workbench
  participant PR as Product Runtime
  participant K as KernelApi
  participant R as RootProcess
  participant A as TaskAgent
  participant T as ProcessTruth
  UI->>PR: submit TASK
  PR->>K: start/resume job
  K->>R: create AgentJob
  R->>A: observe / decide / act / verify
  A->>T: record receipts and events
  T-->>PR: task facts for projection
  PR-->>UI: run state + message feed
```

TASK is the mutation-capable execution path. Completion claims require Kernel facts and receipts, not UI projection alone.

