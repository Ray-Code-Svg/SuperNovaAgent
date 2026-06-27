import type { AppSettings } from "../../protocol/generated/types";
import { useQuery } from "@tanstack/react-query";

import { useI18n } from "../i18n/i18n";
import { listWorkspaces } from "../protocol/workspaceQueries";

export function DataManagerSettings({ settings }: { settings?: AppSettings }) {
  const t = useI18n();
  const workspaces = useQuery({ queryKey: ["workspaces"], queryFn: listWorkspaces });
  const workspaceItems = workspaces.data?.items || [];

  return (
    <div className="sn-settings-pane">
      <div className="sn-settings-row">
        <strong>{t("settings.config")}</strong>
        <span>{settings?.data_paths.app_config_root || ""}</span>
      </div>
      <div className="sn-settings-row">
        <strong>{t("settings.state")}</strong>
        <span>{settings?.data_paths.app_state_root || ""}</span>
      </div>
      <div className="sn-settings-row">
        <strong>{t("settings.registry")}</strong>
        <span>{settings?.data_paths.workspace_registry_path || ""}</span>
      </div>
      <div className="sn-settings-note">
        {t("settings.dataManagerNote")}
      </div>
      <div className="sn-data-registry">
        <strong>{t("settings.registeredWorkspaces")}</strong>
        {workspaces.isLoading && <span>{t("settings.loadingWorkspaces")}</span>}
        {workspaces.error && (
          <span>{workspaces.error instanceof Error ? workspaces.error.message : String(workspaces.error)}</span>
        )}
        {!workspaces.isLoading && !workspaces.error && workspaceItems.length === 0 && (
          <span>{t("settings.noWorkspaces")}</span>
        )}
        {workspaceItems.map((workspace) => (
          <div className="sn-data-registry-row" key={workspace.workspace_uid}>
            <span>{workspace.display_name}</span>
            <small>{workspace.workspace_root}</small>
          </div>
        ))}
      </div>
    </div>
  );
}
