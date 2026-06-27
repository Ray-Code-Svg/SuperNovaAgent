# 快速开始

[English](../quickstart.md) | 中文

本文面向想安装、构建或本地检查 SuperNova 的开发者。SuperNova 是 Windows-first；以下命令默认使用 PowerShell。

## 安装 Windows App

仓库内安装包入口：

- [Windows installer directory](../../releases/windows/)

提交后的 NSIS `.exe` 安装包应放在该目录。GitHub 首屏会通过这个目录导航到桌面应用安装包。

## 从源码构建

前置条件：

- Rust toolchain。
- Node.js 和 npm。
- Windows Tauri build prerequisites。
- 可选：用于 live provider check 的本地 provider configuration。

```powershell
cargo check --workspace
npm.cmd --prefix desktop_shell/ui run typecheck
npm.cmd --prefix desktop_shell/ui run build
npm.cmd --prefix desktop_shell/ui run tauri:build
```

Tauri build 会把 Windows NSIS installer 输出到：

```powershell
Get-ChildItem -LiteralPath desktop_shell/src-tauri/target/release/bundle/nsis -Filter *.exe |
  Sort-Object LastWriteTime -Descending |
  Select-Object -First 1
```

## 开发循环

| 目标 | 命令 |
| --- | --- |
| 检查 Rust crates | `cargo check --workspace` |
| Typecheck Workbench | `npm.cmd --prefix desktop_shell/ui run typecheck` |
| 构建 Workbench assets | `npm.cmd --prefix desktop_shell/ui run build` |
| 构建 Windows desktop package | `npm.cmd --prefix desktop_shell/ui run tauri:build` |

## 本地配置

Live provider features 需要本地配置。不要把 provider keys、local access material、私有路径或包含隐私的截图提交进 Git。

公开 setup docs 不描述内部 credential handling 或 local runtime access-control 细节。

## 继续阅读

- [架构](architecture.md)
- [验证](validation.md)
- [Runtime Contracts](runtime-contracts.md)
