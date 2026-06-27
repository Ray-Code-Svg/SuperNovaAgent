# Windows Installer

[English](README.md) | 中文

该目录是仓库内已提交 SuperNova Windows 安装包的导航入口。

预期产物：

- Tauri NSIS installer：`*.exe`

打包命令：

```powershell
npm.cmd --prefix desktop_shell/ui run tauri:build
```

本地打包输出目录：

```powershell
desktop_shell/src-tauri/target/release/bundle/nsis/
```

编译最新安装包后，把 `.exe` 安装包复制到该目录，再提交 release artifact。
