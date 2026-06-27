# SuperNova Desktop UI

This is the parallel desktop UI workspace for the Tauri v2 shell. It is intentionally isolated from the V2 Process Kernel and backend execution chain.

## Commands

```powershell
npm.cmd install
npm.cmd run dev
npm.cmd run typecheck
npm.cmd run test
npm.cmd run build
```

The dev server defaults to `http://127.0.0.1:5173/`.

## Transport Modes

- `mock`: deterministic fixture mode for UI design and component work.
- `live-http`: consumes the existing `/api/*` endpoints without changing backend contracts.
- `tauri`: uses the desktop shell commands and the current product runtime.

Configure with `VITE_SUPERNOVA_TRANSPORT`. Mock scenarios can be selected with `?scenario=empty-workspace`, `?scenario=many-tasks`, or `VITE_SUPERNOVA_MOCK_SCENARIO`.

## Boundaries

This UI package must not change:

- `process_kernel/`
- `tests/run_v2_acceptance_e2e.py`
- `scripts/generate_*test_pack.py`
- `SuperNovaAcceptancePack` asset rules
- `supernova_kernel_cli` command names or fields
- DeepSeek env/config routing

The UI owns presentation, mock fixtures, client-side request guards, and typed adapters only.
