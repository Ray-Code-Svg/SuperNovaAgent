import type { ReactNode } from "react";
import { Button } from "@fluentui/react-components";
import { DismissRegular } from "@fluentui/react-icons";

import { useI18n } from "../i18n/i18n";
import { useWorkbenchUiStore } from "../state/uiStore";

interface WorkbenchFlyoutProps {
  title: string;
  children: ReactNode;
}

export function WorkbenchFlyout({ title, children }: WorkbenchFlyoutProps) {
  const t = useI18n();
  const setOpenFlyout = useWorkbenchUiStore((state) => state.setOpenFlyout);
  return (
    <div className="sn-flyout" role="dialog" aria-label={title}>
      <div className="sn-flyout-header">
        <span>{title}</span>
        <Button
          appearance="subtle"
          aria-label={t("flyout.close")}
          size="small"
          icon={<DismissRegular />}
          onClick={() => setOpenFlyout(null)}
        />
      </div>
      {children}
    </div>
  );
}
