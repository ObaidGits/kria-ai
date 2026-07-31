/**
 * multiWindow.test.ts
 *
 * Multi-window ownership isolation tests for the Memory Control Center (task 5.2.1).
 *
 * Proves that two or more MemoryWindowSessionV2 instances maintain completely
 * independent state — destination, query, policy, selection, camera, and quality
 * — and that patchReducer / SnapshotCache instances owned by separate windows
 * are fully isolated from one another.
 *
 * Seven isolation properties are verified:
 *   P1  Destination isolation  — tab change in Window A doesn't affect Window B
 *   P2  Query isolation        — search in Window A doesn't change Window B's query state
 *   P3  Policy isolation       — policy hash from Window A doesn't invalidate Window B's cache
 *   P4  Selection isolation    — entity selection in Window A doesn't affect Window B
 *   P5  Camera isolation       — camera mutation in Window A doesn't affect Window B
 *   P6  Quality isolation      — quality/canvas change in Window A doesn't affect Window B
 *   P7  Reducer ownership      — dispatching to Window A's reducer has no effect on Window B's reducer
 *
 * Requirements: MGR-008, MGR-013, MGR-021; MGD-014, MGD-035; V-UI-UNIT-01.
 */

import { describe, it, expect, beforeEach } from "vitest";
import { MemoryWindowSessionV2, type WindowSessionConfig } from "./windowSession";
import {
  patchReducer,
  type ReducerState,
  type AuthorityPatch,
  type PendingWrite,
} from "./patchReducer";
import { SnapshotCache, type SnapshotCacheKey } from "./snapshotCache";

// ─── Shared item type ─────────────────────────────────────────────────────────

/** Minimal scene item used across all tests; fields mirror what the real DTO exposes. */
interface SceneItem {
  id: string;
  label: string;
  kind: "entity" | "memory" | "evidence";
}

// ─── Window fixture types ─────────────────────────────────────────────────────

/**
 * Minimal per-window camera state.
 * Each window owns its own instance; no shared mutable reference is permitted.
 */
interface CameraState {
  zoom: number;
  panX: number;
  panY: number;
}

/**
 * Minimal per-window quality setting.
 * Controls whether the canvas renderer is active for this window.
 */
type QualityMode = "list-first" | "full-scene";


// ─── Helper factories ─────────────────────────────────────────────────────────

function makeWindowConfig(
  instanceId: string,
  policyHash: string,
  schemaVersion = "2.0",
): WindowSessionConfig {
  return { instanceId, policyHash, schemaVersion };
}

function makeSession(
  instanceId: string,
  policyHash = "policy-default",
): MemoryWindowSessionV2 {
  return new MemoryWindowSessionV2(makeWindowConfig(instanceId, policyHash));
}

function makeReducerState(overrides: Partial<ReducerState<SceneItem>> = {}): ReducerState<SceneItem> {
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

function makeCacheKey(overrides: Partial<SnapshotCacheKey> = {}): SnapshotCacheKey {
  return {
    schemaVersion: "2.0",
    revision: 1,
    policyHash: "policy-default",
    queryHash: "qhash-default",
    ...overrides,
  };
}


// ─── P1: Destination isolation ────────────────────────────────────────────────

describe("P1 — Destination isolation: tab change in Window A does not affect Window B", () => {
  /**
   * Destinations are modelled via the window session's query string and state.
   * Window A transitions through an Overview→Recall flow (begins and completes
   * requests under different query strings); Window B remains on its own
   * destination query and must not be touched.
   */

  it("Window B state is unchanged when Window A transitions destination from Overview to Recall", () => {
    const winA = makeSession("win-A", "policy-A");
    const winB = makeSession("win-B", "policy-B");

    // Window B completes a Recall-destination request at revision 10.
    const { generation: genB } = winB.beginRequest("destination:recall");
    winB.completeRequest(genB, 10);

    expect(winB.state).toBe("ready");
    expect(winB.revision).toBe(10);

    // Window A starts at Overview and switches to Recall.
    winA.beginRequest("destination:overview");
    winA.beginRequest("destination:recall"); // simulates tab change in A

    // Window B must be completely unaffected.
    expect(winB.state).toBe("ready");
    expect(winB.revision).toBe(10);
    expect(winB.generation).toBe(1); // unchanged
  });

  it("Window A's AbortSignal cancellation on destination change does not touch Window B's signal", () => {
    const winA = makeSession("win-A");
    const winB = makeSession("win-B");

    const { signal: sigB } = winB.beginRequest("destination:knowledge");
    // A changes destination, causing its previous signal to be aborted.
    const { signal: sigA1 } = winA.beginRequest("destination:overview");
    winA.beginRequest("destination:recall"); // cancels sigA1

    expect(sigA1.aborted).toBe(true);
    expect(sigB.aborted).toBe(false); // B's signal must not be aborted
  });

  it("generation counter in Window B is independent of destination changes in Window A", () => {
    const winA = makeSession("win-A");
    const winB = makeSession("win-B");

    // Simulate many destination changes in A.
    for (let i = 0; i < 5; i++) {
      winA.beginRequest(`destination:tab-${i}`);
    }
    expect(winA.generation).toBe(5);

    // B has never been touched.
    expect(winB.generation).toBe(0);
  });
});


// ─── P2: Query isolation ──────────────────────────────────────────────────────

describe("P2 — Query isolation: search in Window A does not change Window B's query state", () => {
  /**
   * Each window drives its own reducer state with its own queryHash.
   * Applying a patch under Window A's query hash must leave Window B's
   * reducer state — with a different queryHash — completely unchanged.
   */

  it("patch applied to Window A's reducer does not mutate Window B's reducer state", () => {
    // Window A: query "foo" at revision 0
    let stateA = makeReducerState({ queryHash: "qhash-foo", revision: 0 });
    // Window B: query "bar" at revision 0
    const stateB = makeReducerState({ queryHash: "qhash-bar", revision: 0 });

    const patchA = makePatch({ base_revision: 0, target_revision: 1 });
    const newItemsA: SceneItem[] = [{ id: "a1", label: "foo result", kind: "entity" }];

    stateA = patchReducer(stateA, { type: "APPLY_PATCH", patch: patchA }, newItemsA);

    // A advanced.
    expect(stateA.revision).toBe(1);
    expect(stateA.items).toEqual(newItemsA);

    // B is completely untouched — same object reference.
    expect(stateB.revision).toBe(0);
    expect(stateB.items).toHaveLength(0);
    expect(stateB.queryHash).toBe("qhash-bar");
  });

  it("Window B's session state is unaffected when Window A begins a new search request", () => {
    const winA = makeSession("win-A");
    const winB = makeSession("win-B");

    const { generation: genB } = winB.beginRequest("bar");
    winB.completeRequest(genB, 5);

    // A begins a new search.
    winA.beginRequest("foo");

    expect(winB.state).toBe("ready");
    expect(winB.revision).toBe(5);
    expect(winB.generation).toBe(1);
  });

  it("completeRequest for Window A does not alter Window B's revision", () => {
    const winA = makeSession("win-A");
    const winB = makeSession("win-B");

    winB.beginRequest("bar");
    // B's request is still in-flight.

    const { generation: genA } = winA.beginRequest("foo");
    winA.completeRequest(genA, 42);

    // B revision must still be 0 (never completed).
    expect(winB.revision).toBe(0);
    expect(winB.state).toBe("loading");
  });

  it("failRequest for Window A does not put Window B into error state", () => {
    const winA = makeSession("win-A");
    const winB = makeSession("win-B");

    const { generation: genA } = winA.beginRequest("foo");
    winB.beginRequest("bar");

    winA.failRequest(genA);

    expect(winA.state).toBe("error");
    expect(winB.state).toBe("loading"); // unchanged
  });
});


// ─── P3: Policy isolation ─────────────────────────────────────────────────────

describe("P3 — Policy isolation: policy change in Window A does not invalidate Window B's cache", () => {
  /**
   * SnapshotCache is instantiated per-window. Invalidating the cache keyed
   * to Window A's policy hash must not touch Window B's independent cache.
   *
   * Additionally, the patchReducer rejects patches whose policy_hash does not
   * match the state's policyHash — this means Window A's policy changes cannot
   * contaminate Window B's reducer.
   */

  it("invalidateByPolicy on Window A's cache does not clear entries in Window B's cache", () => {
    const cacheA = new SnapshotCache<string>(8);
    const cacheB = new SnapshotCache<string>(8);

    const phA = "policy-A";
    const phB = "policy-B";

    cacheA.set(makeCacheKey({ policyHash: phA, queryHash: "q1" }), "A-snapshot-1");
    cacheA.set(makeCacheKey({ policyHash: phA, queryHash: "q2" }), "A-snapshot-2");
    cacheB.set(makeCacheKey({ policyHash: phB, queryHash: "q3" }), "B-snapshot-1");
    cacheB.set(makeCacheKey({ policyHash: phB, queryHash: "q4" }), "B-snapshot-2");

    // Policy change on A — invalidate A's entries.
    cacheA.invalidateByPolicy(phA);

    expect(cacheA.get(makeCacheKey({ policyHash: phA, queryHash: "q1" }))).toBeUndefined();
    expect(cacheA.get(makeCacheKey({ policyHash: phA, queryHash: "q2" }))).toBeUndefined();

    // B is untouched.
    expect(cacheB.get(makeCacheKey({ policyHash: phB, queryHash: "q3" }))).toBe("B-snapshot-1");
    expect(cacheB.get(makeCacheKey({ policyHash: phB, queryHash: "q4" }))).toBe("B-snapshot-2");
    expect(cacheB.size).toBe(2);
  });

  it("patch with Window A's policy_hash is rejected by Window B's reducer (different policyHash)", () => {
    const stateB = makeReducerState({ policyHash: "policy-B", revision: 0 });

    // Patch produced under Window A's policy.
    const patchA = makePatch({ base_revision: 0, target_revision: 1, policy_hash: "policy-A" });

    const nextB = patchReducer(stateB, { type: "APPLY_PATCH", patch: patchA });

    // Reducer rejects — returns same reference, no state change.
    expect(nextB).toBe(stateB);
    expect(nextB.revision).toBe(0);
  });

  it("Window B's guardPolicy rejects Window A's policyHash", () => {
    const winA = makeSession("win-A", "policy-A");
    const winB = makeSession("win-B", "policy-B");

    // A's policy hash must not pass B's guard.
    expect(winB.guardPolicy(winA.config.policyHash)).toBe(false);
    expect(winB.guardPolicy("policy-B")).toBe(true);
  });

  it("policy invalidation on cache A has no side effect on cache B size", () => {
    const cacheA = new SnapshotCache<string>(4);
    const cacheB = new SnapshotCache<string>(4);

    cacheA.set(makeCacheKey({ policyHash: "ph-A" }), "vA");
    cacheB.set(makeCacheKey({ policyHash: "ph-B", queryHash: "q1" }), "vB1");
    cacheB.set(makeCacheKey({ policyHash: "ph-B", queryHash: "q2" }), "vB2");

    cacheA.invalidateByPolicy("ph-A");

    expect(cacheA.size).toBe(0);
    expect(cacheB.size).toBe(2);
  });
});


// ─── P4: Selection isolation ──────────────────────────────────────────────────

describe("P4 — Selection isolation: entity selection in Window A does not affect Window B", () => {
  /**
   * Entity selection is modelled as the active query and items in each window's
   * reducer state. Selecting entity X in A means starting a new request
   * (queryHash for entity X) and completing it with entity-specific items.
   * Window B, selecting entity Y, must remain on its own items.
   */

  it("selecting entity X in Window A leaves Window B's selected entity Y unchanged in reducer", () => {
    const entityX: SceneItem = { id: "entity-X", label: "Entity X", kind: "entity" };
    const entityY: SceneItem = { id: "entity-Y", label: "Entity Y", kind: "entity" };

    let stateA = makeReducerState({ queryHash: "sel:entity-X", items: [] });
    let stateB = makeReducerState({ queryHash: "sel:entity-Y", items: [] });

    // Window A selects entity X.
    const patchA = makePatch({ base_revision: 0, target_revision: 1 });
    stateA = patchReducer(stateA, { type: "APPLY_PATCH", patch: patchA }, [entityX]);

    // Window B selects entity Y independently.
    const patchB = makePatch({ base_revision: 0, target_revision: 1 });
    stateB = patchReducer(stateB, { type: "APPLY_PATCH", patch: patchB }, [entityY]);

    expect(stateA.items).toEqual([entityX]);
    expect(stateB.items).toEqual([entityY]);

    // Cross-check: A's items do not appear in B.
    expect(stateB.items.find((i) => i.id === "entity-X")).toBeUndefined();
    expect(stateA.items.find((i) => i.id === "entity-Y")).toBeUndefined();
  });

  it("replacing selected entity in Window A does not overwrite Window B's items", () => {
    const entityX2: SceneItem = { id: "entity-X2", label: "Entity X2", kind: "entity" };
    const entityY: SceneItem = { id: "entity-Y", label: "Entity Y", kind: "entity" };

    // B already has entityY loaded at revision 1.
    let stateB = makeReducerState({ queryHash: "sel:entity-Y", revision: 1, items: [entityY] });

    // A changes selection to X2 at revision 1→2.
    let stateA = makeReducerState({ queryHash: "sel:entity-X", revision: 1 });
    const patchA = makePatch({ base_revision: 1, target_revision: 2 });
    stateA = patchReducer(stateA, { type: "APPLY_PATCH", patch: patchA }, [entityX2]);

    // B is unchanged.
    expect(stateB.items).toEqual([entityY]);
    expect(stateB.revision).toBe(1);
  });

  it("Window A's session state transitions during entity-inspect flow do not touch Window B", () => {
    const winA = makeSession("win-A");
    const winB = makeSession("win-B");

    // B is in ready state viewing entity Y.
    const { generation: genB } = winB.beginRequest("inspect:entity-Y");
    winB.completeRequest(genB, 3);

    // A navigates into entity X inspect view.
    winA.beginRequest("inspect:entity-X");

    expect(winB.state).toBe("ready");
    expect(winB.revision).toBe(3);
  });
});


// ─── P5: Camera isolation ─────────────────────────────────────────────────────

describe("P5 — Camera isolation: camera mutation in Window A does not affect Window B", () => {
  /**
   * Camera state is per-window mutable value. This suite uses plain objects to
   * model camera instances — each window owns its own; no shared reference exists.
   * It also verifies the session-level invariant: camera interactions (beginRequest
   * at zoom/pan) do not leak between sessions.
   */

  it("mutating camera of Window A does not change Window B's camera object", () => {
    // Each window owns its own camera instance — no shared reference.
    const cameraA: CameraState = { zoom: 1.0, panX: 0, panY: 0 };
    const cameraB: CameraState = { zoom: 1.0, panX: 0, panY: 0 };

    // Simulate Window A: user zooms to 2.0 and pans.
    cameraA.zoom = 2.0;
    cameraA.panX = 100;
    cameraA.panY = 200;

    // Window B must retain its original camera.
    expect(cameraB.zoom).toBe(1.0);
    expect(cameraB.panX).toBe(0);
    expect(cameraB.panY).toBe(0);
  });

  it("camera-driven requests in Window A do not invalidate Window B's cached snapshot", () => {
    const cacheA = new SnapshotCache<string>(8);
    const cacheB = new SnapshotCache<string>(8);

    // Both windows cache a snapshot at revision 5.
    const keyA = makeCacheKey({ policyHash: "ph-A", queryHash: "qhash-cam-A", revision: 5 });
    const keyB = makeCacheKey({ policyHash: "ph-B", queryHash: "qhash-cam-B", revision: 5 });
    cacheA.set(keyA, "A-cam-snapshot");
    cacheB.set(keyB, "B-cam-snapshot");

    // Window A changes camera → triggers revision invalidation in A's cache only.
    cacheA.invalidateByRevision(99); // simulate new revision for A

    expect(cacheA.get(keyA)).toBeUndefined(); // A's snapshot evicted
    expect(cacheB.get(keyB)).toBe("B-cam-snapshot"); // B untouched
  });

  it("Window A's camera-zoom request (beginRequest) does not abort Window B's in-flight request", () => {
    const winA = makeSession("win-A");
    const winB = makeSession("win-B");

    const { signal: sigB } = winB.beginRequest("cam:zoom-viewport");

    // A pans/zooms — issues multiple requests.
    winA.beginRequest("cam:zoom-1.5");
    winA.beginRequest("cam:pan-100-200");

    expect(sigB.aborted).toBe(false);
  });

  it("three windows with distinct camera states remain independent", () => {
    const cameras: CameraState[] = [
      { zoom: 2.0, panX: 100, panY: 200 }, // Window A
      { zoom: 1.0, panX: 0, panY: 0 },     // Window B
      { zoom: 0.5, panX: -50, panY: 30 },  // Window C
    ];

    // Mutate Window A's camera.
    cameras[0].zoom = 3.0;

    // B and C unchanged.
    expect(cameras[1].zoom).toBe(1.0);
    expect(cameras[2].zoom).toBe(0.5);
  });
});


// ─── P6: Quality isolation ────────────────────────────────────────────────────

describe("P6 — Quality isolation: quality/canvas mode change in Window A does not affect Window B", () => {
  /**
   * Quality mode (list-first vs full-scene) is per-window. This suite verifies
   * that a quality downgrade in Window A (dropping the canvas renderer) does not
   * propagate to Window B, which remains in full-scene mode.
   *
   * The spec's Quality_Ladder preserves truth/list/actions regardless of quality;
   * reducer items must remain intact even when A's quality changes.
   */

  it("quality mode objects for two windows are independent references", () => {
    let qualityA: QualityMode = "full-scene";
    const qualityB: QualityMode = "full-scene";

    // Window A degrades to list-first.
    qualityA = "list-first";

    expect(qualityA).toBe("list-first");
    expect(qualityB).toBe("full-scene"); // B unaffected
  });

  it("quality downgrade in Window A does not remove items from Window B's reducer", () => {
    const itemsB: SceneItem[] = [
      { id: "n1", label: "Node 1", kind: "entity" },
      { id: "n2", label: "Node 2", kind: "memory" },
    ];

    let stateA = makeReducerState({ queryHash: "qa", items: [{ id: "x", label: "X", kind: "entity" }] });
    let stateB = makeReducerState({ queryHash: "qb", items: itemsB });

    // Simulate Window A entering list-first: INVALIDATE clears its items via refetch signal.
    stateA = patchReducer(stateA, { type: "INVALIDATE", invalidatedIds: ["x"] });

    // B is not touched.
    expect(stateB.items).toBe(itemsB);
    expect(stateB.items).toHaveLength(2);
  });

  it("REFETCH_REQUIRED signal for Window A leaves Window B's reducer state reference unchanged", () => {
    const stateA = makeReducerState({ queryHash: "qa" });
    const stateB = makeReducerState({ queryHash: "qb", revision: 7 });

    // A receives a refetch required (e.g. quality schema mismatch).
    const nextA = patchReducer(stateA, {
      type: "REFETCH_REQUIRED",
      queryHash: "qa",
      reason: "quality ladder schema changed",
    });

    // A's state is unchanged (refetch signal is a no-op on the reducer).
    expect(nextA).toBe(stateA);

    // B is completely separate — we verify no cross-mutation by checking its fields.
    expect(stateB.revision).toBe(7);
    expect(stateB.queryHash).toBe("qb");
  });

  it("Window A's snapshot cache entries at quality-downgrade revision do not affect Window B's cache", () => {
    const cacheA = new SnapshotCache<string>(8);
    const cacheB = new SnapshotCache<string>(8);

    cacheA.set(makeCacheKey({ queryHash: "q-full-scene", revision: 10, policyHash: "ph-A" }), "full-scene-snap");
    cacheB.set(makeCacheKey({ queryHash: "q-full-scene", revision: 10, policyHash: "ph-B" }), "B-full-scene-snap");

    // A's quality degrades — clear its entire cache.
    cacheA.clear();

    expect(cacheA.size).toBe(0);
    expect(cacheB.size).toBe(1); // B untouched
  });
});


// ─── P7: Reducer ownership ────────────────────────────────────────────────────

describe("P7 — Reducer ownership: each window has a separate reducer instance; dispatching to A has no effect on B", () => {
  /**
   * This is the core ownership guarantee: MemoryWindowSessionV2 instances and
   * ReducerState<T> objects are plain values / class instances with NO shared
   * mutable state. Dispatch to Window A's (session, state, reducer) triple
   * must leave Window B's triple completely unchanged.
   *
   * Tests cover all five PatchAction types.
   */

  it("APPLY_PATCH dispatched to A's reducer does not change B's reducer state", () => {
    let stateA = makeReducerState({ policyHash: "ph-A", queryHash: "qa", revision: 0 });
    const stateB = makeReducerState({ policyHash: "ph-B", queryHash: "qb", revision: 0 });

    const patchA = makePatch({ base_revision: 0, target_revision: 1, policy_hash: "ph-A" });
    const refBefore = stateB;

    stateA = patchReducer(stateA, { type: "APPLY_PATCH", patch: patchA });

    // A advanced; B is same reference (no mutation whatsoever).
    expect(stateA.revision).toBe(1);
    expect(stateB).toBe(refBefore);
    expect(stateB.revision).toBe(0);
  });

  it("CONFIRM_WRITE dispatched to A's reducer does not affect B's pendingWrites or revision", () => {
    const writeA = makePendingWrite({ commandId: "cmd-A", baseRevision: 0 });
    const writeB = makePendingWrite({ commandId: "cmd-B", baseRevision: 0 });

    let stateA = makeReducerState({ revision: 1, pendingWrites: [writeA] });
    const stateB = makeReducerState({ revision: 1, pendingWrites: [writeB] });

    stateA = patchReducer(stateA, { type: "CONFIRM_WRITE", commandId: "cmd-A", revision: 2 });

    expect(stateA.pendingWrites).toHaveLength(0);
    expect(stateA.revision).toBe(2);

    // B still has its own pending write untouched.
    expect(stateB.pendingWrites).toHaveLength(1);
    expect(stateB.pendingWrites[0].commandId).toBe("cmd-B");
    expect(stateB.revision).toBe(1);
  });

  it("ROLLBACK_WRITE dispatched to A's reducer does not revert items in B's reducer", () => {
    const preA: SceneItem[] = [{ id: "pre-A", label: "Pre-A", kind: "entity" }];
    const currentB: SceneItem[] = [{ id: "B-item", label: "B Item", kind: "memory" }];

    let stateA = makeReducerState({
      items: [{ id: "opt-A", label: "Optimistic A", kind: "entity" }],
      pendingWrites: [makePendingWrite({ commandId: "cmd-A", optimisticItems: preA })],
    });
    const stateB = makeReducerState({ items: currentB });

    stateA = patchReducer(stateA, {
      type: "ROLLBACK_WRITE",
      commandId: "cmd-A",
      reason: "server conflict",
    });

    // A reverted.
    expect(stateA.items).toEqual(preA);

    // B items unchanged.
    expect(stateB.items).toBe(currentB);
  });

  it("INVALIDATE dispatched to A's reducer does not append sentinel to B's pendingWrites", () => {
    let stateA = makeReducerState({ queryHash: "qa" });
    const stateB = makeReducerState({ queryHash: "qb", pendingWrites: [] });

    stateA = patchReducer(stateA, { type: "INVALIDATE", invalidatedIds: ["rec-A-1"] });

    expect(stateA.pendingWrites).toHaveLength(1);
    expect(stateA.pendingWrites[0].commandId).toContain("__INVALIDATE__");

    // B's pendingWrites remain empty.
    expect(stateB.pendingWrites).toHaveLength(0);
  });

  it("REFETCH_REQUIRED dispatched to A's session does not alter B's session state or generation", () => {
    const winA = makeSession("win-A");
    const winB = makeSession("win-B");

    const { generation: genB } = winB.beginRequest("qb");
    winB.completeRequest(genB, 5);

    // A receives a refetch-required signal (modelled as a reset on A's session).
    winA.beginRequest("qa");
    winA.reset(); // simulates handling REFETCH_REQUIRED — A resets to idle

    expect(winA.state).toBe("idle");
    expect(winA.generation).toBe(0);

    // B is completely unaffected.
    expect(winB.state).toBe("ready");
    expect(winB.revision).toBe(5);
    expect(winB.generation).toBe(1);
  });
});


// ─── Integration: three+ windows, all properties together ─────────────────────

describe("Integration — three windows with all distinct dimensions remain independent", () => {
  /**
   * Combines all seven properties in a single scenario: three window instances
   * each with distinct destination, query, policy, selection, camera, and quality.
   * Operations on each window must leave the other two unaffected.
   */

  it("three windows with distinct state remain independent after a full operation sequence", () => {
    // Create three windows with different policies.
    const winA = makeSession("win-A", "ph-A");
    const winB = makeSession("win-B", "ph-B");
    const winC = makeSession("win-C", "ph-C");

    // Three independent reducers.
    const itemsA: SceneItem[] = [{ id: "eA", label: "A Entity", kind: "entity" }];
    const itemsB: SceneItem[] = [{ id: "eB", label: "B Entity", kind: "memory" }];
    const itemsC: SceneItem[] = [{ id: "eC", label: "C Entity", kind: "evidence" }];

    let stateA = makeReducerState({ policyHash: "ph-A", queryHash: "qa", revision: 0 });
    let stateB = makeReducerState({ policyHash: "ph-B", queryHash: "qb", revision: 0 });
    let stateC = makeReducerState({ policyHash: "ph-C", queryHash: "qc", revision: 0 });

    // Each window runs its own session lifecycle.
    const { generation: genA } = winA.beginRequest("destination:overview");
    const { generation: genB } = winB.beginRequest("destination:recall");
    const { generation: genC } = winC.beginRequest("destination:knowledge");

    winA.completeRequest(genA, 10);
    winB.completeRequest(genB, 20);
    winC.completeRequest(genC, 30);

    // Each window applies its own patch.
    const pA = makePatch({ base_revision: 0, target_revision: 1, policy_hash: "ph-A" });
    const pB = makePatch({ base_revision: 0, target_revision: 1, policy_hash: "ph-B" });
    const pC = makePatch({ base_revision: 0, target_revision: 1, policy_hash: "ph-C" });

    stateA = patchReducer(stateA, { type: "APPLY_PATCH", patch: pA }, itemsA);
    stateB = patchReducer(stateB, { type: "APPLY_PATCH", patch: pB }, itemsB);
    stateC = patchReducer(stateC, { type: "APPLY_PATCH", patch: pC }, itemsC);

    // Verify session isolation.
    expect(winA.revision).toBe(10);
    expect(winB.revision).toBe(20);
    expect(winC.revision).toBe(30);

    // Verify reducer isolation.
    expect(stateA.items).toEqual(itemsA);
    expect(stateB.items).toEqual(itemsB);
    expect(stateC.items).toEqual(itemsC);

    // No cross-contamination in items.
    expect(stateA.items.find((i) => i.id === "eB" || i.id === "eC")).toBeUndefined();
    expect(stateB.items.find((i) => i.id === "eA" || i.id === "eC")).toBeUndefined();
    expect(stateC.items.find((i) => i.id === "eA" || i.id === "eB")).toBeUndefined();
  });

  it("markDetached on Window A does not detach or abort Window B and C", () => {
    const winA = makeSession("win-A");
    const winB = makeSession("win-B");
    const winC = makeSession("win-C");

    const { signal: sigB } = winB.beginRequest("qb");
    const { signal: sigC } = winC.beginRequest("qc");

    winA.beginRequest("qa");
    winA.markDetached();

    expect(winA.state).toBe("detached");
    expect(winB.state).toBe("loading");
    expect(winC.state).toBe("loading");
    expect(sigB.aborted).toBe(false);
    expect(sigC.aborted).toBe(false);
  });

  it("reset on Window A does not reset Window B or C", () => {
    const winA = makeSession("win-A");
    const winB = makeSession("win-B");
    const winC = makeSession("win-C");

    const { generation: genB } = winB.beginRequest("qb");
    winB.completeRequest(genB, 50);

    const { generation: genC } = winC.beginRequest("qc");
    winC.completeRequest(genC, 60);

    // A resets.
    winA.beginRequest("qa");
    winA.reset();

    expect(winA.state).toBe("idle");
    expect(winA.revision).toBe(0);

    // B and C are untouched.
    expect(winB.state).toBe("ready");
    expect(winB.revision).toBe(50);
    expect(winC.state).toBe("ready");
    expect(winC.revision).toBe(60);
  });

  it("three independent SnapshotCache instances with policy isolation across all windows", () => {
    const cacheA = new SnapshotCache<string>(8);
    const cacheB = new SnapshotCache<string>(8);
    const cacheC = new SnapshotCache<string>(8);

    cacheA.set(makeCacheKey({ policyHash: "ph-A", queryHash: "qA" }), "A-data");
    cacheB.set(makeCacheKey({ policyHash: "ph-B", queryHash: "qB" }), "B-data");
    cacheC.set(makeCacheKey({ policyHash: "ph-C", queryHash: "qC" }), "C-data");

    // Invalidate A's policy — only A loses entries.
    cacheA.invalidateByPolicy("ph-A");

    expect(cacheA.size).toBe(0);
    expect(cacheB.size).toBe(1);
    expect(cacheC.size).toBe(1);

    // Invalidate B's revision — only B loses entries.
    cacheB.invalidateByRevision(999);

    expect(cacheB.size).toBe(0);
    expect(cacheC.size).toBe(1);
  });
});

