/**
 * patchReducer.test.ts
 *
 * Unit tests for patchReducer<T>: atomic patch application, duplicate/
 * reorder/gap guards, schema and policy mismatch no-ops, write confirmation
 * and rollback, invalidation sentinel, and refetch-required no-op.
 *
 * Requirements: MGR-008 (revision/patch consistency), MGR-004 (policy isolation),
 * F4.1 (window session isolation and convergence).
 */

import { describe, it, expect } from "vitest";
import {
  patchReducer,
  type ReducerState,
  type AuthorityPatch,
  type PatchAction,
  type PendingWrite,
} from "./patchReducer";

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
    recovery_cursor: null,
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

// ─── APPLY_PATCH ──────────────────────────────────────────────────────────────

describe("patchReducer — APPLY_PATCH", () => {
  it("advances revision when base_revision matches current revision", () => {
    const state = makeState({ revision: 5 });
    const patch = makePatch({ base_revision: 5, target_revision: 6 });
    const next = patchReducer(state, { type: "APPLY_PATCH", patch });
    expect(next.revision).toBe(6);
  });

  it("uses provided newItems when base_revision matches", () => {
    const state = makeState({ revision: 0, items: [{ id: "a", label: "old" }] });
    const patch = makePatch({ base_revision: 0, target_revision: 1 });
    const newItems: Item[] = [{ id: "b", label: "new" }];
    const next = patchReducer(state, { type: "APPLY_PATCH", patch }, newItems);
    expect(next.items).toEqual(newItems);
  });

  it("preserves existing items when newItems is undefined", () => {
    const existing: Item[] = [{ id: "a", label: "existing" }];
    const state = makeState({ revision: 0, items: existing });
    const patch = makePatch({ base_revision: 0, target_revision: 1 });
    const next = patchReducer(state, { type: "APPLY_PATCH", patch });
    expect(next.items).toBe(existing); // same reference
  });

  it("preserves other state fields on successful patch", () => {
    const state = makeState({
      revision: 3,
      schemaVersion: "2.0",
      policyHash: "policy-abc",
      queryHash: "query-xyz",
    });
    const patch = makePatch({ base_revision: 3, target_revision: 4 });
    const next = patchReducer(state, { type: "APPLY_PATCH", patch });
    expect(next.schemaVersion).toBe("2.0");
    expect(next.policyHash).toBe("policy-abc");
    expect(next.queryHash).toBe("query-xyz");
  });

  it("is a no-op when base_revision does not match current revision (stale patch)", () => {
    const state = makeState({ revision: 5 });
    const patch = makePatch({ base_revision: 3, target_revision: 4 });
    const next = patchReducer(state, { type: "APPLY_PATCH", patch });
    expect(next).toBe(state); // same reference — no mutation
  });

  it("is a no-op when base_revision is ahead of current revision (gap)", () => {
    const state = makeState({ revision: 2 });
    const patch = makePatch({ base_revision: 5, target_revision: 6 });
    const next = patchReducer(state, { type: "APPLY_PATCH", patch });
    expect(next).toBe(state);
  });

  it("is a no-op on a duplicate patch (same base already applied)", () => {
    // After applying, revision advances. Re-applying the same patch must be
    // a no-op because base_revision (0) no longer equals state.revision (1).
    const state = makeState({ revision: 0 });
    const patch = makePatch({ base_revision: 0, target_revision: 1 });
    const after = patchReducer(state, { type: "APPLY_PATCH", patch });
    expect(after.revision).toBe(1);

    // Re-apply the same patch — revision mismatch → no-op.
    const again = patchReducer(after, { type: "APPLY_PATCH", patch });
    expect(again).toBe(after);
    expect(again.revision).toBe(1);
  });

  it("is a no-op when schema_version does not match", () => {
    const state = makeState({ revision: 0, schemaVersion: "2.0" });
    const patch = makePatch({ base_revision: 0, target_revision: 1, schema_version: "3.0" });
    const next = patchReducer(state, { type: "APPLY_PATCH", patch });
    expect(next).toBe(state);
  });

  it("is a no-op when policy_hash does not match", () => {
    const state = makeState({ revision: 0, policyHash: "policy-abc" });
    const patch = makePatch({ base_revision: 0, target_revision: 1, policy_hash: "policy-NEW" });
    const next = patchReducer(state, { type: "APPLY_PATCH", patch });
    expect(next).toBe(state);
  });

  it("does not mutate the input state on a successful patch", () => {
    const state = makeState({ revision: 0, items: [{ id: "x", label: "x" }] });
    const frozen = Object.freeze({ ...state, items: Object.freeze([...state.items]) });
    const patch = makePatch({ base_revision: 0, target_revision: 1 });
    // Should not throw even though the input is frozen (pure function).
    const next = patchReducer(frozen as ReducerState<Item>, { type: "APPLY_PATCH", patch });
    expect(next).not.toBe(frozen);
    expect(next.revision).toBe(1);
  });
});

// ─── CONFIRM_WRITE ────────────────────────────────────────────────────────────

describe("patchReducer — CONFIRM_WRITE", () => {
  it("removes the matching pending write by commandId", () => {
    const state = makeState({
      revision: 0,
      pendingWrites: [
        makePendingWrite({ commandId: "cmd-1" }),
        makePendingWrite({ commandId: "cmd-2" }),
      ],
    });
    const action: PatchAction = { type: "CONFIRM_WRITE", commandId: "cmd-1", revision: 1 };
    const next = patchReducer(state, action);
    expect(next.pendingWrites).toHaveLength(1);
    expect(next.pendingWrites[0].commandId).toBe("cmd-2");
  });

  it("advances revision when confirmed revision is strictly higher", () => {
    const state = makeState({ revision: 5, pendingWrites: [makePendingWrite()] });
    const action: PatchAction = { type: "CONFIRM_WRITE", commandId: "cmd-1", revision: 7 };
    const next = patchReducer(state, action);
    expect(next.revision).toBe(7);
  });

  it("does not lower revision when confirmed revision is equal to current", () => {
    const state = makeState({ revision: 5, pendingWrites: [makePendingWrite()] });
    const action: PatchAction = { type: "CONFIRM_WRITE", commandId: "cmd-1", revision: 5 };
    const next = patchReducer(state, action);
    expect(next.revision).toBe(5);
  });

  it("does not lower revision when confirmed revision is lower than current", () => {
    // The patch stream may have already advanced the revision ahead of the
    // confirmation.
    const state = makeState({ revision: 10, pendingWrites: [makePendingWrite()] });
    const action: PatchAction = { type: "CONFIRM_WRITE", commandId: "cmd-1", revision: 8 };
    const next = patchReducer(state, action);
    expect(next.revision).toBe(10);
  });

  it("is a no-op on pendingWrites when commandId is not found", () => {
    const existing = [makePendingWrite({ commandId: "cmd-2" })];
    const state = makeState({ pendingWrites: existing });
    const action: PatchAction = { type: "CONFIRM_WRITE", commandId: "cmd-NOT-FOUND", revision: 1 };
    const next = patchReducer(state, action);
    expect(next.pendingWrites).toHaveLength(1);
    expect(next.pendingWrites[0].commandId).toBe("cmd-2");
  });

  it("leaves items unchanged", () => {
    const items: Item[] = [{ id: "a", label: "a" }];
    const state = makeState({ items, pendingWrites: [makePendingWrite()] });
    const action: PatchAction = { type: "CONFIRM_WRITE", commandId: "cmd-1", revision: 1 };
    const next = patchReducer(state, action);
    expect(next.items).toBe(items);
  });
});

// ─── ROLLBACK_WRITE ───────────────────────────────────────────────────────────

describe("patchReducer — ROLLBACK_WRITE", () => {
  it("removes the matching pending write by commandId", () => {
    const state = makeState({
      pendingWrites: [
        makePendingWrite({ commandId: "cmd-1" }),
        makePendingWrite({ commandId: "cmd-2" }),
      ],
    });
    const action: PatchAction = {
      type: "ROLLBACK_WRITE",
      commandId: "cmd-1",
      reason: "server rejected",
    };
    const next = patchReducer(state, action);
    expect(next.pendingWrites).toHaveLength(1);
    expect(next.pendingWrites[0].commandId).toBe("cmd-2");
  });

  it("restores items to pre-optimistic snapshot when optimisticItems is saved", () => {
    const preOptimistic: Item[] = [{ id: "before", label: "before" }];
    const currentItems: Item[] = [{ id: "after", label: "after" }];
    const state = makeState({
      items: currentItems,
      pendingWrites: [
        makePendingWrite({ commandId: "cmd-1", optimisticItems: preOptimistic }),
      ],
    });
    const action: PatchAction = {
      type: "ROLLBACK_WRITE",
      commandId: "cmd-1",
      reason: "conflict",
    };
    const next = patchReducer(state, action);
    expect(next.items).toEqual(preOptimistic);
  });

  it("keeps current items when no optimisticItems snapshot was saved", () => {
    const currentItems: Item[] = [{ id: "current", label: "current" }];
    const state = makeState({
      items: currentItems,
      pendingWrites: [
        // No optimisticItems provided
        makePendingWrite({ commandId: "cmd-1" }),
      ],
    });
    const action: PatchAction = {
      type: "ROLLBACK_WRITE",
      commandId: "cmd-1",
      reason: "timeout",
    };
    const next = patchReducer(state, action);
    expect(next.items).toBe(currentItems);
  });

  it("does not crash when commandId is not found in pendingWrites", () => {
    const state = makeState({ pendingWrites: [] });
    const action: PatchAction = {
      type: "ROLLBACK_WRITE",
      commandId: "cmd-NOT-FOUND",
      reason: "does not exist",
    };
    expect(() => patchReducer(state, action)).not.toThrow();
  });

  it("does not mutate revision", () => {
    const state = makeState({
      revision: 7,
      pendingWrites: [makePendingWrite({ commandId: "cmd-1" })],
    });
    const action: PatchAction = {
      type: "ROLLBACK_WRITE",
      commandId: "cmd-1",
      reason: "error",
    };
    const next = patchReducer(state, action);
    expect(next.revision).toBe(7);
  });
});

// ─── INVALIDATE ───────────────────────────────────────────────────────────────

describe("patchReducer — INVALIDATE", () => {
  it("does not empty items — stale items are preserved for display", () => {
    const items: Item[] = [{ id: "a", label: "a" }, { id: "b", label: "b" }];
    const state = makeState({ items });
    const action: PatchAction = { type: "INVALIDATE", invalidatedIds: ["a"] };
    const next = patchReducer(state, action);
    // Items are kept so the UI can continue to render them while a refetch runs.
    expect(next.items).toBe(items);
  });

  it("appends a sentinel pending-write entry for invalidated IDs", () => {
    const state = makeState({ pendingWrites: [] });
    const action: PatchAction = { type: "INVALIDATE", invalidatedIds: ["rec-1", "rec-2"] };
    const next = patchReducer(state, action);
    expect(next.pendingWrites).toHaveLength(1);
    expect(next.pendingWrites[0].commandId).toContain("__INVALIDATE__");
    expect(next.pendingWrites[0].commandId).toContain("rec-1");
    expect(next.pendingWrites[0].commandId).toContain("rec-2");
  });

  it("preserves existing pending writes when appending the sentinel", () => {
    const existing = makePendingWrite({ commandId: "cmd-existing" });
    const state = makeState({ pendingWrites: [existing] });
    const action: PatchAction = { type: "INVALIDATE", invalidatedIds: ["rec-x"] };
    const next = patchReducer(state, action);
    expect(next.pendingWrites).toHaveLength(2);
    expect(next.pendingWrites[0].commandId).toBe("cmd-existing");
  });

  it("does not mutate revision", () => {
    const state = makeState({ revision: 3 });
    const action: PatchAction = { type: "INVALIDATE", invalidatedIds: [] };
    const next = patchReducer(state, action);
    expect(next.revision).toBe(3);
  });

  it("handles empty invalidatedIds list without throwing", () => {
    const state = makeState();
    expect(() =>
      patchReducer(state, { type: "INVALIDATE", invalidatedIds: [] }),
    ).not.toThrow();
  });
});

// ─── REFETCH_REQUIRED ─────────────────────────────────────────────────────────

describe("patchReducer — REFETCH_REQUIRED", () => {
  it("returns the same state reference (pure no-op)", () => {
    const state = makeState({ revision: 4 });
    const action: PatchAction = {
      type: "REFETCH_REQUIRED",
      queryHash: "query-xyz",
      reason: "schema mismatch detected externally",
    };
    const next = patchReducer(state, action);
    expect(next).toBe(state);
  });

  it("does not mutate items", () => {
    const items: Item[] = [{ id: "a", label: "a" }];
    const state = makeState({ items });
    const action: PatchAction = {
      type: "REFETCH_REQUIRED",
      queryHash: "query-xyz",
      reason: "gap in patch stream",
    };
    const next = patchReducer(state, action);
    expect(next.items).toBe(items);
  });

  it("does not mutate revision", () => {
    const state = makeState({ revision: 99 });
    const action: PatchAction = {
      type: "REFETCH_REQUIRED",
      queryHash: "q",
      reason: "reorder",
    };
    const next = patchReducer(state, action);
    expect(next.revision).toBe(99);
  });

  it("does not mutate pendingWrites", () => {
    const writes = [makePendingWrite()];
    const state = makeState({ pendingWrites: writes });
    const action: PatchAction = {
      type: "REFETCH_REQUIRED",
      queryHash: "q",
      reason: "policy changed",
    };
    const next = patchReducer(state, action);
    expect(next.pendingWrites).toBe(writes);
  });
});

// ─── Integration: sequence of actions ────────────────────────────────────────

describe("patchReducer — integration sequences", () => {
  it("apply valid patch → confirm write → revision is correct", () => {
    let state = makeState({
      revision: 0,
      pendingWrites: [makePendingWrite({ commandId: "cmd-write", baseRevision: 0 })],
    });
    const patch = makePatch({ base_revision: 0, target_revision: 1 });

    // Patch arrives from authority.
    state = patchReducer(state, { type: "APPLY_PATCH", patch });
    expect(state.revision).toBe(1);

    // Confirmation arrives; revision is already advanced, should not regress.
    state = patchReducer(state, {
      type: "CONFIRM_WRITE",
      commandId: "cmd-write",
      revision: 1,
    });
    expect(state.revision).toBe(1);
    expect(state.pendingWrites).toHaveLength(0);
  });

  it("apply valid patch → rollback write → items revert", () => {
    const preOptimistic: Item[] = [{ id: "pre", label: "pre" }];
    const optimistic: Item[] = [{ id: "opt", label: "opt" }];

    let state = makeState({
      revision: 0,
      items: optimistic,
      pendingWrites: [
        makePendingWrite({ commandId: "cmd-add", optimisticItems: preOptimistic }),
      ],
    });

    const patch = makePatch({ base_revision: 0, target_revision: 1 });
    state = patchReducer(state, { type: "APPLY_PATCH", patch }, optimistic);
    expect(state.revision).toBe(1);

    // Rollback removes the optimistic display.
    state = patchReducer(state, {
      type: "ROLLBACK_WRITE",
      commandId: "cmd-add",
      reason: "rejected",
    });
    expect(state.items).toEqual(preOptimistic);
    expect(state.pendingWrites).toHaveLength(0);
  });

  it("mismatched revision patch does not advance state; subsequent correct patch does", () => {
    let state = makeState({ revision: 2 });

    // Stale patch (base=0 but current=2) — no-op.
    const stalePatch = makePatch({ base_revision: 0, target_revision: 1 });
    const afterStale = patchReducer(state, { type: "APPLY_PATCH", patch: stalePatch });
    expect(afterStale).toBe(state); // same reference

    // Correct patch (base=2).
    const correctPatch = makePatch({ base_revision: 2, target_revision: 3 });
    state = patchReducer(state, { type: "APPLY_PATCH", patch: correctPatch });
    expect(state.revision).toBe(3);
  });
});
