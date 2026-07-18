import { describe, it, expect, vi, beforeEach } from "vitest";
import type { ServiceResult } from "../bridge/types";

// Mock the bridge invoke layer so we can assert WHICH existing memory_* command
// each action dispatches (and with what args) — no Tauri runtime in tests.
vi.mock("../bridge/invoke", () => ({
  bridgeInvoke: vi.fn(),
  bridgeInvokeOptional: vi.fn(async () => null),
}));

import { memoryStore, type MemoryFact } from "./memoryStore";
import { bridgeInvoke } from "../bridge/invoke";
import { eventBus } from "./eventBus";

const mockInvoke = bridgeInvoke as unknown as ReturnType<typeof vi.fn>;

function ok<T>(data: T): ServiceResult<T> {
  return { ok: true, data };
}
function fail(message = "boom"): ServiceResult<never> {
  return { ok: false, code: "error", message } as ServiceResult<never>;
}

function makeFact(id: string, over: Partial<MemoryFact> = {}): MemoryFact {
  const now = Date.now();
  return {
    id,
    content: "the sky is blue",
    confidence: 0.8,
    worth: 0.5,
    staleness: 0.1,
    source: "conversation",
    createdAt: now,
    updatedAt: now,
    tags: [],
    ...over,
  };
}

const EXPLAIN = {
  id: "m1",
  content: "the sky is blue",
  memory_type: "semantic",
  state: "active",
  confidence: 0.8,
  importance: 0.4,
  source_event_tag: "user",
  derived_from: ["p1"],
  contradicts: ["c1"],
  worth_success: 3,
  worth_failure: 1,
  worth_samples: 4,
  access_count: 5,
  staleness_class: "Slow",
  superseded_by: null,
};

describe("memoryStore actions — route through EXISTING memory_* commands (Req 5.2/5.3)", () => {
  beforeEach(() => {
    mockInvoke.mockReset();
    memoryStore.setFacts([]);
    memoryStore.clearUndo();
  });

  it("fetchDetail calls memory_explain and normalizes the payload (Req 5.2)", async () => {
    mockInvoke.mockResolvedValueOnce(ok(EXPLAIN));
    const res = await memoryStore.fetchDetail("m1");
    expect(mockInvoke).toHaveBeenCalledWith("memory_explain", { memoryId: "m1" });
    expect(res.ok && res.data).toMatchObject({
      state: "active",
      sourceEventTag: "user",
      derivedFrom: ["p1"],
      contradicts: ["c1"],
      worthSamples: 4,
      stalenessClass: "Slow",
      supersededBy: null,
    });
  });

  it("fetchDetail surfaces a null memory honestly (no longer exists)", async () => {
    mockInvoke.mockResolvedValueOnce(ok(null));
    const res = await memoryStore.fetchDetail("gone");
    expect(res.ok && res.data).toBeNull();
  });

  it("fetchDetail returns an error result on command failure (no silent no-op)", async () => {
    mockInvoke.mockResolvedValueOnce(fail("db locked"));
    const res = await memoryStore.fetchDetail("m1");
    expect(res.ok).toBe(false);
    expect(res.ok === false && res.message).toBe("db locked");
  });

  it("verify calls memory_verify and refreshes updatedAt on a positive verdict", async () => {
    memoryStore.setFacts([makeFact("m1", { updatedAt: 1 })]);
    mockInvoke.mockResolvedValueOnce(ok(true));
    const res = await memoryStore.verify("m1");
    expect(mockInvoke).toHaveBeenCalledWith("memory_verify", { memoryId: "m1" });
    expect(res.ok && res.data).toBe(true);
    expect(memoryStore.facts()[0].updatedAt).toBeGreaterThan(1);
  });

  it("correct records a correction feedback signal with the new text", async () => {
    memoryStore.setFacts([makeFact("m1")]);
    mockInvoke.mockResolvedValueOnce(ok(undefined));
    const res = await memoryStore.correct("m1", "the sky is grey today");
    expect(mockInvoke).toHaveBeenCalledWith("memory_record_feedback", {
      targetId: "m1",
      targetKind: "memory",
      signal: "correction",
      detail: "the sky is grey today",
    });
    expect(res.ok).toBe(true);
    expect(memoryStore.facts()[0].content).toBe("the sky is grey today");
  });

  it("correct rejects empty text without dispatching", async () => {
    const res = await memoryStore.correct("m1", "   ");
    expect(res.ok).toBe(false);
    expect(mockInvoke).not.toHaveBeenCalled();
  });

  it("reinforce and penalize send thumbs_up / thumbs_down", async () => {
    mockInvoke.mockResolvedValue(ok(undefined));
    await memoryStore.reinforce("m1");
    expect(mockInvoke).toHaveBeenCalledWith("memory_record_feedback", {
      targetId: "m1",
      targetKind: "memory",
      signal: "thumbs_up",
    });
    await memoryStore.penalize("m1");
    expect(mockInvoke).toHaveBeenCalledWith("memory_record_feedback", {
      targetId: "m1",
      targetKind: "memory",
      signal: "thumbs_down",
    });
  });

  it("forget calls memory_forget(scope=memory), drops the fact, and buffers an undo", async () => {
    memoryStore.setFacts([makeFact("m1")]);
    mockInvoke.mockResolvedValueOnce(ok(1));
    const res = await memoryStore.forget("m1");
    expect(mockInvoke).toHaveBeenCalledWith("memory_forget", { kind: "memory", value: "m1" });
    expect(res.ok).toBe(true);
    expect(memoryStore.facts()).toHaveLength(0);
    expect(memoryStore.pendingUndo()?.fact.id).toBe("m1");
  });

  it("forget failure leaves the fact in place and buffers nothing", async () => {
    memoryStore.setFacts([makeFact("m1")]);
    mockInvoke.mockResolvedValueOnce(fail("nope"));
    const res = await memoryStore.forget("m1");
    expect(res.ok).toBe(false);
    expect(memoryStore.facts()).toHaveLength(1);
    expect(memoryStore.pendingUndo()).toBeNull();
  });

  it("undoForget re-adds through memory_remember and restores the fact", async () => {
    memoryStore.setFacts([makeFact("m1")]);
    mockInvoke.mockResolvedValueOnce(ok(1)); // forget
    await memoryStore.forget("m1");
    expect(memoryStore.facts()).toHaveLength(0);

    mockInvoke.mockResolvedValueOnce(ok({ decision: "stored" })); // remember
    const res = await memoryStore.undoForget();
    expect(mockInvoke).toHaveBeenLastCalledWith("memory_remember", { text: "the sky is blue" });
    expect(res.ok).toBe(true);
    expect(memoryStore.facts().map((f) => f.id)).toContain("m1");
    expect(memoryStore.pendingUndo()).toBeNull();
  });

  it("hardDelete calls memory_hard_delete(scope=memory) and drops the fact (no undo)", async () => {
    memoryStore.setFacts([makeFact("m1")]);
    mockInvoke.mockResolvedValueOnce(ok(1));
    const res = await memoryStore.hardDelete("m1");
    expect(mockInvoke).toHaveBeenCalledWith("memory_hard_delete", { kind: "memory", value: "m1" });
    expect(res.ok).toBe(true);
    expect(memoryStore.facts()).toHaveLength(0);
    expect(memoryStore.pendingUndo()).toBeNull();
  });
});

describe("memoryStore cognition — triggers EXISTING commands + persists what changed (Req 5.6)", () => {
  beforeEach(() => {
    mockInvoke.mockReset();
    memoryStore.clearCognitionResults();
  });

  it("reflect dispatches memory_reflect and records an 'Insights formed' result", async () => {
    mockInvoke.mockResolvedValueOnce(ok(3));
    const res = await memoryStore.runCognition("reflect");
    expect(mockInvoke).toHaveBeenCalledWith("memory_reflect", undefined);
    expect(res.ok && res.data.changes).toEqual([{ label: "Insights formed", value: 3 }]);
    // Persisted (not a toast): the result is retained in the store history.
    expect(memoryStore.cognitionResults()[0].job).toBe("reflect");
    expect(memoryStore.cognitionResults()[0].ok).toBe(true);
    // Running state cleared after completion.
    expect(memoryStore.isCognitionRunning("reflect")).toBe(false);
  });

  it("dream dispatches memory_run_dream and maps the object payload to changes", async () => {
    mockInvoke.mockResolvedValueOnce(ok({ procedures: 2, goals_merged: 1, worth_recalibrated: 4 }));
    const res = await memoryStore.runCognition("dream");
    expect(mockInvoke).toHaveBeenCalledWith("memory_run_dream", undefined);
    expect(res.ok && res.data.changes).toEqual([
      { label: "Procedures distilled", value: 2 },
      { label: "Goals merged", value: 1 },
      { label: "Worth recalibrated", value: 4 },
    ]);
  });

  it("entity-extraction maps { processed, entities_linked }", async () => {
    mockInvoke.mockResolvedValueOnce(ok({ processed: 10, entities_linked: 7 }));
    const res = await memoryStore.runCognition("entity-extraction");
    expect(mockInvoke).toHaveBeenCalledWith("memory_run_entity_extraction", undefined);
    expect(res.ok && res.data.changes).toEqual([
      { label: "Memories processed", value: 10 },
      { label: "Entities linked", value: 7 },
    ]);
  });

  it("consolidate forwards the session id to memory_consolidate", async () => {
    mockInvoke.mockResolvedValueOnce(ok(5));
    await memoryStore.runCognition("consolidate", { sessionId: "sess-1" });
    expect(mockInvoke).toHaveBeenCalledWith("memory_consolidate", { sessionId: "sess-1" });
  });

  it("stages cognition-started/completed events so the Core reflects running state", async () => {
    const seen: Array<{ name: string; job: string; success?: boolean }> = [];
    const offStart = eventBus.on("memory:cognition-started", (p) =>
      seen.push({ name: "started", job: p.job }),
    );
    const offDone = eventBus.on("memory:cognition-completed", (p) =>
      seen.push({ name: "completed", job: p.job, success: p.success }),
    );
    mockInvoke.mockResolvedValueOnce(ok(1));
    await memoryStore.runCognition("reflect");
    offStart();
    offDone();
    expect(seen).toEqual([
      { name: "started", job: "reflect" },
      { name: "completed", job: "reflect", success: true },
    ]);
  });

  it("records an honest failure result (never a silent no-op) and still emits completed", async () => {
    mockInvoke.mockResolvedValueOnce(fail("engine offline"));
    const res = await memoryStore.runCognition("self-improvement");
    expect(res.ok).toBe(false);
    const latest = memoryStore.cognitionResults()[0];
    expect(latest.ok).toBe(false);
    expect(latest.message).toBe("engine offline");
    expect(memoryStore.isCognitionRunning("self-improvement")).toBe(false);
  });
});

describe("memoryStore cold-start and causal contracts", () => {
  beforeEach(() => {
    mockInvoke.mockReset();
  });

  it("previews cold-start candidates through the canonical bridge", async () => {
    const candidates = [{ path: "/tmp/note.md", detail: "Markdown document" }];
    mockInvoke.mockResolvedValueOnce(ok({ candidates, count: 1 }));

    const result = await memoryStore.previewColdStart("filesystem", "/tmp", 25);

    expect(mockInvoke).toHaveBeenCalledWith(
      "memory_cold_start_preview",
      { source: "filesystem", root: "/tmp", limit: 25 },
      { timeoutMs: 30_000 },
    );
    expect(result).toEqual({ ok: true, data: candidates });
  });

  it("queries causal effects and stores authoritative links", async () => {
    const links = [{ cause: "low battery", effect: "shutdown", strength: 0.9 }];
    mockInvoke.mockResolvedValueOnce(ok({ links, count: 1 }));

    const result = await memoryStore.queryReasoning("effects", "low battery");

    expect(mockInvoke).toHaveBeenCalledWith("memory_causal_effects_of", { cause: "low battery" });
    expect(result.ok && result.data).toEqual({ mode: "effects", query: "low battery", links });
    expect(memoryStore.reasoningQuery()).toEqual({ mode: "effects", query: "low battery", links });
  });

  it("surfaces causal query failures without replacing prior results", async () => {
    const previous = memoryStore.reasoningQuery();
    mockInvoke.mockResolvedValueOnce(fail("causal index unavailable"));

    const result = await memoryStore.queryReasoning("chains", "start");

    expect(result).toEqual({ ok: false, message: "causal index unavailable" });
    expect(memoryStore.reasoningQueryError()).toBe("causal index unavailable");
    expect(memoryStore.reasoningQuery()).toEqual(previous);
  });
});