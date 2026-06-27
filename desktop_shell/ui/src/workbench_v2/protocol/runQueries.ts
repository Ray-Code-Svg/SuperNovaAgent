import { createRuntimeClient } from "./runtimeClient";

export async function listRuns(containerId?: string | null) {
  return (await createRuntimeClient()).runs({
    container_id: containerId || undefined
  });
}
