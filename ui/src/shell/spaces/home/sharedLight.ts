/**
 * Shared-light publisher — the Core → Room light bridge (design.md §3.2 / §13.2,
 * Requirements 1.1, 17.2, 17.5).
 *
 * The Core publishes its live state as CSS custom properties on a root scope;
 * the environmental DOM (floor sheen, Composer top rim-light, chip/Dock tint,
 * Orbit glow) reads them so the whole Room reacts to ONE light — no scene
 * lighting engine, one WebGL surface for the Core only (Req 17.5). This is the
 * single mechanism that makes the Room feel inhabited rather than a webpage.
 *
 * Published variables (design §3.2 / §12.1):
 *   --core-x, --core-y    normalized Core position (floor-sheen offset).
 *   --core-intensity      0..1 current luminance (breath + state).
 *   --core-hue            temperature within the accent family / attention hue,
 *                         published as a per-state `--presence-*` TOKEN reference
 *                         so dark+light parity and zero-raw-color hold (Req 16).
 *   --core-lean           signed lean toward the Composer/arrival.
 *   --core-depth          signed step-forward / depth-recede signal (design §4.1,
 *                         Req 2.6): + glides the Core forward (blocked/needs-you),
 *                         − recedes it (working/turn-active), 0 at rest.
 *
 * ── Meaningful-intent behaviors (task 2.1, Req 2.5 / 2.6) ────────────────────
 * The Core reacts to *meaningful intent ONLY* — arrival, composer focus, typing,
 * voice, approval — and NEVER tracks continuous cursor movement (Req 2.5). Two
 * axes carry this:
 *
 *   • `--core-lean` is driven by meaningful intent toward the Composer. Voice
 *     attention (the `listening` Core state) leans from the state mapping; the
 *     Composer/arrival lean is driven by the `presenceIntent` signal below
 *     (set by the Composer on focus and by HomeSpace on arrival). This is why
 *     the old hardcoded `lean = 0` becomes intent-driven — there is still ONE
 *     shared-light mechanism; the intent just feeds this publisher.
 *   • `--core-depth` is driven purely by the authoritative `coreStore` state:
 *     `blocked` glides forward (Req 2.6: step it forward), turn-active work
 *     (acting/responding/running-automation) recedes in depth (design §4.1).
 *
 * ── Performance contract (the hard part) ─────────────────────────────────────
 * CorePresence is CSS/SVG-first with NO JS animation loop, so idle cost is ~0
 * (Req 20.1 / §11.5). This publisher MUST NOT reintroduce a permanent, always-on
 * rAF loop. Instead it is *reactive + throttled + idle-quiet*:
 *
 *   • Writes are driven by `coreStore` state changes (a Solid `createEffect`),
 *     never by a self-perpetuating rAF. When nothing changes, nothing runs.
 *   • Writes are coalesced to AT MOST ONE per animation frame (Req 17.2 /
 *     design §13.2 "≤1/frame"): many synchronous state changes within a frame
 *     collapse into a single `--core-*` write.
 *   • Publication is PAUSED on window blur and resumed on focus (Req 17.3 /
 *     §11.5 "paused on blur"); a pending frame is cancelled on blur and the
 *     latest state is flushed once on focus.
 *
 * ── Authority invariant ──────────────────────────────────────────────────────
 * This publisher only READS `coreStore.state()`. It NEVER writes back to
 * `coreStore` (guardrails.md / Req 30.3 — `coreStore` is the sole authority for
 * Core state; `coreHint` is advisory only). The Focus engine is separate.
 *
 * Requirements: 1.1, 17.2, 17.5
 */
import { createRenderEffect, createSignal, onCleanup, type Accessor } from "solid-js";
import { coreStore } from "../../../stores";
import type { CoreState } from "../../../stores/coreStore";

/** The shared-light custom properties, in publish order (design §3.2 / §4.1). */
export const SHARED_LIGHT_PROPERTIES = [
  "--core-x",
  "--core-y",
  "--core-intensity",
  "--core-hue",
  "--core-lean",
  "--core-depth",
] as const;

/**
 * Off-center light-pool position (design §3.1 / token defaults `--core-x`
 * `0.5`, `--core-y` `0.42`). The Core is centered on the homepage's vertical
 * axis at rest; positional reaction (cursor-free) is owned by later Core
 * behaviors (task 2.1), so these stay constant here.
 */
const CORE_X = 0.5;
const CORE_Y = 0.42;

/**
 * Per-state luminance for `--core-intensity` (0..1), mapping the §4.1 presence
 * table's "density/light" axis. Attention states stay CALM (blocked/waiting do
 * not blaze — Req 3.3: blocked calms the Core), active conversation reads
 * brightest (listening/speaking), and idle rests low. Scalars only (no color),
 * so token-lint stays green.
 */
const STATE_INTENSITY: Readonly<Record<CoreState, number>> = {
  idle: 0.55,
  listening: 0.9,
  thinking: 0.7,
  planning: 0.7,
  speaking: 0.85,
  responding: 0.85,
  acting: 0.75,
  "running-automation": 0.72,
  watching: 0.68,
  remembering: 0.65,
  reflecting: 0.62,
  learning: 0.66,
  waiting: 0.6,
  blocked: 0.6,
  error: 0.7,
  recovering: 0.62,
};

/**
 * Per-state hue, published as a reference to the matching `--presence-*` design
 * token (defined in `tokens.generated.css`, dark+light parity). Explicit full
 * token references (not string-built) so token-lint can verify every one is
 * defined and zero raw color leaks (Req 16).
 */
const STATE_HUE_VAR: Readonly<Record<CoreState, string>> = {
  idle: "var(--presence-idle)",
  listening: "var(--presence-listening)",
  thinking: "var(--presence-thinking)",
  planning: "var(--presence-planning)",
  speaking: "var(--presence-speaking)",
  responding: "var(--presence-responding)",
  acting: "var(--presence-acting)",
  "running-automation": "var(--presence-running-automation)",
  watching: "var(--presence-watching)",
  remembering: "var(--presence-remembering)",
  reflecting: "var(--presence-reflecting)",
  learning: "var(--presence-learning)",
  waiting: "var(--presence-waiting)",
  blocked: "var(--presence-blocked)",
  error: "var(--presence-error)",
  recovering: "var(--presence-recovering)",
};

/**
 * Per-state signed depth for `--core-depth` (design §4.1, Req 2.6). Positive
 * glides the Core FORWARD ("step it forward" when blocked/needs-you); negative
 * RECEDES it in depth (turn-active work reads "quietly busy"). This is derived
 * purely from the authoritative `coreStore` state — never from cursor position
 * (Req 2.5) — so it is deterministic and unit-testable.
 *
 *   • blocked → +1 (glide forward + calm + warm attention hue — Req 2.6).
 *   • acting / responding / running-automation → −1 (working/turn-active recede).
 *   • everything else → 0 (rest at the Core plane).
 */
const STATE_DEPTH: Readonly<Record<CoreState, number>> = {
  idle: 0,
  listening: 0,
  thinking: 0,
  planning: 0,
  speaking: 0,
  responding: -1,
  acting: -1,
  "running-automation": -1,
  watching: 0,
  remembering: 0,
  reflecting: 0,
  learning: 0,
  waiting: 0,
  blocked: 1,
  error: 0,
  recovering: 0,
};

/**
 * Per-state baseline lean toward the Composer (design §4.1 "attention" row).
 * Voice attention (`listening`) leans as a meaningful-intent reaction; all other
 * states rest at 0 and let the `presenceIntent` signal (composer focus / arrival)
 * drive the lean. Cursor movement NEVER contributes (Req 2.5).
 */
const STATE_LEAN: Readonly<Record<CoreState, number>> = {
  idle: 0,
  listening: 0.6,
  thinking: 0,
  planning: 0,
  speaking: 0,
  responding: 0,
  acting: 0,
  "running-automation": 0,
  watching: 0,
  remembering: 0,
  reflecting: 0,
  learning: 0,
  waiting: 0,
  blocked: 0,
  error: 0,
  recovering: 0,
};

/** The resolved shared-light values for one Core state. */
export interface SharedLight {
  /** Normalized Core X (0..1). */
  x: number;
  /** Normalized Core Y (0..1). */
  y: number;
  /** Current luminance (0..1). */
  intensity: number;
  /**
   * Core temperature, published as a `--presence-*` TOKEN reference (never a
   * raw color) so dark+light parity is preserved via the theme tokens.
   */
  hue: string;
  /** Signed lean toward the Composer/arrival. */
  lean: number;
  /** Signed step-forward (+) / depth-recede (−) signal (design §4.1, Req 2.6). */
  depth: number;
}

/**
 * Pure mapping: Core state → shared-light values. Deterministic and
 * side-effect-free so it is independently unit-testable. `hue` resolves to the
 * per-state presence hue token (`--presence-idle`, `--presence-thinking`, …,
 * `--presence-running-automation`) defined in `tokens.generated.css`.
 *
 * `lean` here is the STATE baseline (voice attention only). The publisher folds
 * in the meaningful-intent lean (composer focus / arrival) from
 * {@link presenceIntent}; the final published `--core-lean` is the stronger of
 * the two so a voice+composer moment doesn't cancel out.
 */
export function sharedLightForState(state: CoreState): SharedLight {
  return {
    x: CORE_X,
    y: CORE_Y,
    intensity: STATE_INTENSITY[state],
    hue: STATE_HUE_VAR[state],
    lean: STATE_LEAN[state],
    depth: STATE_DEPTH[state],
  };
}

// ─── Meaningful-intent lean signal (Req 2.5) ─────────────────────────────────
//
// The Core leans toward the Composer on *meaningful intent only* — composer
// focus (sustained) and arrival/typing (a brief pulse). This is a tiny reactive
// signal, NOT a cursor tracker: there are no mousemove/pointermove listeners
// anywhere in the presence layer. The Composer (task 5.1) calls
// `setComposerFocused` on focus/blur; HomeSpace calls `pulseArrival` on mount.

const [composerFocused, setComposerFocused] = createSignal(false);
const [arrivalLean, setArrivalLean] = createSignal(0);
let arrivalTimer: ReturnType<typeof setTimeout> | undefined;

/** Default arrival/typing lean pulse duration (ms) before it settles back. */
const ARRIVAL_LEAN_MS = 2200;

/**
 * Reactive meaningful-intent lean toward the Composer (0..1). Read by the
 * publisher; folded with the per-state baseline lean. Cursor movement never
 * feeds this (Req 2.5).
 */
export const presenceIntent = {
  /** Current intent-driven lean magnitude (0..1). */
  lean(): number {
    return Math.max(composerFocused() ? 1 : 0, arrivalLean());
  },
  /** Composer focus/blur → sustained lean toward the Composer (Req 2.5 / 4.3). */
  setComposerFocused(focused: boolean): void {
    setComposerFocused(focused);
  },
  /**
   * Arrival / typing → a brief lean that settles back on its own (design §4.1
   * "brief lean toward Composer"). A single one-shot timer (no rAF loop, idle
   * cost ~0). Re-arming restarts the dwell.
   */
  pulseArrival(durationMs: number = ARRIVAL_LEAN_MS): void {
    if (arrivalTimer !== undefined) clearTimeout(arrivalTimer);
    setArrivalLean(1);
    arrivalTimer = setTimeout(() => {
      arrivalTimer = undefined;
      setArrivalLean(0);
    }, durationMs);
  },
  /** Clear all intent lean (teardown/tests). */
  reset(): void {
    if (arrivalTimer !== undefined) {
      clearTimeout(arrivalTimer);
      arrivalTimer = undefined;
    }
    setComposerFocused(false);
    setArrivalLean(0);
  },
};

export interface SharedLightPublisherOptions {
  /**
   * Core-state source. Defaults to the live `coreStore.state` reader. An
   * explicit accessor lets tests/detached surfaces drive it without the store.
   */
  state?: Accessor<CoreState>;
  /**
   * Root scope that receives the `--core-*` writes. Defaults to
   * `document.documentElement` so every consumer (Room, Composer, chip, Dock,
   * Orbit) reads the same light via the `:root` cascade. Overridable in tests.
   */
  target?: HTMLElement | null;
  /** Window used for blur/focus + rAF. Defaults to the global `window`. */
  win?: Window;
  /** Injectable `requestAnimationFrame` (tests drive frames deterministically). */
  requestFrame?: (callback: FrameRequestCallback) => number;
  /** Injectable `cancelAnimationFrame`. */
  cancelFrame?: (handle: number) => void;
  /**
   * Meaningful-intent lean source (0..1). Defaults to {@link presenceIntent}'s
   * live reader. Injectable so tests can drive composer-focus/arrival lean
   * without the shared signal.
   */
  intentLean?: Accessor<number>;
  /** Test hook fired once per actual flush with the values written. */
  onFlush?: (light: SharedLight) => void;
}

/**
 * Mount the shared-light publisher for the lifetime of the calling reactive
 * scope (component/`createRoot`). Reactive + rAF-throttled + paused-on-blur +
 * idle-quiet — see the module header for the full contract. Returns nothing;
 * teardown (frame cancel, listener removal, inline-override reset) is wired via
 * `onCleanup`, so it MUST be called inside a Solid owner.
 */
export function createSharedLightPublisher(options: SharedLightPublisherOptions = {}): void {
  const state = options.state ?? coreStore.state;
  const target =
    options.target ?? (typeof document !== "undefined" ? document.documentElement : null);
  const win = options.win ?? (typeof window !== "undefined" ? window : undefined);
  const raf =
    options.requestFrame ??
    (win && typeof win.requestAnimationFrame === "function"
      ? win.requestAnimationFrame.bind(win)
      : undefined);
  const caf =
    options.cancelFrame ??
    (win && typeof win.cancelAnimationFrame === "function"
      ? win.cancelAnimationFrame.bind(win)
      : undefined);

  // No DOM (SSR): nothing to publish. Token defaults cover first paint.
  if (!target) return;

  let frame: number | null = null;
  /** A state change is pending an unwritten flush. */
  let dirty = false;
  /** Publication is paused (window blurred). */
  let paused = false;

  const intentLean = options.intentLean ?? presenceIntent.lean;

  const write = (): void => {
    const base = sharedLightForState(state());
    // Fold the meaningful-intent lean (composer focus / arrival) into the
    // per-state baseline — the stronger of the two wins so voice + composer
    // don't cancel (Req 2.5). Depth stays purely state-driven (Req 2.6).
    const light: SharedLight = { ...base, lean: Math.max(base.lean, intentLean()) };
    target.style.setProperty("--core-x", `${light.x}`);
    target.style.setProperty("--core-y", `${light.y}`);
    target.style.setProperty("--core-intensity", `${light.intensity}`);
    target.style.setProperty("--core-hue", light.hue);
    target.style.setProperty("--core-lean", `${light.lean}`);
    target.style.setProperty("--core-depth", `${light.depth}`);
    options.onFlush?.(light);
  };

  const flush = (): void => {
    frame = null;
    // Paused after scheduling: keep `dirty` so focus flushes the latest state.
    if (paused) return;
    dirty = false;
    write();
  };

  const schedule = (): void => {
    dirty = true;
    if (paused) return;
    // ≤1 scheduled flush per frame — coalesce a burst of changes into one write.
    if (frame !== null) return;
    if (!raf) {
      // No rAF available (non-browser/SSR fallback): write immediately, once.
      flush();
      return;
    }
    frame = raf(flush);
  };

  // Reactive publication: every `coreStore` state change marks the light dirty
  // and schedules at most one rAF-batched write. A render effect runs
  // synchronously on the state change (before paint, so the CSS vars are ready
  // first frame) — NOT a self-perpetuating rAF. When the state is stable this
  // effect never re-runs, so idle cost is ~0 (Req 20.1).
  createRenderEffect(() => {
    state(); // track Core state (authority: hue / intensity / depth)
    intentLean(); // track meaningful-intent lean (composer focus / arrival)
    schedule();
  });

  // Pause on blur, resume + flush latest on focus (Req 17.3 / §11.5).
  const onBlur = (): void => {
    paused = true;
    if (frame !== null && caf) {
      caf(frame);
      frame = null;
    }
  };
  const onFocus = (): void => {
    const wasPaused = paused;
    paused = false;
    // On regaining focus, flush the current light once so the Room re-syncs to
    // any state that changed while blurred. A discrete user event, not a loop.
    if (wasPaused || dirty) schedule();
  };

  if (win && typeof win.addEventListener === "function") {
    win.addEventListener("blur", onBlur);
    win.addEventListener("focus", onFocus);
  }

  onCleanup(() => {
    if (frame !== null && caf) caf(frame);
    frame = null;
    if (win && typeof win.removeEventListener === "function") {
      win.removeEventListener("blur", onBlur);
      win.removeEventListener("focus", onFocus);
    }
    // Restore the token defaults (idle-quiet reset) so a remounted surface
    // isn't stuck on a stale inline override.
    for (const property of SHARED_LIGHT_PROPERTIES) {
      target.style.removeProperty(property);
    }
  });
}
