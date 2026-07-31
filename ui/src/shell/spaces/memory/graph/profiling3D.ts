/**
 * profiling3D.ts — F6 isolated technical spike: performance profiling instrumentation stubs.
 *
 * Pure TypeScript module — no DOM, no WebGL, no side effects.
 *
 * This module provides the data types and accumulation logic for the F6 spike
 * performance profiling run on reference hardware RHW-001 (task 6.2.6).
 * It is an instrumentation layer only — it records samples and computes statistics.
 * It does NOT call any browser APIs, DOM methods, or WebGL functions.
 *
 * Exports:
 *   • FrameTimingSample  — one rAF-callback frame duration record.
 *   • HeapSample         — one JS heap snapshot.
 *   • GcPauseSample      — one GC pause event.
 *   • ProfilingReport    — aggregated statistics snapshot.
 *   • ProfilingSession   — accumulates samples and computes statistics.
 *
 * Thresholds (preregistered in profiling-protocol.json, frozen):
 *   FPS threshold: p95 frame time ≤ 33.3ms (≥30 FPS at p95).
 *   GC threshold:  no individual GC pause ≥ 50ms.
 *   Minimum sample count for p95/p99 computation: 30 frames.
 *
 * Design invariants (frozen per task 6.2.6):
 *   • computeP95FrameTime/computeP99FrameTime return null when < 30 samples.
 *   • checkFpsThreshold returns 'insufficient-data' when < 30 frame samples.
 *   • checkGcThreshold returns 'insufficient-data' when no GC samples recorded.
 *   • toReport() always returns a complete ProfilingReport; all fields present.
 *   • All methods are pure with respect to external state — only internal
 *     sample arrays are mutated (append-only).
 *   • All functions and methods are pure with respect to output: same samples
 *     always produce the same statistics.
 *
 * Percentile computation:
 *   Uses the "nearest rank" method (ceil of (p/100)*N), 1-indexed.
 *   Sort is ascending by durationMs before index selection.
 *
 * IDs: MGR-001, MGR-004, MGR-012, MGR-015, MGR-026; MGD-003, MGD-026, MGD-046;
 *      task 6.2.6 (F6 pre-production spike only — not a shipped renderer path).
 */

// ─── Thresholds (preregistered, frozen) ──────────────────────────────────────

/** Minimum frame samples required before p95/p99 can be computed. */
export const MIN_FRAME_SAMPLES = 30;

/**
 * FPS pass threshold: p95 frame time must be ≤ this value (ms).
 * Equivalent to ≥30 FPS at the 95th percentile.
 * Frozen per profiling-protocol.json and design.md §4.7.8.
 */
export const FPS_P95_THRESHOLD_MS = 33.3;

/**
 * GC pause fail threshold: any single pause ≥ this value (ms) is a failure.
 * Frozen per profiling-protocol.json.
 */
export const GC_PAUSE_THRESHOLD_MS = 50.0;

// ─── Sample types ─────────────────────────────────────────────────────────────

/**
 * One frame timing sample from a single rAF callback.
 *
 *   frameIndex  — monotonically increasing frame counter (0-based).
 *   durationMs  — time from rAF callback start to draw-call completion (ms).
 *   timestamp   — performance.now() value at rAF callback start (ms).
 */
export interface FrameTimingSample {
  frameIndex: number;
  durationMs: number;
  timestamp: number;
}

/**
 * One JavaScript heap snapshot.
 *
 *   heapUsedBytes  — bytes currently used (performance.memory.usedJSHeapSize).
 *   heapTotalBytes — bytes allocated (performance.memory.totalJSHeapSize).
 *   timestamp      — performance.now() at sampling time (ms).
 */
export interface HeapSample {
  heapUsedBytes: number;
  heapTotalBytes: number;
  timestamp: number;
}

/**
 * One GC pause event.
 *
 *   pauseMs   — duration of the GC pause in milliseconds.
 *   timestamp — performance.now() at pause start (ms).
 *   gcType    — 'minor' (scavenge), 'major' (full GC), or 'unknown'.
 */
export interface GcPauseSample {
  pauseMs: number;
  timestamp: number;
  gcType: 'minor' | 'major' | 'unknown';
}

// ─── Report type ──────────────────────────────────────────────────────────────

/**
 * Aggregated profiling statistics snapshot.
 *
 * All percentile fields are null when insufficient samples were available
 * at the time toReport() was called (< MIN_FRAME_SAMPLES frame samples).
 * peakHeapBytes is null when no HeapSamples have been recorded.
 * maxGcPauseMs is null when no GcPauseSamples have been recorded.
 * fpsThreshold and gcThreshold reflect the pass/fail verdict at report time.
 */
export interface ProfilingReport {
  /** Number of FrameTimingSamples collected. */
  sampleCount: number;
  /** Median (p50) frame time in ms, or null if < MIN_FRAME_SAMPLES. */
  p50FrameMs: number | null;
  /** 95th-percentile frame time in ms, or null if < MIN_FRAME_SAMPLES. */
  p95FrameMs: number | null;
  /** 99th-percentile frame time in ms, or null if < MIN_FRAME_SAMPLES. */
  p99FrameMs: number | null;
  /** Peak JS heap used in bytes across all HeapSamples, or null if none. */
  peakHeapBytes: number | null;
  /** Maximum GC pause in ms across all GcPauseSamples, or null if none. */
  maxGcPauseMs: number | null;
  /** FPS pass/fail verdict based on p95 frame time vs FPS_P95_THRESHOLD_MS. */
  fpsThreshold: 'pass' | 'fail' | 'insufficient-data';
  /** GC pass/fail verdict based on maxGcPauseMs vs GC_PAUSE_THRESHOLD_MS. */
  gcThreshold: 'pass' | 'fail' | 'insufficient-data';
}

// ─── Percentile utility ────────────────────────────────────────────────────────

/**
 * Compute the Nth percentile of a sorted (ascending) numeric array.
 *
 * Uses the "nearest rank" method: rank = ceil((p/100) * n), 1-indexed.
 * Returns null for an empty array or p outside [0, 100].
 *
 * @param sorted - ascending-sorted array of numbers
 * @param p      - percentile to compute (0–100 inclusive)
*/
function percentileOfSorted(sorted: number[], p: number): number | null {
  if (sorted.length === 0 || p < 0 || p > 100) return null;
  const n = sorted.length;
  // Nearest-rank: ceil((p/100)*n), clamped to [1, n]
  const rank = Math.max(1, Math.ceil((p / 100) * n));
  const idx = Math.min(rank, n) - 1; // convert to 0-based
  return sorted[idx]!;
}

// ─── ProfilingSession ─────────────────────────────────────────────────────────

/**
 * Accumulates frame timing, heap, and GC samples and computes profiling statistics.
 *
 * Usage pattern during the F6 spike run:
 *   1. Create a ProfilingSession at the start of a measurement phase.
 *   2. Call addFrameSample() at each rAF callback.
 *   3. Call addHeapSample() at 500ms intervals.
 *   4. Call addGcPauseSample() from the PerformanceObserver GC handler.
 *   5. Call toReport() at the end of the phase to snapshot statistics.
 *   6. Call checkFpsThreshold() and checkGcThreshold() to get pass/fail verdicts.
 *
 * All sample arrays are append-only. The session is not thread-safe (single UI thread).
 */
export class ProfilingSession {
  private readonly frameSamples: FrameTimingSample[] = [];
  private readonly heapSamples: HeapSample[] = [];
  private readonly gcSamples: GcPauseSample[] = [];

  // ─── Sample ingestion ──────────────────────────────────────────────────────

  /**
   * Record one frame timing sample.
   *
   * @param sample - FrameTimingSample from one rAF callback
   */
  addFrameSample(sample: FrameTimingSample): void {
    this.frameSamples.push(sample);
  }

  /**
   * Record one JS heap snapshot.
   *
   * @param sample - HeapSample from performance.memory or equivalent
   */
  addHeapSample(sample: HeapSample): void {
    this.heapSamples.push(sample);
  }

  /**
   * Record one GC pause event.
   *
   * @param sample - GcPauseSample from PerformanceObserver or equivalent
   */
  addGcPauseSample(sample: GcPauseSample): void {
    this.gcSamples.push(sample);
  }

  // ─── Frame time statistics ─────────────────────────────────────────────────

  /**
   * Compute the 95th-percentile frame time in milliseconds.
   *
   * Returns null when fewer than MIN_FRAME_SAMPLES (30) samples have been
   * recorded — insufficient data for a meaningful percentile estimate.
   *
   * Uses nearest-rank method on ascending-sorted durationMs values.
   */
  computeP95FrameTime(): number | null {
    if (this.frameSamples.length < MIN_FRAME_SAMPLES) return null;
    const sorted = this.frameSamples.map((s) => s.durationMs).sort((a, b) => a - b);
    return percentileOfSorted(sorted, 95);
  }

  /**
   * Compute the 99th-percentile frame time in milliseconds.
   *
   * Returns null when fewer than MIN_FRAME_SAMPLES (30) samples recorded.
   */
  computeP99FrameTime(): number | null {
    if (this.frameSamples.length < MIN_FRAME_SAMPLES) return null;
    const sorted = this.frameSamples.map((s) => s.durationMs).sort((a, b) => a - b);
    return percentileOfSorted(sorted, 99);
  }

  /**
   * Compute the 50th-percentile (median) frame time in milliseconds.
   *
   * Returns null when fewer than MIN_FRAME_SAMPLES (30) samples recorded.
   */
  computeP50FrameTime(): number | null {
    if (this.frameSamples.length < MIN_FRAME_SAMPLES) return null;
    const sorted = this.frameSamples.map((s) => s.durationMs).sort((a, b) => a - b);
    return percentileOfSorted(sorted, 50);
  }

  // ─── Heap statistics ───────────────────────────────────────────────────────

  /**
   * Compute the peak JS heap used in bytes across all HeapSamples.
   *
   * Returns null when no HeapSamples have been recorded.
   */
  computePeakHeap(): number | null {
    if (this.heapSamples.length === 0) return null;
    let peak = 0;
    for (const s of this.heapSamples) {
      if (s.heapUsedBytes > peak) peak = s.heapUsedBytes;
    }
    return peak;
  }

  // ─── Threshold checks ─────────────────────────────────────────────────────

  /**
   * Check the FPS threshold: p95 frame time ≤ FPS_P95_THRESHOLD_MS (33.3ms).
   *
   * Returns:
   *   'pass'              — p95 ≤ 33.3ms
   *   'fail'              — p95 > 33.3ms
   *   'insufficient-data' — fewer than MIN_FRAME_SAMPLES samples recorded
   */
  checkFpsThreshold(): 'pass' | 'fail' | 'insufficient-data' {
    const p95 = this.computeP95FrameTime();
    if (p95 === null) return 'insufficient-data';
    return p95 <= FPS_P95_THRESHOLD_MS ? 'pass' : 'fail';
  }

  /**
   * Check the GC pause threshold: no individual pause ≥ GC_PAUSE_THRESHOLD_MS (50ms).
   *
   * Returns:
   *   'pass'              — all recorded pauses are < 50ms (or no pauses recorded,
   *                         but at least one GC sample exists)
   *   'fail'              — at least one pause ≥ 50ms
   *   'insufficient-data' — no GC samples have been recorded at all
   *
   * Note: zero GC pauses observed during a run is 'insufficient-data' because the
   * absence of GC samples may mean the GC observer was not wired up, not that no
   * GC occurred. The caller must confirm that GC observation was active.
   */
  checkGcThreshold(): 'pass' | 'fail' | 'insufficient-data' {
    if (this.gcSamples.length === 0) return 'insufficient-data';
    for (const s of this.gcSamples) {
      if (s.pauseMs >= GC_PAUSE_THRESHOLD_MS) return 'fail';
    }
    return 'pass';
  }

  // ─── Report ────────────────────────────────────────────────────────────────

  /**
   * Produce a complete ProfilingReport snapshot of current session state.
   *
   * All fields are always present in the returned object.
   * Percentile fields are null when < MIN_FRAME_SAMPLES have been recorded.
   * peakHeapBytes is null when no HeapSamples recorded.
   * maxGcPauseMs is null when no GcPauseSamples recorded.
   */
  toReport(): ProfilingReport {
    const p95 = this.computeP95FrameTime();
    const p99 = this.computeP99FrameTime();
    const p50 = this.computeP50FrameTime();
    const peakHeap = this.computePeakHeap();

    const maxGcPauseMs =
      this.gcSamples.length === 0
        ? null
        : Math.max(...this.gcSamples.map((s) => s.pauseMs));

    return {
      sampleCount: this.frameSamples.length,
      p50FrameMs: p50,
      p95FrameMs: p95,
      p99FrameMs: p99,
      peakHeapBytes: peakHeap,
      maxGcPauseMs,
      fpsThreshold: this.checkFpsThreshold(),
      gcThreshold: this.checkGcThreshold(),
    };
  }

  // ─── Inspection (for tests / debugging) ───────────────────────────────────

  /** Return the current count of accumulated frame samples. */
  get frameSampleCount(): number {
    return this.frameSamples.length;
  }

  /** Return the current count of accumulated GC samples. */
  get gcSampleCount(): number {
    return this.gcSamples.length;
  }

  /** Return the current count of accumulated heap samples. */
  get heapSampleCount(): number {
    return this.heapSamples.length;
  }
}
