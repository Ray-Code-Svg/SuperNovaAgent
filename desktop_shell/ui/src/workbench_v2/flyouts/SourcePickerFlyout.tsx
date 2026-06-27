import { useEffect, useMemo, useState } from "react";
import { Badge, Button, Field, Input, Spinner } from "@fluentui/react-components";
import { DismissRegular, DocumentRegular, FolderRegular, OpenFolderRegular } from "@fluentui/react-icons";
import { useQuery } from "@tanstack/react-query";
import type { ReferenceSourceDirective, SourceCandidate, SourceGuidance } from "../../protocol/generated/types";

import { useI18n } from "../i18n/i18n";
import { listSourceCandidates } from "../protocol/settingsQueries";
import { WorkbenchFlyout } from "./WorkbenchFlyout";

interface SourcePickerFlyoutProps {
  containerId: string | null;
  selectedGuidance?: SourceGuidance | null;
  initialQuery?: string;
  busy?: boolean;
  error?: unknown;
  onSave(guidance: SourceGuidance | null): void;
}

type SourceFilter = "all" | "folder" | "file" | "history";

interface SourceGroup {
  key: string;
  title: string;
  items: SourceCandidate[];
}

export function SourcePickerFlyout({
  containerId,
  selectedGuidance,
  initialQuery,
  busy,
  error,
  onSave
}: SourcePickerFlyoutProps) {
  const t = useI18n();
  const [query, setQuery] = useState(initialQuery || "");
  const [sourceFilter, setSourceFilter] = useState<SourceFilter>("all");
  const [selectedItems, setSelectedItems] = useState<ReferenceSourceDirective[]>(selectedGuidance?.selected_sources || []);
  const candidates = useQuery({
    queryKey: ["source-candidates", containerId, query],
    queryFn: () => listSourceCandidates(containerId || "", { q: query || null, limit: 200 }),
    enabled: Boolean(containerId)
  });
  const selectedKeys = useMemo(
    () => new Set(selectedItems.map((item) => itemKey(item))),
    [selectedItems]
  );

  useEffect(() => {
    setSelectedItems(selectedGuidance?.selected_sources || []);
  }, [selectedGuidance?.selected_sources]);

  useEffect(() => {
    setQuery(initialQuery || "");
  }, [initialQuery]);

  function toggle(candidate: SourceCandidate) {
    const key = candidateKey(candidate);
    setSelectedItems((current) => {
      if (current.some((item) => itemKey(item) === key)) {
        return current.filter((item) => itemKey(item) !== key);
      }
      return [...current, candidateToDirective(candidate)];
    });
  }

  function removeSelected(item: ReferenceSourceDirective) {
    const key = itemKey(item);
    setSelectedItems((current) => current.filter((selected) => itemKey(selected) !== key));
  }

  function clearSelected() {
    setSelectedItems([]);
  }

  function save() {
    if (!containerId) return;
    onSave(selectedItems.length > 0 ? defaultSourceGuidance(selectedItems) : null);
  }

  const items = candidates.data?.items || [];
  const filteredItems = useMemo(
    () => items.filter((candidate) => filterCandidate(candidate, sourceFilter)),
    [items, sourceFilter]
  );
  const groupedItems = useMemo(() => groupSourceCandidates(filteredItems, t), [filteredItems, t]);

  return (
    <WorkbenchFlyout title={t("source.title")}>
      <div className="sn-source-picker">
        <section className="sn-source-selected-panel" aria-label={t("source.selectedSourcesAria")}>
          <div className="sn-source-section-title">
            <strong>{t("source.selectedSources")}</strong>
            <Badge appearance={selectedItems.length > 0 ? "filled" : "outline"}>{selectedItems.length}</Badge>
            {selectedItems.length > 0 && (
              <Button appearance="subtle" size="small" onClick={clearSelected}>
                {t("common.clear")}
              </Button>
            )}
          </div>
          <div className="sn-source-token-list">
            {selectedItems.length === 0 ? (
              <span>{t("source.noSourcesSelected")}</span>
            ) : (
              selectedItems.map((item) => (
                <button className="sn-source-token" key={itemKey(item)} onClick={() => removeSelected(item)} type="button">
                  <span>{item.label || item.ref_id}</span>
                  <DismissRegular />
                </button>
              ))
            )}
          </div>
        </section>

        <div className="sn-config-form">
        <Field label={t("source.searchLabel")}>
          <Input
            value={query}
            onChange={(_, data) => setQuery(data.value)}
            placeholder={t("source.searchPlaceholder")}
          />
        </Field>
          <div className="sn-source-filters" aria-label={t("source.filterAria")}>
            {(["all", "folder", "file", "history"] as SourceFilter[]).map((filter) => (
              <Button
                appearance={sourceFilter === filter ? "primary" : "secondary"}
                key={filter}
                size="small"
                onClick={() => setSourceFilter(filter)}
              >
                {filterLabel(filter, t)}
              </Button>
            ))}
          </div>
          <div className="sn-config-note">
            {filteredItems.length} {t("source.visibleCandidates")} / {items.length} {t("source.totalResults")}.
          </div>
        </div>
      </div>

      {candidates.isLoading && <div className="sn-flyout-empty"><Spinner size="tiny" /> {t("source.searching")}</div>}
      {!candidates.isLoading && filteredItems.length === 0 && (
        <div className="sn-flyout-empty">
          <OpenFolderRegular />
          <span>{t("source.noMatching")}</span>
        </div>
      )}
      <div className="sn-flyout-list sn-source-results">
        {groupedItems.map((group) => (
          <section className="sn-source-group" key={group.key}>
            <div className="sn-source-group-title">{group.title}</div>
            {group.items.map((candidate) => {
              const selected = selectedKeys.has(candidateKey(candidate));
              const label = sourceLabel(candidate);
              const name = sourceName(label);
              const path = sourcePath(label);
              const depth = sourceDepth(label);
          return (
            <button
              className="sn-target-option sn-source-option"
              data-selected={selected}
              key={candidateKey(candidate)}
              onClick={() => toggle(candidate)}
                  style={{ paddingLeft: `${10 + depth * 12}px` }}
              type="button"
            >
                  {candidate.source_kind === "workspace_dir" ? <FolderRegular /> : <DocumentRegular />}
                  <div className="sn-source-option-main">
                    {path && <span className="sn-source-path">{path}</span>}
                    <strong className="sn-source-name">{name}</strong>
                <span>{sourceDetailLabel(candidate.detail, candidate.source_kind, t)}</span>
              </div>
              <Badge appearance={selected ? "filled" : "outline"}>
                {selected ? t("common.selected") : sourceKindLabel(candidate.source_kind, t)}
              </Badge>
            </button>
          );
            })}
          </section>
        ))}
      </div>
      <div className="sn-config-actions">
        <Button appearance="primary" disabled={!containerId || busy} onClick={save}>
          {t("source.useSources")}
        </Button>
      </div>
      {error instanceof Error && <div className="sn-inline-error">{error.message}</div>}
    </WorkbenchFlyout>
  );
}

function itemKey(item: ReferenceSourceDirective) {
  return `${item.source_kind}:${item.ref_id}`;
}

function candidateKey(candidate: SourceCandidate) {
  return `${candidate.source_kind}:${candidate.item.ref_id}`;
}

function candidateToDirective(candidate: SourceCandidate): ReferenceSourceDirective {
  return {
    source_kind: candidate.source_kind,
    ref_id: candidate.item.ref_id,
    label: candidate.item.label || candidate.item.ref_id.replace(/^workspace:\/\//, ""),
    usage: "primary_reference_scope",
    include_mode: "reference_only",
    selection_source: "composer_at_token"
  };
}

function defaultSourceGuidance(selectedSources: ReferenceSourceDirective[]): SourceGuidance {
  return {
    semantics: "model_guidance_only",
    materialized_content: false,
    source_scope_enforcement: "none",
    selected_sources: selectedSources,
    user_intent: "Prefer these reference sources when relevant. Inspect file contents or directory listings with workspace read capabilities before making claims."
  };
}

function filterLabel(filter: SourceFilter, t: ReturnType<typeof useI18n>) {
  if (filter === "folder") return t("source.filterFolders");
  if (filter === "file") return t("source.filterFiles");
  if (filter === "history") return t("source.filterHistory");
  return t("source.filterAll");
}

function filterCandidate(candidate: SourceCandidate, filter: SourceFilter) {
  if (filter === "all") return true;
  if (filter === "folder") return candidate.source_kind === "workspace_dir";
  if (filter === "file") return candidate.source_kind === "workspace_file";
  return candidate.source_kind === "history";
}

function groupSourceCandidates(candidates: SourceCandidate[], t: ReturnType<typeof useI18n>): SourceGroup[] {
  const groups = new Map<string, SourceGroup>();
  for (const candidate of candidates) {
    const label = sourceLabel(candidate);
    const groupTitle = candidate.source_kind === "history" ? t("source.historyGroup") : topSegment(label, t);
    const key = candidate.source_kind === "history" ? "history" : `workspace:${groupTitle}`;
    if (!groups.has(key)) {
      groups.set(key, { key, title: groupTitle, items: [] });
    }
    groups.get(key)?.items.push(candidate);
  }
  return Array.from(groups.values())
    .map((group) => ({
      ...group,
      items: group.items.sort((left, right) => sourceLabel(left).localeCompare(sourceLabel(right)))
    }))
    .sort((left, right) => left.title.localeCompare(right.title));
}

function sourceLabel(candidate: SourceCandidate) {
  return candidate.item.label || candidate.item.ref_id.replace(/^workspace:\/\//, "");
}

function sourceName(label: string) {
  const parts = label.split("/").filter(Boolean);
  return parts[parts.length - 1] || label;
}

function sourcePath(label: string) {
  const parts = label.split("/").filter(Boolean);
  parts.pop();
  return parts.join("/");
}

function sourceDepth(label: string) {
  return Math.max(0, label.split("/").filter(Boolean).length - 1);
}

function topSegment(label: string, t: ReturnType<typeof useI18n>) {
  return label.split("/").filter(Boolean)[0] || t("source.workspaceRoot");
}

function sourceKindLabel(sourceKind: string, t: ReturnType<typeof useI18n>) {
  if (sourceKind === "workspace_dir") return t("source.kindWorkspaceDir");
  if (sourceKind === "workspace_file") return t("source.kindWorkspaceFile");
  if (sourceKind === "history") return t("source.kindHistory");
  return sourceKind;
}

function sourceDetailLabel(detail: string | null | undefined, sourceKind: string, t: ReturnType<typeof useI18n>) {
  if (!detail) return sourceKindLabel(sourceKind, t);
  const normalized = detail.trim().toLowerCase();
  if (normalized === "file") return t("source.kindWorkspaceFile");
  if (normalized === "directory" || normalized === "folder") return t("source.kindWorkspaceDir");
  if (normalized === "chat thread") return t("source.detailChatThread");
  if (normalized === "task thread") return t("source.detailTaskThread");
  if (normalized === "interrupted") return t("source.detailInterrupted");
  if (normalized === "completed") return t("source.detailCompleted");
  return detail;
}
