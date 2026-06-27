import { useEffect, useMemo, useState } from "react";
import {
  Accordion,
  AccordionHeader,
  AccordionItem,
  AccordionPanel,
  Badge,
  Button,
  Dropdown,
  Field,
  Input,
  Option
} from "@fluentui/react-components";
import type { ModelConfig, ModelConfigDescriptor } from "../../protocol/generated/types";

import { useI18n } from "../i18n/i18n";
import { WorkbenchFlyout } from "./WorkbenchFlyout";

interface ModelConfigFlyoutProps {
  descriptor?: ModelConfigDescriptor;
  busy?: boolean;
  error?: unknown;
  onSave(config: ModelConfig): void;
}

export function ModelConfigFlyout({
  descriptor,
  busy,
  error,
  onSave
}: ModelConfigFlyoutProps) {
  const t = useI18n();
  const [draft, setDraft] = useState<ModelConfig>(() => descriptor?.active || defaultModelConfig());
  const activeProvider = useMemo(
    () => descriptor?.providers.find((provider) => provider.provider === draft.provider),
    [descriptor?.providers, draft.provider]
  );

  useEffect(() => {
    if (descriptor?.active) {
      setDraft(descriptor.active);
    }
  }, [descriptor?.active]);

  const modelOptions = activeProvider?.model_options.length
    ? activeProvider.model_options
    : defaultModelOptions();
  const thinkingOptions = descriptor?.thinking_options.length ? descriptor.thinking_options : defaultThinkingOptions();
  const reasoningOptions = descriptor?.reasoning_effort_options.length
    ? descriptor.reasoning_effort_options
    : defaultReasoningOptions();
  const tokenBudgetMin = descriptor?.token_budget_min ?? 1;
  const tokenBudgetMax = descriptor?.token_budget_max ?? 131_072;

  return (
    <WorkbenchFlyout title={t("model.title")}>
      <div className="sn-config-summary">
        <strong>{t("model.route")}</strong>
        <span>{providerLabel(activeProvider, draft.provider)} / {optionLabel(modelOptions, draft.model)}</span>
        <Badge appearance="filled">
          {reasoningLabel(draft.reasoning_effort, optionLabel(reasoningOptions, draft.reasoning_effort), t)}
        </Badge>
      </div>

      <div className="sn-config-form">
        <Field label={t("model.provider")}>
          <Dropdown
            selectedOptions={[draft.provider]}
            value={providerLabel(activeProvider, draft.provider)}
            onOptionSelect={(_, data) => {
              const provider = data.optionValue || draft.provider;
              const providerDescriptor = descriptor?.providers.find((item) => item.provider === provider);
              const firstModel =
                providerDescriptor?.model_options[0]?.value || providerDescriptor?.models[0];
              setDraft((current) => ({
                ...current,
                provider,
                model: firstModel || current.model
              }));
            }}
          >
            {(descriptor?.providers || []).map((provider) => (
              <Option key={provider.provider} value={provider.provider}>
                {providerLabel(provider, provider.provider)}
              </Option>
            ))}
          </Dropdown>
        </Field>

        <Field label={t("model.model")}>
          <Dropdown
            selectedOptions={[draft.model]}
            value={optionLabel(modelOptions, draft.model)}
            onOptionSelect={(_, data) =>
              setDraft((current) => ({ ...current, model: data.optionValue || current.model }))
            }
          >
            {modelOptions.map((model) => (
              <Option key={model.value} value={model.value} text={model.label}>
                <div className="sn-model-option">
                  <span>{model.label}</span>
                  <small>{modelDescription(model.value, model.description, t)}</small>
                </div>
              </Option>
            ))}
          </Dropdown>
        </Field>

        <Accordion collapsible defaultOpenItems={[]}>
          <AccordionItem value="advanced">
            <AccordionHeader>{t("model.advanced")}</AccordionHeader>
            <AccordionPanel>
              <div className="sn-config-form">
                <Field label={t("model.thinking")}>
                  <Dropdown
                    selectedOptions={[draft.thinking]}
                    value={thinkingLabel(draft.thinking, optionLabel(thinkingOptions, draft.thinking), t)}
                    onOptionSelect={(_, data) =>
                      setDraft((current) => ({ ...current, thinking: data.optionValue || current.thinking }))
                    }
                    disabled={!activeProvider?.supports_thinking}
                  >
                    {thinkingOptions.map((option) => (
                      <Option key={option.value} value={option.value}>
                        {thinkingLabel(option.value, option.label, t)}
                      </Option>
                    ))}
                  </Dropdown>
                  <div className="sn-config-note">
                    {thinkingDescription(draft.thinking, optionDescription(thinkingOptions, draft.thinking), t)}
                  </div>
                </Field>
                <Field label={t("model.reasoningEffort")}>
                  <Dropdown
                    selectedOptions={[draft.reasoning_effort]}
                    value={reasoningLabel(draft.reasoning_effort, optionLabel(reasoningOptions, draft.reasoning_effort), t)}
                    onOptionSelect={(_, data) =>
                      setDraft((current) => ({
                        ...current,
                        reasoning_effort: data.optionValue || current.reasoning_effort
                      }))
                    }
                  >
                    {reasoningOptions.map((option) => (
                      <Option key={option.value} value={option.value}>
                        {reasoningLabel(option.value, option.label, t)}
                      </Option>
                    ))}
                  </Dropdown>
                  <div className="sn-config-note">
                    {reasoningDescription(draft.reasoning_effort, optionDescription(reasoningOptions, draft.reasoning_effort), t)}
                  </div>
                </Field>
                <Field label={t("model.maxTokenBudget")}>
                  <Input
                    type="number"
                    min={tokenBudgetMin}
                    max={tokenBudgetMax}
                    value={String(draft.token_budget ?? "")}
                    onChange={(_, data) =>
                      setDraft((current) => ({
                        ...current,
                        token_budget: data.value.trim() ? Number(data.value) : null
                      }))
                    }
                  />
                  <div className="sn-config-note">
                    {t("model.defaultBudget")} {descriptor?.token_budget_default ?? 65_536}; {t("model.allowedRange")} {tokenBudgetMin} - {tokenBudgetMax}.
                  </div>
                </Field>
                <div className="sn-config-note">
                  {t("model.strictToolsDesc")}
                </div>
              </div>
            </AccordionPanel>
          </AccordionItem>
        </Accordion>

        <div className="sn-config-actions">
          <Button appearance="primary" disabled={busy || !descriptor} onClick={() => onSave({ ...draft, strict_tools: true })}>
            {t("common.save")}
          </Button>
          <Button disabled={busy || !descriptor?.active} onClick={() => setDraft(descriptor?.active || defaultModelConfig())}>
            {t("common.reset")}
          </Button>
        </div>
        {error instanceof Error && <div className="sn-inline-error">{error.message}</div>}
      </div>
    </WorkbenchFlyout>
  );
}

function defaultModelConfig(): ModelConfig {
  return {
    provider: "deepseek",
    model: "deepseek-v4-flash",
    thinking: "auto",
    reasoning_effort: "high",
    token_budget: 65536,
    strict_tools: true
  };
}

function providerLabel(provider: { display_name?: string; provider: string } | undefined, fallback: string) {
  return provider?.display_name || fallback;
}

function optionDescription(options: Array<{ value: string; description: string }>, value: string) {
  return options.find((option) => option.value === value)?.description || "";
}

function optionLabel(options: Array<{ value: string; label: string }>, value: string) {
  return options.find((option) => option.value === value)?.label || value;
}

function thinkingLabel(value: string, fallback: string, t: ReturnType<typeof useI18n>) {
  if (value === "auto") return t("model.thinking.auto");
  if (value === "enabled") return t("model.thinking.enabled");
  if (value === "disabled") return t("model.thinking.disabled");
  return fallback;
}

function thinkingDescription(value: string, fallback: string, t: ReturnType<typeof useI18n>) {
  if (value === "auto") return t("model.thinking.autoDesc");
  if (value === "enabled") return t("model.thinking.enabledDesc");
  if (value === "disabled") return t("model.thinking.disabledDesc");
  return fallback;
}

function reasoningLabel(value: string, fallback: string, t: ReturnType<typeof useI18n>) {
  if (value === "standard") return t("model.reasoning.standard");
  if (value === "high") return t("model.reasoning.high");
  if (value === "max") return t("model.reasoning.max");
  return fallback;
}

function reasoningDescription(value: string, fallback: string, t: ReturnType<typeof useI18n>) {
  if (value === "standard") return t("model.reasoning.standardDesc");
  if (value === "high") return t("model.reasoning.highDesc");
  if (value === "max") return t("model.reasoning.maxDesc");
  return fallback;
}

function modelDescription(value: string, fallback: string, t: ReturnType<typeof useI18n>) {
  if (value === "deepseek-v4-flash") return t("model.deepseekFlashDesc");
  if (value === "deepseek-v4-pro") return t("model.deepseekProDesc");
  return fallback;
}

function defaultModelOptions() {
  return [
    {
      value: "deepseek-v4-flash",
      label: "DeepSeek V4 Flash",
      description: "Low-latency DeepSeek V4 route for everyday chat and task execution."
    },
    {
      value: "deepseek-v4-pro",
      label: "DeepSeek V4 Pro",
      description: "Higher-capability DeepSeek V4 route for complex reasoning and longer work."
    }
  ];
}

function defaultThinkingOptions() {
  return [
    { value: "auto", label: "Auto", description: "Use the runtime default thinking mode." },
    { value: "enabled", label: "Enabled", description: "Request explicit thinking when supported." },
    { value: "disabled", label: "Disabled", description: "Prefer direct answers without explicit thinking." }
  ];
}

function defaultReasoningOptions() {
  return [
    { value: "standard", label: "Standard", description: "Balanced latency and reasoning depth." },
    { value: "high", label: "High", description: "Deeper reasoning for complex tasks." },
    { value: "max", label: "Max", description: "Maximum reasoning depth for difficult tasks." }
  ];
}
