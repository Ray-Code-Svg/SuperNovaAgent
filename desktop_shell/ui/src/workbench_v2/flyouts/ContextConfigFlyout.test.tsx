import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import type { ContextPack, ContextPackEstimate } from "../../protocol/generated/types";
import { ContextConfigFlyout } from "./ContextConfigFlyout";

describe("ContextConfigFlyout", () => {
  it("separates explicit context items from automatic recent context policy", () => {
    const html = renderToStaticMarkup(
      <ContextConfigFlyout
        containerId="container_1"
        contextPack={pack({ recentChatTurns: 6, recentTasks: 5 })}
        onEstimate={async (contextPack) => estimate(contextPack)}
        onSave={() => undefined}
      />
    );

    expect(html).toContain("0 Context items / 6 Recent chat turns + 5 Recent task runs");
    expect(html).toContain("No explicit context items selected.");
  });
});

function pack({
  recentChatTurns,
  recentTasks
}: {
  recentChatTurns: number;
  recentTasks: number;
}): ContextPack {
  return {
    context_pack_id: "context_pack_1",
    container_id: "container_1",
    selected_items: [],
    excluded_items: [],
    auto_policy: {
      include_recent_chat_turns: recentChatTurns,
      include_recent_tasks: recentTasks,
      prefer_summaries: true
    },
    summary_ref: null,
    estimated_tokens: 0
  };
}

function estimate(contextPack: ContextPack): ContextPackEstimate {
  return {
    context_pack: contextPack,
    estimated_tokens: 0,
    context_window_tokens: 128000,
    usage_ratio: "0.000"
  };
}
