/**
 * Core Tray — reflect Core state on the OS tray/menu-bar glyph.
 *
 * This module owns the subscription that pushes the current KRIA Core state to
 * the OS tray icon. It is an **enhancement layer** (Req 3.4, 18.2): the tray is
 * presentation feedback only — it performs NO orchestration and never blocks or
 * crashes. When the tray command is unavailable (Linux Wayland / no tray), the
 * push degrades silently and the in-app `CorePresence` remains the guaranteed
 * fallback state indicator.
 *
 * ── Bucketing ───────────────────────────────────────────────────────────────
 * A tray glyph cannot legibly express the 14+ Core states, so we collapse them
 * into four coarse buckets (idle / working / needs-attention / error). The
 * backend `set_tray_core_state` command maps each bucket to a tooltip/glyph.
 *
 * ── Coalescing ──────────────────────────────────────────────────────────────
 * Core state can change rapidly (thinking → acting → speaking within
 * milliseconds). To avoid spamming the OS tray, changes are coalesced over a
 * short trailing window and de-duplicated: only the latest bucket in a burst is
 * pushed, and only when it differs from the last bucket already sent.
 *
 * Requirements: 3.4 (tray reflects Core state, enhancement + fallback),
 *               18.2 (tray/hotkey/always-on-top are enhancements w/ fallback)
 */
import { eventBus } from "./eventBus";
import type { Unsubscribe } from "./eventBus";
import { coreStore, ACTIVE_STATES, type CoreState } from "./coreStore";
import { bridgeInvokeOptional } from "../bridge";

// ─── Buckets ─────────────────────────────────────────────────────────────────

/** Coarse tray-glyph buckets (a tray can't show 14 distinct glyphs). */
export type TrayBucket = "idle" | "working" | "needs-attention" | "error";

/** Backend command that updates the tray glyph. Optional/enhancement. */
export const TRAY_COMMAND = "set_tray_core_state";

/** Default trailing-coalesce window (ms) for tray pushes. */
export const DEFAULT_TRAY_THROTTLE_MS = 200;

/**
 * Pure map: Core state → tray bucket.
 *
 * - error / recovering → `error`
 * - blocked / waiting  → `needs-attention`
 * - any ACTIVE_STATES  → `working`
 * - idle (and anything else) → `idle`
 */
export function coreStateToBucket(state: CoreState): TrayBucket {
  if (state === "error" || state === "recovering") return "error";
  if (state === "blocked" || state === "waiting") return "needs-attention";
  if (ACTIVE_STATES.has(state)) return "working";
  return "idle";
}

// ─── Push strategy (injectable for tests) ────────────────────────────────────

/** How a resolved bucket is delivered to the OS. Injectable for testing. */
export type TrayPush = (bucket: TrayBucket) => void;

/**
 * Default push: fire-and-forget through the optional bridge invoke. Never
 * throws — `bridgeInvokeOptional` swallows unavailability (returns null) so a
 * missing tray degrades silently (Req 18.2).
 */
const defaultPush: TrayPush = (bucket) => {
  void bridgeInvokeOptional(TRAY_COMMAND, { state: bucket });
};

export interface CoreTrayOptions {
  /** Override the delivery mechanism (tests inject a spy). */
  push?: TrayPush;
  /** Trailing-coalesce window in ms. */
  throttleMs?: number;
}

// ─── Module state ────────────────────────────────────────────────────────────

let unsub: Unsubscribe | null = null;
let timer: ReturnType<typeof setTimeout> | null = null;
let pendingBucket: TrayBucket | null = null;
let lastPushed: TrayBucket | null = null;
let activePush: TrayPush = defaultPush;
let activeThrottleMs = DEFAULT_TRAY_THROTTLE_MS;

/**
 * Schedule a bucket push on the trailing edge of the coalesce window. A burst
 * of rapid changes collapses into a single push carrying the latest bucket.
 */
function schedule(bucket: TrayBucket): void {
  pendingBucket = bucket;
  if (timer !== null) return; // window already open — just update pendingBucket

  timer = setTimeout(() => {
    timer = null;
    const next = pendingBucket;
    pendingBucket = null;
    if (next === null) return;
    if (next === lastPushed) return; // de-dupe: unchanged bucket, skip
    lastPushed = next;
    try {
      activePush(next);
    } catch {
      // Tray is an enhancement — never let a push failure surface. In-app
      // CorePresence remains the guaranteed fallback (Req 18.2).
    }
  }, activeThrottleMs);
}

// ─── Public API ──────────────────────────────────────────────────────────────

/**
 * Subscribe the tray to Core state changes. Idempotent; returns a dispose fn.
 * Pushes the current Core state once on init so the tray reflects boot state.
 */
export function initCoreTray(options?: CoreTrayOptions): Unsubscribe {
  if (unsub) return disposeCoreTray;

  activePush = options?.push ?? defaultPush;
  activeThrottleMs = options?.throttleMs ?? DEFAULT_TRAY_THROTTLE_MS;
  lastPushed = null;
  pendingBucket = null;

  // Reflect the current state immediately (through the coalescer).
  schedule(coreStateToBucket(coreStore.state()));

  // React to every subsequent Core state change. "none" = observe synchronously;
  // the coalescer here (not the bus) throttles the outgoing tray pushes.
  unsub = eventBus.on(
    "core:state-changed",
    (p) => schedule(coreStateToBucket(p.state as CoreState)),
    "none",
  );

  return disposeCoreTray;
}

/** Detach the tray subscription and clear any pending push. */
export function disposeCoreTray(): void {
  if (timer !== null) {
    clearTimeout(timer);
    timer = null;
  }
  if (unsub) {
    unsub();
    unsub = null;
  }
  pendingBucket = null;
  lastPushed = null;
}
