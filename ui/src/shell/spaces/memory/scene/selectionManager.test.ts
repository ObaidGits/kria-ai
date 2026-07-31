/**
 * selectionManager.test.ts
 *
 * Unit tests for selectionReducer and the disjoint click model:
 *   • SINGLE_CLICK sets selectedId (only).
 *   • DOUBLE_CLICK does NOT change selectedId (expand/fit is canvas-layer only).
 *   • REFRESH removes a stale selection (id absent from newNodeIds).
 *   • REFRESH retains a valid selection (id present in newNodeIds).
 *   • REFRESH announces re-resolution when selection is retained.
 *   • REFRESH announces removal when selection is cleared.
 *   • CLOSE clears selectedId.
 *   • CLOSE with different reasons produces distinct announcements.
 *
 * Requirements: MGR-013, MGR-014, MGR-016; F4.1 invariants.
 */

import { describe, it, expect } from "vitest";
import {
  selectionReducer,
  DOUBLE_CLICK_THRESHOLD_MS,
  type SelectionState,
  type SelectionEvent,
} from "./selectionManager";

// ─── Helpers ──────────────────────────────────────────────────────────────────

function emptyState(): SelectionState {
  return { selectedId: null, lastAnnouncement: null };
}

function selectedState(nodeId: string, announcement = `Selected: ${nodeId}`): SelectionState {
  return { selectedId: nodeId, lastAnnouncement: announcement };
}

// ─── DOUBLE_CLICK_THRESHOLD_MS ────────────────────────────────────────────────

describe("DOUBLE_CLICK_THRESHOLD_MS", () => {
  it("is exported as a positive number", () => {
    expect(typeof DOUBLE_CLICK_THRESHOLD_MS).toBe("number");
    expect(DOUBLE_CLICK_THRESHOLD_MS).toBeGreaterThan(0);
  });

  it("is 300 ms per spec", () => {
    expect(DOUBLE_CLICK_THRESHOLD_MS).toBe(300);
  });
});

// ─── SINGLE_CLICK ─────────────────────────────────────────────────────────────

describe("selectionReducer — SINGLE_CLICK", () => {
  it("sets selectedId to the clicked nodeId", () => {
    const state = emptyState();
    const event: SelectionEvent = { type: "SINGLE_CLICK", nodeId: "node-abc" };
    const next = selectionReducer(state, event);
    expect(next.selectedId).toBe("node-abc");
  });

  it("sets a non-null lastAnnouncement on selection", () => {
    const state = emptyState();
    const event: SelectionEvent = { type: "SINGLE_CLICK", nodeId: "node-abc" };
    const next = selectionReducer(state, event);
    expect(next.lastAnnouncement).not.toBeNull();
  });

  it("announcement includes the nodeId", () => {
    const state = emptyState();
    const event: SelectionEvent = { type: "SINGLE_CLICK", nodeId: "node-xyz" };
    const next = selectionReducer(state, event);
    expect(next.lastAnnouncement).toContain("node-xyz");
  });

  it("announcement starts with 'Selected:'", () => {
    const state = emptyState();
    const event: SelectionEvent = { type: "SINGLE_CLICK", nodeId: "node-1" };
    const next = selectionReducer(state, event);
    expect(next.lastAnnouncement).toMatch(/^Selected:/);
  });

  it("replaces an existing selection with the new nodeId", () => {
    const state = selectedState("old-node");
    const event: SelectionEvent = { type: "SINGLE_CLICK", nodeId: "new-node" };
    const next = selectionReducer(state, event);
    expect(next.selectedId).toBe("new-node");
  });

  it("does not mutate the input state", () => {
    const state = emptyState();
    const frozen = Object.freeze({ ...state });
    const event: SelectionEvent = { type: "SINGLE_CLICK", nodeId: "node-1" };
    expect(() => selectionReducer(frozen, event)).not.toThrow();
    expect(frozen.selectedId).toBeNull();
  });
});

// ─── DOUBLE_CLICK ─────────────────────────────────────────────────────────────

describe("selectionReducer — DOUBLE_CLICK", () => {
  it("does NOT change selectedId when nothing is selected", () => {
    const state = emptyState();
    const event: SelectionEvent = { type: "DOUBLE_CLICK", nodeId: "node-abc" };
    const next = selectionReducer(state, event);
    expect(next.selectedId).toBeNull();
  });

  it("does NOT change selectedId when a different node is selected", () => {
    const state = selectedState("other-node");
    const event: SelectionEvent = { type: "DOUBLE_CLICK", nodeId: "node-abc" };
    const next = selectionReducer(state, event);
    expect(next.selectedId).toBe("other-node");
  });

  it("does NOT change selectedId when the same node is double-clicked", () => {
    const state = selectedState("node-abc");
    const event: SelectionEvent = { type: "DOUBLE_CLICK", nodeId: "node-abc" };
    const next = selectionReducer(state, event);
    expect(next.selectedId).toBe("node-abc");
  });

  it("returns the same state reference (true no-op)", () => {
    const state = selectedState("node-abc");
    const event: SelectionEvent = { type: "DOUBLE_CLICK", nodeId: "node-abc" };
    const next = selectionReducer(state, event);
    // DOUBLE_CLICK is a pure no-op — same object reference expected.
    expect(next).toBe(state);
  });

  it("does NOT change lastAnnouncement", () => {
    const state = selectedState("node-abc", "Selected: node-abc");
    const event: SelectionEvent = { type: "DOUBLE_CLICK", nodeId: "node-abc" };
    const next = selectionReducer(state, event);
    expect(next.lastAnnouncement).toBe("Selected: node-abc");
  });

  it("does NOT change lastAnnouncement even when nothing was announced before", () => {
    const state = emptyState();
    const event: SelectionEvent = { type: "DOUBLE_CLICK", nodeId: "node-1" };
    const next = selectionReducer(state, event);
    expect(next.lastAnnouncement).toBeNull();
  });
});

// ─── REFRESH — stale selection removal ───────────────────────────────────────

describe("selectionReducer — REFRESH (stale selection removed)", () => {
  it("clears selectedId when id is NOT in newNodeIds", () => {
    const state = selectedState("old-node");
    const event: SelectionEvent = { type: "REFRESH", newNodeIds: ["node-a", "node-b"] };
    const next = selectionReducer(state, event);
    expect(next.selectedId).toBeNull();
  });

  it("sets removal announcement when selection is cleared", () => {
    const state = selectedState("old-node");
    const event: SelectionEvent = { type: "REFRESH", newNodeIds: ["node-a"] };
    const next = selectionReducer(state, event);
    expect(next.lastAnnouncement).toBe("Selection removed: node no longer in view");
  });

  it("clears selectedId when newNodeIds is empty", () => {
    const state = selectedState("node-1");
    const event: SelectionEvent = { type: "REFRESH", newNodeIds: [] };
    const next = selectionReducer(state, event);
    expect(next.selectedId).toBeNull();
  });

  it("announces removal when newNodeIds is empty and a node was selected", () => {
    const state = selectedState("node-1");
    const event: SelectionEvent = { type: "REFRESH", newNodeIds: [] };
    const next = selectionReducer(state, event);
    expect(next.lastAnnouncement).toBe("Selection removed: node no longer in view");
  });
});

// ─── REFRESH — valid selection retained ──────────────────────────────────────

describe("selectionReducer — REFRESH (valid selection retained)", () => {
  it("retains selectedId when id IS in newNodeIds", () => {
    const state = selectedState("node-keep");
    const event: SelectionEvent = {
      type: "REFRESH",
      newNodeIds: ["node-a", "node-keep", "node-b"],
    };
    const next = selectionReducer(state, event);
    expect(next.selectedId).toBe("node-keep");
  });

  it("announces re-resolution when selection is retained", () => {
    const state = selectedState("node-keep");
    const event: SelectionEvent = {
      type: "REFRESH",
      newNodeIds: ["node-keep", "node-other"],
    };
    const next = selectionReducer(state, event);
    expect(next.lastAnnouncement).toBe("Selection re-resolved");
  });

  it("retains selectedId when it is the only node in newNodeIds", () => {
    const state = selectedState("solo-node");
    const event: SelectionEvent = { type: "REFRESH", newNodeIds: ["solo-node"] };
    const next = selectionReducer(state, event);
    expect(next.selectedId).toBe("solo-node");
  });
});

// ─── REFRESH — no selection before refresh ────────────────────────────────────

describe("selectionReducer — REFRESH (no prior selection)", () => {
  it("returns the same state reference when no node was selected", () => {
    const state = emptyState();
    const event: SelectionEvent = { type: "REFRESH", newNodeIds: ["node-a", "node-b"] };
    const next = selectionReducer(state, event);
    // Nothing to remove or retain — must be a pure no-op.
    expect(next).toBe(state);
  });

  it("does not announce anything when there was no prior selection", () => {
    const state = emptyState();
    const event: SelectionEvent = { type: "REFRESH", newNodeIds: [] };
    const next = selectionReducer(state, event);
    expect(next.lastAnnouncement).toBeNull();
  });
});

// ─── CLOSE ───────────────────────────────────────────────────────────────────

describe("selectionReducer — CLOSE", () => {
  it("clears selectedId on close (user)", () => {
    const state = selectedState("node-1");
    const event: SelectionEvent = { type: "CLOSE", reason: "user" };
    const next = selectionReducer(state, event);
    expect(next.selectedId).toBeNull();
  });

  it("clears selectedId on close (navigation)", () => {
    const state = selectedState("node-1");
    const event: SelectionEvent = { type: "CLOSE", reason: "navigation" };
    const next = selectionReducer(state, event);
    expect(next.selectedId).toBeNull();
  });

  it("clears selectedId on close (error)", () => {
    const state = selectedState("node-1");
    const event: SelectionEvent = { type: "CLOSE", reason: "error" };
    const next = selectionReducer(state, event);
    expect(next.selectedId).toBeNull();
  });

  it("clears selectedId even when nothing was selected", () => {
    const state = emptyState();
    const event: SelectionEvent = { type: "CLOSE", reason: "user" };
    const next = selectionReducer(state, event);
    expect(next.selectedId).toBeNull();
  });

  it("sets 'Graph view closed' announcement for user reason", () => {
    const state = emptyState();
    const event: SelectionEvent = { type: "CLOSE", reason: "user" };
    const next = selectionReducer(state, event);
    expect(next.lastAnnouncement).toBe("Graph view closed");
  });

  it("sets a distinct announcement for navigation reason", () => {
    const state = emptyState();
    const event: SelectionEvent = { type: "CLOSE", reason: "navigation" };
    const next = selectionReducer(state, event);
    expect(next.lastAnnouncement).toBe("Graph view closed: navigated away");
  });

  it("sets a distinct announcement for error reason", () => {
    const state = emptyState();
    const event: SelectionEvent = { type: "CLOSE", reason: "error" };
    const next = selectionReducer(state, event);
    expect(next.lastAnnouncement).toBe("Graph view closed: error");
  });

  it("all three close reasons produce distinct announcements", () => {
    const makeClose = (reason: "user" | "navigation" | "error") =>
      selectionReducer(emptyState(), { type: "CLOSE", reason }).lastAnnouncement;

    const userMsg = makeClose("user");
    const navMsg = makeClose("navigation");
    const errMsg = makeClose("error");

    expect(userMsg).not.toBe(navMsg);
    expect(userMsg).not.toBe(errMsg);
    expect(navMsg).not.toBe(errMsg);
  });
});

// ─── Purity: no mutation of input state ──────────────────────────────────────

describe("selectionReducer — purity (no input mutation)", () => {
  it("does not mutate the state on SINGLE_CLICK", () => {
    const state = Object.freeze(emptyState());
    expect(() =>
      selectionReducer(state, { type: "SINGLE_CLICK", nodeId: "n1" }),
    ).not.toThrow();
  });

  it("does not mutate the state on REFRESH with removal", () => {
    const state = Object.freeze(selectedState("n1"));
    expect(() =>
      selectionReducer(state, { type: "REFRESH", newNodeIds: [] }),
    ).not.toThrow();
    expect(state.selectedId).toBe("n1");
  });

  it("does not mutate the state on CLOSE", () => {
    const state = Object.freeze(selectedState("n1"));
    expect(() =>
      selectionReducer(state, { type: "CLOSE", reason: "user" }),
    ).not.toThrow();
    expect(state.selectedId).toBe("n1");
  });
});

// ─── Integration: sequences ───────────────────────────────────────────────────

describe("selectionReducer — integration sequences", () => {
  it("SINGLE_CLICK → DOUBLE_CLICK preserves selection", () => {
    let state = emptyState();
    state = selectionReducer(state, { type: "SINGLE_CLICK", nodeId: "node-1" });
    expect(state.selectedId).toBe("node-1");

    // Double-click the same node for expand/fit — must not change selection.
    state = selectionReducer(state, { type: "DOUBLE_CLICK", nodeId: "node-1" });
    expect(state.selectedId).toBe("node-1");
  });

  it("SINGLE_CLICK → DOUBLE_CLICK on different node preserves original selection", () => {
    let state = emptyState();
    state = selectionReducer(state, { type: "SINGLE_CLICK", nodeId: "node-A" });
    state = selectionReducer(state, { type: "DOUBLE_CLICK", nodeId: "node-B" });
    // Selection is still node-A; double-click on node-B only expands/fits.
    expect(state.selectedId).toBe("node-A");
  });

  it("SINGLE_CLICK → REFRESH (node retained) → selection and re-resolution announcement", () => {
    let state = emptyState();
    state = selectionReducer(state, { type: "SINGLE_CLICK", nodeId: "node-1" });
    state = selectionReducer(state, { type: "REFRESH", newNodeIds: ["node-1", "node-2"] });
    expect(state.selectedId).toBe("node-1");
    expect(state.lastAnnouncement).toBe("Selection re-resolved");
  });

  it("SINGLE_CLICK → REFRESH (node removed) → null selection and removal announcement", () => {
    let state = emptyState();
    state = selectionReducer(state, { type: "SINGLE_CLICK", nodeId: "node-gone" });
    state = selectionReducer(state, { type: "REFRESH", newNodeIds: ["node-other"] });
    expect(state.selectedId).toBeNull();
    expect(state.lastAnnouncement).toBe("Selection removed: node no longer in view");
  });

  it("SINGLE_CLICK → CLOSE clears selection with appropriate announcement", () => {
    let state = emptyState();
    state = selectionReducer(state, { type: "SINGLE_CLICK", nodeId: "node-1" });
    state = selectionReducer(state, { type: "CLOSE", reason: "navigation" });
    expect(state.selectedId).toBeNull();
    expect(state.lastAnnouncement).toBe("Graph view closed: navigated away");
  });

  it("multiple SINGLE_CLICKs replace selection each time", () => {
    let state = emptyState();
    state = selectionReducer(state, { type: "SINGLE_CLICK", nodeId: "node-1" });
    state = selectionReducer(state, { type: "SINGLE_CLICK", nodeId: "node-2" });
    state = selectionReducer(state, { type: "SINGLE_CLICK", nodeId: "node-3" });
    expect(state.selectedId).toBe("node-3");
    expect(state.lastAnnouncement).toContain("node-3");
  });
});
