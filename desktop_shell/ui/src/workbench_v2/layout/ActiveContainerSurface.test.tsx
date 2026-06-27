import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it } from "vitest";

import type { ContainerRecord, TaskDetail } from "../../protocol/generated/types";
import { useWorkbenchUiStore } from "../state/uiStore";
import { ActiveContainerSurface } from "./ActiveContainerSurface";

describe("ActiveContainerSurface", () => {
  beforeEach(() => {
    useWorkbenchUiStore.setState({
      modeByContainer: {},
      draftByContainer: {},
      selectedTaskByContainer: {},
      selectedChatThreadByContainer: {},
      sourceGuidanceByContainer: {},
      artifactTargetByContainer: {},
      openFlyout: null
    });
  });

  it("uses the caller-provided window-scoped container key for TASK mode", () => {
    const scopeId = "window_a:workspace_1:container_1";
    useWorkbenchUiStore.getState().setMode(scopeId, "task");

    const html = renderToStaticMarkup(
      <ActiveContainerSurface
        container={container()}
        scopeId={scopeId}
        messages={[]}
        onSubmit={() => {}}
        onModelConfigSave={() => {}}
        onContextPackSave={() => {}}
        onContextPackEstimate={async () => {
          throw new Error("not used");
        }}
        onSelectSourceGuidance={() => {}}
        onSelectArtifactTarget={() => {}}
        onClarificationSubmit={() => {}}
        onForceClose={() => {}}
      />
    );

    expect(html).toContain("data-mode=\"task\"");
    expect(html).toContain("No TASK messages loaded.");
    expect(html).toContain("Agent TASK");
  });

  it("does not let stale running task detail show force close or disable composer", () => {
    const scopeId = "window_a:workspace_1:container_1";
    useWorkbenchUiStore.getState().setMode(scopeId, "task");
    useWorkbenchUiStore.getState().setDraft(scopeId, "next prompt");

    const html = renderToStaticMarkup(
      <ActiveContainerSurface
        container={container()}
        scopeId={scopeId}
        messages={[]}
        selectedTaskDetail={taskDetail("running")}
        busy={false}
        forceCloseVisible={false}
        onSubmit={() => {}}
        onModelConfigSave={() => {}}
        onContextPackSave={() => {}}
        onContextPackEstimate={async () => {
          throw new Error("not used");
        }}
        onSelectSourceGuidance={() => {}}
        onSelectArtifactTarget={() => {}}
        onClarificationSubmit={() => {}}
        onForceClose={() => {}}
      />
    );

    expect(html).not.toContain("sn-composer-force-close");
    expect(html).not.toContain("disabled=\"\"");
  });
});

function container(): ContainerRecord {
  return {
    container_id: "container_1",
    workspace_uid: "workspace_1",
    title: "Container 1",
    status: "active",
    badges: {
      running: 0,
      approval: 0,
      blocked: 0,
      unread: 0,
      artifact_ready: 0
    },
    created_at_ms: 1,
    updated_at_ms: 1,
    last_active_at_ms: 1,
    default_model_config: null,
    context_policy: null
  };
}

function taskDetail(status: string): TaskDetail {
  return {
    task: {
      task_id: "task_1",
      container_id: "container_1",
      job_id: "job_1",
      title: "Task",
      goal: "Do task",
      status,
      badges: {
        running: 0,
        approval: 0,
        blocked: 0,
        unread: 0,
        artifact_ready: 0
      },
      created_at_ms: 1,
      updated_at_ms: 1
    },
    messages: [],
    artifacts: [],
    approvals: [],
    receipts: [],
    selected_output_dir: null,
    destination_fulfilled: null
  };
}
