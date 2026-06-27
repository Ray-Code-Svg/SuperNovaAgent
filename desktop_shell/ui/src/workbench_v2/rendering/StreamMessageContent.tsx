import type { ContainerMessage } from "../../protocol/generated/types";
import { useI18n } from "../i18n/i18n";
import { messageKindLabel, messageStatusLabel, messageTitleLabel } from "./messageDisplay";
import { MessageMarkdown } from "./MessageMarkdown";

interface StreamMessageContentProps {
  message: ContainerMessage;
  title?: string | null;
  text?: string | null;
}

export function StreamMessageContent({ message, title, text }: StreamMessageContentProps) {
  const t = useI18n();
  const rawDisplayText = text ?? message.body_text ?? readableBody(message.body_json);
  const displayText = messageTitleLabel(rawDisplayText, t);
  const displayTitle = messageTitleLabel(title ?? message.title ?? messageKindLabel(message, t), t);

  if (isStatusMessage(message)) {
    return (
      <details className="sn-status-details" open={message.status === "streaming"}>
        <summary>
          <span>{displayTitle}</span>
          <em>{messageStatusLabel(message.status, t)}</em>
        </summary>
        {displayText && (
          <MessageMarkdown text={displayText} mathStable={message.status !== "streaming"} />
        )}
      </details>
    );
  }

  return (
    <>
      {displayTitle && <header>{displayTitle}</header>}
      <MessageMarkdown text={displayText} mathStable={message.status !== "streaming"} />
    </>
  );
}

function isStatusMessage(message: ContainerMessage) {
  return ["phase", "tool_call", "tool_result"].includes(message.message_type);
}

function readableBody(value: unknown) {
  if (!value || typeof value !== "object") return "";
  const data = value as Record<string, unknown>;
  const message = data.message || data.status || data.event_type || data.schema_version;
  return typeof message === "string" ? message : "";
}
