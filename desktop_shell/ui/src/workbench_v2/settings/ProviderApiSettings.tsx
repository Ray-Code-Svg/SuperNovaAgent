import { useEffect, useMemo, useState } from "react";
import { Badge, Button, Field, Input, Spinner } from "@fluentui/react-components";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import type {
  AppSettings,
  ProviderApiRecord,
  ProviderApiTestResult,
  ProviderApiUpdateRequest
} from "../../protocol/generated/types";
import { useI18n } from "../i18n/i18n";
import {
  getProviderApiSettings,
  testProviderApiSettings,
  updateProviderApiSettings
} from "../protocol/settingsQueries";

export function ProviderApiSettings({ settings }: { settings?: AppSettings }) {
  const t = useI18n();
  const queryClient = useQueryClient();
  const providerQuery = useQuery({
    queryKey: ["settings", "provider-api"],
    queryFn: getProviderApiSettings
  });
  const providers = providerQuery.data?.providers ?? settings?.provider_api.providers ?? [];
  const [selectedProvider, setSelectedProvider] = useState("");
  const activeProvider = useMemo(
    () => findActiveProvider(providers, selectedProvider),
    [providers, selectedProvider]
  );
  const providerId = activeProvider?.provider ?? (selectedProvider || "deepseek");
  const [apiKey, setApiKey] = useState("");
  const [testResult, setTestResult] = useState<ProviderApiTestResult | null>(null);

  useEffect(() => {
    if (providers.length === 0) return;
    if (!selectedProvider || !providers.some((provider) => provider.provider === selectedProvider)) {
      setSelectedProvider(providers[0].provider);
    }
  }, [providers, selectedProvider]);

  useEffect(() => {
    setApiKey("");
    setTestResult(null);
  }, [activeProvider?.provider]);

  const saveMutation = useMutation({
    mutationFn: () =>
      updateProviderApiSettings(providerApiKeyUpdateRequest(providerId, apiKey)),
    onSuccess: (data) => {
      queryClient.setQueryData(["settings", "provider-api"], data);
      queryClient.invalidateQueries({ queryKey: ["settings"] });
      setApiKey("");
      setTestResult(null);
    }
  });

  const clearKeyMutation = useMutation({
    mutationFn: () =>
      updateProviderApiSettings(providerApiKeyClearRequest(providerId)),
    onSuccess: (data) => {
      queryClient.setQueryData(["settings", "provider-api"], data);
      queryClient.invalidateQueries({ queryKey: ["settings"] });
      setApiKey("");
      setTestResult(null);
    }
  });

  const testMutation = useMutation({
    mutationFn: () => testProviderApiSettings({ provider: providerId }),
    onSuccess: (result) => setTestResult(result)
  });

  return (
    <div className="sn-settings-pane sn-provider-settings">
      <div className="sn-provider-list" aria-label="Provider list">
        {providerQuery.isLoading && providers.length === 0 ? (
          <span className="sn-settings-help"><Spinner size="tiny" /> {t("settings.loadingProviders")}</span>
        ) : (
          providers.map((provider) => (
            <button
              className="sn-provider-pill"
              data-active={provider.provider === providerId}
              key={provider.provider}
              onClick={() => setSelectedProvider(provider.provider)}
              type="button"
            >
              <strong>{provider.provider}</strong>
              <Badge
                appearance="tint"
                color={provider.api_key_configured ? "success" : "warning"}
              >
                {provider.api_key_configured ? t("settings.configured") : t("settings.missingKey")}
              </Badge>
            </button>
          ))
        )}
      </div>

      <div className="sn-provider-form">
        <div className="sn-settings-row">
          <strong>{t("settings.providerField")}</strong>
          <span>{providerId}</span>
        </div>
        <div className="sn-settings-row">
          <strong>{t("settings.credential")}</strong>
          <span>{activeProvider?.credential_ref ? t("settings.configured") : t("settings.missingKey")}</span>
        </div>
        <div className="sn-settings-row">
          <strong>{t("settings.apiBaseUrl")}</strong>
          <span>{activeProvider?.api_base_url || "https://api.deepseek.com"}</span>
        </div>
        {activeProvider?.validation_status ? (
          <div className="sn-settings-row">
            <strong>{t("settings.status")}</strong>
            <span>{formatProviderStatus(activeProvider.validation_status, t)}</span>
          </div>
        ) : null}
        <Field label={t("settings.apiKey")}>
          <Input
            type="password"
            value={apiKey}
            onChange={(_, data) => setApiKey(data.value)}
            placeholder={activeProvider?.api_key_configured ? t("settings.storedKey") : t("settings.enterApiKey")}
          />
        </Field>
        <div className="sn-settings-help">
          {t("settings.credentialHelp")}
        </div>
        <div className="sn-provider-actions">
          <Button
            appearance="primary"
            disabled={saveMutation.isPending || clearKeyMutation.isPending}
            onClick={() => saveMutation.mutate()}
          >
            {t("common.save")}
          </Button>
          <Button
            disabled={testMutation.isPending || !activeProvider}
            onClick={() => testMutation.mutate()}
          >
            {t("settings.test")}
          </Button>
          <Button
            disabled={!activeProvider?.api_key_configured || clearKeyMutation.isPending}
            onClick={() => clearKeyMutation.mutate()}
          >
            {t("settings.clearKey")}
          </Button>
        </div>
        <ProviderStatus
          error={saveMutation.error || clearKeyMutation.error || testMutation.error}
          result={testResult}
          liveCheckLabel={t("settings.liveProviderCheck")}
          kernelCheckLabel={t("settings.kernelCredentialResolution")}
        />
      </div>
    </div>
  );
}

export function providerApiKeyUpdateRequest(
  provider: string,
  apiKey: string
): ProviderApiUpdateRequest {
  return {
    provider,
    api_key: apiKey.trim() ? apiKey : undefined
  };
}

export function providerApiKeyClearRequest(provider: string): ProviderApiUpdateRequest {
  return {
    provider,
    api_key: ""
  };
}

function findActiveProvider(providers: ProviderApiRecord[], selectedProvider: string) {
  if (selectedProvider) {
    const selected = providers.find((provider) => provider.provider === selectedProvider);
    if (selected) return selected;
  }
  return providers[0];
}

function formatProviderStatus(status: string, t: ReturnType<typeof useI18n>) {
  switch (status) {
    case "credential_stored":
      return t("settings.keyStored");
    case "credential_resolved":
      return t("settings.keyAvailable");
    case "credential_missing":
      return t("settings.missingKey");
    default:
      return status.replace(/_/g, " ");
  }
}

function ProviderStatus({
  error,
  result,
  liveCheckLabel,
  kernelCheckLabel
}: {
  error: unknown;
  result: ProviderApiTestResult | null;
  liveCheckLabel: string;
  kernelCheckLabel: string;
}) {
  if (error instanceof Error) {
    return <div className="sn-provider-status" data-status="error">{error.message}</div>;
  }
  if (!result) {
    return null;
  }
  return (
    <div className="sn-provider-status" data-status={result.status}>
      <strong>{result.status}</strong>
      <span>{result.message}</span>
      <small>{result.live_check_performed ? liveCheckLabel : kernelCheckLabel} via {result.checked_by}</small>
    </div>
  );
}
