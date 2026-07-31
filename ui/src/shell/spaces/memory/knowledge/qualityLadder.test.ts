import { describe, it, expect } from "vitest";
import {
  selectQualityLevel,
  isListFirst,
  maxItemsForLevel,
  type QualityLevel,
  type SystemPressure,
} from "./qualityLadder";

// ─── Fixtures ─────────────────────────────────────────────────────────────────

/** Healthy system pressure — no throttling, low CPU. */
const HEALTHY: SystemPressure = {
  memoryPressureBytes: 0,
  cpuUtilisationPercent: 30,
  thermalState: "nominal",
  batteryPercent: 80,
};

// ─── selectQualityLevel ───────────────────────────────────────────────────────

describe("selectQualityLevel", () => {
  describe("list-first conditions", () => {
    it("returns list-first when canvasAvailable=false regardless of pressure", () => {
      expect(selectQualityLevel(HEALTHY, 10, false)).toBe("list-first");
      expect(selectQualityLevel(HEALTHY, 0, false)).toBe("list-first");
      expect(selectQualityLevel({ ...HEALTHY, cpuUtilisationPercent: 20 }, 50, false)).toBe("list-first");
    });

    it("returns list-first when thermalState=critical", () => {
      const p: SystemPressure = { ...HEALTHY, thermalState: "critical" };
      expect(selectQualityLevel(p, 10, true)).toBe("list-first");
    });

    it("returns list-first when cpuUtilisationPercent >= 90", () => {
      const p90: SystemPressure = { ...HEALTHY, cpuUtilisationPercent: 90 };
      expect(selectQualityLevel(p90, 10, true)).toBe("list-first");

      const p95: SystemPressure = { ...HEALTHY, cpuUtilisationPercent: 95 };
      expect(selectQualityLevel(p95, 5, true)).toBe("list-first");

      const p100: SystemPressure = { ...HEALTHY, cpuUtilisationPercent: 100 };
      expect(selectQualityLevel(p100, 50, true)).toBe("list-first");
    });

    it("critical thermal takes priority over all other rules", () => {
      // Even with low CPU and few items, critical thermal → list-first
      const p: SystemPressure = { ...HEALTHY, cpuUtilisationPercent: 5, thermalState: "critical" };
      expect(selectQualityLevel(p, 5, true)).toBe("list-first");
    });

    it("canvasAvailable=false takes priority even over healthy pressure", () => {
      expect(selectQualityLevel(HEALTHY, 50, false)).toBe("list-first");
    });
  });

  describe("decoration-only conditions", () => {
    it("returns decoration-only when thermalState=throttled", () => {
      const p: SystemPressure = { ...HEALTHY, thermalState: "throttled" };
      expect(selectQualityLevel(p, 10, true)).toBe("decoration-only");
    });

    it("throttled caps at decoration-only even with few items and low CPU", () => {
      const p: SystemPressure = { ...HEALTHY, cpuUtilisationPercent: 10, thermalState: "throttled" };
      expect(selectQualityLevel(p, 5, true)).toBe("decoration-only");
    });

    it("returns decoration-only when sceneItemCount > 180", () => {
      expect(selectQualityLevel(HEALTHY, 181, true)).toBe("decoration-only");
      expect(selectQualityLevel(HEALTHY, 500, true)).toBe("decoration-only");
    });

    it("exactly 180 items is NOT decoration-only (falls into scene-120 range)", () => {
      // 180 > 120 is true, so it returns scene-120 (not scene-180, not decoration-only)
      expect(selectQualityLevel(HEALTHY, 180, true)).toBe("scene-120");
    });
  });

  describe("scene-120 condition", () => {
    it("returns scene-120 when sceneItemCount > 120 and <= 180", () => {
      expect(selectQualityLevel(HEALTHY, 121, true)).toBe("scene-120");
      expect(selectQualityLevel(HEALTHY, 150, true)).toBe("scene-120");
      expect(selectQualityLevel(HEALTHY, 180, true)).toBe("scene-120"); // 180 > 120 → scene-120
    });

    it("exactly 120 items does NOT trigger scene-120", () => {
      expect(selectQualityLevel(HEALTHY, 120, true)).toBe("scene-180");
    });
  });

  describe("with-labels condition", () => {
    it("returns with-labels when cpuUtilisationPercent >= 70 (below 90)", () => {
      const p70: SystemPressure = { ...HEALTHY, cpuUtilisationPercent: 70 };
      expect(selectQualityLevel(p70, 10, true)).toBe("with-labels");

      const p80: SystemPressure = { ...HEALTHY, cpuUtilisationPercent: 80 };
      expect(selectQualityLevel(p80, 50, true)).toBe("with-labels");

      const p89: SystemPressure = { ...HEALTHY, cpuUtilisationPercent: 89 };
      expect(selectQualityLevel(p89, 10, true)).toBe("with-labels");
    });
  });

  describe("scene-180 (normal conditions)", () => {
    it("returns scene-180 under normal conditions (item count <= 120)", () => {
      expect(selectQualityLevel(HEALTHY, 10, true)).toBe("scene-180");
      expect(selectQualityLevel(HEALTHY, 0, true)).toBe("scene-180");
      expect(selectQualityLevel(HEALTHY, 120, true)).toBe("scene-180");
    });

    it("returns scene-180 when CPU is 69% (just below with-labels threshold)", () => {
      const p: SystemPressure = { ...HEALTHY, cpuUtilisationPercent: 69 };
      expect(selectQualityLevel(p, 10, true)).toBe("scene-180");
    });
  });
});

// ─── isListFirst ──────────────────────────────────────────────────────────────

describe("isListFirst", () => {
  it("returns true for list-first", () => {
    expect(isListFirst("list-first")).toBe(true);
  });

  it("returns false for all other quality levels", () => {
    const others: QualityLevel[] = [
      "decoration-only",
      "with-labels",
      "with-analytics",
      "scene-120",
      "scene-180",
    ];
    for (const level of others) {
      expect(isListFirst(level)).toBe(false);
    }
  });
});

// ─── maxItemsForLevel ─────────────────────────────────────────────────────────

describe("maxItemsForLevel", () => {
  it("returns correct max for each level", () => {
    const cases: [QualityLevel, number][] = [
      ["scene-180", 180],
      ["scene-120", 120],
      ["with-analytics", 120],
      ["with-labels", 120],
      ["decoration-only", 240],
      ["list-first", 0],
    ];

    for (const [level, expected] of cases) {
      expect(maxItemsForLevel(level)).toBe(expected);
    }
  });
});
