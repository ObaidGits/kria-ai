/**
 * listNavigation.test.ts — pure unit tests for listNavigation reducers.
 *
 * All tests run in pure TypeScript (no DOM, no SolidJS). Each test verifies
 * a specific invariant of the navigation state machine.
 *
 * Requirements: MGR-006, MGR-014, MGR-023–024, MGR-031;
 *   MGD-026, MGD-030, MGD-046; MG-H01, MG-H04, MG-H10–H12, MG-O05, MG-O25.
 */
import { describe, it, expect } from "vitest";

import {
  initialListNavigationState,
  applySort,
  applyFilter,
  applyNextPage,
  applyPreviousPage,
  applySelection,
  applyFocus,
  applyIntent,
  reconcileAfterRefetch,
  markPendingRefetch,
  clearPendingRefetch,
  type SortField,
  type ListNavigationState,
  type SortState,
  type FilterState,
  type NavigationIntent,
} from "./listNavigation";

// ─── Helpers ──────────────────────────────────────────────────────────────────

/** Quick helper to get a fresh baseline state. */
function fresh(): ListNavigationState {
  return initialListNavigationState();
}

// ─── initialListNavigationState ──────────────────────────────────────────────

describe("initialListNavigationState", () => {
  it("returns correct defaults", () => {
    const state = fresh();
    expect(state.sort.field).toBe("name");
    expect(state.sort.direction).toBe("asc");
    expect(state.filter.kinds).toEqual([]);
    expect(state.filter.truthStates).toEqual([]);
    expect(state.filter.authorityClasses).toEqual([]);
    expect(state.filter.query).toBe("");
    expect(state.cursor).toBeNull();
    expect(state.selectedId).toBeNull();
    expect(state.focusedId).toBeNull();
    expect(state.intent.type).toBe("list");
    expect(state.pendingRefetch).toBe(false);
  });

  it("returns a new object on each call (no shared references)", () => {
    const a = fresh();
    const b = fresh();
    expect(a).not.toBe(b);
    expect(a.sort).not.toBe(b.sort);
    expect(a.filter).not.toBe(b.filter);
  });
});

// ─── applySort ────────────────────────────────────────────────────────────────

describe("applySort", () => {
  it("changes sort field and direction", () => {
    const state = fresh();
    const next = applySort(state, { field: "kind", direction: "desc" });
    expect(next.sort.field).toBe("kind");
    expect(next.sort.direction).toBe("desc");
  });

  it("resets cursor to null", () => {
    const state = applyNextPage(fresh(), "tok_abc");
    expect(state.cursor).toBe("tok_abc");
    const next = applySort(state, { field: "revision", direction: "asc" });
    expect(next.cursor).toBeNull();
  });

  it("does not mutate the input state", () => {
    const state = fresh();
    const before = JSON.stringify(state);
    applySort(state, { field: "kind", direction: "desc" });
    expect(JSON.stringify(state)).toBe(before);
  });

  it("returns a new object", () => {
    const state = fresh();
    const next = applySort(state, { field: "kind", direction: "desc" });
    expect(next).not.toBe(state);
  });

  it("supports all SortField values", () => {
    const fields: SortField[] = [
      "name",
      "kind",
      "truth-state",
      "revision",
      "valid-time-start",
    ];
    for (const field of fields) {
      const next = applySort(fresh(), { field, direction: "asc" });
      expect(next.sort.field).toBe(field);
      expect(next.cursor).toBeNull();
    }
  });

  it("preserves filter when sorting", () => {
    const withFilter = applyFilter(fresh(), {
      kinds: ["entity"],
      truthStates: [],
      authorityClasses: [],
      query: "hello",
    });
    const next = applySort(withFilter, { field: "revision", direction: "desc" });
    expect(next.filter.kinds).toEqual(["entity"]);
    expect(next.filter.query).toBe("hello");
  });
});

// ─── applyFilter ──────────────────────────────────────────────────────────────

describe("applyFilter", () => {
  it("changes filter", () => {
    const state = fresh();
    const filter: FilterState = {
      kinds: ["memory", "entity"],
      truthStates: ["Current"],
      authorityClasses: ["personal"],
      query: "test",
    };
    const next = applyFilter(state, filter);
    expect(next.filter.kinds).toEqual(["memory", "entity"]);
    expect(next.filter.truthStates).toEqual(["Current"]);
    expect(next.filter.authorityClasses).toEqual(["personal"]);
    expect(next.filter.query).toBe("test");
  });

  it("resets cursor to null", () => {
    const state = applyNextPage(fresh(), "tok_xyz");
    const next = applyFilter(state, {
      kinds: [],
      truthStates: [],
      authorityClasses: [],
      query: "",
    });
    expect(next.cursor).toBeNull();
  });

  it("handles partial filter (only kinds specified)", () => {
    const state = fresh();
    const next = applyFilter(state, {
      kinds: ["goal"],
      truthStates: [],
      authorityClasses: [],
      query: "",
    });
    expect(next.filter.kinds).toEqual(["goal"]);
    expect(next.filter.truthStates).toEqual([]);
    expect(next.filter.authorityClasses).toEqual([]);
    expect(next.filter.query).toBe("");
  });

  it("does not mutate the input state", () => {
    const state = fresh();
    const before = JSON.stringify(state);
    applyFilter(state, { kinds: ["x"], truthStates: [], authorityClasses: [], query: "" });
    expect(JSON.stringify(state)).toBe(before);
  });

  it("returns a new object", () => {
    const state = fresh();
    const next = applyFilter(state, { kinds: [], truthStates: [], authorityClasses: [], query: "" });
    expect(next).not.toBe(state);
  });
});

// ─── applyNextPage ────────────────────────────────────────────────────────────

describe("applyNextPage", () => {
  it("sets cursor token", () => {
    const state = fresh();
    const next = applyNextPage(state, "cursor_page2");
    expect(next.cursor).toBe("cursor_page2");
  });

  it("keeps sort unchanged", () => {
    const state = applySort(fresh(), { field: "revision", direction: "desc" });
    const next = applyNextPage(state, "tok");
    expect(next.sort.field).toBe("revision");
    expect(next.sort.direction).toBe("desc");
  });

  it("keeps filter unchanged", () => {
    const state = applyFilter(fresh(), {
      kinds: ["memory"],
      truthStates: [],
      authorityClasses: [],
      query: "q",
    });
    const next = applyNextPage(state, "tok");
    expect(next.filter.kinds).toEqual(["memory"]);
    expect(next.filter.query).toBe("q");
  });

  it("does not mutate the input state", () => {
    const state = fresh();
    const before = JSON.stringify(state);
    applyNextPage(state, "tok");
    expect(JSON.stringify(state)).toBe(before);
  });

  it("returns a new object", () => {
    const state = fresh();
    const next = applyNextPage(state, "tok");
    expect(next).not.toBe(state);
  });
});

// ─── applyPreviousPage ────────────────────────────────────────────────────────

describe("applyPreviousPage", () => {
  it("sets cursor token (going back)", () => {
    const state = applyNextPage(fresh(), "cursor_page3");
    const next = applyPreviousPage(state, "cursor_page2");
    expect(next.cursor).toBe("cursor_page2");
  });

  it("keeps sort unchanged", () => {
    const state = applySort(fresh(), { field: "kind", direction: "asc" });
    const next = applyPreviousPage(state, "back_tok");
    expect(next.sort.field).toBe("kind");
  });

  it("keeps filter unchanged", () => {
    const state = applyFilter(fresh(), {
      kinds: ["entity"],
      truthStates: [],
      authorityClasses: [],
      query: "",
    });
    const next = applyPreviousPage(state, "back_tok");
    expect(next.filter.kinds).toEqual(["entity"]);
  });

  it("does not mutate the input state", () => {
    const state = fresh();
    const before = JSON.stringify(state);
    applyPreviousPage(state, "back_tok");
    expect(JSON.stringify(state)).toBe(before);
  });

  it("returns a new object", () => {
    const state = fresh();
    const next = applyPreviousPage(state, "back_tok");
    expect(next).not.toBe(state);
  });
});

// ─── applySelection ───────────────────────────────────────────────────────────

describe("applySelection", () => {
  it("sets selectedId", () => {
    const next = applySelection(fresh(), "item-001");
    expect(next.selectedId).toBe("item-001");
  });

  it("clears selectedId when passed null", () => {
    const state = applySelection(fresh(), "item-001");
    const next = applySelection(state, null);
    expect(next.selectedId).toBeNull();
  });

  it("does not affect focusedId", () => {
    const state = applyFocus(fresh(), "focus-001");
    const next = applySelection(state, "item-001");
    expect(next.focusedId).toBe("focus-001");
  });

  it("does not mutate the input state", () => {
    const state = fresh();
    const before = JSON.stringify(state);
    applySelection(state, "item-001");
    expect(JSON.stringify(state)).toBe(before);
  });

  it("returns a new object", () => {
    const state = fresh();
    const next = applySelection(state, "item-001");
    expect(next).not.toBe(state);
  });
});

// ─── applyFocus ───────────────────────────────────────────────────────────────

describe("applyFocus", () => {
  it("sets focusedId", () => {
    const next = applyFocus(fresh(), "item-002");
    expect(next.focusedId).toBe("item-002");
  });

  it("clears focusedId when passed null", () => {
    const state = applyFocus(fresh(), "item-002");
    const next = applyFocus(state, null);
    expect(next.focusedId).toBeNull();
  });

  it("does not affect selectedId", () => {
    const state = applySelection(fresh(), "sel-001");
    const next = applyFocus(state, "focus-001");
    expect(next.selectedId).toBe("sel-001");
  });

  it("does not mutate the input state", () => {
    const state = fresh();
    const before = JSON.stringify(state);
    applyFocus(state, "item-002");
    expect(JSON.stringify(state)).toBe(before);
  });

  it("returns a new object", () => {
    const state = fresh();
    const next = applyFocus(state, "item-002");
    expect(next).not.toBe(state);
  });
});

// ─── applyIntent ──────────────────────────────────────────────────────────────

describe("applyIntent", () => {
  it("sets intent type to list", () => {
    const state = applyIntent(fresh(), { type: "expand", itemId: "x" });
    const next = applyIntent(state, { type: "list" });
    expect(next.intent.type).toBe("list");
    expect(next.intent.itemId).toBeUndefined();
  });

  it("sets intent type to expand with itemId", () => {
    const next = applyIntent(fresh(), { type: "expand", itemId: "node-abc" });
    expect(next.intent.type).toBe("expand");
    expect(next.intent.itemId).toBe("node-abc");
  });

  it("sets intent type to path with sourceId and targetId", () => {
    const next = applyIntent(fresh(), {
      type: "path",
      sourceId: "src-001",
      targetId: "tgt-002",
    });
    expect(next.intent.type).toBe("path");
    expect(next.intent.sourceId).toBe("src-001");
    expect(next.intent.targetId).toBe("tgt-002");
  });

  it("sets intent type to trace with traceId", () => {
    const next = applyIntent(fresh(), { type: "trace", traceId: "trace-999" });
    expect(next.intent.type).toBe("trace");
    expect(next.intent.traceId).toBe("trace-999");
  });

  it("does not mutate the input state", () => {
    const state = fresh();
    const before = JSON.stringify(state);
    applyIntent(state, { type: "expand", itemId: "x" });
    expect(JSON.stringify(state)).toBe(before);
  });

  it("returns a new object", () => {
    const state = fresh();
    const next = applyIntent(state, { type: "expand", itemId: "x" });
    expect(next).not.toBe(state);
  });
});

// ─── reconcileAfterRefetch ────────────────────────────────────────────────────

describe("reconcileAfterRefetch", () => {
  it("keeps selection when selectedId is in availableIds", () => {
    const state = applySelection(fresh(), "item-A");
    const { nextState, selectionLost } = reconcileAfterRefetch(state, [
      "item-A",
      "item-B",
    ]);
    expect(nextState.selectedId).toBe("item-A");
    expect(selectionLost).toBe(false);
  });

  it("clears selection and returns selectionLost=true when selectedId not in availableIds", () => {
    const state = applySelection(fresh(), "item-A");
    const { nextState, selectionLost } = reconcileAfterRefetch(state, [
      "item-B",
      "item-C",
    ]);
    expect(nextState.selectedId).toBeNull();
    expect(selectionLost).toBe(true);
  });

  it("keeps focus when focusedId is in availableIds", () => {
    const state = applyFocus(fresh(), "item-B");
    const { nextState, focusLost } = reconcileAfterRefetch(state, [
      "item-A",
      "item-B",
    ]);
    expect(nextState.focusedId).toBe("item-B");
    expect(focusLost).toBe(false);
  });

  it("clears focus and returns focusLost=true when focusedId not in availableIds", () => {
    const state = applyFocus(fresh(), "item-B");
    const { nextState, focusLost } = reconcileAfterRefetch(state, [
      "item-A",
      "item-C",
    ]);
    expect(nextState.focusedId).toBeNull();
    expect(focusLost).toBe(true);
  });

  it("handles null selectedId gracefully (no selectionLost)", () => {
    const state = fresh(); // selectedId is null
    const { nextState, selectionLost } = reconcileAfterRefetch(state, [
      "item-A",
    ]);
    expect(nextState.selectedId).toBeNull();
    expect(selectionLost).toBe(false);
  });

  it("handles null focusedId gracefully (no focusLost)", () => {
    const state = fresh(); // focusedId is null
    const { nextState, focusLost } = reconcileAfterRefetch(state, ["item-A"]);
    expect(nextState.focusedId).toBeNull();
    expect(focusLost).toBe(false);
  });

  it("clears both selection and focus when neither is in availableIds", () => {
    const state = { ...applySelection(fresh(), "sel-X"), focusedId: "foc-Y" };
    const { nextState, selectionLost, focusLost } = reconcileAfterRefetch(
      state,
      ["item-Z"],
    );
    expect(nextState.selectedId).toBeNull();
    expect(nextState.focusedId).toBeNull();
    expect(selectionLost).toBe(true);
    expect(focusLost).toBe(true);
  });

  it("clears pendingRefetch on reconcile", () => {
    const state = markPendingRefetch(applySelection(fresh(), "item-A"));
    expect(state.pendingRefetch).toBe(true);
    const { nextState } = reconcileAfterRefetch(state, ["item-A"]);
    expect(nextState.pendingRefetch).toBe(false);
  });

  it("does not mutate the input state", () => {
    const state = applySelection(fresh(), "item-A");
    const before = JSON.stringify(state);
    reconcileAfterRefetch(state, ["item-B"]);
    expect(JSON.stringify(state)).toBe(before);
  });

  it("handles empty availableIds (both lost)", () => {
    const state = {
      ...applySelection(fresh(), "sel-001"),
      focusedId: "foc-001",
    };
    const { nextState, selectionLost, focusLost } = reconcileAfterRefetch(
      state,
      [],
    );
    expect(nextState.selectedId).toBeNull();
    expect(nextState.focusedId).toBeNull();
    expect(selectionLost).toBe(true);
    expect(focusLost).toBe(true);
  });
});

// ─── markPendingRefetch / clearPendingRefetch ─────────────────────────────────

describe("markPendingRefetch", () => {
  it("sets pendingRefetch to true", () => {
    const next = markPendingRefetch(fresh());
    expect(next.pendingRefetch).toBe(true);
  });

  it("does not mutate the input state", () => {
    const state = fresh();
    const before = JSON.stringify(state);
    markPendingRefetch(state);
    expect(JSON.stringify(state)).toBe(before);
  });

  it("returns a new object", () => {
    const state = fresh();
    const next = markPendingRefetch(state);
    expect(next).not.toBe(state);
  });
});

describe("clearPendingRefetch", () => {
  it("sets pendingRefetch to false", () => {
    const state = markPendingRefetch(fresh());
    const next = clearPendingRefetch(state);
    expect(next.pendingRefetch).toBe(false);
  });

  it("does not mutate the input state", () => {
    const state = markPendingRefetch(fresh());
    const before = JSON.stringify(state);
    clearPendingRefetch(state);
    expect(JSON.stringify(state)).toBe(before);
  });

  it("returns a new object", () => {
    const state = markPendingRefetch(fresh());
    const next = clearPendingRefetch(state);
    expect(next).not.toBe(state);
  });
});

// ─── Immutability: all reducers return new objects ────────────────────────────

describe("immutability — all reducers return new objects", () => {
  it("applySort returns a new object", () => {
    const s = fresh();
    expect(applySort(s, { field: "kind", direction: "desc" })).not.toBe(s);
  });

  it("applyFilter returns a new object", () => {
    const s = fresh();
    expect(
      applyFilter(s, { kinds: [], truthStates: [], authorityClasses: [], query: "" }),
    ).not.toBe(s);
  });

  it("applyNextPage returns a new object", () => {
    const s = fresh();
    expect(applyNextPage(s, "tok")).not.toBe(s);
  });

  it("applyPreviousPage returns a new object", () => {
    const s = fresh();
    expect(applyPreviousPage(s, "tok")).not.toBe(s);
  });

  it("applySelection returns a new object", () => {
    const s = fresh();
    expect(applySelection(s, "id")).not.toBe(s);
  });

  it("applyFocus returns a new object", () => {
    const s = fresh();
    expect(applyFocus(s, "id")).not.toBe(s);
  });

  it("applyIntent returns a new object", () => {
    const s = fresh();
    expect(applyIntent(s, { type: "list" })).not.toBe(s);
  });

  it("markPendingRefetch returns a new object", () => {
    const s = fresh();
    expect(markPendingRefetch(s)).not.toBe(s);
  });

  it("clearPendingRefetch returns a new object", () => {
    const s = markPendingRefetch(fresh());
    expect(clearPendingRefetch(s)).not.toBe(s);
  });
});

// ─── Combined: filter → sort → paginate maintains correct state ───────────────

describe("combined operations", () => {
  it("filter then sort then paginate maintains correct state", () => {
    let state = fresh();

    // 1. Apply a filter
    state = applyFilter(state, {
      kinds: ["memory"],
      truthStates: ["Current"],
      authorityClasses: [],
      query: "knowledge",
    });
    expect(state.cursor).toBeNull();
    expect(state.filter.kinds).toEqual(["memory"]);

    // 2. Apply a sort (should reset cursor)
    state = applySort(state, { field: "revision", direction: "desc" });
    expect(state.cursor).toBeNull();
    expect(state.sort.field).toBe("revision");
    expect(state.sort.direction).toBe("desc");
    // Filter must be preserved
    expect(state.filter.kinds).toEqual(["memory"]);
    expect(state.filter.query).toBe("knowledge");

    // 3. Paginate to next page
    state = applyNextPage(state, "cursor_p2");
    expect(state.cursor).toBe("cursor_p2");
    // Sort and filter still intact
    expect(state.sort.field).toBe("revision");
    expect(state.filter.kinds).toEqual(["memory"]);

    // 4. Applying a new filter resets cursor
    state = applyFilter(state, {
      kinds: ["entity"],
      truthStates: [],
      authorityClasses: [],
      query: "",
    });
    expect(state.cursor).toBeNull();
    expect(state.filter.kinds).toEqual(["entity"]);
    // Sort preserved
    expect(state.sort.field).toBe("revision");
  });

  it("selection and focus survive sort/filter changes", () => {
    let state = applySelection(fresh(), "sel-001");
    state = { ...state, focusedId: "foc-001" };

    state = applySort(state, { field: "kind", direction: "asc" });
    expect(state.selectedId).toBe("sel-001");
    expect(state.focusedId).toBe("foc-001");

    state = applyFilter(state, {
      kinds: [],
      truthStates: [],
      authorityClasses: [],
      query: "q",
    });
    expect(state.selectedId).toBe("sel-001");
    expect(state.focusedId).toBe("foc-001");
  });

  it("full lifecycle: mark pending → reconcile preserves selection", () => {
    let state = applySelection(fresh(), "item-keep");
    state = markPendingRefetch(state);
    expect(state.pendingRefetch).toBe(true);

    const { nextState, selectionLost, focusLost } = reconcileAfterRefetch(
      state,
      ["item-keep", "item-other"],
    );
    expect(nextState.selectedId).toBe("item-keep");
    expect(nextState.pendingRefetch).toBe(false);
    expect(selectionLost).toBe(false);
    expect(focusLost).toBe(false);
  });

  it("intent changes do not reset sort/filter/cursor", () => {
    let state = applyFilter(fresh(), {
      kinds: ["source"],
      truthStates: [],
      authorityClasses: [],
      query: "",
    });
    state = applyNextPage(state, "tok_p2");
    state = applySort(state, { field: "kind", direction: "asc" });
    // After sort cursor was reset, set it again
    state = applyNextPage(state, "tok_p2");

    const next = applyIntent(state, {
      type: "path",
      sourceId: "A",
      targetId: "B",
    });
    expect(next.intent.type).toBe("path");
    expect(next.filter.kinds).toEqual(["source"]);
    expect(next.sort.field).toBe("kind");
    expect(next.cursor).toBe("tok_p2");
  });
});
