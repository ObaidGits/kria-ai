/**
 * perf — typed performance marks/measures around the named UI events that the
 * §5.6 performance budget cares about (design.md §1.22).
 *
 * Uses the standard `performance.mark` / `performance.measure` API so the marks
 * are visible in browser/WebKit devtools AND buffered in-process for the
 * dev-gated perf HUD (Observatory → Diagnostics, later). Marks are cheap, so
 * this module runs in production too; only the HUD is dev-gated.
 *
 * Named events (with their §5.6 budgets):
 *   - space-switch  <150 ms
 *   - palette-open  <100 ms
 *   - first-token   <50 ms  (first streamed token render)
 *   - lens-mount    budget 300 ms (3D/2D lens mount)
 *   - list-scroll   ~16.7 ms (single frame @ 60 fps)
 *
 * A generic string name is also accepted for ad-hoc measures (e.g. app-render).
 */

/** Budgets in milliseconds for the named perf events. `null` = no hard target. */
export const PERF_BUDGETS = {
  "space-switch": 150,
  "palette-open": 100,
  "first-token": 50,
  "lens-mount": 300,
  "list-scroll": 1000 / 60,
} as const;

/** The named UI events tracked against the performance budget. */
export type PerfEventName = keyof typeof PERF_BUDGETS;

export interface PerfMeasure {
  /** Event name (named event or ad-hoc string). */
  name: string;
  /** High-resolution start time (ms since navigation start). */
  startTime: number;
  /** Measured duration in ms. */
  duration: number;
  /** Budget for this event in ms, or null when there is no hard target. */
  budgetMs: number | null;
  /** True when duration exceeded the budget (false when budget is null). */
  overBudget: boolean;
}

/** Opaque handle returned by {@link startMeasure}; pass it to {@link endMeasure}. */
export type PerfHandle = string;

const METRIC_PREFIX = "kria-ui";
const MAX_BUFFERED_METRICS = 200;

type Listener = (measure: PerfMeasure) => void;

const listeners = new Set<Listener>();
const buffer: PerfMeasure[] = [];

declare global {
  interface Window {
    /** Buffered perf measures, exposed for devtools/E2E inspection. */
    __KRIA_UI_METRICS__?: PerfMeasure[];
  }
}

function budgetFor(name: string): number | null {
  return name in PERF_BUDGETS ? PERF_BUDGETS[name as PerfEventName] : null;
}

function record(measure: PerfMeasure): void {
  buffer.push(measure);
  if (buffer.length > MAX_BUFFERED_METRICS) {
    buffer.splice(0, buffer.length - MAX_BUFFERED_METRICS);
  }
  // Mirror onto window for devtools/E2E without duplicating storage.
  if (typeof window !== "undefined") {
    window.__KRIA_UI_METRICS__ = buffer;
  }
  for (const listener of listeners) listener(measure);
}

/**
 * Begin measuring a named event. Returns a handle to pass to {@link endMeasure}.
 * The overload keeps `PerfEventName` autocompletion while still allowing ad-hoc
 * string names.
 */
export function startMeasure(name: PerfEventName | (string & {})): PerfHandle {
  const markName = `${METRIC_PREFIX}:${name}:start:${performance.now().toFixed(3)}`;
  performance.mark(markName);
  return markName;
}

/**
 * Finish measuring a named event started with {@link startMeasure}. Records the
 * measure into the buffer, flags budget overruns, and notifies HUD listeners.
 * Returns the resulting measure (or null if the start mark was missing).
 */
export function endMeasure(name: PerfEventName | (string & {}), startHandle: PerfHandle): PerfMeasure | null {
  const endMark = `${METRIC_PREFIX}:${name}:end:${performance.now().toFixed(3)}`;
  const measureName = `${METRIC_PREFIX}:${name}`;

  performance.mark(endMark);
  try {
    performance.measure(measureName, startHandle, endMark);
  } catch {
    // Start mark was cleared/missing — nothing to measure.
    performance.clearMarks(endMark);
    return null;
  }

  const entries = performance.getEntriesByName(measureName);
  const entry = entries[entries.length - 1];

  performance.clearMarks(startHandle);
  performance.clearMarks(endMark);
  performance.clearMeasures(measureName);

  if (!entry) return null;

  const budgetMs = budgetFor(name);
  const measure: PerfMeasure = {
    name,
    startTime: entry.startTime,
    duration: entry.duration,
    budgetMs,
    overBudget: budgetMs != null && entry.duration > budgetMs,
  };
  record(measure);
  return measure;
}

/**
 * Record a measure from an already-known start time (e.g. an event timestamp
 * captured elsewhere, such as request-sent → first-token-rendered).
 */
export function measureSince(name: PerfEventName | (string & {}), startTime: number): PerfMeasure {
  const duration = Math.max(0, performance.now() - startTime);
  const budgetMs = budgetFor(name);
  const measure: PerfMeasure = {
    name,
    startTime,
    duration,
    budgetMs,
    overBudget: budgetMs != null && duration > budgetMs,
  };
  record(measure);
  return measure;
}

/** Measure a synchronous function's duration under a named event. */
export function track<T>(name: PerfEventName | (string & {}), fn: () => T): T {
  const handle = startMeasure(name);
  try {
    return fn();
  } finally {
    endMeasure(name, handle);
  }
}

/** Snapshot of buffered measures (most recent last). */
export function getMeasures(): readonly PerfMeasure[] {
  return buffer.slice();
}

/** Clear the in-process buffer (and the window mirror). */
export function clearMeasures(): void {
  buffer.length = 0;
  if (typeof window !== "undefined") window.__KRIA_UI_METRICS__ = buffer;
}

/** Subscribe to new measures (used by the perf HUD). Returns an unsubscribe fn. */
export function subscribe(listener: Listener): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}
