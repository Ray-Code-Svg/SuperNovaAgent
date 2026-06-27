import { createRuntimeClient } from "./runtimeClient";

export async function getRuntimeMeta() {
  return (await createRuntimeClient()).runtimeMeta();
}

export async function getRuntimeCapabilities() {
  return (await createRuntimeClient()).runtimeCapabilities();
}

export async function getRuntimeEvents(afterEventId?: number | null, limit?: number | null) {
  return (await createRuntimeClient()).runtimeEvents({
    after_event_id: afterEventId ?? undefined,
    limit: limit ?? undefined
  });
}

export async function getRuntimeDiagnostics() {
  return (await createRuntimeClient()).diagnostics();
}
