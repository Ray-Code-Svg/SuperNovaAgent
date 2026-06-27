# Quickstart

[中文](zh-CN/quickstart.md) | English

This page is for developers who want to install, build, or inspect SuperNova locally. SuperNova is Windows-first; commands assume PowerShell.

## Install The Windows App

The repository installer entrypoint is:

- [Windows installer directory](../releases/windows/)

When a packaged NSIS `.exe` is committed, it should live in that directory. Use it as the GitHub navigation point for downloading the desktop app.

## Build From Source

Prerequisites:

- Rust toolchain.
- Node.js and npm.
- Windows Tauri build prerequisites.
- Optional local provider configuration for live provider checks.

```powershell
cargo check --workspace
npm.cmd --prefix desktop_shell/ui run typecheck
npm.cmd --prefix desktop_shell/ui run build
npm.cmd --prefix desktop_shell/ui run tauri:build
```

The Tauri build writes the Windows NSIS installer under:

```powershell
Get-ChildItem -LiteralPath desktop_shell/src-tauri/target/release/bundle/nsis -Filter *.exe |
  Sort-Object LastWriteTime -Descending |
  Select-Object -First 1
```

## Development Loop

| Goal | Command |
| --- | --- |
| Check Rust crates | `cargo check --workspace` |
| Typecheck Workbench | `npm.cmd --prefix desktop_shell/ui run typecheck` |
| Build Workbench assets | `npm.cmd --prefix desktop_shell/ui run build` |
| Build Windows desktop package | `npm.cmd --prefix desktop_shell/ui run tauri:build` |

## Local Configuration

Live provider features require local configuration. Keep provider keys, local access material, private paths, and screenshots with private data out of Git.

Public setup docs intentionally do not describe internal credential handling or local runtime access-control details.

## Next Reads

- [Architecture](architecture.md)
- [Validation](validation.md)
- [Runtime Contracts](runtime-contracts.md)
