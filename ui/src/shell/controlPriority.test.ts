import { describe, it, expect } from "vitest";
import {
  CONVERSE_CONTROLS,
  CRITICAL_CONTROL_IDS,
  TIER_RANK,
  STATUS_RANK,
  compareByTier,
  compareStatusPriority,
  controlTier,
  partitionControls,
  type TieredControl,
} from "./controlPriority";

const overflowIds = (r: { overflow: TieredControl[] }) => r.overflow.map((c) => c.id);
const inlineIds = (r: { inline: TieredControl[] }) => r.inline.map((c) => c.id);

describe("controlPriority tier model", () => {
  it("orders tiers critical → primary → secondary via comparator", () => {
    expect(TIER_RANK.critical).toBeLessThan(TIER_RANK.primary);
    expect(TIER_RANK.primary).toBeLessThan(TIER_RANK.secondary);
    expect(compareByTier({ id: "a", tier: "critical" }, { id: "b", tier: "primary" })).toBeLessThan(0);
    expect(compareByTier({ id: "a", tier: "primary" }, { id: "b", tier: "secondary" })).toBeLessThan(0);
    expect(compareByTier({ id: "a", tier: "secondary" }, { id: "b", tier: "secondary" })).toBe(0);
  });

  it("never partitions critical controls into overflow, even at zero capacity", () => {
    for (const maxInline of [0, -5, 1, 2, 3]) {
      const result = partitionControls(CONVERSE_CONTROLS, maxInline);
      for (const id of CRITICAL_CONTROL_IDS) {
        expect(overflowIds(result)).not.toContain(id);
        expect(inlineIds(result)).toContain(id);
      }
    }
  });

  it("moves secondary controls to overflow before primary", () => {
    // Capacity fits all criticals + exactly one non-critical slot.
    const criticalCount = CONVERSE_CONTROLS.filter((c) => c.tier === "critical").length;
    const result = partitionControls(CONVERSE_CONTROLS, criticalCount + 1);
    const overflow = overflowIds(result);
    // Every secondary overflowed...
    for (const c of CONVERSE_CONTROLS.filter((c) => c.tier === "secondary")) {
      expect(overflow).toContain(c.id);
    }
    // ...before all primaries: at least one primary stayed inline.
    const primaryInline = inlineIds(result).filter(
      (id) => controlTier(id) === "primary",
    );
    expect(primaryInline.length).toBeGreaterThan(0);
  });

  it("keeps everything inline when capacity is sufficient", () => {
    const result = partitionControls(CONVERSE_CONTROLS, CONVERSE_CONTROLS.length);
    expect(result.overflow).toHaveLength(0);
    expect(result.inline).toHaveLength(CONVERSE_CONTROLS.length);
  });

  it("preserves original relative order within each partition", () => {
    const result = partitionControls(CONVERSE_CONTROLS, 6);
    const order = (ids: string[]) =>
      ids.map((id) => CONVERSE_CONTROLS.findIndex((c) => c.id === id));
    const inlineOrder = order(inlineIds(result));
    const overflowOrder = order(overflowIds(result));
    expect([...inlineOrder].sort((a, b) => a - b)).toEqual(inlineOrder);
    expect([...overflowOrder].sort((a, b) => a - b)).toEqual(overflowOrder);
  });
});

describe("critical affordance invariant (data-driven)", () => {
  it("classifies approvals/Send-Stop/recovery/critical-status/composer as critical", () => {
    for (const id of CRITICAL_CONTROL_IDS) {
      expect(controlTier(id)).toBe("critical");
    }
  });

  it("exposes the §29 affordance-priority set exactly", () => {
    const criticalInMap = CONVERSE_CONTROLS.filter((c) => c.tier === "critical").map((c) => c.id);
    expect(criticalInMap.sort()).toEqual([...CRITICAL_CONTROL_IDS].sort());
  });

  it("classifies convenience actions as secondary and active toggles/tools as primary", () => {
    expect(controlTier("export")).toBe("secondary");
    expect(controlTier("detach")).toBe("secondary");
    expect(controlTier("open-sidebar")).toBe("secondary");
    expect(controlTier("context-rail-toggle")).toBe("primary");
    expect(controlTier("mode-chip")).toBe("primary");
  });
});

describe("status priority ordering (§29)", () => {
  it("ranks approval/error/scoped-control → active work → context → idle", () => {
    expect(STATUS_RANK.critical).toBeLessThan(STATUS_RANK["active-work"]);
    expect(STATUS_RANK["active-work"]).toBeLessThan(STATUS_RANK.context);
    expect(STATUS_RANK.context).toBeLessThan(STATUS_RANK.idle);
    const sorted = (["idle", "critical", "context", "active-work"] as const)
      .slice()
      .sort(compareStatusPriority);
    expect(sorted).toEqual(["critical", "active-work", "context", "idle"]);
  });
});
