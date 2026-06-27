import type { ContainerMessage } from "../../protocol/generated/types";
import type { useI18n } from "../i18n/i18n";

type Translate = ReturnType<typeof useI18n>;

export function messageKindLabel(message: ContainerMessage, t: Translate) {
  if (message.message_type === "reasoning") return t("message.modelReasoning");
  if (message.message_type === "tool_call") return t("message.toolCall");
  if (message.message_type === "tool_result") return t("message.toolResult");
  if (message.message_type === "phase") return message.lane === "task" ? t("message.taskStatus") : t("message.chatStatus");
  if (message.message_type === "artifact") return t("message.artifact");
  if (message.message_type === "approval") return t("message.approval");
  return "";
}

export function messageTitleLabel(value: string | null | undefined, t: Translate) {
  if (!value) return "";
  const normalized = value.trim().toLowerCase();
  if (normalized === "deepseek reasoning") return t("message.deepseekReasoning");
  if (normalized === "deepseek answer") return t("message.deepseekAnswer");
  if (normalized === "chat model call started") return t("message.chatModelCallStarted");
  if (normalized === "chat model call completed") return t("message.chatModelCallCompleted");
  if (normalized === "task model call started") return t("message.taskModelCallStarted");
  if (normalized === "task model call completed") return t("message.taskModelCallCompleted");
  if (normalized === "chat context window checked") return t("message.chatContextWindowChecked");
  if (normalized === "task context window checked") return t("message.taskContextWindowChecked");
  if (normalized === "tool result") return t("message.toolResult");
  if (normalized === "tool call") return t("message.toolCall");
  if (normalized === "approval resolved") return t("message.approvalResolved");
  return value;
}

export function messageStatusLabel(value: string | null | undefined, t: Translate) {
  if (!value) return "";
  const normalized = value.trim().toLowerCase();
  if (normalized === "pending") return t("status.pending");
  if (normalized === "running" || normalized === "streaming") return t("status.running");
  if (normalized === "completed" || normalized === "complete") return t("status.completed");
  if (normalized === "ready") return t("status.ready");
  if (normalized === "verified") return t("status.verified");
  if (normalized === "failed" || normalized === "error") return t("status.failed");
  if (normalized === "interrupted") return t("status.interrupted");
  if (normalized === "unavailable") return t("status.unavailable");
  return value;
}
