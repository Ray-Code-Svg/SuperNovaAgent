import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import type { ContainerMessage, TaskDetail } from "../../protocol/generated/types";
import { TaskMessageStream } from "./TaskMessageStream";

describe("TaskMessageStream", () => {
  it("removes operational panels and task facts from the default task surface", () => {
    const html = renderToStaticMarkup(
      <TaskMessageStream
        messages={[
          message({ message_id: "phase_1", message_type: "phase", title: "Task status", body_text: "running" }),
          message({ message_id: "tool_1", message_type: "tool_call", title: "Tool call", body_text: "delete file" }),
          message({ message_id: "answer_1", message_type: "text", title: "DeepSeek answer", body_text: "Visible task answer" }),
        ]}
        selectedTaskDetail={detail()}
        onClarificationSubmit={() => {}}
      />
    );

    expect(html).toContain("Visible task answer");
    expect(html).not.toContain("Task operational status");
    expect(html).not.toContain("Task tool calls");
    expect(html).not.toContain("Task structured status");
    expect(html).not.toContain("Artifacts");
    expect(html).not.toContain("Receipts");
    expect(html).not.toContain("delete file");
  });

  it("renders running task model reasoning while hiding runtime telemetry", () => {
    const html = renderToStaticMarkup(
      <TaskMessageStream
        messages={[
          message({ message_id: "phase_1", message_type: "phase", title: "TaskRuntime running", body_text: "Kernel TaskRuntime started." }),
          message({
            message_id: "reasoning_1",
            message_type: "reasoning",
            title: "Task reasoning streaming",
            body_text: "Visible task reasoning delta",
            source_kind: "model_stream",
          }),
        ]}
        selectedTaskDetail={detail()}
        onClarificationSubmit={() => {}}
      />
    );

    expect(html).toContain("Visible task reasoning delta");
    expect(html).not.toContain("Kernel TaskRuntime started.");
  });

  it("keeps artifact cards visible through artifact messages", () => {
    const html = renderToStaticMarkup(
      <TaskMessageStream
        messages={[
          message({
            message_id: "artifact_1",
            message_type: "artifact",
            title: "Generated program",
            body_text: "Artifact ready",
            body_json: { data: { artifact_path: "outputs/simulation.py" } },
          }),
        ]}
        selectedTaskDetail={detail()}
        onClarificationSubmit={() => {}}
      />
    );

    expect(html).toContain("Generated program");
    expect(html).toContain("outputs/simulation.py");
  });
});

function message(overrides: Partial<ContainerMessage>): ContainerMessage {
  return {
    message_id: "message",
    workspace_uid: "ws",
    container_id: "container",
    lane: "task",
    role: "agent",
    message_type: "text",
    status: "completed",
    title: null,
    body_text: null,
    body_json: {},
    card_json: {},
    chat_thread_id: null,
    task_id: "task",
    job_id: "job",
    source_kind: "test",
    source_ref: "test",
    source_seq: 1,
    created_at_ms: 1,
    updated_at_ms: 1,
    sort_key: "00000001",
    ...overrides,
  };
}

function detail(): TaskDetail {
  return {
    task: {
      task_id: "task",
      container_id: "container",
      job_id: "job",
      title: "Task",
      goal: "Do work",
      status: "completed",
      badges: { running: 0, approval: 0, blocked: 0, unread: 0, artifact_ready: 1 },
      created_at_ms: 1,
      updated_at_ms: 1,
    },
    messages: [],
    artifacts: [
      {
        artifact_id: "artifact",
        container_id: "container",
        task_id: "task",
        title: "Generated program",
        artifact_type: "file",
        path: "outputs/simulation.py",
        status: "ready",
        capability_id: "os.write_artifact",
        receipt_ref: "receipt://artifact",
        verified: true,
        kind: "text",
        created_at_ms: 1,
      },
    ],
    approvals: [],
    receipts: [
      {
        receipt_id: "receipt",
        task_id: "task",
        capability_id: "os.write_artifact",
        status: "success",
        kind: "artifact",
        receipt_ref: "receipt://artifact",
        artifact_paths: ["outputs/simulation.py"],
        summary: null,
        created_at_ms: 1,
      },
    ],
    selected_output_dir: "outputs",
    destination_fulfilled: true,
  };
}
