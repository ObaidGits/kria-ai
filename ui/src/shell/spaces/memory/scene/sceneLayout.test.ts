/**
 * Tests for memory/scene/sceneLayout.
 *
 * Pure TypeScript — no DOM, no JSX.
 */

import { describe, it, expect } from "vitest";
import {
  computeLayoutSeed,
  selectLayoutStrategy,
  buildLayoutHint,
  type QueryContext,
} from "./sceneLayout";

// ─── computeLayoutSeed ────────────────────────────────────────────────────────

describe("computeLayoutSeed", () => {
  it("same inputs produce the same output", () => {
    const a = computeLayoutSeed("abc123", 42);
    const b = computeLayoutSeed("abc123", 42);
    expect(a).toBe(b);
  });

  it("different queryHash produces a different seed", () => {
    const a = computeLayoutSeed("hash-A", 1);
    const b = computeLayoutSeed("hash-B", 1);
    expect(a).not.toBe(b);
  });

  it("different revision produces a different seed", () => {
    const a = computeLayoutSeed("same-hash", 1);
    const b = computeLayoutSeed("same-hash", 2);
    expect(a).not.toBe(b);
  });

  it("always returns a positive integer (unsigned 32-bit)", () => {
    const cases = [
      ["", 0],
      ["abc", 1],
      ["query-hash-xyz", 9999],
      ["ffffffffffffffff", 2147483647],
    ] as const;

    for (const [hash, rev] of cases) {
      const seed = computeLayoutSeed(hash, rev);
      expect(Number.isInteger(seed)).toBe(true);
      expect(seed).toBeGreaterThanOrEqual(0);
      expect(seed).toBeLessThanOrEqual(0xffffffff);
    }
  });
});

// ─── selectLayoutStrategy ─────────────────────────────────────────────────────

describe("selectLayoutStrategy", () => {
  it("'search' maps to 'search-treemap-grid'", () => {
    expect(selectLayoutStrategy("search")).toBe("search-treemap-grid");
  });

  it("'overview' maps to 'search-treemap-grid'", () => {
    expect(selectLayoutStrategy("overview")).toBe("search-treemap-grid");
  });

  it("'ego' maps to 'ego-radial-rings'", () => {
    expect(selectLayoutStrategy("ego")).toBe("ego-radial-rings");
  });

  it("'path' maps to 'path-layered-dag'", () => {
    expect(selectLayoutStrategy("path")).toBe("path-layered-dag");
  });

  it("'temporal' maps to 'temporal-lanes'", () => {
    expect(selectLayoutStrategy("temporal")).toBe("temporal-lanes");
  });

  it("'goal' maps to 'goal-source-grouped-lane'", () => {
    expect(selectLayoutStrategy("goal")).toBe("goal-source-grouped-lane");
  });

  it("'source' maps to 'goal-source-grouped-lane'", () => {
    expect(selectLayoutStrategy("source")).toBe("goal-source-grouped-lane");
  });
});

// ─── buildLayoutHint ──────────────────────────────────────────────────────────

describe("buildLayoutHint", () => {
  // Helper to build a minimal context
  function ctx(
    queryKind: QueryContext["queryKind"],
    overrides: Partial<Omit<QueryContext, "queryKind">> = {},
  ): QueryContext {
    return {
      queryKind,
      queryHash: "default-hash",
      graphRevision: 1,
      primaryItemId: null,
      maxDepth: null,
      ...overrides,
    };
  }

  it.each([
    ["search",   "search-treemap-grid"],
    ["overview", "search-treemap-grid"],
    ["ego",      "ego-radial-rings"],
    ["path",     "path-layered-dag"],
    ["temporal", "temporal-lanes"],
    ["goal",     "goal-source-grouped-lane"],
    ["source",   "goal-source-grouped-lane"],
  ] as const)("returns correct strategy for queryKind '%s'", (kind, expected) => {
    const hint = buildLayoutHint(ctx(kind));
    expect(hint.strategy).toBe(expected);
  });

  it("returns the primaryItemId for an ego query", () => {
    const hint = buildLayoutHint(ctx("ego", { primaryItemId: "node-42" }));
    expect(hint.primaryItemId).toBe("node-42");
  });

  it("returns the primaryItemId for a path query", () => {
    const hint = buildLayoutHint(ctx("path", { primaryItemId: "start-node" }));
    expect(hint.primaryItemId).toBe("start-node");
  });

  it("returns null primaryItemId for a search query", () => {
    const hint = buildLayoutHint(ctx("search", { primaryItemId: null }));
    expect(hint.primaryItemId).toBeNull();
  });

  it("returns the provided maxDepth", () => {
    const hint = buildLayoutHint(ctx("ego", { maxDepth: 3 }));
    expect(hint.maxDepth).toBe(3);
  });

  it("returns null maxDepth when not provided", () => {
    const hint = buildLayoutHint(ctx("path", { maxDepth: null }));
    expect(hint.maxDepth).toBeNull();
  });

  it("seed is deterministic — same context produces same seed", () => {
    const context = ctx("ego", { queryHash: "stable-hash", graphRevision: 7 });
    const a = buildLayoutHint(context);
    const b = buildLayoutHint(context);
    expect(a.seed).toBe(b.seed);
  });

  it("seed changes when graphRevision changes", () => {
    const a = buildLayoutHint(ctx("ego", { queryHash: "h", graphRevision: 1 }));
    const b = buildLayoutHint(ctx("ego", { queryHash: "h", graphRevision: 2 }));
    expect(a.seed).not.toBe(b.seed);
  });

  it("seed changes when queryHash changes", () => {
    const a = buildLayoutHint(ctx("ego", { queryHash: "hash-1", graphRevision: 5 }));
    const b = buildLayoutHint(ctx("ego", { queryHash: "hash-2", graphRevision: 5 }));
    expect(a.seed).not.toBe(b.seed);
  });
});
