import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  clearAdaptiveUsage,
  dismissAdaptiveSuggestion,
  explainAdaptiveSuggestion,
  getAdaptiveUsage,
  isAdaptiveDismissed,
  isAdaptivePinned,
  MAX_ADAPTIVE_SHIFT,
  rankAdaptiveCandidates,
  rankQuickActions,
  recordAdaptiveUse,
  resetAdaptiveSuggestions,
  retireCoachHint,
  setAdaptivePinned,
  shouldShowCoachHint,
} from "./presentationRanking";

const candidates = (count: number) =>
  Array.from({ length: count }, (_, index) => ({ id: `item-${index}` }));

describe("bounded presentation adaptation (Req 19.1/19.2)", () => {
  beforeEach(() => resetAdaptiveSuggestions());

  it("promotes used candidates and demotes unused peers without deleting either", () => {
    const baseline = candidates(5);
    for (let use = 0; use < 4; use += 1) recordAdaptiveUse("quick-actions", "item-4");

    const ranked = rankQuickActions(baseline);
    expect(ranked.findIndex(({ id }) => id === "item-4")).toBe(2);
    expect(ranked.map(({ id }) => id).sort()).toEqual(baseline.map(({ id }) => id).sort());
  });

  it("keeps protected primary actions at their exact baseline position", () => {
    const primary = { id: "primary", adaptive: false };
    const baseline = [{ id: "secondary-a" }, primary, { id: "secondary-b" }];
    for (let use = 0; use < 10; use += 1) {
      recordAdaptiveUse("quick-actions", "secondary-b");
    }

    expect(rankQuickActions(baseline)[1]).toBe(primary);
  });

  it("only records presentation signals and never invokes candidate actions", () => {
    const run = vi.fn();
    const ranked = rankAdaptiveCandidates("palette", [{ id: "safe", run }]);
    expect(ranked).toHaveLength(1);
    expect(run).not.toHaveBeenCalled();
    expect(getAdaptiveUsage("palette", "safe")).toBeUndefined();
  });

  it("preserves bounded, deterministic order across generated candidate sets", () => {
    for (let count = 1; count <= 12; count += 1) {
      for (let target = 0; target < count; target += 1) {
        clearAdaptiveUsage();
        const baseline = candidates(count);
        for (let use = 0; use <= target; use += 1) {
          recordAdaptiveUse("empty-state", `item-${target}`);
        }
        const first = rankAdaptiveCandidates("empty-state", baseline);
        const second = rankAdaptiveCandidates("empty-state", baseline);
        expect(second).toEqual(first);
        expect(first).toHaveLength(baseline.length);
        for (const [index, candidate] of first.entries()) {
          const original = baseline.findIndex(({ id }) => id === candidate.id);
          expect(Math.abs(index - original)).toBeLessThanOrEqual(MAX_ADAPTIVE_SHIFT);
        }
      }
    }
  });

  it("isolates usage by designated zone", () => {
    recordAdaptiveUse("palette", "item-2");
    expect(rankQuickActions(candidates(3)).map(({ id }) => id)).toEqual([
      "item-0",
      "item-1",
      "item-2",
    ]);
  });
});


describe("explainable adaptive preferences and coach retirement (Req 19.3/19.4)", () => {
  beforeEach(() => resetAdaptiveSuggestions());

  it("explains defaults, learned use, and explicit pins", () => {
    expect(explainAdaptiveSuggestion("empty-state", "ask")).toBe("Default suggestion.");
    recordAdaptiveUse("empty-state", "ask");
    expect(explainAdaptiveSuggestion("empty-state", "ask")).toContain("recently");
    recordAdaptiveUse("empty-state", "ask");
    expect(explainAdaptiveSuggestion("empty-state", "ask")).toContain("often");
    setAdaptivePinned("empty-state", "ask", true);
    expect(explainAdaptiveSuggestion("empty-state", "ask")).toBe("Pinned by you.");
  });

  it("honors dismiss, pin, and reset while keeping candidates recoverable", () => {
    const baseline = candidates(5);
    dismissAdaptiveSuggestion("quick-actions", "item-1");
    setAdaptivePinned("quick-actions", "item-4", true);

    const adapted = rankQuickActions(baseline);
    expect(adapted.some(({ id }) => id === "item-1")).toBe(false);
    expect(isAdaptiveDismissed("quick-actions", "item-1")).toBe(true);
    expect(isAdaptivePinned("quick-actions", "item-4")).toBe(true);

    resetAdaptiveSuggestions("quick-actions");
    expect(rankQuickActions(baseline)).toEqual(baseline);
    expect(isAdaptiveDismissed("quick-actions", "item-1")).toBe(false);
    expect(isAdaptivePinned("quick-actions", "item-4")).toBe(false);
  });

  it("retires each first-run coach permanently after feature use", () => {
    const featureId = `test-feature-${Date.now()}-${Math.random()}`;
    expect(shouldShowCoachHint(featureId)).toBe(true);
    retireCoachHint(featureId);
    expect(shouldShowCoachHint(featureId)).toBe(false);
    resetAdaptiveSuggestions();
    expect(shouldShowCoachHint(featureId)).toBe(false);
  });

  it("preserves preference invariants across generated candidate sets", () => {
    for (let count = 1; count <= 12; count += 1) {
      const baseline = candidates(count);
      for (let selected = 0; selected < count; selected += 1) {
        resetAdaptiveSuggestions();
        const id = `item-${selected}`;
        setAdaptivePinned("quick-actions", id, true);
        expect(rankQuickActions(baseline).some((candidate) => candidate.id === id)).toBe(true);
        dismissAdaptiveSuggestion("quick-actions", id);
        expect(rankQuickActions(baseline).some((candidate) => candidate.id === id)).toBe(false);
        resetAdaptiveSuggestions("quick-actions");
        expect(rankQuickActions(baseline)).toEqual(baseline);
      }
    }
  });
});