# Module Contract: Product Runtime

[English](../../module-contracts/product-runtime.md) | 中文

## 职责

`crates/product_runtime/` 是 Workbench 与 Process Kernel 之间的本地产品服务层。它负责 HTTP/SSE routes、Product DB、message feed、run registry、runtime event log、projection shards、Kernel bridge 与 product-facing orchestration。

Product Runtime 的关键定位是：把 Kernel truth 和桌面产品界面连接起来，但不替代 Kernel truth。

## 关键文件

| 领域 | 文件 |
| --- | --- |
| 启动与状态 | `main.rs`, `bootstrap.rs`, `app_state.rs`, `app_paths.rs` |
| HTTP/SSE | `http/router.rs`, `http/server.rs`, `http/routes/*`, `http/sse.rs` |
| Product services | `services/chat_service.rs`, `task_service.rs`, `workspace_service.rs`, `container_service.rs`, `run_manager.rs` |
| Kernel bridge | `kernel/kernel_bridge.rs`, `kernel/event_projection.rs`, `kernel_worker.rs` |
| Product state | `state/product_db.rs`, `message_feed.rs`, `run_registry.rs`, `runtime_event_log.rs`, `projection_shards.rs` |

## 输入

- Workbench 通过 generated client 发来的 workspace/container/chat/task/settings/model-config 请求。
- Kernel worker 返回的 stream event、task result、timeline event、artifact event。
- 本地 product state 与 projection shard 读写请求。

## 输出

- `ProtocolResponse<T>`。
- SSE stream event。
- message feed。
- run state。
- container/workspace projection。
- artifact/task/chat read model。

## Source Of Truth

| 数据 | Product Runtime 是否为 truth | 说明 |
| --- | --- | --- |
| Workspace/container product metadata | 是，限 product-level metadata | 不代表 Kernel 执行事实。 |
| Message feed | 否 | UI read model，来自 Chat/TASK/runtime event projection。 |
| Run registry | 否 | run supervision/projection，用于产品状态展示。 |
| TASK completion | 否 | 必须回到 Kernel truth / replay / closure result。 |
| Artifact execution result | 否 | 必须关联 Kernel artifact/capability receipt。 |

## 不变量

- route 层只做请求解析与 service 调用，不应承载业务事实判断。
- service 层可以编排 Product DB 和 Kernel bridge，但不能凭 UI 状态推断执行成功。
- projection shard 是高频消息落盘与读模型优化，不是 truth store。
- run registry 用于 run lifecycle 监督，不是 Kernel state 的替代品。
- 公开材料可以说明本地 runtime 访问边界，但不展开内部访问控制实现。

## 验证入口

```powershell
cargo check --workspace
npm.cmd --prefix desktop_shell/ui run typecheck
```

Product Runtime 的当前状态 claim 还需要结合 real Product Runtime smoke、browser validation 或 installed-app replay，不能只靠静态编译。

