import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { fireEvent } from "@solidjs/testing-library";
import { shellStore, type WindowMode } from "../stores/shellStore";
import { converseStore } from "../stores/converseStore";
import { currentRoute, setCurrentRoute, type Route } from "../shell/router";
import {
  activeGuiCognitionSession,
  clearGuiCognitionSession,
  handleGuiCognitionEvent,
} from "../stores/guiCognitionSession";
import { eventBus } from "../stores/eventBus";
import { disposeWindowModeManager, initWindowModeManager } from "./windowModeManager";

/**
 * Task 4.6 — Window Mode transitions are strictly presentation-only.
 *
 * Every transition (Standard↔Mini, Standard/Mini→Immersive,
 * Immersive→Standard via control and via Escape) must PRESERVE:
 *   • router route (typed `currentRoute` + `shellStore.activeSpace` mirror)
 *   • active thread (`converseStore.activeThreadId`)
 *   • Inspector target (`shellStore.inspectorTarget`)
 *   • Composer draft (`converseStore.composerDraft`)
 *   • runtime work state (`converseStore.workBlocks` + active GUI cognition session)
 *   • keyboard focus (`document.activeElement`)
 * …and it must NOT change Router_Authority. A consumed / default-prevented
 * Overlay Escape must ALSO not change Window Mode (Req 11.11 one-layer peel).
 *
 * These run on the browser/jsdom path (no Tauri): native geometry/fullscreen is
 * an enhancement, so the transition here is purely the shellStore signal change
 * plus its events. That is exactly the surface that could accidentally reset a
 * domain store, so it is the right level to prove preservation. Heavy
 * reading-place/focus-across-resize behaviour is property-tested in the Phase 1
 * converse-geometry E2E (task 3.7); this file adds a lightweight focus invariant
 * at the mode-manager level.
 *
 * Validates: Requirements 10.11, 11.11, 10.4
 */

const SEEDED_ROUTE: Route = { space: "memory", segment: "facts", entityId: "fact-42" };

interface DomainSnapshot {
  route: Route;
  activeSpace: string;
  activeThreadId: string | null;
  inspectorTarget: unknown;
  draft: unknown;
  workBlockIds: string[];
  guiTurnId: string | undefined;
  guiLifecycle: string | undefined;
}

function snapshotDomain(): DomainSnapshot {
  const session = activeGuiCognitionSession();
  return {
    route: currentRoute(),
    activeSpace: shellStore.activeSpace(),
    activeThreadId: converseStore.activeThreadId(),
    inspectorTarget: shellStore.inspectorTarget(),
    draft: converseStore.composerDraft(),
    workBlockIds: converseStore.workBlocks().map((block) => block.id),
    guiTurnId: session?.turnId,
    guiLifecycle: session?.lifecycle,
  };
}

function seedDomainState(): void {
  // Route (typed router is the authority; shellStore.activeSpace is the mirror).
  setCurrentRoute({ ...SEEDED_ROUTE });
  shellStore.setActiveSpace("memory");
  // Active thread + a live per-thread draft.
  converseStore.setActiveThread("thread-alpha");
  converseStore.updateDraft({ text: "half-written question about facts", mode: "assistant" });
  // Inspector target (single, non-stacking).
  shellStore.openInspector("memory", "fact-42", { note: "inspecting" });
  // Runtime work state: a running work block …
  converseStore.clearWorkBlocks();
  converseStore.addWorkBlock({
    id: "work-1",
    type: "reasoning",
    status: "running",
    summary: "Thinking about the answer",
    startedAt: Date.now(),
  });
  // … and an active GUI cognition session (lifecycle leaves "idle").
  clearGuiCognitionSession();
  handleGuiCognitionEvent({
    version: 1,
    turn_id: "gui-turn-1",
    workflow_id: "wf-1",
    session_id: "thread-alpha",
    sequence: 1,
    timestamp_ms: Date.now(),
    event: { type: "TurnStarted" },
    // The envelope carries many optional fields; only the identity + event
    // type matter for a preservation proof, so cast the minimal shape.
  } as Parameters<typeof handleGuiCognitionEvent>[0]);
}

function resetAll(): void {
  disposeWindowModeManager();
  shellStore.setWindowMode("standard");
  shellStore.closeInspector();
  shellStore.setActiveSpace("converse");
  setCurrentRoute({ space: "converse" });
  converseStore.clearWorkBlocks();
  converseStore.setActiveThread(null);
  clearGuiCognitionSession();
}

describe("Window Mode transitions preserve domain state (presentation-only)", () => {
  beforeEach(() => {
    resetAll();
    seedDomainState();
  });

  afterEach(() => {
    resetAll();
  });

  const transitions: Array<{ name: string; from: WindowMode; to: WindowMode }> = [
    { name: "Standard → Mini", from: "standard", to: "mini" },
    { name: "Mini → Standard", from: "mini", to: "standard" },
    { name: "Standard → Immersive", from: "standard", to: "immersive" },
    { name: "Mini → Immersive", from: "mini", to: "immersive" },
    { name: "Immersive → Standard (control)", from: "immersive", to: "standard" },
  ];

  for (const { name, from, to } of transitions) {
    it(`${name} changes only the window mode`, () => {
      shellStore.setWindowMode(from);
      const before = snapshotDomain();

      shellStore.setWindowMode(to);

      expect(shellStore.windowMode()).toBe(to);
      expect(snapshotDomain()).toEqual(before);
      // Route authority is unchanged specifically (Req 10.11 Router_Authority).
      expect(currentRoute()).toEqual(SEEDED_ROUTE);
      expect(converseStore.activeThreadId()).toBe("thread-alpha");
      expect(converseStore.composerDraft().text).toBe("half-written question about facts");
      expect(shellStore.inspectorTarget()).toEqual({ type: "memory", id: "fact-42", data: { note: "inspecting" } });
      expect(converseStore.workBlocks().map((b) => b.id)).toEqual(["work-1"]);
      expect(activeGuiCognitionSession()?.turnId).toBe("gui-turn-1");
    });
  }

  it("emits no Space/thread domain events during a mode transition", () => {
    shellStore.setWindowMode("standard");
    const domainEvents: string[] = [];
    const offSpace = eventBus.on("shell:space-changed", () => domainEvents.push("space-changed"));
    const offThread = eventBus.on("converse:thread-switched", () => domainEvents.push("thread-switched"));

    try {
      shellStore.setWindowMode("immersive");
      shellStore.setWindowMode("mini");
      shellStore.setWindowMode("standard");
    } finally {
      offSpace();
      offThread();
    }

    expect(domainEvents).toEqual([]);
  });

  it("Immersive → Standard via Escape preserves route, thread, inspector, draft, and work state", () => {
    shellStore.setWindowMode("immersive");
    initWindowModeManager();

    fireEvent.keyDown(window, { key: "Escape" });

    expect(shellStore.windowMode()).toBe("standard");
    // Domain state is untouched by the Escape-driven mode exit.
    expect(currentRoute()).toEqual(SEEDED_ROUTE);
    expect(shellStore.activeSpace()).toBe("memory");
    expect(converseStore.activeThreadId()).toBe("thread-alpha");
    expect(converseStore.composerDraft().text).toBe("half-written question about facts");
    expect(shellStore.inspectorTarget()).toEqual({ type: "memory", id: "fact-42", data: { note: "inspecting" } });
    expect(converseStore.workBlocks().map((b) => b.id)).toEqual(["work-1"]);
    expect(activeGuiCognitionSession()?.turnId).toBe("gui-turn-1");
  });

  it("preserves keyboard focus across a mode transition (mode-manager focus invariant)", () => {
    const input = document.createElement("input");
    document.body.appendChild(input);
    try {
      input.focus();
      expect(document.activeElement).toBe(input);

      shellStore.setWindowMode("mini");
      shellStore.setWindowMode("immersive");

      // The mode change never steals focus — the mode manager touches only the
      // native window, never the DOM focus ring.
      expect(document.activeElement).toBe(input);
    } finally {
      input.remove();
    }
  });
});

/**
 * Req 11.11 — Escape peels at most one layer. A consumed / default-prevented
 * Overlay Escape must NOT also change Window Mode, and must not disturb any
 * preserved domain state.
 *
 * Validates: Requirements 11.11, 10.11
 */
describe("Consumed Escape guard (one-layer peel)", () => {
  beforeEach(() => {
    resetAll();
    seedDomainState();
  });

  afterEach(() => {
    resetAll();
  });

  it("leaves Immersive intact and domain state unchanged when a higher overlay consumes Escape", () => {
    shellStore.setWindowMode("immersive");
    initWindowModeManager();
    const before = snapshotDomain();

    // A top-most layer consumes Escape first (defaultPrevented).
    const consume = (event: KeyboardEvent) => event.preventDefault();
    window.addEventListener("keydown", consume, { once: true, capture: true });

    fireEvent.keyDown(window, { key: "Escape" });

    // The one-layer peel stops at the overlay: Window Mode stays Immersive …
    expect(shellStore.windowMode()).toBe("immersive");
    // … and no domain state moved.
    expect(snapshotDomain()).toEqual(before);
  });
});
