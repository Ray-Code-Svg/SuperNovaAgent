import type { MouseEvent } from "react";
import { Button, Tooltip } from "@fluentui/react-components";
import { ArrowMaximizeRegular, ArrowSyncRegular, PowerRegular, SubtractRegular } from "@fluentui/react-icons";
import { getCurrentWindow } from "@tauri-apps/api/window";

import { useI18n } from "../i18n/i18n";
import { invokeShell } from "../protocol/runtimeClient";

interface FluentTitleBarProps {
  status: string;
  onRefresh(): void;
}

export function FluentTitleBar({ status, onRefresh }: FluentTitleBarProps) {
  const t = useI18n();

  function handleTitleBarMouseDown(event: MouseEvent<HTMLElement>) {
    if (event.button !== 0) return;
    const target = event.target as HTMLElement | null;
    if (target?.closest("button,a,input,textarea,select,[role='button'],[data-sn-no-window-drag='true']")) return;
    if (!("__TAURI_INTERNALS__" in window)) return;
    void getCurrentWindow().startDragging().catch((error: unknown) => {
      console.error("Failed to start window drag", error);
    });
  }

  return (
    <header className="sn-titlebar" data-tauri-drag-region onMouseDown={handleTitleBarMouseDown}>
      <div className="sn-titlebar-caption" data-tauri-drag-region>SuperNova</div>
      <div className="sn-titlebar-status" data-tauri-drag-region>{status}</div>
      <div className="sn-titlebar-actions" data-sn-no-window-drag="true">
        <Tooltip content={t("title.refresh")} relationship="label">
          <Button appearance="subtle" icon={<ArrowSyncRegular />} onClick={onRefresh} />
        </Tooltip>
        <Tooltip content={t("title.minimize")} relationship="label">
          <Button appearance="subtle" icon={<SubtractRegular />} onClick={() => invokeShell("window_minimize")} />
        </Tooltip>
        <Tooltip content={t("title.maximize")} relationship="label">
          <Button appearance="subtle" icon={<ArrowMaximizeRegular />} onClick={() => invokeShell("window_maximize")} />
        </Tooltip>
        <Tooltip content={t("title.quit")} relationship="label">
          <Button appearance="subtle" icon={<PowerRegular />} onClick={() => invokeShell("app_quit")} />
        </Tooltip>
      </div>
    </header>
  );
}
