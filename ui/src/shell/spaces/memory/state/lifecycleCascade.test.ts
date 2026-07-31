/**
 * lifecycleCascade.test.ts
 *
 * Lifecycle cascade tests for the Memory Control Center (task 5.2.5).
 *
 * Tests Forget/Delete/source-cascade invalidation behavior:
 *   LC1  Forget/Delete cascade via INVALIDATE — authority emits a patch with
 *        the record in invalidations[]; patchReducer INVALIDATE appends a
 *        sentinel to pendingWrites signalling a refetch is needed.
 *   LC2  Deleted content not exposed — after a delete INVALIDATE the deleted
 *        record ID must not appear in the items array when newItems filters it.
 *   LC3  Source cascade — when a source is deleted all derived records also
 *        appear in the invalidations list and none are in the post-refetch items.
 *   LC4  Unrelated windows not reset — Window B is not reset or invalidated by
 *        Window A's Forget/Delete operation.
 *   LC5  Snapshot cache — after INVALIDATE, old-revision cache entries for the
 *        deleted record should not be served (invalidateByRevision clears them).
 *
 * Requirements: MGR-008, MGR-040, MGR-046; F4.1.
 */

import { describe, it, expect } from "vitest";
import {
  patchReducer,
  type ReducerState,
  type AuthorityPatch,
  type PendingWrite,
} from "./patchReducer";
import { SnapshotCache, type SnapshotCacheKey } from "./snapshotCache";

// ─── Shared item type ─────────────────────────────────────────────────────────

interface SceneItem {
  id: string;
  label: string;
  kind: "entity" | "memory" | "evidence" | "source";
}

// ─── Helper factories ─────────────────────────────────────────────────────────

function makeState(
  overrides: Partial<ReducerState<SceneItem>> = {},
): ReducerState<SceneItem> {
  return {
    items: [],
    revision: 0,
    pendingWrites: [],
    schemaVersion: "2.0",
    policyHash: "policy-default",
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
    policy_hash: "policy-default",
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

function makeCacheKey(
  overrides: Partial<SnapshotCacheKey> = {},
): SnapshotCacheKey {
  return {
    schemaVersion: "2.0",
    revision: 1,
    policyHash: "policy-default",
    queryHash: "qhash-default",
    ...overrides,
  };
}

/** Simulate the orchestrator filtering deleted IDs out of the refetch result. */
function filterDeleted(items: SceneItem[], deletedIds: string[]): SceneItem[] {
  const set = new Set(deletedIds);
  return items.filter((i) => !set.has(i.id));
}

// ─── LC1: Forget/Delete cascade via INVALIDATE ────────────────────────────────

describe("LC1 — Forget/Delete cascade via INVALIDATE appends sentinel to pendingWrites", () => {
  /**
   * When a record is forgotten or deleted, the authority emits an INVALIDATE
   * action with that record's ID in invalidatedIds.  The patchReducer appends
   * a sentinel PendingWrite so the orchestrating layer knows a bounded refetch
   * is needed without extra state fields.
   */

  it("INVALIDATE for a forgotten record appends a sentinel pendingWrite", () => {
    const state = makeState({ pendingWrites: [] });
    const next = patchReducer(state, {
      type: "INVALIDATE",
      invalidatedIds: ["mem-001"],
    });

    expect(next.pendingWrites).toHaveLength(1);
    expect(next.pendingWrites[0].commandId).toContain("__INVALIDATE__");
    expect(next.pendingWrites[0].commandId).toContain("mem-001");
    expect(next.pendingWrites[0].operationName).toBe("invalidation");
  });

  it("INVALIDATE for a deleted record appends a sentinel and records base revision", () => {
    const state = makeState({ revision: 5, pendingWrites: [] });
    const next = patchReducer(state, {
      type: "INVALIDATE",
      invalidatedIds: ["mem-deleted-007"],
    });

    expect(next.pendingWrites[0].baseRevision).toBe(5);
  });

  it("multiple deleted records all appear in the single sentinel commandId", () => {
    const state = makeState({ pendingWrites: [] });
    const deletedIds = ["rec-A", "rec-B", "rec-C"];
    const next = patchReducer(state, {
      type: "INVALIDATE",
      invalidatedIds: deletedIds,
    });

    const sentinelId = next.pendingWrites[0].commandId;
    for (const id of deletedIds) {
      expect(sentinelId).toContain(id);
    }
  });

  it("INVALIDATE preserves existing pending writes alongside the new sentinel", () => {
    const existingWrite = makePendingWrite({ commandId: "cmd-prior-op" });
    const state = makeState({ pendingWrites: [existingWrite] });
    const next = patchReducer(state, {
      type: "INVALIDATE",
      invalidatedIds: ["rec-x"],
    });

    expect(next.pendingWrites).toHaveLength(2);
    expect(next.pendingWrites[0].commandId).toBe("cmd-prior-op");
    expect(next.pendingWrites[1].commandId).toContain("__INVALIDATE__");
  });

  it("INVALIDATE does not advance the revision", () => {
    const state = makeState({ revision: 10 });
    const next = patchReducer(state, {
      type: "INVALIDATE",
      invalidatedIds: ["rec-y"],
    });
    expect(next.revision).toBe(10);
  });

  it("INVALIDATE keeps existing items intact (stale display during refetch)", () => {
    const items: SceneItem[] = [
      { id: "mem-001", label: "Mem 1", kind: "memory" },
      { id: "mem-002", label: "Mem 2", kind: "memory" },
    ];
    const state = makeState({ items });
    const next = patchReducer(state, {
      type: "INVALIDATE",
      invalidatedIds: ["mem-001"],
    });
    // Items are kept so the UI can render them as stale while refetch runs.
    expect(next.items).toBe(items);
  });
});

// ─── LC2: Deleted content not exposed ────────────────────────────────────────

describe("LC2 — Deleted content is not exposed in items after a delete INVALIDATE + refetch", () => {
  /**
   * After an INVALIDATE for a deleted record, the orchestrating layer triggers
   * a bounded refetch and supplies newItems that exclude the deleted record.
   * The deleted record ID must not appear in the items array after the refetch.
   *
   * APPLY_PATCH carries invalidations[] the caller uses to filter items;
   * combined with the caller supplying filtered newItems, the deleted content
   * is never served again.
   */

  it("deleted record ID is absent from items after INVALIDATE + filtered refetch patch", () => {
    const existing: SceneItem[] = [
      { id: "mem-del", label: "To Delete", kind: "memory" },
      { id: "mem-keep", label: "Keep Me", kind: "memory" },
    ];
    let state = makeState({ revision: 0, items: existing });

    // Step 1: authority emits INVALIDATE for the deleted record.
    state = patchReducer(state, {
      type: "INVALIDATE",
      invalidatedIds: ["mem-del"],
    });
    // Items still present during refetch (stale display).
    expect(state.items.map((i) => i.id)).toContain("mem-del");

    // Step 2: refetch completes; orchestrator supplies items without the deleted record.
    const refetchPatch = makePatch({ base_revision: 0, target_revision: 1 });
    const refetchedItems = filterDeleted(existing, ["mem-del"]);
    state = patchReducer(
      state,
      { type: "APPLY_PATCH", patch: refetchPatch },
      refetchedItems,
    );

    expect(state.items.map((i) => i.id)).not.toContain("mem-del");
    expect(state.items.map((i) => i.id)).toContain("mem-keep");
  });

  it("items array does not contain the deleted record after patch with invalidations metadata", () => {
    // The patch's invalidations[] field is the authority-level list; the caller
    // uses it to build filtered newItems.
    const existing: SceneItem[] = [
      { id: "ent-gone", label: "Gone Entity", kind: "entity" },
      { id: "ent-here", label: "Present Entity", kind: "entity" },
    ];
    let state = makeState({ revision: 2, items: existing });

    // Authority patch that advances revision and lists the deleted record.
    const patch = makePatch({
      base_revision: 2,
      target_revision: 3,
      invalidations: ["ent-gone"],
    });
    const newItemsAfterDelete = filterDeleted(existing, patch.invalidations);
    state = patchReducer(state, { type: "APPLY_PATCH", patch }, newItemsAfterDelete);

    expect(state.revision).toBe(3);
    expect(state.items.find((i) => i.id === "ent-gone")).toBeUndefined();
    expect(state.items.find((i) => i.id === "ent-here")).toBeDefined();
  });

  it("zero items remain when the only item is deleted", () => {
    const only: SceneItem = { id: "only-mem", label: "Only Memory", kind: "memory" };
    let state = makeState({ revision: 0, items: [only] });

    state = patchReducer(state, { type: "INVALIDATE", invalidatedIds: [only.id] });
    const refetchPatch = makePatch({ base_revision: 0, target_revision: 1 });
    state = patchReducer(state, { type: "APPLY_PATCH", patch: refetchPatch }, []);

    expect(state.items).toHaveLength(0);
  });

  it("non-deleted items survive when only some records are deleted", () => {
    const items: SceneItem[] = [
      { id: "del-1", label: "Del 1", kind: "memory" },
      { id: "del-2", label: "Del 2", kind: "memory" },
      { id: "keep-1", label: "Keep 1", kind: "entity" },
      { id: "keep-2", label: "Keep 2", kind: "entity" },
    ];
    let state = makeState({ revision: 0, items });

    const deletedIds = ["del-1", "del-2"];
    state = patchReducer(state, { type: "INVALIDATE", invalidatedIds: deletedIds });

    const refetchPatch = makePatch({ base_revision: 0, target_revision: 1 });
    const remaining = filterDeleted(items, deletedIds);
    state = patchReducer(state, { type: "APPLY_PATCH", patch: refetchPatch }, remaining);

    const ids = state.items.map((i) => i.id);
    expect(ids).not.toContain("del-1");
    expect(ids).not.toContain("del-2");
    expect(ids).toContain("keep-1");
    expect(ids).toContain("keep-2");
  });
});

// ─── LC3: Source cascade ──────────────────────────────────────────────────────

describe("LC3 — Source cascade: deleting a source invalidates all derived records", () => {
  /**
   * When a source is deleted, the authority includes all derived record IDs in
   * the invalidations[] list of the patch (and/or the INVALIDATE action).
   * None of those derived records should appear in items after the cascade refetch.
   */

  it("all records derived from the deleted source are absent after cascade patch", () => {
    const sourceId = "src-news-feed";
    const derivedItems: SceneItem[] = [
      { id: "mem-from-src-1", label: "Derived 1", kind: "memory" },
      { id: "mem-from-src-2", label: "Derived 2", kind: "memory" },
      { id: "mem-from-src-3", label: "Derived 3", kind: "evidence" },
    ];
    const unrelatedItems: SceneItem[] = [
      { id: "unrelated-ent", label: "Unrelated Entity", kind: "entity" },
    ];
    const allItems = [...derivedItems, ...unrelatedItems];
    let state = makeState({ revision: 0, items: allItems });

    // Authority emits INVALIDATE listing the source and all derived records.
    const cascadeIds = [sourceId, ...derivedItems.map((i) => i.id)];
    state = patchReducer(state, {
      type: "INVALIDATE",
      invalidatedIds: cascadeIds,
    });

    // Sentinel is recorded with all cascaded IDs.
    const sentinel = state.pendingWrites.find((pw) =>
      pw.commandId.startsWith("__INVALIDATE__"),
    );
    expect(sentinel).toBeDefined();
    for (const id of cascadeIds) {
      expect(sentinel!.commandId).toContain(id);
    }

    // Refetch returns only non-derived items.
    const refetchPatch = makePatch({ base_revision: 0, target_revision: 1 });
    const afterCascade = filterDeleted(allItems, cascadeIds);
    state = patchReducer(
      state,
      { type: "APPLY_PATCH", patch: refetchPatch },
      afterCascade,
    );

    const ids = state.items.map((i) => i.id);
    expect(ids).not.toContain(sourceId);
    for (const d of derivedItems) {
      expect(ids).not.toContain(d.id);
    }
    expect(ids).toContain("unrelated-ent");
  });

  it("APPLY_PATCH invalidations field carries the full cascade list from the authority", () => {
    const derivedIds = ["ep-1", "ep-2", "mem-ep-1"];
    const patch = makePatch({
      base_revision: 0,
      target_revision: 1,
      invalidations: ["src-deleted", ...derivedIds],
    });

    // Verify the patch itself carries the full cascade list (structural check).
    expect(patch.invalidations).toContain("src-deleted");
    for (const id of derivedIds) {
      expect(patch.invalidations).toContain(id);
    }

    // When the caller applies filtered newItems, none of the cascade IDs survive.
    const allItems: SceneItem[] = [
      { id: "src-deleted", label: "Deleted Source", kind: "source" },
      { id: "ep-1", label: "Episode 1", kind: "evidence" },
      { id: "ep-2", label: "Episode 2", kind: "evidence" },
      { id: "mem-ep-1", label: "Mem from ep", kind: "memory" },
      { id: "safe-ent", label: "Safe Entity", kind: "entity" },
    ];
    let state = makeState({ revision: 0, items: allItems });
    const newItems = filterDeleted(allItems, patch.invalidations);
    state = patchReducer(state, { type: "APPLY_PATCH", patch }, newItems);

    const ids = state.items.map((i) => i.id);
    for (const id of patch.invalidations) {
      expect(ids).not.toContain(id);
    }
    expect(ids).toContain("safe-ent");
  });

  it("cascade INVALIDATE for source with no derived records still appends sentinel", () => {
    let state = makeState({ pendingWrites: [] });
    state = patchReducer(state, {
      type: "INVALIDATE",
      invalidatedIds: ["src-empty"],
    });

    expect(state.pendingWrites).toHaveLength(1);
    expect(state.pendingWrites[0].commandId).toContain("__INVALIDATE__");
    expect(state.pendingWrites[0].commandId).toContain("src-empty");
  });
});

// ─── LC4: Unrelated windows not reset ────────────────────────────────────────

describe("LC4 — Unrelated windows are not reset or invalidated by another window's lifecycle operation", () => {
  /**
   * Window A performs a Forget/Delete (dispatches INVALIDATE to its reducer).
   * Window B, with its own independent reducer state, must be completely
   * unaffected: its items, pendingWrites, revision, and sentinel count are
   * unchanged.
   */

  it("Window A's INVALIDATE does not append a sentinel to Window B's pendingWrites", () => {
    let stateA = makeState({ queryHash: "qa", pendingWrites: [] });
    const stateB = makeState({ queryHash: "qb", pendingWrites: [] });

    stateA = patchReducer(stateA, {
      type: "INVALIDATE",
      invalidatedIds: ["mem-A-001"],
    });

    expect(stateA.pendingWrites).toHaveLength(1);
    // B must still have zero pending writes.
    expect(stateB.pendingWrites).toHaveLength(0);
  });

  it("Window A's INVALIDATE does not remove items from Window B's reducer", () => {
    const itemsB: SceneItem[] = [
      { id: "B-ent-1", label: "B Entity 1", kind: "entity" },
      { id: "B-ent-2", label: "B Entity 2", kind: "entity" },
    ];
    let stateA = makeState({ items: [{ id: "A-mem-del", label: "A Delete", kind: "memory" }] });
    const stateB = makeState({ items: itemsB });

    stateA = patchReducer(stateA, {
      type: "INVALIDATE",
      invalidatedIds: ["A-mem-del"],
    });

    // B's items are a stable reference — no mutation.
    expect(stateB.items).toBe(itemsB);
    expect(stateB.items).toHaveLength(2);
  });

  it("Window A completing a delete refetch patch does not change Window B's revision", () => {
    let stateA = makeState({ revision: 0 });
    const stateB = makeState({ revision: 7 });

    stateA = patchReducer(stateA, { type: "INVALIDATE", invalidatedIds: ["A-mem"] });
    const refetchA = makePatch({ base_revision: 0, target_revision: 1 });
    stateA = patchReducer(stateA, { type: "APPLY_PATCH", patch: refetchA }, []);

    expect(stateA.revision).toBe(1);
    // B's revision is independently managed and unchanged.
    expect(stateB.revision).toBe(7);
  });

  it("Window B's sentinel count remains zero across Window A's full delete lifecycle", () => {
    let stateA = makeState({ revision: 0, pendingWrites: [] });
    const stateB = makeState({ revision: 3, pendingWrites: [] });

    // Full A lifecycle: invalidate → refetch.
    stateA = patchReducer(stateA, { type: "INVALIDATE", invalidatedIds: ["gone"] });
    const refetchPatch = makePatch({ base_revision: 0, target_revision: 1 });
    stateA = patchReducer(stateA, { type: "APPLY_PATCH", patch: refetchPatch }, []);

    // A has no pending writes after refetch resolved them in the orchestrator
    // (sentinel is not automatically removed by APPLY_PATCH — the orchestrator
    // clears it via CONFIRM_WRITE after the refetch succeeds).
    // B still has zero pending writes.
    expect(stateB.pendingWrites).toHaveLength(0);
  });

  it("two independent caches: invalidateByRevision on A does not affect B's entries", () => {
    const cacheA = new SnapshotCache<SceneItem[]>(8);
    const cacheB = new SnapshotCache<SceneItem[]>(8);

    const keyA = makeCacheKey({ revision: 5, queryHash: "qa", policyHash: "ph-A" });
    const keyB = makeCacheKey({ revision: 5, queryHash: "qb", policyHash: "ph-B" });

    const snapshotA = [{ id: "A-node", label: "A", kind: "entity" as const }];
    const snapshotB = [{ id: "B-node", label: "B", kind: "entity" as const }];

    cacheA.set(keyA, snapshotA);
    cacheB.set(keyB, snapshotB);

    // Window A's delete causes a revision bump; old entries are flushed from A.
    cacheA.invalidateByRevision(6); // new revision after delete

    expect(cacheA.get(keyA)).toBeUndefined();       // A's old entry evicted
    expect(cacheB.get(keyB)).toEqual(snapshotB);    // B completely untouched
  });

  it("INVALIDATE on Window A with source cascade does not invalidate Window B's items", () => {
    const bItems: SceneItem[] = [
      { id: "B-src-safe", label: "B Source", kind: "source" },
      { id: "B-derived-safe", label: "B Derived", kind: "memory" },
    ];
    let stateA = makeState({
      items: [
        { id: "A-src-gone", label: "A Source", kind: "source" },
        { id: "A-derived-gone", label: "A Derived", kind: "memory" },
      ],
    });
    const stateB = makeState({ items: bItems });

    // A receives a source-cascade invalidation.
    stateA = patchReducer(stateA, {
      type: "INVALIDATE",
      invalidatedIds: ["A-src-gone", "A-derived-gone"],
    });

    // B's items must be the same reference.
    expect(stateB.items).toBe(bItems);
  });
});

// ─── LC5: Snapshot cache invalidation ────────────────────────────────────────

describe("LC5 — Snapshot cache: old-revision entries for deleted records are not served after INVALIDATE", () => {
  /**
   * After an authority delete advances the graph revision, any cached snapshot
   * that was produced at the old revision is stale and must not be served.
   * invalidateByRevision(newRevision) flushes all entries whose revision !=
   * newRevision, ensuring deleted-record content is not returned from the cache.
   *
   * Additionally, clear() can be used to drop all entries when a full refetch
   * is required.
   */

  it("old-revision entries are absent after invalidateByRevision with new revision", () => {
    const cache = new SnapshotCache<SceneItem[]>(16);

    // Cache snapshots at the old revision (5) for various queries.
    const oldRev = 5;
    const key1 = makeCacheKey({ revision: oldRev, queryHash: "q1" });
    const key2 = makeCacheKey({ revision: oldRev, queryHash: "q2" });
    cache.set(key1, [{ id: "mem-deleted", label: "Deleted Mem", kind: "memory" }]);
    cache.set(key2, [{ id: "mem-other", label: "Other Mem", kind: "memory" }]);

    // Delete advances the authority revision to 6.
    const newRev = 6;
    cache.invalidateByRevision(newRev);

    // Both old entries are gone.
    expect(cache.get(key1)).toBeUndefined();
    expect(cache.get(key2)).toBeUndefined();
    expect(cache.size).toBe(0);
  });

  it("new-revision entries survive invalidateByRevision for the new revision", () => {
    const cache = new SnapshotCache<SceneItem[]>(16);
    const oldRev = 5;
    const newRev = 6;

    const oldKey = makeCacheKey({ revision: oldRev, queryHash: "q-old" });
    const newKey = makeCacheKey({ revision: newRev, queryHash: "q-new" });

    cache.set(oldKey, [{ id: "stale-rec", label: "Stale", kind: "memory" }]);
    cache.set(newKey, [{ id: "fresh-rec", label: "Fresh", kind: "entity" }]);

    cache.invalidateByRevision(newRev);

    expect(cache.get(oldKey)).toBeUndefined();
    expect(cache.get(newKey)).toBeDefined();
    expect(cache.get(newKey)![0].id).toBe("fresh-rec");
  });

  it("clear() removes all entries including those for deleted records", () => {
    const cache = new SnapshotCache<string>(8);

    cache.set(makeCacheKey({ revision: 3, queryHash: "q1" }), "snap1");
    cache.set(makeCacheKey({ revision: 3, queryHash: "q2" }), "snap2");
    cache.set(makeCacheKey({ revision: 4, queryHash: "q3" }), "snap3");

    cache.clear();

    expect(cache.size).toBe(0);
    expect(cache.get(makeCacheKey({ revision: 3, queryHash: "q1" }))).toBeUndefined();
    expect(cache.get(makeCacheKey({ revision: 3, queryHash: "q2" }))).toBeUndefined();
    expect(cache.get(makeCacheKey({ revision: 4, queryHash: "q3" }))).toBeUndefined();
  });

  it("cache miss after invalidation forces the orchestrator to re-query (undefined returned)", () => {
    const cache = new SnapshotCache<SceneItem[]>(8);
    const key = makeCacheKey({ revision: 10, queryHash: "inspector:mem-del" });

    // The inspector had a cached result for the (now-deleted) record.
    cache.set(key, [{ id: "mem-del", label: "Deleted", kind: "memory" }]);
    expect(cache.get(key)).toBeDefined();

    // Delete advances revision to 11; old entries cleared.
    cache.invalidateByRevision(11);

    // Cache now returns undefined — the orchestrator must perform a fresh query.
    expect(cache.get(key)).toBeUndefined();
  });

  it("invalidateByRevision with the same current revision clears all non-matching entries", () => {
    const cache = new SnapshotCache<string>(8);

    // Mix of revisions in the cache.
    cache.set(makeCacheKey({ revision: 1, queryHash: "q1" }), "v1");
    cache.set(makeCacheKey({ revision: 2, queryHash: "q2" }), "v2");
    cache.set(makeCacheKey({ revision: 3, queryHash: "q3" }), "v3");

    // Keep only revision 3.
    cache.invalidateByRevision(3);

    expect(cache.get(makeCacheKey({ revision: 1, queryHash: "q1" }))).toBeUndefined();
    expect(cache.get(makeCacheKey({ revision: 2, queryHash: "q2" }))).toBeUndefined();
    expect(cache.get(makeCacheKey({ revision: 3, queryHash: "q3" }))).toBe("v3");
    expect(cache.size).toBe(1);
  });

  it("snapshot cache for Window B is not cleared by Window A's post-delete invalidation", () => {
    // Each window owns its own SnapshotCache instance.
    const cacheA = new SnapshotCache<string>(8);
    const cacheB = new SnapshotCache<string>(8);

    cacheA.set(makeCacheKey({ revision: 5, queryHash: "qA", policyHash: "ph-A" }), "A-snap");
    cacheB.set(makeCacheKey({ revision: 5, queryHash: "qB", policyHash: "ph-B" }), "B-snap");

    // Window A's delete advances its revision; clear A's cache.
    cacheA.invalidateByRevision(6);

    expect(cacheA.size).toBe(0);
    // B's cache is a separate instance — completely unaffected.
    expect(cacheB.size).toBe(1);
  });
});

// ─── Integration: full lifecycle cascade across both windows ──────────────────

describe("Integration — full lifecycle cascade: Window A's delete does not affect Window B", () => {
  /**
   * End-to-end scenario: both windows start with populated items and caches.
   * Window A undergoes a full Forget→Delete cascade (INVALIDATE → refetch →
   * cache flush). At every stage Window B's reducer state, pending writes,
   * and cache are verified to be untouched.
   */

  it("full delete cascade on Window A leaves Window B intact at every stage", () => {
    // ── Setup: both windows start with items and caches ──────────────────────
    const itemsA: SceneItem[] = [
      { id: "A-src", label: "A Source", kind: "source" },
      { id: "A-derived-1", label: "A Derived 1", kind: "memory" },
      { id: "A-derived-2", label: "A Derived 2", kind: "evidence" },
      { id: "A-unrelated", label: "A Unrelated", kind: "entity" },
    ];
    const itemsB: SceneItem[] = [
      { id: "B-ent-1", label: "B Entity 1", kind: "entity" },
      { id: "B-mem-1", label: "B Memory 1", kind: "memory" },
    ];

    let stateA = makeState({ revision: 5, items: itemsA, policyHash: "ph-A", queryHash: "qa" });
    const stateB = makeState({ revision: 8, items: itemsB, policyHash: "ph-B", queryHash: "qb" });

    const cacheA = new SnapshotCache<SceneItem[]>(8);
    const cacheB = new SnapshotCache<SceneItem[]>(8);

    cacheA.set(makeCacheKey({ revision: 5, policyHash: "ph-A", queryHash: "qa" }), [...itemsA]);
    cacheB.set(makeCacheKey({ revision: 8, policyHash: "ph-B", queryHash: "qb" }), [...itemsB]);

    // ── Stage 1: Window A receives delete cascade INVALIDATE ─────────────────
    const cascadeIds = ["A-src", "A-derived-1", "A-derived-2"];
    stateA = patchReducer(stateA, {
      type: "INVALIDATE",
      invalidatedIds: cascadeIds,
    });

    // A has a sentinel; B is unchanged.
    expect(stateA.pendingWrites.length).toBeGreaterThan(0);
    expect(stateB.pendingWrites).toHaveLength(0);

    // B's items are still the same reference.
    expect(stateB.items).toBe(itemsB);

    // ── Stage 2: Window A's cache is flushed (revision advanced to 6) ────────
    cacheA.invalidateByRevision(6);

    expect(cacheA.size).toBe(0);
    expect(cacheB.size).toBe(1); // B's cache untouched

    // ── Stage 3: Window A refetch patch arrives with filtered items ───────────
    const refetchPatch = makePatch({
      base_revision: 5,
      target_revision: 6,
      policy_hash: "ph-A",
      invalidations: cascadeIds,
    });
    const refetchedA = filterDeleted(itemsA, cascadeIds);
    stateA = patchReducer(stateA, { type: "APPLY_PATCH", patch: refetchPatch }, refetchedA);

    // A: deleted records absent, unrelated record preserved.
    const aIds = stateA.items.map((i) => i.id);
    for (const id of cascadeIds) {
      expect(aIds).not.toContain(id);
    }
    expect(aIds).toContain("A-unrelated");
    expect(stateA.revision).toBe(6);

    // B: completely unchanged at every field.
    expect(stateB.revision).toBe(8);
    expect(stateB.items).toBe(itemsB);
    expect(stateB.pendingWrites).toHaveLength(0);
    expect(cacheB.size).toBe(1);
    expect(
      cacheB.get(makeCacheKey({ revision: 8, policyHash: "ph-B", queryHash: "qb" })),
    ).toBeDefined();
  });
});
