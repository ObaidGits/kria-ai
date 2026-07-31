/**
 * policyIdentityChange.test.ts
 *
 * Task 5.2.4 — Policy/capability identity change discards incompatible
 * in-flight responses, caches, pending writes, and traces across affected
 * windows only (not unaffected windows).
 *
 * Four scenario groups:
 *   PC1 — In-flight response discard: patchReducer rejects patches whose
 *          policy_hash !== state.policyHash after a policy change on Window A.
 *   PC2 — Cache invalidation: SnapshotCache.invalidateByPolicy clears
 *          old-policy entries for Window A only; Window B's cache is untouched.
 *   PC3 — Pending writes stranded: old-policy pending writes cannot be
 *          confirmed under the new policyHash; they are logically orphaned.
 *   PC4 — Unaffected windows: Window B with a different (unchanged) policy
 *          is not touched by Window A's policy change in any of the above.
 *
 * Requirements: MGR-004 (scope/sensitivity isolation, AC 6), MGR-008
 * (revision/patch consistency), F4.1 (window session isolation).
 */

import { describe, it, expect } from "vitest";
import {
  patchReducer,
  type ReducerState,
  type AuthorityPatch,
  type PendingWrite,
} from "./patchReducer";
import { SnapshotCache, type SnapshotCacheKey } from "./snapshotCache";
import { MemoryWindowSessionV2, type WindowSessionConfig } from "./windowSession";

// ─── Shared item type ─────────────────────────────────────────────────────────

interface Item {
  id: string;
  label: string;
}

// ─── Helper factories ─────────────────────────────────────────────────────────

function makeState(overrides: Partial<ReducerState<Item>> = {}): ReducerState<Item> {
  return {
    items: [],
    revision: 0,
    pendingWrites: [],
    schemaVersion: "2.0",
    policyHash: "policy-v1",
    queryHash: "qhash-default",
    ...overrides,
  };
}

function makePatch(overrides: Partial<AuthorityPatch> = {}): AuthorityPatch {
  return {
    base_revision: 0,
    target_revision: 1,
    changes: [],
    invalidations: [],
    recovery_cursor: null,
    schema_version: "2.0",
    policy_hash: "policy-v1",
    ...overrides,
  };
}

function makePendingWrite(overrides: Partial<PendingWrite> = {}): PendingWrite {
  return {
    commandId: "cmd-default",
    operationName: "test-op",
    baseRevision: 0,
    ...overrides,
  };
}

function makeCacheKey(overrides: Partial<SnapshotCacheKey> = {}): SnapshotCacheKey {
  return {
    schemaVersion: "2.0",
    revision: 1,
    policyHash: "policy-v1",
    queryHash: "qhash-default",
    ...overrides,
  };
}

function makeSession(instanceId: string, policyHash: string): MemoryWindowSessionV2 {
  const cfg: WindowSessionConfig = { instanceId, policyHash, schemaVersion: "2.0" };
  return new MemoryWindowSessionV2(cfg);
}

// ─────────────────────────────────────────────────────────────────────────────
// PC1 — In-flight response discard
// patchReducer rejects patches with old policy_hash after Window A's policy
// identity changes (state.policyHash reflects new policy; patch has old hash).
// ─────────────────────────────────────────────────────────────────────────────

describe("PC1 — In-flight response discard: patch with old policy_hash is rejected after policy change", () => {
  /**
   * When a policy change occurs on Window A, any in-flight response that was
   * produced under the old policy arrives as a patch with old policy_hash.
   * The reducer's Guard 3 ensures these patches are silently discarded.
   * New patches produced under the new policy are accepted.
   */

  it("patch with old policy_hash is rejected (returns same state reference)", () => {
    // Window A now operates under new policy; state reflects the new hash.
    const state = makeState({ policyHash: "policy-v2", revision: 5 });

    // In-flight patch produced before the policy change — still has old hash.
    const oldPolicyPatch = makePatch({
      base_revision: 5,
      target_revision: 6,
      policy_hash: "policy-v1",
    });

    const next = patchReducer(state, { type: "APPLY_PATCH", patch: oldPolicyPatch });

    // Must be a pure no-op — same reference, no state mutation.
    expect(next).toBe(state);
    expect(next.revision).toBe(5);
    expect(next.policyHash).toBe("policy-v2");
  });

  it("patch with new policy_hash is accepted after policy change", () => {
    // Window A state was refreshed under the new policy.
    const state = makeState({ policyHash: "policy-v2", revision: 5 });

    const newPolicyPatch = makePatch({
      base_revision: 5,
      target_revision: 6,
      policy_hash: "policy-v2",
    });
    const newItems: Item[] = [{ id: "n1", label: "new-policy result" }];

    const next = patchReducer(state, { type: "APPLY_PATCH", patch: newPolicyPatch }, newItems);

    expect(next.revision).toBe(6);
    expect(next.policyHash).toBe("policy-v2");
    expect(next.items).toEqual(newItems);
  });

  it("multiple in-flight old-policy patches are all rejected; state does not advance", () => {
    const state = makeState({ policyHash: "policy-v2", revision: 3 });

    const oldPatches = [
      makePatch({ base_revision: 3, target_revision: 4, policy_hash: "policy-v1" }),
      makePatch({ base_revision: 4, target_revision: 5, policy_hash: "policy-v1" }),
      makePatch({ base_revision: 5, target_revision: 6, policy_hash: "policy-v1" }),
    ];

    let current = state;
    for (const patch of oldPatches) {
      const next = patchReducer(current, { type: "APPLY_PATCH", patch });
      expect(next).toBe(current); // every old-policy patch is a no-op
    }
    expect(current.revision).toBe(3); // revision never advanced
  });

  it("old-policy patch does not overwrite items accumulated under new policy", () => {
    const existingItems: Item[] = [{ id: "new-1", label: "new policy item" }];
    const state = makeState({ policyHash: "policy-v2", revision: 7, items: existingItems });

    const staleItems: Item[] = [{ id: "old-1", label: "stale policy item" }];
    const oldPatch = makePatch({
      base_revision: 7,
      target_revision: 8,
      policy_hash: "policy-v1",
    });

    const next = patchReducer(state, { type: "APPLY_PATCH", patch: oldPatch }, staleItems);

    expect(next).toBe(state);
    expect(next.items).toBe(existingItems); // stale items must not replace current
  });

  it("windowSession.guardPolicy correctly rejects old policy hash and accepts new one", () => {
    // Window A was created with old policy; after a policy change a new session
    // would be created with the new policy. guardPolicy validates the hash.
    const winA = makeSession("win-A", "policy-v2");

    // Old-policy response must be rejected.
    expect(winA.guardPolicy("policy-v1")).toBe(false);
    // New-policy response must be accepted.
    expect(winA.guardPolicy("policy-v2")).toBe(true);
  });
});

// ─────────────────────────────────────────────────────────────────────────────
// PC2 — Cache invalidation
// SnapshotCache.invalidateByPolicy clears old-policy entries for the affected
// window (A) but leaves Window B's independent cache completely untouched.
// ─────────────────────────────────────────────────────────────────────────────

describe("PC2 — Cache invalidation: old-policy entries cleared for Window A; Window B's cache untouched", () => {
  /**
   * Each window owns its own SnapshotCache instance. When Window A's policy
   * changes, the orchestrating layer calls invalidateByPolicy(oldHash) on A's
   * cache. Window B's cache is a different object and must not be touched.
   */

  it("invalidateByPolicy on Window A's cache clears all entries with the old policy hash", () => {
    const cacheA = new SnapshotCache<string>(16);

    // Multiple entries cached under old policy for different queries/revisions.
    cacheA.set(makeCacheKey({ policyHash: "policy-v1", queryHash: "q1", revision: 1 }), "snap-q1");
    cacheA.set(makeCacheKey({ policyHash: "policy-v1", queryHash: "q2", revision: 2 }), "snap-q2");
    cacheA.set(makeCacheKey({ policyHash: "policy-v1", queryHash: "q3", revision: 3 }), "snap-q3");

    cacheA.invalidateByPolicy("policy-v1");

    expect(cacheA.size).toBe(0);
    expect(cacheA.get(makeCacheKey({ policyHash: "policy-v1", queryHash: "q1", revision: 1 }))).toBeUndefined();
    expect(cacheA.get(makeCacheKey({ policyHash: "policy-v1", queryHash: "q2", revision: 2 }))).toBeUndefined();
    expect(cacheA.get(makeCacheKey({ policyHash: "policy-v1", queryHash: "q3", revision: 3 }))).toBeUndefined();
  });

  it("Window B's cache is not touched when Window A's policy changes", () => {
    const cacheA = new SnapshotCache<string>(16);
    const cacheB = new SnapshotCache<string>(16);

    // Window A: entries under old policy.
    cacheA.set(makeCacheKey({ policyHash: "policy-v1", queryHash: "qA1" }), "A-old-1");
    cacheA.set(makeCacheKey({ policyHash: "policy-v1", queryHash: "qA2" }), "A-old-2");

    // Window B: entries under its own (different, unchanged) policy.
    cacheB.set(makeCacheKey({ policyHash: "policy-B", queryHash: "qB1" }), "B-snap-1");
    cacheB.set(makeCacheKey({ policyHash: "policy-B", queryHash: "qB2" }), "B-snap-2");

    // Policy change on Window A — invalidate A's old-policy entries.
    cacheA.invalidateByPolicy("policy-v1");

    // A's cache is cleared.
    expect(cacheA.size).toBe(0);

    // B's cache is completely untouched.
    expect(cacheB.size).toBe(2);
    expect(cacheB.get(makeCacheKey({ policyHash: "policy-B", queryHash: "qB1" }))).toBe("B-snap-1");
    expect(cacheB.get(makeCacheKey({ policyHash: "policy-B", queryHash: "qB2" }))).toBe("B-snap-2");
  });

  it("new-policy entries can be added to Window A's cache after invalidation", () => {
    const cacheA = new SnapshotCache<string>(16);

    // Populate under old policy, then invalidate.
    cacheA.set(makeCacheKey({ policyHash: "policy-v1", queryHash: "q1" }), "old-snap");
    cacheA.invalidateByPolicy("policy-v1");
    expect(cacheA.size).toBe(0);

    // After policy change, new-policy entries are stored and retrievable.
    cacheA.set(makeCacheKey({ policyHash: "policy-v2", queryHash: "q1" }), "new-snap");
    expect(cacheA.size).toBe(1);
    expect(cacheA.get(makeCacheKey({ policyHash: "policy-v2", queryHash: "q1" }))).toBe("new-snap");
  });

  it("mixed-policy cache: only old-policy entries are cleared, newer-policy entries survive", () => {
    const cacheA = new SnapshotCache<string>(16);

    // Some entries under old policy, some already under new policy (e.g. partially migrated).
    cacheA.set(makeCacheKey({ policyHash: "policy-v1", queryHash: "old-q" }), "old-snap");
    cacheA.set(makeCacheKey({ policyHash: "policy-v2", queryHash: "new-q" }), "new-snap");
    expect(cacheA.size).toBe(2);

    // Invalidate only the old policy.
    cacheA.invalidateByPolicy("policy-v1");

    expect(cacheA.size).toBe(1);
    expect(cacheA.get(makeCacheKey({ policyHash: "policy-v2", queryHash: "new-q" }))).toBe("new-snap");
    expect(cacheA.get(makeCacheKey({ policyHash: "policy-v1", queryHash: "old-q" }))).toBeUndefined();
  });

  it("invalidating Window A's policy hash has no side effect on Window B's cache size", () => {
    const cacheA = new SnapshotCache<string>(8);
    const cacheB = new SnapshotCache<string>(8);

    cacheA.set(makeCacheKey({ policyHash: "policy-v1", queryHash: "qa" }), "vA");

    cacheB.set(makeCacheKey({ policyHash: "policy-B", queryHash: "q1" }), "vB1");
    cacheB.set(makeCacheKey({ policyHash: "policy-B", queryHash: "q2" }), "vB2");
    cacheB.set(makeCacheKey({ policyHash: "policy-B", queryHash: "q3" }), "vB3");

    // A's policy changes.
    cacheA.invalidateByPolicy("policy-v1");

    expect(cacheA.size).toBe(0);
    expect(cacheB.size).toBe(3); // B is completely unaffected
  });
});

// ─────────────────────────────────────────────────────────────────────────────
// PC3 — Pending writes stranded
// Pending writes based on the old policy are logically orphaned after a policy
// change: they were submitted under old policy context and cannot be confirmed
// under the new policyHash guard. Writes for the new policy proceed normally.
// ─────────────────────────────────────────────────────────────────────────────

describe("PC3 — Pending writes: old-policy pending writes are orphaned after policy change", () => {
  /**
   * A pending write is issued at some baseRevision under old policy context.
   * After a policy change the window operates under new policy. Any confirmation
   * arriving for the old-policy write would still remove the pending write from
   * the reducer (CONFIRM_WRITE only matches by commandId), but the correct
   * orchestration pattern is to rollback orphaned writes and re-issue under the
   * new policy. We verify the reducer behaves correctly in each case.
   */

  it("old-policy pending write stays in state after a new-policy patch is applied", () => {
    // State after policy change: policyHash updated to v2, but old write is still pending.
    const oldWrite = makePendingWrite({ commandId: "cmd-old", baseRevision: 5 });
    let state = makeState({ policyHash: "policy-v2", revision: 5, pendingWrites: [oldWrite] });

    // New-policy patch arrives — applies successfully.
    const newPatch = makePatch({
      base_revision: 5,
      target_revision: 6,
      policy_hash: "policy-v2",
    });
    state = patchReducer(state, { type: "APPLY_PATCH", patch: newPatch });

    expect(state.revision).toBe(6);
    // Old-policy pending write is still tracked — it has NOT been auto-removed.
    expect(state.pendingWrites).toHaveLength(1);
    expect(state.pendingWrites[0].commandId).toBe("cmd-old");
  });

  it("old-policy patch does NOT advance revision even if an old-policy pending write is present", () => {
    const oldWrite = makePendingWrite({ commandId: "cmd-old", baseRevision: 5 });
    const state = makeState({ policyHash: "policy-v2", revision: 5, pendingWrites: [oldWrite] });

    // Old-policy confirmation patch — rejected by policy guard.
    const oldPatch = makePatch({
      base_revision: 5,
      target_revision: 6,
      policy_hash: "policy-v1",
    });

    const next = patchReducer(state, { type: "APPLY_PATCH", patch: oldPatch });

    expect(next).toBe(state); // complete no-op
    expect(next.revision).toBe(5);
    expect(next.pendingWrites).toHaveLength(1);
  });

  it("ROLLBACK_WRITE correctly cleans up an orphaned old-policy pending write", () => {
    const preOptimisticItems: Item[] = [{ id: "pre", label: "pre-write state" }];
    const oldWrite = makePendingWrite({
      commandId: "cmd-orphaned",
      baseRevision: 5,
      optimisticItems: preOptimisticItems,
    });
    let state = makeState({
      policyHash: "policy-v2",
      revision: 6,
      items: [{ id: "opt", label: "optimistic" }],
      pendingWrites: [oldWrite],
    });

    // Orchestrator rolls back the orphaned write after detecting policy mismatch.
    state = patchReducer(state, {
      type: "ROLLBACK_WRITE",
      commandId: "cmd-orphaned",
      reason: "policy identity change; write cannot be confirmed",
    });

    expect(state.pendingWrites).toHaveLength(0);
    // Items reverted to pre-optimistic snapshot.
    expect(state.items).toEqual(preOptimisticItems);
    expect(state.revision).toBe(6); // revision is not affected by rollback
  });

  it("new-policy pending write is correctly tracked and confirmed after policy change", () => {
    const newWrite = makePendingWrite({ commandId: "cmd-new-policy", baseRevision: 6 });
    let state = makeState({ policyHash: "policy-v2", revision: 6, pendingWrites: [newWrite] });

    // New-policy confirmation arrives — removes the write and may advance revision.
    state = patchReducer(state, {
      type: "CONFIRM_WRITE",
      commandId: "cmd-new-policy",
      revision: 7,
    });

    expect(state.pendingWrites).toHaveLength(0);
    expect(state.revision).toBe(7);
  });

  it("windowSession.guardPolicy correctly gates old-policy writes", () => {
    // Session created for the new policy (post-change).
    const session = makeSession("win-A", "policy-v2");

    // An old-policy write confirmation would arrive with old policy hash.
    const oldPolicyHash = "policy-v1";
    const newPolicyHash = "policy-v2";

    expect(session.guardPolicy(oldPolicyHash)).toBe(false); // old write rejected
    expect(session.guardPolicy(newPolicyHash)).toBe(true);  // new write accepted
  });
});

// ─────────────────────────────────────────────────────────────────────────────
// PC4 — Unaffected windows
// Window B with a different policy hash is not affected by any of the above
// policy change operations on Window A.
// ─────────────────────────────────────────────────────────────────────────────

describe("PC4 — Unaffected windows: Window B with a different policy is not touched by Window A's policy change", () => {
  /**
   * Window A undergoes a policy identity change (old policy → new policy).
   * Window B has always operated under its own independent policy (policy-B).
   * All operations on Window A (patch rejection, cache invalidation, write
   * orphaning, rollback) must leave Window B's reducer state, cache, and
   * session completely unchanged.
   */

  it("Window B's reducer continues accepting its own policy patches while Window A rejects old-policy patches", () => {
    // Window A changed policy from v1 to v2; now rejects v1 patches.
    const stateA = makeState({ policyHash: "policy-v2", revision: 5 });
    let stateB = makeState({ policyHash: "policy-B", revision: 5 });

    const oldPatchForA = makePatch({
      base_revision: 5,
      target_revision: 6,
      policy_hash: "policy-v1",
    });
    const patchForB = makePatch({
      base_revision: 5,
      target_revision: 6,
      policy_hash: "policy-B",
    });

    const itemsB: Item[] = [{ id: "b1", label: "B result" }];

    // A rejects its in-flight old-policy patch.
    const nextA = patchReducer(stateA, { type: "APPLY_PATCH", patch: oldPatchForA });
    expect(nextA).toBe(stateA); // no-op

    // B applies its patch normally — unaffected.
    stateB = patchReducer(stateB, { type: "APPLY_PATCH", patch: patchForB }, itemsB);
    expect(stateB.revision).toBe(6);
    expect(stateB.items).toEqual(itemsB);
  });

  it("Window B's cache is not cleared when Window A's policy is invalidated", () => {
    const cacheA = new SnapshotCache<string>(16);
    const cacheB = new SnapshotCache<string>(16);

    cacheA.set(makeCacheKey({ policyHash: "policy-v1", queryHash: "qA1" }), "A-v1-snap");
    cacheB.set(makeCacheKey({ policyHash: "policy-B", queryHash: "qB1" }), "B-snap-1");
    cacheB.set(makeCacheKey({ policyHash: "policy-B", queryHash: "qB2" }), "B-snap-2");

    // Window A policy change — invalidate A's old entries.
    cacheA.invalidateByPolicy("policy-v1");

    // A is empty.
    expect(cacheA.size).toBe(0);

    // B's entries are intact.
    expect(cacheB.size).toBe(2);
    expect(cacheB.get(makeCacheKey({ policyHash: "policy-B", queryHash: "qB1" }))).toBe("B-snap-1");
    expect(cacheB.get(makeCacheKey({ policyHash: "policy-B", queryHash: "qB2" }))).toBe("B-snap-2");
  });

  it("Window B's pending writes are not orphaned by Window A's policy change", () => {
    const writeA = makePendingWrite({ commandId: "cmd-A-orphaned", baseRevision: 3 });
    const writeB = makePendingWrite({ commandId: "cmd-B-active", baseRevision: 3 });

    // A's state after policy change (policyHash updated to v2).
    let stateA = makeState({ policyHash: "policy-v2", revision: 3, pendingWrites: [writeA] });
    let stateB = makeState({ policyHash: "policy-B", revision: 3, pendingWrites: [writeB] });

    // Orchestrator rolls back A's orphaned write.
    stateA = patchReducer(stateA, {
      type: "ROLLBACK_WRITE",
      commandId: "cmd-A-orphaned",
      reason: "policy identity changed",
    });
    expect(stateA.pendingWrites).toHaveLength(0);

    // B's pending write is untouched.
    expect(stateB.pendingWrites).toHaveLength(1);
    expect(stateB.pendingWrites[0].commandId).toBe("cmd-B-active");

    // B can still confirm its write normally.
    stateB = patchReducer(stateB, {
      type: "CONFIRM_WRITE",
      commandId: "cmd-B-active",
      revision: 4,
    });
    expect(stateB.pendingWrites).toHaveLength(0);
    expect(stateB.revision).toBe(4);
  });

  it("Window B's session guardPolicy is not affected by Window A's policy change", () => {
    const winA = makeSession("win-A", "policy-v2"); // post-change
    const winB = makeSession("win-B", "policy-B");

    // A's old policy hash must not pass B's guard.
    expect(winB.guardPolicy("policy-v1")).toBe(false);
    // A's new policy hash also must not pass B's guard.
    expect(winB.guardPolicy("policy-v2")).toBe(false);
    // Only B's own policy hash passes.
    expect(winB.guardPolicy("policy-B")).toBe(true);

    // A's new session correctly accepts new policy only.
    expect(winA.guardPolicy("policy-v1")).toBe(false);
    expect(winA.guardPolicy("policy-v2")).toBe(true);
  });

  it("Window B's session state is unchanged when Window A is reset for policy change", () => {
    const winA = makeSession("win-A", "policy-v1");
    const winB = makeSession("win-B", "policy-B");

    // B completes a request.
    const { generation: genB } = winB.beginRequest("destination:recall");
    winB.completeRequest(genB, 15);

    // A policy change: reset and re-initialize with new policy.
    winA.beginRequest("policy-reload");
    winA.reset(); // A resets to idle

    expect(winA.state).toBe("idle");
    expect(winA.revision).toBe(0);

    // B is completely unaffected.
    expect(winB.state).toBe("ready");
    expect(winB.revision).toBe(15);
    expect(winB.generation).toBe(1);
  });
});

// ─────────────────────────────────────────────────────────────────────────────
// Integration — Full policy change lifecycle across two windows
// ─────────────────────────────────────────────────────────────────────────────

describe("Integration — policy change on Window A: full lifecycle, Window B unaffected throughout", () => {
  /**
   * End-to-end scenario:
   *   1. Both windows start with their own policies and populate caches + pending writes.
   *   2. Window A undergoes a policy identity change (v1 → v2).
   *   3. Old-policy in-flight patches for A are rejected; cache invalidated; writes rolled back.
   *   4. A re-initializes under new policy and resumes normal operation.
   *   5. Throughout all steps, Window B's state, cache, and session are unchanged.
   */

  it("full policy change lifecycle on Window A leaves Window B intact at every stage", () => {
    // ── Setup ──────────────────────────────────────────────────────────────────

    const sessionA = makeSession("win-A", "policy-v1");
    const sessionB = makeSession("win-B", "policy-B");

    const cacheA = new SnapshotCache<string>(16);
    const cacheB = new SnapshotCache<string>(16);

    const preOptimisticA: Item[] = [{ id: "pre-A", label: "pre-A" }];
    const pendingWriteA = makePendingWrite({ commandId: "cmd-A-inflight", baseRevision: 4, optimisticItems: preOptimisticA });
    const pendingWriteB = makePendingWrite({ commandId: "cmd-B-active", baseRevision: 4 });

    // stateA has already been updated to policyHash: "policy-v2" by the orchestrator
    // upon detecting the policy identity change. In-flight patches still carry the old
    // policy_hash: "policy-v1" and will be rejected by the policyHash guard.
    let stateA = makeState({ policyHash: "policy-v2", revision: 4, pendingWrites: [pendingWriteA] });
    let stateB = makeState({ policyHash: "policy-B", revision: 4, pendingWrites: [pendingWriteB] });

    // Populate caches.
    cacheA.set(makeCacheKey({ policyHash: "policy-v1", queryHash: "qA1", revision: 4 }), "A-v1-snap1");
    cacheA.set(makeCacheKey({ policyHash: "policy-v1", queryHash: "qA2", revision: 4 }), "A-v1-snap2");
    cacheB.set(makeCacheKey({ policyHash: "policy-B", queryHash: "qB1", revision: 4 }), "B-snap1");
    cacheB.set(makeCacheKey({ policyHash: "policy-B", queryHash: "qB2", revision: 4 }), "B-snap2");

    // Complete requests for both sessions.
    const { generation: genA } = sessionA.beginRequest("overview");
    sessionA.completeRequest(genA, 4);
    const { generation: genB } = sessionB.beginRequest("recall");
    sessionB.completeRequest(genB, 4);

    // Verify initial state (A already reflects new policy after detection).
    expect(stateA.policyHash).toBe("policy-v2");
    expect(stateB.policyHash).toBe("policy-B");
    expect(cacheA.size).toBe(2);
    expect(cacheB.size).toBe(2);

    // ── Step 1: Policy change detected on Window A ─────────────────────────

    // In-flight old-policy patch arrives — rejected.
    const oldPatch = makePatch({ base_revision: 4, target_revision: 5, policy_hash: "policy-v1" });
    const nextA = patchReducer(stateA, { type: "APPLY_PATCH", patch: oldPatch });
    expect(nextA).toBe(stateA); // no-op

    // Window B independently receives and applies its own patch — unaffected.
    const patchB = makePatch({ base_revision: 4, target_revision: 5, policy_hash: "policy-B" });
    const itemsB: Item[] = [{ id: "b-new", label: "B advanced" }];
    stateB = patchReducer(stateB, { type: "APPLY_PATCH", patch: patchB }, itemsB);
    expect(stateB.revision).toBe(5);

    // ── Step 2: Invalidate A's cache for old policy ────────────────────────

    cacheA.invalidateByPolicy("policy-v1");
    expect(cacheA.size).toBe(0);

    // B's cache is untouched.
    expect(cacheB.size).toBe(2);

    // ── Step 3: Roll back A's orphaned pending write ───────────────────────

    stateA = patchReducer(stateA, {
      type: "ROLLBACK_WRITE",
      commandId: "cmd-A-inflight",
      reason: "policy identity change",
    });
    expect(stateA.pendingWrites).toHaveLength(0);
    expect(stateA.items).toEqual(preOptimisticA); // reverted to pre-optimistic

    // B's pending write is still active.
    expect(stateB.pendingWrites).toHaveLength(1);
    expect(stateB.pendingWrites[0].commandId).toBe("cmd-B-active");

    // ── Step 4: Window A re-initializes under new policy ──────────────────

    // New state for A under new policy.
    stateA = makeState({ policyHash: "policy-v2", revision: 5 });

    // New-policy patch applies correctly.
    const newPatch = makePatch({ base_revision: 5, target_revision: 6, policy_hash: "policy-v2" });
    const newItemsA: Item[] = [{ id: "a-new", label: "A new policy item" }];
    stateA = patchReducer(stateA, { type: "APPLY_PATCH", patch: newPatch }, newItemsA);

    expect(stateA.revision).toBe(6);
    expect(stateA.items).toEqual(newItemsA);

    // ── Step 5: Verify Window B is intact throughout ───────────────────────

    expect(stateB.revision).toBe(5);
    expect(stateB.policyHash).toBe("policy-B");
    expect(stateB.pendingWrites).toHaveLength(1);
    expect(cacheB.size).toBe(2);
    expect(sessionB.state).toBe("ready");
    expect(sessionB.revision).toBe(4);

    // B can still confirm its write.
    stateB = patchReducer(stateB, {
      type: "CONFIRM_WRITE",
      commandId: "cmd-B-active",
      revision: 6,
    });
    expect(stateB.pendingWrites).toHaveLength(0);
    expect(stateB.revision).toBe(6);
  });
});
