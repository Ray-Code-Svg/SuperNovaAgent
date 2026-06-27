import { describe, expect, it } from "vitest";

import type { ContainerMessage, ProtocolEvent } from "../../protocol/generated/types";
import { LocalRuntimeProtocolClient } from "../../protocol/generated/client";
import {
  appendMessageByContainer,
  LIVE_MESSAGE_BUFFER_LIMIT,
  mergeMessages,
  messageBelongsToVisibleContainer,
  streamEventMessage
} from "./streamMessages";

describe("Workbench v2 streaming messages", () => {
  it("emits SSE events before the response stream closes", async () => {
    const encoder = new TextEncoder();
    let controller!: ReadableStreamDefaultController<Uint8Array>;
    const stream = new ReadableStream<Uint8Array>({
      start(activeController) {
        controller = activeController;
      }
    });
    const client = new LocalRuntimeProtocolClient({
      baseUrl: "http://runtime.test",
      fetchImpl: async () => new Response(stream, { status: 200 })
    });
    const received: string[] = [];
    let completed = false;
    let resolveFirstEvent: () => void = () => {};
    const firstEvent = new Promise<void>((resolve) => {
      resolveFirstEvent = resolve;
    });

    const pending = client
      .chatTurnStream("chat_1", {
        message: "hello",
        context_pack: null,
        context_pack_id: null,
        model_config: null,
        session_id: null,
        source_guidance: null
      }, (event) => {
        received.push(event.event_type);
        if (received.length === 1) resolveFirstEvent();
      })
      .then(() => {
        completed = true;
      });

    controller.enqueue(encoder.encode(sse("chat.phase", "evt_1", "running")));
    await firstEvent;

    expect(received).toEqual(["chat.phase"]);
    expect(completed).toBe(false);

    controller.enqueue(encoder.encode(sse("chat.heartbeat", "evt_2", "heartbeat")));
    controller.close();
    await pending;

    expect(received).toEqual(["chat.phase", "chat.heartbeat"]);
    expect(completed).toBe(true);
  });

  it("extracts token-delta message payloads and replaces streaming updates by message id", () => {
    const initial = message({
      body_text: "Hel",
      source_kind: "model_stream",
      source_ref: "model_call_1",
      status: "streaming",
      sort_key: "001"
    });
    const updated = message({
      body_text: "Hello",
      source_kind: "model_stream",
      source_ref: "model_call_1",
      status: "completed",
      sort_key: "002"
    });
    const unrelated = message({ message_id: "msg_2", body_text: "other", sort_key: "003" });
    const event = protocolEvent("chat.answer.delta", updated);

    expect(streamEventMessage(event)).toEqual(updated);
    expect(mergeMessages([initial], [updated, unrelated])).toEqual([updated, unrelated]);
    expect(appendMessageByContainer({ container_1: [initial] }, updated).container_1).toEqual([updated]);
  });

  it("keeps live buffers bounded to the latest window", () => {
    let buffers: Record<string, ContainerMessage[]> = {};
    for (let index = 0; index < LIVE_MESSAGE_BUFFER_LIMIT + 5; index += 1) {
      buffers = appendMessageByContainer(
        buffers,
        message({
          message_id: `msg_${index}`,
          body_text: `message ${index}`,
          created_at_ms: index,
          sort_key: index.toString().padStart(4, "0")
        })
      );
    }

    expect(buffers.container_1).toHaveLength(LIVE_MESSAGE_BUFFER_LIMIT);
    expect(buffers.container_1[0].message_id).toBe("msg_5");
    expect(buffers.container_1.at(-1)?.message_id).toBe(`msg_${LIVE_MESSAGE_BUFFER_LIMIT + 4}`);
  });

  it("only appends full stream messages for the visible container", () => {
    expect(messageBelongsToVisibleContainer(message({ container_id: "container_1" }), "container_1")).toBe(true);
    expect(messageBelongsToVisibleContainer(message({ container_id: "container_2" }), "container_1")).toBe(false);
    expect(messageBelongsToVisibleContainer(message({ container_id: "container_1" }), null)).toBe(false);
  });

  it("extracts chat and task reasoning token deltas as replaceable messages", () => {
    const chatReasoning = message({
      body_text: "thinking",
      message_type: "reasoning",
      source_kind: "model_stream",
      source_ref: "chat_model_call_1",
      status: "streaming"
    });
    const taskReasoning = message({
      body_text: "planning",
      lane: "task",
      message_type: "reasoning",
      source_kind: "model_stream",
      source_ref: "task_model_call_1",
      status: "streaming",
      task_id: "job_1"
    });

    expect(streamEventMessage(protocolEvent("chat.reasoning.delta", chatReasoning))).toEqual(chatReasoning);
    expect(streamEventMessage(protocolEvent("task.reasoning.delta", taskReasoning))).toEqual(taskReasoning);
    expect(appendMessageByContainer({}, taskReasoning).container_1).toEqual([taskReasoning]);
  });

  it("suppresses duplicate chat truth final answers already carried by model stream", () => {
    const body = "This is a substantial final answer that should only render once in the stream.";
    const streamFinal = message({
      message_id: "stream_1",
      body_text: body,
      source_kind: "model_stream",
      status: "completed",
      created_at_ms: 1_000,
      sort_key: "001"
    });
    const chatTruthFinal = message({
      message_id: "truth_1",
      body_text: body,
      source_kind: "chat_truth",
      status: "completed",
      created_at_ms: 2_000,
      sort_key: "002"
    });

    expect(mergeMessages([streamFinal], [chatTruthFinal])).toEqual([streamFinal]);
  });

  it("suppresses duplicate task final answers from completion and job projections", () => {
    const body = "Task final answer body that should appear once in the task stream.";
    const firstFinal = message({
      message_id: "task_final_1",
      body_text: body,
      lane: "task",
      message_type: "text",
      role: "agent",
      task_id: "job_1",
      job_id: "job_1",
      title: "Task final answer",
      sort_key: "001"
    });
    const duplicateFinal = message({
      message_id: "task_final_2",
      body_text: body,
      lane: "task",
      message_type: "text",
      role: "agent",
      task_id: "job_1",
      job_id: "job_1",
      title: "Task final answer",
      sort_key: "002"
    });

    expect(mergeMessages([firstFinal], [duplicateFinal])).toEqual([firstFinal]);
  });
});

function sse(eventType: string, eventId: string, body: string) {
  return [
    `event: ${eventType}`,
    `data: ${JSON.stringify(protocolEvent(eventType, message({ message_id: eventId, body_text: body })))}`,
    "",
    ""
  ].join("\n");
}

function protocolEvent(eventType: string, streamMessage: ContainerMessage): ProtocolEvent<unknown> {
  return {
    protocol_version: "supernova.local_runtime.v1",
    schema_version: "supernova.protocol.event.v1",
    event_id: `evt_${eventType}`,
    event_type: eventType,
    cursor: {
      kind: "message_feed",
      after_event_id: 1
    },
    workspace_id: "ws_1",
    container_id: streamMessage.container_id,
    chat_thread_id: streamMessage.chat_thread_id,
    task_id: streamMessage.task_id,
    job_id: streamMessage.job_id,
    payload: {
      message: streamMessage
    }
  };
}

function message(overrides: Partial<ContainerMessage> = {}): ContainerMessage {
  return {
    body_json: {},
    body_text: "text",
    card_json: {},
    chat_thread_id: "chat_1",
    container_id: "container_1",
    created_at_ms: 1,
    job_id: null,
    lane: "chat",
    message_id: "msg_1",
    message_type: "text",
    role: "assistant",
    sort_key: "001",
    source_kind: "test",
    source_ref: "test",
    source_seq: null,
    status: "streaming",
    task_id: null,
    title: null,
    updated_at_ms: 1,
    workspace_uid: "ws_1",
    ...overrides
  };
}
