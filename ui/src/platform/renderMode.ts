/**
 * platform/renderMode — shared generic lens render-mode gate/store.
 *
 * Capability constellation may consume this state for renderer selection.
 * Shipped Memory Graph currently reads only `isStatic`; it always mounts its
 * authoritative 2D SVG and does not consume `enable3D`. `GraphCanvas3D` remains
 * dormant unless a future MGR-030 Phase 7 integration passes every gate.
 *
 * ── Contract for lens surfaces ──────────────────────────────────────────────
 * A lens MUST:
 *   1. Default to its 2D representation. 2D is ALWAYS available and accessible
 *      (Req 5.5 / 7.5 / 17.5). Never assume 3D.
 *   2. Read `useLensRenderMode()` (reactive) — or the one-shot `lensRenderMode()`
 *      snapshot — and only mount its 3D scene when `enable3D === true`.
 *   3. Treat `isStatic === true` (reduced-motion) as "render a still frame, no
 *      animation loop" — this holds for BOTH the 2D and 3D representations
 *      (Req 16.3 / 16.4 / 17.4). Under reduced-motion the mode is forced to 2D.
 *   4. Re-evaluate reactively: if 3D later disables (e.g. auto-degrade under
 *      load calls `degradeToTwoD`), the lens must tear its scene down and show
 *      the 2D representation without a reload.
 *
 * The gate is 2D-first: 3D is enabled ONLY when capability detection passes AND
 * an on-device §11.3 G2 probe has run and passed (see capabilities.ts). A lens
 * that wants 3D is responsible for RUNNING that probe (via
 * prototypes/gateProbes.ts::runG2Probe) once, then calling `applyProbeResult`.
 * Until then the gate resolves to 2D — an ACCEPTED outcome per §11.2, not a
 * failure.
 */
import { createRoot, createSignal, type Accessor } from "solid-js";
import {
  decideRenderMode,
  detectCapabilities,
  type CapabilitySnapshot,
  type ProbeResult,
  type RenderMode,
} from "./capabilities";

/** Reactive lens render-mode state consumed by graph/constellation surfaces. */
export interface LensRenderState {
  /** Chosen mode. 2D is the default and is always safe. */
  mode: RenderMode;
  /** True only when 3D is enabled (capability + passing G2 probe). */
  enable3D: boolean;
  /** Human-readable reason (diagnostics / a11y notice). */
  reason: string;
  /**
   * True when the user prefers reduced motion. Lenses render a STATIC frame
   * (no animation loop) in this case, in either representation.
   */
  isStatic: boolean;
  /** The capability snapshot the decision was derived from. */
  snapshot: CapabilitySnapshot;
}

/** Reason shown when the user explicitly selects a representation. */
const MANUAL_TWO_D_REASON = "2D fallback: your preference (low power / accessibility)";
const MANUAL_THREE_D_REASON = "3D enabled: your preference";

/**
 * Derive the lens render state from capabilities and explicit user preference.
 * Hard capability constraints (WebGL and reduced motion) still win over a
 * manual 3D request; the performance probe only governs automatic selection.
 */
function computeState(
  snapshot: CapabilitySnapshot,
  preferTwoD: boolean,
  preferThreeD = false,
): LensRenderState {
  if (preferTwoD) {
    return {
      mode: "2d",
      enable3D: false,
      reason: MANUAL_TWO_D_REASON,
      isStatic: snapshot.prefersReducedMotion,
      snapshot,
    };
  }
  if (preferThreeD && snapshot.hasWebGL && !snapshot.prefersReducedMotion) {
    return {
      mode: "3d",
      enable3D: true,
      reason: MANUAL_THREE_D_REASON,
      isStatic: false,
      snapshot,
    };
  }
  const decision = decideRenderMode(snapshot);
  return {
    mode: decision.mode,
    enable3D: decision.enable3D,
    reason: decision.reason,
    isStatic: snapshot.prefersReducedMotion,
    snapshot,
  };
}

/**
 * The global store lives in a detached reactive root so it exists once for the
 * whole app (like the other shell/platform stores) and never gets disposed with
 * a component tree. It holds the capability snapshot and the manual 2D
 * preference; the public render state is derived from both.
 */
const store = createRoot(() => {
  // Seed from a live capability detection. In non-browser/test environments
  // detection resolves to the safe 2D-default posture.
  const initial = detectCapabilities();
  const [snapshot, setSnapshot] = createSignal<CapabilitySnapshot>(initial);
  const [preferTwoD, setPreferTwoDSignal] = createSignal(false);
  const [preferThreeD, setPreferThreeDSignal] = createSignal(false);
  const [state, setState] = createSignal<LensRenderState>(computeState(initial, false));
  return {
    snapshot,
    setSnapshot,
    preferTwoD,
    setPreferTwoDSignal,
    preferThreeD,
    setPreferThreeDSignal,
    state,
    setState,
  };
});

/** Reactive accessor for the current lens render state. */
export const lensRenderMode: Accessor<LensRenderState> = store.state;

/** Reactive accessors for explicit representation preferences. */
export const preferTwoD: Accessor<boolean> = store.preferTwoD;
export const preferThreeD: Accessor<boolean> = store.preferThreeD;

/**
 * (Re)initialize the gate from a capability snapshot. Called once at boot
 * (see platform/boot.ts). Pass an explicit snapshot in tests; defaults to a
 * fresh device detection. Resets manual representation preferences.
 */
export function initRenderMode(snapshot: CapabilitySnapshot = detectCapabilities()): LensRenderState {
  store.setSnapshot(snapshot);
  store.setPreferTwoDSignal(false);
  store.setPreferThreeDSignal(false);
  const next = computeState(snapshot, false);
  store.setState(next);
  return next;
}

/**
 * Apply a live reduced-motion change to the shared lens gate. Turning the
 * kill-switch on immediately forces 2D/static, causing mounted 3D branches to
 * unmount and cancel their render workers. The last verified probe is retained
 * so 3D may become eligible again only after motion is allowed.
 */
export function setReducedMotion(reducedMotion: boolean): LensRenderState {
  const snapshot: CapabilitySnapshot = {
    ...store.snapshot(),
    prefersReducedMotion: reducedMotion,
  };
  store.setSnapshot(snapshot);
  const next = computeState(snapshot, store.preferTwoD(), store.preferThreeD());
  store.setState(next);
  return next;
}

/**
 * Apply the result of an on-device §11.3 G2 probe. This is the ONLY path that
 * can flip a lens into 3D: the new snapshot carries the probe, and the gate
 * re-evaluates. A `null` probe (WebGL absent / probe could not run) keeps 2D.
 * A manual 2D preference still wins.
 */
export function applyProbeResult(probe: ProbeResult | null): LensRenderState {
  const snapshot: CapabilitySnapshot = { ...store.snapshot(), probe };
  store.setSnapshot(snapshot);
  const next = computeState(snapshot, store.preferTwoD(), store.preferThreeD());
  store.setState(next);
  return next;
}

/**
 * Force the gate back to 2D (auto-degrade hook, design.md §5.4). Lens surfaces
 * or a load monitor call this under heavy model load; the lens reacts and tears
 * down its 3D scene. Idempotent.
 */
export function degradeToTwoD(reason = "auto-degraded to 2D under load"): LensRenderState {
  const current = store.state();
  if (!current.enable3D && current.mode === "2d") return current;
  // Drop the probe and explicit 3D preference so the decision stays 2D until
  // the user requests 3D again or a lens re-probes successfully.
  const snapshot: CapabilitySnapshot = { ...store.snapshot(), probe: null };
  store.setSnapshot(snapshot);
  store.setPreferThreeDSignal(false);
  const next: LensRenderState = {
    ...computeState(snapshot, store.preferTwoD()),
    mode: "2d",
    enable3D: false,
    reason,
    snapshot,
  };
  store.setState(next);
  return next;
}

/**
 * Set (or clear) the user's manual 2D preference (Req 5.5 / 17.5 low-power /
 * accessibility toggle). When `true`, the lens shows its 2D representation even
 * on a 3D-capable device; when `false`, the gate returns to the automatic
 * capability-driven decision. Idempotent.
 */
export function setPreferTwoD(prefer: boolean): LensRenderState {
  store.setPreferTwoDSignal(prefer);
  if (prefer) store.setPreferThreeDSignal(false);
  const next = computeState(store.snapshot(), prefer, store.preferThreeD());
  store.setState(next);
  return next;
}

/**
 * Explicitly request 3D. This bypasses only the automatic performance probe;
 * WebGL support and reduced-motion remain hard constraints.
 */
export function setPreferThreeD(): LensRenderState {
  store.setPreferTwoDSignal(false);
  store.setPreferThreeDSignal(true);
  const next = computeState(store.snapshot(), false, true);
  store.setState(next);
  return next;
}

/**
 * Reactive hook for components. Returns the live accessor so Solid tracks reads
 * and lenses re-render when the mode changes (e.g. after a probe or degrade).
 */
export function useLensRenderMode(): Accessor<LensRenderState> {
  return store.state;
}
