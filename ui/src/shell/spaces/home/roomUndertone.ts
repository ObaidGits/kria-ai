/**
 * Room undertone — the time-of-day mood shift (design.md §3.3, Requirements
 * 1.4, 21.4).
 *
 * The Room's charcoal carries a *slow, ≤6% undertone shift* that reads as
 * atmosphere, never data (design §3.3 / L5): morning cools it toward
 * `--color-info-solid`; night warms it toward `--color-warning-solid`; midday
 * rests neutral. It is implemented as a slow interpolation of the single root
 * `--room-undertone` custom property, which `Room.css` overlays on the base
 * gradient (`linear-gradient(var(--room-undertone), var(--room-undertone))`).
 * At rest the token default is `transparent` (no shift).
 *
 * ── Mood, not data (design §3.3 / guardrails.md) ─────────────────────────────
 * The undertone NEVER encodes status. It is a function of wall-clock hour only
 * and is capped at {@link MAX_UNDERTONE_PERCENT} (6%) so it can never be
 * mistaken for a semantic info/warning surface. It is bounded to `transparent`
 * → a ≤6% `color-mix` over the info/warning TOKENS — zero raw color (Req 16.2).
 *
 * ── Idle-quiet (Req 20.1 / §11.5) ────────────────────────────────────────────
 * Time-of-day changes over *hours*, so this MUST NOT run a per-frame or tight
 * timer. Instead it recomputes on a COARSE cadence ({@link DEFAULT_CADENCE_MS},
 * ~10 min), pauses on window blur, and — critically — schedules NOTHING while
 * the steady-lighting preference is on. When nothing is scheduled, nothing
 * runs; idle cost is ~0.
 *
 * ── Steady-lighting (Req 1.4 / 21.4) ─────────────────────────────────────────
 * When the steady-lighting accessibility preference is on (root
 * `data-steady-lighting="true"`, set by platform/accessibilityPreferences), the
 * undertone is fully disabled: the inline override is removed so the token
 * default (`transparent`) resumes, and no timer is scheduled. The controller
 * watches the attribute and enables/disables live.
 *
 * ── Authority invariant ──────────────────────────────────────────────────────
 * Pure presentation. Reads a clock + one root attribute; writes only the
 * `--room-undertone` custom property. No store writes, no orchestration.
 *
 * Requirements: 1.4, 21.4
 */
import { onCleanup } from "solid-js";

/** The `--room-undertone` custom property this controller owns. */
export const ROOM_UNDERTONE_PROPERTY = "--room-undertone";

/**
 * Hard upper bound on the undertone strength (design §3.3 "undertone shift
 * ≤6%"). The mix percentage is `|mood| * MAX_UNDERTONE_PERCENT`, so the shift
 * can never exceed this even at the peak of morning/night.
 */
export const MAX_UNDERTONE_PERCENT = 6;

/**
 * Coarse recompute cadence (~10 min). Time-of-day drifts over hours, so this is
 * deliberately slow — it is NOT an animation timer. The visual interpolation is
 * carried by CSS transition on `--room-undertone` (design §3.3 "slow
 * interpolation"), not by frequent JS writes.
 */
export const DEFAULT_CADENCE_MS = 10 * 60 * 1000;

/** Below this magnitude the undertone rests fully transparent (no shift). */
const NEUTRAL_EPSILON = 0.02;

/**
 * Signed mood anchors: hour-of-day → mood in [-1, 1] where NEGATIVE is cool
 * (morning, toward info) and POSITIVE is warm (night, toward warning). Midday
 * rests near neutral. Values interpolate linearly between anchors (wrapping at
 * 24 == 0) so the tone drifts smoothly with no snapping (design §3.3).
 */
const MOOD_ANCHORS: readonly (readonly [hour: number, mood: number])[] = [
  [0, 1.0], // deep night — warm
  [3, 0.9],
  [5, 0.5],
  [6.5, 0.0], // dawn — crossing to cool
  [8, -1.0], // morning — coolest
  [10, -0.7],
  [12, -0.2],
  [14, 0.0], // midday/afternoon — neutral
  [17, 0.1],
  [19, 0.5], // evening — warming
  [21, 0.85],
  [24, 1.0], // wraps to hour 0
] as const;

/**
 * Signed mood for a given hour-of-day (may be fractional). Negative = cool
 * (morning/info), positive = warm (night/warning), ~0 = neutral. Deterministic
 * and side-effect-free (independently unit-testable).
 */
export function moodForHour(hour: number): number {
  const h = ((hour % 24) + 24) % 24;
  for (let i = 0; i < MOOD_ANCHORS.length - 1; i += 1) {
    const [h0, m0] = MOOD_ANCHORS[i];
    const [h1, m1] = MOOD_ANCHORS[i + 1];
    if (h >= h0 && h <= h1) {
      if (h1 === h0) return m0;
      const t = (h - h0) / (h1 - h0);
      return m0 + (m1 - m0) * t;
    }
  }
  return 0;
}

/** Round to 2 decimals so the emitted `%` string stays compact/stable. */
function round2(value: number): number {
  return Math.round(value * 100) / 100;
}

/**
 * Resolve the `--room-undertone` CSS value for a given hour. Returns either
 * `transparent` (neutral rest) or a bounded (≤{@link MAX_UNDERTONE_PERCENT})
 * `color-mix` over the info/warning TOKENS — never a raw color (Req 16.2):
 *   - morning (cool) → mix toward `--color-info-solid`
 *   - night   (warm) → mix toward `--color-warning-solid`
 */
export function undertoneForHour(hour: number): string {
  const mood = moodForHour(hour);
  if (Math.abs(mood) < NEUTRAL_EPSILON) return "transparent";
  const percent = round2(Math.min(MAX_UNDERTONE_PERCENT, Math.abs(mood) * MAX_UNDERTONE_PERCENT));
  if (percent <= 0) return "transparent";
  const tokenVar = mood < 0 ? "var(--color-info-solid)" : "var(--color-warning-solid)";
  return `color-mix(in oklab, ${tokenVar} ${percent}%, transparent)`;
}

export interface RoomUndertoneOptions {
  /**
   * Root scope that receives the `--room-undertone` write. Defaults to
   * `document.documentElement` so the whole Room cascade sees it. Overridable
   * in tests.
   */
  target?: HTMLElement | null;
  /** Injectable clock (tests drive specific hours). Defaults to `Date.now`. */
  now?: () => Date;
  /** Window used for blur/focus. Defaults to the global `window`. */
  win?: Window;
  /** Coarse recompute cadence in ms. Defaults to {@link DEFAULT_CADENCE_MS}. */
  cadenceMs?: number;
  /** Injectable `setTimeout` (tests control the coarse tick). */
  setTimer?: (callback: () => void, ms: number) => ReturnType<typeof setTimeout>;
  /** Injectable `clearTimeout`. */
  clearTimer?: (handle: ReturnType<typeof setTimeout>) => void;
  /**
   * Steady-lighting predicate (Req 21.4). Defaults to reading the root
   * `data-steady-lighting="true"` attribute set by platform prefs.
   */
  isSteadyLighting?: () => boolean;
  /** Test hook fired once per actual write with the value written. */
  onWrite?: (value: string) => void;
}

/**
 * Read the steady-lighting preference from the root `data-steady-lighting`
 * attribute (Req 21.4). Mirrors the boolean `data-high-contrast` convention set
 * by platform/accessibilityPreferences.
 */
function detectSteadyLighting(): boolean {
  if (typeof document === "undefined") return false;
  const root = document.documentElement;
  return !!root && root.getAttribute("data-steady-lighting") === "true";
}

/**
 * Mount the time-of-day undertone controller for the lifetime of the calling
 * reactive scope (component/`createRoot`). Coarse-scheduled + paused-on-blur +
 * fully disabled under steady-lighting — see the module header for the full
 * contract. Teardown (timer cancel, listener/observer removal, inline-override
 * reset) is wired via `onCleanup`, so it MUST be called inside a Solid owner.
 */
export function createRoomUndertoneController(options: RoomUndertoneOptions = {}): void {
  const target =
    options.target ?? (typeof document !== "undefined" ? document.documentElement : null);
  const win = options.win ?? (typeof window !== "undefined" ? window : undefined);
  const now = options.now ?? (() => new Date());
  const cadenceMs = options.cadenceMs ?? DEFAULT_CADENCE_MS;
  const isSteady = options.isSteadyLighting ?? detectSteadyLighting;
  const setTimer =
    options.setTimer ??
    ((cb: () => void, ms: number) => setTimeout(cb, ms) as ReturnType<typeof setTimeout>);
  const clearTimer =
    options.clearTimer ?? ((handle: ReturnType<typeof setTimeout>) => clearTimeout(handle));

  // No DOM (SSR): nothing to publish. The token default (transparent) covers
  // first paint; the browser controller takes over on hydration.
  if (!target) return;

  let timer: ReturnType<typeof setTimeout> | null = null;
  /** Publication is paused (window blurred) — coarse timer is idle. */
  let paused = false;

  const cancelTimer = (): void => {
    if (timer !== null) {
      clearTimer(timer);
      timer = null;
    }
  };

  /** Remove the inline override so the token default (`transparent`) resumes. */
  const clearUndertone = (): void => {
    target.style.removeProperty(ROOM_UNDERTONE_PROPERTY);
  };

  const writeUndertone = (): void => {
    const value = undertoneForHour(now().getHours());
    target.style.setProperty(ROOM_UNDERTONE_PROPERTY, value);
    options.onWrite?.(value);
  };

  /**
   * Recompute once and, unless disabled/paused, schedule the next coarse tick.
   * Steady-lighting short-circuits everything: the override is cleared and NO
   * timer is scheduled (idle-quiet, Req 21.4).
   */
  const tick = (): void => {
    timer = null;
    if (isSteady()) {
      clearUndertone();
      return; // disabled — schedule nothing
    }
    if (paused) return; // blurred — resume on focus
    writeUndertone();
    // One coarse follow-up timer only (never a tight/perpetual loop).
    timer = setTimer(tick, cadenceMs);
  };

  /**
   * (Re)start from a known-clean state: cancel any pending tick, then either
   * disable (steady-lighting) or recompute + reschedule.
   */
  const restart = (): void => {
    cancelTimer();
    tick();
  };

  const onBlur = (): void => {
    paused = true;
    cancelTimer();
  };
  const onFocus = (): void => {
    paused = false;
    restart(); // recompute on return (the hour may have advanced) + reschedule
  };

  if (win && typeof win.addEventListener === "function") {
    win.addEventListener("blur", onBlur);
    win.addEventListener("focus", onFocus);
  }

  // Watch the steady-lighting attribute so the preference toggles live: turning
  // it on disables + clears; turning it off resumes.
  let observer: MutationObserver | undefined;
  if (typeof MutationObserver !== "undefined" && typeof document !== "undefined") {
    observer = new MutationObserver(() => restart());
    observer.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ["data-steady-lighting"],
    });
  }

  // Initial resolution (writes now, or stays transparent under steady-lighting).
  tick();

  onCleanup(() => {
    cancelTimer();
    observer?.disconnect();
    if (win && typeof win.removeEventListener === "function") {
      win.removeEventListener("blur", onBlur);
      win.removeEventListener("focus", onFocus);
    }
    // Idle-quiet reset so a remounted Room isn't stuck on a stale override.
    clearUndertone();
  });
}
