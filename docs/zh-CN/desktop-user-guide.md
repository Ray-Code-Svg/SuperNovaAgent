# 桌面端用户指南

[English](../desktop-user-guide.md) | 中文

本文面向使用 SuperNova Windows 桌面端 Workbench 的用户。它解释产品界面和使用流程，不解释内部 runtime 实现。

SuperNova 当前是 RC0 desktop candidate。本文用于说明桌面端预期工作流；如果要声明当前版本可发布或已验证，请先按 [验证](validation.md) 重新取证。

下方截图来自 2026-06-27 的当前 desktop Workbench 或 Windows installer，只作为 UI 示例，不作为 release validation evidence。

## 首次启动

1. 从 [releases/windows/](../../releases/windows/) 安装 Windows app，或按 [快速开始](quickstart.md) 从源码构建。
2. 启动 SuperNova，等待启动页完成：
   - Opening SuperNova shell。
   - Starting Product Runtime。
   - Checking Process Kernel。
   - Loading workspace history。
   - Applying desktop settings。
3. 打开 **System Settings**。
4. 在 **Provider API** 中配置 provider credential，并先运行 **Test**，再开始真实 Chat 或 TASK 工作。
5. 在 **Appearance** 中选择语言和主题。
6. 添加 workspace，然后为一个聚焦工作流创建 container。

不要提交 provider key、包含隐私的截图、本机路径或 local access material。

<p align="center">
  <img src="../assets/EXE_InstallerWindows.png" alt="SuperNova Windows installer setup screen" width="520" />
</p>

| 深色启动页 | 浅色启动页 |
| --- | --- |
| <img src="../assets/Welcome%20Page%20Dark.png" alt="SuperNova 深色启动页" width="420" /> | <img src="../assets/Welcome%20Page%20Light.png" alt="SuperNova 浅色启动页" width="420" /> |

## Workbench 核心概念

| 概念 | 含义 |
| --- | --- |
| Workspace | SuperNova 当前处理的本地项目或文件夹根目录。Workspace-scoped capabilities 应该停留在这个边界内。 |
| Container | Workspace 里的一个聚焦工作流。它把相关 Chat、TASK、context、sources、output choices 和 history 放在一起。 |
| Chat | 非 mutation 模式，用于读取、解释、澄清，以及判断请求是否需要 TASK execution。 |
| TASK | 受控执行模式，用于 multi-step work、tool calls、file changes、command runs、artifacts 和 completion evidence。 |
| Sources | 可选的 `@` references，用来引导模型优先参考选中的文件、文件夹或历史 Chat/TASK。 |
| Output Destination | 可选的 `$` output guidance，用来引导 artifact 输出位置；最终是否真实产生文件仍以 runtime receipt 为准。 |
| Artifact | 用户可见的输出，例如 Markdown、CSV、JSON、TXT、DOCX，或 registered capability 生成的 package。 |
| Receipt | Runtime 证据，记录某个 registered capability 是否真的执行，以及它产生了什么。 |

<p align="center">
  <img src="../assets/User%20Guide.png" alt="SuperNova System Settings guide tab inside the Workbench" width="880" />
</p>

## 主流程

1. 在 **PROJECTS** rail 中选择或创建 workspace。
2. 创建或选择 container。
3. 在 composer 里用 **Chat** 做快速提问、解释和检查。
4. 当请求需要本地执行、文件、命令、文档或 artifact 时，切换到 **TASK**。
5. 如果希望 agent 聚焦具体文件、目录或历史记录，添加 `@` sources。
6. 如果希望生成物进入特定目录，设置 `$` output destination。
7. TASK 完成后，先检查 task stream、status、artifacts、receipts 和 completion statement，再判断工作是否完成。

可以在 composer 中输入 `/chat` 和 `/task` 切换模式。Model 和 Context flyout 可以从 Workbench toolbar 或 slash command 打开。

<p align="center">
  <img src="../assets/Slash%20Command.png" alt="Slash command flyout showing Chat, TASK, Model, and Context entries" width="880" />
</p>

## Chat 模式

适合用 Chat 做：

- 解释代码库、文件夹或文档集合。
- 读取选中的文件、目录列表、diff、dataset、Office text、PDF text，或脱敏后的本机环境信息。
- 在缺少关键信息时追问。
- 当请求需要 mutation、terminal execution、long-running work 或 artifact delivery 时，建议切换到 TASK。

Chat 不应该声称已经改文件、运行命令或完成任务。这些属于 TASK。

## TASK 模式

需要 SuperNova 执行受控本地工作时，使用 TASK：

| 目标 | TASK 可以做什么 |
| --- | --- |
| 代码和文件工作 | 在 workspace 边界内改文件，复制、移动、重命名、删除、解压，并记录真实改动。 |
| Workspace 分析 | 创建 SourceSet、分页查看文件集、查重、查看 recent changes、生成 tree index 和 performance inventory。 |
| 命令和服务 | 运行有边界的 foreground command，启动/停止/查询 managed local service，并把 terminal 结果写入 task timeline。 |
| 文档 | 通过 Office worker 读取、验证、创建、改写和比较 DOCX；在支持范围内检查 workbook/PDF text。 |
| 数据集 | 读取 CSV，导出 CSV 或 Markdown，创建 temporary dataset，并保留 schema/row-count 事实。 |
| Artifacts | 通过 registered capabilities 写出用户可见的 Markdown、CSV、JSON、TXT、DOCX 或 package outputs。 |
| 打包 | 生成 zip package、manifest/checksum 辅助文件，并验证 package artifact。 |

TASK 的过程会显示在 task stream、status rail、artifact cards、receipts 和最终 completion statement 中。

## Sources 和 Output

当任务需要优先参考特定上下文时，使用 **Sources**：

- Workspace folders。
- Workspace files。
- 之前的 Chat history。
- 之前的 TASK history。

当交付物应该放到特定 workspace 文件夹时，使用 **Output Destination**。这只是输出位置 guidance；TASK 完成后，需要通过 artifact cards 和 receipts 确认真正创建了什么。

<p align="center">
  <img src="../assets/Source%20Config.png" alt="Source picker for workspace files, directories, Chat, and TASK history" width="880" />
</p>

## Model 和 Context

**Model** flyout 控制 provider route、model、thinking mode、reasoning effort 和 output token budget。

**Context** flyout 控制下一次请求包含多少 recent Chat/TASK history 和 selected context。它可以根据条目选择 compact summary、reference-only 或 fuller context。

当前 provider 行为应通过 **Provider API** settings 里的 live provider test 确认。不能用 mock 或 fallback 结果证明 live-provider readiness。

| 模型配置 | 上下文配置 |
| --- | --- |
| <img src="../assets/Model%20Config.png" alt="Model configuration flyout" width="420" /> | <img src="../assets/Context%20Config.png" alt="Context configuration flyout" width="420" /> |

## Artifacts 和证据

TASK 完成后检查：

1. Task status 是 completed、blocked、failed、interrupted、cancelled 或 waiting for input。
2. Artifact cards 是否列出你预期的用户可见输出。
3. 关键 file、document、terminal、dataset 或 package action 是否有 receipt。
4. Completion statement 是否说明产物、关键来源和已知限制。
5. 文件是否真实存在于 workspace 或选中的 output destination。

如果模型文字说已经生成，但没有 artifact 或 receipt，以 runtime evidence 为准。

## 常见场景

| 场景 | 推荐路径 |
| --- | --- |
| 理解一个代码仓库 | 先用 Chat，选择 `@` 文件或目录，请它解释结构和风险。只有需要修改或交付物时再进入 TASK。 |
| 做小范围代码改动 | 用 TASK，明确目标文件和预期行为，然后检查 diff 和 receipts。 |
| 生成报告 | 用 TASK，选择 sources，设置 output destination，并检查生成的 artifact。 |
| 处理 DOCX | 创建或改写用 TASK；完成后检查 validation 和 artifact cards。 |
| 运行 build 或本地命令 | 用 TASK。命令要有边界，并在 task stream 中检查 terminal result。 |
| 打包交付物 | 用 TASK，明确 source set 和 package destination；检查 zip、manifest/checksum 输出。 |

## 故障排查

| 现象 | 检查项 |
| --- | --- |
| Composer 不可用 | 是否已经选择或创建 container。Chat/TASK 工作需要 container。 |
| Provider call 失败 | 打开 **System Settings -> Provider API**，确认 key 已保存，并运行 **Test**。 |
| TASK 在等待 | 检查 task stream 中是否有 clarification、approval、failure 或 input message。 |
| 没有 artifact | 检查请求是否明确要求输出、是否设置 `$` output destination，以及 receipts 是否显示 create/export/package action。 |
| Run 看起来卡住 | 先刷新 runtime state。只有在 user-facing run 明显 stale 且你理解它不证明外部工作完成时，才使用 force close。 |
| 截图或报告含隐私 | 分享或提交前脱敏。不要公开 provider keys、本机用户名、私有路径或 local access material。 |

## 边界

SuperNova 是本地桌面 agent workbench，不是对整台电脑的无限制控制。它通过 registered runtime capabilities、workspace boundaries 和 receipts 工作。

公开文档不应在没有当前验证的情况下声明 final release readiness、broad security completion 或 live-provider verification。

## 继续阅读

- [快速开始](quickstart.md)
- [验证](validation.md)
- [安全说明](security-model.md)
- [Runtime Contracts](runtime-contracts.md)
