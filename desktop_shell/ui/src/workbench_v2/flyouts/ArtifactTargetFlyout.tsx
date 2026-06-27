import { Badge, Button } from "@fluentui/react-components";
import { DatabaseRegular } from "@fluentui/react-icons";
import { useQuery } from "@tanstack/react-query";
import type { ArtifactTargetOption } from "../../protocol/generated/types";

import { useI18n } from "../i18n/i18n";
import { listArtifactTargets } from "../protocol/artifactQueries";
import type { ArtifactTargetSelection } from "../state/uiStore";
import { WorkbenchFlyout } from "./WorkbenchFlyout";

export function ArtifactTargetFlyout({
  containerId,
  selectedSelection,
  onSelectTarget
}: {
  containerId: string | null;
  selectedSelection?: ArtifactTargetSelection | null;
  onSelectTarget(selection: ArtifactTargetSelection | null): void;
}) {
  const t = useI18n();
  const targets = useQuery({
    queryKey: ["artifact-targets", containerId],
    queryFn: () => listArtifactTargets(containerId || ""),
    enabled: Boolean(containerId)
  });
  const items = targets.data?.items.filter((item) => item.user_visible) || [];

  function selectTarget(item: ArtifactTargetOption) {
    onSelectTarget({
      targetId: item.target_id,
      targetDir: item.target_dir,
      label: item.label
    });
  }

  return (
    <WorkbenchFlyout title={t("artifact.title")}>
      {targets.isLoading && <div className="sn-flyout-empty">{t("artifact.loading")}</div>}
      {!targets.isLoading && items.length === 0 && <div className="sn-flyout-empty">
        <DatabaseRegular />
        <span>{t("artifact.none")}</span>
      </div>}
      <div className="sn-flyout-list">
        {items.map((item) => (
          <section
            className="sn-artifact-target-card"
            data-selected={item.target_id === selectedSelection?.targetId}
            key={item.target_id}
          >
            <button
              className="sn-target-option sn-source-option"
              onClick={() => selectTarget(item)}
              type="button"
            >
              <div>
                <strong>{item.label}</strong>
                <span>{item.target_dir}</span>
              </div>
              <Badge appearance={item.target_id === selectedSelection?.targetId ? "filled" : "outline"}>
                {item.target_id === selectedSelection?.targetId ? t("common.selected") : t("common.directory")}
              </Badge>
            </button>
          </section>
        ))}
      </div>
      <div className="sn-config-actions">
        <Button disabled={!selectedSelection} onClick={() => onSelectTarget(null)}>
          {t("artifact.clear")}
        </Button>
      </div>
    </WorkbenchFlyout>
  );
}
