import { describe, expect, it } from "vitest";

import type { ContainerRecord } from "../../protocol/generated/types";
import {
  activeWorkspaceContainerItems,
  buildRailContainerStateByWorkspace,
  buildRailContainersByWorkspace
} from "./railData";

describe("railData", () => {
  it("keeps workspace-index containers while the active workspace query has no result yet", () => {
    const map = {
      ws_history: [container("container_history", "Historical")]
    };

    const result = buildRailContainersByWorkspace(map, "ws_history", undefined);

    expect(result.ws_history).toHaveLength(1);
    expect(result.ws_history[0].container_id).toBe("container_history");
  });

  it("overrides the active workspace only after active containers are available", () => {
    const result = buildRailContainersByWorkspace(
      { ws_active: [container("container_stale", "Stale", "ws_active")] },
      "ws_active",
      [container("container_fresh", "Fresh", "ws_active")]
    );

    expect(result.ws_active).toHaveLength(1);
    expect(result.ws_active[0].container_id).toBe("container_fresh");
  });

  it("uses workspace-index containers as the active container source during workspace switch", () => {
    const activeItems = activeWorkspaceContainerItems("ws_history", undefined, {
      ws_history: [container("container_history", "Historical")]
    });

    expect(activeItems[0].container_id).toBe("container_history");
  });

  it("does not overwrite a workspace with active container data from another workspace", () => {
    const result = buildRailContainersByWorkspace(
      { ws_active: [container("container_history", "Historical", "ws_active")] },
      "ws_active",
      [container("container_other", "Other", "ws_other")]
    );

    expect(result.ws_active[0].container_id).toBe("container_history");
  });

  it("marks missing workspace container data as loading instead of ready empty data", () => {
    const states = buildRailContainerStateByWorkspace({
      workspaces: [workspace("ws_history")],
      containersByWorkspace: {},
      workspaceContainerMapLoading: true,
      workspaceContainerMapError: false,
      activeWorkspaceId: "ws_history",
      activeContainersLoading: true,
      activeContainersError: false
    });

    expect(states.ws_history).toBe("loading");
  });
});

function workspace(workspaceUid: string) {
  return {
    workspace_uid: workspaceUid,
    workspace_root: "C:/workspace",
    display_name: "Workspace",
    created_at_ms: 1,
    last_opened_at_ms: 1,
    archived: false
  };
}

function container(containerId: string, title: string, workspaceUid = "ws_history"): ContainerRecord {
  return {
    container_id: containerId,
    workspace_uid: workspaceUid,
    title,
    status: "active",
    badges: { running: 0, approval: 0, blocked: 0, unread: 0, artifact_ready: 0 },
    created_at_ms: 1,
    updated_at_ms: 1,
    last_active_at_ms: 1,
    default_model_config: null,
    context_policy: null
  };
}
