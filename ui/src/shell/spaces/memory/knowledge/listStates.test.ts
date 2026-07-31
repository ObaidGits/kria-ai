/**
 * Tests for listStates — pure TypeScript module (no DOM, no JSX).
 *
 * Validates: Requirements MGR-006, MGR-014, MGR-031; MGD-026, MGD-030.
 *
 * Coverage:
 *   - getCopyForState returns correct copy for all 12 states
 *   - Copy never contains "something went wrong", "empty store",
 *     or reveals hidden items for unauthorized
 *   - isRetryable: true for timeout, retryable error; false for all others
 *   - getCorrelationId: correct for each state; null for empty / offline-with-null
 *   - getPreservedIntent: present for loading, timeout, error; null for others
 *   - hasItems: correct for ready/partial/stale/offline variants
 *   - isDegraded: true for stale/partial/offline only
 *   - isHardError: true for unauthorized/malformed/error/recovery only
 *   - Constructor functions produce correct discriminated union shapes
 */

import { describe, it, expect } from "vitest";
import {
  LIST_STATE_COPY,
  getCopyForState,
  isRetryable,
  getCorrelationId,
  getPreservedIntent,
  hasItems,
  isDegraded,
  isHardError,
  mkLoading,
  mkReady,
  mkPartial,
  mkStale,
  mkOffline,
  mkUnauthorized,
  mkTimeout,
  mkMalformed,
  mkError,
  mkDeleted,
  mkRecovery,
  type ListRequestState,
} from "./listStates";

// ─── Fixtures ─────────────────────────────────────────────────────────────────

const CID = "corr-abc-123";
const INTENT = "search:query=foo";
const ITEMS = [{ id: "1" }, { id: "2" }];

// One instance of each state kind
const STATES: ListRequestState[] = [
  { kind: "empty" },
  mkLoading(CID, INTENT),
  mkReady(ITEMS, CID),
  mkPartial(ITEMS, CID, ["bm25"]),
  mkStale(ITEMS, CID, "2024-01-01T00:00:00Z"),
  mkOffline(ITEMS, CID),
  mkUnauthorized(CID, "role:viewer"),
  mkTimeout(CID, INTENT),
  mkMalformed(CID, "unexpected field x"),
  mkError(CID, "db timeout", true, INTENT),
  mkDeleted("item-42", CID),
  mkRecovery(CID, "replica lag detected"),
];

const ALL_KINDS: ListRequestState["kind"][] = [
  "empty", "loading", "ready", "partial", "stale", "offline",
  "unauthorized", "timeout", "malformed", "error", "deleted", "recovery",
];

// ─── getCopyForState ──────────────────────────────────────────────────────────

describe("getCopyForState", () => {
  it("returns a string for every state kind", () => {
    for (const kind of ALL_KINDS) {
      expect(typeof getCopyForState(kind)).toBe("string");
    }
  });

  it("matches LIST_STATE_COPY for every kind", () => {
    for (const kind of ALL_KINDS) {
      expect(getCopyForState(kind)).toBe(LIST_STATE_COPY[kind]);
    }
  });

  it("returns empty string for ready (items speak for themselves)", () => {
    expect(getCopyForState("ready")).toBe("");
  });

  it("empty copy says query found nothing — does NOT claim the store is empty", () => {
    const copy = getCopyForState("empty");
    expect(copy.toLowerCase()).not.toContain("store is empty");
    expect(copy.toLowerCase()).not.toContain("nothing in");
    expect(copy).toContain("match your search");
  });

  it("unauthorized copy does NOT reveal existence of hidden items", () => {
    const copy = getCopyForState("unauthorized");
    expect(copy.toLowerCase()).not.toContain("hidden");
    expect(copy.toLowerCase()).not.toContain("exists");
    expect(copy.toLowerCase()).not.toContain("available");
  });

  it("no state copy contains 'something went wrong'", () => {
    for (const kind of ALL_KINDS) {
      const copy = getCopyForState(kind).toLowerCase();
      expect(copy).not.toContain("something went wrong");
    }
  });

  it("no state copy claims the store is empty", () => {
    for (const kind of ALL_KINDS) {
      const copy = getCopyForState(kind).toLowerCase();
      expect(copy).not.toContain("empty store");
      expect(copy).not.toContain("store is empty");
    }
  });

  // Spot-check exact copy strings
  it("loading copy is exactly 'Loading…'", () => {
    expect(getCopyForState("loading")).toBe("Loading\u2026");
  });

  it("timeout copy mentions retry", () => {
    expect(getCopyForState("timeout").toLowerCase()).toContain("retry");
  });

  it("malformed copy instructs user to report correlation ID", () => {
    expect(getCopyForState("malformed").toLowerCase()).toContain("correlation id");
  });

  it("deleted copy says item is no longer available", () => {
    expect(getCopyForState("deleted")).toContain("no longer available");
  });

  it("recovery copy says writes are disabled", () => {
    expect(getCopyForState("recovery").toLowerCase()).toContain("writes are disabled");
  });
});

// ─── isRetryable ─────────────────────────────────────────────────────────────

describe("isRetryable", () => {
  it("timeout is retryable", () => {
    expect(isRetryable(mkTimeout(CID, INTENT))).toBe(true);
  });

  it("error with retryable=true is retryable", () => {
    expect(isRetryable(mkError(CID, "db down", true, INTENT))).toBe(true);
  });

  it("error with retryable=false is not retryable", () => {
    expect(isRetryable(mkError(CID, "fatal", false, INTENT))).toBe(false);
  });

  const nonRetryableKinds: ListRequestState[] = [
    { kind: "empty" },
    mkLoading(CID, INTENT),
    mkReady(ITEMS, CID),
    mkPartial(ITEMS, CID, []),
    mkStale(ITEMS, CID, "2024-01-01T00:00:00Z"),
    mkOffline(null, null),
    mkUnauthorized(CID, null),
    mkMalformed(CID, "bad"),
    mkDeleted("x", CID),
    mkRecovery(CID, "lag"),
  ];

  it.each(nonRetryableKinds.map((s) => [s.kind, s]))(
    "state '%s' is not retryable",
    (_kind, state) => {
      expect(isRetryable(state)).toBe(false);
    },
  );
});

// ─── getCorrelationId ─────────────────────────────────────────────────────────

describe("getCorrelationId", () => {
  it("returns null for empty", () => {
    expect(getCorrelationId({ kind: "empty" })).toBeNull();
  });

  it("returns null for offline with null lastKnownCorrelationId", () => {
    expect(getCorrelationId(mkOffline(null, null))).toBeNull();
  });

  it("returns lastKnownCorrelationId for offline when set", () => {
    expect(getCorrelationId(mkOffline([], CID))).toBe(CID);
  });

  it("returns correlationId for loading", () => {
    expect(getCorrelationId(mkLoading(CID, INTENT))).toBe(CID);
  });

  it("returns correlationId for ready", () => {
    expect(getCorrelationId(mkReady([], CID))).toBe(CID);
  });

  it("returns correlationId for partial", () => {
    expect(getCorrelationId(mkPartial([], CID, []))).toBe(CID);
  });

  it("returns correlationId for stale", () => {
    expect(getCorrelationId(mkStale([], CID, "2024-01-01T00:00:00Z"))).toBe(CID);
  });

  it("returns correlationId for unauthorized", () => {
    expect(getCorrelationId(mkUnauthorized(CID, null))).toBe(CID);
  });

  it("returns correlationId for timeout", () => {
    expect(getCorrelationId(mkTimeout(CID, INTENT))).toBe(CID);
  });

  it("returns correlationId for malformed", () => {
    expect(getCorrelationId(mkMalformed(CID, "x"))).toBe(CID);
  });

  it("returns correlationId for error", () => {
    expect(getCorrelationId(mkError(CID, "msg", true, INTENT))).toBe(CID);
  });

  it("returns correlationId for deleted", () => {
    expect(getCorrelationId(mkDeleted("item-1", CID))).toBe(CID);
  });

  it("returns correlationId for recovery", () => {
    expect(getCorrelationId(mkRecovery(CID, "diag"))).toBe(CID);
  });
});

// ─── getPreservedIntent ───────────────────────────────────────────────────────

describe("getPreservedIntent", () => {
  it("returns intent for loading", () => {
    expect(getPreservedIntent(mkLoading(CID, INTENT))).toBe(INTENT);
  });

  it("returns intent for timeout", () => {
    expect(getPreservedIntent(mkTimeout(CID, INTENT))).toBe(INTENT);
  });

  it("returns intent for error", () => {
    expect(getPreservedIntent(mkError(CID, "msg", true, INTENT))).toBe(INTENT);
  });

  const noIntentStates: ListRequestState[] = [
    { kind: "empty" },
    mkReady([], CID),
    mkPartial([], CID, []),
    mkStale([], CID, "2024-01-01T00:00:00Z"),
    mkOffline(null, null),
    mkUnauthorized(CID, null),
    mkMalformed(CID, "x"),
    mkDeleted("x", CID),
    mkRecovery(CID, "diag"),
  ];

  it.each(noIntentStates.map((s) => [s.kind, s]))(
    "state '%s' returns null for getPreservedIntent",
    (_kind, state) => {
      expect(getPreservedIntent(state)).toBeNull();
    },
  );
});

// ─── hasItems ─────────────────────────────────────────────────────────────────

describe("hasItems", () => {
  it("true for ready with items", () => {
    expect(hasItems(mkReady(ITEMS, CID))).toBe(true);
  });

  it("false for ready with empty array", () => {
    expect(hasItems(mkReady([], CID))).toBe(false);
  });

  it("true for partial with items", () => {
    expect(hasItems(mkPartial(ITEMS, CID, []))).toBe(true);
  });

  it("false for partial with empty array", () => {
    expect(hasItems(mkPartial([], CID, []))).toBe(false);
  });

  it("true for stale with items", () => {
    expect(hasItems(mkStale(ITEMS, CID, "2024-01-01T00:00:00Z"))).toBe(true);
  });

  it("false for stale with empty array", () => {
    expect(hasItems(mkStale([], CID, "2024-01-01T00:00:00Z"))).toBe(false);
  });

  it("true for offline with non-empty preservedItems", () => {
    expect(hasItems(mkOffline(ITEMS, CID))).toBe(true);
  });

  it("false for offline with null preservedItems", () => {
    expect(hasItems(mkOffline(null, null))).toBe(false);
  });

  it("false for offline with empty preservedItems array", () => {
    expect(hasItems(mkOffline([], CID))).toBe(false);
  });

  const noItemStates: ListRequestState[] = [
    { kind: "empty" },
    mkLoading(CID, INTENT),
    mkUnauthorized(CID, null),
    mkTimeout(CID, INTENT),
    mkMalformed(CID, "x"),
    mkError(CID, "msg", true, INTENT),
    mkDeleted("x", CID),
    mkRecovery(CID, "diag"),
  ];

  it.each(noItemStates.map((s) => [s.kind, s]))(
    "state '%s' has no items",
    (_kind, state) => {
      expect(hasItems(state)).toBe(false);
    },
  );
});

// ─── isDegraded ───────────────────────────────────────────────────────────────

describe("isDegraded", () => {
  it("true for stale", () => {
    expect(isDegraded(mkStale(ITEMS, CID, "2024-01-01T00:00:00Z"))).toBe(true);
  });

  it("true for partial", () => {
    expect(isDegraded(mkPartial(ITEMS, CID, ["bm25"]))).toBe(true);
  });

  it("true for offline", () => {
    expect(isDegraded(mkOffline(null, null))).toBe(true);
  });

  const nonDegradedStates: ListRequestState[] = [
    { kind: "empty" },
    mkLoading(CID, INTENT),
    mkReady(ITEMS, CID),
    mkUnauthorized(CID, null),
    mkTimeout(CID, INTENT),
    mkMalformed(CID, "x"),
    mkError(CID, "msg", true, INTENT),
    mkDeleted("x", CID),
    mkRecovery(CID, "diag"),
  ];

  it.each(nonDegradedStates.map((s) => [s.kind, s]))(
    "state '%s' is not degraded",
    (_kind, state) => {
      expect(isDegraded(state)).toBe(false);
    },
  );
});

// ─── isHardError ─────────────────────────────────────────────────────────────

describe("isHardError", () => {
  it("true for unauthorized", () => {
    expect(isHardError(mkUnauthorized(CID, null))).toBe(true);
  });

  it("true for malformed", () => {
    expect(isHardError(mkMalformed(CID, "x"))).toBe(true);
  });

  it("true for error", () => {
    expect(isHardError(mkError(CID, "msg", false, INTENT))).toBe(true);
  });

  it("true for recovery", () => {
    expect(isHardError(mkRecovery(CID, "diag"))).toBe(true);
  });

  const notHardErrorStates: ListRequestState[] = [
    { kind: "empty" },
    mkLoading(CID, INTENT),
    mkReady(ITEMS, CID),
    mkStale(ITEMS, CID, "2024-01-01T00:00:00Z"),
    mkPartial(ITEMS, CID, []),
    mkOffline(null, null),
    mkTimeout(CID, INTENT),
    mkDeleted("x", CID),
  ];

  it.each(notHardErrorStates.map((s) => [s.kind, s]))(
    "state '%s' is not a hard error",
    (_kind, state) => {
      expect(isHardError(state)).toBe(false);
    },
  );
});

// ─── Constructor shapes ───────────────────────────────────────────────────────

describe("constructor functions — correct shapes", () => {
  it("mkLoading produces loading state with correct fields", () => {
    const s = mkLoading("c1", "intent:search");
    expect(s).toEqual({ kind: "loading", correlationId: "c1", intent: "intent:search" });
  });

  it("mkReady produces ready state with correct fields", () => {
    const s = mkReady([1, 2], "c2");
    expect(s).toEqual({ kind: "ready", items: [1, 2], correlationId: "c2" });
  });

  it("mkPartial produces partial state with correct fields", () => {
    const s = mkPartial([3], "c3", ["bm25", "vector"]);
    expect(s).toEqual({
      kind: "partial",
      items: [3],
      correlationId: "c3",
      unavailableStrategies: ["bm25", "vector"],
    });
  });

  it("mkStale produces stale state with correct fields", () => {
    const s = mkStale([4], "c4", "2024-06-01T12:00:00Z");
    expect(s).toEqual({
      kind: "stale",
      items: [4],
      correlationId: "c4",
      staleSince: "2024-06-01T12:00:00Z",
    });
  });

  it("mkOffline with items produces offline state", () => {
    const s = mkOffline([5, 6], "c5");
    expect(s).toEqual({
      kind: "offline",
      preservedItems: [5, 6],
      lastKnownCorrelationId: "c5",
    });
  });

  it("mkOffline with nulls produces offline state with null fields", () => {
    const s = mkOffline(null, null);
    expect(s).toEqual({
      kind: "offline",
      preservedItems: null,
      lastKnownCorrelationId: null,
    });
  });

  it("mkUnauthorized produces unauthorized state with reason", () => {
    const s = mkUnauthorized("c6", "role:viewer");
    expect(s).toEqual({ kind: "unauthorized", correlationId: "c6", reason: "role:viewer" });
  });

  it("mkUnauthorized produces unauthorized state with null reason", () => {
    const s = mkUnauthorized("c6", null);
    expect(s).toEqual({ kind: "unauthorized", correlationId: "c6", reason: null });
  });

  it("mkTimeout always sets retryable=true", () => {
    const s = mkTimeout("c7", INTENT);
    expect(s).toEqual({ kind: "timeout", correlationId: "c7", retryable: true, intent: INTENT });
  });

  it("mkMalformed produces malformed state with details", () => {
    const s = mkMalformed("c8", "unexpected field x");
    expect(s).toEqual({ kind: "malformed", correlationId: "c8", details: "unexpected field x" });
  });

  it("mkError produces error state with all fields", () => {
    const s = mkError("c9", "db timeout", true, INTENT);
    expect(s).toEqual({
      kind: "error",
      correlationId: "c9",
      message: "db timeout",
      retryable: true,
      intent: INTENT,
    });
  });

  it("mkDeleted produces deleted state", () => {
    const s = mkDeleted("item-99", "c10");
    expect(s).toEqual({ kind: "deleted", itemId: "item-99", correlationId: "c10" });
  });

  it("mkRecovery produces recovery state with diagnostics", () => {
    const s = mkRecovery("c11", "replica lag detected");
    expect(s).toEqual({ kind: "recovery", correlationId: "c11", diagnostics: "replica lag detected" });
  });
});

// ─── All states covered in STATES fixture ────────────────────────────────────

describe("STATES fixture covers all 12 kinds", () => {
  it("all 12 kinds are present", () => {
    const kinds = new Set(STATES.map((s) => s.kind));
    for (const k of ALL_KINDS) {
      expect(kinds.has(k)).toBe(true);
    }
  });
});
