/**
 * Room undertone tests (design §3.3, Requirements 1.4, 21.4).
 *
 * Covers the task-1.3 contract:
 *   1. Mapping — morning cools toward `--color-info-solid`; night warms toward
 *      `--color-warning-solid`; midday rests `transparent`.
 *   2. Bound — the shift is ALWAYS ≤6% and carries zero raw color (tokens +
 *      color-mix only).
 *   3. Steady-lighting DISABLES it — no write, no scheduled timer (Req 21.4).
 *   4. Idle-quiet — coarse single follow-up timer, paused on blur, cleaned up.
 */
import { describe, it, expect, afterEach, vi } from "vitest";
import { createRoot } from "solid-js";
import {
  createRoomUndertoneController,
  moodForHour,
  undertoneForHour,
  MAX_UNDERTONE_PERCENT,
  ROOM_UNDERTONE_PROPERTY,
} from "./roomUndertone";

/** Extract the mix percentage from a `color-mix(... N%, transparent)` value. */
function percentOf(value: string): number {
  const match = value.match(/([\d.]+)%/);
  return match ? Number.parseFloat(match[1]) : 0;
}

/** True when the value contains no raw hex/rgb/hsl color literal. */
function hasNoRawColor(value: string): boolean {
  return !/#[0-9a-f]{3,8}|rgb|hsl/i.test(value);
}

afterEach(() => {
  vi.restoreAllMocks();
});

describe("moodForHour — signed time-of-day mood (design §3.3)", () => {
  it("is cool (negative → info) in the morning and warm (positive → warning) at night", () => {
    expect(moodForHour(8)).toBeLessThan(0); // morning cools
    expect(moodForHour(23)).toBeGreaterThan(0); // night warms
    expect(moodForHour(2)).toBeGreaterThan(0); // deep night warms
  });

  it("rests near neutral in the afternoon", () => {
    expect(Math.abs(moodForHour(14))).toBeLessThan(0.02);
  });

  it("stays within [-1, 1] and wraps 24h for every hour", () => {
    for (let h = -5; h <= 30; h += 0.5) {
      const mood = moodForHour(h);
      expect(mood).toBeGreaterThanOrEqual(-1);
      expect(mood).toBeLessThanOrEqual(1);
    }
    expect(moodForHour(0)).toBeCloseTo(moodForHour(24), 6);
  });
});

describe("undertoneForHour — bounded, token-only CSS value (Req 1.4/16.2)", () => {
  it("mixes toward info-solid in the morning", () => {
    const value = undertoneForHour(8);
    expect(value).toContain("var(--color-info-solid)");
    expect(value).toContain("in oklab");
  });

  it("mixes toward warning-solid at night", () => {
    const value = undertoneForHour(23);
    expect(value).toContain("var(--color-warning-solid)");
  });

  it("rests fully transparent at the neutral afternoon", () => {
    expect(undertoneForHour(14)).toBe("transparent");
  });

  it("NEVER exceeds the 6% bound and NEVER emits raw color, for any hour", () => {
    for (let h = 0; h < 24; h += 0.25) {
      const value = undertoneForHour(h);
      expect(hasNoRawColor(value)).toBe(true);
      if (value !== "transparent") {
        expect(percentOf(value)).toBeLessThanOrEqual(MAX_UNDERTONE_PERCENT);
        expect(percentOf(value)).toBeGreaterThan(0);
      }
    }
  });

  it("reaches the 6% peak at the coolest morning hour", () => {
    // Mood is -1 at 08:00 → |mood| * 6% = 6%.
    expect(percentOf(undertoneForHour(8))).toBe(MAX_UNDERTONE_PERCENT);
  });
});

/** A controllable coarse-timer harness (no real timers fire in tests). */
function makeFakeTimers() {
  let next = 1;
  const queue = new Map<number, () => void>();
  return {
    setTimer: (cb: () => void): ReturnType<typeof setTimeout> => {
      const handle = next++;
      queue.set(handle, cb);
      return handle as unknown as ReturnType<typeof setTimeout>;
    },
    clearTimer: (handle: ReturnType<typeof setTimeout>): void => {
      queue.delete(handle as unknown as number);
    },
    /** Run every queued follow-up tick (one coarse cadence). */
    run(): void {
      const cbs = [...queue.values()];
      queue.clear();
      for (const cb of cbs) cb();
    },
    pending: (): number => queue.size,
  };
}

/** A minimal window stub carrying only blur/focus listeners. */
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
  return { win, dispatch, count: () => [...listeners.values()].reduce((n, s) => n + s.size, 0) };
}

/** A fixed clock at the given hour-of-day. */
const clockAt = (hour: number) => () => new Date(2024, 0, 1, hour, 0, 0);

interface MountArgs {
  hour?: number;
  steady?: boolean | (() => boolean);
}

function mountController(args: MountArgs = {}) {
  const timers = makeFakeTimers();
  const fake = makeFakeWindow();
  const target = document.createElement("div");
  const onWrite = vi.fn();
  const steady = args.steady ?? false;
  let dispose = () => {};
  createRoot((d) => {
    dispose = d;
    createRoomUndertoneController({
      target,
      win: fake.win,
      now: clockAt(args.hour ?? 8),
      setTimer: timers.setTimer,
      clearTimer: timers.clearTimer,
      isSteadyLighting: typeof steady === "function" ? steady : () => steady,
      onWrite,
    });
  });
  return { timers, fake, target, onWrite, dispose };
}

describe("createRoomUndertoneController — steady-lighting + idle-quiet (Req 21.4/20.1)", () => {
  it("writes a bounded mood undertone and schedules ONE coarse follow-up when enabled", () => {
    const { timers, target, onWrite, dispose } = mountController({ hour: 8 });

    expect(onWrite).toHaveBeenCalledTimes(1);
    const written = target.style.getPropertyValue(ROOM_UNDERTONE_PROPERTY);
    expect(written).toContain("var(--color-info-solid)");
    expect(percentOf(written)).toBeLessThanOrEqual(MAX_UNDERTONE_PERCENT);
    // Coarse cadence: exactly one follow-up timer (never a tight/perpetual loop).
    expect(timers.pending()).toBe(1);

    dispose();
  });

  it("is fully DISABLED under steady-lighting: no write, no scheduled timer (Req 21.4)", () => {
    const { timers, target, onWrite, dispose } = mountController({ hour: 8, steady: true });

    expect(onWrite).not.toHaveBeenCalled();
    expect(target.style.getPropertyValue(ROOM_UNDERTONE_PROPERTY)).toBe(""); // token default resumes
    expect(timers.pending()).toBe(0); // idle-quiet — nothing scheduled

    dispose();
  });

  it("disables live when steady-lighting turns on (clears override, cancels timer)", () => {
    let steady = false;
    const { timers, fake, target, dispose } = mountController({
      hour: 23,
      steady: () => steady,
    });
    // Enabled first: warm night undertone written + one timer pending.
    expect(target.style.getPropertyValue(ROOM_UNDERTONE_PROPERTY)).toContain(
      "var(--color-warning-solid)",
    );
    expect(timers.pending()).toBe(1);

    // Preference flips on; a focus event re-reads it and disables.
    steady = true;
    fake.dispatch("focus");
    expect(target.style.getPropertyValue(ROOM_UNDERTONE_PROPERTY)).toBe("");
    expect(timers.pending()).toBe(0);

    dispose();
  });

  it("pauses on window blur and resumes on focus (idle-quiet, Req 20.1)", () => {
    const { timers, fake, onWrite, dispose } = mountController({ hour: 8 });
    expect(onWrite).toHaveBeenCalledTimes(1);
    expect(timers.pending()).toBe(1);

    fake.dispatch("blur"); // pause → cancel the coarse timer
    expect(timers.pending()).toBe(0);

    fake.dispatch("focus"); // resume → recompute + reschedule
    expect(onWrite).toHaveBeenCalledTimes(2);
    expect(timers.pending()).toBe(1);

    dispose();
  });

  it("recomputes on each coarse tick without a tight loop", () => {
    const { timers, onWrite, dispose } = mountController({ hour: 8 });
    expect(onWrite).toHaveBeenCalledTimes(1);

    timers.run(); // one coarse cadence elapses
    expect(onWrite).toHaveBeenCalledTimes(2);
    expect(timers.pending()).toBe(1); // exactly one next tick, never accumulating

    dispose();
  });

  it("removes the inline override and all listeners on cleanup", () => {
    const { timers, fake, target, dispose } = mountController({ hour: 8 });
    expect(target.style.getPropertyValue(ROOM_UNDERTONE_PROPERTY)).not.toBe("");
    expect(fake.count()).toBeGreaterThan(0);

    dispose();

    expect(target.style.getPropertyValue(ROOM_UNDERTONE_PROPERTY)).toBe("");
    expect(timers.pending()).toBe(0);
    expect(fake.count()).toBe(0);
  });
});
