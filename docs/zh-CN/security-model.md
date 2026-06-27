# 安全说明

[English](../security-model.md) | 中文

SuperNova 是本地桌面应用，可能接触本地 workspace data、provider configuration、runtime services 和 generated artifacts。本文面向开发者和用户说明公开安全姿态，不发布内部安全控制实现。

## 公开姿态

| 领域 | 公开规则 |
| --- | --- |
| Provider configuration | provider keys 和本地配置不要进入 Git、截图、日志或 issue。 |
| Workspace data | 本地文件、路径和生成的 artifacts 都应视为用户数据，分享前需要脱敏。 |
| Runtime access | local runtime access 属于敏感边界；公开文档只描述职责，不描述实现级控制。 |
| Tool execution | 文件变更、terminal work、artifact generation 应绑定 runtime evidence 和用户可见状态。 |
| Security claims | 没有当前专门审计时，不声明 broad security completion。 |

## 公开文档可以说什么

可以说明：

- local-first architecture；
- provider configuration 是敏感本地状态；
- tool execution 受 runtime contracts 约束；
- execution truth 与 UI projection 分离；
- broad security claim 需要当前审计证据。

不应包含：

- secrets 或 local access material；
- 私有本机路径、账号标识或未脱敏截图；
- 内部 access-control exchanges；
- 内部 credential handling 细节；
- exploit narratives、bypass examples 或 security test material；
- implementation-level defensive check order。

## Contributor Rules

提交 PR 或发布截图前：

1. 确认没有 provider keys 或 private local state。
2. 脱敏 workspace paths、account names 和 private file names。
3. 实现级安全分析放在内部审计材料中。
4. 当前状态 claim 使用 [验证](validation.md)，不要依赖旧的本地证据。

## 报告安全问题

如果你认为发现了安全问题，报告行为和影响即可，不要在公开 thread 中发布 exploit instructions 或 sensitive local material。
