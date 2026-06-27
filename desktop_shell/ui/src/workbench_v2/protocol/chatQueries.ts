import type { StreamEventHandler } from "../../protocol/generated/client";
import type { ChatTurnStreamRequest, ForceCloseRequest } from "../../protocol/generated/types";
import { createRuntimeClient } from "./runtimeClient";

export async function listChatThreads(containerId: string) {
  return (await createRuntimeClient()).chatThreads(containerId);
}

export async function createChatThread(containerId: string) {
  return (await createRuntimeClient()).createChatThread(containerId, { title: "Chat" });
}

export async function sendChatTurn(
  chatThreadId: string,
  request: ChatTurnStreamRequest,
  onEvent?: StreamEventHandler<unknown>
) {
  return (await createRuntimeClient()).chatTurnStream(chatThreadId, request, onEvent);
}

export async function forceCloseChatTurn(chatThreadId: string, request: ForceCloseRequest = {}) {
  return (await createRuntimeClient()).forceCloseChatTurn(chatThreadId, request);
}
