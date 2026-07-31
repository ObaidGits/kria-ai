/**
 * detachRestore.test.ts
 *
 * Session lifecycle tests for MemoryWindowSessionV2 (task 5.2.6).
 *
 * Covers six lifecycle properties:
 *   DR1  Detach/Restore         — markDetached() → detached; validateDetachedRestore()
 *                                 only true when detached; reset() after detach → idle
 *   DR2  Close cancellation     — markDetached() aborts the active AbortController;
 *                                 any in-flight request's signal.aborted === true
 *   DR3  Subscription cleanup   — After reset(), generation=0, revision=0, state=idle;
 *                                 no state leaks across the session boundary
 *   DR4  Focus return           — failRequest() is a no-op when generation doesn't
 *                                 match (stale response is silently discarded)
 *   DR5  Heap recovery          — After 20 request cycles (beginRequest +
 *                                 completeRequest) the session state is consistent
 *                                 and bounded (no accumulation)
 *   DR6  No orphan              — Multiple beginRequest calls in sequence: only the
 *                                 last signal is alive; all previous signals are aborted
 *                                 (no orphan listeners / controllers)
 *
 * Requirements: MGR-007, MGR-008, MGR-020; F4.1.
 */

import { describe, it, expect, beforeEach } from "vitest";
import {
  MemoryWindowSessionV2,
  type WindowSessionConfig,
  type WindowSessionState,
} from "./windowSession";

// ─── Helpers ──────────────────────────────────────────────────────────────────

function makeConfig(overrides: Partial<WindowSessionConfig> = {}): WindowSessionConfig {
  return {
    instanceId: "win-dr-test",
    policyHash: "policy-dr",
    schemaVersion: "2.0",
    ...overrides,
  };
}

function makeSession(overrides: Partial<WindowSessionConfig> = {}): MemoryWindowSessionV2 {
  return new MemoryWindowSessionV2(makeConfig(overrides));
}

// ─── DR1: Detach / Restore ────────────────────────────────────────────────────

describe("DR1 — Detach/Restore: markDetached() transitions to detached; validateDetachedRestore() guard; reset() returns to idle", () => {
  let session: MemoryWindowSessionV2;

  beforeEach(() => {
    session = makeSession();
  });

  it("markDetached() from idle transitions state to 'detached'", () => {
    session.markDetached();
    expect(session.state).toBe<WindowSessionState>("detached");
  });

  it("markDetached() from loading transitions state to 'detached'", () => {
    session.beginRequest("query");
    session.markDetached();
    expect(session.state).toBe<WindowSessionState>("detached");
  });

  it("markDetached() from ready transitions state to 'detached'", () => {
    const { generation } = session.beginRequest("query");
    session.completeRequest(generation, 5);
    expect(session.state).toBe("ready");
    session.markDetached();
    expect(session.state).toBe<WindowSessionState>("detached");
  });

  it("markDetached() from error transitions state to 'detached'", () => {
    const { generation } = session.beginRequest("query");
    session.failRequest(generation);
    expect(session.state).toBe("error");
    session.markDetached();
    expect(session.state).toBe<WindowSessionState>("detached");
  });

  it("markDetached() from stale transitions state to 'detached'", () => {
    session.markStale();
    session.markDetached();
    expect(session.state).toBe<WindowSessionState>("detached");
  });

  it("validateDetachedRestore() returns true only when state is 'detached'", () => {
    session.markDetached();
    expect(session.validateDetachedRestore()).toBe(true);
  });

  it("validateDetachedRestore() returns false when state is 'idle'", () => {
    expect(session.state).toBe("idle");
    expect(session.validateDetachedRestore()).toBe(false);
  });

  it("validateDetachedRestore() returns false when state is 'loading'", () => {
    session.beginRequest("q");
    expect(session.validateDetachedRestore()).toBe(false);
  });

  it("validateDetachedRestore() returns false when state is 'ready'", () => {
    const { generation } = session.beginRequest("q");
    session.completeRequest(generation, 1);
    expect(session.validateDetachedRestore()).toBe(false);
  });

  it("reset() after markDetached() returns state to 'idle'", () => {
    session.markDetached();
    expect(session.validateDetachedRestore()).toBe(true);
    session.reset();
    expect(session.state).toBe<WindowSessionState>("idle");
    expect(session.validateDetachedRestore()).toBe(false);
  });

  it("reset() after markDetached() resets generation to 0", () => {
    session.beginRequest("q1");
    session.beginRequest("q2");
    session.markDetached();
    session.reset();
    expect(session.generation).toBe(0);
  });

  it("reset() after markDetached() resets revision to 0", () => {
    const { generation } = session.beginRequest("q");
    session.completeRequest(generation, 42);
    session.markDetached();
    session.reset();
    expect(session.revision).toBe(0);
  });

  it("calling markDetached() twice does not throw and keeps state 'detached'", () => {
    expect(() => {
      session.markDetached();
      session.markDetached();
    }).not.toThrow();
    expect(session.state).toBe<WindowSessionState>("detached");
  });
});

// ─── DR2: Close cancellation ──────────────────────────────────────────────────

describe("DR2 — Close cancellation: markDetached() aborts the active AbortController signal", () => {
  let session: MemoryWindowSessionV2;

  beforeEach(() => {
    session = makeSession();
  });

  it("signal is aborted immediately after markDetached() when a request is in flight", () => {
    const { signal } = session.beginRequest("live query");
    expect(signal.aborted).toBe(false);
    session.markDetached();
    expect(signal.aborted).toBe(true);
  });

  it("markDetached() aborts the most recent in-flight signal when multiple were issued", () => {
    session.beginRequest("query-1"); // gen 1 — aborted by gen 2
    const { signal: sig2 } = session.beginRequest("query-2"); // gen 2 — current
    expect(sig2.aborted).toBe(false);
    session.markDetached();
    expect(sig2.aborted).toBe(true);
  });

  it("signal from prior (cancelled) beginRequest remains aborted after markDetached()", () => {
    const { signal: sig1 } = session.beginRequest("first");
    // second beginRequest cancels first
    session.beginRequest("second");
    expect(sig1.aborted).toBe(true);
    // markDetached does not change the already-aborted signal (still aborted)
    session.markDetached();
    expect(sig1.aborted).toBe(true);
  });

  it("markDetached() on idle session (no controller) does not throw", () => {
    expect(session.state).toBe("idle");
    expect(() => session.markDetached()).not.toThrow();
    expect(session.state).toBe("detached");
  });

  it("reset() on a loading session also aborts the in-flight signal", () => {
    const { signal } = session.beginRequest("q");
    expect(signal.aborted).toBe(false);
    session.reset();
    expect(signal.aborted).toBe(true);
  });
});

// ─── DR3: Subscription cleanup ────────────────────────────────────────────────

describe("DR3 — Subscription cleanup: After reset(), generation=0, revision=0, state=idle; no state leaks", () => {
  let session: MemoryWindowSessionV2;

  beforeEach(() => {
    session = makeSession();
  });

  it("reset() resets generation to exactly 0 regardless of how many requests were made", () => {
    for (let i = 0; i < 10; i++) {
      session.beginRequest(`q${i}`);
    }
    expect(session.generation).toBe(10);
    session.reset();
    expect(session.generation).toBe(0);
  });

  it("reset() resets revision to 0 after a completed request had set a non-zero revision", () => {
    const { generation } = session.beginRequest("q");
    session.completeRequest(generation, 77);
    expect(session.revision).toBe(77);
    session.reset();
    expect(session.revision).toBe(0);
  });

  it("reset() transitions state to 'idle' from any source state", () => {
    // From 'loading'
    session.beginRequest("q");
    session.reset();
    expect(session.state).toBe("idle");
  });

  it("reset() transitions state to 'idle' from 'error'", () => {
    const { generation } = session.beginRequest("q");
    session.failRequest(generation);
    session.reset();
    expect(session.state).toBe("idle");
  });

  it("reset() transitions state to 'idle' from 'stale'", () => {
    session.markStale();
    session.reset();
    expect(session.state).toBe("idle");
  });

  it("new request cycle after reset() starts fresh at generation 1", () => {
    const { generation: gen1 } = session.beginRequest("q1");
    session.completeRequest(gen1, 100);
    session.reset();

    const { generation: gen2 } = session.beginRequest("q2");
    expect(gen2).toBe(1); // fresh generation
  });

  it("completeRequest with stale generation after reset() returns false and state stays idle", () => {
    const { generation: oldGen } = session.beginRequest("q1");
    session.reset(); // old gen is stale; generation now 0 (no active request)
    // Attempting to complete with the old token must be rejected
    const result = session.completeRequest(oldGen, 99);
    expect(result).toBe(false);
    expect(session.state).toBe("idle");
    expect(session.revision).toBe(0);
  });

  it("revision guard correctly rejects old revision after reset() + new request cycle", () => {
    const { generation: gen1 } = session.beginRequest("q1");
    session.completeRequest(gen1, 50);
    session.reset();

    // After reset, revision is 0
    expect(session.guardRevision(0)).toBe(true);
    expect(session.guardRevision(50)).toBe(false); // old revision no longer valid
  });

  it("multiple reset() calls in a row keep session stable at generation=0, revision=0, state=idle", () => {
    session.beginRequest("q");
    session.reset();
    session.reset();
    session.reset();
    expect(session.generation).toBe(0);
    expect(session.revision).toBe(0);
    expect(session.state).toBe("idle");
  });
});

// ─── DR4: Focus return ────────────────────────────────────────────────────────

describe("DR4 — Focus return: failRequest() is a no-op when generation doesn't match (stale response ignored)", () => {
  let session: MemoryWindowSessionV2;

  beforeEach(() => {
    session = makeSession();
  });

  it("failRequest() with a stale generation returns false and does not change state", () => {
    const { generation: gen1 } = session.beginRequest("q1");
    // Supersede by a second request
    session.beginRequest("q2");
    expect(session.state).toBe("loading");

    const result = session.failRequest(gen1); // stale
    expect(result).toBe(false);
    expect(session.state).toBe<WindowSessionState>("loading"); // unchanged
  });

  it("failRequest() with a completely outdated generation (gen=0) is a no-op", () => {
    session.beginRequest("q1"); // gen 1
    const result = session.failRequest(0); // generation 0 was never active after beginRequest
    expect(result).toBe(false);
    expect(session.state).toBe("loading");
  });

  it("failRequest() with the correct current generation succeeds and sets state to 'error'", () => {
    const { generation } = session.beginRequest("q");
    const result = session.failRequest(generation);
    expect(result).toBe(true);
    expect(session.state).toBe<WindowSessionState>("error");
  });

  it("failRequest() on a detached session with mismatched generation is a no-op", () => {
    const { generation: gen1 } = session.beginRequest("q");
    session.beginRequest("q2"); // gen2 is now active
    session.markDetached();

    // Stale gen1 fail attempt must be rejected
    const result = session.failRequest(gen1);
    expect(result).toBe(false);
    // State remains detached (not changed to error)
    expect(session.state).toBe<WindowSessionState>("detached");
  });

  it("completeRequest() is also a no-op (returns false) for a stale generation", () => {
    const { generation: gen1 } = session.beginRequest("q1");
    session.beginRequest("q2"); // gen1 is now stale

    const result = session.completeRequest(gen1, 33);
    expect(result).toBe(false);
    expect(session.revision).toBe(0); // revision not updated
    expect(session.state).toBe<WindowSessionState>("loading");
  });

  it("failRequest() does not affect revision even when generation matches (error state has no revision)", () => {
    const { generation } = session.beginRequest("q");
    // First complete to set a revision
    session.completeRequest(generation, 10);

    // Then begin a new request and fail it
    const { generation: gen2 } = session.beginRequest("q2");
    session.failRequest(gen2);

    // revision should still be 10 — not zeroed by failRequest
    expect(session.revision).toBe(10);
    expect(session.state).toBe("error");
  });
});

// ─── DR5: Heap recovery ───────────────────────────────────────────────────────

describe("DR5 — Heap recovery: After 20 request cycles (beginRequest + completeRequest), session state is consistent and bounded", () => {
  let session: MemoryWindowSessionV2;

  beforeEach(() => {
    session = makeSession();
  });

  it("after 20 beginRequest + completeRequest cycles, state is 'ready' and generation is exactly 20", () => {
    for (let i = 1; i <= 20; i++) {
      const { generation } = session.beginRequest(`query-${i}`);
      session.completeRequest(generation, i * 10);
    }
    expect(session.state).toBe<WindowSessionState>("ready");
    expect(session.generation).toBe(20);
    expect(session.revision).toBe(200); // last completed revision = 20 * 10
  });

  it("after 20 cycles, each intermediate signal from prior requests is aborted (no live orphan controllers)", () => {
    const signals: AbortSignal[] = [];
    for (let i = 1; i <= 20; i++) {
      const { generation, signal } = session.beginRequest(`cycle-${i}`);
      signals.push(signal);
      session.completeRequest(generation, i);
    }
    // All signals except the last should have been aborted by subsequent beginRequest calls.
    for (let i = 0; i < signals.length - 1; i++) {
      expect(signals[i].aborted).toBe(true);
    }
    // The last signal is NOT aborted (the request completed successfully).
    // Note: completeRequest doesn't abort the signal — it leaves it open.
    expect(signals[signals.length - 1].aborted).toBe(false);
  });

  it("after 20 cycles, guardRevision correctly reflects the last revision (200) and rejects prior revisions", () => {
    for (let i = 1; i <= 20; i++) {
      const { generation } = session.beginRequest(`q${i}`);
      session.completeRequest(generation, i * 10);
    }
    expect(session.guardRevision(200)).toBe(true);
    expect(session.guardRevision(190)).toBe(false); // previous revision
    expect(session.guardRevision(0)).toBe(false);
  });

  it("after 20 cycles + reset(), session returns to clean initial state", () => {
    for (let i = 1; i <= 20; i++) {
      const { generation } = session.beginRequest(`q${i}`);
      session.completeRequest(generation, i);
    }
    session.reset();
    expect(session.generation).toBe(0);
    expect(session.revision).toBe(0);
    expect(session.state).toBe("idle");
  });

  it("after 20 cycles of beginRequest only (no complete), generation is 20, state is 'loading'", () => {
    // Each beginRequest supersedes the previous — testing that the session
    // doesn't accumulate state from cancelled requests.
    for (let i = 0; i < 20; i++) {
      session.beginRequest(`query-${i}`);
    }
    expect(session.generation).toBe(20);
    expect(session.state).toBe<WindowSessionState>("loading");
    expect(session.revision).toBe(0); // never completed
  });

  it("after 20 cycles of beginRequest + failRequest, generation is 20, state is 'error', revision unchanged", () => {
    for (let i = 1; i <= 20; i++) {
      const { generation } = session.beginRequest(`q-fail-${i}`);
      session.failRequest(generation);
    }
    expect(session.generation).toBe(20);
    expect(session.state).toBe<WindowSessionState>("error");
    expect(session.revision).toBe(0); // never successfully completed
  });
});

// ─── DR6: No orphan listeners / workers ──────────────────────────────────────

describe("DR6 — No orphan: Multiple beginRequest calls in sequence — only the last signal is alive; all previous are aborted", () => {
  let session: MemoryWindowSessionV2;

  beforeEach(() => {
    session = makeSession();
  });

  it("two consecutive beginRequest calls: first signal is aborted, second is alive", () => {
    const { signal: sig1 } = session.beginRequest("first");
    const { signal: sig2 } = session.beginRequest("second");

    expect(sig1.aborted).toBe(true);
    expect(sig2.aborted).toBe(false);
  });

  it("five consecutive beginRequest calls: only the last signal is alive", () => {
    const signals: AbortSignal[] = [];
    for (let i = 0; i < 5; i++) {
      const { signal } = session.beginRequest(`query-${i}`);
      signals.push(signal);
    }

    // First four aborted
    for (let i = 0; i < 4; i++) {
      expect(signals[i].aborted).toBe(true);
    }
    // Last one is alive
    expect(signals[4].aborted).toBe(false);
  });

  it("ten rapid beginRequest calls leave exactly the last signal not aborted", () => {
    const signals: AbortSignal[] = [];
    for (let i = 0; i < 10; i++) {
      const { signal } = session.beginRequest(`rapid-${i}`);
      signals.push(signal);
    }

    const abortedCount = signals.filter((s) => s.aborted).length;
    const liveCount = signals.filter((s) => !s.aborted).length;

    expect(abortedCount).toBe(9);
    expect(liveCount).toBe(1);
    // The last one must be the live signal
    expect(signals[signals.length - 1].aborted).toBe(false);
  });

  it("each beginRequest returns a distinct AbortSignal instance", () => {
    const { signal: sig1 } = session.beginRequest("q1");
    const { signal: sig2 } = session.beginRequest("q2");
    const { signal: sig3 } = session.beginRequest("q3");

    // All three are different objects (each beginRequest creates a new AbortController)
    expect(sig1).not.toBe(sig2);
    expect(sig2).not.toBe(sig3);
    expect(sig1).not.toBe(sig3);
  });

  it("generation increments monotonically across multiple rapid beginRequest calls", () => {
    const generations: number[] = [];
    for (let i = 0; i < 5; i++) {
      const { generation } = session.beginRequest(`q${i}`);
      generations.push(generation);
    }
    // Must be [1, 2, 3, 4, 5]
    for (let i = 0; i < generations.length; i++) {
      expect(generations[i]).toBe(i + 1);
    }
  });

  it("two windows issuing rapid requests do not share orphaned controllers", () => {
    const winA = makeSession({ instanceId: "win-orphan-A" });
    const winB = makeSession({ instanceId: "win-orphan-B" });

    const { signal: sigA1 } = winA.beginRequest("qA1");
    const { signal: sigA2 } = winA.beginRequest("qA2"); // cancels sigA1

    const { signal: sigB1 } = winB.beginRequest("qB1");
    const { signal: sigB2 } = winB.beginRequest("qB2"); // cancels sigB1

    // A's first signal is aborted by A itself
    expect(sigA1.aborted).toBe(true);
    expect(sigA2.aborted).toBe(false);

    // B's first signal is aborted by B itself — not by A
    expect(sigB1.aborted).toBe(true);
    expect(sigB2.aborted).toBe(false);

    // Cross-check: A's second signal was NOT aborted by B's operations
    expect(sigA2.aborted).toBe(false);
  });

  it("after markDetached() all controllers are cleaned up; new beginRequest after reset() creates a fresh controller", () => {
    const { signal: sig1 } = session.beginRequest("q1");
    session.beginRequest("q2");      // sig1 aborted
    session.markDetached();           // sig2 aborted; controller nulled

    session.reset();                  // clean slate

    const { signal: sig3 } = session.beginRequest("q3"); // fresh controller
    expect(sig1.aborted).toBe(true);
    expect(sig3.aborted).toBe(false);
    expect(sig3).not.toBe(sig1); // definitely a new instance
  });
});
