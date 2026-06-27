import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import type { ContainerRecord, RuntimeMeta, WorkspaceRecord } from "../../protocol/generated/types";
import { ProjectsRail } from "./ProjectsRail";

describe("ProjectsRail", () => {
  it("does not render container runtime badges in the default rail", () => {
    const html = renderToStaticMarkup(
      <ProjectsRail
        workspaces={[workspace()]}
        containersByWorkspace={{ ws: [container()] }}
        activeWorkspaceId="ws"
        activeContainerId="container"
        onAddWorkspace={() => {}}
        onAddContainer={() => {}}
        onSelectWorkspace={() => {}}
        onArchiveWorkspace={() => {}}
        onSelectContainer={() => {}}
        onRenameContainer={() => {}}
        onArchiveContainer={() => {}}
        onOpenSettings={() => {}}
      />
    );

    expect(html).toContain("RC0 Container");
    expect(html).not.toContain("running");
    expect(html).not.toContain("blocked");
    expect(html).not.toContain("artifact");
    expect(html).not.toContain("sn-container-badges");
  });

  it("does not synthesize the bootstrap runtime workspace when the registry is empty", () => {
    const html = renderToStaticMarkup(
      <ProjectsRail
        runtime={runtime()}
        workspaces={[]}
        containersByWorkspace={{}}
        activeWorkspaceId="ws_bootstrap"
        activeContainerId={null}
        onAddWorkspace={() => {}}
        onAddContainer={() => {}}
        onSelectWorkspace={() => {}}
        onArchiveWorkspace={() => {}}
        onSelectContainer={() => {}}
        onRenameContainer={() => {}}
        onArchiveContainer={() => {}}
        onOpenSettings={() => {}}
      />
    );

    expect(html).not.toContain("SuperNova");
    expect(html).not.toContain("ws_bootstrap");
  });

  it("shows loading instead of no containers when container history is not loaded yet", () => {
    const html = renderToStaticMarkup(
      <ProjectsRail
        workspaces={[workspace()]}
        containersByWorkspace={{}}
        containerStateByWorkspace={{ ws: "loading" }}
        activeWorkspaceId="ws"
        activeContainerId={null}
        onAddWorkspace={() => {}}
        onAddContainer={() => {}}
        onSelectWorkspace={() => {}}
        onArchiveWorkspace={() => {}}
        onSelectContainer={() => {}}
        onRenameContainer={() => {}}
        onArchiveContainer={() => {}}
        onOpenSettings={() => {}}
      />
    );

    expect(html).toContain("Loading");
    expect(html).not.toContain("No containers");
  });
});

function workspace(): WorkspaceRecord {
  return {
    workspace_uid: "ws",
    workspace_root: "C:/workspace",
    display_name: "Workspace",
    created_at_ms: 1,
    last_opened_at_ms: 1,
    archived: false,
  };
}

function container(): ContainerRecord {
  return {
    container_id: "container",
    workspace_uid: "ws",
    title: "RC0 Container",
    status: "active",
    badges: { running: 1, approval: 0, blocked: 1, unread: 0, artifact_ready: 1 },
    created_at_ms: 1,
    updated_at_ms: 1,
    last_active_at_ms: 1,
    default_model_config: null,
    context_policy: null,
  };
}

function runtime(): RuntimeMeta {
  return {
    workspace_root: "C:/Users/86188/AppData/Local/SuperNova",
    workspace_id: "ws_bootstrap",
    runtime_layer: "rust_product_runtime",
    kernel_layer: "rust_process_kernel",
    transport: "loopback_http_sse",
    python_main_path: false,
    supports: {
      workspace_switch: true,
      sse: true,
      containers: true,
      chat_truth: true,
      process_truth: true,
      appdata_state: true,
    },
    capability_manifest_ref: null,
  };
}
