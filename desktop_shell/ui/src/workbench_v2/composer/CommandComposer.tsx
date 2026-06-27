import { Button, Textarea } from "@fluentui/react-components";
import { DismissCircleRegular, SendRegular } from "@fluentui/react-icons";

import { parseSlashCommand } from "./slashCommandParser";
import { flyoutForDraft } from "./tokenParser";
import { useI18n } from "../i18n/i18n";
import { useWorkbenchUiStore, type ContainerMode } from "../state/uiStore";

interface CommandComposerProps {
  containerId: string | null;
  scopeId: string | null;
  disabled?: boolean;
  forceCloseDisabled?: boolean;
  forceCloseVisible?: boolean;
  onForceClose?(): void;
  onSubmit(value: string, modeAtSubmit: ContainerMode): void;
}

export function CommandComposer({
  containerId,
  scopeId,
  disabled,
  forceCloseDisabled,
  forceCloseVisible,
  onForceClose,
  onSubmit
}: CommandComposerProps) {
  const t = useI18n();
  const draft = useWorkbenchUiStore((state) => state.draft(scopeId));
  const mode = useWorkbenchUiStore((state) => state.mode(scopeId));
  const setDraft = useWorkbenchUiStore((state) => state.setDraft);
  const setMode = useWorkbenchUiStore((state) => state.setMode);
  const setOpenFlyout = useWorkbenchUiStore((state) => state.setOpenFlyout);

  function updateDraft(value: string) {
    setDraft(scopeId, value);
    setOpenFlyout(flyoutForDraft(value));
  }

  function submit() {
    const value = draft.trim();
    if (!value || disabled) return;
    if (value.startsWith("/")) {
      const command = parseSlashCommand(value);
      if (command.mode) {
        setMode(scopeId, command.mode);
        setDraft(scopeId, "");
      }
      if (command.command === "model") setOpenFlyout("model");
      if (command.command === "context") setOpenFlyout("context");
      return;
    }
    onSubmit(value, mode);
    setDraft(scopeId, "");
    setOpenFlyout(null);
  }

  return (
    <div className="sn-composer-stack">
      {forceCloseVisible && (
        <div className="sn-composer-force-close">
          <Button
            appearance="secondary"
            disabled={forceCloseDisabled}
            icon={<DismissCircleRegular />}
            onClick={onForceClose}
            size="small"
          >
            强制关闭当前运行
          </Button>
        </div>
      )}
      <div className="sn-composer" data-mode={mode}>
        <Textarea
          className="sn-composer-input"
          value={draft}
          disabled={!containerId || disabled}
          placeholder={mode === "chat" ? t("composer.chatPlaceholder") : t("composer.taskPlaceholder")}
          onChange={(_, data) => updateDraft(data.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter" && !event.shiftKey) {
              event.preventDefault();
              submit();
            }
            if (event.key === "Escape") {
              setOpenFlyout(null);
            }
          }}
        />
        <Button
          appearance="primary"
          icon={<SendRegular />}
          disabled={!draft.trim() || !containerId || disabled}
          onClick={submit}
        />
      </div>
    </div>
  );
}
