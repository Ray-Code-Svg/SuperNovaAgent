# SuperNova Office Worker

This is the Phase 4 Office Runtime worker for V2.

- Runtime: `.NET/C#`
- Main SDK: `DocumentFormat.OpenXml` (`Microsoft Open XML SDK`)
- Scope: DOCX v1 read/create/rewrite-preview/rewrite-save-as/in-place-preview/in-place-rewrite/diff/validate
- Role: capability worker only. It does not call LLM APIs and does not generate personalized content.

The worker emits JSON receipts for the Rust Process Kernel / Task Agent Runtime to record in `ProcessTruth`.

Example:

```powershell
dotnet run --project .\office_worker\SuperNova.OfficeWorker -- self-test
dotnet run --project .\office_worker\SuperNova.OfficeWorker -- read-text --input .\sample.docx --receipt .\receipt.json
```
