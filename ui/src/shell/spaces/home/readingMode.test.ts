/**
 * Reading Mode — pure-logic unit + property tests (task 8.4, Req 11.1–11.4).
 *
 * Covers the correctness properties Reading Mode rests on:
 *   • First-send ALWAYS recedes, never navigates (Req 11.1) — from a resting
 *     homepage state, a first message resolves to a depth-recession `enter`
 *     (macro state → `reading`), and never any other outcome; transient overlays
 *     / companion are never hijacked.
 *   • Empty ALWAYS reverses (Req 11.3) — an emptied thread while reading resolves
 *     to `exit`, unwinding back to the resting homepage (Core forward, Room re-lit).
 *   • Message-stream dominance preserved (Req 11.4) — whenever the macro state is
 *     `reading`, the canonical focus target is the message stream.
 *   • The near-solid reading backing ALWAYS meets WCAG AA contrast (Req 11.2) for
 *     body/caption conversation text — for BOTH themes and over ANY receded-Room
 *     color it composites over (the near-solid backing guarantees legibility).
 *
 * The reading-mode decision logic is pure (`resolveReadingSync`), so the state
 * properties are verified with `fast-check` (pinned 3.23.2) without a DOM. The
 * contrast property reads the REAL generated token values so it can never drift
 * from what ships.
 */
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import fc from "fast-check";
// Raw import of the generated design tokens so the contrast property checks the
// ACTUAL shipped `--reading-backing` / `--reading-dim` values (no duplication).
import tokensCss from "../../../styles/tokens.generated.css?raw";
// Shared WCAG contrast primitives (task 10.1). Reused verbatim by the
// homepage-wide a11y AA property so there is ONE implementation, not two.
import {
  AA_BODY,
  contrastRatio,
  over,
  parseColor,
  themeBlock,
  tokenValue,
  type Rgba,
} from "./contrastAudit";

import {
  applyReadingSync,
  resolveReadingSync,
  type ReadingSyncAction,
} from "./readingMode";
import {
  homeStore,
  HOME_FOCUS_TARGET,
  type HomeState,
} from "../../../stores/homeStore";

const ALL_HOME_STATES: readonly HomeState[] = [
  "rest",
  "engaged",
  "reading",
  "mode-transition",
  "companion",
  "blocked",
];
/** The two RESTING homepage states a first send may recede from (Req 11.1). */
const RESTING_STATES: readonly HomeState[] = ["rest", "engaged"];
/** States that must never be hijacked by the reading sync. */
const NON_HIJACK_STATES: readonly HomeState[] = ["mode-transition", "companion", "blocked"];

const homeStateArb = fc.constantFrom(...ALL_HOME_STATES);

afterEach(() => homeStore.reset());

// ─── resolveReadingSync — first send recedes, never navigates (Req 11.1) ──────

describe("resolveReadingSync — first send → depth-recession (Req 11.1)", () => {
  it("recedes on first message from a resting homepage state", () => {
    expect(resolveReadingSync({ hasMessages: true, homeState: "engaged" })).toEqual({
      kind: "enter",
      via: "direct",
    });
    // From `rest`, `reading` is only reachable via `engaged`, so it engages first.
    expect(resolveReadingSync({ hasMessages: true, homeState: "rest" })).toEqual({
      kind: "enter",
      via: "engage-first",
    });
  });

  it("does not recede when there are no messages", () => {
    for (const homeState of RESTING_STATES) {
      expect(resolveReadingSync({ hasMessages: false, homeState })).toEqual({ kind: "none" });
    }
  });

  it("never hijacks a transient overlay or the companion ember", () => {
    for (const homeState of NON_HIJACK_STATES) {
      expect(resolveReadingSync({ hasMessages: true, homeState })).toEqual({ kind: "none" });
    }
  });

  /**
   * Property: for EVERY resting homepage state, a first message ALWAYS resolves
   * to a depth-recession `enter` — never `exit`, never `none`, never anything
   * that could navigate away. Navigation is structurally impossible (the sync
   * only ever touches `homeStore` macro state), so "recedes, not navigates" is
   * captured by: the only positive outcome is `enter` and applying it lands in
   * `reading` (message-stream focus), not any other state.
   *
   * Validates: Requirements 11.1
   */
  it("property: first send from a resting state always recedes (never navigates)", () => {
    fc.assert(
      fc.property(fc.constantFrom(...RESTING_STATES), (homeState) => {
        const action = resolveReadingSync({ hasMessages: true, homeState });
        expect(action.kind).toBe("enter");

        homeStore.reset();
        if (homeState === "engaged") homeStore.engage();
        applyReadingSync(action);

        // Recession landed in Reading Mode — the same space, receded — and NOT
        // some navigation target.
        expect(homeStore.state()).toBe("reading");
        expect(homeStore.readingMode()).toBe(true);
      }),
    );
  });

  /**
   * Property: the reading sync NEVER fires from a transient overlay
   * (blocked / mode-transition) or the companion ember, regardless of messages.
   *
   * Validates: Requirements 11.1, 30.1
   */
  it("property: transient/companion states are never auto-entered/exited", () => {
    fc.assert(
      fc.property(fc.constantFrom(...NON_HIJACK_STATES), fc.boolean(), (homeState, hasMessages) => {
        expect(resolveReadingSync({ hasMessages, homeState })).toEqual({ kind: "none" });
      }),
    );
  });
});

// ─── resolveReadingSync — empty reverses (Req 11.3) ───────────────────────────

describe("resolveReadingSync — empty thread → reverse (Req 11.3)", () => {
  it("reverses out of reading when the thread empties", () => {
    expect(resolveReadingSync({ hasMessages: false, homeState: "reading" })).toEqual({
      kind: "exit",
    });
  });

  it("stays in reading while messages remain (no thrash)", () => {
    expect(resolveReadingSync({ hasMessages: true, homeState: "reading" })).toEqual({
      kind: "none",
    });
  });

  /**
   * Property: an emptied thread while reading ALWAYS reverses — applying the
   * action floats the Core forward + re-lights the Room by returning to the
   * resting homepage (`rest`), never leaving a dangling reading surface.
   *
   * Validates: Requirements 11.3
   */
  it("property: empty always reverses back to the resting homepage", () => {
    fc.assert(
      fc.property(fc.constant(true), () => {
        homeStore.reset();
        homeStore.engage();
        homeStore.enterReading();
        expect(homeStore.readingMode()).toBe(true);

        const action = resolveReadingSync({ hasMessages: false, homeState: homeStore.state() });
        expect(action.kind).toBe("exit");
        applyReadingSync(action);

        expect(homeStore.readingMode()).toBe(false);
        expect(homeStore.state()).toBe("rest"); // Core forward, Room re-lit
      }),
    );
  });
});

// ─── Sync convergence + message-stream dominance (Req 11.1/11.3/11.4) ─────────

describe("reading sync convergence + dominance (Req 11.1/11.3/11.4)", () => {
  it("HOME_FOCUS_TARGET.reading is the message stream (dominance, Req 11.4)", () => {
    expect(HOME_FOCUS_TARGET.reading).toBe("message-stream");
  });

  /**
   * Property: driving the pure sync over an ARBITRARY sequence of message-count
   * changes (starting from rest) keeps the homepage in lock-step with the
   * conversation — after each step `readingMode()` equals "the thread has
   * messages" — and WHENEVER the macro state is `reading`, the dominant focus
   * target is always the message stream (conversation-dominance preserved).
   * Also proves idempotence / no thrash: replaying the same observation is a
   * no-op.
   *
   * Validates: Requirements 11.1, 11.3, 11.4
   */
  it("property: sync tracks the conversation and reading always focuses the stream", () => {
    fc.assert(
      fc.property(fc.array(fc.boolean(), { minLength: 1, maxLength: 40 }), (sequence) => {
        homeStore.reset();
        for (const hasMessages of sequence) {
          // Two applications of the same observation to prove idempotence: the
          // second must be a no-op (the sync has converged).
          applyReadingSync(resolveReadingSync({ hasMessages, homeState: homeStore.state() }));
          const settled = homeStore.state();
          applyReadingSync(resolveReadingSync({ hasMessages, homeState: homeStore.state() }));
          expect(homeStore.state()).toBe(settled);

          // Lock-step with the conversation.
          expect(homeStore.readingMode()).toBe(hasMessages);
          // Conversation-dominance: reading always focuses the message stream.
          if (homeStore.readingMode()) {
            expect(homeStore.focusTarget()).toBe("message-stream");
          }
        }
      }),
    );
  });
});

// ─── Reading backing AA contrast (Req 11.2) ───────────────────────────────────

interface ThemeTokens {
  backing: Rgba;
  dim: Rgba;
  textPrimary: Rgba;
  textSecondary: Rgba;
}

function readTheme(css: string, selector: string): ThemeTokens {
  const block = themeBlock(css, selector);
  return {
    backing: parseColor(tokenValue(block, "--reading-backing")),
    dim: parseColor(tokenValue(block, "--reading-dim")),
    textPrimary: parseColor(tokenValue(block, "--color-text-primary")),
    textSecondary: parseColor(tokenValue(block, "--color-text-secondary")),
  };
}

describe("reading backing meets WCAG AA contrast (Req 11.2)", () => {
  const dark = readTheme(tokensCss, ":root");
  const light = readTheme(tokensCss, '[data-theme="light"]');
  const THEMES: ReadonlyArray<readonly [string, ThemeTokens]> = [
    ["dark", dark],
    ["light", light],
  ];

  it("body + caption text clears AA over the composited reading backing (both themes)", () => {
    for (const [name, t] of THEMES) {
      // Worst realistic case: backing over the hard-dim over the room center
      // (the lightest room point in dark, the darkest in light).
      const roomCenterByTheme: Rgba =
        name === "dark" ? { r: 23, g: 32, b: 39, a: 1 } : { r: 232, g: 239, b: 244, a: 1 };
      const bg = over(t.backing, over(t.dim, roomCenterByTheme));
      expect(contrastRatio(t.textPrimary, bg)).toBeGreaterThanOrEqual(AA_BODY);
      expect(contrastRatio(t.textSecondary, bg)).toBeGreaterThanOrEqual(AA_BODY);
    }
  });

  /**
   * Property: the near-solid reading backing guarantees AA for body AND caption
   * text over the receded Room NO MATTER what color the Room recedes to. For any
   * opaque room base, `backing` composited over `dim` over that base keeps
   * text-primary and text-secondary at ≥ 4.5:1 in both themes. This is the
   * legibility guarantee (guardrails.md "Reading Mode meets text contrast on its
   * near-solid backing").
   *
   * Validates: Requirements 11.2
   */
  it("property: AA holds over ANY receded-Room color (both themes)", () => {
    const channel = fc.integer({ min: 0, max: 255 });
    fc.assert(
      fc.property(channel, channel, channel, fc.constantFrom(...THEMES), (r, g, b, [, t]) => {
        const roomBase: Rgba = { r, g, b, a: 1 };
        const bg = over(t.backing, over(t.dim, roomBase));
        expect(contrastRatio(t.textPrimary, bg)).toBeGreaterThanOrEqual(AA_BODY);
        expect(contrastRatio(t.textSecondary, bg)).toBeGreaterThanOrEqual(AA_BODY);
      }),
    );
  });
});
