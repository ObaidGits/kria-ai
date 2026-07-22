import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { coreStore } from "./coreStore";
import { eventBus } from "./eventBus";

describe("coreStore", () => {
  beforeEach(() => {
    coreStore.reset();
  });

  afterEach(() => {
    eventBus.clear();
  });

  describe("initial state", () => {
    it("starts idle", () => {
      expect(coreStore.state()).toBe("idle");
      expect(coreStore.previousState()).toBe("idle");
      expect(coreStore.isIdle()).toBe(true);
      expect(coreStore.isActive()).toBe(false);
      expect(coreStore.needsAttention()).toBe(false);
    });

    it("has no error or block reason", () => {
      expect(coreStore.errorMessage()).toBeNull();
      expect(coreStore.blockReason()).toBeNull();
    });
  });

  describe("state transitions", () => {
    it("transitions to a new state", () => {
      coreStore.setState("thinking");

      expect(coreStore.state()).toBe("thinking");
      expect(coreStore.previousState()).toBe("idle");
      expect(coreStore.isActive()).toBe(true);
      expect(coreStore.isIdle()).toBe(false);
    });

    it("emits core:state-changed on transition", () => {
      const handler = vi.fn();
      eventBus.on("core:state-changed", handler, "none");

      coreStore.setState("listening");

      expect(handler).toHaveBeenCalledWith({ state: "listening", previous: "idle" });
    });

    it("does not emit when setting the same state", () => {
      const handler = vi.fn();
      eventBus.on("core:state-changed", handler, "none");

      coreStore.setState("idle");
      expect(handler).not.toHaveBeenCalled();
    });

    it("tracks previous state through multiple transitions", () => {
      coreStore.setState("thinking");
      coreStore.setState("planning");
      coreStore.setState("acting");

      expect(coreStore.state()).toBe("acting");
      expect(coreStore.previousState()).toBe("planning");
    });

    it("updates stateTimestamp on each transition", () => {
      const t1 = coreStore.stateTimestamp();
      coreStore.setState("thinking");
      const t2 = coreStore.stateTimestamp();
      expect(t2).toBeGreaterThanOrEqual(t1);
    });
  });

  describe("derived signals", () => {
    it("isActive is true for active states", () => {
      const activeStates = [
        "listening", "thinking", "planning", "speaking", "acting",
        "running-automation", "watching", "remembering", "reflecting", "learning",
      ] as const;

      for (const s of activeStates) {
        coreStore.setState(s);
        expect(coreStore.isActive()).toBe(true);
      }
    });

    it("needsAttention is true for attention states", () => {
      coreStore.setState("blocked");
      expect(coreStore.needsAttention()).toBe(true);

      coreStore.setState("error");
      expect(coreStore.needsAttention()).toBe(true);

      coreStore.setState("waiting");
      expect(coreStore.needsAttention()).toBe(true);
    });

    it("needsAttention is false for active/idle states", () => {
      coreStore.setState("thinking");
      expect(coreStore.needsAttention()).toBe(false);

      coreStore.goIdle();
      expect(coreStore.needsAttention()).toBe(false);
    });
  });

  describe("convenience actions", () => {
    it("goIdle transitions to idle", () => {
      coreStore.setState("thinking");
      coreStore.goIdle();
      expect(coreStore.state()).toBe("idle");
    });

    it("setBlocked sets state and reason", () => {
      coreStore.setBlocked("needs approval");
      expect(coreStore.state()).toBe("blocked");
      expect(coreStore.blockReason()).toBe("needs approval");
    });

    it("setError sets state and message", () => {
      coreStore.setError("connection lost");
      expect(coreStore.state()).toBe("error");
      expect(coreStore.errorMessage()).toBe("connection lost");
    });

    it("clears error when transitioning to a non-error/recovery state", () => {
      coreStore.setError("oops");
      expect(coreStore.errorMessage()).toBe("oops");

      // Recovering still has access to the error context (for display)
      coreStore.setState("recovering");
      // But error clears because recovering is not error state
      // Actually: the store clears errorMessage for anything except error/recovering
      // recovering is NOT error and NOT recovering... wait, it IS recovering.
      // Let's verify actual behavior: the code says
      //   if (next !== "error" && next !== "recovering") setErrorMessage(null);
      // So "recovering" does NOT clear the error. That's correct behavior.
      expect(coreStore.errorMessage()).toBe("oops");

      // Transitioning to idle clears it
      coreStore.goIdle();
      expect(coreStore.errorMessage()).toBeNull();
    });

    it("clears blockReason when transitioning away from blocked", () => {
      coreStore.setBlocked("needs input");
      expect(coreStore.blockReason()).toBe("needs input");

      coreStore.goIdle();
      expect(coreStore.blockReason()).toBeNull();
    });
  });

  describe("reset", () => {
    it("resets all state to initial values", () => {
      coreStore.setState("thinking");
      coreStore.setError("something broke");
      coreStore.reset();

      expect(coreStore.state()).toBe("idle");
      expect(coreStore.previousState()).toBe("idle");
      expect(coreStore.errorMessage()).toBeNull();
      expect(coreStore.blockReason()).toBeNull();
    });
  });
});

// ─── Event-fed state machine (task 2.1) ─────────────────────────────────────────

import {
  mapDomainEvent,
  STATE_PRIORITY,
  type CoreDomainEvent,
  type CoreState,
} from "./coreStore";

describe("coreStore — domain-event mapping (mapDomainEvent)", () => {
  it("maps voice states to the correct Core state", () => {
    expect(mapDomainEvent({ kind: "voice", state: "listening" })).toMatchObject({ op: "begin", state: "listening" });
    expect(mapDomainEvent({ kind: "voice", state: "wake_listening" })).toMatchObject({ op: "begin", state: "listening" });
    expect(mapDomainEvent({ kind: "voice", state: "transcribing" })).toMatchObject({ op: "begin", state: "thinking" });
    expect(mapDomainEvent({ kind: "voice", state: "thinking" })).toMatchObject({ op: "begin", state: "thinking" });
    expect(mapDomainEvent({ kind: "voice", state: "speaking" })).toMatchObject({ op: "begin", state: "speaking" });
    expect(mapDomainEvent({ kind: "voice", state: "idle" })).toMatchObject({ op: "end", source: "voice" });
    expect(mapDomainEvent({ kind: "voice", state: "error" })).toMatchObject({ op: "end", source: "voice" });
  });

  it("maps agent phases to thinking/planning/speaking/acting", () => {
    expect(mapDomainEvent({ kind: "agent", phase: "thinking" })).toMatchObject({ op: "begin", state: "thinking" });
    expect(mapDomainEvent({ kind: "agent", phase: "planning" })).toMatchObject({ op: "begin", state: "planning" });
    expect(mapDomainEvent({ kind: "agent", phase: "streaming" })).toMatchObject({ op: "begin", state: "responding" });
    expect(mapDomainEvent({ kind: "agent", phase: "done", sessionId: "s1" })).toMatchObject({ op: "end", source: "agent:s1" });
    expect(mapDomainEvent({ kind: "agent", phase: "error", message: "x" })).toMatchObject({ op: "begin", source: "error", state: "error" });
  });

  it("maps tool calls to acting", () => {
    expect(mapDomainEvent({ kind: "tool", phase: "start", callId: "c1" })).toMatchObject({ op: "begin", source: "tool:c1", state: "acting" });
    expect(mapDomainEvent({ kind: "tool", phase: "done", callId: "c1" })).toMatchObject({ op: "end", source: "tool:c1" });
  });

  it("maps gui-cognition to watching/acting", () => {
    expect(mapDomainEvent({ kind: "gui-cognition", phase: "watching" })).toMatchObject({ op: "begin", state: "watching" });
    expect(mapDomainEvent({ kind: "gui-cognition", phase: "acting" })).toMatchObject({ op: "begin", state: "acting" });
    expect(mapDomainEvent({ kind: "gui-cognition", phase: "done" })).toMatchObject({ op: "end" });
  });

  it("maps workflow runs to running-automation", () => {
    expect(mapDomainEvent({ kind: "workflow", phase: "start", workflowId: "w1" })).toMatchObject({ op: "begin", source: "workflow:w1", state: "running-automation" });
    expect(mapDomainEvent({ kind: "workflow", phase: "progress", workflowId: "w1" })).toMatchObject({ op: "begin", state: "running-automation" });
    expect(mapDomainEvent({ kind: "workflow", phase: "done", workflowId: "w1" })).toMatchObject({ op: "end", source: "workflow:w1" });
    expect(mapDomainEvent({ kind: "workflow", phase: "failed", workflowId: "w1" })).toMatchObject({ op: "end", source: "workflow:w1" });
  });

  it("maps cognition jobs to reflecting/remembering/learning", () => {
    expect(mapDomainEvent({ kind: "cognition", job: "reflect", phase: "start" })).toMatchObject({ op: "begin", state: "reflecting" });
    expect(mapDomainEvent({ kind: "cognition", job: "dream", phase: "start" })).toMatchObject({ op: "begin", state: "reflecting" });
    expect(mapDomainEvent({ kind: "cognition", job: "consolidate", phase: "start" })).toMatchObject({ op: "begin", state: "remembering" });
    expect(mapDomainEvent({ kind: "cognition", job: "entity-extraction", phase: "start" })).toMatchObject({ op: "begin", state: "remembering" });
    expect(mapDomainEvent({ kind: "cognition", job: "active-learning", phase: "start" })).toMatchObject({ op: "begin", state: "learning" });
    expect(mapDomainEvent({ kind: "cognition", job: "self-improvement", phase: "start" })).toMatchObject({ op: "begin", state: "learning" });
  });

  it("maps memory writes to a remembering pulse", () => {
    expect(mapDomainEvent({ kind: "memory", op: "updated" })).toMatchObject({ op: "pulse", source: "memory", state: "remembering" });
  });

  it("maps approvals to blocked and their resolution to end", () => {
    expect(mapDomainEvent({ kind: "approval", phase: "request", id: "a1" })).toMatchObject({ op: "begin", source: "approval:a1", state: "blocked" });
    expect(mapDomainEvent({ kind: "approval", phase: "resolved", id: "a1" })).toMatchObject({ op: "end", source: "approval:a1" });
  });

  it("maps waiting and error phases", () => {
    expect(mapDomainEvent({ kind: "waiting", phase: "start" })).toMatchObject({ op: "begin", state: "waiting" });
    expect(mapDomainEvent({ kind: "error", phase: "raised" })).toMatchObject({ op: "begin", state: "error" });
    expect(mapDomainEvent({ kind: "error", phase: "recovering" })).toMatchObject({ op: "begin", state: "recovering" });
    expect(mapDomainEvent({ kind: "error", phase: "cleared" })).toMatchObject({ op: "end", source: "error" });
  });
});

describe("coreStore — ingest & precedence", () => {
  beforeEach(() => coreStore.reset());
  afterEach(() => eventBus.clear());

  it("reflects a single domain activity", () => {
    coreStore.ingest({ kind: "agent", phase: "thinking", sessionId: "s1" });
    expect(coreStore.state()).toBe("thinking");
    expect(coreStore.isActive()).toBe(true);
  });

  it("returns to idle when the only activity ends", () => {
    coreStore.ingest({ kind: "workflow", phase: "start", workflowId: "w1" });
    expect(coreStore.state()).toBe("running-automation");
    coreStore.ingest({ kind: "workflow", phase: "done", workflowId: "w1" });
    expect(coreStore.state()).toBe("idle");
    expect(coreStore.isIdle()).toBe(true);
  });

  it("blocked (approval) wins over concurrent work and calms back down when resolved", () => {
    coreStore.ingest({ kind: "agent", phase: "thinking", sessionId: "s1" });
    coreStore.ingest({ kind: "tool", phase: "start", callId: "c1" });
    expect(coreStore.state()).toBe("acting"); // acting > thinking

    coreStore.ingest({ kind: "approval", phase: "request", id: "a1" });
    expect(coreStore.state()).toBe("blocked"); // blocked wins
    expect(coreStore.needsAttention()).toBe(true);

    // Resolving the approval falls back to the still-running work, not idle.
    coreStore.ingest({ kind: "approval", phase: "resolved", id: "a1" });
    expect(coreStore.state()).toBe("acting");
  });

  it("error takes precedence over blocked", () => {
    coreStore.ingest({ kind: "approval", phase: "request", id: "a1" });
    expect(coreStore.state()).toBe("blocked");
    coreStore.ingest({ kind: "error", phase: "raised", message: "boom" });
    expect(coreStore.state()).toBe("error");
    expect(coreStore.errorMessage()).toBe("boom");

    // Clearing the error reveals the still-pending approval.
    coreStore.ingest({ kind: "error", phase: "cleared" });
    expect(coreStore.state()).toBe("blocked");
  });

  it("keeps blocked while ANY approval is still pending", () => {
    coreStore.ingest({ kind: "approval", phase: "request", id: "a1" });
    coreStore.ingest({ kind: "approval", phase: "request", id: "a2" });
    expect(coreStore.state()).toBe("blocked");
    coreStore.ingest({ kind: "approval", phase: "resolved", id: "a1" });
    expect(coreStore.state()).toBe("blocked"); // a2 still pending
    coreStore.ingest({ kind: "approval", phase: "resolved", id: "a2" });
    expect(coreStore.state()).toBe("idle");
  });

  it("reset clears the activity set", () => {
    coreStore.ingest({ kind: "workflow", phase: "start", workflowId: "w1" });
    coreStore.ingest({ kind: "approval", phase: "request", id: "a1" });
    coreStore.reset();
    expect(coreStore.state()).toBe("idle");
    // A subsequent unrelated end must not resurrect prior state.
    coreStore.ingest({ kind: "workflow", phase: "done", workflowId: "w1" });
    expect(coreStore.state()).toBe("idle");
  });

  it("memory pulse shows remembering then auto-returns to idle", () => {
    vi.useFakeTimers();
    try {
      coreStore.ingest({ kind: "memory", op: "updated" });
      expect(coreStore.state()).toBe("remembering");
      vi.advanceTimersByTime(2000);
      expect(coreStore.state()).toBe("idle");
    } finally {
      vi.useRealTimers();
    }
  });

  it("all 14+ Core states are reachable via domain events", () => {
    const reach = (evts: CoreDomainEvent[], expected: CoreState) => {
      coreStore.reset();
      for (const e of evts) coreStore.ingest(e);
      expect(coreStore.state()).toBe(expected);
    };

    reach([], "idle");
    reach([{ kind: "voice", state: "listening" }], "listening");
    reach([{ kind: "agent", phase: "thinking" }], "thinking");
    reach([{ kind: "agent", phase: "planning" }], "planning");
    reach([{ kind: "voice", state: "speaking" }], "speaking");
    reach([{ kind: "tool", phase: "start", callId: "c1" }], "acting");
    reach([{ kind: "workflow", phase: "start", workflowId: "w1" }], "running-automation");
    reach([{ kind: "gui-cognition", phase: "watching" }], "watching");
    reach([{ kind: "cognition", job: "consolidate", phase: "start" }], "remembering");
    reach([{ kind: "cognition", job: "reflect", phase: "start" }], "reflecting");
    reach([{ kind: "cognition", job: "active-learning", phase: "start" }], "learning");
    reach([{ kind: "waiting", phase: "start" }], "waiting");
    reach([{ kind: "approval", phase: "request", id: "a1" }], "blocked");
    reach([{ kind: "error", phase: "raised" }], "error");
    reach([{ kind: "error", phase: "recovering" }], "recovering");
  });

  it("precedence table orders attention states above work above idle", () => {
    expect(STATE_PRIORITY.error).toBeGreaterThan(STATE_PRIORITY.blocked);
    expect(STATE_PRIORITY.blocked).toBeGreaterThan(STATE_PRIORITY.acting);
    expect(STATE_PRIORITY.acting).toBeGreaterThan(STATE_PRIORITY.thinking);
    expect(STATE_PRIORITY.thinking).toBeGreaterThan(STATE_PRIORITY.idle);
  });

  it("derived accessors are correct across representative states", () => {
    coreStore.reset();
    coreStore.ingest({ kind: "agent", phase: "thinking" });
    expect(coreStore.isActive()).toBe(true);
    expect(coreStore.needsAttention()).toBe(false);

    coreStore.ingest({ kind: "waiting", phase: "start" });
    expect(coreStore.needsAttention()).toBe(true); // waiting > thinking
  });
});

describe("coreStore — event-bus wiring (initCoreStateMachine)", () => {
  beforeEach(() => coreStore.reset());
  afterEach(() => {
    coreStore.disposeCoreStateMachine();
    eventBus.clear();
  });

  it("drives Core state from bus events and is idempotent", () => {
    coreStore.initCoreStateMachine();
    coreStore.initCoreStateMachine(); // second call must not double-subscribe

    eventBus.emit("voice:state-changed", { state: "listening", previous: "idle" });
    expect(coreStore.state()).toBe("listening");

    eventBus.emit("approval:request", { id: "a1", type: "tool-hitl", payload: {} });
    expect(coreStore.state()).toBe("blocked");

    eventBus.emit("approval:resolved", { id: "a1", action: "approve" });
    expect(coreStore.state()).toBe("listening"); // falls back to still-active voice

    eventBus.emit("voice:state-changed", { state: "idle", previous: "listening" });
    expect(coreStore.state()).toBe("idle");
  });

  it("does not subscribe to core:state-changed (no feedback loop)", () => {
    coreStore.initCoreStateMachine();
    // coreStore emits core:state-changed; a self-subscription would recurse.
    // Reaching a state and staying stable proves no loop.
    eventBus.emit("automation:workflow-started", { workflowId: "w1" });
    expect(coreStore.state()).toBe("running-automation");
    eventBus.emit("automation:workflow-completed", { workflowId: "w1", success: true });
    expect(coreStore.state()).toBe("idle");
  });

  it("dispose detaches subscriptions", () => {
    coreStore.initCoreStateMachine();
    coreStore.disposeCoreStateMachine();
    eventBus.emit("voice:state-changed", { state: "listening", previous: "idle" });
    expect(coreStore.state()).toBe("idle"); // no longer wired
  });
});
