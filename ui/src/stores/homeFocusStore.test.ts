/**
 * Tests for `homeFocusStore` — the Homepage Intelligence Layer (Focus engine).
 *
 * Covers task 3.1 foundation:
 *   • Unit examples: ranking precedence, single-subject binding, resting frame,
 *     chip cap, lit-only orbit, awareness-bridge seam, advisory coreHint.
 *   • Property 1 (Read-model purity): deriving a frame performs no domain writes,
 *     no tool calls, and no sends across randomized signal sequences.
 *     **Validates: Requirements 12.5**
 *   • Property 2 (Single-subject binding): whenever both `voiceLine` and `acs`
 *     render, `voiceLine.subjectId === acs.subjectId`.
 *     **Validates: Requirements 8.4, 12.3**
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import fc from "fast-check";

import {
  deriveFocusFrame,
  homeFocusStore,
  setAwarenessBridge,
  clearAwarenessBridge,
  MAX_CHIPS,
  type AwarenessSignal,
  type FocusInputs,
} from "./homeFocusStore";
import { assertFocusFrame } from "../shell/spaces/home/guardrails";
import { eventBus } from "./eventBus";
import { approvalStore, type ApprovalRequest, type RiskLevel } from "./approvalStore";
import { converseStore, type Thread } from "./converseStore";
import { automationStore, type Workflow, type WorkflowStatus } from "./automationStore";
import { memoryStore, type MemoryFact } from "./memoryStore";
import { notificationStore, type Notification, type NotificationLevel } from "./notificationStore";

// ─── Fixture builders ────────────────────────────────────────────────────────

const approval = (over: Partial<ApprovalRequest> = {}): ApprovalRequest => ({
  id: "a1",
  type: "tool-hitl",
  title: "Delete 3 files",
  description: "Remove temp artifacts",
  risk: "red",
  payload: null,
  createdAt: 1000,
  status: "pending",
  ...over,
});

const thread = (over: Partial<Thread> = {}): Thread => ({
  id: "t1",
  title: "Refactor auth",
  createdAt: 1,
  updatedAt: 100,
  pinned: false,
  archived: false,
  temporary: false,
  ...over,
});

const workflow = (over: Partial<Workflow> = {}): Workflow => ({
  id: "w1",
  name: "Nightly backup",
  description: "Backs up the DB",
  status: "running",
  lastRunAt: 200,
  createdAt: 1,
  ...over,
});

const fact = (over: Partial<MemoryFact> = {}): MemoryFact => ({
  id: "f1",
  content: "You prefer dark mode",
  confidence: 0.9,
  worth: 1,
  staleness: 0,
  source: "chat",
  createdAt: 1,
  updatedAt: 50,
  tags: [],
  ...over,
});

const notice = (over: Partial<Notification> = {}): Notification => ({
  id: "n1",
  level: "needs-you",
  message: "Sign the release",
  createdAt: 1,
  updatedAt: 80,
  read: false,
  count: 1,
  ...over,
});

const emptyInputs = (over: Partial<FocusInputs> = {}): FocusInputs => ({
  approvals: [],
  threads: [],
  activeThreadId: null,
  conversing: false,
  workflows: [],
  facts: [],
  notifications: [],
  awareness: [],
  now: 10_000,
  ...over,
});

// ─── Unit examples ─────────────────────────────────────────────────────────

describe("deriveFocusFrame — structure & resting output (Req 12.5)", () => {
  it("returns a valid empty/resting frame when nothing qualifies", () => {
    const frame = deriveFocusFrame(emptyInputs());
    expect(frame.voiceLine).toBeUndefined();
    expect(frame.acs).toBeUndefined();
    expect(frame.chips).toEqual([]);
    expect(frame.orbit).toEqual([]);
    expect(frame.coreHint).toBeUndefined();
    expect(() => assertFocusFrame(frame)).not.toThrow();
  });

  it("emits only lit orbit points", () => {
    const frame = deriveFocusFrame(emptyInputs({ workflows: [workflow()] }));
    expect(frame.orbit.length).toBeGreaterThan(0);
    expect(frame.orbit.every((p) => p.lit)).toBe(true);
  });

  it("never emits more than MAX_CHIPS chips", () => {
    const frame = deriveFocusFrame(
      emptyInputs({
        approvals: [approval({ id: "a1" }), approval({ id: "a2" })],
        workflows: [workflow({ id: "w1" }), workflow({ id: "w2" })],
        threads: [thread({ id: "t1" }), thread({ id: "t2" })],
      }),
    );
    expect(frame.chips.length).toBeLessThanOrEqual(MAX_CHIPS);
    expect(() => assertFocusFrame(frame)).not.toThrow();
  });
});

describe("deriveFocusFrame — fixed ranking precedence (design §5.3)", () => {
  it("needs-you approval outranks a resumable thread and a learned fact", () => {
    const frame = deriveFocusFrame(
      emptyInputs({
        approvals: [approval()],
        threads: [thread()],
        facts: [fact()],
      }),
    );
    expect(frame.voiceLine?.subjectId).toBe("approval:a1");
  });

  it("high-risk approvals sort ahead of low-risk approvals", () => {
    const frame = deriveFocusFrame(
      emptyInputs({
        approvals: [
          approval({ id: "low", risk: "green", createdAt: 9000 }),
          approval({ id: "high", risk: "red", createdAt: 1 }),
        ],
      }),
    );
    expect(frame.voiceLine?.subjectId).toBe("approval:high");
  });

  it("a running workflow (active session) outranks a resumable thread", () => {
    const frame = deriveFocusFrame(
      emptyInputs({ workflows: [workflow({ status: "running" })], threads: [thread()] }),
    );
    expect(frame.voiceLine?.subjectId).toBe("automation:w1");
  });
});

describe("deriveFocusFrame — single-subject binding (Req 8.4/12.3)", () => {
  it("binds voiceLine and acs to the same subject when both render", () => {
    const frame = deriveFocusFrame(emptyInputs({ approvals: [approval()] }));
    expect(frame.voiceLine).toBeDefined();
    expect(frame.acs).toBeDefined();
    expect(frame.acs!.subjectId).toBe(frame.voiceLine!.subjectId);
  });
});

describe("deriveFocusFrame — advisory coreHint (Req 30.3)", () => {
  it("carries coreHint as an advisory string only", () => {
    const frame = deriveFocusFrame(emptyInputs({ approvals: [approval({ risk: "red" })] }));
    expect(typeof frame.coreHint).toBe("string");
    expect(frame.coreHint).toBe("blocked");
    // Guardrail: coreHint must be advisory (a string), never an object command.
    expect(() => assertFocusFrame(frame)).not.toThrow();
  });
});

describe("awareness bridge seam (task 3.7 wiring)", () => {
  afterEach(() => clearAwarenessBridge());

  it("defaults to no signals and ranks injected awareness by its own priority", () => {
    const signal: AwarenessSignal = {
      id: "meeting",
      capability: "desktop",
      priority: 80,
      recency: 5,
      voiceText: "Meeting in 20 — prep notes?",
      actionable: false,
    };
    setAwarenessBridge({ signals: () => [signal] });
    const inputs = homeFocusStore.readInputs();
    expect(inputs.awareness).toHaveLength(1);
    // priority 80 beats a resumable thread (40).
    const frame = deriveFocusFrame({ ...inputs, threads: [thread()], now: 10_000 });
    expect(frame.voiceLine?.subjectId).toBe("desktop:meeting");
  });
});

// ─── Property 2: Single-subject binding ──────────────────────────────────────
// Validates: Requirements 8.4, 12.3

const arbApproval = fc.record({
  id: fc.string({ minLength: 1, maxLength: 6 }),
  risk: fc.constantFrom("green", "yellow", "red", "black"),
  createdAt: fc.integer({ min: 0, max: 1e9 }),
});
const arbThread = fc.record({
  id: fc.string({ minLength: 1, maxLength: 6 }),
  updatedAt: fc.integer({ min: 0, max: 1e9 }),
  archived: fc.boolean(),
});
const arbWorkflow = fc.record({
  id: fc.string({ minLength: 1, maxLength: 6 }),
  status: fc.constantFrom("idle", "running", "completed", "failed", "paused"),
  lastRunAt: fc.integer({ min: 0, max: 1e9 }),
});
const arbFact = fc.record({
  id: fc.string({ minLength: 1, maxLength: 6 }),
  worth: fc.integer({ min: 0, max: 3 }),
  updatedAt: fc.integer({ min: 0, max: 1e9 }),
});
const arbNotice = fc.record({
  id: fc.string({ minLength: 1, maxLength: 6 }),
  level: fc.constantFrom("info", "success", "warn", "error", "needs-you"),
  read: fc.boolean(),
  updatedAt: fc.integer({ min: 0, max: 1e9 }),
});
const arbAwareness = fc.record({
  id: fc.string({ minLength: 1, maxLength: 6 }),
  priority: fc.integer({ min: 0, max: 120 }),
  recency: fc.integer({ min: 0, max: 1e9 }),
  hasAcs: fc.boolean(),
});

const arbInputs: fc.Arbitrary<FocusInputs> = fc.record({
  approvals: fc.array(arbApproval, { maxLength: 5 }),
  threads: fc.array(arbThread, { maxLength: 5 }),
  workflows: fc.array(arbWorkflow, { maxLength: 5 }),
  facts: fc.array(arbFact, { maxLength: 5 }),
  notifications: fc.array(arbNotice, { maxLength: 5 }),
  awareness: fc.array(arbAwareness, { maxLength: 5 }),
}).map((r) => ({
  approvals: r.approvals.map((a) =>
    approval({ id: a.id, risk: a.risk as RiskLevel, createdAt: a.createdAt }),
  ),
  threads: r.threads.map((t) => thread({ id: t.id, updatedAt: t.updatedAt, archived: t.archived })),
  activeThreadId: null,
  conversing: false,
  workflows: r.workflows.map((w) =>
    workflow({ id: w.id, status: w.status as WorkflowStatus, lastRunAt: w.lastRunAt }),
  ),
  facts: r.facts.map((f) => fact({ id: f.id, worth: f.worth, updatedAt: f.updatedAt })),
  notifications: r.notifications.map((n) =>
    notice({ id: n.id, level: n.level as NotificationLevel, read: n.read, updatedAt: n.updatedAt }),
  ),
  awareness: r.awareness.map(
    (s): AwarenessSignal => ({
      id: s.id,
      capability: "desktop",
      priority: s.priority,
      recency: s.recency,
      voiceText: `signal ${s.id}`,
      actionable: false,
      acsTitle: s.hasAcs ? `title ${s.id}` : undefined,
      acsLine: s.hasAcs ? `line ${s.id}` : undefined,
    }),
  ),
  now: 10_000,
}));

describe("Property 2: single-subject binding holds for all frames", () => {
  it("voiceLine.subjectId === acs.subjectId whenever both present", () => {
    fc.assert(
      fc.property(arbInputs, (inputs) => {
        const frame = deriveFocusFrame(inputs);
        if (frame.voiceLine && frame.acs) {
          expect(frame.acs.subjectId).toBe(frame.voiceLine.subjectId);
        }
        // Structural guardrails must also hold for every generated frame.
        expect(frame.chips.length).toBeLessThanOrEqual(MAX_CHIPS);
        expect(frame.orbit.every((p) => p.lit)).toBe(true);
        expect(() => assertFocusFrame(frame)).not.toThrow();
      }),
      { numRuns: 500 },
    );
  });

  it("derivation is deterministic (same inputs → identical frame)", () => {
    fc.assert(
      fc.property(arbInputs, (inputs) => {
        expect(deriveFocusFrame(inputs)).toEqual(deriveFocusFrame(inputs));
      }),
    );
  });
});

// ─── Property 1: Read-model purity ───────────────────────────────────────────
// Validates: Requirements 12.5

function resetStores(): void {
  approvalStore.setQueue([]);
  converseStore.setThreads([]);
  converseStore.setActiveThread(null);
  automationStore.setWorkflows([]);
  memoryStore.setFacts([]);
  notificationStore.setNotifications([]);
  clearAwarenessBridge();
}

/** Observable snapshot of every domain store the Focus engine reads. */
function snapshot(): string {
  return JSON.stringify({
    approvals: approvalStore.queue(),
    threads: converseStore.threads(),
    activeThreadId: converseStore.activeThreadId(),
    thinking: converseStore.thinking(),
    workflows: automationStore.workflows(),
    facts: memoryStore.facts(),
    notifications: notificationStore.notifications(),
  });
}

describe("Property 1: read-model purity (no domain writes / tool calls / sends)", () => {
  beforeEach(() => resetStores());
  afterEach(() => {
    vi.restoreAllMocks();
    resetStores();
  });

  it("reading the live frame across randomized signal sequences mutates nothing and emits nothing", () => {
    fc.assert(
      fc.property(fc.array(arbInputs, { minLength: 1, maxLength: 8 }), (sequence) => {
        for (const inputs of sequence) {
          // Apply the generated signals to the LIVE domain stores.
          approvalStore.setQueue([...inputs.approvals]);
          converseStore.setThreads([...inputs.threads]);
          automationStore.setWorkflows([...inputs.workflows]);
          memoryStore.setFacts([...inputs.facts]);
          notificationStore.setNotifications([...inputs.notifications]);
          setAwarenessBridge({ signals: () => inputs.awareness });

          const before = snapshot();
          const emitSpy = vi.spyOn(eventBus, "emit");

          // Read the reactive frame — the operation under test.
          const frame = homeFocusStore.frame();

          // Purity: no domain-store mutation, no event emission (⇒ no tool call
          // or send, which all route through the bus).
          expect(snapshot()).toBe(before);
          expect(emitSpy).not.toHaveBeenCalled();
          // And the frame is always structurally valid.
          expect(() => assertFocusFrame(frame)).not.toThrow();

          emitSpy.mockRestore();
        }
      }),
      { numRuns: 200 },
    );
  });
});

// ═══════════════════════════════════════════════════════════════════════════
// Task 3.2 — Staged pipeline: confidence, priority aging, temporal reasoning,
// TTL/expiration, and the "low-confidence → low-emphasis only" invariant.
// ═══════════════════════════════════════════════════════════════════════════

import {
  FOCUS_PRIORITY,
  CONFIDENCE_FLOOR,
  CONFIDENCE_MEDIUM,
  CONFIDENCE_HIGH,
  AGING_HALF_LIFE_MS,
  FINISHED_WORK_TTL_MS,
  classifyEmphasis,
} from "./homeFocusStore";

const MIN = 60_000;

/** Build a desktop-awareness signal with sane confidence-model defaults. */
const signal = (over: Partial<AwarenessSignal> = {}): AwarenessSignal => ({
  id: "s1",
  capability: "desktop",
  priority: 80,
  recency: 10_000,
  voiceText: "Meeting in 20 — prep notes?",
  actionable: false,
  ...over,
});

// ─── Confidence → emphasis mapping (§24 stages 3 & 8) ────────────────────────

describe("classifyEmphasis — confidence thresholds (§24 stage 8)", () => {
  it("maps confidence bands to emphasis (high/medium/low/hidden)", () => {
    expect(classifyEmphasis(1)).toBe("high");
    expect(classifyEmphasis(CONFIDENCE_HIGH)).toBe("high");
    expect(classifyEmphasis(CONFIDENCE_HIGH - 0.001)).toBe("medium");
    expect(classifyEmphasis(CONFIDENCE_MEDIUM)).toBe("medium");
    expect(classifyEmphasis(CONFIDENCE_MEDIUM - 0.001)).toBe("low");
    expect(classifyEmphasis(CONFIDENCE_FLOOR)).toBe("low");
    expect(classifyEmphasis(CONFIDENCE_FLOOR - 0.001)).toBe("hidden");
    expect(classifyEmphasis(0)).toBe("hidden");
  });
});

describe("deriveFocusFrame — per-subject confidence & emphasis (Req 24.2)", () => {
  it("a fully-trusted approval is high emphasis: Voice Line + ACS + coreHint", () => {
    const frame = deriveFocusFrame(emptyInputs({ approvals: [approval({ risk: "red" })] }));
    expect(frame.voiceLine?.emphasis).toBe("high");
    expect(frame.voiceLine?.confidence).toBe(1);
    expect(frame.acs).toBeDefined();
    expect(frame.coreHint).toBe("blocked");
  });

  it("default desktop awareness is medium: Voice Line only (no ACS, no coreHint/blaze)", () => {
    // 0.75 raw × 0.8 trust = 0.6 → medium.
    const frame = deriveFocusFrame(
      emptyInputs({
        awareness: [signal({ id: "m", acsTitle: "Standup", acsLine: "in 20" })],
        now: 10_000,
      }),
    );
    expect(frame.voiceLine?.subjectId).toBe("desktop:m");
    expect(frame.voiceLine?.emphasis).toBe("medium");
    expect(frame.acs).toBeUndefined(); // medium never earns the ACS expansion
    expect(frame.coreHint).toBeUndefined(); // never a step-forward blaze
    expect(() => assertFocusFrame(frame)).not.toThrow();
  });

  it("a low-confidence subject never headlines — it only lights the Orbit", () => {
    // 0.25 raw × 0.8 trust = 0.2 → low. Alone, nothing headlines.
    const frame = deriveFocusFrame(
      emptyInputs({ awareness: [signal({ id: "weak", confidence: 0.25 })], now: 10_000 }),
    );
    expect(frame.voiceLine).toBeUndefined();
    expect(frame.acs).toBeUndefined();
    expect(frame.orbit.some((p) => p.capability === "desktop")).toBe(true);
  });

  it("a below-floor subject does not surface at all (not even Orbit)", () => {
    // 0.1 raw × 0.8 trust = 0.08 < floor → hidden.
    const frame = deriveFocusFrame(
      emptyInputs({ awareness: [signal({ id: "noise", confidence: 0.1 })], now: 10_000 }),
    );
    expect(frame.voiceLine).toBeUndefined();
    expect(frame.orbit).toEqual([]);
    expect(frame.chips).toEqual([]);
  });

  it("a higher-PRIORITY low-confidence subject is demoted below a fresher confident one", () => {
    // Awareness has the higher precedence band (80) but low confidence (0.2);
    // the resumable thread is lower band (40) but confident (0.85). The
    // confident subject wins the headline — low-confidence never blazes.
    const frame = deriveFocusFrame(
      emptyInputs({
        awareness: [signal({ id: "weak", priority: FOCUS_PRIORITY.imminentEvent, confidence: 0.25 })],
        threads: [thread({ id: "keep", updatedAt: 9_000 })],
        now: 10_000,
      }),
    );
    expect(frame.voiceLine?.subjectId).toBe("thread:keep");
    // the low-confidence awareness still lights its Orbit point (low emphasis).
    expect(frame.orbit.some((p) => p.capability === "desktop")).toBe(true);
  });
});

// ─── Priority aging (§24.2) ──────────────────────────────────────────────────

describe("deriveFocusFrame — priority aging (§24.2)", () => {
  const completed = workflow({ id: "done", status: "completed", lastRunAt: 1_000_000 });

  it("finished work loses emphasis as it ages (high → medium → low → gone)", () => {
    const base = 1_000_000;
    const at = (now: number) =>
      deriveFocusFrame(emptyInputs({ workflows: [completed], now }));

    const fresh = at(base); // age 0
    expect(fresh.voiceLine?.emphasis).toBe("high");

    const oneHalfLife = at(base + AGING_HALF_LIFE_MS); // 0.9 × 0.5 = 0.45
    expect(oneHalfLife.voiceLine?.emphasis).toBe("medium");
    expect(oneHalfLife.acs).toBeUndefined();

    const older = at(base + 100 * MIN); // 0.9 × 0.5^(100/45) ≈ 0.19 → low
    expect(older.voiceLine).toBeUndefined(); // no longer headline-worthy
    expect(older.orbit.some((p) => p.capability === "automation")).toBe(true);

    const expired = at(base + FINISHED_WORK_TTL_MS + 1); // past TTL → removed
    expect(expired.orbit.some((p) => p.capability === "automation")).toBe(false);
  });

  it("confidence is monotonically non-increasing as a decaying subject ages", () => {
    const base = 1_000_000;
    const c0 = deriveFocusFrame(emptyInputs({ workflows: [completed], now: base }))
      .voiceLine?.confidence;
    const c1 = deriveFocusFrame(emptyInputs({ workflows: [completed], now: base + 10 * MIN }))
      .voiceLine?.confidence;
    expect(c0).toBeGreaterThan(c1!);
  });

  it("approvals never decay — an old pending approval stays a confident headline", () => {
    const frame = deriveFocusFrame(
      emptyInputs({ approvals: [approval({ risk: "green", createdAt: 0 })], now: 1e11 }),
    );
    expect(frame.voiceLine?.subjectId).toBe("approval:a1");
    expect(frame.voiceLine?.emphasis).toBe("high");
    expect(frame.voiceLine?.confidence).toBe(1);
  });
});

// ─── Temporal reasoning: imminence escalation (§24.2) ────────────────────────

describe("deriveFocusFrame — temporal reasoning (§24.2)", () => {
  const base = 1_000_000;
  const meeting = signal({
    id: "mtg",
    voiceText: "Meeting soon",
    confidence: 1,
    sourceTrust: 1,
    decays: false, // isolate the temporal factor from aging
    startsAt: base + 60 * MIN,
    leadWindowMs: 30 * MIN,
  });

  it("escalates confidence as the moment nears", () => {
    const far = deriveFocusFrame(emptyInputs({ awareness: [meeting], now: base }))
      .voiceLine?.confidence; // remaining 60m > lead 30m → base factor 0.6
    const near = deriveFocusFrame(emptyInputs({ awareness: [meeting], now: base + 45 * MIN }))
      .voiceLine?.confidence; // remaining 15m → factor 0.8
    const atMoment = deriveFocusFrame(emptyInputs({ awareness: [meeting], now: base + 60 * MIN }))
      .voiceLine?.confidence; // remaining 0 → factor 1

    expect(far).toBeCloseTo(0.6, 5);
    expect(near).toBeGreaterThan(far!);
    expect(atMoment).toBeCloseTo(1, 5);
    expect(atMoment!).toBeGreaterThan(near!);
  });

  it("expires after the moment + grace passes (temporal subject drops out)", () => {
    const duringGrace = deriveFocusFrame(
      emptyInputs({ awareness: [meeting], now: base + 60 * MIN + 4 * MIN }),
    );
    expect(duringGrace.voiceLine?.subjectId).toBe("desktop:mtg");

    const afterGrace = deriveFocusFrame(
      emptyInputs({ awareness: [meeting], now: base + 60 * MIN + 6 * MIN }),
    );
    expect(afterGrace.voiceLine).toBeUndefined();
    expect(afterGrace.orbit).toEqual([]);
  });
});

// ─── TTL / expiration (§24.3) ────────────────────────────────────────────────

describe("deriveFocusFrame — TTL / expiration (§24.3)", () => {
  it("removes a subject past its absolute expiry", () => {
    const frame = deriveFocusFrame(
      emptyInputs({
        awareness: [signal({ id: "gone", confidence: 1, sourceTrust: 1, expiresAt: 9_999 })],
        now: 10_000,
      }),
    );
    expect(frame.voiceLine).toBeUndefined();
    expect(frame.orbit).toEqual([]);
  });

  it("removes a subject once older than its TTL", () => {
    const frame = deriveFocusFrame(
      emptyInputs({
        awareness: [
          signal({ id: "stale", confidence: 1, sourceTrust: 1, recency: 0, ttlMs: 5_000 }),
        ],
        now: 10_000, // age 10_000 > ttl 5_000
      }),
    );
    expect(frame.voiceLine).toBeUndefined();
  });
});

// ─── Property: low-confidence never headlines (Req 24.2) ─────────────────────
// Validates: Requirements 24.2

describe("Property: the Voice Line headline always has confidence ≥ MEDIUM", () => {
  it("low-confidence subjects are never the headline across all inputs", () => {
    fc.assert(
      fc.property(arbInputs, (inputs) => {
        const frame = deriveFocusFrame(inputs);
        if (frame.voiceLine) {
          expect(frame.voiceLine.confidence).toBeGreaterThanOrEqual(CONFIDENCE_MEDIUM);
          expect(frame.voiceLine.emphasis === "high" || frame.voiceLine.emphasis === "medium").toBe(
            true,
          );
        }
        // An ACS + coreHint only ever accompany a HIGH-emphasis headline.
        if (frame.acs) {
          expect(frame.voiceLine?.emphasis).toBe("high");
        }
      }),
      { numRuns: 500 },
    );
  });
});

// ─── Property: expired subjects never surface (Req 24.3) ─────────────────────
// Validates: Requirements 24.3

const arbExpiredAwareness = fc
  .record({
    id: fc.string({ minLength: 1, maxLength: 6 }),
    age: fc.integer({ min: 1, max: 1e6 }),
  })
  .map(
    (r): AwarenessSignal => ({
      id: r.id,
      capability: "desktop",
      priority: FOCUS_PRIORITY.imminentEvent,
      recency: 10_000,
      voiceText: `expired ${r.id}`,
      confidence: 1,
      sourceTrust: 1,
      expiresAt: 10_000 - r.age, // strictly before `now`
    }),
  );

describe("Property: a frame built only from expired subjects fully rests", () => {
  it("no expired subject appears in the Voice Line, ACS, chips, or Orbit", () => {
    fc.assert(
      fc.property(fc.array(arbExpiredAwareness, { minLength: 1, maxLength: 8 }), (awareness) => {
        const frame = deriveFocusFrame(emptyInputs({ awareness, now: 10_000 }));
        expect(frame.voiceLine).toBeUndefined();
        expect(frame.acs).toBeUndefined();
        expect(frame.chips).toEqual([]);
        expect(frame.orbit).toEqual([]);
        expect(() => assertFocusFrame(frame)).not.toThrow();
      }),
      { numRuns: 300 },
    );
  });
});

// ─── Property: determinism under the injected clock (Req 24.1 / Property 4) ──
// Validates: Requirements 24.1, 12.2, 12.3

describe("Property: derivation is deterministic under the injected `now` clock", () => {
  it("same inputs + same clock → identical frame", () => {
    fc.assert(
      fc.property(arbInputs, fc.integer({ min: 0, max: 2e9 }), (inputs, now) => {
        const withClock = { ...inputs, now };
        expect(deriveFocusFrame(withClock)).toEqual(deriveFocusFrame(withClock));
      }),
      { numRuns: 300 },
    );
  });
});

// ═══════════════════════════════════════════════════════════════════════════
// Task 3.3 — Conflict resolution, notification suppression, debounced recompute
// (≤1/~250ms), and anti-flicker dwell. Determinism + no-thrash.
// Validates: Requirements 12.2, 12.3, 12.4, 24.4, 24.5
// ═══════════════════════════════════════════════════════════════════════════

import { createRoot } from "solid-js";
import {
  notificationQualifies,
  createRecomputeThrottle,
  createDwellStabilizer,
  createInterruptibilityGate,
  isInterruptibilityBlocked,
  createLiveFocusFrame,
  RECOMPUTE_DEBOUNCE_MS,
  MIN_DWELL_MS,
  MAX_BLOCKED_SURFACES,
  BLOCKED_CONTEXT_SOURCE_IDS,
  type FocusFrame,
} from "./homeFocusStore";

// ─── Conflict resolution: precedence → source-trust → recency (Req 24.5) ─────

describe("conflict resolution — deterministic total order (Req 12.2/12.3/24.5)", () => {
  it("resolves same-band conflicts by source-trust then recency", () => {
    // Two awareness signals in the SAME precedence band (imminentEvent=80),
    // both fully confident so both are headline-eligible. The more trustworthy
    // source wins even though it is slightly older (source-trust before recency).
    const frame = deriveFocusFrame(
      emptyInputs({
        awareness: [
          signal({ id: "trusted", priority: 80, recency: 100, confidence: 1, sourceTrust: 1 }),
          signal({ id: "shaky", priority: 80, recency: 9_000, confidence: 1, sourceTrust: 0.85 }),
        ],
        now: 10_000,
      }),
    );
    expect(frame.voiceLine?.subjectId).toBe("desktop:trusted");
  });

  it("falls back to recency when source-trust ties", () => {
    const frame = deriveFocusFrame(
      emptyInputs({
        awareness: [
          signal({ id: "old", priority: 80, recency: 100, confidence: 1, sourceTrust: 1 }),
          signal({ id: "fresh", priority: 80, recency: 9_000, confidence: 1, sourceTrust: 1 }),
        ],
        now: 10_000,
      }),
    );
    expect(frame.voiceLine?.subjectId).toBe("desktop:fresh");
  });

  it("precedence always dominates source-trust and recency (never two subjects)", () => {
    // A needs-you approval (precedence 100) beats a fresher, fully-trusted
    // imminent event (precedence 80). Only one subject ever headlines.
    const frame = deriveFocusFrame(
      emptyInputs({
        approvals: [approval({ id: "a", risk: "red", createdAt: 1 })],
        awareness: [signal({ id: "mtg", priority: 80, recency: 9_999, confidence: 1, sourceTrust: 1 })],
        now: 10_000,
      }),
    );
    expect(frame.voiceLine?.subjectId).toBe("approval:a");
    // ACS binds to the same subject — never a competing one (Req 12.3).
    expect(frame.acs?.subjectId).toBe("approval:a");
  });

  it("exposes the headline's fixed-precedence band on the Voice Line", () => {
    const frame = deriveFocusFrame(emptyInputs({ approvals: [approval()] }));
    expect(frame.voiceLine?.priority).toBe(FOCUS_PRIORITY.needsYou);
  });
});

// ─── Notification suppression (Req 12.4) ─────────────────────────────────────

describe("notificationQualifies — suppression rule (Req 12.4)", () => {
  it("surfaces the needs-you attention tier", () => {
    expect(notificationQualifies(notice({ level: "needs-you" }))).toBe(true);
  });

  it("surfaces warn/error problem tiers", () => {
    expect(notificationQualifies(notice({ level: "warn" }))).toBe(true);
    expect(notificationQualifies(notice({ level: "error" }))).toBe(true);
  });

  it("suppresses low-value ambient info/success with no action", () => {
    expect(notificationQualifies(notice({ level: "info" }))).toBe(false);
    expect(notificationQualifies(notice({ level: "success" }))).toBe(false);
  });

  it("surfaces an ambient notice only when it carries a real action", () => {
    expect(
      notificationQualifies(notice({ level: "info", action: { label: "View", route: "memory" } })),
    ).toBe(true);
  });

  it("never surfaces a read or dismissed notice", () => {
    expect(notificationQualifies(notice({ level: "needs-you", read: true }))).toBe(false);
    expect(notificationQualifies(notice({ level: "error", dismissedAt: 5 }))).toBe(false);
  });

  it("low-value ambient notices are dropped from the derived frame", () => {
    const frame = deriveFocusFrame(
      emptyInputs({ notifications: [notice({ id: "chatter", level: "success" })] }),
    );
    expect(frame.voiceLine).toBeUndefined();
    expect(frame.orbit).toEqual([]);
  });
});

// ─── Debounced recompute throttle: ≤1/~250ms, idle-quiet (Req 24.4) ──────────

/** Deterministic fake timer harness for the throttle (single pending timer). */
function fakeTimers() {
  let clock = 0;
  let pending: { at: number; fn: () => void } | null = null;
  let seq = 0;
  return {
    now: () => clock,
    setTimer: (fn: () => void, ms: number): number => {
      pending = { at: clock + ms, fn };
      return ++seq;
    },
    clearTimer: () => {
      pending = null;
    },
    /** Advance the clock, firing the pending timer if its deadline passes. */
    advance(ms: number) {
      const target = clock + ms;
      // Fire at most the single pending one-shot timer (throttle arms ≤1).
      while (pending && pending.at <= target) {
        clock = pending.at;
        const { fn } = pending;
        pending = null;
        fn();
      }
      clock = target;
    },
    get hasPending() {
      return pending !== null;
    },
  };
}

describe("createRecomputeThrottle — debounce ≤1/interval + idle-quiet (Req 24.4)", () => {
  it("runs the leading edge immediately", () => {
    const timers = fakeTimers();
    let runs = 0;
    const throttle = createRecomputeThrottle(() => runs++, { ...timers, intervalMs: 250 });
    throttle.schedule();
    expect(runs).toBe(1);
    expect(timers.hasPending).toBe(false); // idle after a single call
  });

  it("coalesces a burst within one window to a single trailing recompute", () => {
    const timers = fakeTimers();
    let runs = 0;
    const throttle = createRecomputeThrottle(() => runs++, { ...timers, intervalMs: 250 });

    // A burst of 100 signal changes across 0..200ms (all inside one window).
    throttle.schedule(); // leading run @0
    for (let i = 0; i < 99; i++) {
      timers.advance(2); // 2,4,...,198ms
      throttle.schedule();
    }
    expect(runs).toBe(1); // still only the leading run; trailing armed
    expect(timers.hasPending).toBe(true);

    timers.advance(250); // cross the window boundary → trailing fires once
    expect(runs).toBe(2); // 100 changes coalesced to exactly 2 recomputes
    expect(timers.hasPending).toBe(false); // idle-quiet: no perpetual timer
  });

  it("never exceeds one run per interval under sustained scheduling", () => {
    const timers = fakeTimers();
    let runs = 0;
    const throttle = createRecomputeThrottle(() => runs++, { ...timers, intervalMs: 250 });
    // Schedule every 50ms for ~2s (40 changes). Cadence must stay ≤1/250ms.
    for (let i = 0; i < 40; i++) {
      throttle.schedule();
      timers.advance(50);
    }
    // 40 changes over 2000ms → at most ceil(2000/250)+1 = 9 recomputes.
    expect(runs).toBeLessThanOrEqual(9);
    expect(runs).toBeGreaterThan(1);
  });

  it("goes fully idle (no timer) when signals stop changing", () => {
    const timers = fakeTimers();
    const throttle = createRecomputeThrottle(() => {}, { ...timers, intervalMs: 250 });
    throttle.schedule();
    timers.advance(50);
    throttle.schedule(); // arms trailing
    expect(timers.hasPending).toBe(true);
    timers.advance(250); // trailing fires
    expect(timers.hasPending).toBe(false);
    // No further scheduling → no timer ever re-arms.
    timers.advance(10_000);
    expect(timers.hasPending).toBe(false);
  });

  it("uses the documented default window", () => {
    expect(RECOMPUTE_DEBOUNCE_MS).toBe(250);
  });
});

// ─── Anti-flicker dwell stabilizer (Req 12.4, §5.4) ──────────────────────────

/** Frame with a single-subject headline at a given precedence band. */
function headlineFrame(subjectId: string, priority: number): FocusFrame {
  return {
    voiceLine: {
      subjectId,
      text: `about ${subjectId}`,
      key: subjectId,
      actionable: false,
      priority,
      confidence: 1,
      emphasis: "high",
    },
    acs: {
      subjectId,
      title: subjectId,
      line: "detail",
      ownerRoute: { space: "converse" },
    },
    chips: [],
    orbit: [],
  };
}

describe("createDwellStabilizer — anti-flicker hold (Req 12.4, §5.4)", () => {
  it("adopts the first headline immediately", () => {
    const dwell = createDwellStabilizer({ minDwellMs: 6_000 });
    const out = dwell.stabilize(headlineFrame("s1", 60), 0);
    expect(out.voiceLine?.subjectId).toBe("s1");
  });

  it("holds a subject against a LOWER-priority challenger until dwell elapses", () => {
    const dwell = createDwellStabilizer({ minDwellMs: 6_000 });
    dwell.stabilize(headlineFrame("session", 60), 0); // adopt @0

    // A lower-priority resumable thread (40) arrives at 3s — still within dwell.
    const held = dwell.stabilize(headlineFrame("thread", 40), 3_000);
    expect(held.voiceLine?.subjectId).toBe("session"); // incumbent held
    // Voice Line + ACS stay bound to the same (incumbent) subject.
    expect(held.acs?.subjectId).toBe("session");

    // After 6s the challenger may take over.
    const swapped = dwell.stabilize(headlineFrame("thread", 40), 6_100);
    expect(swapped.voiceLine?.subjectId).toBe("thread");
  });

  it("lets a strictly HIGHER-priority subject preempt immediately", () => {
    const dwell = createDwellStabilizer({ minDwellMs: 6_000 });
    dwell.stabilize(headlineFrame("session", 60), 0); // adopt @0
    // A needs-you approval (100) arrives at 1s — preempts despite dwell.
    const preempted = dwell.stabilize(headlineFrame("approval", 100), 1_000);
    expect(preempted.voiceLine?.subjectId).toBe("approval");
  });

  it("refreshes content for the same subject without re-arming dwell", () => {
    const dwell = createDwellStabilizer({ minDwellMs: 6_000 });
    dwell.stabilize(headlineFrame("session", 60), 0);
    // Same subject re-derived at 3s (content refresh), then a lower challenger
    // at 6.1s must still be allowed (dwell counts from the ORIGINAL adopt @0).
    dwell.stabilize(headlineFrame("session", 60), 3_000);
    const swapped = dwell.stabilize(headlineFrame("thread", 40), 6_100);
    expect(swapped.voiceLine?.subjectId).toBe("thread");
  });

  it("releases the incumbent immediately when the frame rests (no stale content)", () => {
    const dwell = createDwellStabilizer({ minDwellMs: 6_000 });
    dwell.stabilize(headlineFrame("session", 60), 0);
    const resting: FocusFrame = { chips: [], orbit: [] };
    const out = dwell.stabilize(resting, 1_000); // within dwell, but subject gone
    expect(out.voiceLine).toBeUndefined();
  });

  it("uses the documented default dwell", () => {
    expect(MIN_DWELL_MS).toBe(6_000);
  });
});

// ─── Property: dwell monotonicity / no-thrash (Req 12.4) ─────────────────────
// Validates: Requirements 12.4

describe("Property: a headline is never replaced by ≤-priority subjects within dwell", () => {
  it("holds the incumbent across arbitrary lower/equal-priority churn", () => {
    fc.assert(
      fc.property(
        fc.integer({ min: 1, max: 100 }), // incumbent priority
        fc.array(
          fc.record({
            id: fc.string({ minLength: 1, maxLength: 4 }),
            // challenger priority is always ≤ incumbent (never higher)
            dp: fc.integer({ min: 0, max: 99 }),
            dt: fc.integer({ min: 0, max: 5_999 }), // within the 6s dwell
          }),
          { maxLength: 12 },
        ),
        (incumbentPriority, churn) => {
          const dwell = createDwellStabilizer({ minDwellMs: 6_000 });
          dwell.stabilize(headlineFrame("INCUMBENT", incumbentPriority), 0);
          for (const c of churn) {
            const challengerPriority = Math.max(0, incumbentPriority - c.dp);
            // strictly-lower or equal priority challenger, inside dwell window
            if (challengerPriority >= incumbentPriority) continue;
            const out = dwell.stabilize(headlineFrame(c.id, challengerPriority), c.dt);
            expect(out.voiceLine?.subjectId).toBe("INCUMBENT");
            // binding invariant survives the hold
            if (out.voiceLine && out.acs) {
              expect(out.acs.subjectId).toBe(out.voiceLine.subjectId);
            }
          }
        },
      ),
      { numRuns: 300 },
    );
  });
});

// ─── Property: throttle emits ≤1 run per interval window (Req 24.4) ──────────
// Validates: Requirements 24.4

describe("Property: recompute cadence never exceeds one per interval", () => {
  it("coalesces arbitrary schedule bursts to ≤ ceil(span/interval)+1 runs", () => {
    fc.assert(
      fc.property(
        fc.array(fc.integer({ min: 0, max: 200 }), { minLength: 1, maxLength: 60 }),
        (gaps) => {
          const timers = fakeTimers();
          let runs = 0;
          const interval = 250;
          const throttle = createRecomputeThrottle(() => runs++, { ...timers, intervalMs: interval });
          let span = 0;
          for (const gap of gaps) {
            timers.advance(gap);
            span += gap;
            throttle.schedule();
          }
          // Flush any trailing run.
          timers.advance(interval);
          const upperBound = Math.ceil(span / interval) + 2;
          expect(runs).toBeLessThanOrEqual(upperBound);
          expect(runs).toBeGreaterThanOrEqual(1);
        },
      ),
      { numRuns: 300 },
    );
  });
});

// ─── Integration: live frame coalesces store bursts + holds a subject ────────

describe("createLiveFocusFrame — debounce + dwell wired to the live stores", () => {
  beforeEach(() => resetStores());
  afterEach(() => resetStores());

  it("derives its initial stable frame from the live stores through the dwell layer", () => {
    // Seed a needs-you approval alongside lower-priority churn signals.
    approvalStore.setQueue([approval({ id: "a", risk: "red", createdAt: 1 })]);
    automationStore.setWorkflows([workflow({ id: "w", status: "completed", lastRunAt: 5 })]);
    memoryStore.setFacts([fact({ id: "f", updatedAt: 5 })]);

    createRoot((dispose) => {
      const live = createLiveFocusFrame({ intervalMs: 250, minDwellMs: 6_000 });
      const frame = live.frame();
      // The high-precedence approval is the stable headline; ACS binds to it.
      expect(frame.voiceLine?.subjectId).toBe("approval:a");
      expect(frame.acs?.subjectId).toBe("approval:a");
      expect(() => assertFocusFrame(frame)).not.toThrow();
      live.dispose();
      dispose();
    });
  });

  it("reacts to a store change and re-publishes a coalesced, stable frame", async () => {
    approvalStore.setQueue([approval({ id: "a", risk: "red", createdAt: 1 })]);

    await new Promise<void>((resolve) => {
      createRoot((dispose) => {
        const live = createLiveFocusFrame({ intervalMs: 250, minDwellMs: 6_000 });
        // Mutate a lower-priority store; the effect coalesces the recompute.
        memoryStore.setFacts([fact({ id: "f", updatedAt: 5 })]);
        // Let the reactive effect + leading-edge recompute flush.
        queueMicrotask(() => {
          const frame = live.frame();
          // Approval (needs-you) still headlines through the churn (no thrash).
          expect(frame.voiceLine?.subjectId).toBe("approval:a");
          expect(() => assertFocusFrame(frame)).not.toThrow();
          live.dispose();
          dispose();
          resolve();
        });
      });
    });
  });
});

// ═══════════════════════════════════════════════════════════════════════════
// Task 3.4 — Greeting familiarity-scaling (full → short → none), no-consecutive-
// repeat, milestone-only rare greetings, cold-start truthfulness (no fabricated
// personalization), capped learned-facts, and bounded preference learning via
// the existing adaptive-ranking module.
// Validates: Requirements 12.6, 12.7, 24.6, 24.7, 27.1, 27.3
// ═══════════════════════════════════════════════════════════════════════════

import {
  deriveGreeting,
  personalizeFrame,
  GREETING_FULL_MAX_SESSIONS,
  GREETING_SHORT_MAX_SESSIONS,
  GREETING_MILESTONES,
  type GreetingInput,
  type FocusGreeting,
} from "./homeFocusStore";
import {
  dismissAdaptiveSuggestion,
  recordAdaptiveUse,
  resetAdaptiveSuggestions,
} from "../adaptive";
import { homeGreetingStore, LEARNED_FACT_COOLDOWN_MS } from "./homeGreetingStore";

/** Numeric verbosity rank (omitted greeting counts as `none`). */
const vrank = (g: FocusGreeting | undefined): number =>
  g === undefined || g.verbosity === "none" ? 0 : g.verbosity === "short" ? 1 : 2;

const greetingInput = (over: Partial<GreetingInput> = {}): GreetingInput => ({
  sessionCount: 0,
  dayStreak: 1,
  hourOfDay: 19, // evening
  ...over,
});

// ─── Familiarity-scaling (Req 12.6, §5.5) ────────────────────────────────────

describe("deriveGreeting — familiarity scaling full → short → none (Req 12.6)", () => {
  it("gives a new user a FULL greeting", () => {
    const g = deriveGreeting(greetingInput({ sessionCount: 1, name: "Obaid" }));
    expect(g?.verbosity).toBe("full");
    expect(g?.text).toBe("Good evening, Obaid.");
  });

  it("gives a regular user a SHORT greeting", () => {
    const g = deriveGreeting(greetingInput({ sessionCount: GREETING_FULL_MAX_SESSIONS + 1 }));
    expect(g?.verbosity).toBe("short");
    expect(g?.text).toBe("Evening.");
  });

  it("omits the greeting for a daily/power user (none)", () => {
    const g = deriveGreeting(greetingInput({ sessionCount: GREETING_SHORT_MAX_SESSIONS + 1 }));
    expect(g).toBeUndefined();
  });

  it("uses the time-of-day segment deterministically", () => {
    expect(deriveGreeting(greetingInput({ hourOfDay: 8, sessionCount: 1 }))?.text).toBe("Good morning.");
    expect(deriveGreeting(greetingInput({ hourOfDay: 14, sessionCount: 1 }))?.text).toBe(
      "Good afternoon.",
    );
    expect(deriveGreeting(greetingInput({ hourOfDay: 2, sessionCount: 1 }))?.text).toBe("Hello.");
  });
});

// ─── Milestone-only rare greetings (Req 27.1, §5.5) ──────────────────────────

describe("deriveGreeting — milestone-only rare greetings (Req 27.1)", () => {
  it("surfaces a rare milestone greeting even for a power user who normally gets none", () => {
    const g = deriveGreeting(
      greetingInput({ sessionCount: 500, dayStreak: 100 }), // would be `none` by familiarity
    );
    expect(g?.verbosity).toBe("full");
    expect(g?.text).toBe("100 days together.");
  });

  it("never repeats the milestone greeting consecutively (falls back to familiarity)", () => {
    const g = deriveGreeting(
      greetingInput({ sessionCount: 500, dayStreak: 100, lastGreetingText: "100 days together." }),
    );
    // Already shown last time → power user falls back to no greeting.
    expect(g).toBeUndefined();
  });

  it("does not treat a non-milestone streak as a milestone", () => {
    const notMilestone = 42;
    expect(GREETING_MILESTONES.includes(notMilestone)).toBe(false);
    const g = deriveGreeting(greetingInput({ sessionCount: 1, dayStreak: notMilestone, name: "A" }));
    expect(g?.text).toBe("Good evening, A."); // normal familiarity greeting
  });
});

// ─── No-consecutive-repeat (Req 12.6) ────────────────────────────────────────

describe("deriveGreeting — no-consecutive-repeat (Req 12.6)", () => {
  it("steps verbosity down when the base greeting would repeat", () => {
    // A new user would get "Good evening." — but that was just shown, so it
    // steps down to the short "Evening." instead (never the same text twice).
    const g = deriveGreeting(greetingInput({ sessionCount: 1, lastGreetingText: "Good evening." }));
    expect(g?.text).toBe("Evening.");
    expect(g?.text).not.toBe("Good evening.");
  });

  it("omits entirely when the short form would also repeat", () => {
    const g = deriveGreeting(
      greetingInput({ sessionCount: GREETING_FULL_MAX_SESSIONS + 1, lastGreetingText: "Evening." }),
    );
    expect(g).toBeUndefined();
  });
});

// ─── Cold-start truthfulness (Req 24.6, 27.3) ────────────────────────────────

describe("deriveGreeting — cold-start truthful generic output (Req 24.6/27.3)", () => {
  it("gives a brand-new user a truthful generic greeting with NO fabricated name", () => {
    const g = deriveGreeting(greetingInput({ sessionCount: 0 })); // cold start, no name
    expect(g?.verbosity).toBe("full");
    expect(g?.text).toBe("Good evening.");
    expect(g?.text).not.toContain(","); // no "welcome back, <name>" fabrication
  });
});

// ─── Property: no-consecutive-repeat holds across sequences (Req 12.6) ───────
// Validates: Requirements 12.6

const arbGreetingStep = fc.record({
  sessionCount: fc.integer({ min: 0, max: 40 }),
  dayStreak: fc.integer({ min: 0, max: 1200 }),
  hourOfDay: fc.integer({ min: 0, max: 23 }),
  name: fc.option(fc.string({ minLength: 1, maxLength: 8 }), { nil: undefined }),
});

describe("Property: greeting text never repeats consecutively (Req 12.6)", () => {
  it("no two consecutive derivations yield the same greeting text", () => {
    fc.assert(
      fc.property(fc.array(arbGreetingStep, { minLength: 1, maxLength: 20 }), (steps) => {
        let last: string | undefined;
        for (const step of steps) {
          const g = deriveGreeting({ ...step, lastGreetingText: last });
          if (g && last !== undefined) {
            expect(g.text).not.toBe(last);
          }
          last = g?.text; // omission clears the guard (undefined)
        }
      }),
      { numRuns: 500 },
    );
  });
});

// ─── Property: verbosity is monotonic (non-increasing) with familiarity ──────
// Validates: Requirements 12.6, 27.1

describe("Property: greeting verbosity is non-increasing as familiarity grows", () => {
  it("more sessions never yield a MORE verbose greeting (all else equal)", () => {
    fc.assert(
      fc.property(
        fc.integer({ min: 0, max: 60 }),
        fc.integer({ min: 0, max: 60 }),
        fc.integer({ min: 0, max: 23 }),
        fc.option(fc.string({ minLength: 1, maxLength: 8 }), { nil: undefined }),
        (a, b, hourOfDay, name) => {
          const lo = Math.min(a, b);
          const hi = Math.max(a, b);
          // Isolate familiarity: no repeat guard, a non-milestone streak.
          const base = { dayStreak: 3, hourOfDay, name, lastGreetingText: undefined };
          const gLo = deriveGreeting({ ...base, sessionCount: lo });
          const gHi = deriveGreeting({ ...base, sessionCount: hi });
          expect(vrank(gLo)).toBeGreaterThanOrEqual(vrank(gHi));
        },
      ),
      { numRuns: 500 },
    );
  });
});

// ─── Property: cold-start never fabricates personalization (Req 24.6/27.3) ───
// Validates: Requirements 24.6, 27.3

describe("Property: cold-start greeting never fabricates personalization", () => {
  it("a nameless user's greeting is always generic (no injected name)", () => {
    fc.assert(
      fc.property(
        fc.integer({ min: 0, max: 60 }),
        fc.integer({ min: 0, max: 23 }),
        fc.option(fc.string({ maxLength: 12 }), { nil: undefined }),
        (sessionCount, hourOfDay, lastGreetingText) => {
          const g = deriveGreeting({ sessionCount, dayStreak: 3, hourOfDay, lastGreetingText });
          if (g) {
            // With no name, the greeting is one of the fixed generic forms and
            // never contains a name segment (", <name>.").
            expect(g.text).not.toMatch(/,\s/);
            const generic = [
              "Good morning.",
              "Good afternoon.",
              "Good evening.",
              "Hello.",
              "Morning.",
              "Afternoon.",
              "Evening.",
            ];
            expect(generic).toContain(g.text);
          }
        },
      ),
      { numRuns: 500 },
    );
  });
});

// ─── Capped learned-facts (Req 12.7, 27.3, §5.6) ─────────────────────────────

describe("deriveFocusFrame — learned-fact frequency cap (Req 12.7/27.3)", () => {
  it("surfaces a worthwhile learned fact when the cap gate is open", () => {
    const frame = deriveFocusFrame(
      emptyInputs({ facts: [fact({ id: "linux", confidence: 0.9 })], learnedFactAllowed: true }),
    );
    expect(frame.orbit.some((p) => p.capability === "memory")).toBe(true);
  });

  it("withholds the learned fact entirely when the cap gate is closed", () => {
    const frame = deriveFocusFrame(
      emptyInputs({ facts: [fact({ id: "linux", confidence: 0.9 })], learnedFactAllowed: false }),
    );
    expect(frame.voiceLine?.subjectId?.startsWith("fact:") ?? false).toBe(false);
    expect(frame.orbit.some((p) => p.capability === "memory")).toBe(false);
    expect(frame.chips.every((c) => !c.id.startsWith("fact:"))).toBe(true);
  });
});

// ─── Property: a closed cap gate hides every learned-fact subject (Req 12.7) ─
// Validates: Requirements 12.7, 27.3

describe("Property: learned-facts are fully withheld when frequency-capped", () => {
  it("no learned-fact subject surfaces anywhere when learnedFactAllowed is false", () => {
    fc.assert(
      fc.property(arbInputs, (inputs) => {
        const capped = { ...inputs, learnedFactAllowed: false };
        const frame = deriveFocusFrame(capped);
        expect(frame.voiceLine?.subjectId?.startsWith("fact:") ?? false).toBe(false);
        expect(frame.orbit.some((p) => p.capability === "memory")).toBe(false);
        expect(frame.chips.some((c) => c.id.startsWith("fact:"))).toBe(false);
      }),
      { numRuns: 300 },
    );
  });
});

// ─── Bounded dismiss preference (Req 24.7) ───────────────────────────────────

describe("deriveFocusFrame — bounded dismiss preference (Req 24.7)", () => {
  it("suppresses a dismissed subject from the headline (exact-subject only)", () => {
    const base = emptyInputs({ approvals: [approval({ id: "a1", risk: "red" })] });
    // Without dismissal the approval headlines…
    expect(deriveFocusFrame(base).voiceLine?.subjectId).toBe("approval:a1");
    // …dismissing that exact subject removes it (no headline, calm frame).
    const frame = deriveFocusFrame({ ...base, dismissedSubjects: ["approval:a1"] });
    expect(frame.voiceLine).toBeUndefined();
    expect(frame.orbit.some((p) => p.capability === "approval")).toBe(false);
  });

  it("leaves non-dismissed subjects untouched (bounded, no band reorder)", () => {
    const frame = deriveFocusFrame(
      emptyInputs({
        approvals: [approval({ id: "keep", risk: "red" })],
        threads: [thread({ id: "t1" })],
        dismissedSubjects: ["approval:other"],
      }),
    );
    expect(frame.voiceLine?.subjectId).toBe("approval:keep");
  });
});

// ─── Bounded preference learning REUSES the adaptive-ranking module (Req 24.7) ─

describe("personalizeFrame — bounded chip reorder via adaptive-ranking (Req 24.7)", () => {
  beforeEach(() => resetAdaptiveSuggestions("empty-state"));
  afterEach(() => resetAdaptiveSuggestions("empty-state"));

  const chip = (id: string) => ({ id, label: id, icon: "dot", kind: "route" as const, payload: "converse" as const });

  it("preserves every chip and the ≤MAX_CHIPS budget", () => {
    const frame: FocusFrame = { chips: [chip("a"), chip("b"), chip("c")], orbit: [] };
    const out = personalizeFrame(frame);
    expect(out.chips.map((c) => c.id).sort()).toEqual(["a", "b", "c"]);
    expect(out.chips.length).toBeLessThanOrEqual(MAX_CHIPS);
  });

  it("promotes a habitually-used chip within the adaptive bound", () => {
    // Record repeated use of the last chip; the module may promote it up to its
    // ±shift bound — a bounded, presentation-only reorder (never a new chip).
    recordAdaptiveUse("empty-state", "c");
    recordAdaptiveUse("empty-state", "c");
    const frame: FocusFrame = { chips: [chip("a"), chip("b"), chip("c")], orbit: [] };
    const out = personalizeFrame(frame);
    expect(out.chips.findIndex((c) => c.id === "c")).toBeLessThan(2); // moved up, bounded
    expect(out.chips.map((c) => c.id).sort()).toEqual(["a", "b", "c"]); // none lost
  });

  it("filters a dismissed chip from the surface", () => {
    dismissAdaptiveSuggestion("empty-state", "b");
    const frame: FocusFrame = { chips: [chip("a"), chip("b"), chip("c")], orbit: [] };
    const out = personalizeFrame(frame);
    expect(out.chips.some((c) => c.id === "b")).toBe(false);
  });
});

// ─── homeGreetingStore: persisted familiarity + learned-fact cap ─────────────

describe("homeGreetingStore — session/streak + learned-fact cap", () => {
  beforeEach(() => homeGreetingStore.resetGreetingState());
  afterEach(() => homeGreetingStore.resetGreetingState());

  const DAY = 24 * 60 * 60 * 1000;

  it("cold start reads sessionCount 0 (brand-new user)", () => {
    expect(homeGreetingStore.readGreetingInput(0).sessionCount).toBe(0);
  });

  it("increments session count and grows the streak across consecutive days", () => {
    homeGreetingStore.beginSession(0); // day 0
    expect(homeGreetingStore.readGreetingInput(0).sessionCount).toBe(1);
    expect(homeGreetingStore.readGreetingInput(0).dayStreak).toBe(1);

    homeGreetingStore.beginSession(DAY); // next day
    expect(homeGreetingStore.readGreetingInput(DAY).dayStreak).toBe(2);

    homeGreetingStore.beginSession(DAY + 1000); // same day → streak unchanged
    expect(homeGreetingStore.readGreetingInput(DAY).dayStreak).toBe(2);

    homeGreetingStore.beginSession(DAY * 5); // gap → reset to 1
    expect(homeGreetingStore.readGreetingInput(DAY * 5).dayStreak).toBe(1);
  });

  it("records the last greeting so the next derivation can avoid repeating it", () => {
    homeGreetingStore.noteGreetingShown("Good evening.");
    expect(homeGreetingStore.readGreetingInput(0).lastGreetingText).toBe("Good evening.");
    homeGreetingStore.noteGreetingShown(undefined); // omission clears the guard
    expect(homeGreetingStore.readGreetingInput(0).lastGreetingText).toBeUndefined();
  });

  it("caps learned-fact frequency via the cooldown window", () => {
    expect(homeGreetingStore.learnedFactAllowed(0)).toBe(true);
    homeGreetingStore.noteLearnedFactShown(0);
    expect(homeGreetingStore.learnedFactAllowed(0)).toBe(false); // inside cooldown
    expect(homeGreetingStore.learnedFactAllowed(LEARNED_FACT_COOLDOWN_MS - 1)).toBe(false);
    expect(homeGreetingStore.learnedFactAllowed(LEARNED_FACT_COOLDOWN_MS)).toBe(true); // reopened
  });

  it("never fabricates a name (only reflects an explicitly set one)", () => {
    expect(homeGreetingStore.readGreetingInput(0).name).toBeUndefined();
    homeGreetingStore.setUserName("Obaid");
    expect(homeGreetingStore.readGreetingInput(0).name).toBe("Obaid");
  });
});

// ═══════════════════════════════════════════════════════════════════════════
// Task 3.5 — Valid empty/resting output at EVERY capability tier.
//
// Guarantees that `deriveFocusFrame` (and the live layer) ALWAYS returns a
// structurally valid, calm resting output — at every tier (Tier 0 = Core +
// Composer only, no stores wired), when every signal source is empty, and when
// any subset of sources is absent/throwing (a broken subsystem degrades to
// OMISSION, never a broken frame).
// Validates: Requirements 12.5, 1.5, 28.1
// ═══════════════════════════════════════════════════════════════════════════

/**
 * An array-like signal source that throws on ANY access (iteration, `.map`,
 * `.filter`, …) — simulates a subsystem that is present in the input shape but
 * broken/unavailable at read time. The per-source guard must degrade it to
 * omission (Req 28.1) rather than let it break the whole frame.
 */
function throwingSource<T>(): readonly T[] {
  return new Proxy([] as T[], {
    get() {
      throw new Error("subsystem unavailable");
    },
  }) as unknown as readonly T[];
}

/** The six array-valued signal sources the Focus engine fuses. */
const SOURCE_KEYS = [
  "approvals",
  "threads",
  "workflows",
  "facts",
  "notifications",
  "awareness",
] as const;
type SourceKey = (typeof SOURCE_KEYS)[number];

/** Assert a frame is a valid, calm, fully-resting output (no headline, no filler). */
function expectFullyResting(frame: FocusFrame): void {
  expect(frame.voiceLine).toBeUndefined();
  expect(frame.acs).toBeUndefined();
  expect(frame.chips).toEqual([]);
  expect(frame.orbit).toEqual([]);
  expect(frame.coreHint).toBeUndefined();
  // Structural guardrails hold (release-blocking invariants never violated).
  expect(() => assertFocusFrame(frame)).not.toThrow();
}

// ─── Pure engine: resting guarantee across tiers & failures ──────────────────

describe("deriveFocusFrame — valid resting output at every tier (Req 12.5/1.5/28.1)", () => {
  it("fully rests at Tier 0 / when every signal source is empty", () => {
    // Tier 0 (Core + Composer only): no signals at all → calm resting frame.
    expectFullyResting(deriveFocusFrame(emptyInputs()));
  });

  it("rests calmly with no greeting state and no personalization inputs", () => {
    const frame = deriveFocusFrame(emptyInputs({ greeting: undefined }));
    expect(frame.greeting).toBeUndefined();
    expectFullyResting(frame);
  });

  it("a single throwing source degrades to omission — other tiers still surface", () => {
    // Awareness subsystem is broken, but a wired approval must still headline
    // (each subsystem degrades independently — design §30).
    const frame = deriveFocusFrame(
      emptyInputs({
        approvals: [approval({ id: "a1", risk: "red" })],
        awareness: throwingSource<AwarenessSignal>(),
      }),
    );
    expect(frame.voiceLine?.subjectId).toBe("approval:a1");
    expect(frame.acs?.subjectId).toBe("approval:a1");
    expect(() => assertFocusFrame(frame)).not.toThrow();
  });

  it("produces a valid resting output when EVERY source throws", () => {
    const allBroken = emptyInputs();
    for (const key of SOURCE_KEYS) {
      // deliberately break each array-valued source
      (allBroken as unknown as Record<SourceKey, readonly unknown[]>)[key] = throwingSource();
    }
    expectFullyResting(deriveFocusFrame(allBroken));
  });
});

// ─── Property: all-empty / all-throwing sources → valid resting frame ────────
// Validates: Requirements 12.5, 1.5, 28.1

describe("Property: any mix of empty/throwing sources yields a calm resting frame", () => {
  it("no subset of broken/absent sources ever breaks the frame or emits filler", () => {
    fc.assert(
      fc.property(fc.subarray([...SOURCE_KEYS]), (broken) => {
        const inputs = emptyInputs();
        for (const key of broken) {
          (inputs as unknown as Record<SourceKey, readonly unknown[]>)[key] = throwingSource();
        }
        // Nothing qualifies (all sources empty or broken) → fully-resting frame.
        expectFullyResting(deriveFocusFrame(inputs));
      }),
      { numRuns: 300 },
    );
  });
});

// ─── Property: a broken source is indistinguishable from an omitted one ──────
// Validates: Requirements 28.1, 12.5

describe("Property: per-source failure degrades to omission (never breaks the frame)", () => {
  it("breaking any subset of sources equals omitting them, and stays valid", () => {
    fc.assert(
      fc.property(arbInputs, fc.subarray([...SOURCE_KEYS]), (inputs, broken) => {
        const brokenInputs = { ...inputs } as FocusInputs;
        const omittedInputs = { ...inputs } as FocusInputs;
        for (const key of broken) {
          (brokenInputs as unknown as Record<SourceKey, readonly unknown[]>)[key] = throwingSource();
          (omittedInputs as unknown as Record<SourceKey, readonly unknown[]>)[key] = [];
        }
        const brokenFrame = deriveFocusFrame(brokenInputs);
        // Always structurally valid — a broken subsystem never breaks the frame.
        expect(() => assertFocusFrame(brokenFrame)).not.toThrow();
        // Degrade-to-omission: a throwing source contributes exactly nothing, so
        // the frame is identical to one where those sources were simply empty.
        expect(brokenFrame).toEqual(deriveFocusFrame(omittedInputs));
      }),
      { numRuns: 400 },
    );
  });
});

// ─── Live layer: resting guarantee when subsystems are missing/broken ────────

describe("createLiveFocusFrame — valid resting output across tiers (Req 28.1)", () => {
  beforeEach(() => {
    resetStores();
    homeGreetingStore.resetGreetingState();
  });
  afterEach(() => {
    vi.restoreAllMocks();
    resetStores();
    homeGreetingStore.resetGreetingState();
  });

  it("rests calmly at Tier 0 (no stores wired / all empty)", () => {
    createRoot((dispose) => {
      const live = createLiveFocusFrame();
      // A cold-start greeting is allowed (Core + optional greeting only) but no
      // Focus subject, chips, or Orbit may appear when nothing qualifies.
      expectFullyResting(live.frame());
      live.dispose();
      dispose();
    });
  });

  it("a throwing subsystem degrades to omission without breaking the live frame", () => {
    // A wired approval works; memory + awareness subsystems are broken.
    approvalStore.setQueue([approval({ id: "a", risk: "red", createdAt: 1 })]);
    vi.spyOn(memoryStore, "facts").mockImplementation(() => {
      throw new Error("memory unavailable");
    });
    setAwarenessBridge({
      signals: () => {
        throw new Error("awareness unavailable");
      },
    });

    createRoot((dispose) => {
      const live = createLiveFocusFrame();
      const frame = live.frame();
      // The working subsystem still surfaces; the broken ones are simply omitted.
      expect(frame.voiceLine?.subjectId).toBe("approval:a");
      expect(() => assertFocusFrame(frame)).not.toThrow();
      live.dispose();
      dispose();
    });
  });

  it("produces a valid resting output when EVERY domain store throws", () => {
    const boom = (name: string) => () => {
      throw new Error(`${name} unavailable`);
    };
    vi.spyOn(approvalStore, "queue").mockImplementation(boom("approvals"));
    vi.spyOn(converseStore, "threads").mockImplementation(boom("threads"));
    vi.spyOn(converseStore, "activeThreadId").mockImplementation(boom("activeThread"));
    vi.spyOn(converseStore, "thinking").mockImplementation(boom("thinking"));
    vi.spyOn(automationStore, "workflows").mockImplementation(boom("workflows"));
    vi.spyOn(memoryStore, "facts").mockImplementation(boom("facts"));
    vi.spyOn(notificationStore, "active").mockImplementation(boom("notifications"));
    setAwarenessBridge({ signals: boom("awareness") });

    createRoot((dispose) => {
      const live = createLiveFocusFrame();
      // Every subsystem is down, yet the homepage still yields a calm, valid frame.
      expectFullyResting(live.frame());
      live.dispose();
      dispose();
    });
  });
});

// ─── Capability Tier resolver (task 3.6, Req 28.1/28.2, design §30) ──────────

import {
  resolveCapabilityState,
  subsystemsAtTier,
  capabilitySubjects,
  ALL_SUBSYSTEMS,
  SUBSYSTEM_TIER,
  type CapabilitySubsystem,
  type CapabilityAvailability,
  type CapabilitySubject,
} from "./homeFocusStore";

/** Availability map with the given subsystems turned OFF (all others ON). */
function availabilityWithout(off: readonly CapabilitySubsystem[]): Partial<CapabilityAvailability> {
  const partial: Partial<CapabilityAvailability> = {};
  for (const subsystem of off) partial[subsystem] = false;
  return partial;
}

/** The subsystem a subjectId belongs to is reported by {@link capabilitySubjects}. */
function subjectIds(subjects: readonly CapabilitySubject[]): string[] {
  return subjects.map((s) => s.subjectId).sort();
}

/** A rich signal snapshot exercising every subsystem at once (Tier 5). */
function richInputs(over: Partial<FocusInputs> = {}): FocusInputs {
  return emptyInputs({
    approvals: [approval({ id: "a1", risk: "red" })],
    threads: [thread({ id: "t1", updatedAt: 9_000 })],
    workflows: [workflow({ id: "w1", status: "running" })],
    facts: [fact({ id: "f1", confidence: 0.9, worth: 1 })],
    notifications: [notice({ id: "n1", level: "needs-you" })],
    awareness: [
      signal({ id: "desk", capability: "desktop", confidence: 1, sourceTrust: 1 }),
      signal({ id: "cal", capability: "calendar", confidence: 1, sourceTrust: 1, priority: 80 }),
    ],
    now: 10_000,
    ...over,
  });
}

describe("resolveCapabilityState — tier model (Req 28.2)", () => {
  it("reports Tier 5 (Everything) with the full tier ladder when all subsystems are up", () => {
    const state = resolveCapabilityState();
    expect(state.tier0Usable).toBe(true);
    expect(state.availableTiers).toEqual([0, 1, 2, 3, 4, 5]);
    expect(state.highestTier).toBe(5);
    for (const subsystem of ALL_SUBSYSTEMS) expect(state.subsystems[subsystem]).toBe(true);
  });

  it("keeps only Tier 0 usable when every subsystem is off", () => {
    const state = resolveCapabilityState(availabilityWithout(ALL_SUBSYSTEMS));
    expect(state.tier0Usable).toBe(true);
    expect(state.availableTiers).toEqual([0]);
    expect(state.highestTier).toBe(0);
  });

  it("drops only the missing tier — a down subsystem never removes another tier", () => {
    // Desktop awareness (Tier 3) off; calendar (Tier 4) still up → Tier 4 present,
    // Tier 3 absent, no Tier 5 (not everything is up). Tiers 1/2 untouched.
    const state = resolveCapabilityState(availabilityWithout(["desktop"]));
    expect(state.availableTiers).toEqual([0, 1, 2, 4]);
    expect(state.highestTier).toBe(4);
    expect(state.subsystems.desktop).toBe(false);
    expect(state.subsystems.calendar).toBe(true);
  });

  it("keeps a tier available while any one of its subsystems is up (conversation/memory share Tier 1)", () => {
    const onlyMemory = resolveCapabilityState(availabilityWithout(["conversation"]));
    expect(onlyMemory.availableTiers).toContain(1); // memory still carries Tier 1
    const neither = resolveCapabilityState(availabilityWithout(["conversation", "memory"]));
    expect(neither.availableTiers).not.toContain(1);
  });

  it("subsystemsAtTier + SUBSYSTEM_TIER agree with the design §30 additive model", () => {
    expect(subsystemsAtTier(0).sort()).toEqual(["approval", "notification"]);
    expect(subsystemsAtTier(1).sort()).toEqual(["conversation", "memory"]);
    expect(subsystemsAtTier(2)).toEqual(["automation"]);
    expect(subsystemsAtTier(3)).toEqual(["desktop"]);
    expect(subsystemsAtTier(4)).toEqual(["calendar"]);
    for (const subsystem of ALL_SUBSYSTEMS) {
      expect(subsystemsAtTier(SUBSYSTEM_TIER[subsystem])).toContain(subsystem);
    }
  });
});

describe("deriveFocusFrame — Tier 0 stays fully usable (Req 28.1)", () => {
  it("rests calmly even with rich signals when every subsystem is unavailable", () => {
    // Tier 0 = Core + Composer + talk: no subsystem contributes, but the frame
    // is still structurally valid and calm — never broken.
    const frame = deriveFocusFrame(richInputs({ availability: availabilityWithout(ALL_SUBSYSTEMS) }));
    expectFullyResting(frame);
  });

  it("still greets at Tier 0 (greeting is core courtesy, not a gated subsystem)", () => {
    const frame = deriveFocusFrame(
      richInputs({
        availability: availabilityWithout(ALL_SUBSYSTEMS),
        greeting: { sessionCount: 0, dayStreak: 0, hourOfDay: 9 },
      }),
    );
    expect(frame.greeting?.text).toBe("Good morning.");
    // No subjects surface — Tier 0 has presence + composer + talk + greeting only.
    expect(frame.voiceLine).toBeUndefined();
    expect(frame.chips).toEqual([]);
    expect(frame.orbit).toEqual([]);
    expect(() => assertFocusFrame(frame)).not.toThrow();
  });
});

describe("capabilitySubjects — independent degradation (Req 28.1/28.2)", () => {
  it("removing a subsystem removes ONLY its subjects, leaving the others intact", () => {
    const all = capabilitySubjects(richInputs());
    // Every subsystem with signals contributes at least one subject at Tier 5.
    expect(all.some((s) => s.subsystem === "approval")).toBe(true);
    expect(all.some((s) => s.subsystem === "automation")).toBe(true);
    expect(all.some((s) => s.subsystem === "calendar")).toBe(true);

    // Disable automation only.
    const withoutAutomation = capabilitySubjects(
      richInputs({ availability: availabilityWithout(["automation"]) }),
    );
    // Automation subjects are gone…
    expect(withoutAutomation.some((s) => s.subsystem === "automation")).toBe(false);
    // …and every other subsystem's subjects are exactly preserved.
    expect(subjectIds(withoutAutomation)).toEqual(
      subjectIds(all.filter((s) => s.subsystem !== "automation")),
    );
  });

  it("degrades desktop and calendar independently (both ride the same bridge)", () => {
    const all = capabilitySubjects(richInputs());
    const withoutDesktop = capabilitySubjects(
      richInputs({ availability: availabilityWithout(["desktop"]) }),
    );
    expect(withoutDesktop.some((s) => s.subsystem === "desktop")).toBe(false);
    // Calendar (Tier 4) survives even though desktop (Tier 3) was removed.
    expect(withoutDesktop.some((s) => s.subsystem === "calendar")).toBe(true);
    expect(subjectIds(withoutDesktop)).toEqual(
      subjectIds(all.filter((s) => s.subsystem !== "desktop")),
    );
  });

  it("tags each surfacing subject with its subsystem's tier", () => {
    for (const subject of capabilitySubjects(richInputs())) {
      expect(subject.tier).toBe(SUBSYSTEM_TIER[subject.subsystem]);
    }
  });
});

// ─── Property: independent degradation (Req 28.2) ────────────────────────────
// Validates: Requirements 28.2

describe("Property: removing any subset of subsystems removes only those subjects", () => {
  it("degraded subjects == full subjects minus the disabled subsystems' subjects", () => {
    fc.assert(
      fc.property(arbInputs, fc.subarray([...ALL_SUBSYSTEMS]), (inputs, disabled) => {
        const disabledSet = new Set<CapabilitySubsystem>(disabled);
        const all = capabilitySubjects({ ...inputs, availability: undefined });
        const degraded = capabilitySubjects({
          ...inputs,
          availability: availabilityWithout(disabled),
        });
        // 1. Independence: the degraded set is EXACTLY the full set with the
        //    disabled subsystems' subjects removed — no other subject is touched.
        expect(subjectIds(degraded)).toEqual(
          subjectIds(all.filter((s) => !disabledSet.has(s.subsystem))),
        );
        // 2. No disabled subsystem contributes any subject.
        expect(degraded.every((s) => !disabledSet.has(s.subsystem))).toBe(true);
      }),
      { numRuns: 500 },
    );
  });
});

// ─── Property: Tier 0 always usable + frame always valid (Req 28.1) ──────────
// Validates: Requirements 28.1

const arbAvailability: fc.Arbitrary<Partial<CapabilityAvailability>> = fc
  .subarray([...ALL_SUBSYSTEMS])
  .map(availabilityWithout);

describe("Property: the frame is valid at every capability tier and Tier 0 stays usable", () => {
  it("any availability yields a structurally valid frame; all-off rests calmly", () => {
    fc.assert(
      fc.property(arbInputs, arbAvailability, (inputs, availability) => {
        const frame = deriveFocusFrame({ ...inputs, availability });
        // Frame always valid — a missing tier never breaks the frame (Req 28.1).
        expect(() => assertFocusFrame(frame)).not.toThrow();
        expect(frame.chips.length).toBeLessThanOrEqual(MAX_CHIPS);
        expect(frame.orbit.every((p) => p.lit)).toBe(true);
        if (frame.voiceLine && frame.acs) {
          expect(frame.acs.subjectId).toBe(frame.voiceLine.subjectId);
        }
      }),
      { numRuns: 500 },
    );
  });

  it("Tier 0 (all subsystems off) always fully rests, whatever the signals", () => {
    fc.assert(
      fc.property(arbInputs, (inputs) => {
        const frame = deriveFocusFrame({
          ...inputs,
          greeting: undefined,
          availability: availabilityWithout(ALL_SUBSYSTEMS),
        });
        expectFullyResting(frame);
      }),
      { numRuns: 300 },
    );
  });

  it("resolveCapabilityState always keeps Tier 0 usable and listed", () => {
    fc.assert(
      fc.property(arbAvailability, (availability) => {
        const state = resolveCapabilityState(availability);
        expect(state.tier0Usable).toBe(true);
        expect(state.availableTiers).toContain(0);
        expect(state.availableTiers[0]).toBe(0);
        expect(state.highestTier).toBeGreaterThanOrEqual(0);
      }),
      { numRuns: 300 },
    );
  });
});

// ─── Live layer: capability-tier state is queryable (task 3.6) ───────────────

describe("homeFocusStore.capabilityState — live queryable state (Req 28)", () => {
  afterEach(() => {
    clearAwarenessBridge();
  });

  it("reports desktop awareness OFF by default (no bridge wired)", () => {
    clearAwarenessBridge();
    const state = homeFocusStore.capabilityState();
    expect(state.tier0Usable).toBe(true);
    expect(state.subsystems.desktop).toBe(false);
    expect(state.subsystems.calendar).toBe(false);
    // First-party subsystems are present in the local build.
    expect(state.subsystems.approval).toBe(true);
    expect(state.subsystems.automation).toBe(true);
    expect(state.availableTiers).not.toContain(3);
    expect(state.availableTiers).not.toContain(5);
  });

  it("reports desktop + calendar up (Tier 5) once a real bridge is wired", () => {
    setAwarenessBridge({ signals: () => [] });
    const state = homeFocusStore.capabilityState();
    expect(state.subsystems.desktop).toBe(true);
    expect(state.subsystems.calendar).toBe(true);
    expect(state.availableTiers).toEqual([0, 1, 2, 3, 4, 5]);
    expect(state.highestTier).toBe(5);
  });
});

// ═══════════════════════════════════════════════════════════════════════════
// Task 3.9 — Interruptibility gate (default-silent posture; only RED approvals
// surface in a blocked context — calm, via the ember, never audio; at-most-one
// gentle re-surface then age-out). Unit + property.
// Validates: Requirements 26.1, 26.2, 26.3, 26.4
// ═══════════════════════════════════════════════════════════════════════════

// ─── Blocked-context detection (Req 26.2/26.3) ───────────────────────────────

describe("isInterruptibilityBlocked — blocked-context detection (Req 26.2)", () => {
  it("is interruptible by default (no signals, no override)", () => {
    expect(isInterruptibilityBlocked(emptyInputs())).toBe(false);
  });

  it("is blocked via the explicit override input", () => {
    expect(isInterruptibilityBlocked(emptyInputs({ interruptibilityBlocked: true }))).toBe(true);
  });

  it("is blocked when a signal reports blocksInterruptibility", () => {
    const inputs = emptyInputs({
      awareness: [signal({ id: "call", blocksInterruptibility: true })],
    });
    expect(isInterruptibilityBlocked(inputs)).toBe(true);
  });

  it("is blocked when a known blocked-context source id is present", () => {
    for (const id of BLOCKED_CONTEXT_SOURCE_IDS) {
      const inputs = emptyInputs({ awareness: [signal({ id })] });
      expect(isInterruptibilityBlocked(inputs)).toBe(true);
    }
  });

  it("stays interruptible for an unrelated awareness signal", () => {
    expect(isInterruptibilityBlocked(emptyInputs({ awareness: [signal({ id: "media" })] }))).toBe(
      false,
    );
  });
});

// ─── Default-silent posture: only RED approvals surface (Req 26.1/26.2) ──────

describe("deriveFocusFrame — blocked context surfaces only RED approvals (Req 26.2)", () => {
  it("surfaces a RED approval calmly and flags the blocked context", () => {
    const frame = deriveFocusFrame(
      emptyInputs({
        approvals: [approval({ id: "danger", risk: "red" })],
        interruptibilityBlocked: true,
      }),
    );
    expect(frame.blockedContext).toBe(true);
    expect(frame.voiceLine?.subjectId).toBe("approval:danger");
    // Same subject binds Voice Line + ACS (Property 2 preserved).
    expect(frame.acs?.subjectId).toBe("approval:danger");
    // Advisory calm coreHint only — never an audio/voice/speaking directive.
    expect(frame.coreHint).toBe("blocked");
    expect(() => assertFocusFrame(frame)).not.toThrow();
  });

  it("surfaces a BLACK approval too (also RED-band risk)", () => {
    const frame = deriveFocusFrame(
      emptyInputs({ approvals: [approval({ id: "wipe", risk: "black" })], interruptibilityBlocked: true }),
    );
    expect(frame.voiceLine?.subjectId).toBe("approval:wipe");
  });

  it("defers a YELLOW approval (not RED) — nothing headlines, frame rests calmly", () => {
    const frame = deriveFocusFrame(
      emptyInputs({ approvals: [approval({ id: "mild", risk: "yellow" })], interruptibilityBlocked: true }),
    );
    expect(frame.blockedContext).toBe(true);
    expect(frame.voiceLine).toBeUndefined();
    expect(frame.acs).toBeUndefined();
    expect(frame.chips).toEqual([]);
    expect(frame.orbit).toEqual([]);
    expect(() => assertFocusFrame(frame)).not.toThrow();
  });

  it("defers every non-RED subject (thread, running automation, notice, awareness)", () => {
    const frame = deriveFocusFrame(
      emptyInputs({
        threads: [thread({ id: "t1", updatedAt: 9_000 })],
        workflows: [workflow({ id: "w1", status: "running" })],
        notifications: [notice({ id: "n1", level: "needs-you" })],
        awareness: [signal({ id: "media", priority: FOCUS_PRIORITY.imminentEvent })],
        interruptibilityBlocked: true,
      }),
    );
    expect(frame.voiceLine).toBeUndefined();
    expect(frame.chips).toEqual([]);
    expect(frame.orbit).toEqual([]);
  });

  it("defers the noise but lets a co-present RED approval through (only it surfaces)", () => {
    const frame = deriveFocusFrame(
      emptyInputs({
        approvals: [approval({ id: "danger", risk: "red" }), approval({ id: "mild", risk: "yellow" })],
        threads: [thread({ id: "t1", updatedAt: 9_000 })],
        workflows: [workflow({ id: "w1", status: "running" })],
        interruptibilityBlocked: true,
      }),
    );
    expect(frame.voiceLine?.subjectId).toBe("approval:danger");
    // Every chip + orbit point belongs to the RED approval only.
    expect(frame.chips.every((c) => c.id === "approval:danger")).toBe(true);
    expect(frame.orbit.every((p) => p.capability === "approval")).toBe(true);
  });
});

// ─── Never audio in a blocked context (Req 26.3 / §26.5) ─────────────────────

/** Every key the engine's FocusFrame may carry. No audio/voice/sound directive exists. */
const FOCUS_FRAME_KEYS = new Set([
  "greeting",
  "voiceLine",
  "acs",
  "chips",
  "orbit",
  "coreHint",
  "blockedContext",
]);

describe("deriveFocusFrame — never requests audio in a blocked context (Req 26.3)", () => {
  it("emits no audio-requesting field and no speaking coreHint", () => {
    const frame = deriveFocusFrame(
      emptyInputs({ approvals: [approval({ id: "danger", risk: "red" })], interruptibilityBlocked: true }),
    );
    // The engine has NO audio output: the frame carries only its known keys.
    for (const key of Object.keys(frame)) expect(FOCUS_FRAME_KEYS.has(key)).toBe(true);
    // The advisory hint is calm, never a speaking/voice state.
    expect(frame.coreHint).not.toBe("speaking");
  });
});

// ─── Interruptible (non-blocked) context is unchanged (Req 26.1) ─────────────

describe("deriveFocusFrame — interruptible context posture unchanged (Req 26.1)", () => {
  it("surfaces non-RED subjects normally and does not flag the frame", () => {
    const inputs = emptyInputs({ threads: [thread({ id: "t1", updatedAt: 9_000 })] });
    const frame = deriveFocusFrame(inputs);
    expect(frame.blockedContext).toBeUndefined();
    expect(frame.voiceLine?.subjectId).toBe("thread:t1");
  });

  it("a RED approval in an interruptible context still earns its full step-forward", () => {
    const frame = deriveFocusFrame(emptyInputs({ approvals: [approval({ id: "d", risk: "red" })] }));
    expect(frame.blockedContext).toBeUndefined();
    expect(frame.voiceLine?.subjectId).toBe("approval:d");
    expect(frame.acs).toBeDefined();
    expect(frame.coreHint).toBe("blocked");
  });
});

// ─── At-most-one gentle re-surface then age-out (Req 26.4) ───────────────────

const blockedApprovalFrame = (subjectId = "approval:r1"): FocusFrame => ({
  blockedContext: true,
  voiceLine: {
    subjectId,
    text: "Confirm destructive action",
    key: subjectId,
    actionable: true,
    priority: FOCUS_PRIORITY.needsYou,
    confidence: 1,
    emphasis: "high",
  },
  acs: { subjectId, title: "Confirm", line: "danger", ownerRoute: { space: "converse" } },
  coreHint: "blocked",
  chips: [],
  orbit: [],
});
const blockedRestFrame = (): FocusFrame => ({ blockedContext: true, chips: [], orbit: [] });
const openFrame = (): FocusFrame => ({ chips: [], orbit: [] });

describe("createInterruptibilityGate — re-surface then age-out (Req 26.4)", () => {
  it("shows the initial surface and one gentle re-surface, then ages out", () => {
    const gate = createInterruptibilityGate();
    // Appearance 1 (initial) — shown.
    expect(gate.gate(blockedApprovalFrame()).voiceLine?.subjectId).toBe("approval:r1");
    // Disappears (e.g. context churn), then re-surfaces — appearance 2, shown.
    expect(gate.gate(blockedRestFrame()).voiceLine).toBeUndefined();
    expect(gate.gate(blockedApprovalFrame()).voiceLine?.subjectId).toBe("approval:r1");
    // Disappears again, then a THIRD appearance is aged out (suppressed).
    gate.gate(blockedRestFrame());
    const aged = gate.gate(blockedApprovalFrame());
    expect(aged.voiceLine).toBeUndefined();
    expect(aged.acs).toBeUndefined();
    expect(aged.coreHint).toBeUndefined();
    expect(aged.blockedContext).toBe(true); // still blocked, just silent now
  });

  it("a continuously-shown subject does not consume its re-surface budget", () => {
    const gate = createInterruptibilityGate();
    // Staying shown across many frames is ONE appearance (no rising edges).
    for (let i = 0; i < 10; i += 1) {
      expect(gate.gate(blockedApprovalFrame()).voiceLine?.subjectId).toBe("approval:r1");
    }
    // It may still re-surface once after actually disappearing.
    gate.gate(blockedRestFrame());
    expect(gate.gate(blockedApprovalFrame()).voiceLine?.subjectId).toBe("approval:r1");
  });

  it("passes an interruptible frame through untouched and resets tracking", () => {
    const gate = createInterruptibilityGate();
    // Burn the whole budget while blocked → aged out.
    gate.gate(blockedApprovalFrame());
    gate.gate(blockedRestFrame());
    gate.gate(blockedApprovalFrame());
    gate.gate(blockedRestFrame());
    expect(gate.gate(blockedApprovalFrame()).voiceLine).toBeUndefined(); // aged out
    // Context clears → passthrough + reset.
    const open = openFrame();
    expect(gate.gate(open)).toBe(open);
    // A NEW blocked context starts the count over (fresh initial surface).
    expect(gate.gate(blockedApprovalFrame()).voiceLine?.subjectId).toBe("approval:r1");
  });

  it("tracks each blocked subject independently", () => {
    const gate = createInterruptibilityGate();
    expect(gate.gate(blockedApprovalFrame("approval:a")).voiceLine?.subjectId).toBe("approval:a");
    // A different subject taking over is its own first appearance.
    expect(gate.gate(blockedApprovalFrame("approval:b")).voiceLine?.subjectId).toBe("approval:b");
  });
});

// ─── Property: in a blocked context only RED approvals ever surface (Req 26.2)
// Validates: Requirements 26.1, 26.2

describe("Property: a blocked context surfaces only RED approvals", () => {
  it("every surfacing subject is a RED (red/black) approval; all else deferred", () => {
    fc.assert(
      fc.property(arbInputs, (inputs) => {
        const blocked = deriveFocusFrame({ ...inputs, interruptibilityBlocked: true });
        // An id is RED-capable iff some approval with that id is red/black.
        const redIds = new Set(
          inputs.approvals.filter((a) => a.risk === "red" || a.risk === "black").map((a) => a.id),
        );
        expect(blocked.blockedContext).toBe(true);
        if (blocked.voiceLine) {
          const id = blocked.voiceLine.subjectId.replace(/^approval:/, "");
          expect(blocked.voiceLine.subjectId.startsWith("approval:")).toBe(true);
          expect(redIds.has(id)).toBe(true);
        }
        // No chip / orbit point may belong to a non-RED-approval subject.
        for (const chip of blocked.chips) {
          expect(chip.id.startsWith("approval:")).toBe(true);
          expect(redIds.has(chip.id.replace(/^approval:/, ""))).toBe(true);
        }
        expect(blocked.orbit.every((p) => p.capability === "approval")).toBe(true);
        // Structural guardrails always hold.
        expect(() => assertFocusFrame(blocked)).not.toThrow();
      }),
      { numRuns: 500 },
    );
  });
});

// ─── Property: the blocked frame never carries an audio directive (Req 26.3) ─
// Validates: Requirements 26.3

describe("Property: a blocked frame never requests audio", () => {
  it("carries only known FocusFrame keys and never a speaking coreHint", () => {
    fc.assert(
      fc.property(arbInputs, (inputs) => {
        const blocked = deriveFocusFrame({ ...inputs, interruptibilityBlocked: true });
        for (const key of Object.keys(blocked)) expect(FOCUS_FRAME_KEYS.has(key)).toBe(true);
        expect(blocked.coreHint).not.toBe("speaking");
      }),
      { numRuns: 300 },
    );
  });
});

// ─── Property: at-most-one re-surface then age-out (Req 26.4) ────────────────
// Validates: Requirements 26.4

describe("Property: a blocked RED subject surfaces in at most MAX_BLOCKED_SURFACES episodes", () => {
  it("counts rising-edge appearances and never exceeds the budget while blocked", () => {
    fc.assert(
      fc.property(
        // A run of blocked frames: true = the RED subject is present, false = gone.
        fc.array(fc.boolean(), { minLength: 1, maxLength: 40 }),
        (present) => {
          const gate = createInterruptibilityGate();
          let episodes = 0;
          let prevShown = false;
          for (const isPresent of present) {
            const out = gate.gate(isPresent ? blockedApprovalFrame() : blockedRestFrame());
            const shown = out.voiceLine?.subjectId === "approval:r1";
            if (shown && !prevShown) episodes += 1; // a fresh shown-episode
            prevShown = shown;
          }
          expect(episodes).toBeLessThanOrEqual(MAX_BLOCKED_SURFACES);
        },
      ),
      { numRuns: 500 },
    );
  });
});

// ─── Property: non-blocked context is unchanged (Req 26.1) ───────────────────
// Validates: Requirements 26.1

describe("Property: blocking only ever removes subjects (never invents one)", () => {
  it("the open frame is never flagged; every blocked chip was also an open chip", () => {
    fc.assert(
      fc.property(arbInputs, (inputs) => {
        const open = deriveFocusFrame(inputs);
        const blocked = deriveFocusFrame({ ...inputs, interruptibilityBlocked: true });
        // The interruptible frame is never flagged blocked (posture unchanged).
        expect(open.blockedContext).toBeUndefined();
        // Blocking only DEFERS subjects: any chip shown while blocked (only RED
        // approvals produce chips here, and they outrank every other chip) was
        // also present while open — the blocked chip set is a subset. (The
        // headline SUBJECT may differ: suppressing a higher-priority awareness
        // signal can promote a lower RED approval to the calm blocked headline.)
        const openChipIds = new Set(open.chips.map((c) => c.id));
        for (const chip of blocked.chips) expect(openChipIds.has(chip.id)).toBe(true);
      }),
      { numRuns: 500 },
    );
  });
});
