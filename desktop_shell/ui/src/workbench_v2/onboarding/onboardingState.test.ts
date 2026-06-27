import { describe, expect, it } from "vitest";

import type { AppSettings, ProviderApiRecord } from "../../protocol/generated/types";
import { ONBOARDING_GUIDE_VERSION, shouldAutoOpenOnboardingGuide, summarizeProviderGuideState } from "./onboardingState";

describe("onboarding state helpers", () => {
  it("summarizes missing and configured provider state without mutating settings", () => {
    const missingSettings = appSettings([{ provider: "deepseek", api_key_configured: false }]);
    const configuredSettings = appSettings([{
      provider: "deepseek",
      api_key_configured: true,
      validation_status: "credential_resolved"
    }]);

    expect(summarizeProviderGuideState(missingSettings)).toMatchObject({
      providerId: "deepseek",
      configured: false,
      configuredCount: 0,
      missingCount: 1,
      status: "missing"
    });
    expect(summarizeProviderGuideState(configuredSettings)).toMatchObject({
      providerId: "deepseek",
      configured: true,
      configuredCount: 1,
      missingCount: 0,
      validationStatus: "credential_resolved",
      status: "configured"
    });
  });

  it("reports unknown provider state before settings load", () => {
    expect(summarizeProviderGuideState()).toMatchObject({
      providerId: "deepseek",
      configured: false,
      configuredCount: 0,
      missingCount: 0,
      status: "unknown"
    });
  });

  it("does not auto-open for the current seen guide version", () => {
    expect(shouldAutoOpenOnboardingGuide({
      startupComplete: true,
      settingsReady: true,
      seenVersion: ONBOARDING_GUIDE_VERSION,
      open: false
    })).toBe(false);
  });
});

function appSettings(providers: ProviderApiRecord[]): AppSettings {
  return {
    provider_api: { providers },
    data_paths: {
      app_config_root: "C:/SuperNova/config",
      app_state_root: "C:/SuperNova/state",
      workspace_registry_path: "C:/SuperNova/state/workspaces.json"
    },
    appearance: {
      language: "en-US",
      theme: "dark"
    }
  };
}
