import { beforeEach, describe, expect, it, vi } from "vitest";
import { memoryStore } from "./memory";

const { invokeMock, listenMock, liveHandlers } = vi.hoisted(() => {
  const liveHandlers: Array<(e: { payload: { kind: string } }) => void> = [];
  return {
    invokeMock: vi.fn(),
    liveHandlers,
    listenMock: vi.fn(async (_event: string, cb: (e: { payload: { kind: string } }) => void) => {
      liveHandlers.push(cb);
      return () => undefined;
    }),
  };
});

vi.mock("@tauri-apps/api/core", () => ({ invoke: invokeMock }));
vi.mock("@tauri-apps/api/event", () => ({ listen: listenMock }));

const hit = (id: string) => ({
  id,
  content: `mem ${id}`,
  memory_type: "semantic",
  namespace: "core",
  confidence: 0.9,
  importance: 0.5,
  decay_score: 1.0,
  access_count: 0,
  state: "Active",
  created_at: "2026-01-01T00:00:00Z",
  score: 0.8,
});

describe("memory store", () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it("doSearch populates reactive results and last query", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "memory_search") return { query: "dark", results: [hit("a"), hit("b")], count: 2 };
      return null;
    });
    const res = await memoryStore.doSearch("dark");
    expect(res.length).toBe(2);
    expect(memoryStore.searchResults.data().map((m) => m.id)).toEqual(["a", "b"]);
    expect(memoryStore.lastSearchQuery()).toBe("dark");
    expect(invokeMock).toHaveBeenCalledWith("memory_search", { query: "dark", limit: 30 });
  });

  it("refreshGoals maps the goals list", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "memory_goals_list")
        return {
          goals: [
            {
              id: "g1",
              kind: "user",
              title: "ship P3",
              status: "active",
              confidence: 0.6,
              priority: 6,
              parent_id: null,
              created_at: "x",
              last_progress_at: null,
            },
          ],
          count: 1,
        };
      return null;
    });
    await memoryStore.refreshGoals();
    expect(memoryStore.goals.data()[0].title).toBe("ship P3");
  });

  it("createGoal invokes the command and refreshes", async () => {
    const calls: string[] = [];
    invokeMock.mockImplementation(async (cmd: string) => {
      calls.push(cmd);
      if (cmd === "memory_goal_create") return "new-goal-id";
      if (cmd === "memory_goals_list") return { goals: [], count: 0 };
      return null;
    });
    const id = await memoryStore.createGoal("learn rust");
    expect(id).toBe("new-goal-id");
    expect(calls).toContain("memory_goal_create");
    expect(calls).toContain("memory_goals_list");
  });

  it("surfaces errors on the resource", async () => {
    invokeMock.mockImplementation(async () => {
      throw new Error("boom");
    });
    await expect(memoryStore.refreshHealth()).rejects.toThrow("boom");
    expect(memoryStore.health.error()).toContain("boom");
  });

  it("doSearch captures the retrieval trace (explainability)", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "memory_search")
        return {
          query: "x",
          results: [hit("a")],
          count: 1,
          trace: { query_class: "factual", vector_used: true, fts_used: true, candidates: 5, returned: 1 },
        };
      return null;
    });
    await memoryStore.doSearch("x");
    expect(memoryStore.lastTrace()?.query_class).toBe("factual");
    expect(memoryStore.lastTrace()?.candidates).toBe(5);
  });

  it("explain returns provenance/worth", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "memory_explain")
        return {
          id: "m1", content: "c", memory_type: "semantic", state: "active",
          confidence: 0.9, importance: 0.4, source_event_tag: "user",
          derived_from: [], contradicts: ["x"], worth_success: 2, worth_failure: 0,
          worth_samples: 2, access_count: 3, staleness_class: "fresh", superseded_by: null,
        };
      return null;
    });
    const ex = await memoryStore.api.explain("m1");
    expect(ex?.worth_success).toBe(2);
    expect(ex?.contradicts.length).toBe(1);
  });

  it("healthReport maps distributions", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "memory_health_report")
        return {
          total_active: 10, total_archived: 2, total_superseded: 1, total_forgotten: 0,
          by_type: [{ label: "semantic", count: 7 }], by_staleness: [{ label: "fresh", count: 10 }],
          avg_confidence: 0.8, unresolved_contradictions: 1, knowledge_gaps: 3,
          enrichment_backlog: 0, outbox_pending: 0,
        };
      return null;
    });
    await memoryStore.refreshHealthReport();
    expect(memoryStore.healthReport.data()?.total_active).toBe(10);
    expect(memoryStore.healthReport.data()?.by_type[0].label).toBe("semantic");
  });

  it("graph predict + create relationship route correctly", async () => {
    const calls: string[] = [];
    invokeMock.mockImplementation(async (cmd: string) => {
      calls.push(cmd);
      if (cmd === "memory_graph_predict_links")
        return { predictions: [{ target: "t1", display_name: "T1", score: 0.5, shared_neighbors: 2 }], count: 1 };
      if (cmd === "memory_graph_create_relationship") return "rel-id";
      return null;
    });
    const preds = await memoryStore.api.graphPredictLinks("e1", 5);
    expect(preds.predictions[0].target).toBe("t1");
    const relId = await memoryStore.api.graphCreateRelationship("e1", "t1", "related_to", 0.7);
    expect(relId).toBe("rel-id");
    expect(calls).toContain("memory_graph_predict_links");
    expect(calls).toContain("memory_graph_create_relationship");
  });

  it("reasoningReplay returns ordered traces", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "memory_reasoning_replay")
        return { traces: [{ id: "r1", session_id: "s", task_label: "t", kind: "chain", content: "step", confidence: 0.9, success: true, created_at: "x" }], count: 1 };
      return null;
    });
    const r = await memoryStore.api.reasoningReplay("s");
    expect(r.traces[0].kind).toBe("chain");
  });

  it("subscribeLive registers a listener and live events trigger coalesced refresh", async () => {
    vi.useFakeTimers();
    const calls: string[] = [];
    invokeMock.mockImplementation(async (cmd: string) => {
      calls.push(cmd);
      if (cmd === "memory_metrics")
        return {
          active_memories: 0, unresolved_gaps: 0,
          goals: { candidate: 0, active: 0, paused: 0, completed: 0, failed: 0, abandoned: 0, total: 0, completion_rate: 0 },
          plans: { distinct_plans: 0, total_executions: 0, success_rate: 0 }, summary: "",
        };
      if (cmd === "memory_goals_list") return { goals: [], count: 0 };
      if (cmd === "memory_timeline") return { entries: [], count: 0 };
      return null;
    });

    await memoryStore.subscribeLive();
    expect(listenMock).toHaveBeenCalledWith("memory://changed", expect.any(Function));
    expect(memoryStore.liveActive()).toBe(true);

    // Simulate two backend events in a burst → coalesced into one refresh pass.
    liveHandlers.forEach((h) => h({ payload: { kind: "created" } }));
    liveHandlers.forEach((h) => h({ payload: { kind: "goal" } }));
    expect(memoryStore.liveEventCount()).toBeGreaterThanOrEqual(2);

    await vi.advanceTimersByTimeAsync(500);
    // goal kind → goals refresh; created kind → timeline refresh; always metrics.
    expect(calls).toContain("memory_metrics");
    expect(calls).toContain("memory_goals_list");
    expect(calls).toContain("memory_timeline");

    memoryStore.unsubscribeLive();
    expect(memoryStore.liveActive()).toBe(false);
    vi.useRealTimers();
  });

  it("recordFeedback (P6) routes to memory_record_feedback with signal", async () => {
    const args: any[] = [];
    invokeMock.mockImplementation(async (cmd: string, a: any) => {
      args.push([cmd, a]);
      return null;
    });
    await memoryStore.api.recordFeedback("m1", "memory", "thumbs_up", "great");
    const call = args.find(([c]) => c === "memory_record_feedback");
    expect(call).toBeTruthy();
    expect(call[1]).toMatchObject({ targetId: "m1", targetKind: "memory", signal: "thumbs_up", detail: "great" });
  });

  it("cold start preview + import (P9) route to the gated commands", async () => {
    const calls: string[] = [];
    invokeMock.mockImplementation(async (cmd: string) => {
      calls.push(cmd);
      if (cmd === "memory_cold_start_preview")
        return { candidates: [{ source: "filesystem", path: "/x/note.md", detail: "md · 1 KB" }], count: 1 };
      if (cmd === "memory_cold_start_import") return 1;
      return null;
    });
    const preview = await memoryStore.api.coldStartPreview("filesystem", "/x", 50);
    expect(preview.candidates[0].path).toBe("/x/note.md");
    const n = await memoryStore.api.coldStartImport("filesystem", preview.candidates);
    expect(n).toBe(1);
    expect(calls).toContain("memory_cold_start_preview");
    expect(calls).toContain("memory_cold_start_import");
  });

  it("hardDelete routes to the hard-delete command", async () => {
    const calls: string[] = [];
    invokeMock.mockImplementation(async (cmd: string) => {
      calls.push(cmd);
      if (cmd === "memory_hard_delete") return 3;
      if (cmd === "memory_timeline") return { entries: [], count: 0 };
      if (cmd === "memory_metrics")
        return {
          active_memories: 0,
          unresolved_gaps: 0,
          goals: { candidate: 0, active: 0, paused: 0, completed: 0, failed: 0, abandoned: 0, total: 0, completion_rate: 0 },
          plans: { distinct_plans: 0, total_executions: 0, success_rate: 0 },
          summary: "",
        };
      return null;
    });
    const n = await memoryStore.hardDelete("memory", "abc");
    expect(n).toBe(3);
    expect(calls).toContain("memory_hard_delete");
  });
});
