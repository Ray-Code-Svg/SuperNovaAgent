import { Button } from "@fluentui/react-components";
import type { ContainerMessage } from "../../protocol/generated/types";
import { isOperationalStatusMessage } from "../rendering/messageFilters";
import { useI18n } from "../i18n/i18n";
import { useStickyScroll } from "../rendering/useStickyScroll";
import { ChatStreamEventRow } from "./ChatStreamEventRow";

interface ChatMessageStreamProps {
  messages: ContainerMessage[];
}

export function ChatMessageStream({ messages }: ChatMessageStreamProps) {
  const t = useI18n();
  const chatMessages = messages.filter((message) => message.lane === "chat");
  const contentMessages = chatMessages.filter((message) => !isOperationalStatusMessage(message));
  const latestChatMessage = chatMessages[chatMessages.length - 1];
  const latestContentMessage = contentMessages[contentMessages.length - 1];
  const { isPinnedToBottom, jumpToLatest, streamRef, updatePinnedState } = useStickyScroll([
    chatMessages.length,
    latestChatMessage?.message_id,
    latestChatMessage?.updated_at_ms,
    latestChatMessage?.source_seq,
    latestChatMessage?.status,
    latestContentMessage?.message_id,
    latestContentMessage?.body_text?.length,
  ]);

  return (
    <div className="sn-message-stream-shell">
      <div className="sn-message-stream" onScroll={updatePinnedState} ref={streamRef}>
        {chatMessages.length === 0 ? (
          <div className="sn-empty-stream">{t("stream.emptyChat")}</div>
        ) : (
          contentMessages.map((message) => (
            <ChatStreamEventRow key={message.message_id} message={message} />
          ))
        )}
      </div>
      {!isPinnedToBottom && (
        <Button appearance="primary" className="sn-jump-latest" size="small" onClick={jumpToLatest}>
          {t("stream.jumpLatest")}
        </Button>
      )}
    </div>
  );
}
