/**
 * trustIndicator — pure logic for the homepage Trust affordance (design.md §9 /
 * §19 "Trust", Requirement 9).
 *
 * Trust is communicated PRIMARILY through behavior, not marketing (Req 9.1):
 *
 *   • **Stays lit offline** — KRIA is local-first, so being offline is the
 *     *normal, healthy* state, never an error. The on-device confirmation
 *     therefore remains fully present/`lit` regardless of connectivity. There
 *     is NO unlit / error / degraded trust state; {@link resolveTrustView}
 *     returns `lit: true` for every possible input (this is a correctness
 *     property, see the PBT suite).
 *
 *   • **Muted, never emerald** — the on-device confirmation is a small
 *     NEUTRAL-toned cue near the Composer, never the earned emerald accent and
 *     never marketing language (Req 9.2). {@link resolveTrustView} always
 *     reports `tone: "muted"`.
 *
 *   • **Visible Core→edge reach on desktop action** — when KRIA reaches out to
 *     the OS/desktop (acting on the computer, running an automation on the
 *     device) there is a visible directed "reach" cue from the Core toward the
 *     screen edge (Req 9.1). This is derived PURELY from the authoritative
 *     `coreStore` state ({@link isDesktopAction}); it is a presence cue, bounded
 *     to the duration of the action — not a free-running animation.
 *
 * ── Read-model only (no backend capability) ──────────────────────────────────
 * This module is a pure projection of two EXISTING signals — the browser/webview
 * connectivity state (`navigator.onLine`) and the authoritative `coreStore`
 * state. It introduces NO new backend/Rust capability and never writes
 * `coreStore` (guardrails.md / Req 30.3). Full privacy/state detail lives in
 * Settings; the indicator only ROUTES there on demand (Req 9.3) — it never
 * mutates a setting or executes an action.
 *
 * Requirements: 9.1, 9.2, 9.3.
 */
import type { Route } from "../../router";
import type { CoreState } from "../../../stores/coreStore";

/**
 * The Trust confirmation is ALWAYS neutral/muted (Req 9.2) — a single-value
 * union so the type system prevents an emerald/accent tone from ever being
 * introduced here.
 */
export type TrustTone = "muted";

/** Connectivity, projected from the browser/webview online state. */
export type Connectivity = "online" | "offline";

/**
 * Core states in which KRIA is "acting on the computer" — reaching out to the
 * OS/desktop — and therefore SHOWS a directed Core→edge reach cue (Req 9.1).
 *
 *   • `acting`             — tool/GUI-cognition execution against the device
 *                            (files, shell, apps, system, on-screen action).
 *   • `running-automation` — a workflow acting on the device.
 *
 * Observation-only states (`watching`, `thinking`, …) are deliberately excluded:
 * they are not KRIA *acting* on the computer, so they raise no reach. Derived
 * purely from the authoritative `coreStore` state (never cursor/OS polling), so
 * it is deterministic and unit-testable.
 */
export const DESKTOP_ACTION_STATES: ReadonlySet<CoreState> = new Set<CoreState>([
  "acting",
  "running-automation",
]);

/** True when the Core state represents KRIA acting on the OS/desktop (Req 9.1). */
export function isDesktopAction(state: CoreState): boolean {
  return DESKTOP_ACTION_STATES.has(state);
}

/**
 * The route the Trust indicator hands off to for full privacy/on-device detail
 * (Req 9.3 / design §17 "Settings hosts trust/privacy detail"). Routing ONLY —
 * the "Memory & Privacy" Settings group owns recall/retention/local-data detail.
 * No Tauri contract is renamed; this reuses the existing typed router + Settings
 * group id.
 */
export const TRUST_SETTINGS_ROUTE: Route = { space: "settings", segment: "memory-privacy" };

/** Inputs to the Trust projection (both existing, consumed read-only). */
export interface TrustInput {
  /** Browser/webview connectivity (`navigator.onLine`). */
  online: boolean;
  /** Authoritative Core state (`coreStore.state()`). */
  coreState: CoreState;
}

/** The resolved, render-ready Trust view. */
export interface TrustView {
  /**
   * Whether the on-device confirmation is present/lit. ALWAYS `true` — local-
   * first means offline is healthy, so trust is never unlit or shown as an
   * error (Req 9.1, stays-lit-offline).
   */
  lit: boolean;
  /** Confirmation tone — ALWAYS `"muted"` (neutral, never emerald — Req 9.2). */
  tone: TrustTone;
  /** Whether a directed Core→edge reach cue is active (desktop action — Req 9.1). */
  reach: boolean;
  /** Connectivity, for the glanceable secondary hint (never an error state). */
  connectivity: Connectivity;
  /** Short glanceable label (no marketing language — Req 9.2). */
  label: string;
  /**
   * Accessible description of the current trust state as TEXT (meaning never by
   * color alone — Req 21.2), including the routing hint to Settings (Req 9.3).
   */
  detail: string;
}

/** The glanceable on-device label. Plain, non-marketing (Req 9.2). */
export const TRUST_LABEL = "On-device";

/**
 * Pure projection: connectivity + Core state → the render-ready Trust view.
 *
 * Invariants (correctness properties, verified by the PBT suite):
 *   • `lit === true` for EVERY input (stays lit offline — Req 9.1).
 *   • `tone === "muted"` for EVERY input (never emerald — Req 9.2).
 *   • `reach === isDesktopAction(coreState)` (reach iff acting on device — Req 9.1).
 *
 * Deterministic and side-effect-free.
 */
export function resolveTrustView(input: TrustInput): TrustView {
  const connectivity: Connectivity = input.online ? "online" : "offline";
  const reach = isDesktopAction(input.coreState);

  // Behavior-first trust: lit whether online OR offline (local-first). The
  // connectivity word is a calm secondary hint, never an error.
  const connectivityHint = connectivity === "offline" ? "Running offline" : "Running locally";
  const reachHint = reach ? " Acting on this device." : "";

  return {
    lit: true,
    tone: "muted",
    reach,
    connectivity,
    label: TRUST_LABEL,
    detail: `${connectivityHint}, on your device.${reachHint} Open Settings for privacy detail.`,
  };
}
