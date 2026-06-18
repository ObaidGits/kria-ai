import { beforeEach, describe, expect, it, vi } from "vitest";

const { invokeMock } = vi.hoisted(() => {
  const invokeMock = vi.fn(async (command: string, args?: Record<string, unknown>) => {
    switch (command) {
      case "task_list":
        return [
          {
            id: 1,
            title: "t",
            notes: null,
            source: "manual",
            status: "open",
            priority_bucket: "normal",
            priority_score: 200,
            due_at: null,
            external_ref: null,
            created_at: "",
            updated_at: "",
          },
        ];
      case "task_add":
        return {
          id: 2,
          title: args?.title,
          notes: null,
          source: "manual",
          status: "open",
          priority_bucket: "normal",
          priority_score: 200,
          due_at: null,
          external_ref: null,
          created_at: "",
          updated_at: "",
        };
      case "task_stats":
        return {
          total: 1,
          open: 1,
          in_progress: 0,
          blocked: 0,
          waiting: 0,
          done: 0,
          overdue: 0,
          done_today: 0,
          urgent: 0,
          important: 0,
        };
      case "reminder_list":
        return [];
      case "reminder_set":
        return {
          id: 1,
          message: args?.message,
          fire_at: "",
          fired: false,
          task_id: null,
          created_at: "",
        };
      case "get_briefing_config":
        return {
          sections: [{ source: "gmail", enabled: true, query: "is:unread", max: 10 }],
          schedule: { auto: false, time: "08:00", delivery: ["notification"] },
        };
      case "set_briefing_config":
        return args?.config;
      default:
        return null;
    }
  });
  vi.stubGlobal("setInterval", vi.fn(() => 1));
  return { invokeMock };
});

vi.mock("@tauri-apps/api/core", () => ({ invoke: invokeMock }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock("highlight.js/styles/github-dark.css?url", () => ({ default: "d" }));
vi.mock("highlight.js/styles/github.css?url", () => ({ default: "l" }));

import { appStore } from "./app";

describe("Phase 2 tasks + Phase 1.5 briefing store wiring", () => {
  beforeEach(() => invokeMock.mockClear());

  it("loadTasks calls task_list with camelCase args", async () => {
    const r = await appStore.loadTasks({ activeOnly: true });
    expect(r.length).toBe(1);
    expect(invokeMock).toHaveBeenCalledWith("task_list", {
      status: null,
      bucket: null,
      activeOnly: true,
    });
  });

  it("addTask calls task_add", async () => {
    await appStore.addTask("hello");
    expect(invokeMock).toHaveBeenCalledWith("task_add", {
      title: "hello",
      notes: null,
      dueAt: null,
      source: null,
    });
  });

  it("setReminder calls reminder_set", async () => {
    await appStore.setReminder("call mom", { fireInMinutes: 30 });
    expect(invokeMock).toHaveBeenCalledWith("reminder_set", {
      message: "call mom",
      when: null,
      fireInMinutes: 30,
      fireAt: null,
      recurrence: null,
    });
  });

  it("briefing config load + save roundtrip", async () => {
    const cfg = await appStore.loadBriefingConfig();
    expect(cfg?.sections.length).toBe(1);
    await appStore.saveBriefingConfig(cfg!);
    expect(invokeMock).toHaveBeenCalledWith("set_briefing_config", { config: cfg });
  });
});
