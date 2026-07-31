/**
 * windowSession.test.ts
 *
 * Unit tests for MemoryWindowSessionV2 exact states, per-instance request
 * ownership, generation increment/cancel, query/policy/revision guards,
 * and detached restore validation.
 *
 * Requirements: F4.1 (MGR-007, MGR-008, MGR-020).
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
    instanceId: "win-test-1",
    policyHash: "policy-abc",
    schemaVersion: "2.0",
    ...overrides,
  };
}

function makeSession(overrides: Partial<WindowSessionConfig> = {}): MemoryWindowSessionV2 {
  return new MemoryWindowSessionV2(makeConfig(overrides));
}

// ─── Tests ────────────────────────────────────────────────────────────────────

describe("MemoryWindowSessionV2", () => {
  let session: MemoryWindowSessionV2;

  beforeEach(() => {
    session = makeSession();
  });

  // ── Initial state ──────────────────────────────────────────────────────────

  describe("initial state", () => {
    it("starts with state 'idle'", () => {
      expect(session.state).toBe<WindowSessionState>("idle");
    });

    it("starts with generation 0", () => {
      expect(session.generation).toBe(0);
    });

    it("starts with revision 0", () => {
      expect(session.revision).toBe(0);
    });

    it("exposes instanceId from config", () => {
      const s = makeSession({ instanceId: "win-xyz" });
      expect(s.instanceId).toBe("win-xyz");
    });

    it("exposes config fields", () => {
      const cfg = makeConfig({ policyHash: "ph-1", schemaVersion: "2.1" });
      const s = new MemoryWindowSessionV2(cfg);
      expect(s.config.policyHash).toBe("ph-1");
      expect(s.config.schemaVersion).toBe("2.1");
    });
  });

  // ── beginRequest ──────────────────────────────────────────────────────────

  describe("beginRequest", () => {
    it("increments generation from 0 to 1 on first call", () => {
      session.beginRequest("hello world");
      expect(session.generation).toBe(1);
    });

    it("sets state to 'loading'", () => {
      session.beginRequest("q");
      expect(session.state).toBe<WindowSessionState>("loading");
    });

    it("returns the new generation in the token", () => {
      const token = session.beginRequest("q");
      expect(token.generation).toBe(1);
    });

    it("returns an AbortSignal in the token", () => {
      const token = session.beginRequest("q");
      expect(token.signal).toBeInstanceOf(AbortSignal);
    });

    it("signal is not aborted initially", () => {
      const { signal } = session.beginRequest("q");
      expect(signal.aborted).toBe(false);
    });

    it("increments generation on each successive call", () => {
      session.beginRequest("a");
      session.beginRequest("b");
      session.beginRequest("c");
      expect(session.generation).toBe(3);
    });

    it("each call returns a distinct generation", () => {
      const t1 = session.beginRequest("a");
      const t2 = session.beginRequest("b");
      expect(t1.generation).not.toBe(t2.generation);
    });
  });

  // ── Cancellation: second beginRequest aborts first signal ────────────────

  describe("cancellation on successive beginRequest", () => {
    it("first signal is aborted when second beginRequest is called", () => {
      const first = session.beginRequest("first query");
      const firstSignal = first.signal;

      // The first signal should still be live at this point.
      expect(firstSignal.aborted).toBe(false);

      // A second request cancels the first.
      session.beginRequest("second query");

      expect(firstSignal.aborted).toBe(true);
    });

    it("second signal is not aborted after second beginRequest", () => {
      session.beginRequest("first");
      const { signal: second } = session.beginRequest("second");
      expect(second.aborted).toBe(false);
    });

    it("state remains 'loading' after second beginRequest", () => {
      session.beginRequest("first");
      session.beginRequest("second");
      expect(session.state).toBe<WindowSessionState>("loading");
    });
  });

  // ── completeRequest ───────────────────────────────────────────────────────

  describe("completeRequest", () => {
    it("returns true and sets state to 'ready' for matching generation", () => {
      const { generation } = session.beginRequest("q");
      const result = session.completeRequest(generation, 42);
      expect(result).toBe(true);
      expect(session.state).toBe<WindowSessionState>("ready");
    });

    it("records the revision when completing successfully", () => {
      const { generation } = session.beginRequest("q");
      session.completeRequest(generation, 99);
      expect(session.revision).toBe(99);
    });

    it("returns false for a stale (mismatched) generation", () => {
      const { generation } = session.beginRequest("q");
      // Start a second request — makes first generation stale.
      session.beginRequest("q2");
      const result = session.completeRequest(generation, 10);
      expect(result).toBe(false);
    });

    it("does not mutate state on stale generation", () => {
      const { generation } = session.beginRequest("q");
      session.beginRequest("q2"); // state is 'loading' for gen 2
      session.completeRequest(generation, 10); // stale
      // State should still reflect the in-flight second request.
      expect(session.state).toBe<WindowSessionState>("loading");
    });

    it("does not update revision on stale generation", () => {
      const { generation } = session.beginRequest("q");
      session.beginRequest("q2");
      session.completeRequest(generation, 777);
      expect(session.revision).toBe(0); // unchanged
    });
  });

  // ── failRequest ───────────────────────────────────────────────────────────

  describe("failRequest", () => {
    it("returns true and sets state to 'error' for matching generation", () => {
      const { generation } = session.beginRequest("q");
      const result = session.failRequest(generation);
      expect(result).toBe(true);
      expect(session.state).toBe<WindowSessionState>("error");
    });

    it("returns false for a stale generation", () => {
      const { generation } = session.beginRequest("q");
      session.beginRequest("q2");
      const result = session.failRequest(generation);
      expect(result).toBe(false);
    });

    it("does not mutate state for a stale generation", () => {
      const { generation } = session.beginRequest("q");
      session.beginRequest("q2");
      session.failRequest(generation);
      expect(session.state).toBe<WindowSessionState>("loading");
    });
  });

  // ── guardRevision ─────────────────────────────────────────────────────────

  describe("guardRevision", () => {
    it("returns true when baseRevision matches the current revision (0 initially)", () => {
      expect(session.guardRevision(0)).toBe(true);
    });

    it("returns false when baseRevision does not match", () => {
      expect(session.guardRevision(5)).toBe(false);
    });

    it("returns true after a completed request with the same revision", () => {
      const { generation } = session.beginRequest("q");
      session.completeRequest(generation, 7);
      expect(session.guardRevision(7)).toBe(true);
    });

    it("returns false when revision has advanced beyond the supplied base", () => {
      const { generation } = session.beginRequest("q");
      session.completeRequest(generation, 7);
      expect(session.guardRevision(6)).toBe(false);
    });
  });

  // ── guardPolicy ──────────────────────────────────────────────────────────

  describe("guardPolicy", () => {
    it("returns true for the configured policy hash", () => {
      const s = makeSession({ policyHash: "ph-correct" });
      expect(s.guardPolicy("ph-correct")).toBe(true);
    });

    it("returns false for a different policy hash", () => {
      const s = makeSession({ policyHash: "ph-correct" });
      expect(s.guardPolicy("ph-other")).toBe(false);
    });

    it("returns false for an empty string when config is non-empty", () => {
      const s = makeSession({ policyHash: "ph-correct" });
      expect(s.guardPolicy("")).toBe(false);
    });
  });

  // ── markDetached ─────────────────────────────────────────────────────────

  describe("markDetached", () => {
    it("sets state to 'detached'", () => {
      session.markDetached();
      expect(session.state).toBe<WindowSessionState>("detached");
    });

    it("sets state to 'detached' even when a request is in flight", () => {
      session.beginRequest("q");
      session.markDetached();
      expect(session.state).toBe<WindowSessionState>("detached");
    });

    it("aborts the active signal when detaching from a loading state", () => {
      const { signal } = session.beginRequest("q");
      session.markDetached();
      expect(signal.aborted).toBe(true);
    });

    it("can be called on an idle session without throwing", () => {
      expect(() => session.markDetached()).not.toThrow();
    });
  });

  // ── validateDetachedRestore ───────────────────────────────────────────────

  describe("validateDetachedRestore", () => {
    it("returns true when state is 'detached'", () => {
      session.markDetached();
      expect(session.validateDetachedRestore()).toBe(true);
    });

    it("returns false when state is 'idle'", () => {
      expect(session.validateDetachedRestore()).toBe(false);
    });

    it("returns false when state is 'loading'", () => {
      session.beginRequest("q");
      expect(session.validateDetachedRestore()).toBe(false);
    });

    it("returns false when state is 'ready'", () => {
      const { generation } = session.beginRequest("q");
      session.completeRequest(generation, 1);
      expect(session.validateDetachedRestore()).toBe(false);
    });

    it("returns false when state is 'error'", () => {
      const { generation } = session.beginRequest("q");
      session.failRequest(generation);
      expect(session.validateDetachedRestore()).toBe(false);
    });

    it("returns false when state is 'stale'", () => {
      session.markStale();
      expect(session.validateDetachedRestore()).toBe(false);
    });
  });

  // ── markStale ─────────────────────────────────────────────────────────────

  describe("markStale", () => {
    it("sets state to 'stale'", () => {
      session.markStale();
      expect(session.state).toBe<WindowSessionState>("stale");
    });

    it("sets state to 'stale' from 'ready'", () => {
      const { generation } = session.beginRequest("q");
      session.completeRequest(generation, 1);
      session.markStale();
      expect(session.state).toBe<WindowSessionState>("stale");
    });
  });

  // ── reset ─────────────────────────────────────────────────────────────────

  describe("reset", () => {
    it("sets state back to 'idle'", () => {
      session.beginRequest("q");
      session.reset();
      expect(session.state).toBe<WindowSessionState>("idle");
    });

    it("resets generation to 0", () => {
      session.beginRequest("q");
      session.beginRequest("q2");
      session.reset();
      expect(session.generation).toBe(0);
    });

    it("resets revision to 0", () => {
      const { generation } = session.beginRequest("q");
      session.completeRequest(generation, 55);
      session.reset();
      expect(session.revision).toBe(0);
    });

    it("can be called from 'detached' state", () => {
      session.markDetached();
      session.reset();
      expect(session.state).toBe<WindowSessionState>("idle");
    });

    it("aborts the active abort controller on reset", () => {
      const { signal } = session.beginRequest("q");
      session.reset();
      expect(signal.aborted).toBe(true);
    });

    it("can be called repeatedly without throwing", () => {
      session.reset();
      session.reset();
      expect(session.state).toBe<WindowSessionState>("idle");
    });

    it("allows a new request cycle after reset", () => {
      const { generation: gen1 } = session.beginRequest("q");
      session.completeRequest(gen1, 10);
      session.reset();

      const { generation: gen2 } = session.beginRequest("q2");
      expect(gen2).toBe(1);
      const ok = session.completeRequest(gen2, 20);
      expect(ok).toBe(true);
      expect(session.revision).toBe(20);
    });
  });
});
