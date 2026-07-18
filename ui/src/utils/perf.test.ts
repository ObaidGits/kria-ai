import { afterEach, describe, expect, it, vi } from "vitest";
import {
  PERF_BUDGETS,
  clearMeasures,
  endMeasure,
  getMeasures,
  measureSince,
  startMeasure,
  subscribe,
  track,
  type PerfMeasure,
} from "./perf";

afterEach(() => {
  clearMeasures();
  vi.restoreAllMocks();
});

describe("perf named events", () => {
  it("exposes the §5.6 budgets for the tracked events", () => {
    expect(PERF_BUDGETS["space-switch"]).toBe(150);
    expect(PERF_BUDGETS["palette-open"]).toBe(100);
    expect(PERF_BUDGETS["first-token"]).toBe(50);
    expect(PERF_BUDGETS["lens-mount"]).toBe(300);
    expect(PERF_BUDGETS["list-scroll"]).toBeCloseTo(1000 / 60, 5);
  });

  it("records a measure with duration and budget metadata", () => {
    const handle = startMeasure("space-switch");
    const measure = endMeasure("space-switch", handle);

    expect(measure).not.toBeNull();
    expect(measure!.name).toBe("space-switch");
    expect(measure!.budgetMs).toBe(150);
    expect(measure!.duration).toBeGreaterThanOrEqual(0);
    expect(getMeasures()).toHaveLength(1);
  });

  it("flags an over-budget measure via measureSince", () => {
    const now = performance.now();
    // Started 500ms ago → over the 150ms space-switch budget.
    const measure = measureSince("space-switch", now - 500);
    expect(measure.overBudget).toBe(true);
    expect(measure.duration).toBeGreaterThanOrEqual(500);
  });

  it("does not flag an ad-hoc measure with no budget as over-budget", () => {
    const measure = measureSince("app-render", performance.now() - 9999);
    expect(measure.budgetMs).toBeNull();
    expect(measure.overBudget).toBe(false);
  });

  it("notifies subscribers on each new measure", () => {
    const seen: PerfMeasure[] = [];
    const unsubscribe = subscribe((m) => seen.push(m));

    const handle = startMeasure("palette-open");
    endMeasure("palette-open", handle);
    measureSince("first-token", performance.now());

    expect(seen).toHaveLength(2);
    expect(seen[0].name).toBe("palette-open");
    expect(seen[1].name).toBe("first-token");

    unsubscribe();
    measureSince("lens-mount", performance.now());
    expect(seen).toHaveLength(2); // no more notifications after unsubscribe
  });

  it("track() measures a synchronous function and returns its result", () => {
    const result = track("list-scroll", () => 21 * 2);
    expect(result).toBe(42);
    const measures = getMeasures();
    expect(measures[measures.length - 1]?.name).toBe("list-scroll");
  });

  it("returns null when the start mark is missing", () => {
    const measure = endMeasure("space-switch", "kria-ui:nonexistent:start:0");
    expect(measure).toBeNull();
  });

  it("caps the buffer and mirrors onto window.__KRIA_UI_METRICS__", () => {
    for (let i = 0; i < 250; i++) measureSince("list-scroll", performance.now());
    expect(getMeasures().length).toBeLessThanOrEqual(200);
    expect(window.__KRIA_UI_METRICS__?.length).toBe(getMeasures().length);
  });
});
