import { useEffect, useState } from "react";
import { Badge, Button, Dropdown, Field, Input, Option, Switch } from "@fluentui/react-components";
import { DismissRegular } from "@fluentui/react-icons";
import type { ContextPack, ContextPackEstimate, ContextPackItem } from "../../protocol/generated/types";

import { useI18n } from "../i18n/i18n";
import { WorkbenchFlyout } from "./WorkbenchFlyout";

interface ContextConfigFlyoutProps {
  containerId: string | null;
  contextPack?: ContextPack | null;
  busy?: boolean;
  error?: unknown;
  onEstimate(pack: ContextPack): Promise<ContextPackEstimate>;
  onSave(pack: ContextPack): void;
}

export function ContextConfigFlyout({
  containerId,
  contextPack,
  busy,
  error,
  onEstimate,
  onSave
}: ContextConfigFlyoutProps) {
  const t = useI18n();
  const [draft, setDraft] = useState<ContextPack>(() => sanitizeContextPack(contextPack || defaultContextPack(containerId)));
  const [estimate, setEstimate] = useState<ContextPackEstimate | null>(null);
  const [estimateBusy, setEstimateBusy] = useState(false);
  const selectedCount = draft.selected_items.length;

  useEffect(() => {
    setDraft(sanitizeContextPack(contextPack || defaultContextPack(containerId)));
    setEstimate(null);
  }, [containerId, contextPack]);

  async function estimateDraft() {
    if (!containerId) return;
    setEstimateBusy(true);
    try {
      const result = await onEstimate(sanitizeContextPack({ ...draft, container_id: containerId }));
      setEstimate(result);
      setDraft(sanitizeContextPack(result.context_pack));
    } finally {
      setEstimateBusy(false);
    }
  }

  function removeContextItem(item: ContextPackItem) {
    const key = itemKey(item);
    setDraft((current) => ({
      ...current,
      selected_items: current.selected_items.filter((selected) => itemKey(selected) !== key)
    }));
  }

  function clearContextItems() {
    setDraft((current) => ({
      ...current,
      selected_items: []
    }));
  }

  function updateContextItemMode(item: ContextPackItem, includeMode: string) {
    const key = itemKey(item);
    setDraft((current) => ({
      ...current,
      selected_items: current.selected_items.map((selected) =>
        itemKey(selected) === key ? { ...selected, include_mode: includeMode } : selected
      )
    }));
  }

  return (
    <WorkbenchFlyout title={t("context.title")}>
      <div className="sn-config-summary">
        <strong>{draft.context_pack_id || t("context.defaultPack")}</strong>
        <span>
          {selectedCount} {t("context.items")} / {draft.auto_policy.include_recent_chat_turns}{" "}
          {t("context.recentChatTurns")} + {draft.auto_policy.include_recent_tasks} {t("context.recentTaskRuns")}
        </span>
        <Badge>{estimate?.estimated_tokens ?? draft.estimated_tokens ?? 0} {t("common.tokens")}</Badge>
      </div>

      <div className="sn-config-form">
        <Field label={t("context.recentChatTurns")}>
          <Input
            type="number"
            min={0}
            max={50}
            value={String(draft.auto_policy.include_recent_chat_turns)}
            onChange={(_, data) =>
              setDraft((current) => ({
                ...current,
                auto_policy: {
                  ...current.auto_policy,
                  include_recent_chat_turns: clampNumber(data.value, 0, 50)
                }
              }))
            }
          />
        </Field>
        <Field label={t("context.recentTaskRuns")}>
          <Input
            type="number"
            min={0}
            max={30}
            value={String(draft.auto_policy.include_recent_tasks)}
            onChange={(_, data) =>
              setDraft((current) => ({
                ...current,
                auto_policy: {
                  ...current.auto_policy,
                  include_recent_tasks: clampNumber(data.value, 0, 30)
                }
              }))
            }
          />
        </Field>
        <Switch
          checked={draft.auto_policy.prefer_summaries}
          label={t("context.preferSummaries")}
          onChange={(_, data) =>
            setDraft((current) => ({
              ...current,
              auto_policy: { ...current.auto_policy, prefer_summaries: data.checked }
            }))
          }
        />

        <div className="sn-context-source-list" aria-label={t("context.items")}>
          <div className="sn-context-source-list-header">
            <strong>{t("context.items")}</strong>
            {selectedCount > 0 && (
              <Button appearance="subtle" size="small" onClick={clearContextItems}>
                {t("context.clearAll")}
              </Button>
            )}
          </div>
          {draft.selected_items.length === 0 ? (
            <span>{t("context.noExplicitItems")}</span>
          ) : (
            draft.selected_items.map((item) => (
              <div className="sn-context-source-row" key={`${item.item_kind}:${item.ref_id}`}>
                <div className="sn-context-source-main">
                  <strong>{item.label || item.ref_id}</strong>
                  <span>{item.item_kind}</span>
                </div>
                <div className="sn-context-source-actions">
                  <Dropdown
                    className="sn-context-source-mode"
                    selectedOptions={[item.include_mode]}
                    size="small"
                    value={includeModeLabel(item.include_mode, t)}
                    onOptionSelect={(_, data) => updateContextItemMode(item, data.optionValue || item.include_mode)}
                  >
                    <Option value="summary">{t("context.summary")}</Option>
                    <Option value="ref_only">{t("context.refOnly")}</Option>
                    <Option value="full">{t("context.full")}</Option>
                  </Dropdown>
                  <Button
                    appearance="subtle"
                    aria-label={`${t("context.remove")} ${item.label || item.ref_id}`}
                    icon={<DismissRegular />}
                    size="small"
                    onClick={() => removeContextItem(item)}
                  >
                    {t("context.remove")}
                  </Button>
                </div>
              </div>
            ))
          )}
        </div>

        <div className="sn-config-actions">
          <Button disabled={!containerId || estimateBusy || busy} onClick={estimateDraft}>
            {t("context.estimate")}
          </Button>
          <Button
            appearance="primary"
            disabled={!containerId || busy}
            onClick={() => containerId && onSave(sanitizeContextPack({ ...draft, container_id: containerId }))}
          >
            {t("common.save")}
          </Button>
        </div>
        {estimate && (
          <div className="sn-config-note">
            {t("context.window")}: {estimate.context_window_tokens} / {t("context.usage")} {estimate.usage_ratio}
          </div>
        )}
        {error instanceof Error && <div className="sn-inline-error">{error.message}</div>}
      </div>
    </WorkbenchFlyout>
  );
}

function defaultContextPack(containerId: string | null): ContextPack {
  return {
    context_pack_id: "",
    container_id: containerId || "",
    selected_items: [],
    excluded_items: [],
    auto_policy: {
      include_recent_chat_turns: 6,
      include_recent_tasks: 3,
      prefer_summaries: true
    },
    summary_ref: null,
    estimated_tokens: 0
  };
}

function sanitizeContextPack(pack: ContextPack): ContextPack {
  return {
    ...pack,
    selected_items: pack.selected_items.filter((item) => item.item_kind !== "source_ref"),
    excluded_items: pack.excluded_items.filter((item) => item.item_kind !== "source_ref")
  };
}

function clampNumber(value: string, min: number, max: number) {
  const parsed = Number(value);
  if (!Number.isFinite(parsed)) return min;
  return Math.max(min, Math.min(max, parsed));
}

function itemKey(item: ContextPackItem) {
  return `${item.item_kind}:${item.ref_id}`;
}

function includeModeLabel(value: string, t: ReturnType<typeof useI18n>) {
  if (value === "ref_only") return t("context.refOnly");
  if (value === "full") return t("context.full");
  return t("context.summary");
}
