import { BotRegular, PersonRegular } from "@fluentui/react-icons";
import type { ContainerMessage } from "../../protocol/generated/types";
import { StreamMessageContent } from "../rendering/StreamMessageContent";

interface ChatStreamEventRowProps {
  message: ContainerMessage;
}

export function ChatStreamEventRow({ message }: ChatStreamEventRowProps) {
  const isUser = message.role === "user";
  return (
    <div className="sn-message-row" data-role={message.role} data-type={message.message_type}>
      <div className="sn-message-avatar">{isUser ? <PersonRegular /> : <BotRegular />}</div>
      <article className="sn-message-card">
        <StreamMessageContent message={message} />
      </article>
    </div>
  );
}
