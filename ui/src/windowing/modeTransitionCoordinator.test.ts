import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import fc from "fast-check";
import {
  MODE_TRANSITION_MS,
  captureSharedContext,
  requestWindowMode,
  settlePendingModeTransition,
  syncViewModeFromShell,
} from "./modeTransitionCoordinator";
import { shellStore, type WindowMode } from "../stores/shellStore";
import { homeStore } from "../stores/homeStore";
import { coreStore, type CoreState } from "../stores/coreStore";
import { converseStore } from "../stores/converseStore";

/**
 * Task 8.2 — Continuous mode transitions with the Core as continuity anchor and
 * shared-state preservation (thread / Core state / draft / Focus subject),
 * reachable by keyboard/palette/AT, with a reduced-motion instant-switch
 * degrade that still preserves state.
 *
 * The coordinator is the single funnel every trigger uses, so proving its
 * behaviour proves the trigger behaviour (palette `cmd.mode.*`, WindowModeSwitch
 * buttons, and the Immersive→Standard Escape path all call `requestWindowMode`).
 *
 * Validates: Requirements 13.2, 13.3, 13.5
 */

const MODES: readonly WindowMode[] = ["immersive", "standard", "mini", "companion"];

function resetAll(): void {
  settlePendingModeTransition();
  homeStore.reset();
  coreStore.reset();
  converseStore.setActiveThread(null);
  converseStore.updateDraft({ text: "", attachments: [] });
  shellStore.setWindowMode("standard");
  syncViewModeFromShell();
}

describe("modeTransitionCoordinator — continuous transitions (Req 13.2/13.3/13.5)", () => {
  beforeEach(() => {
    resetAll();
  });

  afterEach(() => {
    resetAll();
    document.documentElement.removeAttribute("data-reduced-motion");
  });

  it("captures live shared state and preserves it across an instant (reduced-motion) switch", () => {
    converseStore.setActiveThread("thread-x");
    converseStore.updateDraft({ text: "half-written question", attachments: [] });
    coreStore.setState("thinking");

    expect(requestWindowMode("mini", { reducedMotion: true })).toBe(true);

    // Native presentation + view mode both landed on the target …
    expect(shellStore.windowMode()).toBe("mini");
    expect(homeStore.viewMode()).toBe("mini");
    // … the transient overlay settled back to the pre-transition stable state.
    expect(homeStore.state()).toBe("rest");

    // Shared context captured the live values BEFORE the switch (Req 13.3).
    expect(homeStore.sharedContext()).toEqual({
      threadId: "thread-x",
      coreState: "thinking",
      draft: "half-written question",
      focusSubjectId: null,
    });
    // coreStore is never written by the coordinator — only snapshotted.
    expect(coreStore.state()).toBe("thinking");
  });

  it("makes the Core the continuity anchor during the transition (timer path)", () => {
    vi.useFakeTimers();
    try {
      coreStore.setState("listening");

      expect(requestWindowMode("immersive", { reducedMotion: false, durationMs: MODE_TRANSITION_MS })).toBe(true);

      // Mid-transition: Core is the focus anchor, native mode already applied,
      // but the staged view mode is NOT yet committed (continuous, not a reload).
      expect(homeStore.state()).toBe("mode-transition");
      expect(homeStore.focusTarget()).toBe("core");
      expect(homeStore.viewMode()).toBe("standard");
      expect(shellStore.windowMode()).toBe("immersive");
      expect(coreStore.state()).toBe("listening"); // untouched → visually continuous

      vi.advanceTimersByTime(MODE_TRANSITION_MS);

      // Settled: staged view mode committed, stable macro state restored.
      expect(homeStore.viewMode()).toBe("immersive");
      expect(homeStore.state()).toBe("rest");
      expect(coreStore.state()).toBe("listening");
    } finally {
      vi.useRealTimers();
    }
  });

  it("degrades to an instant switch under reduced-motion (settles synchronously)", () => {
    expect(requestWindowMode("mini", { reducedMotion: true })).toBe(true);
    // No dwell: the transient overlay is already gone and the view mode committed
    // in the same tick — an instant switch, not an animated one.
    expect(homeStore.state()).not.toBe("mode-transition");
    expect(homeStore.state()).toBe("rest");
    expect(homeStore.viewMode()).toBe("mini");
  });

  it("honors the global reduced-motion root stamp as an instant switch", () => {
    document.documentElement.setAttribute("data-reduced-motion", "on");
    try {
      expect(requestWindowMode("immersive")).toBe(true);
      // Probed reduced-motion from the root stamp → synchronous settle.
      expect(homeStore.state()).not.toBe("mode-transition");
      expect(homeStore.viewMode()).toBe("immersive");
    } finally {
      document.documentElement.removeAttribute("data-reduced-motion");
    }
  });

  it("is a no-op when already in the target mode", () => {
    expect(shellStore.windowMode()).toBe("standard");
    expect(requestWindowMode("standard")).toBe(false);
    expect(homeStore.state()).toBe("rest");
  });

  it("resolves into the companion macro state when switching to Companion", () => {
    expect(requestWindowMode("companion", { reducedMotion: true })).toBe(true);
    expect(shellStore.windowMode()).toBe("companion");
    expect(homeStore.viewMode()).toBe("companion");
    expect(homeStore.state()).toBe("companion");
    expect(homeStore.companion().active).toBe(true);
  });

  it("persists the active conversation + draft across a switch (presentation-only)", () => {
    converseStore.setActiveThread("thread-keep");
    converseStore.updateDraft({ text: "keep me", attachments: [] });

    requestWindowMode("mini", { reducedMotion: true });
    requestWindowMode("immersive", { reducedMotion: true });

    // The domain store is never reset by a mode switch (Req 13.4).
    expect(converseStore.activeThreadId()).toBe("thread-keep");
    expect(converseStore.composerDraft().text).toBe("keep me");
  });

  it("completes an in-flight transition before starting a superseding one", () => {
    vi.useFakeTimers();
    try {
      requestWindowMode("mini", { reducedMotion: false, durationMs: MODE_TRANSITION_MS });
      expect(homeStore.state()).toBe("mode-transition");

      // A second request arrives before the first settles: it settles the first
      // (commits Mini) then runs continuously to Immersive.
      requestWindowMode("immersive", { reducedMotion: true });

      expect(shellStore.windowMode()).toBe("immersive");
      expect(homeStore.viewMode()).toBe("immersive");
      expect(homeStore.state()).toBe("rest");
    } finally {
      vi.useRealTimers();
    }
  });

  it("captureSharedContext snapshots live values without touching coreStore", () => {
    converseStore.setActiveThread("thread-cap");
    converseStore.updateDraft({ text: "cap", attachments: [] });
    coreStore.setState("acting");

    captureSharedContext();

    expect(homeStore.sharedContext()).toEqual({
      threadId: "thread-cap",
      coreState: "acting",
      draft: "cap",
      focusSubjectId: null,
    });
    expect(coreStore.state()).toBe("acting");
  });

  it("syncViewModeFromShell aligns homeStore.viewMode with the shell mode", () => {
    // A bare shell mode change (e.g. restored from storage) leaves viewMode stale…
    shellStore.setWindowMode("mini");
    homeStore.setViewMode("standard");
    expect(homeStore.viewMode()).toBe("standard");

    syncViewModeFromShell();
    expect(homeStore.viewMode()).toBe("mini");
  });
});

/**
 * Property: for ANY start mode and ANY sequence of target modes, a run of
 * continuous transitions preserves the shared state captured at the first real
 * switch (thread / draft / Core-state snapshot), never mutates `coreStore`, and
 * always lands with `shellStore.windowMode() === homeStore.viewMode()`.
 *
 * Validates: Requirements 13.2, 13.3, 13.5
 */
describe("modeTransitionCoordinator — preservation invariants (property-based)", () => {
  afterEach(() => {
    resetAll();
    document.documentElement.removeAttribute("data-reduced-motion");
  });

  it("preserves shared state + Core across arbitrary transition sequences", () => {
    fc.assert(
      fc.property(
        fc.constantFrom(...MODES),
        fc.array(fc.constantFrom(...MODES), { minLength: 1, maxLength: 6 }),
        fc.string(),
        fc.constantFrom<CoreState>("idle", "thinking", "blocked", "listening", "acting"),
        (startMode, sequence, draftText, coreState) => {
          resetAll();
          converseStore.setActiveThread("pbt-thread");
          converseStore.updateDraft({ text: draftText, attachments: [] });
          coreStore.reset();
          coreStore.setState(coreState);
          shellStore.setWindowMode(startMode);
          syncViewModeFromShell();

          // Simulate the effective final mode + whether any real switch occurs.
          let current = startMode;
          let switched = false;
          for (const target of sequence) {
            if (target !== current) {
              switched = true;
              current = target;
            }
          }

          for (const target of sequence) {
            requestWindowMode(target, { reducedMotion: true });
          }

          // Native mode and view mode always end aligned on the effective mode.
          expect(shellStore.windowMode()).toBe(current);
          expect(homeStore.viewMode()).toBe(current);
          // The coordinator NEVER writes coreStore (authority invariant).
          expect(coreStore.state()).toBe(coreState);
          // No timers leak under the reduced-motion instant path.
          expect(homeStore.state()).not.toBe("mode-transition");

          if (switched) {
            // Shared state captured at the first switch is preserved unchanged.
            const ctx = homeStore.sharedContext();
            expect(ctx.threadId).toBe("pbt-thread");
            expect(ctx.draft).toBe(draftText);
            expect(ctx.coreState).toBe(coreState);
          }
        },
      ),
      { numRuns: 100 },
    );
  });
});
