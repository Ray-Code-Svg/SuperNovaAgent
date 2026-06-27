import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import type { ContainerMessage } from "../../protocol/generated/types";
import { ChatMessageStream } from "./ChatMessageStream";

describe("ChatMessageStream", () => {
  it("keeps operational status messages out of the default chat surface", () => {
    const html = renderToStaticMarkup(
      <ChatMessageStream
        messages={[
          message({ message_id: "phase_1", message_type: "phase", title: "Chat status", body_text: "running" }),
          message({ message_id: "tool_1", message_type: "tool_call", title: "Tool call", body_text: "read file" }),
          message({ message_id: "answer_1", message_type: "text", title: "DeepSeek answer", body_text: "Visible chat answer" }),
        ]}
      />
    );

    expect(html).toContain("Visible chat answer");
    expect(html).not.toContain("Chat operational status");
    expect(html).not.toContain("Chat tool calls");
    expect(html).not.toContain("read file");
  });

  it("renders running model stream reasoning and answer without operational telemetry", () => {
    const html = renderToStaticMarkup(
      <ChatMessageStream
        messages={[
          message({ message_id: "phase_1", message_type: "phase", title: "ChatRuntime running", body_text: "Kernel ChatRuntime is processing this turn." }),
          message({
            message_id: "reasoning_1",
            role: "agent",
            message_type: "reasoning",
            title: "DeepSeek reasoning streaming",
            body_text: "Visible reasoning delta",
            source_kind: "model_stream",
          }),
          message({
            message_id: "answer_1",
            message_type: "text",
            title: "DeepSeek answer streaming",
            body_text: "Visible answer delta",
            source_kind: "model_stream",
          }),
        ]}
      />
    );

    expect(html).toContain("Visible reasoning delta");
    expect(html).toContain("Visible answer delta");
    expect(html).not.toContain("Kernel ChatRuntime is processing this turn.");
  });
});

function message(overrides: Partial<ContainerMessage>): ContainerMessage {
  return {
    message_id: "message",
    workspace_uid: "ws",
    container_id: "container",
    lane: "chat",
    role: "assistant",
    message_type: "text",
    status: "completed",
    title: null,
    body_text: null,
    body_json: {},
    card_json: {},
    chat_thread_id: "chat",
    task_id: null,
    job_id: null,
    source_kind: "test",
    source_ref: "test",
    source_seq: 1,
    created_at_ms: 1,
    updated_at_ms: 1,
    sort_key: "00000001",
    ...overrides,
  };
}
