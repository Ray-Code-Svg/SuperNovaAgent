import type { AppSettings } from "../../protocol/generated/types";

export const ONBOARDING_GUIDE_VERSION = "rc0-batch2b-v1";

export interface OnboardingAutoOpenInput {
  startupComplete: boolean;
  settingsReady: boolean;
  seenVersion: string | null | undefined;
  open: boolean;
}

export interface ProviderGuideState {
  providerId: string;
  configured: boolean;
  configuredCount: number;
  missingCount: number;
  validationStatus: string | null;
  status: "unknown" | "configured" | "missing";
}

export function shouldAutoOpenOnboardingGuide({
  startupComplete,
  settingsReady,
  seenVersion,
  open
}: OnboardingAutoOpenInput) {
  return startupComplete && settingsReady && !open && seenVersion !== ONBOARDING_GUIDE_VERSION;
}

export function summarizeProviderGuideState(settings?: AppSettings): ProviderGuideState {
  const providers = settings?.provider_api.providers || [];
  const primaryProvider = providers.find((provider) => provider.provider === "deepseek") || providers[0];
  const configuredCount = providers.filter((provider) => provider.api_key_configured).length;
  const missingCount = providers.length ? providers.length - configuredCount : 0;
  const configured = Boolean(primaryProvider?.api_key_configured);

  return {
    providerId: primaryProvider?.provider || "deepseek",
    configured,
    configuredCount,
    missingCount,
    validationStatus: primaryProvider?.validation_status || null,
    status: primaryProvider ? (configured ? "configured" : "missing") : "unknown"
  };
}
