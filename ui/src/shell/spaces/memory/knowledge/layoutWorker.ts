/**
 * memory/knowledge/layoutWorker — Deterministic query layouts and packed worker protocol.
 *
 * Pure TypeScript module — no JSX, no DOM, no side effects.
 *
 * Implements a seeded grid layout that guarantees: same seed + same items →
 * same positions. Workers use the packed WorkerMessage / WorkerResponse protocol
 * with a generationId cancellation guard so stale responses are always rejected.
 *
 * Design invariants (F4.7.2):
 *   • computeLayout is O(n) and deterministic.
 *   • No continuous force simulation — layout is computed once per request.
 *   • Positions are always within [0, width] × [0, height].
 *   • isLayoutResultCurrent guards against stale worker responses.
 *
 * IDs: MGD-003, MGD-046; MG-M09–M11, MG-O19.
 */

// ─── Types ────────────────────────────────────────────────────────────────────

/** Input parameters for a layout computation. */
export interface LayoutInput {
  /** Query/revision deterministic seed — equal seed + items → equal layout. */
  seed: number;
  /** Ordered list of item IDs to position. */
  itemIds: string[];
  /** Viewport width in pixels. */
  width: number;
  /** Viewport height in pixels. */
  height: number;
  /** Named layout strategy hint (currently used only for documentation; grid is always used). */
  strategy: string;
}

/** Output of a layout computation. */
export interface LayoutOutput {
  /** Computed positions, keyed by item ID. */
  positions: Map<string, { x: number; y: number }>;
  /** Seed that was used to produce this output. */
  seed: number;
  /** Monotonically increasing ID for cancellation guard. */
  generationId: number;
}

/** Message sent from the main thread to the layout worker. */
export interface WorkerMessage {
  type: "layout-request";
  input: LayoutInput;
  generationId: number;
}

/** Response sent from the layout worker back to the main thread. */
export interface WorkerResponse {
  type: "layout-result";
  output: LayoutOutput;
  generationId: number;
}

// ─── Layout computation ───────────────────────────────────────────────────────

/**
 * Tiny seeded LCG PRNG (Lehmer/Park-Miller, period 2^31-2).
 * Returns a float in [0, 1). Pure function of the seed.
 */
function lcgNext(state: number): { value: number; state: number } {
  // multiplier 48271, modulus 2^31-1
  const next = (Math.imul(state, 48271) >>> 0) % 2147483647;
  return { value: next / 2147483647, state: next || 1 };
}

/**
 * Compute a deterministic grid layout for the given items.
 *
 * Algorithm:
 *   1. Arrange items on an integer grid (ceil(sqrt(n)) columns).
 *   2. Compute a base cell position from grid coordinates.
 *   3. Apply a small per-item jitter derived from `seed XOR itemIndex` so
 *      different seeds produce visually distinct distributions while keeping
 *      items within bounds.
 *
 * Same seed + same itemIds in the same order → identical positions map.
 * Positions are always within [0, width] × [0, height].
 *
 * Pure function — O(n), completes well within 50 ms for normal scenes.
 */
export function computeLayout(
  input: LayoutInput,
  generationId: number,
): LayoutOutput {
  const { seed, itemIds, width, height } = input;
  const positions = new Map<string, { x: number; y: number }>();

  const n = itemIds.length;
  if (n === 0) {
    return { positions, seed, generationId };
  }

  // Grid dimensions
  const cols = Math.max(1, Math.ceil(Math.sqrt(n)));
  const rows = Math.max(1, Math.ceil(n / cols));

  // Cell size with 5% margin
  const margin = 0.05;
  const cellW = width / cols;
  const cellH = height / rows;
  const jitterMaxX = cellW * 0.35;
  const jitterMaxY = cellH * 0.35;

  for (let i = 0; i < n; i++) {
    const col = i % cols;
    const row = Math.floor(i / cols);

    // Base position: centre of the cell
    const baseX = (col + 0.5) * cellW;
    const baseY = (row + 0.5) * cellH;

    // Deterministic jitter seeded from (seed XOR index)
    const jitterSeed = ((seed ^ i) >>> 0) || 1;
    const r1 = lcgNext(jitterSeed);
    const r2 = lcgNext(r1.state);

    // Jitter in [-jitterMax, +jitterMax]
    const jx = (r1.value - 0.5) * 2 * jitterMaxX;
    const jy = (r2.value - 0.5) * 2 * jitterMaxY;

    // Clamp to [margin*w, (1-margin)*w] to stay within bounds
    const x = Math.min(
      Math.max(baseX + jx, width * margin),
      width * (1 - margin),
    );
    const y = Math.min(
      Math.max(baseY + jy, height * margin),
      height * (1 - margin),
    );

    positions.set(itemIds[i], { x, y });
  }

  return { positions, seed, generationId };
}

// ─── Cancellation guard ───────────────────────────────────────────────────────

/**
 * Returns true when the worker response belongs to the current layout generation.
 *
 * Use this to discard responses from superseded (cancelled) layout requests:
 *
 * ```ts
 * worker.onmessage = (e: MessageEvent<WorkerResponse>) => {
 *   if (!isLayoutResultCurrent(e.data, currentGenerationId)) return; // stale — discard
 *   applyLayout(e.data.output);
 * };
 * ```
 */
export function isLayoutResultCurrent(
  response: WorkerResponse,
  currentGenerationId: number,
): boolean {
  return response.generationId === currentGenerationId;
}
