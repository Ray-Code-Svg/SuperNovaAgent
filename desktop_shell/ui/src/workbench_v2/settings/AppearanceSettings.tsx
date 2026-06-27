import { Field, Radio, RadioGroup } from "@fluentui/react-components";
import { useMutation, useQueryClient } from "@tanstack/react-query";

import type { AppSettings } from "../../protocol/generated/types";
import { useI18n } from "../i18n/i18n";
import { updateSettings } from "../protocol/settingsQueries";
import { useWorkbenchUiStore, type DisplayLanguage, type DisplayTheme } from "../state/uiStore";

export function AppearanceSettings({ settings }: { settings?: AppSettings }) {
  const t = useI18n();
  const queryClient = useQueryClient();
  const setLanguage = useWorkbenchUiStore((state) => state.setLanguage);
  const setTheme = useWorkbenchUiStore((state) => state.setTheme);
  const language = settings?.appearance.language || useWorkbenchUiStore.getState().language;
  const theme = settings?.appearance.theme || useWorkbenchUiStore.getState().theme;

  const mutation = useMutation({
    mutationFn: updateSettings,
    onSuccess: (updated) => {
      queryClient.setQueryData(["settings"], updated);
      setLanguage(updated.appearance.language);
      setTheme(updated.appearance.theme);
    }
  });

  function saveAppearance(next: { language?: DisplayLanguage; theme?: DisplayTheme }) {
    if (!settings) return;
    const updated: AppSettings = {
      ...settings,
      appearance: {
        language: next.language || language,
        theme: next.theme || theme
      }
    };
    setLanguage(updated.appearance.language);
    setTheme(updated.appearance.theme);
    mutation.mutate(updated);
  }

  return (
    <div className="sn-settings-pane">
      <Field label={t("settings.language")}>
        <RadioGroup
          layout="horizontal"
          value={language}
          onChange={(_, data) => saveAppearance({ language: data.value as DisplayLanguage })}
        >
          <Radio value="zh-CN" label={t("settings.language.zh")} disabled={!settings || mutation.isPending} />
          <Radio value="en-US" label={t("settings.language.en")} disabled={!settings || mutation.isPending} />
        </RadioGroup>
      </Field>
      <Field label={t("settings.theme")}>
        <RadioGroup
          layout="horizontal"
          value={theme}
          onChange={(_, data) => saveAppearance({ theme: data.value as DisplayTheme })}
        >
          <Radio value="light" label={t("settings.theme.light")} disabled={!settings || mutation.isPending} />
          <Radio value="dark" label={t("settings.theme.dark")} disabled={!settings || mutation.isPending} />
        </RadioGroup>
      </Field>
      <div className="sn-settings-note">{t("settings.applyNote")}</div>
      {mutation.error && (
        <div className="sn-inline-error">
          {mutation.error instanceof Error ? mutation.error.message : String(mutation.error)}
        </div>
      )}
    </div>
  );
}
