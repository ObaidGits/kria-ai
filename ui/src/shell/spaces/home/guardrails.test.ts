import { describe, it, expect } from "vitest";
// Raw source of homeStore, imported type-safely via vite/client's `*?raw`
// module declaration (avoids a node:fs type dependency in the src tsconfig).
import homeStoreSource from "../../../stores/homeStore.ts?raw";
import {
  MAX_CHIPS,
  checkSingleAcs,
  checkChipBudget,
  checkCoreHintAdvisory,
  checkFocusFrame,
  assertFocusFrame,
  RESTING_FILLER_SELECTORS,
  DISSOLVES_WHEN_EMPTY_ATTR,
  findRestingFiller,
  findEmptyStandingSurfaces,
  checkRestingCalm,
  assertRestingCalm,
  GuardrailViolationError,
  type GuardrailChip,
  type GuardrailFocusFrame,
} from "./guardrails";
// Standalone linter runs in Node; Vitest imports its pure detectors directly.
// @ts-expect-error Standalone ESM script has no generated declaration file.
import { findCoreStoreWrites, findAccentOnRoomBase, findForbiddenCaptureKinds, findAwarenessNetworkEgress } from "../../../../scripts/guardrail-lint.mjs";
// Raw source of the desktop-awareness modules, scanned by the privacy detectors.
import desktopAwarenessSource from "../../../stores/desktopAwarenessBridge.ts?raw";
import awarenessPrivacySource from "../../../stores/awarenessPrivacy.ts?raw";

const chips = (n: number): GuardrailChip[] =>
  Array.from({ length: n }, (_, i) => ({ id: `c${i}`, label: `chip ${i}`, kind: "route" as const }));

describe("homepage guardrails — runtime FocusFrame validators (Req 30.2)", () => {
  describe("single ACS (Req 8.1/8.4)", () => {
    it("allows a single ACS subject", () => {
      expect(checkSingleAcs({ acs: { subjectId: "s1" } })).toBeNull();
      expect(checkSingleAcs({ acs: null })).toBeNull();
      expect(checkSingleAcs({})).toBeNull();
    });

    it("flags two ACS surfaces at once", () => {
      const v = checkSingleAcs({ acs: [{ subjectId: "s1" }, { subjectId: "s2" }] });
      expect(v?.rule).toBe("single-acs");
    });
  });

  describe("chip budget (Req 5.1, ≤3)", () => {
    it("allows up to MAX_CHIPS chips", () => {
      expect(MAX_CHIPS).toBe(3);
      expect(checkChipBudget({ chips: chips(3) })).toBeNull();
      expect(checkChipBudget({ chips: [] })).toBeNull();
      expect(checkChipBudget({})).toBeNull();
    });

    it("flags more than three chips", () => {
      const v = checkChipBudget({ chips: chips(4) });
      expect(v?.rule).toBe("chip-budget");
    });
  });

  describe("coreHint advisory-only (Req 30.3)", () => {
    it("allows an advisory string hint", () => {
      expect(checkCoreHintAdvisory({ coreHint: "thinking" })).toBeNull();
      expect(checkCoreHintAdvisory({})).toBeNull();
    });

    it("flags a non-string (authoritative-looking) hint", () => {
      const v = checkCoreHintAdvisory({ coreHint: { setState: "blocked" } as unknown as string });
      expect(v?.rule).toBe("corehint-advisory");
    });
  });

  describe("checkFocusFrame / assertFocusFrame", () => {
    it("returns no violations for a clean frame", () => {
      const frame: GuardrailFocusFrame = { acs: { subjectId: "s1" }, chips: chips(2), coreHint: "idle" };
      expect(checkFocusFrame(frame)).toEqual([]);
      expect(() => assertFocusFrame(frame)).not.toThrow();
    });

    it("throws a release-blocking error listing every violation", () => {
      const bad: GuardrailFocusFrame = {
        acs: [{ subjectId: "a" }, { subjectId: "b" }] as unknown,
        chips: chips(5),
      };
      expect(checkFocusFrame(bad)).toHaveLength(2);
      expect(() => assertFocusFrame(bad)).toThrow(GuardrailViolationError);
    });
  });
});

describe("homepage guardrails — resting calm / no filler (Req 1.5, 8.3)", () => {
  /** Build a detached homepage root with the given inner HTML. */
  const root = (html: string): HTMLElement => {
    const el = document.createElement("div");
    el.innerHTML = html;
    return el;
  };

  describe("no dashboard-style filler at rest (Req 1.5)", () => {
    it("passes for a calm homepage (Core + greeting only)", () => {
      const el = root(
        `<div class="kria-home__core"><span role="img" aria-label="idle"></span></div>` +
          `<h2 data-slot="greeting">What can I help with?</h2>`,
      );
      expect(findRestingFiller(el)).toEqual([]);
      expect(checkRestingCalm(el)).toEqual([]);
      expect(() => assertRestingCalm(el)).not.toThrow();
    });

    it("flags each kind of forbidden filler marker", () => {
      for (const selector of RESTING_FILLER_SELECTORS) {
        // Turn `[data-x]` into an element carrying that attribute.
        const attr = selector.replace(/^\[|\]$/g, "");
        const el = root(`<div ${attr}>filler</div>`);
        const violations = findRestingFiller(el);
        expect(violations).toHaveLength(1);
        expect(violations[0]?.rule).toBe("resting-filler");
      }
    });

    it("flags a stat tile / chart / activity feed on the resting homepage", () => {
      const el = root(
        `<div data-stat-tile>3 tasks</div>` +
          `<div data-chart></div>` +
          `<div data-activity-feed></div>`,
      );
      expect(findRestingFiller(el)).toHaveLength(3);
      expect(() => assertRestingCalm(el)).toThrow(GuardrailViolationError);
    });
  });

  describe("dissolvable surfaces never render an empty box (Req 8.3)", () => {
    it("passes when a dissolvable surface has real content", () => {
      const el = root(`<div ${DISSOLVES_WHEN_EMPTY_ATTR}><p>Meeting in 20</p></div>`);
      expect(findEmptyStandingSurfaces(el)).toEqual([]);
    });

    it("flags a present-but-empty ACS (no children, no text)", () => {
      const el = root(`<div ${DISSOLVES_WHEN_EMPTY_ATTR}></div>`);
      const violations = findEmptyStandingSurfaces(el);
      expect(violations).toHaveLength(1);
      expect(violations[0]?.rule).toBe("empty-surface");
    });

    it("flags a surface explicitly marked data-empty=true even with markup", () => {
      const el = root(`<div ${DISSOLVES_WHEN_EMPTY_ATTR} data-empty="true"><span></span></div>`);
      expect(findEmptyStandingSurfaces(el)).toHaveLength(1);
    });

    it("treats whitespace-only content as empty", () => {
      const el = root(`<div ${DISSOLVES_WHEN_EMPTY_ATTR}>   </div>`);
      expect(findEmptyStandingSurfaces(el)).toHaveLength(1);
    });
  });

  it("checkRestingCalm aggregates both filler and empty-surface violations", () => {
    const el = root(`<div data-widget></div><div ${DISSOLVES_WHEN_EMPTY_ATTR}></div>`);
    const violations = checkRestingCalm(el);
    expect(violations.map((v) => v.rule).sort()).toEqual(["empty-surface", "resting-filler"]);
    expect(() => assertRestingCalm(el)).toThrow(GuardrailViolationError);
  });
});

describe("homepage guardrails — static lint detectors (Req 30.3 / accent discipline)", () => {
  describe("coreHint never written back to coreStore", () => {
    it("flags a coreStore mutator call", () => {
      expect(findCoreStoreWrites("coreStore.setState('blocked')")).toHaveLength(1);
      expect(findCoreStoreWrites("coreStore.setBlocked('x'); coreStore.ingest(e)")).toHaveLength(2);
    });

    it("does not flag read-only coreStore access", () => {
      expect(findCoreStoreWrites("const s = coreStore.state();")).toHaveLength(0);
      expect(findCoreStoreWrites("coreStore.isIdle()")).toHaveLength(0);
    });

    it("homeStore.ts contains no coreStore write-back", () => {
      expect(findCoreStoreWrites(homeStoreSource)).toEqual([]);
    });
  });

  describe("no accent on the Room base", () => {
    it("flags the accent token inside a Room-base rule", () => {
      const css = ".kria-room__base { background: var(--color-accent-500); }";
      expect(findAccentOnRoomBase(css)).toHaveLength(1);
    });

    it("does not flag accent used outside the Room base", () => {
      const css = ".kria-core__pool { color: var(--color-accent-500); }";
      expect(findAccentOnRoomBase(css)).toHaveLength(0);
    });

    it("does not flag a Room base that uses only room tokens", () => {
      const css = ".kria-room__base { background: var(--room-gradient-top); }";
      expect(findAccentOnRoomBase(css)).toHaveLength(0);
    });
  });

  describe("awareness privacy — no forbidden capture kinds (Req 25.4)", () => {
    it("flags a forbidden capture kind used as a source integration", () => {
      expect(findForbiddenCaptureKinds('integration: "keylog",')).toHaveLength(1);
      expect(findForbiddenCaptureKinds('integration: "clipboard-capture"')).toHaveLength(1);
      expect(findForbiddenCaptureKinds('integration: "screen-content-capture"')).toHaveLength(1);
    });

    it("does not flag an allowlisted local integration", () => {
      expect(findForbiddenCaptureKinds('integration: "mpris",')).toHaveLength(0);
      expect(findForbiddenCaptureKinds('integration: "calendar-integration",')).toHaveLength(0);
    });

    it("the real desktop-awareness modules declare no forbidden capture kind", () => {
      expect(findForbiddenCaptureKinds(desktopAwarenessSource)).toEqual([]);
      expect(findForbiddenCaptureKinds(awarenessPrivacySource)).toEqual([]);
    });
  });

  describe("awareness privacy — all-local, no network egress (Req 25.5)", () => {
    it("flags outbound-network primitives", () => {
      expect(findAwarenessNetworkEgress("await fetch('https://x')")).toHaveLength(1);
      expect(findAwarenessNetworkEgress("new WebSocket('wss://x')")).toHaveLength(1);
      expect(findAwarenessNetworkEgress("navigator.sendBeacon('/x')")).toHaveLength(1);
    });

    it("the real desktop-awareness modules perform no network egress", () => {
      expect(findAwarenessNetworkEgress(desktopAwarenessSource)).toEqual([]);
      expect(findAwarenessNetworkEgress(awarenessPrivacySource)).toEqual([]);
    });
  });
});
