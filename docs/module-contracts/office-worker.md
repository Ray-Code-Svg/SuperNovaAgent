# Module Contract: Office Worker

[中文](../zh-CN/module-contracts/office-worker.md) | English

`office_worker/` is a .NET subprocess used by Kernel `OfficeRuntime` for Office document operations. It does not call models and does not decide task strategy.

## Key Files

| Area | Files |
| --- | --- |
| Worker entry | `office_worker/SuperNova.OfficeWorker/Program.cs` |
| Capability metadata | `office_worker/SuperNova.OfficeWorker/capability_manifest.json` |
| Project | `office_worker/SuperNova.OfficeWorker/SuperNova.OfficeWorker.csproj` |
| Kernel caller | `process_kernel/src/office_runtime.rs` |

## Invariants

- The model must produce or select content before Office mutation.
- Office Worker performs deterministic document operations and returns structured results.
- Kernel records the relevant operation facts and receipts.

