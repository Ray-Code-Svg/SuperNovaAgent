import { Button } from "@fluentui/react-components";
import { DeleteRegular, HistoryRegular } from "@fluentui/react-icons";
import { useMutation, useQuery } from "@tanstack/react-query";

import {
  deleteContainer,
  listArchivedContainers,
  restoreContainer
} from "../protocol/containerQueries";
import { useI18n } from "../i18n/i18n";
import { queryClient } from "../protocol/queryClient";

export function ArchivedContainerManager() {
  const t = useI18n();
  const archived = useQuery({ queryKey: ["containers", "archived"], queryFn: listArchivedContainers });
  const restoreMutation = useMutation({
    mutationFn: restoreContainer,
    onSuccess: () => refreshContainerQueries()
  });
  const deleteMutation = useMutation({
    mutationFn: deleteContainer,
    onSuccess: () => refreshContainerQueries()
  });

  if (archived.isLoading) {
    return (
      <div className="sn-settings-pane">
        <span>{t("settings.archivedLoading")}</span>
      </div>
    );
  }

  if (archived.error) {
    return (
      <div className="sn-settings-pane">
        <span>{archived.error instanceof Error ? archived.error.message : String(archived.error)}</span>
      </div>
    );
  }

  const items = archived.data?.items || [];

  return (
    <div className="sn-settings-pane">
      <div className="sn-settings-note">
        {t("settings.archivedDeleteNote")}
      </div>
      {items.length === 0 && <span>{t("settings.noArchived")}</span>}
      {items.map((container) => (
        <div className="sn-archived-row" key={container.container_id}>
          <div>
            <strong>{container.title}</strong>
            <span>{container.container_id}</span>
          </div>
          <div className="sn-archived-actions">
            <Button
              appearance="subtle"
              icon={<HistoryRegular />}
              onClick={() => restoreMutation.mutate(container.container_id)}
            >
              {t("settings.restore")}
            </Button>
            <Button
              appearance="subtle"
              icon={<DeleteRegular />}
              onClick={() => {
                if (window.confirm(t("settings.deleteArchivedConfirm").replace("{title}", container.title))) {
                  deleteMutation.mutate(container.container_id);
                }
              }}
            >
              {t("settings.markDeleted")}
            </Button>
          </div>
        </div>
      ))}
    </div>
  );
}

function refreshContainerQueries() {
  queryClient.invalidateQueries({ queryKey: ["containers"] });
  queryClient.invalidateQueries({ queryKey: ["containers", "archived"] });
}
