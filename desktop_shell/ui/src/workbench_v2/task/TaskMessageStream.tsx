import { Button } from "@fluentui/react-components";
import { BotRegular, PersonRegular } from "@fluentui/react-icons";
import type { ContainerMessage, TaskDetail } from "../../protocol/generated/types";
import { isOperationalStatusMessage } from "../rendering/messageFilters";
import { useI18n } from "../i18n/i18n";
import { StreamMessageContent } from "../rendering/StreamMessageContent";
import { useStickyScroll } from "../rendering/useStickyScroll";
import { ArtifactReadyCard } from "./ArtifactReadyCard";
import { ClarificationCard } from "./ClarificationCard";

interface TaskMessageStreamProps {
  messages: ContainerMessage[];
  selectedTaskId?: string | null;
  selectedTaskDetail?: TaskDetail;
  approvalBusy?: boolean;
  onClarificationSubmit(taskId: string, input: string): void;
}

export function TaskMessageStream({
  approvalBusy,
  messages,
  selectedTaskDetail,
  onClarificationSubmit
}: TaskMessageStreamProps) {
  const t = useI18n();
  const taskMessages = messages.filter((message) => message.lane === "task");
  const contentMessages = taskMessages.filter(
    (message) => !isOperationalStatusMessage(message) && message.message_type !== "approval"
  );
  const latestTaskMessage = taskMessages[taskMessages.length - 1];
  const latestContentMessage = contentMessages[contentMessages.length - 1];
  const { isPinnedToBottom, jumpToLatest, streamRef, updatePinnedState } = useStickyScroll([
    taskMessages.length,
    latestTaskMessage?.message_id,
    latestTaskMessage?.updated_at_ms,
    latestTaskMessage?.source_seq,
    latestTaskMessage?.status,
    latestContentMessage?.message_id,
    latestContentMessage?.body_text?.length,
  ]);

  return (
    <div className="sn-message-stream-shell">
      <div className="sn-message-stream" onScroll={updatePinnedState} ref={streamRef}>
        {taskMessages.length === 0 ? (
          <div className="sn-empty-stream">{t("stream.emptyTask")}</div>
        ) : (
          contentMessages.map((message) => {
            return (
              <div
                className="sn-message-row"
                data-role={message.role}
                data-type={message.message_type}
                key={message.message_id}
              >
                <div className="sn-message-avatar">{message.role === "user" ? <PersonRegular /> : <BotRegular />}</div>
                <article className="sn-message-card">
                  <StreamMessageContent
                    message={message}
                    title={message.title}
                    text={message.body_text || ""}
                  />
                  {message.message_type === "artifact" && (
                    <ArtifactReadyCard artifact={artifactForMessage(selectedTaskDetail, message)} />
                  )}
                  {isClarificationMessage(message) && (
                    <ClarificationCard
                      busy={approvalBusy}
                      message={message}
                      onSubmit={onClarificationSubmit}
                    />
                  )}
                </article>
              </div>
            );
          })
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

function isClarificationMessage(message: ContainerMessage) {
  return (
    message.lane === "task" &&
    message.status === "waiting_user" &&
    message.title === "Clarification requested"
  );
}

function artifactForMessage(detail: TaskDetail | undefined, message: ContainerMessage) {
  const body = message.body_json as { data?: Record<string, unknown>; artifact_path?: unknown };
  const path = body.data?.artifact_path || body.data?.archive_path || body.artifact_path;
  if (typeof path !== "string") return detail?.artifacts[0];
  return detail?.artifacts.find((artifact) => artifact.path === path) || detail?.artifacts[0];
}
