import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import type { AppSettings } from "../../protocol/generated/types";
import {
  ProviderApiSettings,
  providerApiKeyClearRequest,
  providerApiKeyUpdateRequest
} from "./ProviderApiSettings";

describe("ProviderApiSettings", () => {
  it("renders the DeepSeek endpoint as read-only and does not render a base URL input", () => {
    const html = renderToStaticMarkup(
      <QueryClientProvider client={new QueryClient()}>
        <ProviderApiSettings settings={appSettings()} />
      </QueryClientProvider>
    );

    expect(html).toContain("Provider endpoint");
    expect(html).toContain("https://api.deepseek.com");
    expect(html).not.toContain("placeholder=\"https://api.deepseek.com\"");
  });

  it("does not submit api_base_url from provider API key actions", () => {
    expect(providerApiKeyUpdateRequest("deepseek", " secret ")).toEqual({
      provider: "deepseek",
      api_key: " secret "
    });
    expect(providerApiKeyUpdateRequest("deepseek", "   ")).toEqual({
      provider: "deepseek",
      api_key: undefined
    });
    expect(providerApiKeyClearRequest("deepseek")).toEqual({
      provider: "deepseek",
      api_key: ""
    });
  });
});

function appSettings(): AppSettings {
  return {
    provider_api: {
      providers: [
        {
          provider: "deepseek",
          api_base_url: "https://api.deepseek.com",
          api_key_configured: true,
          credential_ref: "kernel_credential://provider/deepseek/test",
          validation_status: "credential_stored"
        }
      ]
    },
    data_paths: {
      app_config_root: "C:/SuperNova/config",
      app_state_root: "C:/SuperNova/state",
      workspace_registry_path: "C:/SuperNova/config/workspace_registry.sqlite3"
    },
    appearance: {
      language: "en-US",
      theme: "dark"
    }
  };
}
