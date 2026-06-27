import { Badge } from "@fluentui/react-components";
import type { ArtifactRecord } from "../../protocol/generated/types";
import { useI18n } from "../i18n/i18n";
import { messageStatusLabel } from "../rendering/messageDisplay";

export function ArtifactReadyCard({ artifact }: { artifact?: ArtifactRecord }) {
  const t = useI18n();
  return (
    <div className="sn-artifact-card">
      <strong>{artifact?.title || t("message.artifact")}</strong>
      {artifact?.path && <code>{artifact.path}</code>}
      {artifact?.capability_id && <small>{artifact.capability_id}</small>}
      {artifact?.kind && <small>{artifact.kind}</small>}
      {artifact?.receipt_ref && <code>{artifact.receipt_ref}</code>}
      <Badge appearance={artifact?.verified ? "filled" : "outline"}>
        {messageStatusLabel(artifact?.verified ? "verified" : artifact?.status || "ready", t)}
      </Badge>
    </div>
  );
}
