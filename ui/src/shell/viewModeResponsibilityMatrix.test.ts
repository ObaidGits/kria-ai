/**
 * Task 8.7 — View Mode Responsibility Matrix (design.md §29).
 *
 * Two guarantees are proven here:
 *   1. The matrix is TOTAL and faithful — every canonical View Mode × every
 *      homepage element has a well-formed responsibility, transcribed exactly
 *      from the §29 table, with `conditional` cells always carrying a trigger.
 *   2. The declared "persists across all modes" state (active thread, Core
 *      emotional state, Composer draft, current Focus subject) survives EVERY
 *      mode-pair transition unchanged — verified both at the homeStore state
 *      machine (which every mode composition passes through) and through the
 *      real `modeTransitionCoordinator` funnel that every keyboard/palette/AT
 *      trigger uses.
 *
 * Validates: Requirements 13.1, 13.2, 13.3, 13.5
 */
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import fc from "fast-check";

import {
  HOME_ELEMENTS,
  PERSISTENT_SHARED_STATE,
  VIEW_MODES,
  VIEW_MODE_RESPONSIBILITY_MATRIX,
  elementResponsibility,
  isElementVisibleAtRest,
  type HomeElement,
} from "./viewModeResponsibilityMatrix";
import { homeStore, type HomeViewMode, type HomeSharedContext } from "../stores/homeStore";
import { coreStore, type CoreState } from "../stores/coreStore";
import { converseStore } from "../stores/converseStore";
import { shellStore } from "../stores/shellStore";
import {
  requestWindowMode,
  settlePendingModeTransition,
  syncViewModeFromShell,
} from "../windowing/modeTransitionCoordinator";

const CORE_STATES: readonly CoreState[] = ["idle", "thinking", "blocked", "listening", "acting"];

// ─── 1. Faithful, total transcription of the §29 table ────────────────────────

describe("§29 responsibility matrix — faithful transcription", () => {
  it("maps every cell exactly as design.md §29 specifies", () => {
    // A compact fixture of the §29 table: [presence, trigger|undefined].
    type Cell = [HomeElement, "shown" | "hidden" | "conditional", string | undefined];
    const expected: Record<HomeViewMode, Cell[]> = {
      immersive: [
        ["core", "shown", undefined],
        ["room", "shown", undefined],
        ["voiceLine", "shown", undefined],
        ["composer", "shown", undefined],
        ["chips", "shown", undefined],
        ["acs", "shown", undefined],
        ["orbit", "shown", undefined],
        ["hiddenDock", "conditional", "intent"],
        ["notifications", "shown", undefined],
        ["conversation", "shown", undefined],
      ],
      standard: [
        ["core", "shown", undefined],
        ["room", "shown", undefined],
        ["voiceLine", "shown", undefined],
        ["composer", "shown", undefined],
        ["chips", "shown", undefined],
        ["acs", "shown", undefined],
        ["orbit", "shown", undefined],
        ["hiddenDock", "conditional", "intent"],
        ["notifications", "shown", undefined],
        ["conversation", "shown", undefined],
      ],
      mini: [
        ["core", "shown", undefined],
        ["room", "shown", undefined],
        ["voiceLine", "shown", undefined],
        ["composer", "shown", undefined],
        ["chips", "hidden", undefined],
        ["acs", "hidden", undefined],
        ["orbit", "hidden", undefined],
        ["hiddenDock", "conditional", "palette"],
        ["notifications", "conditional", "critical"],
        ["conversation", "shown", undefined],
      ],
      companion: [
        ["core", "shown", undefined],
        ["room", "hidden", undefined],
        ["voiceLine", "conditional", "brighten"],
        ["composer", "conditional", "click"],
        ["chips", "hidden", undefined],
        ["acs", "hidden", undefined],
        ["orbit", "hidden", undefined],
        ["hiddenDock", "hidden", undefined],
        ["notifications", "conditional", "brighten"],
        ["conversation", "conditional", "open"],
      ],
    };

    for (const mode of VIEW_MODES) {
      for (const [element, presence, trigger] of expected[mode]) {
        const cell = elementResponsibility(element, mode);
        expect(cell.presence, `${mode}.${element}.presence`).toBe(presence);
        expect(cell.trigger, `${mode}.${element}.trigger`).toBe(trigger);
      }
    }
  });

  it("keeps the Core present in every mode (never hidden)", () => {
    for (const mode of VIEW_MODES) {
      expect(elementResponsibility("core", mode).presence).not.toBe("hidden");
    }
  });

  it("hides chips / ACS / orbit outside Immersive & Standard (guardrail: Mini/Companion curate)", () => {
    for (const element of ["chips", "acs", "orbit"] as const) {
      expect(isElementVisibleAtRest(element, "immersive")).toBe(true);
      expect(isElementVisibleAtRest(element, "standard")).toBe(true);
      expect(isElementVisibleAtRest(element, "mini")).toBe(false);
      expect(isElementVisibleAtRest(element, "companion")).toBe(false);
    }
  });

  it("makes the Companion surface the ember only (Room none; Composer/conversation on trigger)", () => {
    expect(elementResponsibility("room", "companion").presence).toBe("hidden");
    expect(elementResponsibility("composer", "companion").presence).toBe("conditional");
    expect(elementResponsibility("conversation", "companion").presence).toBe("conditional");
  });

  it("declares the four §29 persistent-state keys, aligned with HomeSharedContext", () => {
    expect([...PERSISTENT_SHARED_STATE]).toEqual([
      "thread",
      "coreState",
      "draft",
      "focusSubject",
    ]);
    // The declared keys must have a 1:1 counterpart in the live shared context.
    const ctxKeys = Object.keys(homeStore.sharedContext()).sort();
    expect(ctxKeys).toEqual(["coreState", "draft", "focusSubjectId", "threadId"]);
  });
});

// ─── 2. Property: the matrix is total and well-formed ──────────────────────────

describe("§29 responsibility matrix — totality (property-based)", () => {
  it("defines a well-formed responsibility for EVERY mode × EVERY element", () => {
    fc.assert(
      fc.property(
        fc.constantFrom(...VIEW_MODES),
        fc.constantFrom(...HOME_ELEMENTS),
        (mode, element) => {
          const cell = elementResponsibility(element, mode);
          // Presence is one of the three known kinds.
          expect(["shown", "hidden", "conditional"]).toContain(cell.presence);
          // Every cell carries a non-empty §29 detail phrase.
          expect(cell.detail.length).toBeGreaterThan(0);
          // trigger present IFF conditional (coupling invariant).
          if (cell.presence === "conditional") {
            expect(cell.trigger).toBeTruthy();
          } else {
            expect(cell.trigger).toBeUndefined();
          }
          // `isElementVisibleAtRest` is exactly `presence === "shown"`.
          expect(isElementVisibleAtRest(element, mode)).toBe(cell.presence === "shown");
        },
      ),
      { numRuns: 200 },
    );
  });

  it("has no missing cells: matrix key sets equal the canonical axes", () => {
    expect(Object.keys(VIEW_MODE_RESPONSIBILITY_MATRIX).sort()).toEqual([...VIEW_MODES].sort());
    for (const mode of VIEW_MODES) {
      expect(Object.keys(VIEW_MODE_RESPONSIBILITY_MATRIX[mode]).sort()).toEqual(
        [...HOME_ELEMENTS].sort(),
      );
    }
  });
});

// ─── 3. Property: shared state persists across ALL mode-pair transitions ───────

function resetHome(): void {
  settlePendingModeTransition();
  homeStore.reset();
  coreStore.reset();
  converseStore.setActiveThread(null);
  converseStore.updateDraft({ text: "", attachments: [] });
  shellStore.setWindowMode("standard");
  syncViewModeFromShell();
}

describe("§29 persistence — homeStore machine preserves shared context across every mode pair", () => {
  afterEach(() => resetHome());

  it("preserves thread/Core-state/draft/Focus-subject for ANY from→to mode + ANY context", () => {
    fc.assert(
      fc.property(
        fc.constantFrom(...VIEW_MODES),
        fc.constantFrom(...VIEW_MODES),
        fc.record({
          threadId: fc.option(fc.string(), { nil: null }),
          coreState: fc.option(fc.constantFrom(...CORE_STATES), { nil: null }),
          draft: fc.string(),
          focusSubjectId: fc.option(fc.string(), { nil: null }),
        }),
        (from, to, ctx: HomeSharedContext) => {
          resetHome();
          // Seed the declared persistent state, then start from `from` at rest.
          homeStore.setViewMode(from);
          homeStore.updateSharedContext(ctx);
          expect(homeStore.state()).toBe("rest");

          // Drive the continuous transition through the machine every mode uses.
          homeStore.beginModeTransition(to);
          homeStore.completeModeTransition();

          // The view mode landed on the target …
          expect(homeStore.viewMode()).toBe(to);
          // … Companion resolves to the companion macro state, others rest.
          expect(homeStore.state()).toBe(to === "companion" ? "companion" : "rest");
          // … and EVERY §29 persistent field survived unchanged (by construction).
          expect(homeStore.sharedContext()).toEqual(ctx);
        },
      ),
      { numRuns: 200 },
    );
  });
});

describe("§29 persistence — the real coordinator funnel preserves live state across every mode pair", () => {
  beforeEach(() => resetHome());
  afterEach(() => {
    resetHome();
    document.documentElement.removeAttribute("data-reduced-motion");
  });

  it("preserves the live thread / draft / Core-state snapshot across ANY from→to switch", () => {
    fc.assert(
      fc.property(
        fc.constantFrom(...VIEW_MODES),
        fc.constantFrom(...VIEW_MODES),
        fc.string(),
        fc.constantFrom(...CORE_STATES),
        (from, to, draftText, coreState) => {
          resetHome();
          // Seed live domain stores (the sources the coordinator snapshots).
          converseStore.setActiveThread("thread-8-7");
          converseStore.updateDraft({ text: draftText, attachments: [] });
          coreStore.setState(coreState);
          shellStore.setWindowMode(from);
          syncViewModeFromShell();

          // Continuous, reduced-motion instant switch through the real funnel.
          requestWindowMode(to, { reducedMotion: true });

          // Native + view mode aligned on the effective mode.
          expect(shellStore.windowMode()).toBe(to);
          expect(homeStore.viewMode()).toBe(to);
          // No leaked transient overlay.
          expect(homeStore.state()).not.toBe("mode-transition");
          // coreStore is never written by the coordinator (authority invariant).
          expect(coreStore.state()).toBe(coreState);
          // Conversations persist (presentation-only switch, Req 13.4).
          expect(converseStore.activeThreadId()).toBe("thread-8-7");
          expect(converseStore.composerDraft().text).toBe(draftText);

          if (from !== to) {
            // The §29 persistent snapshot was captured + preserved.
            const snapshot = homeStore.sharedContext();
            expect(snapshot.threadId).toBe("thread-8-7");
            expect(snapshot.draft).toBe(draftText);
            expect(snapshot.coreState).toBe(coreState);
          }
        },
      ),
      { numRuns: 120 },
    );
  });
});
