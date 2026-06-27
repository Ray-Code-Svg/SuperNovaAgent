const WINDOW_ID_STORAGE_KEY = "supernova.workbench_v2.window_id";

export function getWorkbenchWindowId(storage: Storage | null = browserSessionStorage()): string {
  const existing = safeStorageGet(storage, WINDOW_ID_STORAGE_KEY);
  if (existing) return existing;
  const generated = generateWindowId();
  safeStorageSet(storage, WINDOW_ID_STORAGE_KEY, generated);
  return generated;
}

export function scopedWindowWorkspaceKey(
  windowId: string | null | undefined,
  workspaceId: string | null | undefined
) {
  if (!windowId || !workspaceId) return null;
  return `${windowId}:${workspaceId}`;
}

export function scopedContainerStateKey(
  windowId: string | null | undefined,
  workspaceId: string | null | undefined,
  containerId: string | null | undefined
) {
  if (!containerId) return null;
  return `${windowId || "window"}:${workspaceId || "workspace"}:${containerId}`;
}

function browserSessionStorage(): Storage | null {
  if (typeof window === "undefined") return null;
  return window.sessionStorage || null;
}

function safeStorageGet(storage: Storage | null, key: string) {
  try {
    return storage?.getItem(key) || null;
  } catch {
    return null;
  }
}

function safeStorageSet(storage: Storage | null, key: string, value: string) {
  try {
    storage?.setItem(key, value);
  } catch {
    // Session storage can be unavailable in hardened webviews; the in-memory id remains valid.
  }
}

function generateWindowId() {
  const cryptoApi = typeof crypto === "undefined" ? null : crypto;
  if (cryptoApi?.randomUUID) return `window_${cryptoApi.randomUUID()}`;
  return `window_${Date.now()}_${Math.random().toString(36).slice(2)}`;
}
