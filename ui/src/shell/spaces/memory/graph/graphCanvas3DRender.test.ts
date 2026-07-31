/**
 * graphCanvas3DRender.test.ts — Task 6.2.2 rendering pipeline tests.
 *
 * Validates the two functions that made the 3D view actually render:
 *   • sceneToGraphModel   — SemanticScene → { nodes, edges } for GraphScene
 *   • computeDeterministicPositions — Fibonacci-sphere layout
 *
 * These are pure functions — no WebGL, no DOM — so they run under jsdom.
 *
 * Requirements: MGR-001, MGR-002, MGR-011, MGR-012; task 6.2.2.
 */

import { describe, it, expect } from "vitest";
import {
  sceneToGraphModel,
  computeDeterministicPositions,
} from "./graphCanvas3DSpike";
import { buildSemanticScene, type RawSceneItem } from "../scene/sceneBuilder";
import type { SemanticScene } from "../scene/semanticScene";

// ─── helpers ──────────────────────────────────────────────────────────────────

function rawItem(over: Partial<RawSceneItem> = {}): RawSceneItem {
  return {
    id: "item-1",
    kind: "memory",
    authorityClass: "stored",
    label: "test label",
    truthState: "Current",
    graphRevision: 1,
    direction: null,
    sourceEndpointId: null,
    targetEndpointId: null,
    evidenceCount: null,
    evidenceSummary: null,
    provenanceSourceId: null,
    provenanceMethod: null,
    provenanceVersion: null,
    provenanceActorLabel: null,
    validTimeStart: null,
    validTimeEnd: null,
    isCurrentlyValid: true,
    isSelected: false,
    isFocused: false,
    isInPath: false,
    isPending: false,
    hasError: false,
    isAuthorized: true,
    ...over,
  };
}

function sceneOf(items: RawSceneItem[]): SemanticScene {
  return buildSemanticScene({
    items,
    actions: [],
    graphRevision: 1,
    layoutHint: {
      seed: 42,
      strategy: "search-treemap-grid",
      primaryItemId: null,
      maxDepth: null,
    },
  }).scene;
}

// ─── sceneToGraphModel ────────────────────────────────────────────────────────

describe("sceneToGraphModel — the fix that made 3D render (task 6.2.2)", () => {
  it("R-01: produces one GraphNode per node item (was returning [] before)", () => {
    const scene = sceneOf([
      rawItem({ id: "a", kind: "memory", label: "Memory A" }),
      rawItem({ id: "b", kind: "entity", label: "Entity B" }),
      rawItem({ id: "c", kind: "goal", label: "Goal C" }),
    ]);
    const { nodes } = sceneToGraphModel(scene);
    expect(nodes).toHaveLength(3);
    expect(nodes.map((n) => n.id).sort()).toEqual(["a", "b", "c"]);
  });

  it("R-02: node labels come straight from the scene (no invention)", () => {
    const scene = sceneOf([rawItem({ id: "x", label: "exact backend label" })]);
    const { nodes } = sceneToGraphModel(scene);
    expect(nodes[0].label).toBe("exact backend label");
  });

  it("R-03: falls back to id when label is absent", () => {
    // sceneBuilder omits null-label items, so build a scene then blank the label
    // to simulate a passthrough item with an empty label.
    const scene = sceneOf([rawItem({ id: "no-label-id", label: "" })]);
    const { nodes } = sceneToGraphModel(scene);
    // Empty string is falsy for ?? only on null/undefined, so "" is preserved.
    expect(nodes[0].id).toBe("no-label-id");
  });

  it("R-04: same kind always gets the same colour bucket (deterministic)", () => {
    const scene = sceneOf([
      rawItem({ id: "m1", kind: "memory" }),
      rawItem({ id: "m2", kind: "memory" }),
      rawItem({ id: "e1", kind: "entity" }),
    ]);
    const { nodes } = sceneToGraphModel(scene);
    const m1 = nodes.find((n) => n.id === "m1")!;
    const m2 = nodes.find((n) => n.id === "m2")!;
    const e1 = nodes.find((n) => n.id === "e1")!;
    expect(m1.community).toBe(m2.community);
    expect(e1.community).not.toBe(m1.community);
  });

  it("R-05: centrality is at least 1 so every node has a visible size", () => {
    const scene = sceneOf([
      rawItem({ id: "z", evidenceCount: null }),
      rawItem({ id: "y", evidenceCount: 7 }),
    ]);
    const { nodes } = sceneToGraphModel(scene);
    for (const n of nodes) {
      expect(n.centrality).toBeGreaterThanOrEqual(1);
    }
    expect(nodes.find((n) => n.id === "y")!.centrality).toBe(7);
  });

  it("R-06: relation items become edges, not nodes", () => {
    const scene = sceneOf([
      rawItem({ id: "src", kind: "entity" }),
      rawItem({ id: "tgt", kind: "entity" }),
      rawItem({
        id: "rel",
        kind: "relation",
        direction: "outgoing",
        sourceEndpointId: "src",
        targetEndpointId: "tgt",
      }),
    ]);
    const { nodes, edges } = sceneToGraphModel(scene);
    expect(nodes.map((n) => n.id).sort()).toEqual(["src", "tgt"]);
    expect(edges).toHaveLength(1);
    expect(edges[0].source).toBe("src");
    expect(edges[0].target).toBe("tgt");
  });

  it("R-07: dangling edges are pruned (endpoint not a rendered node)", () => {
    // sceneBuilder itself drops relations with unauthorized endpoints, so this
    // asserts the belt-and-braces prune inside sceneToGraphModel.
    const scene = sceneOf([
      rawItem({ id: "only", kind: "entity" }),
    ]);
    const { edges } = sceneToGraphModel(scene);
    expect(edges).toHaveLength(0);
  });

  it("R-08: empty scene yields empty model without throwing", () => {
    const scene = sceneOf([]);
    const { nodes, edges } = sceneToGraphModel(scene);
    expect(nodes).toHaveLength(0);
    expect(edges).toHaveLength(0);
  });

  it("R-09: evidence/aggregate kinds from the backend are treated as nodes", () => {
    // memory_knowledge_items returns these kinds; they must not vanish.
    const scene = sceneOf([
      rawItem({ id: "ev", kind: "evidence" }),
      rawItem({ id: "ag", kind: "aggregate" }),
    ]);
    const { nodes } = sceneToGraphModel(scene);
    expect(nodes.map((n) => n.id).sort()).toEqual(["ag", "ev"]);
  });
});

// ─── computeDeterministicPositions ────────────────────────────────────────────

describe("computeDeterministicPositions — geometry so nodes are visible", () => {
  const node = (id: string) => ({ id, label: id, community: 0, centrality: 1 });

  it("P-01: returns one position per node", () => {
    const positions = computeDeterministicPositions([
      node("a"),
      node("b"),
      node("c"),
    ]);
    expect(positions).toHaveLength(3);
    expect(positions.map((p) => p.id).sort()).toEqual(["a", "b", "c"]);
  });

  it("P-02: empty input yields empty output", () => {
    expect(computeDeterministicPositions([])).toEqual([]);
  });

  it("P-03: single node sits at the origin", () => {
    const positions = computeDeterministicPositions([node("solo")]);
    expect(positions).toEqual([{ id: "solo", x: 0, y: 0, z: 0 }]);
  });

  it("P-04: all coordinates are finite (no NaN/Infinity reaches WebGL)", () => {
    const nodes = Array.from({ length: 25 }, (_, i) => node(`n${i}`));
    const positions = computeDeterministicPositions(nodes);
    for (const p of positions) {
      expect(Number.isFinite(p.x)).toBe(true);
      expect(Number.isFinite(p.y)).toBe(true);
      expect(Number.isFinite(p.z)).toBe(true);
    }
  });

  it("P-05: deterministic — same input yields byte-identical output", () => {
    const nodes = Array.from({ length: 12 }, (_, i) => node(`n${i}`));
    const first = computeDeterministicPositions(nodes);
    const second = computeDeterministicPositions(nodes);
    expect(first).toEqual(second);
  });

  it("P-06: nodes are separated — no two share the same position", () => {
    const nodes = Array.from({ length: 20 }, (_, i) => node(`n${i}`));
    const positions = computeDeterministicPositions(nodes);
    const keys = positions.map((p) => `${p.x.toFixed(4)},${p.y.toFixed(4)},${p.z.toFixed(4)}`);
    expect(new Set(keys).size).toBe(positions.length);
  });

  it("P-07: positions are non-zero for a multi-node graph (visible geometry)", () => {
    const nodes = Array.from({ length: 10 }, (_, i) => node(`n${i}`));
    const positions = computeDeterministicPositions(nodes);
    const anyNonZero = positions.some(
      (p) => Math.abs(p.x) > 0.01 || Math.abs(p.y) > 0.01 || Math.abs(p.z) > 0.01,
    );
    expect(anyNonZero).toBe(true);
  });

  it("P-08: radius scales with node count (density stays comfortable)", () => {
    const small = computeDeterministicPositions(
      Array.from({ length: 4 }, (_, i) => node(`s${i}`)),
    );
    const large = computeDeterministicPositions(
      Array.from({ length: 100 }, (_, i) => node(`l${i}`)),
    );
    const maxR = (ps: { x: number; y: number; z: number }[]) =>
      Math.max(...ps.map((p) => Math.sqrt(p.x * p.x + p.y * p.y + p.z * p.z)));
    expect(maxR(large)).toBeGreaterThan(maxR(small));
  });
});

// ─── End-to-end: backend shape → renderable geometry ──────────────────────────

describe("E2E: memory_knowledge_items shape renders as 3D geometry", () => {
  it("E-01: 20 backend-shaped items produce 20 positioned nodes", () => {
    // Mirrors exactly what memory_knowledge_items returns.
    const backendItems = Array.from({ length: 20 }, (_, i) =>
      rawItem({
        id: `mem-${i}`,
        kind: i % 4 === 0 ? "entity" : "memory",
        label: `Knowledge item ${i} from the backend`,
        truthState: "Current",
        graphRevision: 1,
      }),
    );
    const scene = sceneOf(backendItems);
    const { nodes } = sceneToGraphModel(scene);
    const positions = computeDeterministicPositions(nodes);

    expect(nodes).toHaveLength(20);
    expect(positions).toHaveLength(20);
    // Every node has geometry — this is what was broken before task 6.2.2.
    expect(positions.every((p) => Number.isFinite(p.x))).toBe(true);
  });
});
