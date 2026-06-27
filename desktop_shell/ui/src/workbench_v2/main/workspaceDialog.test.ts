import { describe, expect, it } from "vitest";

import { selectedWorkspaceRoot } from "./workspaceDialog";

describe("selectedWorkspaceRoot", () => {
  it("returns the selected workspace root", () => {
    expect(selectedWorkspaceRoot({ status: "selected", workspace_root: " C:/workspace " })).toBe(
      "C:/workspace"
    );
  });

  it("ignores cancelled or empty workspace dialog results", () => {
    expect(selectedWorkspaceRoot({ status: "cancelled", workspace_root: "C:/SuperNova" })).toBeNull();
    expect(selectedWorkspaceRoot({ status: "selected", workspace_root: "" })).toBeNull();
    expect(selectedWorkspaceRoot(null)).toBeNull();
  });
});
