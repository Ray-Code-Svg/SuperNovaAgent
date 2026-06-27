import { create } from "zustand";
import { persist } from "zustand/middleware";
import type { SourceGuidance } from "../../protocol/generated/types";
import { scopedWindowWorkspaceKey } from "./windowScope";

export type ContainerMode = "chat" | "task";
export type WorkbenchFlyout = "slash" | "source" | "artifact" | "model" | "context" | null;
export type DisplayLanguage = "zh-CN" | "en-US";
export type DisplayTheme = "light" | "dark";
export type SettingsTab = "provider" | "data" | "appearance" | "runtime" | "diagnostics" | "archived" | "guide";

export interface ArtifactTargetSelection {
  targetId: string;
  targetDir: string;
  label: string;
  artifactType?: string | null;
  saveStrategy?: string | null;
}

export interface WorkbenchUiState {
  activeContainerId: string | null;
  activeContainerByWorkspace: Record<string, string | null>;
  activeContainerByWindowWorkspace: Record<string, string | null>;
  modeByContainer: Record<string, ContainerMode>;
  draftByContainer: Record<string, string>;
  selectedTaskByContainer: Record<string, string | null>;
  selectedChatThreadByContainer: Record<string, string | null>;
  sourceGuidanceByContainer: Record<string, SourceGuidance | null>;
  artifactTargetByContainer: Record<string, ArtifactTargetSelection | string | null>;
  expandedWorkspaceById: Record<string, boolean>;
  language: DisplayLanguage;
  theme: DisplayTheme;
  openFlyout: WorkbenchFlyout;
  settingsOpen: boolean;
  settingsTab: SettingsTab;
  onboardingGuideOpen: boolean;
  onboardingGuideSeenVersion: string | null;
  activeContainer(workspaceId: string | null | undefined, windowId?: string | null): string | null;
  setActiveContainer(containerId: string | null, workspaceId?: string | null, windowId?: string | null): void;
  mode(scopeId: string | null): ContainerMode;
  setMode(scopeId: string | null, mode: ContainerMode): void;
  draft(scopeId: string | null): string;
  setDraft(scopeId: string | null, draft: string): void;
  selectedTask(scopeId: string | null): string | null;
  setSelectedTask(scopeId: string | null, taskId: string | null): void;
  selectedChatThread(scopeId: string | null): string | null;
  setSelectedChatThread(scopeId: string | null, chatThreadId: string | null): void;
  sourceGuidance(scopeId: string | null): SourceGuidance | null;
  setSourceGuidance(scopeId: string | null, guidance: SourceGuidance | null): void;
  artifactTarget(scopeId: string | null): ArtifactTargetSelection | null;
  setArtifactTarget(scopeId: string | null, selection: ArtifactTargetSelection | null): void;
  workspaceExpanded(workspaceId: string | null | undefined, defaultExpanded?: boolean): boolean;
  setWorkspaceExpanded(workspaceId: string, expanded: boolean): void;
  setLanguage(language: DisplayLanguage): void;
  setTheme(theme: DisplayTheme): void;
  setOpenFlyout(flyout: WorkbenchFlyout): void;
  setSettingsOpen(open: boolean): void;
  setSettingsTab(tab: SettingsTab): void;
  openSettingsTab(tab: SettingsTab): void;
  openOnboardingGuide(): void;
  dismissOnboardingGuide(version: string): void;
}

export const useWorkbenchUiStore = create<WorkbenchUiState>()(
  persist(
    (set, get) => ({
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
      onboardingGuideSeenVersion: null,
      activeContainer: (workspaceId, windowId) => {
        const scopedKey = scopedWindowWorkspaceKey(windowId, workspaceId);
        if (scopedKey) {
          const scoped = get().activeContainerByWindowWorkspace[scopedKey];
          if (scoped !== undefined) return scoped;
        }
        return workspaceId
          ? get().activeContainerByWorkspace[workspaceId] || null
          : get().activeContainerId;
      },
      setActiveContainer: (activeContainerId, workspaceId, windowId) => {
        const scopedKey = scopedWindowWorkspaceKey(windowId, workspaceId);
        set((state) => {
          if (scopedKey) {
            return {
              activeContainerByWindowWorkspace: {
                ...state.activeContainerByWindowWorkspace,
                [scopedKey]: activeContainerId
              }
            };
          }
          return {
            activeContainerId,
            activeContainerByWorkspace: workspaceId
              ? { ...state.activeContainerByWorkspace, [workspaceId]: activeContainerId }
              : state.activeContainerByWorkspace
          };
        });
      },
      mode: (scopeId) =>
        scopeId ? get().modeByContainer[scopeId] || "chat" : "chat",
      setMode: (scopeId, mode) => {
        if (!scopeId) return;
        set((state) => ({
          modeByContainer: { ...state.modeByContainer, [scopeId]: mode }
        }));
      },
      draft: (scopeId) =>
        scopeId ? get().draftByContainer[scopeId] || "" : "",
      setDraft: (scopeId, draft) => {
        if (!scopeId) return;
        set((state) => ({
          draftByContainer: { ...state.draftByContainer, [scopeId]: draft }
        }));
      },
      selectedTask: (scopeId) =>
        scopeId ? get().selectedTaskByContainer[scopeId] || null : null,
      setSelectedTask: (scopeId, taskId) => {
        if (!scopeId) return;
        set((state) => ({
          selectedTaskByContainer: { ...state.selectedTaskByContainer, [scopeId]: taskId }
        }));
      },
      selectedChatThread: (scopeId) =>
        scopeId ? get().selectedChatThreadByContainer[scopeId] || null : null,
      setSelectedChatThread: (scopeId, chatThreadId) => {
        if (!scopeId) return;
        set((state) => ({
          selectedChatThreadByContainer: { ...state.selectedChatThreadByContainer, [scopeId]: chatThreadId }
        }));
      },
      sourceGuidance: (scopeId) =>
        scopeId ? get().sourceGuidanceByContainer[scopeId] || null : null,
      setSourceGuidance: (scopeId, guidance) => {
        if (!scopeId) return;
        set((state) => ({
          sourceGuidanceByContainer: { ...state.sourceGuidanceByContainer, [scopeId]: normalizeSourceGuidance(guidance) }
        }));
      },
      artifactTarget: (scopeId) =>
        scopeId ? stableArtifactTargetSelection(get().artifactTargetByContainer[scopeId]) : null,
      setArtifactTarget: (scopeId, selection) => {
        if (!scopeId) return;
        set((state) => ({
          artifactTargetByContainer: { ...state.artifactTargetByContainer, [scopeId]: selection }
        }));
      },
      workspaceExpanded: (workspaceId, defaultExpanded = false) => {
        if (!workspaceId) return defaultExpanded;
        const stored = get().expandedWorkspaceById[workspaceId];
        return stored ?? defaultExpanded;
      },
      setWorkspaceExpanded: (workspaceId, expanded) => {
        set((state) => ({
          expandedWorkspaceById: { ...state.expandedWorkspaceById, [workspaceId]: expanded }
        }));
      },
      setLanguage: (language) => set({ language }),
      setTheme: (theme) => set({ theme }),
      setOpenFlyout: (openFlyout) => set({ openFlyout }),
      setSettingsOpen: (settingsOpen) => set({ settingsOpen }),
      setSettingsTab: (settingsTab) => set({ settingsTab }),
      openSettingsTab: (settingsTab) => set({ settingsOpen: true, settingsTab }),
      openOnboardingGuide: () => set({ onboardingGuideOpen: true }),
      dismissOnboardingGuide: (onboardingGuideSeenVersion) =>
        set({ onboardingGuideOpen: false, onboardingGuideSeenVersion })
    }),
    {
      name: "supernova.workbench_v2.ui",
      partialize: persistedWorkbenchUiState
    }
  )
);

export function persistedWorkbenchUiState(state: WorkbenchUiState) {
  return {
    activeContainerId: state.activeContainerId,
    activeContainerByWorkspace: state.activeContainerByWorkspace,
    activeContainerByWindowWorkspace: state.activeContainerByWindowWorkspace,
    modeByContainer: state.modeByContainer,
    draftByContainer: state.draftByContainer,
    selectedTaskByContainer: state.selectedTaskByContainer,
    selectedChatThreadByContainer: state.selectedChatThreadByContainer,
    sourceGuidanceByContainer: state.sourceGuidanceByContainer,
    artifactTargetByContainer: state.artifactTargetByContainer,
    expandedWorkspaceById: state.expandedWorkspaceById,
    language: state.language,
    theme: state.theme,
    onboardingGuideSeenVersion: state.onboardingGuideSeenVersion
  };
}

function normalizeSourceGuidance(value: SourceGuidance | null | undefined): SourceGuidance | null {
  if (!value?.selected_sources?.length) return null;
  return {
    semantics: "model_guidance_only",
    materialized_content: false,
    source_scope_enforcement: "none",
    selected_sources: value.selected_sources.map((source) => ({
      ...source,
      usage: source.usage || "primary_reference_scope",
      include_mode: source.include_mode || "reference_only",
      selection_source: source.selection_source || "composer_at_token"
    })),
    user_intent: value.user_intent || null
  };
}

function normalizeArtifactTargetSelection(value: ArtifactTargetSelection | string | null | undefined) {
  if (!value) return null;
  if (typeof value === "string") {
    return {
      targetId: value,
      targetDir: "",
      label: value,
      artifactType: null,
      saveStrategy: null
    };
  }
  if (!value.targetId) return null;
  return {
    targetId: value.targetId,
    targetDir: value.targetDir || "",
    label: value.label || value.targetId,
    artifactType: value.artifactType || null,
    saveStrategy: value.saveStrategy || null
  };
}

function stableArtifactTargetSelection(value: ArtifactTargetSelection | string | null | undefined) {
  if (!value || typeof value === "string") return null;
  return value;
}
