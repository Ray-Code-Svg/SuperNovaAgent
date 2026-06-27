import { createRuntimeClient } from "./runtimeClient";

export async function listArtifactTargets(containerId: string) {
  return (await createRuntimeClient()).artifactTargets(containerId);
}
