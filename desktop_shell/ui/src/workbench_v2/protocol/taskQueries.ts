import type { StreamEventHandler } from "../../protocol/generated/client";
import type {
  ForceCloseRequest,
  TaskStreamRequest,
  TaskUserInputRequest
} from "../../protocol/generated/types";
import { createRuntimeClient } from "./runtimeClient";

export async function listTasks(containerId: string) {
  return (await createRuntimeClient()).tasks(containerId);
}

export async function getTask(taskId: string) {
  return (await createRuntimeClient()).task(taskId);
}

export async function startTask(
  containerId: string,
  request: TaskStreamRequest,
  onEvent?: StreamEventHandler<unknown>
) {
  return (await createRuntimeClient()).startTaskStream(containerId, request, onEvent);
}

export async function streamTaskEvents(
  taskId: string,
  afterEventId?: number | null,
  onEvent?: StreamEventHandler<unknown>
) {
  return (await createRuntimeClient()).taskEventsStream(taskId, {
    after_event_id: afterEventId ?? undefined
  }, onEvent);
}

export async function submitTaskUserInput(taskId: string, request: TaskUserInputRequest) {
  return (await createRuntimeClient()).submitTaskUserInput(taskId, request);
}

export async function forceCloseTask(taskId: string, request: ForceCloseRequest = {}) {
  return (await createRuntimeClient()).forceCloseTask(taskId, request);
}
