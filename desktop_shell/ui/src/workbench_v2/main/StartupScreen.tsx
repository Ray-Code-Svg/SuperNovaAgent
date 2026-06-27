import type { CSSProperties } from "react";
import { Button } from "@fluentui/react-components";

import { useI18n, type MessageKey } from "../i18n/i18n";

export interface StartupStage {
  labelKey: MessageKey;
  status: "pending" | "active" | "complete";
}

interface StartupScreenProps {
  stages: StartupStage[];
  error?: string;
  ready: boolean;
  onEnter(): void;
  onRetry(): void;
}

export function StartupScreen({ stages, error, ready, onEnter, onRetry }: StartupScreenProps) {
  const t = useI18n();
  const completedStages = stages.filter((stage) => stage.status === "complete").length;
  const progress = ready ? 100 : Math.max(12, Math.round((completedStages / Math.max(stages.length, 1)) * 100));
  const statusLabel = error
    ? t("startup.statusError")
    : ready
      ? t("startup.statusReady")
      : t("startup.statusLoading");
  return (
    <div className="sn-startup-screen" role="status" aria-live="polite">
      <div className="sn-startup-mark" role="img" aria-label="SuperNova">
        <span className="sn-startup-glyph-wrap" aria-hidden="true">
          <img className="sn-startup-glyph" src="/supernova-startup-glyph-reference.png?v=rc0-reference-glyph-feather" alt="" />
        </span>
      </div>
      <div
        className="sn-startup-status-glass"
        data-ready={ready}
        style={{ "--sn-startup-progress": `${progress}%` } as CSSProperties}
      >
        <span className="sn-startup-status-dot" />
        <strong>{statusLabel}</strong>
      </div>
      {error && (
        <div className="sn-startup-error">
          <span>{error}</span>
          <Button appearance="primary" onClick={onRetry}>{t("startup.retry")}</Button>
        </div>
      )}
      {ready && !error && (
        <div className="sn-startup-actions">
          <Button appearance="primary" onClick={onEnter}>{t("startup.enter")}</Button>
        </div>
      )}
    </div>
  );
}
