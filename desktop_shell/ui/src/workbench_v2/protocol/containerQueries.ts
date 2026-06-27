import type { UpdateContainerRequest } from "../../protocol/generated/types";
import { createRuntimeClient } from "./runtimeClient";

export async function listContainers() {
  return (await createRuntimeClient()).containers();
}

export async function listWorkspaceContainers(workspaceUid: string) {
  return (await createRuntimeClient()).workspaceContainers(workspaceUid);
}

export async function createContainer(title?: string) {
  return (await createRuntimeClient()).createContainer({ title });
}

export async function listArchivedContainers() {
  return (await createRuntimeClient()).archivedContainers();
}

export async function archiveContainer(containerId: string) {
  return (await createRuntimeClient()).archiveContainer(containerId);
}

export async function activateContainer(containerId: string) {
  return (await createRuntimeClient()).activateContainer(containerId);
}

export async function updateContainer(containerId: string, request: UpdateContainerRequest) {
  return (await createRuntimeClient()).updateContainer(containerId, request);
}

export async function restoreContainer(containerId: string) {
  return (await createRuntimeClient()).restoreContainer(containerId);
}

export async function deleteContainer(containerId: string) {
  return (await createRuntimeClient()).deleteContainer(containerId);
}

export async function getContainerSnapshot(containerId: string) {
  return (await createRuntimeClient()).containerSnapshot(containerId);
}
