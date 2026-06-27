import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import type { TaskRecord } from "../../protocol/generated/types";
import { TaskStatusRail } from "./TaskStatusRail";

describe("TaskStatusRail", () => {
  it("renders rail copy through the i18n catalog", () => {
    const html = renderToStaticMarkup(
      <TaskStatusRail tasks={[]} activeTaskId={null} onSelectTask={() => {}} />
    );

    expect(html).toContain("TASK LIST");
    expect(html).toContain("No task history");
  });

  it("renders artifact labels through the i18n catalog", () => {
    const html = renderToStaticMarkup(
      <TaskStatusRail tasks={[task()]} activeTaskId={null} onSelectTask={() => {}} />
    );

    expect(html).toContain("2 artifact");
  });
});

function task(): TaskRecord {
  return {
    task_id: "task_1",
    container_id: "container_1",
    job_id: "job_1",
    title: "Task 1",
    goal: "Do task",
    status: "completed",
    badges: {
      running: 0,
      approval: 0,
      blocked: 0,
      unread: 0,
      artifact_ready: 2
    },
    created_at_ms: 1,
    updated_at_ms: 1
  };
}
