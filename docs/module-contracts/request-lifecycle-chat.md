# Chat Request Lifecycle

[中文](../zh-CN/module-contracts/request-lifecycle-chat.md) | English

```mermaid
sequenceDiagram
  participant UI as Workbench
  participant PR as Product Runtime
  participant K as KernelApi
  participant C as ChatRuntime
  participant T as ChatTruth
  UI->>PR: submit chat turn
  PR->>K: chat request
  K->>C: run ChatRuntime
  C->>T: record chat facts
  C-->>PR: stream/projection events
  PR-->>UI: SSE + message feed
```

Chat is non-mutating. If a request needs workspace mutation, terminal execution, or artifact delivery, it belongs to TASK semantics.

