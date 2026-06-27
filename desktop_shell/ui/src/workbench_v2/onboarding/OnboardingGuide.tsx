import type { ReactNode } from "react";
import { Badge, Button } from "@fluentui/react-components";
import {
  BookOpenRegular,
  ChatRegular,
  CheckmarkCircleRegular,
  DatabaseRegular,
  DocumentAddRegular,
  KeyRegular,
  SettingsRegular,
  TaskListSquareLtrRegular,
  WarningRegular,
  WindowRegular
} from "@fluentui/react-icons";

import type { AppSettings } from "../../protocol/generated/types";
import { useI18n, type MessageKey } from "../i18n/i18n";
import { openExternalUrl } from "../protocol/runtimeClient";
import { summarizeProviderGuideState } from "./onboardingState";

interface OnboardingGuideProps {
  settings?: AppSettings;
  variant?: "modal" | "settings";
  onOpenProviderSettings?(): void;
  onOpenAppearanceSettings?(): void;
  onDismiss?(): void;
}

interface GuideCard {
  icon: ReactNode;
  titleKey: MessageKey;
  bodyKey: MessageKey;
  itemKeys?: MessageKey[];
}

const GUIDE_CARDS: GuideCard[] = [
  {
    icon: <WindowRegular />,
    titleKey: "onboarding.workspaceTitle",
    bodyKey: "onboarding.workspaceBody"
  },
  {
    icon: <ChatRegular />,
    titleKey: "onboarding.chatTaskTitle",
    bodyKey: "onboarding.chatTaskBody"
  },
  {
    icon: <TaskListSquareLtrRegular />,
    titleKey: "onboarding.taskCapabilityTitle",
    bodyKey: "onboarding.taskCapabilityBody",
    itemKeys: [
      "onboarding.taskCapabilityRead",
      "onboarding.taskCapabilityBatch",
      "onboarding.taskCapabilityWrite",
      "onboarding.taskCapabilitySpecialized",
      "onboarding.taskCapabilityCodeCommand",
      "onboarding.taskCapabilityBoundary"
    ]
  },
  {
    icon: <DocumentAddRegular />,
    titleKey: "onboarding.sourceArtifactTitle",
    bodyKey: "onboarding.sourceArtifactBody"
  },
  {
    icon: <SettingsRegular />,
    titleKey: "onboarding.modelContextTitle",
    bodyKey: "onboarding.modelContextBody"
  },
  {
    icon: <DatabaseRegular />,
    titleKey: "onboarding.diagnosticsTitle",
    bodyKey: "onboarding.diagnosticsBody"
  }
];

const PRODUCT_INTRO_ITEMS: MessageKey[] = [
  "onboarding.productPointChat",
  "onboarding.productPointTask",
  "onboarding.productPointContainer"
];

const DEEPSEEK_API_KEYS_URL = "https://platform.deepseek.com/api_keys";

export function OnboardingGuide({
  settings,
  variant = "settings",
  onOpenProviderSettings,
  onOpenAppearanceSettings,
  onDismiss
}: OnboardingGuideProps) {
  const t = useI18n();
  const provider = summarizeProviderGuideState(settings);
  const providerConfigured = provider.status === "configured";
  const providerLabel = providerConfigured
    ? t("onboarding.providerConfigured")
    : provider.status === "unknown"
      ? t("onboarding.providerUnknown")
      : t("onboarding.providerMissing");

  return (
    <div className="sn-onboarding-guide" data-variant={variant}>
      <header className="sn-onboarding-header">
        <span className="sn-onboarding-header-icon" aria-hidden="true">
          <BookOpenRegular />
        </span>
        <div>
          <span className="sn-onboarding-eyebrow">{t("onboarding.eyebrow")}</span>
          <h2>{t("onboarding.title")}</h2>
          <p>{t("onboarding.subtitle")}</p>
        </div>
      </header>

      <section className="sn-onboarding-product" aria-label={t("onboarding.productTitle")}>
        <div className="sn-onboarding-product-main">
          <span className="sn-onboarding-product-icon" aria-hidden="true">
            <WindowRegular />
          </span>
          <div>
            <h3>{t("onboarding.productTitle")}</h3>
            <p>{t("onboarding.productBody")}</p>
          </div>
        </div>
        <div className="sn-onboarding-product-points">
          {PRODUCT_INTRO_ITEMS.map((itemKey) => (
            <div key={itemKey}>
              <CheckmarkCircleRegular aria-hidden="true" />
              <span>{t(itemKey)}</span>
            </div>
          ))}
        </div>
      </section>

      <section className="sn-onboarding-provider" aria-label={t("onboarding.providerTitle")}>
        <div className="sn-onboarding-provider-title">
          <KeyRegular aria-hidden="true" />
          <div>
            <h3>{t("onboarding.providerTitle")}</h3>
            <p>{providerConfigured ? t("onboarding.providerBodyConfigured") : t("onboarding.providerBodyMissing")}</p>
          </div>
          <Badge appearance="tint" color={providerConfigured ? "success" : "warning"}>
            {providerLabel}
          </Badge>
        </div>
        <div className="sn-onboarding-steps">
          <div><CheckmarkCircleRegular aria-hidden="true" /> {t("onboarding.stepApiKey")}</div>
          <div><CheckmarkCircleRegular aria-hidden="true" /> {t("onboarding.stepProvider")}</div>
          <div><CheckmarkCircleRegular aria-hidden="true" /> {t("onboarding.stepLanguage")}</div>
        </div>
        <div className="sn-onboarding-url">
          <span>{t("onboarding.providerApply")}</span>
          <a
            href={DEEPSEEK_API_KEYS_URL}
            onClick={(event) => {
              event.preventDefault();
              void openExternalUrl(DEEPSEEK_API_KEYS_URL);
            }}
            rel="noreferrer"
            target="_blank"
          >
            {DEEPSEEK_API_KEYS_URL}
          </a>
        </div>
        <div className="sn-onboarding-actions">
          <Button appearance="primary" icon={<SettingsRegular />} onClick={onOpenProviderSettings}>
            {t("onboarding.providerOpenSettings")}
          </Button>
          <Button icon={<SettingsRegular />} onClick={onOpenAppearanceSettings}>
            {t("onboarding.appearanceOpenSettings")}
          </Button>
        </div>
      </section>

      <section className="sn-onboarding-checklist" aria-label={t("onboarding.checklistTitle")}>
        <div>
          <TaskListSquareLtrRegular aria-hidden="true" />
          <strong>{t("onboarding.checklistTitle")}</strong>
        </div>
        <ol>
          <li>{t("onboarding.stepWorkspace")}</li>
          <li>{t("onboarding.stepChatTask")}</li>
        </ol>
      </section>

      <section className="sn-onboarding-grid">
        {GUIDE_CARDS.map((card) => (
          <article className="sn-onboarding-card" key={card.titleKey}>
            <span className="sn-onboarding-card-icon" aria-hidden="true">{card.icon}</span>
            <h3>{t(card.titleKey)}</h3>
            <p>{t(card.bodyKey)}</p>
            {card.itemKeys?.length ? (
              <ul>
                {card.itemKeys.map((itemKey) => (
                  <li key={itemKey}>{t(itemKey)}</li>
                ))}
              </ul>
            ) : null}
          </article>
        ))}
      </section>

      {!providerConfigured && (
        <div className="sn-onboarding-note" role="note">
          <WarningRegular aria-hidden="true" />
          <span>{t("onboarding.providerStoreTest")}</span>
        </div>
      )}

      {onDismiss && (
        <div className="sn-onboarding-footer">
          <Button appearance="primary" onClick={onDismiss}>{t("onboarding.startUsing")}</Button>
        </div>
      )}
    </div>
  );
}
