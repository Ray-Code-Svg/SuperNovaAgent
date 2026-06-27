import type { WorkbenchFlyout } from "../state/uiStore";

export function flyoutForDraft(value: string): WorkbenchFlyout {
  const trimmed = value.trimStart();
  if (trimmed.startsWith("/")) return "slash";
  if (lastToken(value).startsWith("@")) return "source";
  if (lastToken(value).startsWith("$")) return "artifact";
  return null;
}

function lastToken(value: string) {
  const parts = value.split(/\s+/);
  return parts[parts.length - 1] || "";
}
