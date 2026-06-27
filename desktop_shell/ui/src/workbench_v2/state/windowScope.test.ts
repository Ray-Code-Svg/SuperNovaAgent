import { describe, expect, it } from "vitest";

import { getWorkbenchWindowId, scopedContainerStateKey, scopedWindowWorkspaceKey } from "./windowScope";

class MemoryStorage implements Storage {
  private values = new Map<string, string>();
  get length() {
    return this.values.size;
  }
  clear(): void {
    this.values.clear();
  }
  getItem(key: string): string | null {
    return this.values.get(key) || null;
  }
  key(index: number): string | null {
    return Array.from(this.values.keys())[index] || null;
  }
  removeItem(key: string): void {
    this.values.delete(key);
  }
  setItem(key: string, value: string): void {
    this.values.set(key, value);
  }
}

describe("Workbench window scope", () => {
  it("keeps a stable id within one window storage", () => {
    const storage = new MemoryStorage();
    const first = getWorkbenchWindowId(storage);
    const second = getWorkbenchWindowId(storage);

    expect(first).toMatch(/^window_/);
    expect(second).toBe(first);
  });

  it("separates active and container state by window id", () => {
    expect(scopedWindowWorkspaceKey("window_a", "workspace_1")).toBe("window_a:workspace_1");
    expect(scopedWindowWorkspaceKey("window_b", "workspace_1")).toBe("window_b:workspace_1");
    expect(scopedContainerStateKey("window_a", "workspace_1", "container_1")).toBe(
      "window_a:workspace_1:container_1"
    );
    expect(scopedContainerStateKey("window_b", "workspace_1", "container_1")).toBe(
      "window_b:workspace_1:container_1"
    );
  });
});
