import { useState } from "react";
import { Button, Input, Tooltip } from "@fluentui/react-components";
import {
  AddCircleRegular,
  ArchiveRegular,
  ChevronDownRegular,
  ChevronRightRegular,
  EditRegular,
  FolderRegular,
  SettingsRegular
} from "@fluentui/react-icons";

import type { ContainerRecord, RuntimeMeta, WorkspaceRecord } from "../../protocol/generated/types";
import { useI18n } from "../i18n/i18n";
import type { WorkspaceContainerLoadState } from "../main/railData";
import { useWorkbenchUiStore } from "../state/uiStore";

interface ProjectsRailProps {
  runtime?: RuntimeMeta;
  workspaces: WorkspaceRecord[];
  containersByWorkspace: Record<string, ContainerRecord[]>;
  containerStateByWorkspace?: Record<string, WorkspaceContainerLoadState>;
  activeWorkspaceId?: string | null;
  activeContainerId: string | null;
  onAddWorkspace(): void;
  onAddContainer(workspaceUid: string): void;
  onSelectWorkspace(workspaceUid: string): void;
  onArchiveWorkspace(workspaceUid: string): void;
  onSelectContainer(workspaceUid: string, containerId: string): void;
  onRenameContainer(workspaceUid: string, containerId: string, title: string): void;
  onArchiveContainer(workspaceUid: string, containerId: string): void;
  onOpenSettings(): void;
}

export function ProjectsRail({
  runtime,
  workspaces,
  containersByWorkspace,
  containerStateByWorkspace = {},
  activeWorkspaceId,
  activeContainerId,
  onAddWorkspace,
  onAddContainer,
  onSelectWorkspace,
  onArchiveWorkspace,
  onSelectContainer,
  onRenameContainer,
  onArchiveContainer,
  onOpenSettings
}: ProjectsRailProps) {
  const t = useI18n();
  const expandedWorkspaceById = useWorkbenchUiStore((state) => state.expandedWorkspaceById);
  const setWorkspaceExpanded = useWorkbenchUiStore((state) => state.setWorkspaceExpanded);
  const [editingContainer, setEditingContainer] = useState<{ workspaceUid: string; containerId: string } | null>(null);
  const [draftTitle, setDraftTitle] = useState("");
  const workspaceRecords = workspaces;

  function workspaceExpanded(workspaceUid: string) {
    return expandedWorkspaceById[workspaceUid] ?? workspaceUid === activeWorkspaceId;
  }

  function startRename(workspaceUid: string, container: ContainerRecord) {
    setEditingContainer({ workspaceUid, containerId: container.container_id });
    setDraftTitle(container.title);
  }

  function commitRename() {
    if (!editingContainer) return;
    const title = draftTitle.trim();
    if (title) {
      onRenameContainer(editingContainer.workspaceUid, editingContainer.containerId, title);
    }
    setEditingContainer(null);
    setDraftTitle("");
  }

  function cancelRename() {
    setEditingContainer(null);
    setDraftTitle("");
  }

  return (
    <aside className="sn-projects-rail">
      <div className="sn-rail-title">
        <span>{t("projects.title")}</span>
        <Tooltip content={t("projects.addWorkspace")} relationship="label">
          <Button appearance="subtle" icon={<AddCircleRegular />} onClick={onAddWorkspace} />
        </Tooltip>
      </div>
      <div className="sn-workspace-list">
        {workspaceRecords.map((workspace) => {
          const expanded = workspaceExpanded(workspace.workspace_uid);
          const containers = containersByWorkspace[workspace.workspace_uid] || [];
          const containerState = containerStateByWorkspace[workspace.workspace_uid] || "ready";
          return (
            <section className="sn-workspace-group" key={workspace.workspace_uid}>
              <div className="sn-workspace-header">
                <Tooltip content={expanded ? t("projects.collapse") : t("projects.expand")} relationship="label">
                  <Button
                    appearance="subtle"
                    size="small"
                    icon={expanded ? <ChevronDownRegular /> : <ChevronRightRegular />}
                    onClick={() => setWorkspaceExpanded(workspace.workspace_uid, !expanded)}
                  />
                </Tooltip>
                <button
                  className="sn-workspace-select"
                  data-active={workspace.workspace_uid === activeWorkspaceId}
                  onClick={() => {
                    setWorkspaceExpanded(workspace.workspace_uid, true);
                    onSelectWorkspace(workspace.workspace_uid);
                  }}
                  type="button"
                >
                  <FolderRegular />
                  <strong>{workspace.display_name}</strong>
                </button>
                <Tooltip content={t("projects.addContainer")} relationship="label">
                  <Button
                    appearance="subtle"
                    size="small"
                    icon={<AddCircleRegular />}
                    onClick={() => onAddContainer(workspace.workspace_uid)}
                  />
                </Tooltip>
                <Tooltip content={t("projects.archiveWorkspace")} relationship="label">
                  <Button
                    appearance="subtle"
                    size="small"
                    disabled={workspaceRecords.length <= 1}
                    icon={<ArchiveRegular />}
                    onClick={() => onArchiveWorkspace(workspace.workspace_uid)}
                  />
                </Tooltip>
              </div>
              {expanded && (
                <div className="sn-container-list">
                  {containers.length === 0 && (
                    <span className="sn-container-empty">
                      {containerState === "loading" && t("common.loading")}
                      {containerState === "error" && t("status.unavailable")}
                      {containerState === "ready" && t("projects.noContainers")}
                    </span>
                  )}
                  {containers.map((container) => {
                    const editing = editingContainer?.containerId === container.container_id;
                    return (
                      <div className="sn-container-row" data-active={container.container_id === activeContainerId} key={container.container_id}>
                        {editing ? (
                          <form
                            className="sn-container-rename"
                            onSubmit={(event) => {
                              event.preventDefault();
                              commitRename();
                            }}
                          >
                            <Input
                              autoFocus
                              size="small"
                              value={draftTitle}
                              onBlur={commitRename}
                              onChange={(_, data) => setDraftTitle(data.value)}
                              onKeyDown={(event) => {
                                if (event.key === "Escape") {
                                  event.preventDefault();
                                  cancelRename();
                                }
                              }}
                            />
                          </form>
                        ) : (
                          <button
                            className="sn-container-select"
                            onClick={() => onSelectContainer(workspace.workspace_uid, container.container_id)}
                            type="button"
                          >
                            <span>{container.title}</span>
                          </button>
                        )}
                        <Tooltip content={t("projects.renameContainer")} relationship="label">
                          <Button
                            appearance="subtle"
                            size="small"
                            icon={<EditRegular />}
                            onClick={() => startRename(workspace.workspace_uid, container)}
                          />
                        </Tooltip>
                        <Tooltip content={t("projects.archiveContainer")} relationship="label">
                          <Button
                            appearance="subtle"
                            size="small"
                            icon={<ArchiveRegular />}
                            onClick={() => onArchiveContainer(workspace.workspace_uid, container.container_id)}
                          />
                        </Tooltip>
                      </div>
                    );
                  })}
                </div>
              )}
            </section>
          );
        })}
      </div>
      <div className="sn-rail-footer">
        <Tooltip content={t("settings.title")} relationship="label">
          <Button appearance="subtle" icon={<SettingsRegular />} onClick={onOpenSettings} />
        </Tooltip>
      </div>
    </aside>
  );
}
