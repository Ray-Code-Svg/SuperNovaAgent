import type { ContainerMode } from "../state/uiStore";

export interface SlashCommandResult {
  command: "chat" | "task" | "start_task" | "model" | "context" | null;
  mode?: ContainerMode;
}

export function parseSlashCommand(value: string): SlashCommandResult {
  const command = value.trim().replace(/^\/+/, "").toLowerCase();
  if (command === "chat") {
    return { command: "chat", mode: "chat" };
  }
  if (command === "task") {
    return { command: "task", mode: "task" };
  }
  if (command === "start task" || command === "start-task") {
    return { command: "start_task", mode: "task" };
  }
  if (command === "model") {
    return { command: "model" };
  }
  if (command === "context") {
    return { command: "context" };
  }
  return { command: null };
}
