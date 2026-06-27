# Module Contract: Local Runtime Protocol

[English](../../module-contracts/local-runtime-protocol.md) | 中文

## 职责

`crates/local_runtime_protocol/` 是 Workbench 与 Product Runtime 之间的 typed protocol boundary。它定义 DTO、response envelope、error shape、stream event、task/chat/workspace/container/artifact 等产品对象。

它不是执行层，不直接调用 Kernel，不保存 truth，只定义前后端可以稳定共享的数据合同。

## 关键文件

| 领域 | 文件 |
| --- | --- |
| Envelope/error | `envelope.rs`, `error.rs` |
| Runtime/stream/run | `runtime.rs`, `stream.rs`, `run.rs` |
| Workspace/container | `workspace.rs`, `container.rs` |
| Chat/TASK | `chat.rs`, `task.rs` |
| Artifact/capability | `artifact.rs`, `ui_capability.rs` |
| Settings/model/context | `settings.rs`, `model_config.rs`, `context_pack.rs`, `guidance.rs` |

## 输入

- Product Runtime service 层的 domain result。
- Kernel bridge 投影后的 product-facing state。
- settings/model/context 等本地配置读模型。

## 输出

- Rust DTO。
- frontend generated client 可消费的 schema。
- Workbench query/mutation 需要的 request/response shape。

## 不变量

- protocol DTO 只能表达 UI 应该知道的 product/runtime state。
- protocol 不应泄漏 Kernel 内部执行细节或安全基线实现。
- 新增字段必须考虑前端兼容性、空值语义、历史 projection 的读写。
- protocol field 的含义必须稳定，不能用同一个字段同时表示 truth、projection 和 UI-local state。

## 阅读建议

先读 `envelope.rs` 和 `stream.rs`，理解 response/stream 的共同结构；再读 `chat.rs`、`task.rs`、`run.rs`，最后读 workspace/container/artifact/settings 相关 DTO。这样能先建立“UI 能看见什么”的边界。

