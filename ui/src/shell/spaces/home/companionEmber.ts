/**
 * CompanionEmber — pure logic + preference for the floating cross-application
 * ember (design.md §8/§9, Requirements 13.4, 15.1–15.5). The component
 * (`CompanionEmber.tsx`) is thin presentation over these deterministic,
 * side-effect-free helpers so the correctness properties (ember mirrors the
 * authoritative Core state; brighten ONLY for meaningful needs; opt-out
 * disables it) are unit- and property-testable in isolation.
 *
 * ── Authority invariant (Req 30.3, guardrails.md "Never") ────────────────────
 * `coreStore` is the SOLE authority for Core state. Everything here only READS
 * a `CoreState` VALUE (the ember inherits / mirrors it, design §8.1) — nothing
 * in this module writes `coreStore`. The ember therefore always reflects the
 * one Core, never a divergent copy.
 *
 * ── Cheap 2D only (Req 15.2) ─────────────────────────────────────────────────
 * The ember is a CSS/2D glow (it reuses `CorePresence`, which is CSS/SVG-first,
 * NOT the WebGL 3D scene). No render-mode/3D logic lives here.
 */
import { createSignal, type JSX } from "solid-js";
import { ATTENTION_STATES, type CoreState } from "../../../stores/coreStore";
import type { EdgeAnchor, HomeViewMode } from "../../../stores/homeStore";
import type { GeometryMonitor, WindowGeometry } from "../../../windowing/windowGeometry";

// ─── Meaningful-need gate (Req 15.2) ─────────────────────────────────────────

/**
 * The Core states that, on their own, constitute a *meaningful need* worth
 * brightening the ember for: the attention states (`blocked` / `waiting` /
 * `error`). These are exactly `coreStore.ATTENTION_STATES` — the ember reuses
 * the SAME meaningful-intent classification the CorePresence work established,
 * so "needs you" is defined once. Ordinary work states (thinking, acting,
 * responding, remembering, …) and `idle` are deliberately NOT here: the ember
 * must NOT brighten for idle chatter (design §9 "brightens only for meaningful
 * needs"; guardrails "rest to silence when nothing true qualifies").
 */
export const MEANINGFUL_NEED_STATES: ReadonlySet<CoreState> = ATTENTION_STATES;

/** Extra, non-state meaningful-need signals (design §9 examples). */
export interface MeaningfulNeedSignals {
  /** A decision is pending in the Approval Center ("approval", design §9). */
  pendingApproval?: boolean;
  /** Requested work just finished — a brief celebratory brighten (design §9). */
  workJustFinished?: boolean;
}

/**
 * Whether the ember should brighten. TRUE iff the mirrored Core state is a
 * meaningful-need (attention) state, OR an approval is pending, OR requested
 * work just finished. Pure and total: any other state (idle / ordinary work)
 * returns FALSE, so the ember stays a calm dim presence and never brightens for
 * idle chatter (Req 15.2). This is the single brighten authority the component
 * and its tests share.
 */
export function isMeaningfulNeed(state: CoreState, signals?: MeaningfulNeedSignals): boolean {
  return (
    MEANINGFUL_NEED_STATES.has(state) ||
    signals?.pendingApproval === true ||
    signals?.workJustFinished === true
  );
}

// ─── Compositor fallback resolution (Req 15.5) ───────────────────────────────

/** How the ember is presented: a real always-on-top OS window, or in-app. */
export type CompanionPresentation = "floating-window" | "in-app";

/** Capabilities that decide the ember presentation (probed, then passed in). */
export interface CompanionCompositorCaps {
  /** A Tauri multi-window host is available (vs a plain browser/webview). */
  tauri: boolean;
  /** The compositor permits always-on-top + global positioning (Wayland/X11). */
  alwaysOnTopSupported: boolean;
}

/**
 * Resolve the ember presentation from compositor capabilities (Req 15.5). A
 * true floating always-on-top ember needs BOTH a Tauri host AND compositor
 * support; otherwise degrade to the guaranteed in-app presence — never break.
 * Pure so the degrade decision is fully unit-testable.
 */
export function resolveCompanionPresentation(caps: CompanionCompositorCaps): CompanionPresentation {
  return caps.tauri && caps.alwaysOnTopSupported ? "floating-window" : "in-app";
}

// ─── Window geometry per mode (Req 13.4 / 15.1) ──────────────────────────────

/** The ember's small square window edge length (logical px). */
export const EMBER_LOGICAL_SIZE = 96;
/** Margin from the work-area edge the ember anchors against (logical px). */
export const EMBER_LOGICAL_MARGIN = 24;
/** Default screen corner the ember anchors to when none is remembered. */
export const DEFAULT_EMBER_ANCHOR: EdgeAnchor = "bottom-right";

/** The four edge anchors, in nudge-cycle order (design §9 "optional reposition"). */
export const EMBER_ANCHORS: readonly EdgeAnchor[] = [
  "bottom-right",
  "bottom-left",
  "top-left",
  "top-right",
] as const;

/** Next anchor in the cycle — powers keyboard/tap reposition (design §9). */
export function nextEmberAnchor(anchor: EdgeAnchor): EdgeAnchor {
  const i = EMBER_ANCHORS.indexOf(anchor);
  return EMBER_ANCHORS[(i + 1) % EMBER_ANCHORS.length];
}

/**
 * The small, edge-anchored always-on-top ember window geometry for a monitor
 * (design §8.1 "Companion is always-on-top + small"). Scales the logical size
 * by the monitor scale factor and clamps into the work area. Pure so the
 * per-mode geometry math is unit-testable without a live desktop.
 */
export function emberWindowGeometry(monitor: GeometryMonitor, anchor: EdgeAnchor): WindowGeometry {
  const work = monitor.workArea;
  const scale = monitor.scaleFactor;
  const size = Math.min(EMBER_LOGICAL_SIZE * scale, work.size.width, work.size.height);
  const margin = Math.min(EMBER_LOGICAL_MARGIN * scale, Math.max(0, work.size.width - size));

  const isRight = anchor === "top-right" || anchor === "bottom-right";
  const isBottom = anchor === "bottom-left" || anchor === "bottom-right";

  const x = isRight
    ? work.position.x + work.size.width - size - margin
    : work.position.x + margin;
  const y = isBottom
    ? work.position.y + work.size.height - size - margin
    : work.position.y + margin;

  return { x: Math.round(x), y: Math.round(y), width: Math.round(size), height: Math.round(size), scaleFactor: scale };
}

/**
 * CSS anchoring for the in-app ember overlay: which corner it pins to. Returns
 * only the sides that should be set (the others stay `auto`), so the ember sits
 * flush in the chosen corner with the standard edge margin. Pure/token-driven.
 */
export function emberAnchorStyle(anchor: EdgeAnchor): JSX.CSSProperties {
  const isRight = anchor === "top-right" || anchor === "bottom-right";
  const isBottom = anchor === "bottom-left" || anchor === "bottom-right";
  return {
    top: isBottom ? "auto" : "var(--space-4)",
    bottom: isBottom ? "var(--space-4)" : "auto",
    left: isRight ? "auto" : "var(--space-4)",
    right: isRight ? "var(--space-4)" : "auto",
  };
}

// ─── On-by-default opt-out preference (Req 15.4) ─────────────────────────────

/**
 * Companion Mode is ON by default with a one-setting opt-out (Req 15.4). This
 * is a LOCAL UI preference (single-user, local-first): persisted in
 * localStorage — no backend/config command is added (task scope: consume
 * existing contracts only). Default = enabled; only an explicit opt-out
 * disables the ember.
 */
export const COMPANION_ENABLED_STORAGE_KEY = "kria_companion_enabled_v1";

function readCompanionEnabled(): boolean {
  if (typeof window === "undefined") return true; // on-by-default (SSR/no-DOM)
  try {
    // Only the explicit string "false" opts out; anything else (incl. unset)
    // keeps the on-by-default posture.
    return window.localStorage.getItem(COMPANION_ENABLED_STORAGE_KEY) !== "false";
  } catch {
    return true;
  }
}

const [companionEnabled, setCompanionEnabledSignal] = createSignal(readCompanionEnabled());

/** The one-setting opt-out toggle. `true` = ember active (default). */
export const companionPreference = {
  /** Whether the floating ember is enabled (on-by-default). */
  enabled: companionEnabled,
  /** Set the opt-out preference and persist it (local-first). */
  setEnabled(value: boolean): void {
    setCompanionEnabledSignal(value);
    if (typeof window === "undefined") return;
    try {
      window.localStorage.setItem(COMPANION_ENABLED_STORAGE_KEY, value ? "true" : "false");
    } catch {
      // Preference persistence is an enhancement; the ember still honors the
      // in-memory signal for this session when storage is unavailable.
    }
  },
  /** Re-read from storage (tests / cross-tab hydration). */
  refresh(): void {
    setCompanionEnabledSignal(readCompanionEnabled());
  },
} as const;

// ─── Return-mode memory (Req 15.3 continuous return) ─────────────────────────

/**
 * The view mode to return to when the ember condenses back into the window
 * (design §9 "return to window → homepage state restored"). Tracks the last
 * non-Companion mode so returning restores where the user came from; defaults
 * to Standard (the recommended default) before any switch. Kept module-local
 * (presentation-only continuity), never a domain store.
 */
let priorMode: Exclude<HomeViewMode, "companion"> = "standard";

/** Record the mode the user was in before Companion (called on view changes). */
export function rememberPriorMode(mode: HomeViewMode): void {
  if (mode !== "companion") priorMode = mode;
}

/** The mode a Companion return should restore (design §9). */
export function returnViewMode(): Exclude<HomeViewMode, "companion"> {
  return priorMode;
}
