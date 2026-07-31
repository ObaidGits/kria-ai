/**
 * profiling3D.test.ts — Unit tests for the F6 profiling instrumentation stubs.
 *
 * Covers (task 6.2.6):
 *   - computeP95FrameTime returns null for < 30 samples.
 *   - p95 of known distribution computes correctly.
 *   - checkFpsThreshold passes when p95 ≤ 33.3, fails when > 33.3.
 *   - checkGcThreshold passes when no pause ≥ 50ms, fails when any pause ≥ 50ms.
 *   - toReport includes all required fields with correct values.
 *   - computeP50/P99 correctness and null guard.
 *   - computePeakHeap correctness and null guard.
 *   - toReport.gcThreshold is 'insufficient-data' when no GC samples recorded.
 *
 * No DOM, no WebGL — pure logic tests.
 *
 * Requirements: MGR-001, MGR-004, MGR-012, MGR-015, MGR-026; MGD-003, MGD-026.
 * Spec task: 6.2.6
 *
 * **Validates: Requirements MGR-015, MGR-026**
 */

import { describe, it, expect, beforeEach } from 'vitest';
import {
  ProfilingSession,
  MIN_FRAME_SAMPLES,
  FPS_P95_THRESHOLD_MS,
  GC_PAUSE_THRESHOLD_MS,
  type FrameTimingSample,
  type HeapSample,
  type GcPauseSample,
  type ProfilingReport,
} from './profiling3D';

// ─── Helpers ──────────────────────────────────────────────────────────────────

/** Build a FrameTimingSample with a given duration. */
function frameSample(durationMs: number, frameIndex = 0): FrameTimingSample {
  return { frameIndex, durationMs, timestamp: 1000 + frameIndex };
}

/** Build N frame samples all with the same duration. */
function uniformFrameSamples(count: number, durationMs: number): FrameTimingSample[] {
  const result: FrameTimingSample[] = [];
  for (let i = 0; i < count; i++) result.push(frameSample(durationMs, i));
  return result;
}

/** Build a HeapSample with a given heapUsedBytes. */
function heapSample(heapUsedBytes: number): HeapSample {
  return { heapUsedBytes, heapTotalBytes: heapUsedBytes * 2, timestamp: Date.now() };
}

/** Build a GcPauseSample with a given pauseMs. */
function gcSample(pauseMs: number, gcType: GcPauseSample['gcType'] = 'minor'): GcPauseSample {
  return { pauseMs, gcType, timestamp: Date.now() };
}

/** Add N frame samples to a session. */
function addFrameSamples(session: ProfilingSession, samples: FrameTimingSample[]): void {
  for (const s of samples) session.addFrameSample(s);
}

// ─── Constants ────────────────────────────────────────────────────────────────

describe('profiling3D — exported constants', () => {
  it('MIN_FRAME_SAMPLES is 30', () => {
    expect(MIN_FRAME_SAMPLES).toBe(30);
  });

  it('FPS_P95_THRESHOLD_MS is 33.3', () => {
    expect(FPS_P95_THRESHOLD_MS).toBeCloseTo(33.3);
  });

  it('GC_PAUSE_THRESHOLD_MS is 50', () => {
    expect(GC_PAUSE_THRESHOLD_MS).toBe(50.0);
  });
});

// ─── computeP95FrameTime — null guard ────────────────────────────────────────

describe('ProfilingSession — computeP95FrameTime null guard', () => {
  let session: ProfilingSession;

  beforeEach(() => { session = new ProfilingSession(); });

  it('returns null with 0 samples', () => {
    expect(session.computeP95FrameTime()).toBeNull();
  });

  it('returns null with 1 sample', () => {
    session.addFrameSample(frameSample(10));
    expect(session.computeP95FrameTime()).toBeNull();
  });

  it('returns null with 29 samples (one below threshold)', () => {
    addFrameSamples(session, uniformFrameSamples(29, 10));
    expect(session.computeP95FrameTime()).toBeNull();
  });

  it('returns a number with exactly 30 samples', () => {
    addFrameSamples(session, uniformFrameSamples(30, 10));
    expect(session.computeP95FrameTime()).not.toBeNull();
  });

  it('returns a number with more than 30 samples', () => {
    addFrameSamples(session, uniformFrameSamples(60, 10));
    expect(session.computeP95FrameTime()).not.toBeNull();
  });
});

// ─── computeP95FrameTime — correct value ─────────────────────────────────────

describe('ProfilingSession — computeP95FrameTime correctness', () => {
  let session: ProfilingSession;

  beforeEach(() => { session = new ProfilingSession(); });

  it('p95 of 100 uniform 20ms frames is 20ms', () => {
    addFrameSamples(session, uniformFrameSamples(100, 20));
    expect(session.computeP95FrameTime()).toBeCloseTo(20);
  });

  it('p95 of [1..100]ms distribution is 95ms (nearest-rank: ceil(0.95*100)=95)', () => {
    // Build samples 1, 2, ..., 100 ms (already sorted ascending by construction)
    for (let i = 1; i <= 100; i++) {
      session.addFrameSample(frameSample(i, i - 1));
    }
    // Nearest-rank: rank = ceil(0.95*100) = 95, 0-indexed = 94 → value = 95
    expect(session.computeP95FrameTime()).toBe(95);
  });

  it('p95 with one very large outlier is determined by that outlier at tail', () => {
    // 29 samples at 10ms, 1 sample at 500ms → 30 total; p95 = value at rank 29 of sorted
    addFrameSamples(session, uniformFrameSamples(29, 10));
    session.addFrameSample(frameSample(500, 29));
    // sorted: [10,10,...10 (29),500]; rank=ceil(0.95*30)=29; index=28 → value=10
    expect(session.computeP95FrameTime()).toBe(10);
  });

  it('p95 of 30 samples — 28 at 10ms, 2 at 100ms', () => {
    // sorted: [10*28, 100*2]; rank=ceil(0.95*30)=29; index=28 → value=100
    addFrameSamples(session, uniformFrameSamples(28, 10));
    addFrameSamples(session, uniformFrameSamples(2, 100));
    expect(session.computeP95FrameTime()).toBe(100);
  });

  it('p95 is stable regardless of insertion order', () => {
    // Add samples out of order: 100, 1, 50, 20, ...
    const durations = [100, 1, 50, 20, 30];
    for (let repeat = 0; repeat < 6; repeat++) {
      for (const d of durations) session.addFrameSample(frameSample(d, session.frameSampleCount));
    }
    // 30 samples total, sorted: [1,1,1,1,1,1, 20,20,20,20,20,20, 30*6, 50*6, 100*6]
    const p95 = session.computeP95FrameTime();
    expect(p95).not.toBeNull();
    // rank=ceil(0.95*30)=29, index=28; sorted array position 28 is 100 (last 6 entries)
    expect(p95).toBe(100);
  });
});

// ─── computeP50 and computeP99 ────────────────────────────────────────────────

describe('ProfilingSession — computeP50 and computeP99', () => {
  let session: ProfilingSession;

  beforeEach(() => { session = new ProfilingSession(); });

  it('computeP50FrameTime returns null below 30 samples', () => {
    addFrameSamples(session, uniformFrameSamples(29, 10));
    expect(session.computeP50FrameTime()).toBeNull();
  });

  it('computeP99FrameTime returns null below 30 samples', () => {
    addFrameSamples(session, uniformFrameSamples(29, 10));
    expect(session.computeP99FrameTime()).toBeNull();
  });

  it('p50 of [1..100] is 50 (nearest-rank: ceil(0.5*100)=50, index=49)', () => {
    for (let i = 1; i <= 100; i++) session.addFrameSample(frameSample(i, i - 1));
    expect(session.computeP50FrameTime()).toBe(50);
  });

  it('p99 of [1..100] is 99 (nearest-rank: ceil(0.99*100)=99, index=98)', () => {
    for (let i = 1; i <= 100; i++) session.addFrameSample(frameSample(i, i - 1));
    expect(session.computeP99FrameTime()).toBe(99);
  });

  it('p50 ≤ p95 ≤ p99 for any distribution', () => {
    // Random-ish mix of durations
    const durations = [5, 10, 15, 8, 22, 30, 40, 50, 60, 100];
    for (let r = 0; r < 10; r++) {
      for (const d of durations) session.addFrameSample(frameSample(d, session.frameSampleCount));
    }
    const p50 = session.computeP50FrameTime()!;
    const p95 = session.computeP95FrameTime()!;
    const p99 = session.computeP99FrameTime()!;
    expect(p50).not.toBeNull();
    expect(p50).toBeLessThanOrEqual(p95);
    expect(p95).toBeLessThanOrEqual(p99);
  });
});

// ─── checkFpsThreshold ────────────────────────────────────────────────────────

describe('ProfilingSession — checkFpsThreshold', () => {
  let session: ProfilingSession;

  beforeEach(() => { session = new ProfilingSession(); });

  it("returns 'insufficient-data' with 0 samples", () => {
    expect(session.checkFpsThreshold()).toBe('insufficient-data');
  });

  it("returns 'insufficient-data' with 29 samples", () => {
    addFrameSamples(session, uniformFrameSamples(29, 10));
    expect(session.checkFpsThreshold()).toBe('insufficient-data');
  });

  it("returns 'pass' when p95 is exactly 33.3ms (boundary — ≤ threshold)", () => {
    // 30 uniform samples at 33.3ms → p95 = 33.3ms
    addFrameSamples(session, uniformFrameSamples(30, 33.3));
    expect(session.checkFpsThreshold()).toBe('pass');
  });

  it("returns 'pass' when p95 is well below threshold (10ms)", () => {
    addFrameSamples(session, uniformFrameSamples(60, 10));
    expect(session.checkFpsThreshold()).toBe('pass');
  });

  it("returns 'fail' when p95 is just above threshold (33.4ms)", () => {
    // 30 samples at 33.4ms → p95 = 33.4ms > 33.3ms
    addFrameSamples(session, uniformFrameSamples(30, 33.4));
    expect(session.checkFpsThreshold()).toBe('fail');
  });

  it("returns 'fail' when p95 is far above threshold (100ms)", () => {
    addFrameSamples(session, uniformFrameSamples(60, 100));
    expect(session.checkFpsThreshold()).toBe('fail');
  });

  it("returns 'fail' when a tail of slow frames pushes p95 above threshold", () => {
    // 29 samples at 10ms, 31 at 50ms → sorted, p95 of 60 = rank 57, index 56 → 50ms > 33.3ms
    addFrameSamples(session, uniformFrameSamples(29, 10));
    addFrameSamples(session, uniformFrameSamples(31, 50));
    expect(session.checkFpsThreshold()).toBe('fail');
  });

  it("returns 'pass' when only a very small tail exceeds threshold but p95 does not", () => {
    // 58 samples at 10ms + 2 at 100ms → 60 total
    // sorted: [10*58, 100*2]; rank=ceil(0.95*60)=57; index=56 → value=10ms → pass
    addFrameSamples(session, uniformFrameSamples(58, 10));
    addFrameSamples(session, uniformFrameSamples(2, 100));
    expect(session.checkFpsThreshold()).toBe('pass');
  });
});

// ─── checkGcThreshold ────────────────────────────────────────────────────────

describe('ProfilingSession — checkGcThreshold', () => {
  let session: ProfilingSession;

  beforeEach(() => { session = new ProfilingSession(); });

  it("returns 'insufficient-data' when no GC samples recorded", () => {
    expect(session.checkGcThreshold()).toBe('insufficient-data');
  });

  it("returns 'pass' when all pauses are below 50ms", () => {
    session.addGcPauseSample(gcSample(10));
    session.addGcPauseSample(gcSample(20));
    session.addGcPauseSample(gcSample(49.9));
    expect(session.checkGcThreshold()).toBe('pass');
  });

  it("returns 'fail' when exactly one pause equals 50ms (boundary — ≥ threshold)", () => {
    session.addGcPauseSample(gcSample(10));
    session.addGcPauseSample(gcSample(50));
    expect(session.checkGcThreshold()).toBe('fail');
  });

  it("returns 'fail' when exactly one pause exceeds 50ms (51ms)", () => {
    session.addGcPauseSample(gcSample(51));
    expect(session.checkGcThreshold()).toBe('fail');
  });

  it("returns 'fail' when any of many pauses is ≥ 50ms", () => {
    for (let i = 1; i <= 10; i++) session.addGcPauseSample(gcSample(i * 4)); // 4..40ms
    session.addGcPauseSample(gcSample(80)); // the one failing pause
    expect(session.checkGcThreshold()).toBe('fail');
  });

  it("returns 'pass' for a single tiny pause (1ms)", () => {
    session.addGcPauseSample(gcSample(1));
    expect(session.checkGcThreshold()).toBe('pass');
  });

  it("returns 'pass' for gcType 'major' when pause < 50ms", () => {
    session.addGcPauseSample(gcSample(30, 'major'));
    expect(session.checkGcThreshold()).toBe('pass');
  });

  it("returns 'fail' for gcType 'major' when pause = 50ms", () => {
    session.addGcPauseSample(gcSample(50, 'major'));
    expect(session.checkGcThreshold()).toBe('fail');
  });
});

// ─── computePeakHeap ─────────────────────────────────────────────────────────

describe('ProfilingSession — computePeakHeap', () => {
  let session: ProfilingSession;

  beforeEach(() => { session = new ProfilingSession(); });

  it('returns null when no heap samples recorded', () => {
    expect(session.computePeakHeap()).toBeNull();
  });

  it('returns the single value for one heap sample', () => {
    session.addHeapSample(heapSample(1_000_000));
    expect(session.computePeakHeap()).toBe(1_000_000);
  });

  it('returns the maximum of multiple heap samples', () => {
    session.addHeapSample(heapSample(1_000_000));
    session.addHeapSample(heapSample(50_000_000));
    session.addHeapSample(heapSample(10_000_000));
    expect(session.computePeakHeap()).toBe(50_000_000);
  });

  it('returns the last value when samples are monotonically increasing', () => {
    for (let i = 1; i <= 10; i++) session.addHeapSample(heapSample(i * 10_000_000));
    expect(session.computePeakHeap()).toBe(100_000_000);
  });
});

// ─── toReport ────────────────────────────────────────────────────────────────

describe('ProfilingSession — toReport', () => {
  it('returns all required fields on an empty session', () => {
    const session = new ProfilingSession();
    const report: ProfilingReport = session.toReport();

    expect(report).toHaveProperty('sampleCount');
    expect(report).toHaveProperty('p50FrameMs');
    expect(report).toHaveProperty('p95FrameMs');
    expect(report).toHaveProperty('p99FrameMs');
    expect(report).toHaveProperty('peakHeapBytes');
    expect(report).toHaveProperty('maxGcPauseMs');
    expect(report).toHaveProperty('fpsThreshold');
    expect(report).toHaveProperty('gcThreshold');
  });

  it('sampleCount is 0 for empty session', () => {
    expect(new ProfilingSession().toReport().sampleCount).toBe(0);
  });

  it('all percentile fields are null for empty session', () => {
    const report = new ProfilingSession().toReport();
    expect(report.p50FrameMs).toBeNull();
    expect(report.p95FrameMs).toBeNull();
    expect(report.p99FrameMs).toBeNull();
  });

  it('peakHeapBytes is null when no heap samples', () => {
    expect(new ProfilingSession().toReport().peakHeapBytes).toBeNull();
  });

  it('maxGcPauseMs is null when no GC samples', () => {
    expect(new ProfilingSession().toReport().maxGcPauseMs).toBeNull();
  });

  it("fpsThreshold is 'insufficient-data' on empty session", () => {
    expect(new ProfilingSession().toReport().fpsThreshold).toBe('insufficient-data');
  });

  it("gcThreshold is 'insufficient-data' on empty session", () => {
    expect(new ProfilingSession().toReport().gcThreshold).toBe('insufficient-data');
  });

  it('sampleCount matches number of frame samples added', () => {
    const session = new ProfilingSession();
    addFrameSamples(session, uniformFrameSamples(45, 15));
    expect(session.toReport().sampleCount).toBe(45);
  });

  it('p95FrameMs matches computeP95FrameTime()', () => {
    const session = new ProfilingSession();
    addFrameSamples(session, uniformFrameSamples(60, 20));
    expect(session.toReport().p95FrameMs).toBe(session.computeP95FrameTime());
  });

  it('peakHeapBytes matches computePeakHeap()', () => {
    const session = new ProfilingSession();
    session.addHeapSample(heapSample(5_000_000));
    session.addHeapSample(heapSample(15_000_000));
    expect(session.toReport().peakHeapBytes).toBe(session.computePeakHeap());
  });

  it('maxGcPauseMs is the max of all recorded GC pauses', () => {
    const session = new ProfilingSession();
    session.addGcPauseSample(gcSample(10));
    session.addGcPauseSample(gcSample(45));
    session.addGcPauseSample(gcSample(30));
    expect(session.toReport().maxGcPauseMs).toBe(45);
  });

  it("fpsThreshold is 'pass' when 30 samples all at 10ms", () => {
    const session = new ProfilingSession();
    addFrameSamples(session, uniformFrameSamples(30, 10));
    expect(session.toReport().fpsThreshold).toBe('pass');
  });

  it("fpsThreshold is 'fail' when 30 samples all at 40ms", () => {
    const session = new ProfilingSession();
    addFrameSamples(session, uniformFrameSamples(30, 40));
    expect(session.toReport().fpsThreshold).toBe('fail');
  });

  it("gcThreshold is 'pass' when GC pauses are all < 50ms", () => {
    const session = new ProfilingSession();
    session.addGcPauseSample(gcSample(20));
    session.addGcPauseSample(gcSample(35));
    expect(session.toReport().gcThreshold).toBe('pass');
  });

  it("gcThreshold is 'fail' when any GC pause is ≥ 50ms", () => {
    const session = new ProfilingSession();
    session.addGcPauseSample(gcSample(20));
    session.addGcPauseSample(gcSample(50));
    expect(session.toReport().gcThreshold).toBe('fail');
  });

  it('report from a fully populated session has no null percentile fields', () => {
    const session = new ProfilingSession();
    addFrameSamples(session, uniformFrameSamples(60, 16));
    session.addHeapSample(heapSample(10_000_000));
    session.addGcPauseSample(gcSample(5));
    const report = session.toReport();
    expect(report.p50FrameMs).not.toBeNull();
    expect(report.p95FrameMs).not.toBeNull();
    expect(report.p99FrameMs).not.toBeNull();
    expect(report.peakHeapBytes).not.toBeNull();
    expect(report.maxGcPauseMs).not.toBeNull();
  });

  it('repeated calls to toReport() return consistent results given same samples', () => {
    const session = new ProfilingSession();
    addFrameSamples(session, uniformFrameSamples(30, 25));
    session.addHeapSample(heapSample(8_000_000));
    session.addGcPauseSample(gcSample(15));
    const r1 = session.toReport();
    const r2 = session.toReport();
    expect(r1).toEqual(r2);
  });
});
