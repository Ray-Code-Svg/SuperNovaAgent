import { createRuntimeClient } from "./runtimeClient";
import type {
  AppSettings,
  ContextPack,
  ModelConfig,
  ProviderApiTestRequest,
  ProviderApiUpdateRequest,
  SourceCandidateRequest
} from "../../protocol/generated/types";

export async function getModelConfig() {
  return (await createRuntimeClient()).modelConfig();
}

export async function updateModelConfig(request: ModelConfig) {
  return (await createRuntimeClient()).updateModelConfig(request);
}

export async function getContextPack(containerId: string) {
  return (await createRuntimeClient()).contextPack(containerId);
}

export async function saveContextPack(containerId: string, request: ContextPack) {
  return (await createRuntimeClient()).saveContextPack(containerId, request);
}

export async function estimateContextPack(containerId: string, request: ContextPack) {
  return (await createRuntimeClient()).estimateContextPack(containerId, request);
}

export async function listSourceCandidates(containerId: string, request: SourceCandidateRequest = {}) {
  return (await createRuntimeClient()).sourceCandidates(containerId, request);
}

export async function getSettings() {
  return (await createRuntimeClient()).settings();
}

export async function updateSettings(request: AppSettings) {
  return (await createRuntimeClient()).updateSettings(request);
}

export async function getProviderApiSettings() {
  return (await createRuntimeClient()).providerSettings();
}

export async function updateProviderApiSettings(request: ProviderApiUpdateRequest) {
  return (await createRuntimeClient()).updateProviderSettings(request);
}

export async function testProviderApiSettings(request: ProviderApiTestRequest) {
  return (await createRuntimeClient()).testProviderSettings(request);
}
