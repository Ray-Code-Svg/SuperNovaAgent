# Architecture

[中文](zh-CN/architecture.md) | English

SuperNova is a Windows desktop AI Workbench built around a local runtime boundary. The user works in Workbench v2; Product Runtime handles product APIs, projections, and run supervision; Local Runtime Protocol defines typed DTOs; Process Kernel owns execution truth.

## System Map

```mermaid
flowchart TB
  User["User"]

  subgraph UX["UX chain - Workbench v2"]
    Shell["Tauri shell<br/>window + startup"]
    Workbench["Workbench v2<br/>workspace / container / Chat / TASK"]
    Composer["Command composer<br/>mode + sources + artifact target"]
    Streams["Message streams<br/>Chat / TASK / run state"]
  end

  subgraph Runtime["Control flow - Product Runtime"]
    Client["Generated protocol client"]
    Routes["HTTP routes + SSE"]
    Services["Product services<br/>chat / task / workspace / settings"]
    Runs["Run manager<br/>run registry"]
    Projector["Event projection<br/>message feed + projection shards"]
    DB["Product DB<br/>workspace / container / read models"]
  end

  subgraph Protocol["Data contract"]
    DTO["Local Runtime Protocol<br/>request / response DTOs + stream events"]
  end

  subgraph Kernel["Execution truth - Process Kernel"]
    KernelApi["KernelApi"]
    ChatRuntime["ChatRuntime<br/>ChatTruth"]
    TaskRuntime["RootProcess + TaskAgent<br/>ProcessTruth"]
    Model["ModelRuntime<br/>provider path"]
    Caps["Registered capabilities<br/>Office / OS / terminal / artifact"]
    Truth["Truth stores<br/>receipts + replayable events"]
  end

  subgraph Workers["Local workers"]
    Office["Office Worker<br/>DOCX operations"]
  end

  User -->|intent| Workbench
  Shell --> Workbench
  Workbench --> Composer
  Workbench --> Streams
  Composer -->|query / mutation| Client
  Client --> DTO
  DTO --> Routes
  Routes --> Services
  Services -->|Chat turn| KernelApi
  Services -->|TASK run / resume| Runs
  Runs --> KernelApi
  KernelApi --> ChatRuntime
  KernelApi --> TaskRuntime
  ChatRuntime --> Model
  TaskRuntime --> Model
  Model -->|tool-call intent| TaskRuntime
  TaskRuntime -->|checked action| Caps
  Caps --> Office
  ChatRuntime --> Truth
  TaskRuntime --> Truth
  Caps --> Truth
  Truth -->|runtime events| Projector
  Projector --> DB
  Projector --> Routes
  Routes -->|SSE + read models| Client
  Client --> Workbench
  DB --> Services
```

## Layers

| Layer | Source path | Role |
| --- | --- | --- |
| Workbench v2 | `desktop_shell/ui/src/workbench_v2/` | React/Tauri product surface for workspace, container, Chat, TASK, settings, run state, and artifacts. |
| Tauri shell | `desktop_shell/src-tauri/` | Desktop window, app packaging, static assets, and Windows installer configuration. |
| Product Runtime | `crates/product_runtime/` | Local HTTP/SSE service, product state, run supervision, event projection, and Kernel bridge. |
| Local Runtime Protocol | `crates/local_runtime_protocol/` | Typed request/response and stream-event boundary consumed by Workbench. |
| Process Kernel | `process_kernel/` | Chat/TASK execution authority, model runtime path, capability execution, receipts, and truth stores. |
| Office Worker | `office_worker/` | Local document worker used by Kernel Office capability operations. |

## Flow Types

| Flow | Description |
| --- | --- |
| UX flow | The user works through Workbench v2. UI state covers selection, drafts, display mode, and visible streams. |
| Control flow | Product Runtime receives requests, starts Chat/TASK work, supervises runs, and streams product-facing updates. |
| Data flow | Local Runtime Protocol DTOs keep the UI/runtime boundary typed. Product DB and projection shards store read models for the UI. |
| Truth flow | Process Kernel records `ChatTruth`, `ProcessTruth`, capability receipts, and replayable execution events. |

## Chat And TASK

| Dimension | Chat | TASK |
| --- | --- | --- |
| Purpose | Conversation, read-only inspection, clarification, or suggesting a task. | Controlled agent work that may produce artifacts or workspace changes. |
| Kernel runtime | `ChatRuntime` | `RootProcess` + `TaskAgent` |
| Truth domain | `ChatTruth` | `ProcessTruth` |
| UI surface | Chat stream | TASK stream, run state, artifact cards, approvals where applicable |
| Completion evidence | Chat control decision and transcript facts. | Kernel receipts, truth events, and deliverable evidence. |

## Tool Intent Boundary

Provider-native tool calls are treated as model intent. They do not become completed work until the runtime maps the intent to a registered capability, executes it inside the Kernel boundary, and records a receipt.

```mermaid
flowchart LR
  Intent["Provider tool-call intent"]
  Runtime["Runtime mapping"]
  Capability["Registered capability"]
  Receipt["Receipt"]
  Truth["Kernel truth"]
  Projection["Product projection"]
  UI["Workbench"]

  Intent --> Runtime --> Capability --> Receipt --> Truth --> Projection --> UI
```

## Read Next

- [Runtime Contracts](runtime-contracts.md)
- [Quickstart](quickstart.md)
- [Validation](validation.md)
