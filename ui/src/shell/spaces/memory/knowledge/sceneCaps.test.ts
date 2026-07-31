/**
 * sceneCaps.test.ts — pure unit tests for sceneCaps.ts
 *
 * No DOM, no JSX, no SolidJS — pure TypeScript / Vitest.
 *
 * Covers:
 *   • applyBalancedCaps: no truncation when under all caps
 *   • applyBalancedCaps: truncated=true / reason=node-cap when nodes > 240
 *   • applyBalancedCaps: truncated=true / reason=edge-cap when edges > 360
 *   • applyBalancedCaps: labels capped at 80
 *   • applyHardCaps: slices at 500 nodes
 *   • applyHardCaps: slices at 750 edges
 *   • applyHardCaps: preserves arrays under hard caps unchanged
 *   • No input arrays are mutated
 *
 * IDs: MGD-003; MG-M09, MG-O19.
 */
import { describe, it, expect } from "vitest";

import {
  applyBalancedCaps,
  applyHardCaps,
  BALANCED_NODE_CAP,
  BALANCED_EDGE_CAP,
  BALANCED_LABEL_CAP,
  HARD_NODE_CAP,
  HARD_EDGE_CAP,
} from "./sceneCaps";

// ─── Helpers ──────────────────────────────────────────────────────────────────

function ids(n: number, prefix = "id"): string[] {
  return Array.from({ length: n }, (_, i) => `${prefix}-${i}`);
}

// ─── applyBalancedCaps — under cap ────────────────────────────────────────────

describe("applyBalancedCaps — no truncation when under all caps", () => {
  it("returns truncated=false when all inputs are under balanced caps", () => {
    const result = applyBalancedCaps(
      ids(10, "node"),
      ids(20, "edge"),
      ids(5, "label"),
    );
    expect(result.truncated).toBe(false);
    expect(result.truncationReason).toBe("none");
    expect(result.nodeCapExceeded).toBe(false);
    expect(result.edgeCapExceeded).toBe(false);
  });

  it("preserves all node IDs when under cap", () => {
    const nodes = ids(5, "n");
    const result = applyBalancedCaps(nodes, [], []);
    expect(result.nodeIds).toEqual(nodes);
  });

  it("preserves all edge IDs when under cap", () => {
    const edges = ids(10, "e");
    const result = applyBalancedCaps([], edges, []);
    expect(result.edgeIds).toEqual(edges);
  });

  it("preserves all label IDs when under cap", () => {
    const labels = ids(3, "l");
    const result = applyBalancedCaps([], [], labels);
    expect(result.labelIds).toEqual(labels);
  });

  it("exact balanced cap limits do not trigger truncation", () => {
    const result = applyBalancedCaps(
      ids(BALANCED_NODE_CAP, "n"),
      ids(BALANCED_EDGE_CAP, "e"),
      ids(BALANCED_LABEL_CAP, "l"),
    );
    expect(result.truncated).toBe(false);
    expect(result.nodeIds.length).toBe(BALANCED_NODE_CAP);
    expect(result.edgeIds.length).toBe(BALANCED_EDGE_CAP);
    expect(result.labelIds.length).toBe(BALANCED_LABEL_CAP);
  });
});

// ─── applyBalancedCaps — node cap exceeded ────────────────────────────────────

describe("applyBalancedCaps — node cap", () => {
  it("truncated=true when nodes > BALANCED_NODE_CAP", () => {
    const result = applyBalancedCaps(ids(BALANCED_NODE_CAP + 1, "n"), [], []);
    expect(result.truncated).toBe(true);
    expect(result.nodeCapExceeded).toBe(true);
  });

  it("truncationReason = node-cap", () => {
    const result = applyBalancedCaps(ids(300, "n"), [], []);
    expect(result.truncationReason).toBe("node-cap");
  });

  it("nodeIds sliced at BALANCED_NODE_CAP", () => {
    const result = applyBalancedCaps(ids(300, "n"), [], []);
    expect(result.nodeIds.length).toBe(BALANCED_NODE_CAP);
  });

  it("first BALANCED_NODE_CAP items survive in order", () => {
    const nodes = ids(300, "n");
    const result = applyBalancedCaps(nodes, [], []);
    expect(result.nodeIds).toEqual(nodes.slice(0, BALANCED_NODE_CAP));
  });

  it("edgeIds unaffected when only nodes exceed cap", () => {
    const edges = ids(10, "e");
    const result = applyBalancedCaps(ids(300, "n"), edges, []);
    expect(result.edgeIds).toEqual(edges);
    expect(result.edgeCapExceeded).toBe(false);
  });
});

// ─── applyBalancedCaps — edge cap exceeded ────────────────────────────────────

describe("applyBalancedCaps — edge cap", () => {
  it("truncated=true when edges > BALANCED_EDGE_CAP", () => {
    const result = applyBalancedCaps([], ids(BALANCED_EDGE_CAP + 1, "e"), []);
    expect(result.truncated).toBe(true);
    expect(result.edgeCapExceeded).toBe(true);
  });

  it("truncationReason = edge-cap when only edges exceed", () => {
    const result = applyBalancedCaps([], ids(400, "e"), []);
    expect(result.truncationReason).toBe("edge-cap");
  });

  it("edgeIds sliced at BALANCED_EDGE_CAP", () => {
    const result = applyBalancedCaps([], ids(500, "e"), []);
    expect(result.edgeIds.length).toBe(BALANCED_EDGE_CAP);
  });

  it("first BALANCED_EDGE_CAP items survive in order", () => {
    const edges = ids(500, "e");
    const result = applyBalancedCaps([], edges, []);
    expect(result.edgeIds).toEqual(edges.slice(0, BALANCED_EDGE_CAP));
  });

  it("truncationReason is node-cap when both nodes and edges exceed (node checked first)", () => {
    const result = applyBalancedCaps(
      ids(BALANCED_NODE_CAP + 1, "n"),
      ids(BALANCED_EDGE_CAP + 1, "e"),
      [],
    );
    expect(result.truncationReason).toBe("node-cap");
    expect(result.nodeCapExceeded).toBe(true);
    expect(result.edgeCapExceeded).toBe(true);
  });
});

// ─── applyBalancedCaps — label cap ───────────────────────────────────────────

describe("applyBalancedCaps — label cap", () => {
  it("labels capped at BALANCED_LABEL_CAP", () => {
    const labels = ids(200, "l");
    const result = applyBalancedCaps([], [], labels);
    expect(result.labelIds.length).toBe(BALANCED_LABEL_CAP);
  });

  it("truncated=true when labels > BALANCED_LABEL_CAP", () => {
    const result = applyBalancedCaps([], [], ids(BALANCED_LABEL_CAP + 1, "l"));
    expect(result.truncated).toBe(true);
  });

  it("truncationReason = label-cap when only labels exceed", () => {
    const result = applyBalancedCaps([], [], ids(200, "l"));
    expect(result.truncationReason).toBe("label-cap");
  });

  it("first BALANCED_LABEL_CAP labels survive in order", () => {
    const labels = ids(200, "l");
    const result = applyBalancedCaps([], [], labels);
    expect(result.labelIds).toEqual(labels.slice(0, BALANCED_LABEL_CAP));
  });

  it("exact BALANCED_LABEL_CAP labels — no truncation", () => {
    const result = applyBalancedCaps([], [], ids(BALANCED_LABEL_CAP, "l"));
    expect(result.truncated).toBe(false);
    expect(result.labelIds.length).toBe(BALANCED_LABEL_CAP);
  });
});

// ─── applyBalancedCaps — immutability ────────────────────────────────────────

describe("applyBalancedCaps — immutability", () => {
  it("does not mutate the input arrays", () => {
    const nodes = ids(300, "n");
    const edges = ids(400, "e");
    const labels = ids(200, "l");
    const nodesBefore = [...nodes];
    const edgesBefore = [...edges];
    const labelsBefore = [...labels];
    applyBalancedCaps(nodes, edges, labels);
    expect(nodes).toEqual(nodesBefore);
    expect(edges).toEqual(edgesBefore);
    expect(labels).toEqual(labelsBefore);
  });
});

// ─── applyHardCaps — over hard cap ───────────────────────────────────────────

describe("applyHardCaps — slicing", () => {
  it("slices nodeIds at HARD_NODE_CAP", () => {
    const nodes = ids(HARD_NODE_CAP + 50, "n");
    const result = applyHardCaps(nodes, []);
    expect(result.nodeIds.length).toBe(HARD_NODE_CAP);
  });

  it("first HARD_NODE_CAP items survive in order", () => {
    const nodes = ids(HARD_NODE_CAP + 50, "n");
    const result = applyHardCaps(nodes, []);
    expect(result.nodeIds).toEqual(nodes.slice(0, HARD_NODE_CAP));
  });

  it("slices edgeIds at HARD_EDGE_CAP", () => {
    const edges = ids(HARD_EDGE_CAP + 50, "e");
    const result = applyHardCaps([], edges);
    expect(result.edgeIds.length).toBe(HARD_EDGE_CAP);
  });

  it("first HARD_EDGE_CAP items survive in order", () => {
    const edges = ids(HARD_EDGE_CAP + 50, "e");
    const result = applyHardCaps([], edges);
    expect(result.edgeIds).toEqual(edges.slice(0, HARD_EDGE_CAP));
  });

  it("slices both simultaneously", () => {
    const result = applyHardCaps(
      ids(HARD_NODE_CAP + 100, "n"),
      ids(HARD_EDGE_CAP + 100, "e"),
    );
    expect(result.nodeIds.length).toBe(HARD_NODE_CAP);
    expect(result.edgeIds.length).toBe(HARD_EDGE_CAP);
  });
});

// ─── applyHardCaps — under hard cap ──────────────────────────────────────────

describe("applyHardCaps — preserves arrays under hard caps", () => {
  it("preserves node array unchanged when under HARD_NODE_CAP", () => {
    const nodes = ids(10, "n");
    const result = applyHardCaps(nodes, []);
    expect(result.nodeIds).toEqual(nodes);
  });

  it("preserves edge array unchanged when under HARD_EDGE_CAP", () => {
    const edges = ids(20, "e");
    const result = applyHardCaps([], edges);
    expect(result.edgeIds).toEqual(edges);
  });

  it("returns new array objects even when under cap (defensive copy)", () => {
    const nodes = ids(5, "n");
    const edges = ids(5, "e");
    const result = applyHardCaps(nodes, edges);
    expect(result.nodeIds).not.toBe(nodes);
    expect(result.edgeIds).not.toBe(edges);
  });

  it("exact hard cap limits — no slicing", () => {
    const result = applyHardCaps(ids(HARD_NODE_CAP, "n"), ids(HARD_EDGE_CAP, "e"));
    expect(result.nodeIds.length).toBe(HARD_NODE_CAP);
    expect(result.edgeIds.length).toBe(HARD_EDGE_CAP);
  });
});

// ─── applyHardCaps — immutability ────────────────────────────────────────────

describe("applyHardCaps — immutability", () => {
  it("does not mutate the input arrays", () => {
    const nodes = ids(HARD_NODE_CAP + 10, "n");
    const edges = ids(HARD_EDGE_CAP + 10, "e");
    const nodesBefore = [...nodes];
    const edgesBefore = [...edges];
    applyHardCaps(nodes, edges);
    expect(nodes).toEqual(nodesBefore);
    expect(edges).toEqual(edgesBefore);
  });
});
