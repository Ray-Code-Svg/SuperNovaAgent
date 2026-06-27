import { useEffect, useMemo, useState } from "react";
import { Dialog, DialogBody, DialogContent, DialogSurface, DialogTitle } from "@fluentui/react-components";
import { keepPreviousData, useMutation, useQuery } from "@tanstack/react-query";

import type {
  ArtifactDestinationGuidance,
  ContainerMessage,
  ContextPack,
  ModelConfig,
  ProtocolEvent,
  SourceGuidance
} from "../../protocol/generated/types";
import { ActiveContainerSurface } from "../layout/ActiveContainerSurface";
import { ProjectsRail } from "../layout/ProjectsRail";
import { SystemSettingsDialog } from "../settings/SystemSettingsDialog";
import { ArtifactTargetSelection, type ContainerMode, useWorkbenchUiStore } from "../state/uiStore";
import { getWorkbenchWindowId, scopedContainerStateKey } from "../state/windowScope";
import {
  activateContainer,
  archiveContainer,
  createContainer,
  listContainers,
  listWorkspaceContainers,
  updateContainer
} from "../protocol/containerQueries";
import {
  activateWorkspace,
  archiveWorkspace,
  createWorkspace,
  listWorkspaces
} from "../protocol/workspaceQueries";
import { createChatThread, forceCloseChatTurn, listChatThreads, sendChatTurn } from "../protocol/chatQueries";
import {
  forceCloseTask,
  getTask,
  startTask,
  submitTaskUserInput
} from "../protocol/taskQueries";
import { listRuns } from "../protocol/runQueries";
import { listArtifactTargets } from "../protocol/artifactQueries";
import {
  estimateContextPack,
  getContextPack,
  getModelConfig,
  getSettings,
  saveContextPack,
  updateModelConfig
} from "../protocol/settingsQueries";
import { getRuntimeCapabilities, getRuntimeDiagnostics, getRuntimeEvents, getRuntimeMeta } from "../protocol/runtimeQueries";
import { createRuntimeClient, invokeShell } from "../protocol/runtimeClient";
import { queryClient } from "../protocol/queryClient";
import { useI18n } from "../i18n/i18n";
import { OnboardingGuide } from "../onboarding/OnboardingGuide";
import { ONBOARDING_GUIDE_VERSION, shouldAutoOpenOnboardingGuide } from "../onboarding/onboardingState";
import { FluentAppShell } from "./FluentAppShell";
import { FluentTitleBar } from "./FluentTitleBar";
import {
  activeWorkspaceContainerItems,
  buildRailContainersByWorkspace,
  buildRailContainerStateByWorkspace
} from "./railData";
import { RuntimeStatusBar } from "./RuntimeStatusBar";
import {
  blockingRunFromPage,
  forceCloseTargetFromBlockingRun,
  runPageHasBlockingRun
} from "./runStatus";
import { StartupScreen, type StartupStage } from "./StartupScreen";
import {
  appendMessageByContainer,
  mergeMessages,
  messageBelongsToVisibleContainer,
  streamEventMessage
} from "./streamMessages";
import { selectedWorkspaceRoot, type WorkspaceDialogResult } from "./workspaceDialog";

const EMPTY_MESSAGES: ContainerMessage[] = [];

export function WorkbenchV2() {
  const t = useI18n();
  const [windowId] = useState(() => getWorkbenchWindowId());
  const [busyByContainer, setBusyByContainer] = useState<Record<string, boolean>>({});
  const [initialStartupComplete, setInitialStartupComplete] = useState(false);
  const [liveMessagesByContainer, setLiveMessagesByContainer] = useState<Record<string, ContainerMessage[]>>({});
  const runtime = useQuery({ queryKey: ["runtime-meta"], queryFn: getRuntimeMeta });
  const capabilities = useQuery({ queryKey: ["runtime-capabilities"], queryFn: getRuntimeCapabilities });
  const workspaceId = runtime.data?.workspace_id || null;
  const activeContainerId = useWorkbenchUiStore((state) => state.activeContainer(workspaceId, windowId));
  const setActiveContainer = useWorkbenchUiStore((state) => state.setActiveContainer);
  const setSelectedTask = useWorkbenchUiStore((state) => state.setSelectedTask);
  const setSelectedChatThread = useWorkbenchUiStore((state) => state.setSelectedChatThread);
  const setSourceGuidance = useWorkbenchUiStore((state) => state.setSourceGuidance);
  const setArtifactTarget = useWorkbenchUiStore((state) => state.setArtifactTarget);
  const setSettingsOpen = useWorkbenchUiStore((state) => state.setSettingsOpen);
  const setSettingsTab = useWorkbenchUiStore((state) => state.setSettingsTab);
  const openSettingsTab = useWorkbenchUiStore((state) => state.openSettingsTab);
  const openOnboardingGuide = useWorkbenchUiStore((state) => state.openOnboardingGuide);
  const dismissOnboardingGuide = useWorkbenchUiStore((state) => state.dismissOnboardingGuide);
  const setLanguage = useWorkbenchUiStore((state) => state.setLanguage);
  const setTheme = useWorkbenchUiStore((state) => state.setTheme);
  const settingsOpen = useWorkbenchUiStore((state) => state.settingsOpen);
  const settingsTab = useWorkbenchUiStore((state) => state.settingsTab);
  const onboardingGuideOpen = useWorkbenchUiStore((state) => state.onboardingGuideOpen);
  const onboardingGuideSeenVersion = useWorkbenchUiStore((state) => state.onboardingGuideSeenVersion);

  useEffect(() => {
    const preventNativeContextMenu = (event: MouseEvent) => {
      event.preventDefault();
    };
    document.addEventListener("contextmenu", preventNativeContextMenu, true);
    return () => document.removeEventListener("contextmenu", preventNativeContextMenu, true);
  }, []);

  const diagnostics = useQuery({ queryKey: ["diagnostics"], queryFn: getRuntimeDiagnostics });
  const workspaces = useQuery({
    queryKey: ["workspaces"],
    queryFn: listWorkspaces,
    placeholderData: keepPreviousData
  });
  const activeWorkspace = workspaces.data?.items.find((workspace) => workspace.workspace_uid === workspaceId) || null;
  const containers = useQuery({
    queryKey: ["containers", runtime.data?.workspace_id],
    queryFn: listContainers,
    enabled: Boolean(runtime.data?.workspace_id)
  });
  const workspaceContainerMap = useQuery({
    queryKey: ["workspace-containers", runtime.data?.workspace_id, workspaceIds(workspaces.data?.items)],
    queryFn: async () => {
      const items = workspaces.data?.items || [];
      const pairs = await Promise.all(
        items.map(async (workspace) => [
          workspace.workspace_uid,
          (await listWorkspaceContainers(workspace.workspace_uid)).items
        ] as const)
      );
      return Object.fromEntries(pairs);
    },
    enabled: Boolean(runtime.data?.workspace_id && workspaces.data?.items.length),
    placeholderData: keepPreviousData
  });
  const containersByWorkspace = useMemo(
    () =>
      buildRailContainersByWorkspace(
        workspaceContainerMap.data,
        runtime.data?.workspace_id,
        containers.data?.items
      ),
    [containers.data?.items, runtime.data?.workspace_id, workspaceContainerMap.data]
  );
  const containerStateByWorkspace = useMemo(
    () =>
      buildRailContainerStateByWorkspace({
        workspaces: workspaces.data?.items || [],
        containersByWorkspace,
        workspaceContainerMapLoading: workspaceContainerMap.isLoading || workspaceContainerMap.isFetching,
        workspaceContainerMapError: workspaceContainerMap.isError,
        activeWorkspaceId: workspaceId,
        activeContainersLoading: containers.isLoading || containers.isFetching,
        activeContainersError: containers.isError
      }),
    [
      containers.isError,
      containers.isFetching,
      containers.isLoading,
      containersByWorkspace,
      workspaceContainerMap.isError,
      workspaceContainerMap.isFetching,
      workspaceContainerMap.isLoading,
      workspaceId,
      workspaces.data?.items
    ]
  );
  const activeWorkspaceContainers = activeWorkspaceContainerItems(
    workspaceId,
    containers.data?.items,
    workspaceContainerMap.data
  );
  const activeContainer = activeWorkspaceContainers.find((item) => item.container_id === activeContainerId);
  const containerId = activeContainer?.container_id || null;
  const scopeId = scopedContainerStateKey(windowId, workspaceId, containerId);
  const mode = useWorkbenchUiStore((state) => state.mode(scopeId));
  const selectedTaskId = useWorkbenchUiStore((state) => state.selectedTask(scopeId));
  const selectedChatThreadId = useWorkbenchUiStore((state) => state.selectedChatThread(scopeId));
  const selectedSourceGuidance = useWorkbenchUiStore((state) => state.sourceGuidance(scopeId));
  const selectedArtifactTarget = useWorkbenchUiStore((state) => state.artifactTarget(scopeId));
  const messages = useQuery({
    queryKey: ["container-messages", containerId, mode],
    queryFn: async () => {
      if (!containerId) return { items: [] as ContainerMessage[], count: 0 };
      const client = await createRuntimeClient();
      return client.containerMessages(containerId, { limit: 200, lane: mode });
    },
    enabled: Boolean(containerId)
  });
  const runs = useQuery({
    queryKey: ["runs", containerId],
    queryFn: () => listRuns(containerId),
    enabled: Boolean(containerId),
    refetchInterval: (query) => (runPageHasBlockingRun(query.state.data) ? 2000 : false)
  });
  const liveMessages = containerId ? liveMessagesByContainer[containerId] || EMPTY_MESSAGES : EMPTY_MESSAGES;
  const chatThreads = useQuery({
    queryKey: ["chat-threads", containerId],
    queryFn: () => listChatThreads(containerId || ""),
    enabled: Boolean(containerId)
  });
  const selectedTaskDetail = useQuery({
    queryKey: ["task-detail", selectedTaskId],
    queryFn: () => getTask(selectedTaskId || ""),
    enabled: Boolean(selectedTaskId)
  });
  const contextPack = useQuery({
    queryKey: ["context-pack", containerId],
    queryFn: () => getContextPack(containerId || ""),
    enabled: Boolean(containerId)
  });
  const modelConfig = useQuery({ queryKey: ["model-config"], queryFn: getModelConfig });
  const settings = useQuery({ queryKey: ["settings"], queryFn: getSettings });
  const artifactTargets = useQuery({
    queryKey: ["artifact-targets", containerId],
    queryFn: () => listArtifactTargets(containerId || ""),
    enabled: Boolean(containerId)
  });

  const archiveContainerMutation = useMutation({
    mutationFn: archiveContainer,
    onSuccess: () => refreshContainerRailQueries()
  });
  const renameContainerMutation = useMutation({
    mutationFn: async ({
      workspaceUid,
      containerId,
      title
    }: {
      workspaceUid: string;
      containerId: string;
      title: string;
    }) => {
      if (workspaceUid !== runtime.data?.workspace_id) {
        const activation = await activateWorkspace(workspaceUid);
        setActiveContainer(activation.recent_active_container_id || null, activation.workspace.workspace_uid, windowId);
        await invalidateActiveWorkspaceQueries();
      }
      return updateContainer(containerId, { title });
    },
    onSuccess: async () => {
      await refreshContainerRailQueries();
    }
  });
  const archiveWorkspaceMutation = useMutation({
    mutationFn: archiveWorkspace,
    onSuccess: async (_workspace, archivedWorkspaceUid) => {
      if (archivedWorkspaceUid === workspaceId) {
        setActiveContainer(null, workspaceId, windowId);
      }
      await invalidateActiveWorkspaceQueries();
    }
  });
  const modelConfigMutation = useMutation({
    mutationFn: updateModelConfig,
    onSuccess: async (descriptor) => {
      queryClient.setQueryData(["model-config"], descriptor);
      await queryClient.invalidateQueries({ queryKey: ["runtime-capabilities"] });
    }
  });
  const contextPackSaveMutation = useMutation({
    mutationFn: async (pack: ContextPack) => {
      if (!containerId) throw new Error("No active container for context pack save.");
      return saveContextPack(containerId, pack);
    },
    onSuccess: async (pack) => {
      queryClient.setQueryData(["context-pack", pack.container_id], pack);
      await queryClient.invalidateQueries({ queryKey: ["source-candidates", pack.container_id] });
    }
  });
  const activateContainerMutation = useMutation({
    mutationFn: activateContainer,
    onSuccess: () => refreshContainerRailQueries()
  });
  const activateWorkspaceMutation = useMutation({
    mutationFn: activateWorkspace,
    onSuccess: async (activation) => {
      setActiveContainer(activation.recent_active_container_id || null, activation.workspace.workspace_uid, windowId);
      await invalidateActiveWorkspaceQueries();
    }
  });
  const taskUserInputMutation = useMutation({
    mutationFn: async ({ taskId, input }: { taskId: string; input: string; containerId: string }) =>
      submitTaskUserInput(taskId, { input }),
    onSuccess: async (result) => {
      for (const message of result.messages) {
        setLiveMessagesByContainer((current) => appendMessageByContainer(current, message));
      }
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: ["tasks", result.task.container_id] }),
        queryClient.invalidateQueries({ queryKey: ["task-detail", result.task.task_id] }),
        queryClient.invalidateQueries({ queryKey: ["container-messages", result.task.container_id] }),
        queryClient.invalidateQueries({ queryKey: ["runs", result.task.container_id] })
      ]);
    }
  });
  type ForceCloseTarget = {
    containerId: string;
    mode: "chat" | "task";
    chatThreadId?: string | null;
    taskId?: string | null;
  };
  const forceCloseMutation = useMutation({
    mutationFn: async ({ mode, chatThreadId, taskId }: ForceCloseTarget) => {
      const reason = "用户强制关闭";
      if (mode === "chat") {
        if (!chatThreadId) throw new Error("No active chat thread to force close.");
        return forceCloseChatTurn(chatThreadId, { reason });
      }
      if (!taskId) throw new Error("No active task to force close.");
      return forceCloseTask(taskId, { reason });
    },
    onSuccess: async (result, variables) => {
      setContainerBusy(variables.containerId, false);
      for (const message of result.messages) {
        setLiveMessagesByContainer((current) => appendMessageByContainer(current, message));
      }
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: ["container-messages", variables.containerId] }),
        queryClient.invalidateQueries({ queryKey: ["chat-threads", variables.containerId] }),
        queryClient.invalidateQueries({ queryKey: ["tasks", variables.containerId] }),
        queryClient.invalidateQueries({ queryKey: ["runs", variables.containerId] }),
        queryClient.invalidateQueries({ queryKey: ["task-detail", selectedTaskId] })
      ]);
    }
  });
  const localContainerBusy = containerId ? Boolean(busyByContainer[containerId]) : false;
  const blockingRun = blockingRunFromPage(runs.data);
  const blockingRunForceCloseTarget = forceCloseTargetFromBlockingRun(blockingRun);
  const runBusy = Boolean(blockingRun);
  const taskUserInputPendingForContainer =
    taskUserInputMutation.isPending && taskUserInputMutation.variables?.containerId === containerId;
  const forceClosePendingForContainer =
    forceCloseMutation.isPending && forceCloseMutation.variables?.containerId === containerId;
  const interactionBusy = Boolean(
    containerId &&
      (localContainerBusy ||
        runBusy ||
        taskUserInputPendingForContainer ||
        forceClosePendingForContainer)
  );
  const visibleStatus =
    errorText(runtime.error) ||
    errorText(capabilities.error) ||
    errorText(containers.error) ||
    errorText(workspaceContainerMap.error) ||
    errorText(workspaces.error) ||
    errorText(modelConfigMutation.error) ||
    errorText(contextPackSaveMutation.error) ||
    errorText(renameContainerMutation.error) ||
    errorText(activateWorkspaceMutation.error) ||
    errorText(archiveWorkspaceMutation.error) ||
    errorText(activateContainerMutation.error) ||
    errorText(taskUserInputMutation.error) ||
    errorText(forceCloseMutation.error) ||
    diagnostics.data?.runtime_status ||
    t("title.status.loading");
  const startupStages = startupStageState({
    runtimeReady: runtime.isSuccess,
    kernelReady: capabilities.isSuccess && diagnostics.isSuccess,
    historyReady:
      workspaces.isSuccess &&
      (!workspaces.data?.items.length || workspaceContainerMap.isSuccess) &&
      (!runtime.data?.workspace_id || containers.isSuccess),
    settingsReady: settings.isSuccess
  });
  const startupError =
    errorText(runtime.error) ||
    errorText(capabilities.error) ||
    errorText(workspaces.error) ||
    errorText(workspaceContainerMap.error) ||
    errorText(containers.error) ||
    errorText(settings.error);
  const startupReady = startupStages.every((stage) => stage.status === "complete");

  useEffect(() => {
    if (!startupReady || initialStartupComplete) return;
    function handleStartupKeyDown(event: KeyboardEvent) {
      if (event.key !== "Enter" || event.repeat) return;
      event.preventDefault();
      setInitialStartupComplete(true);
    }
    window.addEventListener("keydown", handleStartupKeyDown);
    return () => window.removeEventListener("keydown", handleStartupKeyDown);
  }, [initialStartupComplete, startupReady]);

  useEffect(() => {
    const appearance = settings.data?.appearance;
    if (!appearance) return;
    setLanguage(appearance.language);
    setTheme(appearance.theme);
    document.documentElement.lang = appearance.language;
  }, [setLanguage, setTheme, settings.data?.appearance]);

  useEffect(() => {
    if (!shouldAutoOpenOnboardingGuide({
      startupComplete: initialStartupComplete,
      settingsReady: settings.isSuccess,
      seenVersion: onboardingGuideSeenVersion,
      open: onboardingGuideOpen
    })) {
      return;
    }
    openOnboardingGuide();
  }, [
    initialStartupComplete,
    onboardingGuideOpen,
    onboardingGuideSeenVersion,
    openOnboardingGuide,
    settings.isSuccess
  ]);

  useEffect(() => {
    if (!workspaceId || !containers.isSuccess || !containers.data?.items) return;
    const current = containers.data.items.find((item) => item.container_id === activeContainerId);
    if (activeContainerId && !current) {
      setActiveContainer(null, workspaceId, windowId);
    }
  }, [activeContainerId, containers.data?.items, containers.isSuccess, setActiveContainer, windowId, workspaceId]);

  useEffect(() => {
    if (!workspaceId) return;
    let cancelled = false;
    let cursor = 0;
    const visibleContainerId = containerId;

    async function pollRuntimeEvents() {
      while (!cancelled) {
        try {
          const events = await getRuntimeEvents(cursor, 200);
          const touchedContainerIds = new Set<string>();
          const touchedTaskIds = new Set<string>();

          for (const event of events) {
            const nextCursor = event.cursor?.after_event_id;
            if (typeof nextCursor === "number" && nextCursor > cursor) {
              cursor = nextCursor;
            }

            const message = streamEventMessage(event);
            if (message) {
              if (messageBelongsToVisibleContainer(message, visibleContainerId)) {
                setLiveMessagesByContainer((current) => appendMessageByContainer(current, message));
              }
              touchedContainerIds.add(message.container_id);
              if (messageShouldRefreshTaskDetail(message) && message.task_id) {
                touchedTaskIds.add(message.task_id);
              }
              continue;
            }

            if (event.container_id) {
              touchedContainerIds.add(event.container_id);
            }
            const taskId = event.task_id || event.job_id;
            if (taskId) {
              touchedTaskIds.add(taskId);
            }
          }

          if (touchedContainerIds.size > 0 || touchedTaskIds.size > 0) {
            await refreshContainerSummaries(touchedContainerIds, touchedTaskIds);
          }
          await sleep(cancelled ? 0 : 2000);
        } catch {
          await sleep(cancelled ? 0 : 4000);
        }
      }
    }

    void pollRuntimeEvents();
    return () => {
      cancelled = true;
    };
  }, [containerId, scopeId, workspaceId]);

  const sortedMessages = useMemo(
    () => mergeMessages(messages.data?.items || EMPTY_MESSAGES, liveMessages),
    [liveMessages, messages.data?.items]
  );

  async function addWorkspace() {
    const dialog = await invokeShell<WorkspaceDialogResult>("workspace_choose_dialog");
    const workspaceRoot = selectedWorkspaceRoot(dialog);
    if (workspaceRoot) {
      const workspace = await createWorkspace(workspaceRoot);
      const activation = await activateWorkspace(workspace.workspace_uid);
      setActiveContainer(activation.recent_active_container_id || null, activation.workspace.workspace_uid, windowId);
      await invalidateActiveWorkspaceQueries();
    }
  }

  async function addContainer(workspaceUid: string) {
    if (workspaceUid !== runtime.data?.workspace_id) {
      const activation = await activateWorkspace(workspaceUid);
      setActiveContainer(activation.recent_active_container_id || null, activation.workspace.workspace_uid, windowId);
      await invalidateActiveWorkspaceQueries();
    }
    const container = await createContainer(`Container ${Date.now().toString().slice(-4)}`);
    setActiveContainer(container.container_id, workspaceUid, windowId);
    await refreshContainerRailQueries();
  }

  async function selectContainer(workspaceUid: string, containerId: string) {
    if (workspaceUid !== runtime.data?.workspace_id) {
      const activation = await activateWorkspace(workspaceUid);
      setActiveContainer(containerId, activation.workspace.workspace_uid, windowId);
      await invalidateActiveWorkspaceQueries();
    } else {
      setActiveContainer(containerId, workspaceId, windowId);
    }
    activateContainerMutation.mutate(containerId);
  }

  function renameContainer(workspaceUid: string, containerId: string, title: string) {
    renameContainerMutation.mutate({ workspaceUid, containerId, title });
  }

  async function archiveContainerFromRail(workspaceUid: string, containerId: string) {
    if (workspaceUid !== runtime.data?.workspace_id) {
      const activation = await activateWorkspace(workspaceUid);
      setActiveContainer(activation.recent_active_container_id || null, activation.workspace.workspace_uid, windowId);
      await invalidateActiveWorkspaceQueries();
    }
    archiveContainerMutation.mutate(containerId);
  }

  async function submit(value: string, modeAtSubmit?: ContainerMode) {
    if (!containerId) return;
    const currentContainerId = containerId;
    const currentScopeId = scopeId;
    const routeMode = modeAtSubmit || mode;
    setContainerBusy(currentContainerId, true);
    try {
      if (routeMode === "chat") {
        let thread = selectedChatThreadId
          ? chatThreads.data?.items.find((item) => item.chat_thread_id === selectedChatThreadId)
          : undefined;
        if (!thread) {
          thread = await createChatThread(currentContainerId);
          setSelectedChatThread(currentScopeId, thread.chat_thread_id);
        }
        await sendChatTurn(thread.chat_thread_id, {
          message: value,
          context_pack: contextPackForRequest(contextPack.data),
          source_guidance: selectedSourceGuidance,
          model_config: modelConfig.data?.active || null
        }, (event) => appendStreamMessage(event, { containerId: currentContainerId }));
      } else {
        await startTask(currentContainerId, {
          goal: value,
          context_pack_id: null,
          source_guidance: selectedSourceGuidance,
          model_config: modelConfig.data?.active || null,
          artifact_destination: artifactDestinationGuidance(selectedArtifactTarget),
          artifact_target: null,
          auto_approve: false
        }, (event) => appendStreamMessage(event, { selectTask: true, containerId: currentContainerId }));
      }
      setSourceGuidance(currentScopeId, null);
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: ["container-messages", currentContainerId] }),
        queryClient.invalidateQueries({ queryKey: ["chat-threads", currentContainerId] }),
        queryClient.invalidateQueries({ queryKey: ["tasks", currentContainerId] }),
        queryClient.invalidateQueries({ queryKey: ["runs", currentContainerId] })
      ]);
    } finally {
      setContainerBusy(currentContainerId, false);
    }
  }

  function setContainerBusy(targetContainerId: string, value: boolean) {
    setBusyByContainer((current) => {
      if (value) {
        return { ...current, [targetContainerId]: true };
      }
      if (!current[targetContainerId]) return current;
      const next = { ...current };
      delete next[targetContainerId];
      return next;
    });
  }

  function appendStreamMessage(
    event: ProtocolEvent<unknown>,
    options: { selectTask?: boolean; containerId?: string } = {}
  ) {
    if (options.containerId && streamEventClosesInteraction(event.event_type)) {
      setContainerBusy(options.containerId, false);
      void Promise.all([
        queryClient.invalidateQueries({ queryKey: ["container-messages", options.containerId] }),
        queryClient.invalidateQueries({ queryKey: ["chat-threads", options.containerId] }),
        queryClient.invalidateQueries({ queryKey: ["tasks", options.containerId] }),
        queryClient.invalidateQueries({ queryKey: ["runs", options.containerId] })
      ]);
    }
    const message = streamEventMessage(event);
    if (!message) return;
    setLiveMessagesByContainer((current) => appendMessageByContainer(current, message));
    if (messageShouldRefreshTaskDetail(message) && message.task_id) {
      void queryClient.invalidateQueries({ queryKey: ["task-detail", message.task_id] });
      void queryClient.invalidateQueries({ queryKey: ["tasks", message.container_id] });
      void queryClient.invalidateQueries({ queryKey: ["runs", message.container_id] });
    }
    if (options.selectTask && message.lane === "task" && message.task_id) {
      setSelectedTask(scopeId, message.task_id);
    }
  }

  function dismissCurrentOnboardingGuide() {
    dismissOnboardingGuide(ONBOARDING_GUIDE_VERSION);
  }

  function openProviderSettingsFromOnboarding() {
    dismissCurrentOnboardingGuide();
    openSettingsTab("provider");
  }

  function openAppearanceSettingsFromOnboarding() {
    dismissCurrentOnboardingGuide();
    openSettingsTab("appearance");
  }

  return (
    <FluentAppShell>
      <div className="sn-workbench-v2">
        <FluentTitleBar
          status={visibleStatus}
          onRefresh={() => queryClient.invalidateQueries()}
        />
        <RuntimeStatusBar runtimeReady={Boolean(runtime.data)} workspaceName={activeWorkspace?.display_name || null} />
        <div className="sn-workbench-body">
          <ProjectsRail
            runtime={runtime.data}
            workspaces={workspaces.data?.items || []}
            containersByWorkspace={containersByWorkspace}
            containerStateByWorkspace={containerStateByWorkspace}
            activeWorkspaceId={runtime.data?.workspace_id}
            activeContainerId={containerId}
            onAddWorkspace={addWorkspace}
            onAddContainer={addContainer}
            onSelectWorkspace={(workspaceUid) => activateWorkspaceMutation.mutate(workspaceUid)}
            onArchiveWorkspace={(workspaceUid) => archiveWorkspaceMutation.mutate(workspaceUid)}
            onSelectContainer={selectContainer}
            onRenameContainer={renameContainer}
            onArchiveContainer={archiveContainerFromRail}
            onOpenSettings={() => setSettingsOpen(true)}
          />
          <ActiveContainerSurface
            container={activeContainer}
            scopeId={scopeId}
            messages={sortedMessages}
            capabilities={capabilities.data}
            contextPack={contextPack.data}
            selectedSourceGuidance={selectedSourceGuidance}
            modelConfig={modelConfig.data}
            selectedTaskId={selectedTaskId}
            selectedTaskDetail={selectedTaskDetail.data}
            selectedArtifactTarget={selectedArtifactTarget}
            artifactTargetOptions={artifactTargets.data?.items || []}
            busy={interactionBusy}
            forceCloseVisible={interactionBusy || Boolean(blockingRunForceCloseTarget)}
            forceCloseDisabled={
              forceClosePendingForContainer ||
              (runBusy
                ? !blockingRunForceCloseTarget
                : mode === "chat"
                  ? !selectedChatThreadId
                  : !(selectedTaskId || selectedTaskDetail.data?.task.task_id))
            }
            modelConfigBusy={modelConfigMutation.isPending}
            contextConfigBusy={contextPackSaveMutation.isPending}
            modelConfigError={modelConfigMutation.error}
            contextConfigError={contextPackSaveMutation.error}
            onSubmit={submit}
            onModelConfigSave={(config: ModelConfig) => modelConfigMutation.mutate(config)}
            onContextPackSave={(pack: ContextPack) => contextPackSaveMutation.mutate(pack)}
            onContextPackEstimate={(pack: ContextPack) => {
              if (!containerId) throw new Error("No active container for context estimate.");
              return estimateContextPack(containerId, pack);
            }}
            onSelectSourceGuidance={(guidance: SourceGuidance | null) => setSourceGuidance(scopeId, guidance)}
            onSelectArtifactTarget={(selection) => setArtifactTarget(scopeId, selection)}
            onClarificationSubmit={(taskId, input) =>
              taskUserInputMutation.mutate({ taskId, input, containerId: containerId || "" })
            }
            onForceClose={() => {
              if (!containerId) return;
              const runTarget = forceCloseTargetFromBlockingRun(blockingRun);
              forceCloseMutation.mutate({
                containerId,
                mode: runTarget?.mode || mode,
                chatThreadId: runTarget?.chatThreadId || selectedChatThreadId,
                taskId: runTarget?.taskId || selectedTaskId || selectedTaskDetail.data?.task.task_id || null
              });
            }}
          />
        </div>
        <SystemSettingsDialog
          open={settingsOpen}
          selectedTab={settingsTab}
          settings={settings.data}
          diagnostics={diagnostics.data}
          onOpenChange={setSettingsOpen}
          onTabChange={setSettingsTab}
          onOpenProviderSettings={() => openSettingsTab("provider")}
          onOpenAppearanceSettings={() => openSettingsTab("appearance")}
        />
        <Dialog
          open={onboardingGuideOpen}
          onOpenChange={(_, data) => {
            if (!data.open) dismissCurrentOnboardingGuide();
          }}
        >
          <DialogSurface className="sn-onboarding-dialog">
            <DialogBody>
              <DialogTitle>{t("settings.guide")}</DialogTitle>
              <DialogContent>
                <OnboardingGuide
                  settings={settings.data}
                  variant="modal"
                  onOpenProviderSettings={openProviderSettingsFromOnboarding}
                  onOpenAppearanceSettings={openAppearanceSettingsFromOnboarding}
                  onDismiss={dismissCurrentOnboardingGuide}
                />
              </DialogContent>
            </DialogBody>
          </DialogSurface>
        </Dialog>
        {!initialStartupComplete && (
          <StartupScreen
            stages={startupStages}
            error={startupError}
            ready={startupReady}
            onEnter={() => setInitialStartupComplete(true)}
            onRetry={() => queryClient.invalidateQueries()}
          />
        )}
      </div>
    </FluentAppShell>
  );
}

function errorText(error: unknown) {
  if (!error) return "";
  return error instanceof Error ? error.message : String(error);
}

async function invalidateActiveWorkspaceQueries() {
  queryClient.removeQueries({ queryKey: ["containers"] });
  queryClient.removeQueries({ queryKey: ["container-messages"] });
  queryClient.removeQueries({ queryKey: ["chat-threads"] });
  queryClient.removeQueries({ queryKey: ["tasks"] });
  queryClient.removeQueries({ queryKey: ["context-pack"] });
  await Promise.all([
    queryClient.invalidateQueries({ queryKey: ["runtime-meta"] }),
    queryClient.invalidateQueries({ queryKey: ["diagnostics"] }),
    queryClient.invalidateQueries({ queryKey: ["workspaces"] }),
    queryClient.invalidateQueries({ queryKey: ["containers"] }),
    queryClient.invalidateQueries({ queryKey: ["workspace-containers"] }),
    queryClient.invalidateQueries({ queryKey: ["settings"] }),
    queryClient.invalidateQueries({ queryKey: ["settings", "provider-api"] }),
    queryClient.invalidateQueries({ queryKey: ["model-config"] })
  ]);
}

async function refreshContainerRailQueries() {
  await Promise.all([
    queryClient.invalidateQueries({ queryKey: ["containers"] }),
    queryClient.invalidateQueries({ queryKey: ["workspace-containers"] })
  ]);
}

function workspaceIds(workspaces?: Array<{ workspace_uid: string }>) {
  return (workspaces || []).map((workspace) => workspace.workspace_uid).join("|");
}

function startupStageState(flags: {
  runtimeReady: boolean;
  kernelReady: boolean;
  historyReady: boolean;
  settingsReady: boolean;
}): StartupStage[] {
  const raw = [
    ["startup.shell", true],
    ["startup.runtime", flags.runtimeReady],
    ["startup.kernel", flags.kernelReady],
    ["startup.history", flags.historyReady],
    ["startup.settings", flags.settingsReady],
    ["startup.ready", flags.runtimeReady && flags.kernelReady && flags.historyReady && flags.settingsReady]
  ] as const;
  const firstPending = raw.findIndex(([, ready]) => !ready);
  return raw.map(([labelKey, ready], index) => ({
    labelKey,
    status: ready ? "complete" : index === firstPending ? "active" : "pending"
  }));
}

function artifactDestinationGuidance(selection?: ArtifactTargetSelection | null): ArtifactDestinationGuidance | null {
  if (!selection?.targetDir) return null;
  return {
    semantics: "model_guidance_only",
    enforcement: "none",
    materialized_artifact: false,
    selected_output_dir: selection.targetDir,
    label: selection.label
  };
}

function contextPackForRequest(pack?: ContextPack | null): ContextPack | null {
  if (!pack) return null;
  return {
    ...pack,
    selected_items: pack.selected_items.filter((item) => item.item_kind !== "source_ref"),
    excluded_items: pack.excluded_items.filter((item) => item.item_kind !== "source_ref")
  };
}

async function refreshContainerSummaries(containerIds: Set<string>, taskIds: Set<string> = new Set()) {
  await Promise.all([
    ...Array.from(containerIds).map((containerId) =>
      queryClient.invalidateQueries({ queryKey: ["tasks", containerId] })
    ),
    ...Array.from(containerIds).map((containerId) =>
      queryClient.invalidateQueries({ queryKey: ["runs", containerId] })
    ),
    ...Array.from(taskIds).map((taskId) =>
      queryClient.invalidateQueries({ queryKey: ["task-detail", taskId] })
    )
  ]);
}

function sleep(ms: number) {
  return new Promise<void>((resolve) => window.setTimeout(resolve, ms));
}

function streamEventClosesInteraction(eventType: string) {
  return ["task.complete", "task.error", "chat.complete", "chat.error"].includes(eventType);
}

function messageShouldRefreshTaskDetail(message: ContainerMessage) {
  if (message.lane !== "task" || !message.task_id) return false;
  if (message.source_kind === "model_stream") return false;
  if (message.source_kind === "process_truth") return true;
  if (message.message_type === "approval" || message.message_type === "artifact" || message.message_type === "error") {
    return true;
  }
  return message.source_ref === "task_runtime_running" && message.status !== "streaming";
}
