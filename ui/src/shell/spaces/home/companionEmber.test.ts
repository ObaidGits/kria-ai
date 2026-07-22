/**
 * companionEmber — pure-logic unit + property tests (task 8.3, Req 15).
 *
 * Covers the correctness properties the Companion ember rests on:
 *   • brighten ONLY for meaningful needs (Req 15.2),
 *   • compositor-fallback degrade decision (Req 15.5),
 *   • small edge-anchored ember geometry stays inside the work area (Req 13.4),
 *   • the on-by-default opt-out (Req 15.4),
 *   • the reposition anchor cycle + continuous-return mode memory (Req 15.3).
 */
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import fc from "fast-check";

import {
  COMPANION_ENABLED_STORAGE_KEY,
  DEFAULT_EMBER_ANCHOR,
  EMBER_ANCHORS,
  MEANINGFUL_NEED_STATES,
  companionPreference,
  emberAnchorStyle,
  emberWindowGeometry,
  isMeaningfulNeed,
  nextEmberAnchor,
  rememberPriorMode,
  resolveCompanionPresentation,
  returnViewMode,
} from "./companionEmber";
import { ATTENTION_STATES, type CoreState } from "../../../stores/coreStore";
import type { EdgeAnchor } from "../../../stores/homeStore";
import type { GeometryMonitor } from "../../../windowing/windowGeometry";

const ALL_CORE_STATES: readonly CoreState[] = [
  "idle", "listening", "thinking", "planning", "speaking", "responding", "acting",
  "running-automation", "watching", "remembering", "reflecting", "learning",
  "waiting", "blocked", "error", "recovering",
];

const coreStateArb = fc.constantFrom(...ALL_CORE_STATES);
const anchorArb = fc.constantFrom<EdgeAnchor>(...EMBER_ANCHORS);

const monitorArb: fc.Arbitrary<GeometryMonitor> = fc.record({
  workArea: fc.record({
    position: fc.record({ x: fc.integer({ min: -2000, max: 2000 }), y: fc.integer({ min: -2000, max: 2000 }) }),
    size: fc.record({ width: fc.integer({ min: 300, max: 6000 }), height: fc.integer({ min: 300, max: 6000 }) }),
  }),
  scaleFactor: fc.constantFrom(1, 1.25, 1.5, 2),
});

// ─── isMeaningfulNeed (Req 15.2) ─────────────────────────────────────────────

describe("isMeaningfulNeed — brighten only for meaningful needs (Req 15.2)", () => {
  it("brightens for the attention states and no others", () => {
    expect(isMeaningfulNeed("blocked")).toBe(true);
    expect(isMeaningfulNeed("waiting")).toBe(true);
    expect(isMeaningfulNeed("error")).toBe(true);
    // Ordinary work + idle must NOT brighten (no idle chatter).
    expect(isMeaningfulNeed("idle")).toBe(false);
    expect(isMeaningfulNeed("thinking")).toBe(false);
    expect(isMeaningfulNeed("acting")).toBe(false);
    expect(isMeaningfulNeed("responding")).toBe(false);
  });

  it("brightens for a pending approval or just-finished work regardless of state", () => {
    expect(isMeaningfulNeed("idle", { pendingApproval: true })).toBe(true);
    expect(isMeaningfulNeed("thinking", { workJustFinished: true })).toBe(true);
  });

  it("MEANINGFUL_NEED_STATES equals coreStore ATTENTION_STATES", () => {
    expect([...MEANINGFUL_NEED_STATES].sort()).toEqual([...ATTENTION_STATES].sort());
  });

  /**
   * Property: with no extra signals, the brighten gate is EXACTLY the attention
   * classification — for every Core state, brighten ⇔ attention-state. This is
   * the "brighten only on meaningful need, never idle chatter" invariant.
   *
   * Validates: Requirements 15.2
   */
  it("property: brighten ⇔ attention state (no signals)", () => {
    fc.assert(
      fc.property(coreStateArb, (state) => {
        expect(isMeaningfulNeed(state)).toBe(ATTENTION_STATES.has(state));
      }),
    );
  });

  /**
   * Property: an explicit meaningful signal (approval pending OR work finished)
   * ALWAYS brightens, for every Core state.
   *
   * Validates: Requirements 15.2
   */
  it("property: any meaningful signal forces brighten", () => {
    fc.assert(
      fc.property(coreStateArb, fc.boolean(), fc.boolean(), (state, pendingApproval, workJustFinished) => {
        const expected = ATTENTION_STATES.has(state) || pendingApproval || workJustFinished;
        expect(isMeaningfulNeed(state, { pendingApproval, workJustFinished })).toBe(expected);
      }),
    );
  });
});

// ─── resolveCompanionPresentation (Req 15.5) ─────────────────────────────────

describe("resolveCompanionPresentation — compositor fallback (Req 15.5)", () => {
  /**
   * Property: a floating always-on-top ember requires BOTH a Tauri host AND
   * compositor always-on-top support; otherwise it degrades to in-app — never
   * a broken state.
   *
   * Validates: Requirements 15.5
   */
  it("property: floating-window iff tauri AND always-on-top supported", () => {
    fc.assert(
      fc.property(fc.boolean(), fc.boolean(), (tauri, alwaysOnTopSupported) => {
        const result = resolveCompanionPresentation({ tauri, alwaysOnTopSupported });
        expect(result).toBe(tauri && alwaysOnTopSupported ? "floating-window" : "in-app");
      }),
    );
  });
});

// ─── emberWindowGeometry (Req 13.4) ──────────────────────────────────────────

describe("emberWindowGeometry — small, edge-anchored, inside the work area (Req 13.4)", () => {
  /**
   * Property: for any monitor + anchor, the ember geometry is a positive square
   * that fits entirely within the monitor work area.
   *
   * Validates: Requirements 13.4, 15.1
   */
  it("property: square ember stays within the work area", () => {
    fc.assert(
      fc.property(monitorArb, anchorArb, (monitor, anchor) => {
        const g = emberWindowGeometry(monitor, anchor);
        const work = monitor.workArea;
        expect(g.width).toBeGreaterThan(0);
        expect(g.width).toBe(g.height); // square ember
        expect(g.x).toBeGreaterThanOrEqual(work.position.x);
        expect(g.y).toBeGreaterThanOrEqual(work.position.y);
        expect(g.x + g.width).toBeLessThanOrEqual(work.position.x + work.size.width);
        expect(g.y + g.height).toBeLessThanOrEqual(work.position.y + work.size.height);
      }),
    );
  });
});

// ─── anchor cycle + CSS anchoring (Req 15.3 reposition) ──────────────────────

describe("nextEmberAnchor / emberAnchorStyle — reposition (Req 15.3)", () => {
  it("cycles through all four corners with period four", () => {
    let a: EdgeAnchor = DEFAULT_EMBER_ANCHOR;
    const seen = new Set<EdgeAnchor>();
    for (let i = 0; i < EMBER_ANCHORS.length; i++) {
      seen.add(a);
      a = nextEmberAnchor(a);
    }
    expect(seen.size).toBe(4);
    expect(a).toBe(DEFAULT_EMBER_ANCHOR); // returned to start after four hops
  });

  it("pins exactly one horizontal + one vertical edge per anchor", () => {
    for (const anchor of EMBER_ANCHORS) {
      const s = emberAnchorStyle(anchor);
      // Exactly one of top/bottom is auto, and one of left/right is auto.
      expect([s.top, s.bottom].filter((v) => v === "auto").length).toBe(1);
      expect([s.left, s.right].filter((v) => v === "auto").length).toBe(1);
    }
  });
});

// ─── on-by-default opt-out (Req 15.4) ────────────────────────────────────────

describe("companionPreference — on-by-default opt-out (Req 15.4)", () => {
  beforeEach(() => {
    window.localStorage.clear();
    companionPreference.refresh();
  });
  afterEach(() => {
    window.localStorage.clear();
    companionPreference.refresh();
  });

  it("defaults to enabled with no stored preference", () => {
    expect(companionPreference.enabled()).toBe(true);
  });

  it("opting out persists and disables; opting back in re-enables", () => {
    companionPreference.setEnabled(false);
    expect(companionPreference.enabled()).toBe(false);
    expect(window.localStorage.getItem(COMPANION_ENABLED_STORAGE_KEY)).toBe("false");

    companionPreference.refresh(); // survives a re-read (persistence)
    expect(companionPreference.enabled()).toBe(false);

    companionPreference.setEnabled(true);
    expect(companionPreference.enabled()).toBe(true);
    companionPreference.refresh();
    expect(companionPreference.enabled()).toBe(true);
  });

  it("treats any non-\"false\" stored value as enabled (on-by-default posture)", () => {
    window.localStorage.setItem(COMPANION_ENABLED_STORAGE_KEY, "garbage");
    companionPreference.refresh();
    expect(companionPreference.enabled()).toBe(true);
  });
});

// ─── continuous-return mode memory (Req 15.3) ────────────────────────────────

describe("rememberPriorMode / returnViewMode — continuous return (Req 15.3)", () => {
  it("remembers the last non-Companion mode and ignores Companion", () => {
    rememberPriorMode("immersive");
    expect(returnViewMode()).toBe("immersive");
    rememberPriorMode("companion"); // must not overwrite the return target
    expect(returnViewMode()).toBe("immersive");
    rememberPriorMode("mini");
    expect(returnViewMode()).toBe("mini");
    // restore a clean default for other tests
    rememberPriorMode("standard");
  });
});
