import type { ContainerMessage, TaskDetail } from "../../protocol/generated/types";
import { TaskMessageStream } from "./TaskMessageStream";

interface AgentTaskSurfaceProps {
  messages: ContainerMessage[];
  selectedTaskId?: string | null;
  selectedTaskDetail?: TaskDetail;
  approvalBusy?: boolean;
  onClarificationSubmit(taskId: string, input: string): void;
}

export function AgentTaskSurface({
  approvalBusy,
  messages,
  selectedTaskId,
  selectedTaskDetail,
  onClarificationSubmit
}: AgentTaskSurfaceProps) {
  return (
    <TaskMessageStream
      approvalBusy={approvalBusy}
      messages={messages}
      selectedTaskId={selectedTaskId}
      selectedTaskDetail={selectedTaskDetail}
      onClarificationSubmit={onClarificationSubmit}
    />
  );
}
