/**
 * graphModel helper tests. Core mapping/cap assertions support shipped 2D;
 * GL-specific LOD/culling/degrade assertions cover dormant utilities only and
 * do not prove a 3D renderer is integrated.
 * sizing, community coloring (accent excluded), LOD + label selection, frustum
 * culling, and the auto-degrade decision.
 */
import { describe, it, expect } from "vitest";
import {
  applyNodeCap,
  buildCommunityIndex,
  communityColorToken,
  computeLOD,
  cullNodes,
  DEFAULT_LOD,
  evaluateDegrade,
  isInViewBounds,
  mapCentralityToNodes,
  mapPredictions,
  mapRelationshipsToEdges,
  maxCentrality,
  nodeSizeForCentrality,
  pruneEdges,
  selectLabelSet,
  SELECTION_COLOR_TOKEN,
  COMMUNITY_COLOR_TOKENS,
  NEUTRAL_NODE_COLOR_TOKEN,
  type GraphNode,
  type Vec2,
  type ViewBounds,
} from "./graphModel";

const node = (id: string, centrality = 1, community = -1): GraphNode => ({
  id,
  label: id.toUpperCase(),
  centrality,
  community,
});

describe("data → graph mapping", () => {
  it("builds a community index (first community wins on duplicates)", () => {
    const idx = buildCommunityIndex([
      ["a", "b"],
      ["b", "c"],
    ]);
    expect(idx.get("a")).toBe(0);
    expect(idx.get("b")).toBe(0); // first wins
    expect(idx.get("c")).toBe(1);
  });

  it("maps centrality nodes with community + degree, unknown community = -1", () => {
    const idx = buildCommunityIndex([["a"]]);
    const nodes = mapCentralityToNodes(
      [
        { entity: "a", display_name: "Alpha", degree: 5 },
        { entity: "z", display_name: "Zeta", degree: 0 },
      ],
      idx,
    );
    expect(nodes[0]).toEqual({ id: "a", label: "Alpha", community: 0, centrality: 5 });
    expect(nodes[1]).toEqual({ id: "z", label: "Zeta", community: -1, centrality: 0 });
  });

  it("maps relationships to real (non-predicted) edges and predictions", () => {
    const edges = mapRelationshipsToEdges([{ source_id: "a", target_id: "b", rel_type: "knows" }]);
    expect(edges[0]).toEqual({ source: "a", target: "b", relType: "knows", predicted: false });
    const preds = mapPredictions([{ target: "c", display_name: "Gamma", score: 0.9 }]);
    expect(preds[0]).toEqual({ target: "c", label: "Gamma", score: 0.9 });
  });
});

describe("node cap — showing N of M (§5.4)", () => {
  it("keeps the top-N by centrality and reports 'Showing N of M'", () => {
    const nodes = [node("a", 1), node("b", 9), node("c", 5), node("d", 3)];
    const capped = applyNodeCap(nodes, 2);
    expect(capped.shown.map((n) => n.id)).toEqual(["b", "c"]); // highest centrality
    expect(capped.shownCount).toBe(2);
    expect(capped.total).toBe(4);
    expect(capped.capped).toBe(true);
    expect(capped.label).toBe("Showing 2 of 4");
  });

  it("reports 'Showing all N' when nothing is elided", () => {
    const capped = applyNodeCap([node("a", 1), node("b", 2)], 10);
    expect(capped.capped).toBe(false);
    expect(capped.label).toBe("Showing all 2");
  });

  it("breaks centrality ties deterministically by id", () => {
    const capped = applyNodeCap([node("b", 5), node("a", 5)], 1);
    expect(capped.shown[0].id).toBe("a");
  });

  it("treats cap <= 0 as no cap", () => {
    const capped = applyNodeCap([node("a"), node("b")], 0);
    expect(capped.shownCount).toBe(2);
    expect(capped.capped).toBe(false);
  });

  it("prunes edges whose endpoints were elided by the cap", () => {
    const kept = new Set(["a", "b"]);
    const edges = pruneEdges(
      [
        { source: "a", target: "b" },
        { source: "a", target: "z" },
      ],
      kept,
    );
    expect(edges).toHaveLength(1);
    expect(edges[0].target).toBe("b");
  });
});

describe("centrality → node size", () => {
  it("scales between min and max with a sqrt curve; max centrality → max size", () => {
    const nodes = [node("a", 0), node("b", 100)];
    const max = maxCentrality(nodes);
    expect(max).toBe(100);
    const small = nodeSizeForCentrality(0, max);
    const big = nodeSizeForCentrality(100, max);
    expect(small).toBeCloseTo(0.6); // DEFAULT_NODE_SIZE.min
    expect(big).toBeCloseTo(2.4); // DEFAULT_NODE_SIZE.max
    expect(big).toBeGreaterThan(small);
  });

  it("returns the min size when max centrality is 0", () => {
    expect(nodeSizeForCentrality(0, 0)).toBeCloseTo(0.6);
  });
});

describe("community → color token (accent reserved for selection)", () => {
  it("maps -1 to the neutral token and wraps others by modulo", () => {
    expect(communityColorToken(-1)).toBe(NEUTRAL_NODE_COLOR_TOKEN);
    expect(communityColorToken(0)).toBe(COMMUNITY_COLOR_TOKENS[0]);
    expect(communityColorToken(COMMUNITY_COLOR_TOKENS.length)).toBe(COMMUNITY_COLOR_TOKENS[0]);
  });

  it("never uses the accent (selection) token for a community", () => {
    for (let c = 0; c < 50; c++) {
      expect(communityColorToken(c)).not.toBe(SELECTION_COLOR_TOKEN);
    }
    expect(COMMUNITY_COLOR_TOKENS).not.toContain(SELECTION_COLOR_TOKEN);
  });
});

describe("LOD + label selection (§5.4 labels only for focused/near set)", () => {
  it("classifies LOD tiers by camera distance", () => {
    expect(computeLOD(DEFAULT_LOD.near - 1)).toBe("near");
    expect(computeLOD(DEFAULT_LOD.mid - 1)).toBe("mid");
    expect(computeLOD(DEFAULT_LOD.mid + 1)).toBe("far");
  });

  it("always labels the focused node plus the nearest nodes, capped", () => {
    const labels = selectLabelSet(
      [
        { id: "focus", distance: 500 }, // far, but focused → still labelled
        { id: "n1", distance: 2 },
        { id: "n2", distance: 5 },
        { id: "far", distance: 999 }, // beyond near band → excluded
      ],
      "focus",
      DEFAULT_LOD,
      2,
    );
    expect(labels.has("focus")).toBe(true);
    expect(labels.has("n1")).toBe(true); // nearest fills remaining slot
    expect(labels.has("far")).toBe(false);
    expect(labels.size).toBe(2); // focus + 1 near (maxLabels)
  });

  it("labels only near nodes when there is no focus", () => {
    const labels = selectLabelSet(
      [
        { id: "n1", distance: 2 },
        { id: "far", distance: 999 },
      ],
      null,
    );
    expect(labels.has("n1")).toBe(true);
    expect(labels.has("far")).toBe(false);
  });
});

describe("frustum / bounds culling", () => {
  const bounds: ViewBounds = { minX: 0, minY: 0, maxX: 10, maxY: 10 };
  it("includes in-bounds and excludes out-of-bounds positions", () => {
    expect(isInViewBounds({ x: 5, y: 5 }, bounds)).toBe(true);
    expect(isInViewBounds({ x: -1, y: 5 }, bounds)).toBe(false);
    expect(isInViewBounds({ x: -1, y: 5 }, bounds, 2)).toBe(true); // padding
  });

  it("culls to the visible set and drops nodes with no projection", () => {
    const projected = new Map<string, Vec2>([
      ["a", { x: 5, y: 5 }],
      ["b", { x: 99, y: 99 }],
    ]);
    const visible = cullNodes(["a", "b", "c"], projected, bounds);
    expect([...visible]).toEqual(["a"]);
  });
});

describe("auto-degrade decision (§5.4)", () => {
  it("degrades when WebGL is absent", () => {
    const d = evaluateDegrade({ hasWebGL: false, reducedMotion: false, recentFps: [60] });
    expect(d.degrade).toBe(true);
    expect(d.reason).toMatch(/WebGL/);
  });

  it("degrades under reduced-motion", () => {
    const d = evaluateDegrade({ hasWebGL: true, reducedMotion: true, recentFps: [60] });
    expect(d.degrade).toBe(true);
    expect(d.reason).toMatch(/reduced-motion/);
  });

  it("degrades under heavy model load", () => {
    const d = evaluateDegrade({
      hasWebGL: true,
      reducedMotion: false,
      recentFps: [60],
      heavyModelLoad: true,
    });
    expect(d.degrade).toBe(true);
    expect(d.reason).toMatch(/load/);
  });

  it("degrades on sustained low FPS across the window", () => {
    const d = evaluateDegrade({ hasWebGL: true, reducedMotion: false, recentFps: [10, 12, 15] });
    expect(d.degrade).toBe(true);
    expect(d.reason).toMatch(/low FPS/);
  });

  it("retains 3D when within budget", () => {
    const d = evaluateDegrade({ hasWebGL: true, reducedMotion: false, recentFps: [60, 58, 61] });
    expect(d.degrade).toBe(false);
  });

  it("does not degrade on a single low sample (not sustained)", () => {
    const d = evaluateDegrade({ hasWebGL: true, reducedMotion: false, recentFps: [60, 60, 10] });
    expect(d.degrade).toBe(false);
  });
});
