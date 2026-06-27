import { Dialog, DialogBody, DialogContent, DialogSurface, DialogTitle, Tab, TabList } from "@fluentui/react-components";
import type { AppSettings, DiagnosticsSnapshot } from "../../protocol/generated/types";
import { useI18n } from "../i18n/i18n";
import { OnboardingGuide } from "../onboarding/OnboardingGuide";
import type { SettingsTab } from "../state/uiStore";
import { AppearanceSettings } from "./AppearanceSettings";
import { ArchivedContainerManager } from "./ArchivedContainerManager";
import { DataManagerSettings } from "./DataManagerSettings";
import { ProviderApiSettings } from "./ProviderApiSettings";

interface SystemSettingsDialogProps {
  open: boolean;
  selectedTab: SettingsTab;
  settings?: AppSettings;
  diagnostics?: DiagnosticsSnapshot;
  onOpenChange(open: boolean): void;
  onTabChange(tab: SettingsTab): void;
  onOpenProviderSettings(): void;
  onOpenAppearanceSettings(): void;
}

export function SystemSettingsDialog({
  open,
  selectedTab,
  settings,
  diagnostics,
  onOpenChange,
  onTabChange,
  onOpenProviderSettings,
  onOpenAppearanceSettings
}: SystemSettingsDialogProps) {
  const t = useI18n();
  return (
    <Dialog open={open} onOpenChange={(_, data) => onOpenChange(data.open)}>
      <DialogSurface className="sn-settings-dialog">
        <DialogBody>
          <DialogTitle>{t("settings.title")}</DialogTitle>
          <DialogContent>
            <div className="sn-settings-layout">
              <TabList selectedValue={selectedTab} onTabSelect={(_, data) => onTabChange(data.value as SettingsTab)} vertical>
                <Tab value="guide">{t("settings.guide")}</Tab>
                <Tab value="provider">{t("settings.provider")}</Tab>
                <Tab value="data">{t("settings.data")}</Tab>
                <Tab value="appearance">{t("settings.appearance")}</Tab>
                <Tab value="runtime">{t("settings.runtime")}</Tab>
                <Tab value="diagnostics">{t("settings.diagnostics")}</Tab>
                <Tab value="archived">{t("settings.archived")}</Tab>
              </TabList>
              <div className="sn-settings-content">
                {selectedTab === "guide" && (
                  <OnboardingGuide
                    settings={settings}
                    variant="settings"
                    onOpenProviderSettings={onOpenProviderSettings}
                    onOpenAppearanceSettings={onOpenAppearanceSettings}
                  />
                )}
                {selectedTab === "provider" && <ProviderApiSettings settings={settings} />}
                {selectedTab === "data" && <DataManagerSettings settings={settings} />}
                {selectedTab === "appearance" && <AppearanceSettings settings={settings} />}
                {selectedTab === "runtime" && (
                  <div className="sn-settings-pane">
                    <div className="sn-settings-row"><strong>{t("settings.runtime")}</strong><span>{diagnostics?.runtime_layer}</span></div>
                    <div className="sn-settings-row"><strong>{t("settings.kernel")}</strong><span>{diagnostics?.kernel_layer}</span></div>
                  </div>
                )}
                {selectedTab === "diagnostics" && (
                  <div className="sn-settings-pane">
                    <div className="sn-settings-row"><strong>{t("settings.status")}</strong><span>{diagnostics?.runtime_status}</span></div>
                    <div className="sn-settings-row"><strong>{t("settings.protocol")}</strong><span>{diagnostics?.protocol_version}</span></div>
                  </div>
                )}
                {selectedTab === "archived" && <ArchivedContainerManager />}
              </div>
            </div>
          </DialogContent>
        </DialogBody>
      </DialogSurface>
    </Dialog>
  );
}
