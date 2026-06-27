import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it } from "vitest";

import { CommandComposer } from "./CommandComposer";
import { useWorkbenchUiStore } from "../state/uiStore";

describe("CommandComposer", () => {
  beforeEach(() => {
    useWorkbenchUiStore.setState({
      modeByContainer: {},
      draftByContainer: {},
      openFlyout: null
    });
  });

  it("renders force close as a row above the input controls", () => {
    const html = renderToStaticMarkup(
      <CommandComposer
        containerId="container"
        scopeId="scope"
        forceCloseVisible
        onForceClose={() => {}}
        onSubmit={() => {}}
      />
    );

    expect(html).toContain("sn-composer-stack");
    expect(html).toContain("sn-composer-force-close");
    expect(html.indexOf("sn-composer-force-close")).toBeLessThan(html.indexOf("sn-composer-input"));
  });

  it("renders from the same scope mode used for submission", () => {
    useWorkbenchUiStore.getState().setMode("window_a:workspace_1:container_1", "task");

    const html = renderToStaticMarkup(
      <CommandComposer
        containerId="container_1"
        scopeId="window_a:workspace_1:container_1"
        onSubmit={() => {}}
      />
    );

    expect(html).toContain("data-mode=\"task\"");
    expect(html).toContain("Agent TASK");
  });
});
