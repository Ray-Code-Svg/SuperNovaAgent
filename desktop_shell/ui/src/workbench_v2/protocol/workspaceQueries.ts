import { createRuntimeClient } from "./runtimeClient";

export async function listWorkspaces() {
  return (await createRuntimeClient()).workspaces();
}

export async function createWorkspace(workspaceRoot: string) {
  return (await createRuntimeClient()).createWorkspace({ workspace_root: workspaceRoot });
}

export async function activateWorkspace(workspaceUid: string) {
  return (await createRuntimeClient()).activateWorkspace({ workspace_uid: workspaceUid });
}

export async function archiveWorkspace(workspaceUid: string) {
  return (await createRuntimeClient()).archiveWorkspace(workspaceUid);
}
