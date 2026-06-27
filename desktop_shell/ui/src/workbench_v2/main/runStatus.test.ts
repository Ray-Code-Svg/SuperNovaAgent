import { describe, expect, it } from "vitest";

import {
  blockingRunFromPage,
  forceCloseTargetFromBlockingRun,
  runBlocksContainerInteraction
} from "./runStatus";
import type { RunRecord } from "../../protocol/generated/types";

describe("runBlocksContainerInteraction", () => {
  it("blocks only active or cancellation-pending run states", () => {
    expect(runBlocksContainerInteraction({ status: "queued", cancel_requested: false })).toBe(true);
    expect(runBlocksContainerInteraction({ status: "running", cancel_requested: false })).toBe(true);
    expect(runBlocksContainerInteraction({ status: "completed", cancel_requested: true })).toBe(true);
    expect(runBlocksContainerInteraction({ status: "completed", cancel_requested: false })).toBe(false);
    expect(runBlocksContainerInteraction({ status: "failed", cancel_requested: false })).toBe(false);
  });

  it("selects force-close target from the blocking run instead of current UI mode", () => {
    const staleTaskRun = run({
      run_id: "run_task",
      run_kind: "task",
      status: "running",
      task_id: "task_1",
      job_id: "job_1"
    });
    const completedChatRun = run({
      run_id: "run_chat",
      run_kind: "chat",
      status: "completed",
      chat_thread_id: "chat_1"
    });

    const blockingRun = blockingRunFromPage({ items: [completedChatRun, staleTaskRun] });

    expect(blockingRun?.run_id).toBe("run_task");
    expect(forceCloseTargetFromBlockingRun(blockingRun)).toEqual({
      mode: "task",
      taskId: "task_1"
    });
  });
});

function run(overrides: Partial<RunRecord>): RunRecord {
  return {
    run_id: "run",
    workspace_uid: "workspace",
    container_id: "container",
    run_kind: "task",
    chat_thread_id: null,
    task_id: null,
    job_id: null,
    worker_id: "worker",
    status: "running",
    cancel_requested: false,
    heartbeat_at_ms: null,
    started_at_ms: 1,
    updated_at_ms: 1,
    error_message: null,
    ...overrides
  };
}
