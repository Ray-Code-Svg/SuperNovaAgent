import { Button } from "@fluentui/react-components";
import { BotRegular, ClipboardTaskRegular, DatabaseRegular, WindowSettingsRegular } from "@fluentui/react-icons";
import type {
  ArtifactTargetOption,
  ContainerMessage,
  ContainerRecord,
  ContextPack,
  ContextPackEstimate,
  ModelConfig,
  ModelConfigDescriptor,
  SourceGuidance,
  TaskDetail,
  UiCapabilityManifest
} from "../../protocol/generated/types";
import { AgentChatSurface } from "../chat/AgentChatSurface";
import { CommandComposer } from "../composer/CommandComposer";
import { ArtifactTargetFlyout } from "../flyouts/ArtifactTargetFlyout";
import { ContextConfigFlyout } from "../flyouts/ContextConfigFlyout";
import { ModelConfigFlyout } from "../flyouts/ModelConfigFlyout";
import { SlashCommandFlyout } from "../flyouts/SlashCommandFlyout";
import { SourcePickerFlyout } from "../flyouts/SourcePickerFlyout";
import { AgentTaskSurface } from "../task/AgentTaskSurface";
import { useI18n } from "../i18n/i18n";
import type { ArtifactTargetSelection, ContainerMode } from "../state/uiStore";
import { useWorkbenchUiStore } from "../state/uiStore";

interface ActiveContainerSurfaceProps {
  container?: ContainerRecord;
  scopeId: string | null;
  messages: ContainerMessage[];
  capabilities?: UiCapabilityManifest;
  contextPack?: ContextPack | null;
  selectedSourceGuidance?: SourceGuidance | null;
  modelConfig?: ModelConfigDescriptor;
  selectedTaskId?: string | null;
  selectedTaskDetail?: TaskDetail;
  selectedArtifactTarget?: ArtifactTargetSelection | null;
  artifactTargetOptions?: ArtifactTargetOption[];
  busy?: boolean;
  forceCloseVisible?: boolean;
  forceCloseDisabled?: boolean;
  modelConfigBusy?: boolean;
  contextConfigBusy?: boolean;
  modelConfigError?: unknown;
  contextConfigError?: unknown;
  onSubmit(value: string, modeAtSubmit: ContainerMode): void;
  onModelConfigSave(config: ModelConfig): void;
  onContextPackSave(pack: ContextPack): void;
  onContextPackEstimate(pack: ContextPack): Promise<ContextPackEstimate>;
  onSelectSourceGuidance(guidance: SourceGuidance | null): void;
  onSelectArtifactTarget(selection: ArtifactTargetSelection | null): void;
  onClarificationSubmit(taskId: string, input: string): void;
  onForceClose(): void;
}

export function ActiveContainerSurface({
  container,
  scopeId,
  messages,
  capabilities,
  contextPack,
  selectedSourceGuidance,
  modelConfig,
  selectedTaskId,
  selectedTaskDetail,
  selectedArtifactTarget,
  artifactTargetOptions = [],
  busy,
  forceCloseVisible,
  forceCloseDisabled,
  modelConfigBusy,
  contextConfigBusy,
  modelConfigError,
  contextConfigError,
  onSubmit,
  onModelConfigSave,
  onContextPackSave,
  onContextPackEstimate,
  onSelectSourceGuidance,
  onSelectArtifactTarget,
  onClarificationSubmit,
  onForceClose
}: ActiveContainerSurfaceProps) {
  const t = useI18n();
  const containerId = container?.container_id || null;
  const mode = useWorkbenchUiStore((state) => state.mode(scopeId));
  const draft = useWorkbenchUiStore((state) => state.draft(scopeId));
  const setDraft = useWorkbenchUiStore((state) => state.setDraft);
  const openFlyout = useWorkbenchUiStore((state) => state.openFlyout);
  const setOpenFlyout = useWorkbenchUiStore((state) => state.setOpenFlyout);
  const activeModelLabel = modelLabel(modelConfig, t);
  const artifactTarget = artifactTargetOptions.find((item) => item.target_id === selectedArtifactTarget?.targetId);
  const contextUsage = latestContextWindowUsage(messages, mode, modelConfig);
  const showForceClose = Boolean(forceCloseVisible ?? busy);

  return (
    <main className="sn-active-container">
      <div className="sn-container-commandbar">
        <div>
          <strong>{container?.title || t("container.none")}</strong>
          <span>{container?.container_id || t("container.selectOrCreate")}</span>
        </div>
        <div className="sn-commandbar-actions">
          <Button appearance="subtle" icon={<BotRegular />} onClick={() => setOpenFlyout("model")}>
            {activeModelLabel}
          </Button>
          <Button appearance="subtle" icon={<DatabaseRegular />} onClick={() => setOpenFlyout("context")}>
            {t("container.context")}
          </Button>
        </div>
      </div>
      <div className="sn-stream-frame" data-mode={mode}>
        {mode === "chat" ? (
          <AgentChatSurface messages={messages} />
        ) : (
          <AgentTaskSurface
            approvalBusy={busy}
            messages={messages}
            selectedTaskId={selectedTaskId}
            selectedTaskDetail={selectedTaskDetail}
            onClarificationSubmit={onClarificationSubmit}
          />
        )}
        <div className="sn-context-meter" title={contextUsage?.detailTitle}>
          <ClipboardTaskRegular />
          <span>{mode === "chat" ? t("container.modeChat") : t("container.modeTask")} {t("container.contextEstimate")}</span>
          <strong>{contextUsage?.label ?? contextPack?.estimated_tokens ?? 0}</strong>
          {contextUsage?.ratioLabel && <em>{contextUsage.ratioLabel}</em>}
        </div>
      </div>
      <div className="sn-composer-area">
        {selectedSourceGuidance && (
          <div className="sn-composer-selection sn-composer-selection-reference" aria-label={t("source.selectedSourcesAria")}>
            <button type="button" onClick={() => setOpenFlyout("source")}>
              <span>@</span>
              <strong>{sourceGuidanceLabel(selectedSourceGuidance, t)}</strong>
              <em>{t("composer.referenceGuidance")}</em>
            </button>
            <Button appearance="subtle" size="small" onClick={() => onSelectSourceGuidance(null)}>
              {t("common.clear")}
            </Button>
          </div>
        )}
        {selectedArtifactTarget && (
          <div className="sn-composer-selection" aria-label={t("artifact.title")}>
            <button type="button" onClick={() => setOpenFlyout("artifact")}>
              <span>$</span>
              <strong>{selectedArtifactTarget.label || artifactTarget?.label || selectedArtifactTarget.targetId}</strong>
              <em>{selectedArtifactTarget.targetDir || artifactTarget?.target_dir || t("composer.workspaceOutput")}</em>
            </button>
            <Button appearance="subtle" size="small" onClick={() => onSelectArtifactTarget(null)}>
              {t("common.clear")}
            </Button>
          </div>
        )}
        <CommandComposer
          containerId={containerId}
          scopeId={scopeId}
          disabled={!container || busy}
          forceCloseVisible={showForceClose}
          forceCloseDisabled={forceCloseDisabled || !showForceClose}
          onForceClose={onForceClose}
          onSubmit={onSubmit}
        />
        <Button appearance="subtle" icon={<WindowSettingsRegular />} onClick={() => setOpenFlyout("model")} />
      </div>
      {openFlyout && (
        <div className="sn-flyout-anchor">
          {openFlyout === "slash" && <SlashCommandFlyout scopeId={scopeId} capabilities={capabilities} />}
          {openFlyout === "source" && (
            <SourcePickerFlyout
              containerId={containerId}
              selectedGuidance={selectedSourceGuidance}
              initialQuery={sourceQueryFromDraft(draft)}
              busy={contextConfigBusy}
              error={contextConfigError}
              onSave={(guidance) => {
                onSelectSourceGuidance(guidance);
                setDraft(scopeId, clearComposerToken(draft, "@"));
              }}
            />
          )}
          {openFlyout === "artifact" && (
            <ArtifactTargetFlyout
              containerId={containerId}
              selectedSelection={selectedArtifactTarget}
              onSelectTarget={(selection) => {
                onSelectArtifactTarget(selection);
                if (selection) setDraft(scopeId, clearComposerToken(draft, "$"));
              }}
            />
          )}
          {openFlyout === "model" && (
            <ModelConfigFlyout
              descriptor={modelConfig}
              busy={modelConfigBusy}
              error={modelConfigError}
              onSave={onModelConfigSave}
            />
          )}
          {openFlyout === "context" && (
            <ContextConfigFlyout
              containerId={containerId}
              contextPack={contextPack}
              busy={contextConfigBusy}
              error={contextConfigError}
              onEstimate={onContextPackEstimate}
              onSave={onContextPackSave}
            />
          )}
        </div>
      )}
    </main>
  );
}

function sourceQueryFromDraft(value: string) {
  const token = value.split(/\s+/).reverse().find((part) => part.startsWith("@"));
  return token ? token.replace(/^@+/, "") : "";
}

function clearComposerToken(value: string, prefix: string) {
  const trimmedEnd = value.trimEnd();
  if (!trimmedEnd) return value;
  const boundary = trimmedEnd.lastIndexOf(" ");
  const lastToken = boundary >= 0 ? trimmedEnd.slice(boundary + 1) : trimmedEnd;
  if (!lastToken.startsWith(prefix)) return value;
  return boundary >= 0 ? trimmedEnd.slice(0, boundary).trimEnd() : "";
}

function sourceGuidanceLabel(guidance: SourceGuidance, t: ReturnType<typeof useI18n>) {
  const sources = guidance.selected_sources || [];
  if (sources.length === 0) return t("source.noSources");
  if (sources.length === 1) return sources[0].label || sources[0].ref_id.replace(/^workspace:\/\//, "");
  return `${sources.length} ${t("source.selectedSources")}`;
}

function modelLabel(descriptor: ModelConfigDescriptor | undefined, t: ReturnType<typeof useI18n>) {
  if (!descriptor) return t("container.modelFallback");
  const activeModel = descriptor.active.model;
  return (
    descriptor.providers
      .find((provider) => provider.provider === descriptor.active.provider)
      ?.model_options.find((option) => option.value === activeModel)?.label || activeModel
  );
}

function latestContextWindowUsage(
  messages: ContainerMessage[],
  mode: "chat" | "task",
  modelConfig?: ModelConfigDescriptor
) {
  const lane = mode === "chat" ? "chat" : "task";
  for (const message of [...messages].reverse()) {
    if (message.lane !== lane) continue;
    const estimate = contextWindowEstimateFromMessage(message);
    if (!estimate) continue;
    const used = estimate.estimated_total_tokens || estimate.estimated_input_tokens;
    if (!used || !estimate.context_window_tokens) continue;
    const contextWindowTokens = normalizedContextWindowTokens(estimate.context_window_tokens, modelConfig);
    const ratio = contextWindowTokens > 0
      ? used / contextWindowTokens
      : typeof estimate.usage_ratio === "number"
      ? estimate.usage_ratio
      : 0;
    return {
      label: `~${formatTokenCount(used)} / ${formatTokenCount(contextWindowTokens)}`,
      ratioLabel: `${Math.max(1, Math.round(ratio * 100))}% last request`,
      detailTitle: contextUsageDetail(estimate, contextWindowTokens)
    };
  }
  return null;
}

function contextWindowEstimateFromMessage(message: ContainerMessage) {
  const body = message.body_json as { estimate?: unknown } | null;
  if (!body || typeof body !== "object" || !("estimate" in body)) return null;
  const estimate = body.estimate as Record<string, unknown>;
  const contextWindowTokens = numberValue(estimate.context_window_tokens);
  const estimatedInputTokens = numberValue(estimate.estimated_input_tokens);
  const estimatedTotalTokens = numberValue(estimate.estimated_total_tokens);
  const usageRatio = numberValue(estimate.usage_ratio);
  const breakdown = estimate.breakdown && typeof estimate.breakdown === "object"
    ? estimate.breakdown as Record<string, unknown>
    : {};
  if (!contextWindowTokens || (!estimatedInputTokens && !estimatedTotalTokens)) return null;
  return {
    context_window_tokens: contextWindowTokens,
    estimated_input_tokens: estimatedInputTokens,
    estimated_total_tokens: estimatedTotalTokens,
    breakdown,
    usage_ratio: usageRatio
  };
}

function numberValue(value: unknown) {
  return typeof value === "number" && Number.isFinite(value) ? value : 0;
}

function formatTokenCount(value: number) {
  if (value >= 1_000_000) return `${(value / 1_000_000).toFixed(value >= 10_000_000 ? 0 : 1)}M`;
  if (value >= 1_000) return `${Math.round(value / 1_000)}k`;
  return String(value);
}

function normalizedContextWindowTokens(value: number, modelConfig?: ModelConfigDescriptor) {
  const provider = modelConfig?.active.provider.toLowerCase() || "";
  const model = modelConfig?.active.model.toLowerCase() || "";
  if ((provider.includes("deepseek") || model.includes("deepseek")) && value > 1_000_000) {
    return 1_000_000;
  }
  return value;
}

function contextUsageDetail(
  estimate: ReturnType<typeof contextWindowEstimateFromMessage>,
  displayedContextWindowTokens: number
) {
  if (!estimate) return undefined;
  const rows = [
    `Last request estimate: ${estimate.estimated_total_tokens || estimate.estimated_input_tokens}`,
    `Displayed model context window: ${displayedContextWindowTokens}`,
  ];
  if (displayedContextWindowTokens !== estimate.context_window_tokens) {
    rows.push(`Recorded event context window: ${estimate.context_window_tokens}`);
  }
  const breakdown = Object.entries(estimate.breakdown || {})
    .filter(([, value]) => typeof value === "number" && value > 0)
    .map(([key, value]) => `${key}: ${value}`);
  if (breakdown.length > 0) rows.push(`Breakdown: ${breakdown.join(", ")}`);
  return rows.join("\n");
}
