import type { ContainerRecord, WorkspaceRecord } from "../../protocol/generated/types";

export type WorkspaceContainerLoadState = "ready" | "loading" | "error";

export function buildRailContainersByWorkspace(
  workspaceContainerMap: Record<string, ContainerRecord[]> | undefined,
  activeWorkspaceId: string | null | undefined,
  activeContainers: ContainerRecord[] | undefined
) {
  const containersByWorkspace = { ...(workspaceContainerMap || {}) };
  if (activeWorkspaceId && activeContainers && containersBelongToWorkspace(activeContainers, activeWorkspaceId)) {
    containersByWorkspace[activeWorkspaceId] = activeContainers;
  }
  return containersByWorkspace;
}

export function activeWorkspaceContainerItems(
  workspaceId: string | null,
  activeContainers: ContainerRecord[] | undefined,
  workspaceContainerMap: Record<string, ContainerRecord[]> | undefined
) {
  if (!workspaceId) return [];
  if (activeContainers && containersBelongToWorkspace(activeContainers, workspaceId)) {
    return activeContainers;
  }
  return workspaceContainerMap?.[workspaceId] || [];
}

export function buildRailContainerStateByWorkspace({
  workspaces,
  containersByWorkspace,
  workspaceContainerMapLoading,
  workspaceContainerMapError,
  activeWorkspaceId,
  activeContainersLoading,
  activeContainersError
}: {
  workspaces: WorkspaceRecord[];
  containersByWorkspace: Record<string, ContainerRecord[]>;
  workspaceContainerMapLoading: boolean;
  workspaceContainerMapError: boolean;
  activeWorkspaceId: string | null;
  activeContainersLoading: boolean;
  activeContainersError: boolean;
}) {
  const states: Record<string, WorkspaceContainerLoadState> = {};
  for (const workspace of workspaces) {
    const hasKnownContainers = hasOwn(containersByWorkspace, workspace.workspace_uid);
    if (hasKnownContainers) {
      states[workspace.workspace_uid] = "ready";
    } else if (workspaceContainerMapError) {
      states[workspace.workspace_uid] = "error";
    } else if (workspaceContainerMapLoading) {
      states[workspace.workspace_uid] = "loading";
    } else {
      states[workspace.workspace_uid] = "ready";
    }
  }
  if (activeWorkspaceId && !hasOwn(containersByWorkspace, activeWorkspaceId)) {
    if (activeContainersError) {
      states[activeWorkspaceId] = "error";
    } else if (activeContainersLoading) {
      states[activeWorkspaceId] = "loading";
    }
  }
  return states;
}

function containersBelongToWorkspace(containers: ContainerRecord[], workspaceId: string) {
  return containers.length === 0 || containers.every((container) => container.workspace_uid === workspaceId);
}

function hasOwn(record: Record<string, unknown>, key: string) {
  return Object.prototype.hasOwnProperty.call(record, key);
}
