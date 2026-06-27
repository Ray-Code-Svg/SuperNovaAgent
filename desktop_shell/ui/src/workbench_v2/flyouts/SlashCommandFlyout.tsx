import { useEffect, useRef, useState } from "react";
import { Button } from "@fluentui/react-components";
import type { UiCapabilityManifest } from "../../protocol/generated/types";

import { WorkbenchFlyout } from "./WorkbenchFlyout";
import { useI18n } from "../i18n/i18n";
import { useWorkbenchUiStore } from "../state/uiStore";

interface SlashCommandFlyoutProps {
  scopeId: string | null;
  capabilities?: UiCapabilityManifest;
}

export function SlashCommandFlyout({ scopeId, capabilities }: SlashCommandFlyoutProps) {
  const t = useI18n();
  const setMode = useWorkbenchUiStore((state) => state.setMode);
  const setDraft = useWorkbenchUiStore((state) => state.setDraft);
  const setOpenFlyout = useWorkbenchUiStore((state) => state.setOpenFlyout);
  const manifestCommands = capabilities?.commands.length ? capabilities.commands : DEFAULT_COMMANDS;
  const commands = visibleSlashCommands(manifestCommands);
  const [activeIndex, setActiveIndex] = useState(0);
  const listRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    setActiveIndex(0);
  }, [commands.length]);

  useEffect(() => {
    listRef.current?.focus();
  }, []);

  function run(commandId: string) {
    if (commandId === "command.chat") {
      setMode(scopeId, "chat");
      setDraft(scopeId, "");
      setOpenFlyout(null);
      return;
    }
    if (commandId === "command.task" || commandId === "command.start_task") {
      setMode(scopeId, "task");
      setDraft(scopeId, "");
      setOpenFlyout(null);
      return;
    }
    if (commandId === "command.model") {
      setDraft(scopeId, "");
      setOpenFlyout("model");
      return;
    }
    if (commandId === "command.context") {
      setDraft(scopeId, "");
      setOpenFlyout("context");
      return;
    }
    setDraft(scopeId, "");
    setOpenFlyout(null);
  }

  function runActive() {
    const command = commands[activeIndex];
    if (command) run(command.command_id);
  }

  function moveActive(delta: number) {
    setActiveIndex((current) => {
      if (commands.length === 0) return 0;
      return (current + delta + commands.length) % commands.length;
    });
  }

  return (
    <WorkbenchFlyout title={t("slash.title")}>
      <div
        aria-activedescendant={commands[activeIndex]?.command_id}
        className="sn-flyout-list"
        onKeyDown={(event) => {
          if (event.key === "Escape") {
            event.preventDefault();
            setOpenFlyout(null);
            return;
          }
          if (event.key === "PageDown" || event.key === "ArrowDown") {
            event.preventDefault();
            moveActive(1);
          }
          if (event.key === "PageUp" || event.key === "ArrowUp") {
            event.preventDefault();
            moveActive(-1);
          }
          if (event.key === "Enter") {
            event.preventDefault();
            runActive();
          }
        }}
        ref={listRef}
        role="listbox"
        tabIndex={0}
      >
        {commands.map((command, index) => (
          <Button
            appearance="subtle"
            aria-selected={index === activeIndex}
            data-active={index === activeIndex}
            id={command.command_id}
            key={command.command_id}
            onClick={() => run(command.command_id)}
            onMouseEnter={() => setActiveIndex(index)}
            role="option"
            title={commandDescription(command.command_id, command.description, t)}
          >
            {commandLabel(command.command_id, command.label, t)}
          </Button>
        ))}
      </div>
    </WorkbenchFlyout>
  );
}

function commandLabel(commandId: string, fallback: string, t: ReturnType<typeof useI18n>) {
  if (commandId === "command.chat") return t("slash.chat");
  if (commandId === "command.task" || commandId === "command.start_task") return t("slash.task");
  if (commandId === "command.model") return t("slash.model");
  if (commandId === "command.context") return t("slash.context");
  return fallback;
}

function commandDescription(commandId: string, fallback: string, t: ReturnType<typeof useI18n>) {
  if (commandId === "command.chat") return t("slash.chatDesc");
  if (commandId === "command.task" || commandId === "command.start_task") return t("slash.taskDesc");
  if (commandId === "command.model") return t("slash.modelDesc");
  if (commandId === "command.context") return t("slash.contextDesc");
  return fallback;
}

function visibleSlashCommands(commands: UiCapabilityManifest["commands"]) {
  const hasCanonicalTask = commands.some((command) => command.command_id === "command.task");
  const seen = new Set<string>();
  return commands.filter((command) => {
    if (hasCanonicalTask && command.command_id === "command.start_task") return false;
    const semanticKey = command.command_id === "command.start_task" ? "command.task" : command.command_id;
    if (seen.has(semanticKey)) return false;
    seen.add(semanticKey);
    return true;
  });
}

const DEFAULT_COMMANDS = [
  {
    command_id: "command.chat",
    label: "Chat",
    description: "Switch to AgentChat mode",
    capability_id: "mode.switch.chat"
  },
  {
    command_id: "command.task",
    label: "Task",
    description: "Switch to Agent TASK mode",
    capability_id: "mode.switch.task"
  },
  {
    command_id: "command.model",
    label: "Model",
    description: "Open model configuration",
    capability_id: "model.config.read"
  },
  {
    command_id: "command.context",
    label: "Context",
    description: "Open context pack configuration",
    capability_id: "context.pack.read"
  }
];
