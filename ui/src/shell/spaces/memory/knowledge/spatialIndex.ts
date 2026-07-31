/**
 * memory/knowledge/spatialIndex — Uniform spatial grid for culling and hit testing.
 *
 * Pure TypeScript module — no JSX, no DOM, no side effects.
 *
 * Builds a uniform grid from a positions map and provides:
 *   • queryVisibleItems — O(cells in viewport) culling with 64 px overscan.
 *   • hitTest — O(1) point hit test that only checks nearby grid cells.
 *
 * Both operations are O(cells in region), never O(corpus).
 *
 * Design invariants (F4.7.4 / F4.7.5):
 *   • Cell key format: "col:row".
 *   • Default cell size: 128 px (configurable).
 *   • Overscan constant: 64 px on each side.
 *   • hitTest inspects only the 3×3 neighbourhood of the target cell.
 *
 * IDs: MGD-003; MG-M09, MG-O19.
 */

// ─── Types ────────────────────────────────────────────────────────────────────

/** One cell of the spatial grid. */
export interface SpatialCell {
  /** Item IDs whose positions fall within this cell. */
  items: string[];
}

/** Uniform spatial grid. */
export interface SpatialGrid {
  cellSize: number;
  cols: number;
  rows: number;
  /** Map of non-empty cells, keyed "col:row". */
  cells: Map<string, SpatialCell>;
}

/** Axis-aligned rectangle. */
export interface Rect {
  x: number;
  y: number;
  width: number;
  height: number;
}

// ─── Constants ────────────────────────────────────────────────────────────────

const DEFAULT_CELL_SIZE = 128;
const OVERSCAN_PX = 64;

// ─── Helpers ──────────────────────────────────────────────────────────────────

function cellKey(col: number, row: number): string {
  return `${col}:${row}`;
}

function posToCell(
  x: number,
  y: number,
  cellSize: number,
): { col: number; row: number } {
  return {
    col: Math.floor(x / cellSize),
    row: Math.floor(y / cellSize),
  };
}

// ─── buildSpatialGrid ────────────────────────────────────────────────────────

/**
 * Build a uniform spatial grid from a positions map.
 *
 * Items outside [0, viewportWidth] × [0, viewportHeight] are still indexed
 * (their cell coordinates may be negative or beyond the stated cols/rows),
 * but cols/rows represent the tight bounding box of the viewport only.
 *
 * Pure function — O(n).
 */
export function buildSpatialGrid(
  positions: Map<string, { x: number; y: number }>,
  viewportWidth: number,
  viewportHeight: number,
  cellSize: number = DEFAULT_CELL_SIZE,
): SpatialGrid {
  const cs = cellSize > 0 ? cellSize : DEFAULT_CELL_SIZE;
  const cols = Math.max(1, Math.ceil(viewportWidth / cs));
  const rows = Math.max(1, Math.ceil(viewportHeight / cs));
  const cells = new Map<string, SpatialCell>();

  for (const [id, pos] of positions) {
    const { col, row } = posToCell(pos.x, pos.y, cs);
    const key = cellKey(col, row);
    let cell = cells.get(key);
    if (cell === undefined) {
      cell = { items: [] };
      cells.set(key, cell);
    }
    cell.items.push(id);
  }

  return { cellSize: cs, cols, rows, cells };
}

// ─── queryVisibleItems ───────────────────────────────────────────────────────

/**
 * Query item IDs visible within a viewport rectangle plus a 64 px overscan.
 *
 * Only cells that overlap the expanded query rect are examined — never the full
 * corpus. Duplicate IDs are never returned (each item appears in exactly one
 * cell).
 *
 * Pure function — O(cells in expanded viewport).
 */
export function queryVisibleItems(
  grid: SpatialGrid,
  viewport: Rect,
): string[] {
  const { cellSize } = grid;

  // Expand viewport by OVERSCAN_PX on each side
  const qx = viewport.x - OVERSCAN_PX;
  const qy = viewport.y - OVERSCAN_PX;
  const qx2 = viewport.x + viewport.width + OVERSCAN_PX;
  const qy2 = viewport.y + viewport.height + OVERSCAN_PX;

  // Cell range that overlaps the expanded query rect
  const colMin = Math.floor(qx / cellSize);
  const rowMin = Math.floor(qy / cellSize);
  const colMax = Math.floor(qx2 / cellSize);
  const rowMax = Math.floor(qy2 / cellSize);

  const result: string[] = [];

  for (let row = rowMin; row <= rowMax; row++) {
    for (let col = colMin; col <= colMax; col++) {
      const cell = grid.cells.get(cellKey(col, row));
      if (cell !== undefined) {
        for (const id of cell.items) {
          result.push(id);
        }
      }
    }
  }

  return result;
}

// ─── hitTest ─────────────────────────────────────────────────────────────────

/**
 * Find the closest item within `radius` pixels of point (px, py).
 *
 * Only the 3×3 neighbourhood of cells around (px, py) is examined — this
 * guarantees no O(corpus) scan. Returns null when no item is within `radius`.
 *
 * When multiple items are within radius, the closest one (Euclidean distance)
 * is returned. Ties are broken by insertion order.
 *
 * Pure function.
 */
export function hitTest(
  grid: SpatialGrid,
  positions: Map<string, { x: number; y: number }>,
  px: number,
  py: number,
  radius: number,
): string | null {
  const { cellSize } = grid;
  const { col: centerCol, row: centerRow } = posToCell(px, py, cellSize);

  let bestId: string | null = null;
  let bestDist = Infinity;
  const r2 = radius * radius;

  // Check 3×3 neighbourhood (one extra cell of margin for items near cell edges)
  for (let drow = -1; drow <= 1; drow++) {
    for (let dcol = -1; dcol <= 1; dcol++) {
      const cell = grid.cells.get(
        cellKey(centerCol + dcol, centerRow + drow),
      );
      if (cell === undefined) continue;

      for (const id of cell.items) {
        const pos = positions.get(id);
        if (pos === undefined) continue;
        const dx = pos.x - px;
        const dy = pos.y - py;
        const dist2 = dx * dx + dy * dy;
        if (dist2 <= r2 && dist2 < bestDist) {
          bestDist = dist2;
          bestId = id;
        }
      }
    }
  }

  return bestId;
}
