import { useI18n } from "../i18n/i18n";

interface RuntimeStatusBarProps {
  runtimeReady: boolean;
  workspaceName?: string | null;
}

export function RuntimeStatusBar({ runtimeReady, workspaceName }: RuntimeStatusBarProps) {
  const t = useI18n();
  return (
    <div className="sn-runtime-strip">
      {workspaceName ? <strong>{workspaceName}</strong> : null}
      <span>{runtimeReady ? t("startup.ready") : t("title.status.loading")}</span>
    </div>
  );
}
