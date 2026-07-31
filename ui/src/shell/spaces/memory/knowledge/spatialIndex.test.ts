/**
 * spatialIndex.test.ts — pure unit tests for spatialIndex.ts
 *
 * No DOM, no JSX, no SolidJS — pure TypeScript / Vitest.
 *
 * Covers:
 *   • buildSpatialGrid: places items in correct cells
 *   • queryVisibleItems: returns items within viewport + 64px overscan
 *   • queryVisibleItems: excludes items far outside viewport
 *   • hitTest: returns closest item within radius
 *   • hitTest: returns null when no item within radius
 *   • hitTest: does not scan all corpus items (only nearby cells)
 *
 * IDs: MGD-003; MG-M09, MG-O19.
 */
import { describe, it, expect, vi } from "vitest";

import {
  buildSpatialGrid,
  queryVisibleItems,
  hitTest,
  type Rect,
} from "./spatialIndex";

// ─── Helpers ──────────────────────────────────────────────────────────────────

function makePositions(
  entries: Array<[string, { x: number; y: number }]>,
): Map<string, { x: number; y: number }> {
  return new Map(entries);
}

// ─── buildSpatialGrid — cell placement ───────────────────────────────────────

describe("buildSpatialGrid — cell placement", () => {
  it("places an item in the correct cell", () => {
    // Default cell size is 128. Item at (64, 64) → col=0, row=0
    const positions = makePositions([["a", { x: 64, y: 64 }]]);
    const grid = buildSpatialGrid(positions, 1024, 768);
    expect(grid.cells.has("0:0")).toBe(true);
    expect(grid.cells.get("0:0")!.items).toContain("a");
  });

  it("places items in different cells when they are in different grid sectors", () => {
    // Cell size 128: item at (10,10) → 0:0, item at (200, 200) → 1:1
    const positions = makePositions([
      ["a", { x: 10, y: 10 }],
      ["b", { x: 200, y: 200 }],
    ]);
    const grid = buildSpatialGrid(positions, 1024, 768, 128);
    expect(grid.cells.get("0:0")!.items).toContain("a");
    expect(grid.cells.get("1:1")!.items).toContain("b");
    // They must be in different cells
    expect(grid.cells.get("0:0")!.items).not.toContain("b");
  });

  it("places two items in the same cell when they share a grid sector", () => {
    // Both items in cell 0:0 (x in [0,127], y in [0,127])
    const positions = makePositions([
      ["a", { x: 10, y: 10 }],
      ["b", { x: 100, y: 100 }],
    ]);
    const grid = buildSpatialGrid(positions, 1024, 768, 128);
    const cell = grid.cells.get("0:0")!;
    expect(cell.items).toContain("a");
    expect(cell.items).toContain("b");
  });

  it("empty positions → empty cells map", () => {
    const grid = buildSpatialGrid(new Map(), 1024, 768);
    expect(grid.cells.size).toBe(0);
  });

  it("returns correct cols/rows for the viewport", () => {
    // 1024 / 128 = 8 cols, 768 / 128 = 6 rows
    const grid = buildSpatialGrid(new Map(), 1024, 768, 128);
    expect(grid.cols).toBe(8);
    expect(grid.rows).toBe(6);
  });

  it("stores cell size on the grid object", () => {
    const grid = buildSpatialGrid(new Map(), 1024, 768, 64);
    expect(grid.cellSize).toBe(64);
  });
});

// ─── queryVisibleItems — within viewport + overscan ──────────────────────────

describe("queryVisibleItems — items within viewport + 64px overscan", () => {
  const viewport: Rect = { x: 0, y: 0, width: 800, height: 600 };

  it("returns an item that is inside the viewport", () => {
    const positions = makePositions([["inside", { x: 400, y: 300 }]]);
    const grid = buildSpatialGrid(positions, 1024, 768);
    const visible = queryVisibleItems(grid, viewport);
    expect(visible).toContain("inside");
  });

  it("returns an item just outside the viewport but within 64px overscan", () => {
    // 32px to the right of the viewport edge (x = 800 + 32 = 832)
    const positions = makePositions([["overscan", { x: 832, y: 300 }]]);
    const grid = buildSpatialGrid(positions, 2048, 768);
    const visible = queryVisibleItems(grid, viewport);
    expect(visible).toContain("overscan");
  });

  it("returns an item exactly at the 64px overscan boundary", () => {
    // x = 800 + 64 = 864 is still within the expanded rect
    const positions = makePositions([["edge", { x: 864, y: 300 }]]);
    const grid = buildSpatialGrid(positions, 2048, 768);
    const visible = queryVisibleItems(grid, viewport);
    expect(visible).toContain("edge");
  });

  it("returns items at all four overscan edges", () => {
    // Above (y=-50), below (y=660), left (x=-50), right (x=855)
    const positions = makePositions([
      ["top", { x: 400, y: -50 }],
      ["bottom", { x: 400, y: 660 }],
      ["left", { x: -50, y: 300 }],
      ["right", { x: 855, y: 300 }],
    ]);
    const grid = buildSpatialGrid(positions, 2048, 2048);
    const visible = queryVisibleItems(grid, viewport);
    expect(visible).toContain("top");
    expect(visible).toContain("bottom");
    expect(visible).toContain("left");
    expect(visible).toContain("right");
  });

  it("returns all items when all are within the viewport", () => {
    const n = 20;
    const entries: Array<[string, { x: number; y: number }]> = Array.from(
      { length: n },
      (_, i) => [`item-${i}`, { x: 50 + i * 30, y: 100 }],
    );
    const positions = makePositions(entries);
    const grid = buildSpatialGrid(positions, 1024, 768);
    const visible = queryVisibleItems(grid, viewport);
    for (let i = 0; i < n; i++) {
      expect(visible).toContain(`item-${i}`);
    }
  });
});

// ─── queryVisibleItems — exclusion ───────────────────────────────────────────

describe("queryVisibleItems — excludes items far outside viewport", () => {
  const viewport: Rect = { x: 0, y: 0, width: 800, height: 600 };

  it("excludes an item more than 64px beyond the right edge", () => {
    // x = 800 + 65 + 128 (extra full cell clear) = 993 — well outside overscan
    const positions = makePositions([["far-right", { x: 993, y: 300 }]]);
    const grid = buildSpatialGrid(positions, 4096, 768);
    const visible = queryVisibleItems(grid, viewport);
    expect(visible).not.toContain("far-right");
  });

  it("excludes an item far above the viewport", () => {
    const positions = makePositions([["far-up", { x: 400, y: -500 }]]);
    const grid = buildSpatialGrid(positions, 1024, 4096);
    const visible = queryVisibleItems(grid, viewport);
    expect(visible).not.toContain("far-up");
  });

  it("excludes an item far below the viewport", () => {
    const positions = makePositions([["far-down", { x: 400, y: 1500 }]]);
    const grid = buildSpatialGrid(positions, 1024, 4096);
    const visible = queryVisibleItems(grid, viewport);
    expect(visible).not.toContain("far-down");
  });

  it("excludes far items while retaining near items", () => {
    const positions = makePositions([
      ["near", { x: 400, y: 300 }],
      ["far", { x: 5000, y: 5000 }],
    ]);
    const grid = buildSpatialGrid(positions, 10000, 10000);
    const visible = queryVisibleItems(grid, viewport);
    expect(visible).toContain("near");
    expect(visible).not.toContain("far");
  });
});

// ─── hitTest — closest item within radius ────────────────────────────────────

describe("hitTest — returns closest item within radius", () => {
  it("finds the single item within radius", () => {
    const positions = makePositions([["target", { x: 100, y: 100 }]]);
    const grid = buildSpatialGrid(positions, 1024, 768);
    const hit = hitTest(grid, positions, 105, 105, 20);
    expect(hit).toBe("target");
  });

  it("returns the closest item when multiple are within radius", () => {
    const positions = makePositions([
      ["far", { x: 100, y: 100 }],
      ["near", { x: 110, y: 110 }],
    ]);
    const grid = buildSpatialGrid(positions, 1024, 768);
    // Query point at (112, 112) — nearer to "near"
    const hit = hitTest(grid, positions, 112, 112, 50);
    expect(hit).toBe("near");
  });

  it("radius is respected — item just outside radius is not returned", () => {
    const positions = makePositions([["far", { x: 200, y: 200 }]]);
    const grid = buildSpatialGrid(positions, 1024, 768);
    // Distance ≈ 141 px, radius = 10
    const hit = hitTest(grid, positions, 100, 100, 10);
    expect(hit).toBeNull();
  });

  it("item exactly at radius boundary is included (dist² <= r²)", () => {
    // Place item at (100, 100), query at (100, 120), radius = 20 → dist = 20
    const positions = makePositions([["on-edge", { x: 100, y: 100 }]]);
    const grid = buildSpatialGrid(positions, 1024, 768);
    const hit = hitTest(grid, positions, 100, 120, 20);
    expect(hit).toBe("on-edge");
  });
});

// ─── hitTest — null when no item in radius ───────────────────────────────────

describe("hitTest — returns null when no item within radius", () => {
  it("returns null for empty grid", () => {
    const grid = buildSpatialGrid(new Map(), 1024, 768);
    const hit = hitTest(grid, new Map(), 400, 300, 50);
    expect(hit).toBeNull();
  });

  it("returns null when the nearest item is outside radius", () => {
    const positions = makePositions([["a", { x: 500, y: 500 }]]);
    const grid = buildSpatialGrid(positions, 1024, 768);
    const hit = hitTest(grid, positions, 100, 100, 10);
    expect(hit).toBeNull();
  });

  it("returns null when query point has no items in surrounding cells", () => {
    // Items are far away in a completely different region
    const positions = makePositions([
      ["a", { x: 900, y: 700 }],
      ["b", { x: 950, y: 750 }],
    ]);
    const grid = buildSpatialGrid(positions, 1024, 768);
    const hit = hitTest(grid, positions, 100, 100, 30);
    expect(hit).toBeNull();
  });
});

// ─── hitTest — does not scan all corpus ──────────────────────────────────────

describe("hitTest — does not scan all corpus items", () => {
  it("finds the right item among a large corpus without scanning all", () => {
    // Build a large corpus scattered across the grid
    const n = 500;
    const entries: Array<[string, { x: number; y: number }]> = Array.from(
      { length: n },
      (_, i) => [`corpus-${i}`, { x: (i % 32) * 128 + 5, y: Math.floor(i / 32) * 128 + 5 }],
    );
    // Place a known target near the query point
    entries.push(["hit-target", { x: 305, y: 305 }]);

    const positions = makePositions(entries);
    const grid = buildSpatialGrid(positions, 4096, 4096, 128);

    // Query near the target — should return it
    const hit = hitTest(grid, positions, 310, 310, 30);
    expect(hit).toBe("hit-target");
  });

  it("cells map is consulted sparsely — only 3×3 neighbourhood cells accessed", () => {
    // Spy on Map.prototype.get to count how many distinct cell keys were accessed
    const positions = makePositions([["a", { x: 64, y: 64 }]]);
    const grid = buildSpatialGrid(positions, 2048, 2048, 128);

    const accessedKeys = new Set<string>();
    const originalGet = grid.cells.get.bind(grid.cells);
    vi.spyOn(grid.cells, "get").mockImplementation((key: string) => {
      accessedKeys.add(key);
      return originalGet(key);
    });

    hitTest(grid, positions, 64, 64, 10);

    // At most 9 cell lookups (3×3 neighbourhood)
    expect(accessedKeys.size).toBeLessThanOrEqual(9);
  });
});
