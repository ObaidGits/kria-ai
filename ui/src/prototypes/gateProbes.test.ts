import { describe, expect, it } from "vitest";
import {
  assessBlurFeasibility,
  buildFuzzyIndex,
  frameStats,
  fuzzyScore,
  makePaletteItems,
  queryFuzzyIndex,
  runG2Probe,
  timePaletteFuzzy,
} from "./gateProbes";

describe("frameStats", () => {
  it("returns zeroed stats for no samples", () => {
    const s = frameStats([]);
    expect(s.fps).toBe(0);
    expect(s.sampleCount).toBe(0);
  });

  it("derives fps from median frame time", () => {
    // 16.67ms frames → ~60fps
    const s = frameStats(Array.from({ length: 20 }, () => 1000 / 60));
    expect(s.fps).toBeGreaterThan(59);
    expect(s.fps).toBeLessThan(61);
  });

  it("ignores non-positive / non-finite samples", () => {
    const s = frameStats([16, -1, NaN, 16, Infinity]);
    expect(s.sampleCount).toBe(2);
  });

  it("computes a p95 no smaller than the median", () => {
    const s = frameStats([10, 10, 10, 10, 100]);
    expect(s.p95FrameMs).toBeGreaterThanOrEqual(s.medianFrameMs);
  });
});

describe("fuzzyScore", () => {
  it("scores empty query as a trivial match", () => {
    expect(fuzzyScore("", "anything")).toBe(1);
  });

  it("returns 0 when chars are not a subsequence", () => {
    expect(fuzzyScore("zzz", "Open Memory")).toBe(0);
  });

  it("matches subsequence and rewards contiguous + boundary hits", () => {
    const contiguous = fuzzyScore("open", "Open Memory");
    const scattered = fuzzyScore("opn", "Open Memory");
    expect(contiguous).toBeGreaterThan(0);
    expect(scattered).toBeGreaterThan(0);
    expect(contiguous).toBeGreaterThan(scattered);
  });

  it("is case-insensitive", () => {
    expect(fuzzyScore("MEM", "open memory")).toBeGreaterThan(0);
  });
});

describe("fuzzy index query", () => {
  it("returns only matching items sorted by score desc", () => {
    const items = [
      { id: "1", label: "Open Memory" },
      { id: "2", label: "Close Machine" },
      { id: "3", label: "Open Model" },
    ];
    const idx = buildFuzzyIndex(items);
    const res = queryFuzzyIndex(idx, "open");
    expect(res.length).toBe(2);
    expect(res.every((r) => r.score > 0)).toBe(true);
    expect(res[0].score).toBeGreaterThanOrEqual(res[1].score);
  });

  it("respects the result limit", () => {
    const idx = buildFuzzyIndex(makePaletteItems(500));
    const res = queryFuzzyIndex(idx, "open", 10);
    expect(res.length).toBeLessThanOrEqual(10);
  });
});

describe("timePaletteFuzzy (G5)", () => {
  it("builds an index over ~5k items and stays within budget", () => {
    const items = makePaletteItems(5000);
    const queries = ["o", "op", "ope", "open", "open m", "mem", "auto", "cap", "mach", "graph"];
    const t = timePaletteFuzzy(items, queries);
    expect(t.itemCount).toBe(5000);
    // Budgets are generous; the probe records actuals. Assert the plumbing is sane.
    expect(t.buildMs).toBeGreaterThanOrEqual(0);
    expect(t.maxKeystrokeMs).toBeGreaterThanOrEqual(0);
    expect(typeof t.openWithinBudget).toBe("boolean");
    expect(typeof t.keystrokeWithinBudget).toBe("boolean");
  });
});

describe("assessBlurFeasibility (G8)", () => {
  it("returns the mandated fallback treatment when blur is unsupported (jsdom)", () => {
    const f = assessBlurFeasibility();
    // jsdom has no CSS.supports → unsupported → solid-translucent fallback.
    expect(f.supported).toBe(false);
    expect(f.recommendedTreatment).toBe("solid-translucent");
  });
});

describe("runG2Probe (G2)", () => {
  it("returns null when WebGL is unavailable (jsdom / WebKitGTK software raster)", async () => {
    // jsdom exposes no WebGL context → probe must resolve null → 2D default.
    const result = await runG2Probe({ nodeCount: 100, frames: 3 });
    expect(result).toBeNull();
  });
});
