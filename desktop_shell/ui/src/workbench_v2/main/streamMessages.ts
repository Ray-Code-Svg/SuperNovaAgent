import type { ContainerMessage, ProtocolEvent } from "../../protocol/generated/types";

const EMPTY_MESSAGES: ContainerMessage[] = [];
export const LIVE_MESSAGE_BUFFER_LIMIT = 240;

export function streamEventMessage(event: ProtocolEvent<unknown>): ContainerMessage | null {
  const payload = event.payload as { message?: ContainerMessage | null } | null;
  const message = payload?.message;
  if (!message?.message_id || !message.container_id) return null;
  return message;
}

export function messageBelongsToVisibleContainer(
  message: ContainerMessage,
  visibleContainerId: string | null | undefined
) {
  return Boolean(visibleContainerId && message.container_id === visibleContainerId);
}

export function mergeMessages(base: ContainerMessage[], additions: ContainerMessage[]) {
  const byId = new Map<string, ContainerMessage>();
  for (const message of base) {
    byId.set(message.message_id, message);
  }
  for (const message of additions) {
    byId.set(message.message_id, message);
  }
  return suppressDuplicateTaskFinalAnswers(
    suppressDuplicateChatFinalAnswers(
      Array.from(byId.values()).sort(compareMessagesForDisplay)
    )
  );
}

export function appendMessageByContainer(
  current: Record<string, ContainerMessage[]>,
  message: ContainerMessage,
  limit = LIVE_MESSAGE_BUFFER_LIMIT
) {
  return {
    ...current,
    [message.container_id]: limitMessages(
      mergeMessages(current[message.container_id] || EMPTY_MESSAGES, [message]),
      limit
    )
  };
}

export function limitMessages(messages: ContainerMessage[], limit = LIVE_MESSAGE_BUFFER_LIMIT) {
  if (messages.length <= limit) return messages;
  return messages.slice(messages.length - limit);
}

function suppressDuplicateChatFinalAnswers(messages: ContainerMessage[]) {
  const kept: ContainerMessage[] = [];
  for (const message of messages) {
    if (!isChatTruthAssistantText(message)) {
      kept.push(message);
      continue;
    }
    const duplicateStream = kept.find(
      (candidate) =>
        isModelStreamAssistantText(candidate) &&
        candidate.chat_thread_id === message.chat_thread_id &&
        sameSubstantialText(candidate.body_text, message.body_text) &&
        Math.abs(candidate.created_at_ms - message.created_at_ms) <= 120_000
    );
    if (!duplicateStream) kept.push(message);
  }
  return kept;
}

function suppressDuplicateTaskFinalAnswers(messages: ContainerMessage[]) {
  const kept: ContainerMessage[] = [];
  const finalByTaskAndBody = new Set<string>();
  for (const message of messages) {
    if (!isTaskFinalAnswer(message)) {
      kept.push(message);
      continue;
    }
    const key = `${message.task_id || message.job_id || ""}:${normalizeText(message.body_text)}`;
    if (!finalByTaskAndBody.has(key)) {
      finalByTaskAndBody.add(key);
      kept.push(message);
    }
  }
  return kept;
}

function isTaskFinalAnswer(message: ContainerMessage) {
  return (
    message.lane === "task" &&
    message.role === "agent" &&
    message.message_type === "text" &&
    message.title === "Task final answer"
  );
}

function isChatTruthAssistantText(message: ContainerMessage) {
  return (
    message.lane === "chat" &&
    message.role === "assistant" &&
    message.message_type === "text" &&
    message.source_kind === "chat_truth"
  );
}

function isModelStreamAssistantText(message: ContainerMessage) {
  return (
    message.lane === "chat" &&
    message.role === "assistant" &&
    message.message_type === "text" &&
    message.source_kind === "model_stream"
  );
}

function sameSubstantialText(left?: string | null, right?: string | null) {
  const normalizedLeft = normalizeText(left);
  if (normalizedLeft.length < 30) return false;
  return normalizedLeft === normalizeText(right);
}

function normalizeText(value?: string | null) {
  return (value || "").split(/\s+/).filter(Boolean).join(" ");
}

function compareMessagesForDisplay(left: ContainerMessage, right: ContainerMessage) {
  const created = left.created_at_ms - right.created_at_ms;
  if (created !== 0) return created;
  const sourceSeq = (left.source_seq || 0) - (right.source_seq || 0);
  if (sourceSeq !== 0) return sourceSeq;
  const sortKey = left.sort_key.localeCompare(right.sort_key);
  if (sortKey !== 0) return sortKey;
  return left.message_id.localeCompare(right.message_id);
}
