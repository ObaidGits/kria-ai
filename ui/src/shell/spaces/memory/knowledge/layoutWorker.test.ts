/**
 * layoutWorker.test.ts — pure unit tests for layoutWorker.ts
 *
 * No DOM, no JSX, no SolidJS — pure TypeScript / Vitest.
 *
 * Covers:
 *   • Determinism: same seed + items → same positions
 *   • Seed sensitivity: different seed → different positions
 *   • Item sensitivity: different items → different positions
 *   • isLayoutResultCurrent: true when IDs match, false when stale
 *   • All input items appear in the output positions map
 *   • Empty item list → empty positions map
 *   • All positions within [0, width] × [0, height]
 *
 * IDs: MGD-003, MGD-046; MG-M09–M11, MG-O19.
 */
import { describe, it, expect } from "vitest";

import {
  computeLayout,
  isLayoutResultCurrent,
  type LayoutInput,
  type WorkerResponse,
  type LayoutOutput,
} from "./layoutWorker";

// ─── Helpers ──────────────────────────────────────────────────────────────────

function makeInput(
  overrides: Partial<LayoutInput> = {},
): LayoutInput {
  return {
    seed: 42,
    itemIds: ["a", "b", "c", "d"],
    width: 800,
    height: 600,
    strategy: "search-treemap-grid",
    ...overrides,
  };
}

function makeResponse(
  output: LayoutOutput,
  generationId: number,
): WorkerResponse {
  return { type: "layout-result", output, generationId };
}

/** Serialise a positions map to a stable string for equality assertions. */
function serialisePositions(
  positions: Map<string, { x: number; y: number }>,
): string {
  const entries = [...positions.entries()].sort(([a], [b]) =>
    a < b ? -1 : a > b ? 1 : 0,
  );
  return JSON.stringify(entries);
}

// ─── computeLayout: determinism ──────────────────────────────────────────────

describe("computeLayout — determinism", () => {
  it("same seed + same items produce identical positions", () => {
    const input = makeInput();
    const r1 = computeLayout(input, 1);
    const r2 = computeLayout(input, 1);
    expect(serialisePositions(r1.positions)).toBe(
      serialisePositions(r2.positions),
    );
  });

  it("same seed + same items (different generationId) produce identical positions", () => {
    const input = makeInput();
    const r1 = computeLayout(input, 1);
    const r2 = computeLayout(input, 99);
    // generationId does not affect positions
    expect(serialisePositions(r1.positions)).toBe(
      serialisePositions(r2.positions),
    );
  });

  it("reflects the generationId in the output", () => {
    const result = computeLayout(makeInput(), 7);
    expect(result.generationId).toBe(7);
  });

  it("reflects the seed in the output", () => {
    const result = computeLayout(makeInput({ seed: 123 }), 1);
    expect(result.seed).toBe(123);
  });
});

// ─── computeLayout: seed sensitivity ─────────────────────────────────────────

describe("computeLayout — seed sensitivity", () => {
  it("different seeds produce different positions for the same items", () => {
    const items = ["x", "y", "z", "w", "v"];
    const r1 = computeLayout(makeInput({ seed: 1, itemIds: items }), 1);
    const r2 = computeLayout(makeInput({ seed: 9999, itemIds: items }), 1);
    // With 5 items and different seeds the layouts should differ
    expect(serialisePositions(r1.positions)).not.toBe(
      serialisePositions(r2.positions),
    );
  });
});

// ─── computeLayout: item sensitivity ─────────────────────────────────────────

describe("computeLayout — item sensitivity", () => {
  it("different item lists produce different positions", () => {
    const r1 = computeLayout(
      makeInput({ seed: 42, itemIds: ["a", "b", "c"] }),
      1,
    );
    const r2 = computeLayout(
      makeInput({ seed: 42, itemIds: ["x", "y", "z"] }),
      1,
    );
    expect(serialisePositions(r1.positions)).not.toBe(
      serialisePositions(r2.positions),
    );
  });

  it("different item count produces different layout", () => {
    const r1 = computeLayout(makeInput({ itemIds: ["a"] }), 1);
    const r2 = computeLayout(makeInput({ itemIds: ["a", "b"] }), 1);
    // At minimum the maps have different sizes
    expect(r1.positions.size).not.toBe(r2.positions.size);
  });
});

// ─── computeLayout: coverage ──────────────────────────────────────────────────

describe("computeLayout — all items appear in output", () => {
  it("returns positions for all input items", () => {
    const itemIds = ["n1", "n2", "n3", "n4", "n5"];
    const result = computeLayout(makeInput({ itemIds }), 1);
    for (const id of itemIds) {
      expect(result.positions.has(id)).toBe(true);
    }
    expect(result.positions.size).toBe(itemIds.length);
  });

  it("returns exactly the supplied IDs — no extras", () => {
    const itemIds = ["alpha", "beta", "gamma"];
    const result = computeLayout(makeInput({ itemIds }), 1);
    const keys = [...result.positions.keys()].sort();
    expect(keys).toEqual([...itemIds].sort());
  });
});

// ─── computeLayout: empty items ───────────────────────────────────────────────

describe("computeLayout — empty item list", () => {
  it("returns an empty positions map", () => {
    const result = computeLayout(makeInput({ itemIds: [] }), 1);
    expect(result.positions.size).toBe(0);
  });

  it("empty result still carries seed and generationId", () => {
    const result = computeLayout(makeInput({ seed: 77, itemIds: [] }), 5);
    expect(result.seed).toBe(77);
    expect(result.generationId).toBe(5);
  });
});

// ─── computeLayout: bounds ────────────────────────────────────────────────────

describe("computeLayout — positions within bounds", () => {
  it("all positions are within [0, width] × [0, height]", () => {
    const width = 1024;
    const height = 768;
    const itemIds = Array.from({ length: 50 }, (_, i) => `item-${i}`);
    const result = computeLayout(
      makeInput({ seed: 0xdeadbeef, itemIds, width, height }),
      1,
    );
    for (const [id, pos] of result.positions) {
      expect(pos.x).toBeGreaterThanOrEqual(0);
      expect(pos.x).toBeLessThanOrEqual(width);
      expect(pos.y).toBeGreaterThanOrEqual(0);
      expect(pos.y).toBeLessThanOrEqual(height);
      void id; // suppress unused-var warning
    }
  });

  it("single item is within bounds", () => {
    const result = computeLayout(
      makeInput({ itemIds: ["solo"], width: 400, height: 300 }),
      1,
    );
    const pos = result.positions.get("solo")!;
    expect(pos.x).toBeGreaterThanOrEqual(0);
    expect(pos.x).toBeLessThanOrEqual(400);
    expect(pos.y).toBeGreaterThanOrEqual(0);
    expect(pos.y).toBeLessThanOrEqual(300);
  });

  it("100-item layout stays within bounds", () => {
    const itemIds = Array.from({ length: 100 }, (_, i) => `n${i}`);
    const result = computeLayout(
      makeInput({ seed: 1, itemIds, width: 800, height: 600 }),
      1,
    );
    for (const pos of result.positions.values()) {
      expect(pos.x).toBeGreaterThanOrEqual(0);
      expect(pos.x).toBeLessThanOrEqual(800);
      expect(pos.y).toBeGreaterThanOrEqual(0);
      expect(pos.y).toBeLessThanOrEqual(600);
    }
  });
});

// ─── isLayoutResultCurrent ────────────────────────────────────────────────────

describe("isLayoutResultCurrent", () => {
  function makeOutput(generationId: number): LayoutOutput {
    return {
      positions: new Map(),
      seed: 1,
      generationId,
    };
  }

  it("returns true when generationIds match", () => {
    const response = makeResponse(makeOutput(5), 5);
    expect(isLayoutResultCurrent(response, 5)).toBe(true);
  });

  it("returns false when response generationId is older than current", () => {
    const response = makeResponse(makeOutput(3), 3);
    expect(isLayoutResultCurrent(response, 7)).toBe(false);
  });

  it("returns false when response generationId is ahead of current", () => {
    const response = makeResponse(makeOutput(10), 10);
    expect(isLayoutResultCurrent(response, 9)).toBe(false);
  });

  it("returns true for generationId = 0 match", () => {
    const response = makeResponse(makeOutput(0), 0);
    expect(isLayoutResultCurrent(response, 0)).toBe(true);
  });

  it("returns false for generationId = 0 vs 1", () => {
    const response = makeResponse(makeOutput(0), 0);
    expect(isLayoutResultCurrent(response, 1)).toBe(false);
  });
});
