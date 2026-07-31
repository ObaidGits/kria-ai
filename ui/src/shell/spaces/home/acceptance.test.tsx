/**
 * Homepage Presence Redesign — ACCEPTANCE SUITE (task 10.3, design.md §19).
 *
 * This file is the COHESIVE, auditable acceptance layer for the redesign. It
 * does two jobs:
 *
 *   1. It ADDS the end-to-end (E2E) acceptance flows §19 calls for that were not
 *      yet wired together across layers:
 *        • send → Reading Mode        (first send recedes the homepage; empty reverses)
 *        • mode transitions preserve state (thread / draft / Core snapshot across
 *          the full Immersive/Standard/Mini/Companion axis)
 *        • companion return           (continuous return restores the prior mode)
 *      plus a Focus-engine acceptance tie-in (deterministic resting output +
 *      single-subject Voice-Line/ACS binding) exercised through the SHIPPED
 *      `deriveFocusFrame` read-model.
 *
 *   2. It records the §19 acceptance-criterion → owning-test MAPPING so coverage
 *      is auditable in one place. Most §19 criteria are already satisfied by the
 *      per-phase suites shipped in Phases 1–9; those are REFERENCED here rather
 *      than duplicated. The full table also lives in
 *      `.kiro/specs/homepage-presence-redesign/acceptance-criteria-map.md`.
 *
 * ── §19 Acceptance-criterion → test mapping ──────────────────────────────────
 *  R1  Room ................. Room.test.tsx, roomUndertone.test.ts, sharedLight.test.ts,
 *                             HomeSpace.test.tsx (resting calm + reduced-motion)
 *  R2  Core ................. CorePresence.test.tsx, coreStore.test.ts, coreNarration.test.ts,
 *                             CoreShell3D*.test.tsx (two interactions / no-nav / per-state aria)
 *  R3  Voice Line ........... VoiceLine.test.tsx (≤1 line / dwell / once-announce / route-only)
 *  R4/5/6 Composer .......... Composer.test.tsx, Composer.stageSend.test.tsx (stage-not-send / draft / Send↔Stop)
 *  R5  Chips ................ ContextualChips.test.tsx (≤3 / stage|route only / omit when no action)
 *  R6  Orbit ................ ContextualOrbit.test.tsx (partial / routing-only / static-dot fallback)
 *  R7/14 Navigation Rail ... NavigationRail.test.tsx (canonical order / AT reach / modes)
 *  R8  ACS .................. AdaptiveContextSurface.test.tsx (single subject / ≤1 action / dissolve-no-empty-box)
 *                             + this file (Voice-Line/ACS same-subject binding via deriveFocusFrame)
 *  R9  Trust ................ TrustIndicator.test.tsx, trustIndicator.pbt.test.ts
 *  R10 Permission ........... PermissionSurface.test.tsx, permissionUx.test.ts (green/yellow/red, no modal-on-modal)
 *  R11 Reading Mode ......... readingMode.test.ts, ReadingBackdrop.test.tsx
 *                             + this file (E2E: send → recession; empty → reverse, live controller)
 *  R12 Focus engine ......... homeFocusStore.test.ts (ranking / anti-flicker / empty / pure read-model)
 *                             + this file (deterministic resting output + single-subject binding)
 *  R13 View Modes ........... modeTransitionCoordinator.test.ts, viewModeResponsibilityMatrix.test.ts,
 *                             windowModeTransitionPreservation.test.ts
 *                             + this file (E2E: full 4-mode round-trip preserves shared state)
 *  R15 Companion ............ companionEmber.test.ts, CompanionEmber.test.tsx
 *                             + this file (E2E: companion return restores prior mode)
 *  R16 Design system ........ token-lint.test.ts, design-system/*, `npm run lint:tokens`
 *  R17 Motion ............... motion.test.ts, motionBudget.test.ts, perfInvariants.pbt.test.ts
 *  R18 Modal-vs-Page ........ modalPageFramework.test.ts, modalHost.test.ts, ModalHost.test.tsx
 *  R19 Cross-page ........... navigationCoverage.test.tsx, terminology.test.ts, authority.test.ts
 *  R20 Performance .......... linuxDesktopValidation.test.ts, coreRenderMode.test.ts, renderMode.test.ts
 *  R21 Accessibility ........ homepageAccessibility.test.tsx, statusPresenceAccessibility.test.tsx
 *  R22 Migration ............ featureFlags.test.ts, migration/* (owned by task 10.4)
 *
 * E2E level decision: these acceptance flows run at the vitest + jsdom
 * integration level (real stores + real controllers/coordinator wired together),
 * NOT Playwright. A true Playwright e2e needs the Tauri host running (native
 * window geometry / fullscreen), which is unavailable in this harness; the
 * presence homepage is SolidJS-in-jsdom, so the integration level is where the
 * cross-layer wiring is actually observable. The native-window Playwright
 * scenarios are documented as CI-gated additions in the acceptance map, not
 * fabricated here.
 *
 * Validates: Requirements 11, 12, 13, 15 (acceptance), and — via the mapping
 * above — all remaining §19 criteria through their referenced suites.
 */
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { render, cleanup, waitFor } from "@solidjs/testing-library";

import { createReadingModeController } from "./readingMode";
import { rememberPriorMode, returnViewMode } from "./companionEmber";
import {
  requestWindowMode,
  settlePendingModeTransition,
  syncViewModeFromShell,
} from "../../../windowing/modeTransitionCoordinator";
import { homeStore } from "../../../stores/homeStore";
import { coreStore } from "../../../stores/coreStore";
import { converseStore, type Message } from "../../../stores/converseStore";
import { shellStore, type WindowMode } from "../../../stores/shellStore";
import {
  deriveFocusFrame,
  FOCUS_PRIORITY,
  type AwarenessSignal,
  type FocusInputs,
} from "../../../stores/homeFocusStore";

// ─── Shared reset ─────────────────────────────────────────────────────────────

function resetAll(): void {
  settlePendingModeTransition();
  cleanup();
  homeStore.reset();
  coreStore.reset();
  converseStore.clearMessages();
  converseStore.setActiveThread(null);
  converseStore.updateDraft({ text: "", attachments: [] });
  shellStore.setWindowMode("standard");
  syncViewModeFromShell();
  rememberPriorMode("standard");
}

beforeEach(resetAll);
afterEach(() => {
  resetAll();
  document.documentElement.removeAttribute("data-reduced-motion");
});

const userMessage = (id: string): Message => ({
  id,
  threadId: "acc-thread",
  role: "user",
  content: "hello",
  timestamp: Date.now(),
});

/**
 * Mounts the SHIPPED reading-mode controller inside a reactive owner so its
 * `createEffect` + `onCleanup` run exactly as they do in ConverseSpace. Returns
 * nothing visible — the observable effect is on `homeStore`.
 */
function ReadingHarness() {
  createReadingModeController();
  return <span data-testid="reading-harness" />;
}

// ═══ E2E: send → Reading Mode (R11.1 / R11.3) ═════════════════════════════════

describe("E2E — send → Reading Mode (Req 11.1/11.3)", () => {
  it("first send recedes the homepage into Reading Mode (not a page-swap)", async () => {
    converseStore.setActiveThread("acc-thread");
    render(() => <ReadingHarness />);

    // Resting before any message.
    expect(homeStore.readingMode()).toBe(false);
    expect(homeStore.state()).toBe("rest");

    // First send → the live controller wires converse → homeStore recession.
    converseStore.addMessage(userMessage("m1"));

    await waitFor(() => expect(homeStore.state()).toBe("reading"));
    expect(homeStore.readingMode()).toBe(true);
    // Depth-recession lands in the SAME space, focused on the message stream —
    // never a navigation target (Req 11.4 dominance).
    expect(homeStore.focusTarget()).toBe("message-stream");
  });

  it("reverses out of Reading Mode when the thread empties (Req 11.3)", async () => {
    converseStore.setActiveThread("acc-thread");
    render(() => <ReadingHarness />);

    converseStore.addMessage(userMessage("m1"));
    await waitFor(() => expect(homeStore.readingMode()).toBe(true));

    // Empty the thread → recession reverses back to the resting homepage.
    converseStore.clearMessages();
    await waitFor(() => expect(homeStore.state()).toBe("rest"));
    expect(homeStore.readingMode()).toBe(false);
  });
});

// ═══ E2E: mode transitions preserve state (R13.2/13.3/13.5) ═══════════════════

describe("E2E — mode transitions preserve shared state across the full axis (Req 13.2/13.3/13.5)", () => {
  const AXIS: readonly WindowMode[] = ["immersive", "standard", "mini", "companion", "standard"];

  it("a full Immersive→Standard→Mini→Companion→Standard round-trip preserves thread + draft + Core", () => {
    converseStore.setActiveThread("keep-thread");
    converseStore.updateDraft({ text: "half-written idea", attachments: [] });
    coreStore.setState("thinking");

    shellStore.setWindowMode("standard");
    syncViewModeFromShell();

    for (const target of AXIS) {
      requestWindowMode(target, { reducedMotion: true });

      // Native presentation + view mode stay aligned; the transient overlay
      // always settles under the instant (reduced-motion) path.
      expect(shellStore.windowMode()).toBe(target);
      expect(homeStore.viewMode()).toBe(target);
      expect(homeStore.state()).not.toBe("mode-transition");

      // Shared state survives EVERY switch by construction (Req 13.3) …
      expect(converseStore.activeThreadId()).toBe("keep-thread");
      expect(converseStore.composerDraft().text).toBe("half-written idea");
      // … and the coordinator NEVER writes coreStore (authority invariant, Req 30.3).
      expect(coreStore.state()).toBe("thinking");
    }

    // The snapshot captured into homeStore mirrors the live shared context.
    const ctx = homeStore.sharedContext();
    expect(ctx.threadId).toBe("keep-thread");
    expect(ctx.draft).toBe("half-written idea");
    expect(ctx.coreState).toBe("thinking");
  });

  it("switching to Companion resolves into the companion macro state (Req 13.1)", () => {
    requestWindowMode("companion", { reducedMotion: true });
    expect(homeStore.viewMode()).toBe("companion");
    expect(homeStore.state()).toBe("companion");
    expect(homeStore.companion().active).toBe(true);
  });
});

// ═══ E2E: companion return (R15.3 / R13.2) ════════════════════════════════════

describe("E2E — companion return restores the prior mode (Req 15.3/13.2)", () => {
  it("continuous return from Companion restores the pre-companion mode with state intact", () => {
    // Seed a live conversation in Immersive, then remember it as the prior mode.
    converseStore.setActiveThread("companion-thread");
    converseStore.updateDraft({ text: "resume this", attachments: [] });
    coreStore.setState("listening");
    shellStore.setWindowMode("immersive");
    syncViewModeFromShell();
    rememberPriorMode(shellStore.windowMode());

    // Enter Companion (condensed cross-application ember).
    requestWindowMode("companion", { reducedMotion: true });
    expect(homeStore.state()).toBe("companion");
    expect(homeStore.viewMode()).toBe("companion");

    // Continuous return: the ember hands back to the remembered mode via the
    // SAME path CompanionEmber uses — `requestWindowMode(returnViewMode())`.
    const back = returnViewMode();
    expect(back).toBe("immersive"); // Companion never overwrites the return target
    requestWindowMode(back, { reducedMotion: true });

    // The prior VIEW MODE + native window are restored (the observable
    // continuous-return contract, Req 15.3) and the switch has settled …
    expect(shellStore.windowMode()).toBe("immersive");
    expect(homeStore.viewMode()).toBe("immersive");
    expect(homeStore.state()).not.toBe("mode-transition");
    // … with the conversation + Core snapshot fully preserved (Req 13.3/13.4).
    expect(converseStore.activeThreadId()).toBe("companion-thread");
    expect(converseStore.composerDraft().text).toBe("resume this");
    expect(coreStore.state()).toBe("listening");
  });
});

// ═══ Focus engine acceptance tie-in (R12 / R8.4) ══════════════════════════════

describe("Focus engine acceptance — shipped read-model (Req 12/8.4)", () => {
  const resting: FocusInputs = {
    approvals: [],
    threads: [],
    activeThreadId: null,
    conversing: false,
    workflows: [],
    facts: [],
    notifications: [],
    awareness: [],
    now: 0,
  };

  it("produces a structurally valid resting frame when nothing qualifies (Req 12.5)", () => {
    const frame = deriveFocusFrame(resting);
    expect(frame.voiceLine).toBeUndefined();
    expect(frame.acs).toBeUndefined();
    expect(frame.chips).toEqual([]);
    expect(frame.orbit).toEqual([]);
  });

  it("is deterministic — identical inputs yield an identical frame (Req 12.2 anti-oscillation)", () => {
    expect(deriveFocusFrame(resting)).toEqual(deriveFocusFrame(resting));
  });

  it("binds the Voice Line and ACS to the SAME subject when both render (Req 8.4/12.3)", () => {
    const signal: AwarenessSignal = {
      id: "standup",
      capability: "desktop",
      priority: FOCUS_PRIORITY.imminentEvent,
      recency: 100,
      voiceText: "Standup in 20 minutes.",
      confidence: 1,
      sourceTrust: 1,
      acsTitle: "Standup",
      acsLine: "in 20 minutes",
    };
    const frame = deriveFocusFrame({ ...resting, awareness: [signal], now: 10_000 });

    expect(frame.voiceLine).toBeDefined();
    expect(frame.acs).toBeDefined();
    // The single-subject invariant: never two competing subjects.
    expect(frame.acs?.subjectId).toBe(frame.voiceLine?.subjectId);
  });
});
