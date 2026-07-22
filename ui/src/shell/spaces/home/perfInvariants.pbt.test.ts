/**
 * Performance-invariant property tests (task 10.2 — perf validation gate).
 *
 * These strengthen the Requirement 20 acceptance-criteria coverage with
 * fast-check (pinned 3.23.2) property tests over randomized inputs. They do NOT
 * introduce new design correctness properties (design.md defines P1–P8, all
 * Focus-engine); they exercise the *already-specified* Req 20 behaviors —
 * idle-quiet ≤1 write/frame, paused-on-blur, and deterministic 2D auto-degrade —
 * across the full input space rather than a handful of examples.
 *
 * Invariants under test:
 *   • INV-1 (Req 17.2 / 20.1) — the shared-light publisher flushes AT MOST ONCE
 *     per animation frame for ANY burst of Core-state changes, then goes
 *     idle-quiet (schedules no further frames once stable).
 *   • INV-2 (Req 17.3 / 20.1) — publication is ALWAYS paused on blur (no flush
 *     for any state sequence while blurred) and resumes with exactly one flush
 *     carrying the latest state on focus.
 *   • INV-3 (Req 20.3 / 20.4) — the render-mode resolver collects degrade
 *     triggers in the one documented, stable order for ANY trigger combination,
 *     and `auto` degrades to the first-class 2D path iff any trigger is active
 *     (2D auto-degrade verified).
 *   • INV-4 (Req 20.3) — an EXPLICIT 3D preference is still overridden to 2D by
 *     any hard trigger (reduced-motion / no-WebGL / low-power / frame-drop) for
 *     any combination; only a lone failed perf gate is bypassable by request.
 *
 * Validates: Requirements 20.1, 20.3, 20.4
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import fc from "fast-check";
import { createRoot } from "solid-js";
import { coreStore, type CoreState } from "../../../stores/coreStore";
import { createSharedLightPublisher, presenceIntent } from "./sharedLight";
import {
  activeCoreDegradeTriggers,
  resolveCoreRenderMode,
  type CoreDegradeTrigger,
  type CoreRenderInputs,
} from "../../../platform/coreRenderMode";
import type { CapabilitySnapshot } from "../../../platform/capabilities";

/** The full §4.1 Core-state set the publisher maps. */
const CORE_STATES: CoreState[] = [
  "idle", "listening", "thinking", "planning", "speaking", "responding",
  "acting", "running-automation", "watching", "remembering", "reflecting",
  "learning", "waiting", "blocked", "error", "recovering",
];

/** The degrade-trigger collection order documented in coreRenderMode.ts. */
const DOCUMENTED_TRIGGER_ORDER: CoreDegradeTrigger[] = [
  "reduced-motion", "no-webgl", "low-power", "failed-gate", "frame-drop",
];

/** A controllable rAF: callbacks queue until `flushFrame()` runs them. */
function makeFakeRaf() {
  let nextHandle = 1;
  const queue = new Map<number, FrameRequestCallback>();
  return {
    requestFrame: (cb: FrameRequestCallback): number => {
      const handle = nextHandle++;
      queue.set(handle, cb);
      return handle;
    },
    cancelFrame: (handle: number): void => {
      queue.delete(handle);
    },
    flushFrame(): number {
      const callbacks = [...queue.values()];
      queue.clear();
      for (const cb of callbacks) cb(performance.now());
      return callbacks.length;
    },
    pending(): number {
      return queue.size;
    },
  };
}

/** A window stub carrying only blur/focus listeners (rAF injected separately). */
function makeFakeWindow() {
  const listeners = new Map<string, Set<EventListener>>();
  const win = {
    addEventListener(type: string, cb: EventListener) {
      (listeners.get(type) ?? listeners.set(type, new Set()).get(type)!).add(cb);
    },
    removeEventListener(type: string, cb: EventListener) {
      listeners.get(type)?.delete(cb);
    },
  } as unknown as Window;
  return {
    win,
    dispatch: (type: string) => {
      for (const cb of listeners.get(type) ?? []) cb(new Event(type));
    },
  };
}

function mountPublisher() {
  const raf = makeFakeRaf();
  const fake = makeFakeWindow();
  const target = document.createElement("div");
  let flushes = 0;
  let dispose = () => {};
  createRoot((d) => {
    dispose = d;
    createSharedLightPublisher({
      target,
      win: fake.win,
      requestFrame: raf.requestFrame,
      cancelFrame: raf.cancelFrame,
      onFlush: () => {
        flushes += 1;
      },
    });
  });
  return { raf, fake, target, dispose, flushCount: () => flushes };
}

/** Build a valid capability snapshot from the two gate-relevant flags. */
function snapshot(reducedMotion: boolean, hasWebGL: boolean): CapabilitySnapshot {
  return {
    webglTier: hasWebGL ? "webgl2" : "none",
    hasWebGL,
    prefersReducedMotion: reducedMotion,
    supportsBackdropFilter: hasWebGL,
    probe: null,
  };
}

const coreStateArb = fc.constantFrom(...CORE_STATES);

beforeEach(() => {
  // Randomized state bursts intentionally cross "unusual" coreStore transitions
  // (setState still applies them — that is what we assert). Mute the advisory
  // transition warning so the property runs stay quiet without weakening it.
  vi.spyOn(console, "warn").mockImplementation(() => {});
});

afterEach(() => {
  coreStore.reset();
  presenceIntent.reset();
  vi.restoreAllMocks();
});

describe("INV-1: publisher flushes ≤1/frame then goes idle-quiet (Req 17.2/20.1)", () => {
  it("collapses ANY burst of Core-state changes into a single flush per frame", () => {
    fc.assert(
      fc.property(fc.array(coreStateArb, { minLength: 1, maxLength: 12 }), (states) => {
        coreStore.reset();
        const { raf, target, dispose, flushCount } = mountPublisher();
        try {
          // Mount scheduled exactly one frame; no synchronous write yet.
          expect(raf.pending()).toBe(1);

          // Apply the whole burst before the frame runs.
          for (const s of states) coreStore.setState(s);
          // Still coalesced to at most one pending frame.
          expect(raf.pending()).toBe(1);

          // One frame → exactly one flush carrying the LATEST state.
          const ran = raf.flushFrame();
          expect(ran).toBe(1);
          expect(flushCount()).toBe(1);
          const last = states[states.length - 1];
          expect(target.style.getPropertyValue("--core-hue")).toBe(`var(--presence-${last})`);

          // Idle-quiet: stable state schedules no further frames (no rAF loop).
          expect(raf.pending()).toBe(0);
          expect(raf.flushFrame()).toBe(0);
          expect(raf.pending()).toBe(0);
        } finally {
          dispose();
          coreStore.reset();
        }
      }),
    );
  });
});

describe("INV-2: publication always pauses on blur, resumes once on focus (Req 17.3/20.1)", () => {
  it("writes nothing while blurred and flushes exactly the latest state on focus", () => {
    fc.assert(
      fc.property(fc.array(coreStateArb, { minLength: 1, maxLength: 12 }), (states) => {
        coreStore.reset();
        const { raf, fake, target, dispose, flushCount } = mountPublisher();
        try {
          raf.flushFrame(); // initial idle flush
          expect(flushCount()).toBe(1);

          // Blur → paused. No state change may schedule or write.
          fake.dispatch("blur");
          for (const s of states) coreStore.setState(s);
          expect(raf.pending()).toBe(0);
          expect(raf.flushFrame()).toBe(0);
          expect(flushCount()).toBe(1); // still just the pre-blur flush

          // Focus → resume + flush the latest state exactly once.
          fake.dispatch("focus");
          expect(raf.pending()).toBe(1);
          raf.flushFrame();
          expect(flushCount()).toBe(2);
          const last = states[states.length - 1];
          expect(target.style.getPropertyValue("--core-hue")).toBe(`var(--presence-${last})`);
        } finally {
          dispose();
          coreStore.reset();
        }
      }),
    );
  });
});

describe("INV-3: degrade triggers collect in documented order; auto→2D iff any (Req 20.3/20.4)", () => {
  it("orders triggers stably and auto-degrades to the 2D path for any combination", () => {
    fc.assert(
      fc.property(
        fc.record({
          reducedMotion: fc.boolean(),
          hasWebGL: fc.boolean(),
          lowPower: fc.boolean(),
          gatePassed: fc.boolean(),
          frameDrop: fc.boolean(),
        }),
        (flags) => {
          const inputs: CoreRenderInputs = {
            preference: "auto",
            snapshot: snapshot(flags.reducedMotion, flags.hasWebGL),
            gatePassed: flags.gatePassed,
            lowPower: flags.lowPower,
            frameDrop: flags.frameDrop,
          };

          const expected = DOCUMENTED_TRIGGER_ORDER.filter((t) => {
            switch (t) {
              case "reduced-motion": return flags.reducedMotion;
              case "no-webgl": return !flags.hasWebGL;
              case "low-power": return flags.lowPower;
              case "failed-gate": return !flags.gatePassed;
              case "frame-drop": return flags.frameDrop;
            }
          });

          const triggers = activeCoreDegradeTriggers(inputs);
          // Exact documented order (not just a set) — deterministic diagnostics.
          expect(triggers).toEqual(expected);

          const decision = resolveCoreRenderMode(inputs);
          // 2D auto-degrade: any active trigger forces the first-class 2D path.
          if (expected.length > 0) {
            expect(decision.mode).toBe("2d");
            expect(decision.enable3D).toBe(false);
            expect(decision.degraded).toBe(true);
          } else {
            expect(decision.mode).toBe("3d");
            expect(decision.enable3D).toBe(true);
            expect(decision.degraded).toBe(false);
          }
        },
      ),
    );
  });
});

describe("INV-4: explicit 3D still forced to 2D by any hard trigger (Req 20.3)", () => {
  it("hard triggers override an explicit 3D request for any combination", () => {
    const HARD: CoreDegradeTrigger[] = ["reduced-motion", "no-webgl", "low-power", "frame-drop"];
    fc.assert(
      fc.property(
        fc.record({
          reducedMotion: fc.boolean(),
          hasWebGL: fc.boolean(),
          lowPower: fc.boolean(),
          gatePassed: fc.boolean(),
          frameDrop: fc.boolean(),
        }),
        (flags) => {
          const inputs: CoreRenderInputs = {
            preference: "3d",
            snapshot: snapshot(flags.reducedMotion, flags.hasWebGL),
            gatePassed: flags.gatePassed,
            lowPower: flags.lowPower,
            frameDrop: flags.frameDrop,
          };
          const triggers = activeCoreDegradeTriggers(inputs);
          const hasHard = triggers.some((t) => HARD.includes(t));
          const decision = resolveCoreRenderMode(inputs);

          if (hasHard) {
            // Accessibility/budget-critical triggers always win over a request.
            expect(decision.mode).toBe("2d");
            expect(decision.enable3D).toBe(false);
          } else {
            // Only a lone failed perf gate remains — bypassable by explicit 3D.
            expect(decision.mode).toBe("3d");
            expect(decision.enable3D).toBe(true);
          }
        },
      ),
    );
  });
});
