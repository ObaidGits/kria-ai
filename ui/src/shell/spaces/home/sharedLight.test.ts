/**
 * Shared-light publisher tests (design §3.2 / §13.2, Req 1.1, 17.2, 17.5).
 *
 * Asserts the four contract points from task 1.2:
 *   1. ≤1 `--core-*` write per animation frame (a burst of state changes
 *      collapses into one flush).
 *   2. Publication is PAUSED on window blur and resumed (latest state flushed)
 *      on focus.
 *   3. Correct per-state hue mapping — `--core-hue` is a `var(--presence-<state>)`
 *      TOKEN reference (zero raw color; dark+light parity).
 *   4. The publisher NEVER writes back to `coreStore` (Req 30.3 authority).
 */
import { describe, it, expect, afterEach, vi } from "vitest";
import { createRoot } from "solid-js";
import { coreStore, type CoreState } from "../../../stores/coreStore";
import {
  createSharedLightPublisher,
  sharedLightForState,
  presenceIntent,
  SHARED_LIGHT_PROPERTIES,
} from "./sharedLight";

/** A controllable rAF: callbacks queue until `flushFrame()` runs the next one. */
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
    /** Run every currently-queued callback (one animation frame). */
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

/**
 * A minimal window stub carrying only blur/focus listeners (rAF is injected
 * separately), so tests never touch the real global window.
 */
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
  const dispatch = (type: string) => {
    for (const cb of listeners.get(type) ?? []) cb(new Event(type));
  };
  return {
    win,
    dispatch,
    count: () => [...listeners.values()].reduce((n, s) => n + s.size, 0),
    /** The distinct event types the publisher registered (for cursor-free assertions). */
    types: () => [...listeners.keys()],
  };
}

afterEach(() => {
  coreStore.reset();
  presenceIntent.reset();
  vi.restoreAllMocks();
  vi.useRealTimers();
});

describe("sharedLightForState — pure state → light mapping (Req 17.5)", () => {
  it("publishes the hue as a var(--presence-<state>) token for ALL 16 states, never a raw color", () => {
    // The full §4.1 state set — the publisher and CorePresence.css must agree on
    // every one (single source of truth = the --presence-* tokens).
    const states: CoreState[] = [
      "idle", "listening", "thinking", "planning", "speaking", "responding",
      "acting", "running-automation", "watching", "remembering", "reflecting",
      "learning", "waiting", "blocked", "error", "recovering",
    ];
    for (const state of states) {
      const hue = sharedLightForState(state).hue;
      expect(hue).toBe(`var(--presence-${state})`);
      // Zero raw color: no hex / rgb literal ever leaves the mapper.
      expect(hue).not.toMatch(/#[0-9a-f]{3,8}|rgb|hsl/i);
    }
  });

  it("keeps intensity within [0,1] for every Core state and rests idle low", () => {
    const all: CoreState[] = [
      "idle", "listening", "thinking", "planning", "speaking", "responding",
      "acting", "running-automation", "watching", "remembering", "reflecting",
      "learning", "waiting", "blocked", "error", "recovering",
    ];
    for (const state of all) {
      const { intensity } = sharedLightForState(state);
      expect(intensity).toBeGreaterThanOrEqual(0);
      expect(intensity).toBeLessThanOrEqual(1);
    }
    // Idle is calmer than active conversation (density/light axis, §4.1).
    expect(sharedLightForState("idle").intensity).toBeLessThan(
      sharedLightForState("listening").intensity,
    );
    // Blocked stays calm — it must not blaze (Req 3.3).
    expect(sharedLightForState("blocked").intensity).toBeLessThanOrEqual(
      sharedLightForState("listening").intensity,
    );
  });
});

/**
 * Mount a publisher inside a Solid root and return the handle + dispose. The
 * publisher is created inside the root (so `onCleanup`/render-effect wiring is
 * owned) but state changes are driven OUTSIDE the root callback in the test
 * body — this mirrors real usage, where each `coreStore` change lands in its
 * own tick so the render effect flushes between changes (rather than batching
 * everything into one synchronous root execution).
 */
function mountPublisher(overrides: Partial<Parameters<typeof createSharedLightPublisher>[0]> = {}) {
  const raf = makeFakeRaf();
  const fake = makeFakeWindow();
  const target = document.createElement("div");
  const onFlush = vi.fn();
  let dispose = () => {};
  createRoot((d) => {
    dispose = d;
    createSharedLightPublisher({
      target,
      win: fake.win,
      requestFrame: raf.requestFrame,
      cancelFrame: raf.cancelFrame,
      onFlush,
      ...overrides,
    });
  });
  return { raf, fake, target, onFlush, dispose };
}

describe("createSharedLightPublisher — throttled, paused-on-blur, idle-quiet", () => {
  it("writes at most one --core-* flush per animation frame across a burst (Req 17.2)", () => {
    coreStore.reset(); // idle
    const { raf, target, onFlush, dispose } = mountPublisher();

    // Initial mount scheduled exactly one frame (no synchronous write yet).
    expect(onFlush).toHaveBeenCalledTimes(0);
    expect(raf.pending()).toBe(1);

    // A burst of state changes before the frame runs…
    coreStore.setState("thinking");
    coreStore.setState("planning");
    coreStore.setState("acting");
    // …still only one pending frame (coalesced to ≤1/frame).
    expect(raf.pending()).toBe(1);

    // One frame → exactly one flush carrying the LATEST state.
    raf.flushFrame();
    expect(onFlush).toHaveBeenCalledTimes(1);
    expect(target.style.getPropertyValue("--core-hue")).toBe("var(--presence-acting)");

    dispose();
  });

  it("reactively schedules a write when the Core state changes while focused (Req 1.1)", () => {
    coreStore.reset();
    const { raf, target, onFlush, dispose } = mountPublisher();

    raf.flushFrame(); // initial idle write
    expect(onFlush).toHaveBeenCalledTimes(1);
    expect(raf.pending()).toBe(0);

    // A state change must reactively schedule exactly one new frame.
    coreStore.setState("thinking");
    expect(raf.pending()).toBe(1);
    raf.flushFrame();
    expect(onFlush).toHaveBeenCalledTimes(2);
    expect(target.style.getPropertyValue("--core-hue")).toBe("var(--presence-thinking)");

    dispose();
  });

  it("pauses publication on window blur and flushes the latest state on focus (Req 17.3/§11.5)", () => {
    coreStore.reset();
    const { raf, fake, target, onFlush, dispose } = mountPublisher();

    // Flush the initial idle frame.
    raf.flushFrame();
    expect(onFlush).toHaveBeenCalledTimes(1);
    expect(target.style.getPropertyValue("--core-hue")).toBe("var(--presence-idle)");

    // Blur → paused. State changes must NOT schedule/write while blurred.
    fake.dispatch("blur");
    coreStore.setState("thinking");
    coreStore.setState("speaking");
    expect(raf.pending()).toBe(0); // nothing scheduled while paused
    raf.flushFrame(); // no-op
    expect(onFlush).toHaveBeenCalledTimes(1); // still just the idle flush
    expect(target.style.getPropertyValue("--core-hue")).toBe("var(--presence-idle)");

    // Focus → resume and flush the latest state once.
    fake.dispatch("focus");
    expect(raf.pending()).toBe(1);
    raf.flushFrame();
    expect(onFlush).toHaveBeenCalledTimes(2);
    expect(target.style.getPropertyValue("--core-hue")).toBe("var(--presence-speaking)");

    dispose();
  });

  it("is idle-quiet: schedules no further frames once the state is stable (Req 20.1)", () => {
    coreStore.reset();
    const { raf, dispose } = mountPublisher();

    raf.flushFrame(); // initial write
    // No state change → no perpetual loop: nothing re-scheduled.
    expect(raf.pending()).toBe(0);
    expect(raf.flushFrame()).toBe(0);
    expect(raf.pending()).toBe(0);

    dispose();
  });

  it("writes all five shared-light custom properties to the target root scope", () => {
    coreStore.setState("thinking");
    const { raf, target, dispose } = mountPublisher();
    raf.flushFrame();

    for (const property of SHARED_LIGHT_PROPERTIES) {
      expect(target.style.getPropertyValue(property)).not.toBe("");
    }
    expect(target.style.getPropertyValue("--core-hue")).toBe("var(--presence-thinking)");

    dispose();
  });

  it("removes the inline --core-* overrides on cleanup (token defaults resume)", () => {
    coreStore.setState("acting");
    const { raf, target, dispose } = mountPublisher();
    raf.flushFrame();
    expect(target.style.getPropertyValue("--core-hue")).toBe("var(--presence-acting)");

    dispose();

    for (const property of SHARED_LIGHT_PROPERTIES) {
      expect(target.style.getPropertyValue(property)).toBe("");
    }
  });

  it("NEVER writes back to coreStore — publication is read-only (Req 30.3)", () => {
    // Spy every authoritative mutator; the publisher must call none of them.
    const setState = vi.spyOn(coreStore, "setState");
    const ingest = vi.spyOn(coreStore, "ingest");
    const setBlocked = vi.spyOn(coreStore, "setBlocked");
    const setError = vi.spyOn(coreStore, "setError");
    const goIdle = vi.spyOn(coreStore, "goIdle");

    const { raf, dispose } = mountPublisher();
    raf.flushFrame();
    // Drive a change through the read path too — still no write-back.
    coreStore.setState("thinking");
    raf.flushFrame();

    // The publisher itself never invokes a mutator (the test's own setState is
    // excluded — we assert the mutators were only called by the test, once).
    expect(ingest).not.toHaveBeenCalled();
    expect(setBlocked).not.toHaveBeenCalled();
    expect(setError).not.toHaveBeenCalled();
    expect(goIdle).not.toHaveBeenCalled();
    expect(setState).toHaveBeenCalledTimes(1); // only the test's own call

    dispose();
  });
});

// ─── Task 2.1: meaningful-intent behaviors (Req 2.5 / 2.6) ───────────────────

describe("meaningful-intent posture — step-forward / recede / lean (Req 2.6, §4.1)", () => {
  it("steps the Core FORWARD and warms it to the attention hue when blocked (Req 2.6)", () => {
    const light = sharedLightForState("blocked");
    // "I need you," calm — glide forward toward the user.
    expect(light.depth).toBe(1);
    // Warm to the attention hue: --presence-blocked resolves to the warning token.
    expect(light.hue).toBe("var(--presence-blocked)");
    // Calm, not blazing (Req 3.3) — verified alongside forward posture.
    expect(light.intensity).toBeLessThanOrEqual(sharedLightForState("listening").intensity);
  });

  it("RECEDES the Core in depth while a turn is active/working (§4.1)", () => {
    for (const working of ["acting", "responding", "running-automation"] as const) {
      expect(sharedLightForState(working).depth).toBe(-1);
    }
  });

  it("rests at the Core plane (depth 0) for every non-blocked, non-working state", () => {
    const resting: CoreState[] = [
      "idle", "listening", "thinking", "planning", "speaking", "watching",
      "remembering", "reflecting", "learning", "waiting", "error", "recovering",
    ];
    for (const state of resting) {
      expect(sharedLightForState(state).depth).toBe(0);
    }
  });

  it("leans toward the Composer on voice attention (listening) only, not on other states", () => {
    // Voice attention is a meaningful-intent reaction (§4.1 attention row).
    expect(sharedLightForState("listening").lean).toBeGreaterThan(0);
    // Non-voice states rest at 0 baseline and let presenceIntent drive the lean.
    for (const state of ["idle", "thinking", "acting", "blocked"] as const) {
      expect(sharedLightForState(state).lean).toBe(0);
    }
  });
});

describe("presenceIntent — composer-focus / arrival lean (Req 2.5 / 4.3)", () => {
  it("leans while the Composer is focused and settles when it blurs", () => {
    expect(presenceIntent.lean()).toBe(0);
    presenceIntent.setComposerFocused(true);
    expect(presenceIntent.lean()).toBe(1);
    presenceIntent.setComposerFocused(false);
    expect(presenceIntent.lean()).toBe(0);
  });

  it("pulses a brief arrival lean that settles back on its own (no perpetual loop)", () => {
    vi.useFakeTimers();
    presenceIntent.pulseArrival(2000);
    expect(presenceIntent.lean()).toBe(1);
    vi.advanceTimersByTime(1999);
    expect(presenceIntent.lean()).toBe(1);
    vi.advanceTimersByTime(1);
    expect(presenceIntent.lean()).toBe(0);
  });

  it("takes the stronger of composer-focus and arrival lean so they never cancel", () => {
    vi.useFakeTimers();
    presenceIntent.setComposerFocused(true);
    presenceIntent.pulseArrival(2000);
    expect(presenceIntent.lean()).toBe(1);
    // Arrival settles, but a still-focused Composer keeps the lean.
    vi.advanceTimersByTime(2000);
    expect(presenceIntent.lean()).toBe(1);
    presenceIntent.setComposerFocused(false);
    expect(presenceIntent.lean()).toBe(0);
  });
});

describe("publisher folds meaningful-intent lean into the state baseline (Req 2.5)", () => {
  it("publishes the intent-driven lean even when the state baseline is 0 (idle + composer focus)", () => {
    coreStore.reset(); // idle → baseline lean 0
    const { raf, target, dispose } = mountPublisher({ intentLean: () => 1 });
    raf.flushFrame();
    // The old hardcoded lean=0 is now intent-driven: idle + focus leans fully.
    expect(target.style.getPropertyValue("--core-lean")).toBe("1");
    dispose();
  });

  it("keeps the stronger of state-baseline and intent lean (voice + weak intent)", () => {
    coreStore.setState("listening"); // baseline lean 0.6
    const { raf, target, dispose } = mountPublisher({ intentLean: () => 0.2 });
    raf.flushFrame();
    expect(target.style.getPropertyValue("--core-lean")).toBe("0.6");
    dispose();
  });

  it("keeps --core-depth purely state-driven, never influenced by intent lean", () => {
    coreStore.setState("blocked");
    const { raf, target, dispose } = mountPublisher({ intentLean: () => 1 });
    raf.flushFrame();
    expect(target.style.getPropertyValue("--core-depth")).toBe("1"); // step forward
    expect(target.style.getPropertyValue("--core-lean")).toBe("1"); // intent only
    dispose();
  });
});

describe("no cursor tracking — reactions fire on meaningful intent only (Req 2.5)", () => {
  it("registers ONLY blur/focus listeners — never mousemove/pointermove", () => {
    coreStore.reset();
    const { raf, fake, dispose } = mountPublisher();
    raf.flushFrame();

    const registered = fake.types();
    expect(registered).toContain("blur");
    expect(registered).toContain("focus");
    expect(registered).not.toContain("mousemove");
    expect(registered).not.toContain("pointermove");
    expect(registered).not.toContain("mouseover");
    // Exactly the two lifecycle listeners — nothing cursor-driven.
    expect(new Set(registered)).toEqual(new Set(["blur", "focus"]));

    dispose();
  });

  it("exposes no cursor API on presenceIntent (only meaningful-intent entry points)", () => {
    expect(new Set(Object.keys(presenceIntent))).toEqual(
      new Set(["lean", "setComposerFocused", "pulseArrival", "reset"]),
    );
  });
});
