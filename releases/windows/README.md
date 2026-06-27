# Windows Installer

[中文](README.zh-CN.md) | English

This directory is the repository navigation point for committed SuperNova Windows installer packages.

Expected artifact:

- Tauri NSIS installer: `*.exe`

Build command:

```powershell
npm.cmd --prefix desktop_shell/ui run tauri:build
```

Local build output:

```powershell
desktop_shell/src-tauri/target/release/bundle/nsis/
```

After building the latest installer, copy the `.exe` package into this directory before committing the release artifact.
