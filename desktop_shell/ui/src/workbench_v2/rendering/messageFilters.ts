import type { ContainerMessage } from "../../protocol/generated/types";

export function isOperationalStatusMessage(message: ContainerMessage) {
  return message.message_type === "phase" || message.message_type === "tool_call" || message.message_type === "tool_result";
}
