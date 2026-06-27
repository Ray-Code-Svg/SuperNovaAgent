import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it } from "vitest";

import type { UiCapabilityManifest } from "../../protocol/generated/types";
import { useWorkbenchUiStore } from "../state/uiStore";
import { SlashCommandFlyout } from "./SlashCommandFlyout";

describe("SlashCommandFlyout", () => {
  beforeEach(() => {
    useWorkbenchUiStore.setState({
      language: "en-US",
      modeByContainer: {},
      draftByContainer: {},
      openFlyout: "slash"
    });
  });

  it("renders a single task command when the runtime manifest includes legacy start_task", () => {
    const html = renderToStaticMarkup(
      <SlashCommandFlyout
        scopeId="scope"
        capabilities={manifest([
          command("command.chat", "Chat", "Switch to AgentChat mode", "mode.switch.chat"),
          command("command.task", "Task", "Switch to Agent TASK mode", "mode.switch.task"),
          command("command.start_task", "Start Task", "Start a task from the active container", "task.start"),
          command("command.model", "Model", "Open model configuration", "model.config.read")
        ])}
      />
    );

    expect(html.match(/Switch to Agent TASK mode/g)).toHaveLength(1);
    expect(html).toContain("Chat");
    expect(html).toContain("Task");
    expect(html).toContain("Model");
  });
});

function command(command_id: string, label: string, description: string, capability_id: string) {
  return { command_id, label, description, capability_id };
}

function manifest(commands: UiCapabilityManifest["commands"]): UiCapabilityManifest {
  return {
    commands,
    workspace_actions: [],
    container_actions: [],
    composer_tokens: [],
    model_config: {
      active: {
        provider: "deepseek",
        model: "deepseek-v4-flash",
        thinking: "auto",
        reasoning_effort: "high",
        token_budget: 4096,
        strict_tools: true
      },
      providers: [],
      thinking_options: [],
      reasoning_effort_options: [],
      token_budget_min: 1,
      token_budget_max: 65536,
      token_budget_default: 4096,
      strict_tools_label: "Strict provider tools",
      strict_tools_description: "Require provider tool calls to map to registered capabilities.",
      advanced_defaults_collapsed: true,
      user_summary: "DeepSeek V4 Flash"
    },
    context_config: {
      supports_history_chat: true,
      supports_history_task: true,
      supports_container_default_pack: true,
      supports_compaction: true
    },
    settings: []
  };
}
