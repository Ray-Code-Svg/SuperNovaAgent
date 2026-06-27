import type { RunRecord } from "../../protocol/generated/types";

export function runBlocksContainerInteraction(run: Pick<RunRecord, "status" | "cancel_requested">) {
  if (run.cancel_requested) return true;
  return ["queued", "running", "waiting_approval", "waiting_user", "cancel_requested"].includes(
    run.status.toLowerCase()
  );
}

export function runPageHasBlockingRun(page?: { items: RunRecord[] } | null) {
  return Boolean(page?.items.some(runBlocksContainerInteraction));
}

export function blockingRunFromPage(page?: { items: RunRecord[] } | null) {
  return page?.items.find(runBlocksContainerInteraction) || null;
}

export type ForceCloseRunTarget =
  | { mode: "chat"; chatThreadId: string; taskId?: null }
  | { mode: "task"; taskId: string; chatThreadId?: null };

export function forceCloseTargetFromBlockingRun(run?: RunRecord | null): ForceCloseRunTarget | null {
  if (!run || !runBlocksContainerInteraction(run)) return null;
  if (run.task_id || run.job_id) {
    return { mode: "task", taskId: run.task_id || run.job_id || "" };
  }
  if (run.chat_thread_id) {
    return { mode: "chat", chatThreadId: run.chat_thread_id };
  }
  return null;
}
