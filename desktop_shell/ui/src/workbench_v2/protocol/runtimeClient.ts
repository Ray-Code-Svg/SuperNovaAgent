import { openUrl } from "@tauri-apps/plugin-opener";
import { LocalRuntimeProtocolClient } from "../../protocol/generated/client";

type InvokeFn = <T>(command: string, args?: Record<string, unknown>) => Promise<T>;

declare global {
  interface Window {
    __TAURI__?: {
      core?: {
        invoke?: InvokeFn;
      };
    };
  }
}

interface ShellRuntimeResponse {
  runtime?: {
    base_url?: string;
    runtime_token?: string;
  } | null;
}

interface ExternalOpenResponse {
  schema_version: string;
  shell_layer: string;
  status: string;
  url: string;
}

interface RuntimeAccess {
  baseUrl: string;
  runtimeToken: string;
}

let cachedRuntimeAccess: RuntimeAccess | null = null;

async function resolveRuntimeAccess(): Promise<RuntimeAccess> {
  if (cachedRuntimeAccess) {
    return cachedRuntimeAccess;
  }
  if (import.meta.env.VITE_SUPERNOVA_RUNTIME_URL) {
    const runtimeToken = String(import.meta.env.VITE_SUPERNOVA_RUNTIME_TOKEN || "").trim();
    if (!runtimeToken) {
      throw new Error("VITE_SUPERNOVA_RUNTIME_TOKEN is required when VITE_SUPERNOVA_RUNTIME_URL is set.");
    }
    cachedRuntimeAccess = {
      baseUrl: String(import.meta.env.VITE_SUPERNOVA_RUNTIME_URL),
      runtimeToken
    };
    return cachedRuntimeAccess;
  }
  const invoke = window.__TAURI__?.core?.invoke;
  if (invoke) {
    const status = await invoke<ShellRuntimeResponse>("runtime_ensure");
    const baseUrl = status.runtime?.base_url;
    const runtimeToken = status.runtime?.runtime_token;
    if (baseUrl && runtimeToken) {
      cachedRuntimeAccess = { baseUrl, runtimeToken };
      return cachedRuntimeAccess;
    }
  }
  throw new Error("SuperNova runtime token is unavailable. Start the desktop shell or set VITE_SUPERNOVA_RUNTIME_TOKEN.");
}

export async function resolveRuntimeBaseUrl() {
  return (await resolveRuntimeAccess()).baseUrl;
}

export async function createRuntimeClient() {
  const runtime = await resolveRuntimeAccess();
  return new LocalRuntimeProtocolClient({
    baseUrl: runtime.baseUrl,
    runtimeToken: runtime.runtimeToken
  });
}

export async function invokeShell<T>(command: string, args?: Record<string, unknown>): Promise<T | null> {
  const invoke = window.__TAURI__?.core?.invoke;
  if (!invoke) {
    return null;
  }
  return invoke<T>(command, args);
}

export async function openExternalUrl(url: string): Promise<ExternalOpenResponse> {
  const invoke = window.__TAURI__?.core?.invoke;
  if (invoke) {
    await openUrl(url);
    return {
      schema_version: "tauri_plugin_opener",
      shell_layer: "tauri_desktop_shell",
      status: "opened",
      url
    };
  }
  window.open(url, "_blank", "noopener,noreferrer");
  return {
    schema_version: "browser_dev_fallback",
    shell_layer: "browser",
    status: "opened",
    url
  };
}
