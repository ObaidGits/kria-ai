/**
 * Core Tray tests.
 *
 * Verifies (task 2.3):
 *  - Core state → tray bucket mapping (idle / working / needs-attention / error)
 *  - the subscription pushes the bucket to the tray on Core state change
 *  - rapid changes coalesce into a single trailing push (no tray spam)
 *  - unchanged buckets are de-duplicated (not re-pushed)
 *  - an unavailable / throwing tray push degrades silently (never crashes)
 *
 * Requirements: 3.4, 18.2
 */
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import {
  coreStateToBucket,
  initCoreTray,
  disposeCoreTray,
  DEFAULT_TRAY_THROTTLE_MS,
  type TrayBucket,
} from "./coreTray";
import { coreStore, type CoreState } from "./coreStore";
import { eventBus } from "./eventBus";

describe("coreStateToBucket", () => {
  it("maps error/recovering → error", () => {
    expect(coreStateToBucket("error")).toBe("error");
    expect(coreStateToBucket("recovering")).toBe("error");
  });

  it("maps blocked/waiting → needs-attention", () => {
    expect(coreStateToBucket("blocked")).toBe("needs-attention");
    expect(coreStateToBucket("waiting")).toBe("needs-attention");
  });

  it("maps active work states → working", () => {
    const working: CoreState[] = [
      "listening",
      "thinking",
      "planning",
      "speaking",
      "acting",
      "running-automation",
      "watching",
      "remembering",
      "reflecting",
      "learning",
    ];
    for (const s of working) {
      expect(coreStateToBucket(s)).toBe("working");
    }
  });

  it("maps idle → idle", () => {
    expect(coreStateToBucket("idle")).toBe("idle");
  });
});

describe("initCoreTray", () => {
  let pushed: TrayBucket[];
  const push = (b: TrayBucket) => pushed.push(b);

  beforeEach(() => {
    vi.useFakeTimers();
    coreStore.reset();
    disposeCoreTray();
    pushed = [];
  });

  afterEach(() => {
    disposeCoreTray();
    eventBus.clear();
    vi.useRealTimers();
  });

  it("pushes the current (idle) state once on init", () => {
    initCoreTray({ push });
    expect(pushed).toEqual([]); // trailing — nothing yet
    vi.advanceTimersByTime(DEFAULT_TRAY_THROTTLE_MS);
    expect(pushed).toEqual(["idle"]);
  });

  it("pushes the new bucket when Core state changes", () => {
    initCoreTray({ push });
    vi.advanceTimersByTime(DEFAULT_TRAY_THROTTLE_MS); // flush initial idle

    coreStore.setState("thinking");
    vi.advanceTimersByTime(DEFAULT_TRAY_THROTTLE_MS);
    expect(pushed).toEqual(["idle", "working"]);

    coreStore.setBlocked("needs approval");
    vi.advanceTimersByTime(DEFAULT_TRAY_THROTTLE_MS);
    expect(pushed).toEqual(["idle", "working", "needs-attention"]);
  });

  it("coalesces a burst of rapid changes into one trailing push", () => {
    initCoreTray({ push });
    vi.advanceTimersByTime(DEFAULT_TRAY_THROTTLE_MS); // flush initial idle
    pushed.length = 0;

    // Rapid changes inside one throttle window — only the latest bucket sends.
    coreStore.setState("thinking"); // working
    coreStore.setState("planning"); // working
    coreStore.setState("blocked"); // needs-attention (final)
    expect(pushed).toEqual([]); // nothing sent mid-window

    vi.advanceTimersByTime(DEFAULT_TRAY_THROTTLE_MS);
    expect(pushed).toEqual(["needs-attention"]);
  });

  it("de-duplicates unchanged buckets across windows", () => {
    initCoreTray({ push });
    vi.advanceTimersByTime(DEFAULT_TRAY_THROTTLE_MS); // idle
    pushed.length = 0;

    // thinking → acting both map to "working": only one push total.
    coreStore.setState("thinking");
    vi.advanceTimersByTime(DEFAULT_TRAY_THROTTLE_MS);
    coreStore.setState("acting");
    vi.advanceTimersByTime(DEFAULT_TRAY_THROTTLE_MS);

    expect(pushed).toEqual(["working"]);
  });

  it("is idempotent — a second init does not double-subscribe", () => {
    initCoreTray({ push });
    initCoreTray({ push });
    vi.advanceTimersByTime(DEFAULT_TRAY_THROTTLE_MS);
    pushed.length = 0;

    coreStore.setState("thinking");
    vi.advanceTimersByTime(DEFAULT_TRAY_THROTTLE_MS);
    expect(pushed).toEqual(["working"]); // exactly one, not two
  });

  it("stops pushing after dispose", () => {
    initCoreTray({ push });
    vi.advanceTimersByTime(DEFAULT_TRAY_THROTTLE_MS);
    pushed.length = 0;

    disposeCoreTray();
    coreStore.setState("thinking");
    vi.advanceTimersByTime(DEFAULT_TRAY_THROTTLE_MS);
    expect(pushed).toEqual([]);
  });

  it("degrades silently when the push throws (tray unavailable)", () => {
    const throwingPush = () => {
      throw new Error("no tray on this DE");
    };
    initCoreTray({ push: throwingPush });

    // Flushing the pending push must not throw despite the failing delivery.
    expect(() => vi.advanceTimersByTime(DEFAULT_TRAY_THROTTLE_MS)).not.toThrow();

    // Subsequent state changes also stay silent.
    coreStore.setState("error");
    expect(() => vi.advanceTimersByTime(DEFAULT_TRAY_THROTTLE_MS)).not.toThrow();
  });
});
