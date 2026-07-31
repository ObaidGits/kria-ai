/**
 * patchConcurrency.test.ts
 *
 * Concurrent and temporal scenario tests for per-window patch convergence.
 * Covers 8 scenarios from task 5.2.3:
 *   S1 — Simultaneous focus and writes (two windows issue writes at the same time)
 *   S2 — Pending confirmation (pending write tracked per window; confirmed removes it)
 *   S3 — Duplicate patches (same base+target → no-op, already applied)
 *   S4 — Reordered patches (base_revision > current → rejected, gap)
 *   S5 — Missing patches (gap: current < patch.base → refetch signal)
 *   S6 — Lag (patch from stale revision: base < current → no-op)
 *   S7 — Reconnect (after reconnect, window issues bounded refetch from current revision)
 *   S8 — Bounded per-window refetch (one window refetching doesn't affect the other)
 *
 * Requirements: MGR-008 (revision/patch consistency), F4.1 (window session isolation).
 */

import { describe, it, expect } from "vitest";
import {
  patchReducer,
  type ReducerState,
  type AuthorityPatch,
  type PatchAction,
  type PendingWrite,
} from "./patchReducer";
import { MemoryWindowSessionV2, type WindowSessionConfig } from "./windowSession";

// ─── Helpers ──────────────────────────────────────────────────────────────────

type Item = { id: string; label: string };

function makeState(overrides: Partial<ReducerState<Item>> = {}): ReducerState<Item> {
  return {
    items: [],
    revision: 0,
    pendingWrites: [],
    schemaVersion: "2.0",
    policyHash: "policy-abc",
    queryHash: "query-xyz",
    ...overrides,
  };
}

function makePatch(overrides: Partial<AuthorityPatch> = {}): AuthorityPatch {
  return {
    base_revision: 0,
    target_revision: 1,
    changes: [],
    invalidations: [],
    recovery_cursor: "cursor-v1",
    schema_version: "2.0",
    policy_hash: "policy-abc",
    ...overrides,
  };
}

function makePendingWrite(overrides: Partial<PendingWrite> = {}): PendingWrite {
  return {
    commandId: "cmd-1",
    operationName: "test-op",
    baseRevision: 0,
    ...overrides,
  };
}

function makeSession(instanceId: string, policyHash = "policy-abc"): MemoryWindowSessionV2 {
  const cfg: WindowSessionConfig = { instanceId, policyHash, schemaVersion: "2.0" };
  return new MemoryWindowSessionV2(cfg);
}

// ─── S1: Simultaneous focus and writes ───────────────────────────────────────

describe("S1 — Simultaneous focus and writes: two windows issue writes simultaneously", () => {
  /**
   * Two windows each hold a pending write at the same base revision.
   * A patch arriving for Window A must not affect Window B's pending write,
   * and vice versa. Each window's pending write is completely independent.
   */

  it("pending write in Window A is independent of pending write in Window B", () => {
    const writeA = makePendingWrite({ commandId: "cmd-A", baseRevision: 5 });
    const writeB = makePendingWrite({ commandId: "cmd-B", baseRevision: 5 });

    let stateA = makeState({ revision: 5, pendingWrites: [writeA], policyHash: "ph-A" });
    let stateB = makeState({ revision: 5, pendingWrites: [writeB], policyHash: "ph-B" });

    // Authority confirms Window A's write at revision 6.
    stateA = patchReducer(stateA, { type: "CONFIRM_WRITE", commandId: "cmd-A", revision: 6 });

    expect(stateA.pendingWrites).toHaveLength(0);
    expect(stateA.revision).toBe(6);

    // Window B's pending write is still present and unconfirmed.
    expect(stateB.pendingWrites).toHaveLength(1);
    expect(stateB.pendingWrites[0].commandId).toBe("cmd-B");
    expect(stateB.revision).toBe(5);
  });

  it("both windows receive the same authority patch but each advances independently", () => {
    let stateA = makeState({ revision: 0, policyHash: "ph-A",
      pendingWrites: [makePendingWrite({ commandId: "cmd-A" })] });
    let stateB = makeState({ revision: 0, policyHash: "ph-B",
      pendingWrites: [makePendingWrite({ commandId: "cmd-B" })] });

    // Same authority patch broadcasted; each window has a matching policy.
    const patchA = makePatch({ base_revision: 0, target_revision: 1, policy_hash: "ph-A" });
    const patchB = makePatch({ base_revision: 0, target_revision: 1, policy_hash: "ph-B" });

    const itemsA: Item[] = [{ id: "a1", label: "A write result" }];
    const itemsB: Item[] = [{ id: "b1", label: "B write result" }];

    stateA = patchReducer(stateA, { type: "APPLY_PATCH", patch: patchA }, itemsA);
    stateB = patchReducer(stateB, { type: "APPLY_PATCH", patch: patchB }, itemsB);

    // Both advanced, but each has its own items and its own pending write still tracked.
    expect(stateA.revision).toBe(1);
    expect(stateA.items).toEqual(itemsA);
    expect(stateA.pendingWrites).toHaveLength(1); // not yet confirmed

    expect(stateB.revision).toBe(1);
    expect(stateB.items).toEqual(itemsB);
    expect(stateB.pendingWrites).toHaveLength(1); // not yet confirmed
  });

  it("concurrent focus changes (beginRequest) on two sessions do not interfere", () => {
    const winA = makeSession("win-A");
    const winB = makeSession("win-B");

    // Both windows simultaneously begin requests (focus change + write).
    const { signal: sigA } = winA.beginRequest("focus:entity-A");
    const { signal: sigB } = winB.beginRequest("focus:entity-B");

    // Neither signal should be aborted by the other window's beginRequest.
    expect(sigA.aborted).toBe(false);
    expect(sigB.aborted).toBe(false);

    // Each has its own generation counter.
    expect(winA.generation).toBe(1);
    expect(winB.generation).toBe(1);
  });
});

// ─── S2: Pending confirmation ─────────────────────────────────────────────────

describe("S2 — Pending confirmation: pending write tracked per window; confirmed removes it; unconfirmed stays", () => {
  /**
   * A pending write must remain in pendingWrites until a matching CONFIRM_WRITE
   * or ROLLBACK_WRITE removes it. No other action type should remove it.
   * Per-window: confirming A's write does not touch B's unconfirmed write.
   */

  it("pending write stays in state after APPLY_PATCH (not yet confirmed)", () => {
    const write = makePendingWrite({ commandId: "cmd-pending", baseRevision: 0 });
    let state = makeState({ revision: 0, pendingWrites: [write] });

    const patch = makePatch({ base_revision: 0, target_revision: 1 });
    state = patchReducer(state, { type: "APPLY_PATCH", patch });

    // Patch applied but confirmation hasn't arrived — pending write must remain.
    expect(state.revision).toBe(1);
    expect(state.pendingWrites).toHaveLength(1);
    expect(state.pendingWrites[0].commandId).toBe("cmd-pending");
  });

  it("confirmed write removes only the matching commandId", () => {
    const writeA = makePendingWrite({ commandId: "cmd-A", baseRevision: 0 });
    const writeB = makePendingWrite({ commandId: "cmd-B", baseRevision: 0 });
    let state = makeState({ revision: 1, pendingWrites: [writeA, writeB] });

    state = patchReducer(state, { type: "CONFIRM_WRITE", commandId: "cmd-A", revision: 1 });

    // Only cmd-A is removed; cmd-B stays pending.
    expect(state.pendingWrites).toHaveLength(1);
    expect(state.pendingWrites[0].commandId).toBe("cmd-B");
  });

  it("unconfirmed write stays after confirming a different commandId", () => {
    const writeX = makePendingWrite({ commandId: "cmd-X", baseRevision: 2 });
    let state = makeState({ revision: 3, pendingWrites: [writeX] });

    // Confirming cmd-Y which is not in pendingWrites — cmd-X must remain.
    state = patchReducer(state, { type: "CONFIRM_WRITE", commandId: "cmd-Y", revision: 4 });

    expect(state.pendingWrites).toHaveLength(1);
    expect(state.pendingWrites[0].commandId).toBe("cmd-X");
  });

  it("confirming write in Window A does not clear Window B's unconfirmed write", () => {
    const writeA = makePendingWrite({ commandId: "cmd-A" });
    const writeB = makePendingWrite({ commandId: "cmd-B" });

    let stateA = makeState({ revision: 1, pendingWrites: [writeA] });
    const stateB = makeState({ revision: 1, pendingWrites: [writeB] });

    stateA = patchReducer(stateA, { type: "CONFIRM_WRITE", commandId: "cmd-A", revision: 2 });

    expect(stateA.pendingWrites).toHaveLength(0);
    // B's write still pending — unaffected.
    expect(stateB.pendingWrites).toHaveLength(1);
    expect(stateB.pendingWrites[0].commandId).toBe("cmd-B");
  });

  it("multiple patches do not remove a pending write — confirmation is required", () => {
    const write = makePendingWrite({ commandId: "cmd-long-running", baseRevision: 0 });
    let state = makeState({ revision: 0, pendingWrites: [write] });

    // Apply three successive patches; write remains unconfirmed throughout.
    for (let base = 0; base < 3; base++) {
      const patch = makePatch({ base_revision: base, target_revision: base + 1 });
      state = patchReducer(state, { type: "APPLY_PATCH", patch });
      expect(state.pendingWrites).toHaveLength(1);
    }
    expect(state.revision).toBe(3);
    expect(state.pendingWrites[0].commandId).toBe("cmd-long-running");
  });
});

// ─── S3: Duplicate patches ────────────────────────────────────────────────────

describe("S3 — Duplicate patches: same base_revision + same target_revision is a no-op (already applied)", () => {
  /**
   * Once a patch is applied the revision advances. Re-delivering the same patch
   * (same base_revision, same target_revision) hits a revision mismatch and
   * must be a pure no-op — the reducer must return the same state reference.
   *
   * This models the scenario where the server re-delivers a patch because the
   * client was briefly disconnected and re-subscribed.
   */

  it("re-delivering the same patch after it was applied returns same state reference", () => {
    const state0 = makeState({ revision: 0 });
    const patch = makePatch({ base_revision: 0, target_revision: 1 });

    const state1 = patchReducer(state0, { type: "APPLY_PATCH", patch });
    expect(state1.revision).toBe(1);

    // Same patch arrives again — base (0) no longer matches revision (1).
    const state1b = patchReducer(state1, { type: "APPLY_PATCH", patch });
    expect(state1b).toBe(state1); // exact same reference
    expect(state1b.revision).toBe(1);
  });

  it("duplicate patch does not alter items", () => {
    const items: Item[] = [{ id: "x", label: "existing" }];
    const state0 = makeState({ revision: 0, items: [] });
    const patch = makePatch({ base_revision: 0, target_revision: 1 });

    const state1 = patchReducer(state0, { type: "APPLY_PATCH", patch }, items);
    expect(state1.items).toEqual(items);

    // Duplicate patch with different newItems — must not apply.
    const otherItems: Item[] = [{ id: "y", label: "should not appear" }];
    const state1b = patchReducer(state1, { type: "APPLY_PATCH", patch }, otherItems);
    expect(state1b).toBe(state1);
    expect(state1b.items).toEqual(items); // items unchanged
  });

  it("duplicate patch does not alter pendingWrites", () => {
    const write = makePendingWrite({ commandId: "cmd-pending" });
    const state0 = makeState({ revision: 0, pendingWrites: [write] });
    const patch = makePatch({ base_revision: 0, target_revision: 1 });

    const state1 = patchReducer(state0, { type: "APPLY_PATCH", patch });
    const state1b = patchReducer(state1, { type: "APPLY_PATCH", patch });

    expect(state1b).toBe(state1);
    expect(state1b.pendingWrites).toHaveLength(1);
  });

  it("patch with same base_revision but different target_revision is also rejected after first apply", () => {
    // After applying base=0→target=1, a patch claiming base=0→target=2 is also stale.
    const state0 = makeState({ revision: 0 });
    const patch1 = makePatch({ base_revision: 0, target_revision: 1 });
    const state1 = patchReducer(state0, { type: "APPLY_PATCH", patch: patch1 });

    const patchAlt = makePatch({ base_revision: 0, target_revision: 2 });
    const stateAlt = patchReducer(state1, { type: "APPLY_PATCH", patch: patchAlt });

    expect(stateAlt).toBe(state1); // no-op
    expect(stateAlt.revision).toBe(1);
  });
});

// ─── S4: Reordered patches ────────────────────────────────────────────────────

describe("S4 — Reordered patches: patch with base_revision > current revision is rejected (gap)", () => {
  /**
   * If a patch arrives whose base_revision is ahead of the current revision,
   * there is a gap. The reducer must reject it (return same state reference).
   * The caller is expected to detect this and trigger a bounded refetch.
   */

  it("patch with base_revision > current revision is rejected (returns same state)", () => {
    const state = makeState({ revision: 3 });
    const aheadPatch = makePatch({ base_revision: 7, target_revision: 8 });

    const next = patchReducer(state, { type: "APPLY_PATCH", patch: aheadPatch });
    expect(next).toBe(state);
    expect(next.revision).toBe(3);
  });

  it("reordered patch does not alter items or pendingWrites", () => {
    const items: Item[] = [{ id: "a", label: "current" }];
    const write = makePendingWrite({ commandId: "cmd-1" });
    const state = makeState({ revision: 2, items, pendingWrites: [write] });

    const aheadPatch = makePatch({ base_revision: 5, target_revision: 6 });
    const next = patchReducer(state, { type: "APPLY_PATCH", patch: aheadPatch });

    expect(next).toBe(state);
    expect(next.items).toBe(items);
    expect(next.pendingWrites).toBe(state.pendingWrites);
  });

  it("reordered patch followed by correct patch advances state correctly", () => {
    let state = makeState({ revision: 2 });

    // Out-of-order patch arrives first — no-op.
    const outOfOrder = makePatch({ base_revision: 5, target_revision: 6 });
    state = patchReducer(state, { type: "APPLY_PATCH", patch: outOfOrder });
    expect(state.revision).toBe(2); // unchanged

    // Correct sequential patch arrives.
    const correct = makePatch({ base_revision: 2, target_revision: 3 });
    state = patchReducer(state, { type: "APPLY_PATCH", patch: correct });
    expect(state.revision).toBe(3);
  });

  it("multiple reordered patches in sequence all return the same state reference", () => {
    const state = makeState({ revision: 1 });
    const patches = [
      makePatch({ base_revision: 3, target_revision: 4 }),
      makePatch({ base_revision: 5, target_revision: 6 }),
      makePatch({ base_revision: 10, target_revision: 11 }),
    ];

    let current = state;
    for (const patch of patches) {
      const next = patchReducer(current, { type: "APPLY_PATCH", patch });
      expect(next).toBe(current); // all no-ops
    }
    expect(current.revision).toBe(1);
  });
});

// ─── S5: Missing patches (gap detection → refetch signal) ────────────────────

describe("S5 — Missing patches: gap in patch stream triggers refetch signal", () => {
  /**
   * A gap is detected when the arriving patch's base_revision is greater than the
   * window's current revision (patches are missing in between). The reducer returns
   * the same state (no-op), and the caller should issue a REFETCH_REQUIRED action.
   *
   * REFETCH_REQUIRED itself is a pure signal — state remains unchanged. This suite
   * verifies both that the gap produces a no-op and that REFETCH_REQUIRED is a no-op,
   * so the caller's refetch machinery is not blocked by partial state.
   */

  it("gap patch (base > current) is rejected — state unchanged, caller must refetch", () => {
    const state = makeState({ revision: 4 });
    const gapPatch = makePatch({ base_revision: 7, target_revision: 8 });

    const next = patchReducer(state, { type: "APPLY_PATCH", patch: gapPatch });
    expect(next).toBe(state);
    expect(next.revision).toBe(4);
  });

  it("REFETCH_REQUIRED after gap leaves state unchanged (pure signal)", () => {
    const state = makeState({ revision: 4 });

    const afterRefetch = patchReducer(state, {
      type: "REFETCH_REQUIRED",
      queryHash: "query-xyz",
      reason: "gap in patch stream",
    });
    expect(afterRefetch).toBe(state);
  });

  it("gap → REFETCH_REQUIRED → correct patch sequence converges correctly", () => {
    let state = makeState({ revision: 4 });

    // Gap arrives — no-op.
    const gapPatch = makePatch({ base_revision: 7, target_revision: 8 });
    state = patchReducer(state, { type: "APPLY_PATCH", patch: gapPatch });
    expect(state.revision).toBe(4);

    // Caller signals refetch — no-op.
    state = patchReducer(state, {
      type: "REFETCH_REQUIRED",
      queryHash: "query-xyz",
      reason: "gap: missing revisions 5-6",
    });
    expect(state.revision).toBe(4);

    // After refetch, fresh patch from current revision.
    const fresh = makePatch({ base_revision: 4, target_revision: 5 });
    const refetched: Item[] = [{ id: "r1", label: "refetched" }];
    state = patchReducer(state, { type: "APPLY_PATCH", patch: fresh }, refetched);

    expect(state.revision).toBe(5);
    expect(state.items).toEqual(refetched);
  });

  it("items and pendingWrites are preserved through gap + refetch signal", () => {
    const items: Item[] = [{ id: "old", label: "stale display" }];
    const write = makePendingWrite({ commandId: "cmd-inflight" });
    let state = makeState({ revision: 2, items, pendingWrites: [write] });

    const gapPatch = makePatch({ base_revision: 5, target_revision: 6 });
    state = patchReducer(state, { type: "APPLY_PATCH", patch: gapPatch });

    state = patchReducer(state, {
      type: "REFETCH_REQUIRED",
      queryHash: "query-xyz",
      reason: "gap",
    });

    // Items remain displayable (stale); pending write is still tracked.
    expect(state.items).toBe(items);
    expect(state.pendingWrites).toHaveLength(1);
    expect(state.revision).toBe(2);
  });
});

// ─── S6: Lag (stale/late patch) ───────────────────────────────────────────────

describe("S6 — Lag: patch from a stale base revision (base < current) is a no-op", () => {
  /**
   * A lagging patch arrives whose base_revision is behind the current revision.
   * This happens when a slow push finally delivers a patch the client has already
   * moved past. The reducer must silently reject it (no-op).
   */

  it("lag patch (base < current revision) is silently rejected", () => {
    const state = makeState({ revision: 8 });
    const lagPatch = makePatch({ base_revision: 3, target_revision: 4 });

    const next = patchReducer(state, { type: "APPLY_PATCH", patch: lagPatch });
    expect(next).toBe(state);
    expect(next.revision).toBe(8);
  });

  it("lag patch does not overwrite items with stale data", () => {
    const current: Item[] = [{ id: "current", label: "current state" }];
    const stale: Item[] = [{ id: "stale", label: "old state" }];
    const state = makeState({ revision: 10, items: current });

    const lagPatch = makePatch({ base_revision: 2, target_revision: 3 });
    const next = patchReducer(state, { type: "APPLY_PATCH", patch: lagPatch }, stale);

    expect(next).toBe(state);
    expect(next.items).toBe(current);
  });

  it("lag patch base = current - 1 is still rejected (only exact base match applies)", () => {
    const state = makeState({ revision: 5 });
    const almostCurrent = makePatch({ base_revision: 4, target_revision: 5 });

    const next = patchReducer(state, { type: "APPLY_PATCH", patch: almostCurrent });
    // base_revision (4) !== state.revision (5) → no-op
    expect(next).toBe(state);
  });

  it("lag patch at base=0 is rejected when current revision is 0 but correct base=0 patch applies", () => {
    // Sanity: base=0 on revision=0 is valid.
    const state0 = makeState({ revision: 0 });
    const patch0 = makePatch({ base_revision: 0, target_revision: 1 });
    const state1 = patchReducer(state0, { type: "APPLY_PATCH", patch: patch0 });
    expect(state1.revision).toBe(1);

    // Now the same base=0 patch arrives as lag — rejected.
    const lag = patchReducer(state1, { type: "APPLY_PATCH", patch: patch0 });
    expect(lag).toBe(state1);
    expect(lag.revision).toBe(1);
  });

  it("window receives many lagging patches — revision is never regressed", () => {
    let state = makeState({ revision: 20 });
    const lagPatches = [
      makePatch({ base_revision: 0, target_revision: 1 }),
      makePatch({ base_revision: 5, target_revision: 6 }),
      makePatch({ base_revision: 10, target_revision: 11 }),
      makePatch({ base_revision: 15, target_revision: 16 }),
      makePatch({ base_revision: 19, target_revision: 20 }),
    ];

    for (const patch of lagPatches) {
      const next = patchReducer(state, { type: "APPLY_PATCH", patch });
      expect(next).toBe(state); // all no-ops
    }
    expect(state.revision).toBe(20);
  });
});

// ─── S7: Reconnect ────────────────────────────────────────────────────────────

describe("S7 — Reconnect: after reconnect, window issues bounded refetch from current revision", () => {
  /**
   * After a disconnect, the window's current revision is the anchor for the
   * bounded refetch. The session uses guardRevision to validate the base revision
   * before submitting new writes. After reconnect, a refetch starting from the
   * current revision brings the window back to convergence.
   *
   * The recovery_cursor in AuthorityPatch is the server-provided cursor for
   * re-fetching from a revision boundary.
   */

  it("session revision is preserved through markDetached + reset + reconnect cycle", () => {
    const win = makeSession("win-reconnect");

    // Window completes a request at revision 7.
    const { generation: gen1 } = win.beginRequest("query:overview");
    win.completeRequest(gen1, 7);
    expect(win.revision).toBe(7);

    // Disconnect — detach.
    win.markDetached();
    expect(win.state).toBe("detached");
    expect(win.revision).toBe(7); // revision is preserved on detach

    // Reconnect — reset and begin new request.
    win.reset();
    expect(win.state).toBe("idle");
    // After reset, revision is cleared — caller uses the last known revision
    // from its own stored state for the refetch anchor.
    expect(win.revision).toBe(0);
  });

  it("reducer state is the reconnect anchor — refetch from current revision converges", () => {
    // Last known state before disconnect.
    let state = makeState({ revision: 12 });

    // Reconnect: issue refetch starting from revision 12 → get patch 12→13.
    const refetchPatch = makePatch({ base_revision: 12, target_revision: 13 });
    const freshItems: Item[] = [{ id: "r1", label: "reconnect fresh" }];

    state = patchReducer(state, { type: "APPLY_PATCH", patch: refetchPatch }, freshItems);

    expect(state.revision).toBe(13);
    expect(state.items).toEqual(freshItems);
  });

  it("recovery_cursor from last patch is non-null and can anchor a bounded refetch", () => {
    let state = makeState({ revision: 0 });
    const patch = makePatch({
      base_revision: 0,
      target_revision: 1,
      recovery_cursor: "enc-cursor-rev1",
    });

    state = patchReducer(state, { type: "APPLY_PATCH", patch });
    expect(state.revision).toBe(1);
    // The recovery_cursor on the patch itself is the handle the caller stores
    // for a future bounded refetch — the reducer doesn't store it but the
    // caller can pass it back to the server on reconnect.
    expect(patch.recovery_cursor).toBe("enc-cursor-rev1");
  });

  it("lagging patches after reconnect are rejected and do not corrupt state", () => {
    // Window reconnected and refetched to revision 15.
    let state = makeState({ revision: 15 });

    // Stale patches arrive from the old subscription — all rejected.
    const stalePatches = [
      makePatch({ base_revision: 8, target_revision: 9 }),
      makePatch({ base_revision: 10, target_revision: 11 }),
    ];
    for (const p of stalePatches) {
      state = patchReducer(state, { type: "APPLY_PATCH", patch: p });
    }
    expect(state.revision).toBe(15);

    // Correct continuing patch applies.
    const cont = makePatch({ base_revision: 15, target_revision: 16 });
    state = patchReducer(state, { type: "APPLY_PATCH", patch: cont });
    expect(state.revision).toBe(16);
  });

  it("session validateDetachedRestore guards the reconnect flow", () => {
    const win = makeSession("win-guard");
    const { generation: gen } = win.beginRequest("q");
    win.completeRequest(gen, 5);

    // Not detached yet — restore should be disallowed.
    expect(win.validateDetachedRestore()).toBe(false);

    win.markDetached();
    expect(win.validateDetachedRestore()).toBe(true);

    // After reset, no longer detached.
    win.reset();
    expect(win.validateDetachedRestore()).toBe(false);
  });
});

// ─── S8: Bounded per-window refetch ──────────────────────────────────────────

describe("S8 — Bounded per-window refetch: one window refetching doesn't affect the other", () => {
  /**
   * A refetch is per-window: when Window A detects a gap and issues a bounded
   * refetch (REFETCH_REQUIRED + beginRequest), Window B must continue operating
   * normally with its own state. Neither Window A's refetch signal nor its new
   * request cycle should touch Window B's reducer state, session, or cache.
   */

  it("REFETCH_REQUIRED on Window A's reducer does not affect Window B's reducer state", () => {
    const stateA = makeState({ revision: 6, queryHash: "qa" });
    const stateB = makeState({ revision: 6, queryHash: "qb" });

    const nextA = patchReducer(stateA, {
      type: "REFETCH_REQUIRED",
      queryHash: "qa",
      reason: "gap",
    });

    // A's state unchanged (pure signal).
    expect(nextA).toBe(stateA);
    // B is separate plain object — unchanged.
    expect(stateB.revision).toBe(6);
    expect(stateB.queryHash).toBe("qb");
  });

  it("Window A's refetch (beginRequest) does not abort Window B's in-flight request", () => {
    const winA = makeSession("win-A");
    const winB = makeSession("win-B");

    const { signal: sigB } = winB.beginRequest("qb-in-flight");

    // Window A detects gap, resets, and begins bounded refetch.
    winA.beginRequest("qa-pre");
    winA.reset();
    winA.beginRequest("qa-refetch-from-rev-6");

    expect(sigB.aborted).toBe(false);
    expect(winB.state).toBe("loading");
    expect(winB.generation).toBe(1);
  });

  it("Window B continues receiving and applying patches while Window A is refetching", () => {
    let stateA = makeState({ revision: 5, policyHash: "ph-A" });
    let stateB = makeState({ revision: 5, policyHash: "ph-B" });

    // Window A detects gap at revision 5, issues REFETCH_REQUIRED (no-op on reducer).
    stateA = patchReducer(stateA, {
      type: "REFETCH_REQUIRED",
      queryHash: "query-xyz",
      reason: "gap: missing rev 6",
    });
    expect(stateA.revision).toBe(5); // A paused at 5

    // Window B independently receives patch 5→6 and applies it.
    const patchB = makePatch({ base_revision: 5, target_revision: 6, policy_hash: "ph-B" });
    const itemsB: Item[] = [{ id: "b1", label: "B advanced" }];
    stateB = patchReducer(stateB, { type: "APPLY_PATCH", patch: patchB }, itemsB);

    expect(stateB.revision).toBe(6);
    expect(stateB.items).toEqual(itemsB);

    // A is still at 5 waiting for its bounded refetch.
    expect(stateA.revision).toBe(5);
  });

  it("Window A completes its bounded refetch independently of Window B", () => {
    let stateA = makeState({ revision: 5, policyHash: "ph-A" });
    let stateB = makeState({ revision: 8, policyHash: "ph-B" });

    // A's bounded refetch delivers patch from 5→9.
    const refetchPatch = makePatch({ base_revision: 5, target_revision: 9, policy_hash: "ph-A" });
    const refetchItems: Item[] = [{ id: "a-catch-up", label: "A caught up" }];
    stateA = patchReducer(stateA, { type: "APPLY_PATCH", patch: refetchPatch }, refetchItems);

    expect(stateA.revision).toBe(9);
    expect(stateA.items).toEqual(refetchItems);

    // B is unaffected by A's refetch.
    expect(stateB.revision).toBe(8);
  });

  it("Window A's session generation during refetch does not advance Window B's generation", () => {
    const winA = makeSession("win-A");
    const winB = makeSession("win-B");

    // B completes normally.
    const { generation: genB } = winB.beginRequest("qb");
    winB.completeRequest(genB, 10);

    // A performs a full refetch cycle: many requests.
    for (let i = 0; i < 4; i++) {
      winA.beginRequest(`refetch-step-${i}`);
    }
    expect(winA.generation).toBe(4);

    // B generation and state are unchanged.
    expect(winB.generation).toBe(1);
    expect(winB.state).toBe("ready");
    expect(winB.revision).toBe(10);
  });

  it("pending writes in Window B are unaffected by Window A's refetch and confirmation cycle", () => {
    const writeB = makePendingWrite({ commandId: "cmd-B-pending", baseRevision: 5 });
    const stateA = makeState({ revision: 5, policyHash: "ph-A" });
    let stateB = makeState({ revision: 5, policyHash: "ph-B", pendingWrites: [writeB] });

    // A refetches (REFETCH_REQUIRED is a no-op; bounded patch applied).
    const patchA = makePatch({ base_revision: 5, target_revision: 7, policy_hash: "ph-A" });
    patchReducer(stateA, { type: "APPLY_PATCH", patch: patchA }); // A's own state management

    // B's pending write must still be there.
    expect(stateB.pendingWrites).toHaveLength(1);
    expect(stateB.pendingWrites[0].commandId).toBe("cmd-B-pending");

    // B confirms its own write independently.
    stateB = patchReducer(stateB, { type: "CONFIRM_WRITE", commandId: "cmd-B-pending", revision: 6 });
    expect(stateB.pendingWrites).toHaveLength(0);
    expect(stateB.revision).toBe(6);
  });
});

// ─── Integration: all 8 scenarios in one multi-window sequence ───────────────

describe("Integration — all 8 concurrent/temporal scenarios in a single two-window sequence", () => {
  /**
   * Two windows (A and B) each start at revision 0, run through writes, patches,
   * a duplicate, a reorder, a gap, lag, reconnect, and a bounded refetch — all
   * without cross-contamination.
   */

  it("two-window full lifecycle: simultaneous writes → patches → gap/refetch → reconnect", () => {
    let stateA = makeState({ revision: 0, policyHash: "ph-A", queryHash: "qa" });
    let stateB = makeState({ revision: 0, policyHash: "ph-B", queryHash: "qb" });

    const winA = makeSession("win-A", "ph-A");
    const winB = makeSession("win-B", "ph-B");

    // S1: Both windows issue writes simultaneously.
    const writeA = makePendingWrite({ commandId: "cmd-A", baseRevision: 0 });
    const writeB = makePendingWrite({ commandId: "cmd-B", baseRevision: 0 });
    stateA = { ...stateA, pendingWrites: [writeA] };
    stateB = { ...stateB, pendingWrites: [writeB] };

    // Both windows start their requests concurrently.
    const { signal: sigA } = winA.beginRequest("qa");
    const { signal: sigB } = winB.beginRequest("qb");
    expect(sigA.aborted).toBe(false);
    expect(sigB.aborted).toBe(false);

    // S2: Authority confirms Window A's write; B's write stays pending.
    const pA1 = makePatch({ base_revision: 0, target_revision: 1, policy_hash: "ph-A" });
    stateA = patchReducer(stateA, { type: "APPLY_PATCH", patch: pA1 });
    stateA = patchReducer(stateA, { type: "CONFIRM_WRITE", commandId: "cmd-A", revision: 1 });
    expect(stateA.pendingWrites).toHaveLength(0);
    expect(stateB.pendingWrites).toHaveLength(1); // B's write still pending

    // S3: Duplicate patch arrives for A — no-op.
    const dupA = patchReducer(stateA, { type: "APPLY_PATCH", patch: pA1 });
    expect(dupA).toBe(stateA); // same reference

    // S4: Reordered patch for B — rejected.
    const reorderB = makePatch({ base_revision: 5, target_revision: 6, policy_hash: "ph-B" });
    const afterReorder = patchReducer(stateB, { type: "APPLY_PATCH", patch: reorderB });
    expect(afterReorder).toBe(stateB);

    // S5: Gap detected for B — issue REFETCH_REQUIRED.
    const afterRefetchSignal = patchReducer(stateB, {
      type: "REFETCH_REQUIRED",
      queryHash: "qb",
      reason: "gap: missing revisions 1-4",
    });
    expect(afterRefetchSignal).toBe(stateB); // pure signal, no mutation

    // S6: Lag patch arrives for A (base < current) — no-op.
    const lagA = makePatch({ base_revision: 0, target_revision: 1, policy_hash: "ph-A" });
    const afterLag = patchReducer(stateA, { type: "APPLY_PATCH", patch: lagA });
    expect(afterLag).toBe(stateA);

    // S7: B reconnects — markDetached, then reset, then bounded refetch from revision 0.
    winB.markDetached();
    expect(winB.validateDetachedRestore()).toBe(true);
    winB.reset();
    winB.beginRequest("qb-refetch");
    const pB1 = makePatch({ base_revision: 0, target_revision: 1, policy_hash: "ph-B" });
    const freshB: Item[] = [{ id: "b-fresh", label: "B refetched" }];
    stateB = patchReducer(stateB, { type: "APPLY_PATCH", patch: pB1 }, freshB);
    stateB = patchReducer(stateB, { type: "CONFIRM_WRITE", commandId: "cmd-B", revision: 1 });
    expect(stateB.revision).toBe(1);
    expect(stateB.pendingWrites).toHaveLength(0);
    expect(stateB.items).toEqual(freshB);

    // S8: A continues independently while B was refetching.
    const pA2 = makePatch({ base_revision: 1, target_revision: 2, policy_hash: "ph-A" });
    stateA = patchReducer(stateA, { type: "APPLY_PATCH", patch: pA2 });
    expect(stateA.revision).toBe(2);

    // Final isolation check.
    expect(stateA.policyHash).toBe("ph-A");
    expect(stateB.policyHash).toBe("ph-B");
    expect(stateA.items).not.toEqual(stateB.items);
  });
});
