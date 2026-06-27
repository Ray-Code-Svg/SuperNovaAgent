import { Badge } from "@fluentui/react-components";
import { ClipboardTaskRegular } from "@fluentui/react-icons";
import type { TaskRecord } from "../../protocol/generated/types";

import { useI18n } from "../i18n/i18n";

interface TaskStatusRailProps {
  tasks: TaskRecord[];
  activeTaskId?: string | null;
  onSelectTask(taskId: string | null): void;
}

export function TaskStatusRail({ tasks, activeTaskId, onSelectTask }: TaskStatusRailProps) {
  const t = useI18n();
  const running = tasks.filter((task) => task.status === "running").length;
  const blocked = tasks.reduce((count, task) => count + (task.badges?.blocked || (task.status === "blocked" ? 1 : 0)), 0);
  const artifacts = tasks.reduce((count, task) => count + (task.badges?.artifact_ready || 0), 0);
  return (
    <aside className="sn-task-rail">
      <ClipboardTaskRegular />
      <div className="sn-task-rail-title">{t("taskRail.title")}</div>
      <div className="sn-task-rail-badges">
        <Badge>{tasks.length}</Badge>
        <Badge appearance={running ? "filled" : "outline"}>{running}</Badge>
        <Badge appearance={blocked ? "filled" : "outline"}>{blocked}</Badge>
        <Badge appearance={artifacts ? "filled" : "outline"}>{artifacts}</Badge>
      </div>
      <div className="sn-task-rail-history">
        {tasks.length === 0 ? (
          <span>{t("taskRail.noHistory")}</span>
        ) : (
          tasks.slice(0, 8).map((task) => (
            <TaskHistoryButton
              activeTaskId={activeTaskId}
              artifactLabel={t("taskRail.artifact")}
              key={task.task_id}
              onSelectTask={onSelectTask}
              task={task}
            />
          ))
        )}
      </div>
    </aside>
  );
}

function TaskHistoryButton({
  activeTaskId,
  artifactLabel,
  onSelectTask,
  task
}: {
  activeTaskId?: string | null;
  artifactLabel: string;
  onSelectTask(taskId: string | null): void;
  task: TaskRecord;
}) {
  const artifacts = task.badges?.artifact_ready || 0;
  return (
    <button
      className="sn-task-history-item"
      data-active={task.task_id === activeTaskId}
      onClick={() => onSelectTask(task.task_id === activeTaskId ? null : task.task_id)}
      type="button"
    >
      <span>{task.title}</span>
      {artifacts ? (
        <small>
          {`${artifacts} ${artifactLabel}`}
        </small>
      ) : null}
    </button>
  );
}
