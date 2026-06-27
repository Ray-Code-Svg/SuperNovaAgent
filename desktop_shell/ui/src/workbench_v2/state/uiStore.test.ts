import { beforeEach, describe, expect, it } from "vitest";

import { ONBOARDING_GUIDE_VERSION, shouldAutoOpenOnboardingGuide } from "../onboarding/onboardingState";
import { persistedWorkbenchUiState, useWorkbenchUiStore } from "./uiStore";

describe("Workbench UI store", () => {
  beforeEach(() => {
    useWorkbenchUiStore.setState({
      activeContainerId: null,
      activeContainerByWorkspace: {},
      activeContainerByWindowWorkspace: {},
      modeByContainer: {},
      draftByContainer: {},
      selectedTaskByContainer: {},
      selectedChatThreadByContainer: {},
      sourceGuidanceByContainer: {},
      artifactTargetByContainer: {},
      expandedWorkspaceById: {},
      language: "en-US",
      theme: "dark",
      openFlyout: null,
      settingsOpen: false,
      settingsTab: "provider",
      onboardingGuideOpen: false,
      onboardingGuideSeenVersion: null
    });
  });

  it("keeps active container selection isolated per window", () => {
    useWorkbenchUiStore.getState().setActiveContainer("container_a", "workspace_1", "window_a");
    useWorkbenchUiStore.getState().setActiveContainer("container_b", "workspace_1", "window_b");

    expect(useWorkbenchUiStore.getState().activeContainer("workspace_1", "window_a")).toBe("container_a");
    expect(useWorkbenchUiStore.getState().activeContainer("workspace_1", "window_b")).toBe("container_b");
    expect(useWorkbenchUiStore.getState().activeContainer("workspace_1")).toBeNull();
  });

  it("defaults to an unseen closed onboarding guide", () => {
    expect(useWorkbenchUiStore.getState().onboardingGuideOpen).toBe(false);
    expect(useWorkbenchUiStore.getState().onboardingGuideSeenVersion).toBeNull();
    expect(useWorkbenchUiStore.getState().settingsTab).toBe("provider");
  });

  it("marks onboarding as seen when dismissed", () => {
    useWorkbenchUiStore.getState().openOnboardingGuide();
    useWorkbenchUiStore.getState().dismissOnboardingGuide(ONBOARDING_GUIDE_VERSION);

    expect(useWorkbenchUiStore.getState().onboardingGuideOpen).toBe(false);
    expect(useWorkbenchUiStore.getState().onboardingGuideSeenVersion).toBe(ONBOARDING_GUIDE_VERSION);
  });

  it("opens settings on the requested tab without persisting transient UI state", () => {
    useWorkbenchUiStore.getState().openSettingsTab("appearance");
    useWorkbenchUiStore.getState().openOnboardingGuide();
    useWorkbenchUiStore.getState().dismissOnboardingGuide(ONBOARDING_GUIDE_VERSION);

    const persisted = persistedWorkbenchUiState(useWorkbenchUiStore.getState());

    expect(useWorkbenchUiStore.getState().settingsOpen).toBe(true);
    expect(useWorkbenchUiStore.getState().settingsTab).toBe("appearance");
    expect(persisted).toHaveProperty("onboardingGuideSeenVersion", ONBOARDING_GUIDE_VERSION);
    expect(persisted).not.toHaveProperty("settingsOpen");
    expect(persisted).not.toHaveProperty("settingsTab");
    expect(persisted).not.toHaveProperty("onboardingGuideOpen");
  });

  it("auto-opens onboarding only after startup and settings are ready for an unseen version", () => {
    expect(shouldAutoOpenOnboardingGuide({
      startupComplete: false,
      settingsReady: true,
      seenVersion: null,
      open: false
    })).toBe(false);
    expect(shouldAutoOpenOnboardingGuide({
      startupComplete: true,
      settingsReady: false,
      seenVersion: null,
      open: false
    })).toBe(false);
    expect(shouldAutoOpenOnboardingGuide({
      startupComplete: true,
      settingsReady: true,
      seenVersion: ONBOARDING_GUIDE_VERSION,
      open: false
    })).toBe(false);
    expect(shouldAutoOpenOnboardingGuide({
      startupComplete: true,
      settingsReady: true,
      seenVersion: "older-guide",
      open: false
    })).toBe(true);
    expect(shouldAutoOpenOnboardingGuide({
      startupComplete: true,
      settingsReady: true,
      seenVersion: null,
      open: true
    })).toBe(false);
  });
});
