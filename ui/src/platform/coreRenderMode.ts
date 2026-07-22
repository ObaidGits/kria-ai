/**
 * platform/coreRenderMode — the homepage Core-3D capability gate + runtime
 * render-mode resolver (task 0.3).
 *
 * Design source: design.md §13.3 (GPU budgeting & Linux — "Capability gate at
 * boot: reuse prototype gates G1/G8 + a Core-3D gate; probe WebKitGTK/Wayland/
 * GPU; enable 3D Core only if it passes; else 2D path"), §13.4 (graceful
 * degradation), §11.5 (budget & degradation), §4.3 (3D upgrade capability-gated).
 * Requirements 17.4 (reduced-motion / kill-switch → static frames), 20.2
 * (pass the Linux gates before enabling the 3D Core), 20.3 (permanent, first-
 * class 2D degrade path auto-selected under reduced-motion, no-WebGL, low-power,
 * or failed gates), 20.4 (measurable degradation triggers).
 *
 * ── Relationship to the LENS gate (platform/renderMode.ts) ───────────────────
 * `renderMode.ts` is the gate for the Memory graph / Capability constellation
 * *lenses* (§5.4, G2). THIS module is the gate for the homepage *Core* — a
 * different surface (a single low-poly WebGL Core, §4.3 / §13.2) with its own
 * Core-3D performance gate and its own runtime degrade triggers. It reuses the
 * shared capability detection (`capabilities.ts`) rather than duplicating it.
 *
 * ── The contract for the 3D Core (task 7.1 consumes this) ────────────────────
 * The 3D Core MUST:
 *   1. Treat 2D as the DEFAULT and PERMANENT path. 2D is a first-class supported
 *      mode, never a fallback afterthought (design §0, Req 20.3). Never assume 3D.
 *   2. Read `useCoreRenderMode()` (reactive) — or the one-shot `coreRenderMode()`
 *      snapshot — and only mount the WebGL Core when `enable3D === true`.
 *   3. Re-evaluate reactively: whenever ANY degrade trigger fires
 *      (reduced-motion, no-WebGL, low-power, failed gate, runtime frame-drop)
 *      the resolver flips to 2D and the Core must tear its context down and
 *      render the 2D CSS/SVG Core — without a reload (Req 20.3 / 20.4).
 *
 * This module is 2D-first: `auto` resolves to 3D ONLY when the Core-3D gate has
 * passed AND no degrade trigger is active. Any failing/missing precondition
 * resolves to "2d" — an ACCEPTED outcome per §11.2/§20.3, not a failure.
 */
import { createRoot, createSignal, type Accessor } from "solid-js";
import {
  detectCapabilities,
  type CapabilitySnapshot,
  type ProbeResult,
} from "./capabilities";

/**
 * The stored Core render-mode preference (HomeUiState.renderMode, design §13.1):
 *   - "2d"   — force the permanent 2D path (explicit preference / low power).
 *   - "3d"   — request the 3D Core (bypasses only the automatic Core-3D perf
 *              gate; hard + runtime-safety triggers still force 2D).
 *   - "auto" — let the gate decide: 3D only when it passes and nothing degrades.
 */
export type CoreRenderMode = "2d" | "3d" | "auto";

/** The concrete mode the Core actually renders in. Always a real surface. */
export type CoreResolvedMode = "2d" | "3d";

/**
 * The degrade triggers that force the first-class 2D path (design §13.4 /
 * §11.5, Req 20.3 / 20.4). Any active trigger defaults the Core to 2D.
 */
export type CoreDegradeTrigger =
  | "reduced-motion"
  | "no-webgl"
  | "low-power"
  | "failed-gate"
  | "frame-drop";

/** Inputs the resolver evaluates. All are independently testable/mockable. */
export interface CoreRenderInputs {
  /** Stored preference (HomeUiState.renderMode). */
  preference: CoreRenderMode;
  /** Device capability snapshot (WebGL tier + reduced-motion, etc.). */
  snapshot: CapabilitySnapshot;
  /** Whether the on-device Core-3D performance gate has run AND passed. */
  gatePassed: boolean;
  /** Low-power / battery-saver posture reported by the platform. */
  lowPower: boolean;
  /** Sustained runtime frame-drop detected by the Core's frame monitor. */
  frameDrop: boolean;
}

/** The resolved render decision consumed by the Core + diagnostics. */
export interface CoreRenderDecision {
  /** The stored preference echoed back (2d | 3d | auto). */
  preference: CoreRenderMode;
  /** The concrete resolved mode. Always "2d" or "3d". */
  mode: CoreResolvedMode;
  /** Convenience: true iff `mode === "3d"`. The 3D Core mounts only when true. */
  enable3D: boolean;
  /** Every degrade trigger currently active (may be empty). */
  triggers: CoreDegradeTrigger[];
  /** True when 2D was chosen because a trigger fired (vs. by preference). */
  degraded: boolean;
  /** Human-readable reason, surfaced in diagnostics / a11y notices. */
  reason: string;
}

/**
 * The Core-3D gate fps floor (design §11.5: Core capped 30–45 fps). A device
 * that cannot sustain this at the Core's target size fails the gate → 2D.
 */
export const CORE_3D_MIN_SUSTAINED_FPS = 30;

// --- the Core-3D capability gate --------------------------------------------

/**
 * Evaluate a Core-3D on-device probe against the sustained-fps floor
 * (design §13.3 "Core-3D gate: sustained fps at target size"). The probe reuses
 * the shared WebGL frame-timing machinery (prototypes/gateProbes.ts). A `null`
 * probe (WebGL absent / probe never ran) fails the gate — 2D is the default
 * until a passing probe is recorded.
 */
export function coreGatePasses(probe: ProbeResult | null): boolean {
  if (!probe) return false;
  return probe.interactionFps >= CORE_3D_MIN_SUSTAINED_FPS && probe.idleQuiet;
}

// --- the resolver -----------------------------------------------------------

/**
 * Collect every degrade trigger currently active for the given inputs
 * (design §13.4 / §11.5, Req 20.3 / 20.4). Pure + order-stable so diagnostics
 * and tests can assert the exact set.
 */
export function activeCoreDegradeTriggers(inputs: CoreRenderInputs): CoreDegradeTrigger[] {
  const triggers: CoreDegradeTrigger[] = [];
  if (inputs.snapshot.prefersReducedMotion) triggers.push("reduced-motion");
  if (!inputs.snapshot.hasWebGL) triggers.push("no-webgl");
  if (inputs.lowPower) triggers.push("low-power");
  if (!inputs.gatePassed) triggers.push("failed-gate");
  if (inputs.frameDrop) triggers.push("frame-drop");
  return triggers;
}

/**
 * Triggers that force 2D even when the user has EXPLICITLY requested 3D.
 * Explicit "3d" bypasses ONLY the automatic performance gate ("failed-gate") —
 * mirroring the lens gate's manual-3D semantics — but hard capability limits
 * (reduced-motion, no-WebGL) and runtime-safety degrades (low-power, frame-drop)
 * always win, because rendering 3D there would break accessibility or budget.
 */
const HARD_TRIGGERS: ReadonlySet<CoreDegradeTrigger> = new Set([
  "reduced-motion",
  "no-webgl",
  "low-power",
  "frame-drop",
]);

function twoD(
  preference: CoreRenderMode,
  triggers: CoreDegradeTrigger[],
  degraded: boolean,
  reason: string,
): CoreRenderDecision {
  return { preference, mode: "2d", enable3D: false, triggers, degraded, reason };
}

function threeD(
  preference: CoreRenderMode,
  triggers: CoreDegradeTrigger[],
  reason: string,
): CoreRenderDecision {
  return { preference, mode: "3d", enable3D: true, triggers, degraded: false, reason };
}

/**
 * The runtime render-mode resolver (task 0.3). Resolves a preference
 * (2d | 3d | auto) + capabilities + degrade triggers into a concrete Core
 * render decision. 2D is the first-class permanent path: the resolver defaults
 * to 2D whenever a trigger fires (Req 20.3 / 20.4).
 */
export function resolveCoreRenderMode(inputs: CoreRenderInputs): CoreRenderDecision {
  const { preference } = inputs;
  const triggers = activeCoreDegradeTriggers(inputs);

  // Explicit 2D: the permanent, first-class path. Chosen by preference, not a
  // degrade — but active triggers are still reported for diagnostics.
  if (preference === "2d") {
    return twoD(preference, triggers, false, "2D: explicit preference (first-class permanent path)");
  }

  if (preference === "3d") {
    const hard = triggers.filter((t) => HARD_TRIGGERS.has(t));
    if (hard.length > 0) {
      return twoD(
        preference,
        triggers,
        true,
        `2D: degrade trigger(s) override explicit 3D — ${hard.join(", ")}`,
      );
    }
    return threeD(preference, triggers, "3D: explicit preference (Core-3D gate bypassed by request)");
  }

  // auto: 3D only when nothing degrades (which includes a passing Core-3D gate).
  if (triggers.length > 0) {
    return twoD(preference, triggers, true, `2D: auto-degraded — ${triggers.join(", ")}`);
  }
  return threeD(preference, triggers, "3D: auto — Core-3D gate passed, no degrade triggers");
}

// --- the reactive store -----------------------------------------------------

/**
 * The global Core render-mode store lives in a detached reactive root so it
 * exists once for the whole app (like the sibling lens `renderMode.ts` store)
 * and is never disposed with a component tree. It holds the resolver inputs;
 * the public decision is derived from them and re-resolved on every mutation.
 */
const store = createRoot(() => {
  const initialSnapshot = detectCapabilities();
  const [preference, setPreferenceSignal] = createSignal<CoreRenderMode>("auto");
  const [snapshot, setSnapshotSignal] = createSignal<CapabilitySnapshot>(initialSnapshot);
  const [gatePassed, setGatePassedSignal] = createSignal(false);
  const [lowPower, setLowPowerSignal] = createSignal(false);
  const [frameDrop, setFrameDropSignal] = createSignal(false);

  const decision: Accessor<CoreRenderDecision> = () =>
    resolveCoreRenderMode({
      preference: preference(),
      snapshot: snapshot(),
      gatePassed: gatePassed(),
      lowPower: lowPower(),
      frameDrop: frameDrop(),
    });

  return {
    preference,
    setPreferenceSignal,
    snapshot,
    setSnapshotSignal,
    gatePassed,
    setGatePassedSignal,
    lowPower,
    setLowPowerSignal,
    frameDrop,
    setFrameDropSignal,
    decision,
  };
});

/** Reactive accessor for the current Core render decision. */
export const coreRenderMode: Accessor<CoreRenderDecision> = store.decision;

/** Reactive accessor for the stored preference. */
export const coreRenderPreference: Accessor<CoreRenderMode> = store.preference;

/**
 * (Re)initialize the Core gate from a capability snapshot + preference. Called
 * once at boot (platform/boot.ts). Pass an explicit snapshot in tests; defaults
 * to a fresh device detection. Resets the gate/low-power/frame-drop signals to
 * their safe (2D-default) posture.
 */
export function initCoreRenderMode(
  snapshot: CapabilitySnapshot = detectCapabilities(),
  preference: CoreRenderMode = "auto",
): CoreRenderDecision {
  store.setSnapshotSignal(snapshot);
  store.setPreferenceSignal(preference);
  store.setGatePassedSignal(false);
  store.setLowPowerSignal(false);
  store.setFrameDropSignal(false);
  return store.decision();
}

/** Set the stored Core render preference (2d | 3d | auto). Idempotent. */
export function setCoreRenderPreference(preference: CoreRenderMode): CoreRenderDecision {
  store.setPreferenceSignal(preference);
  return store.decision();
}

/**
 * Apply the result of an on-device Core-3D gate probe (design §13.3). This is
 * the only path that can satisfy the "failed-gate" trigger for `auto`. Passing
 * a `null` probe (WebGL absent / probe could not run) keeps the gate failed.
 */
export function applyCoreGateResult(probe: ProbeResult | null): CoreRenderDecision {
  store.setGatePassedSignal(coreGatePasses(probe));
  return store.decision();
}

/**
 * Report the Core-3D gate pass/fail directly (when the caller has already
 * evaluated a probe, e.g. the Linux-matrix gate runner, task 7.3).
 */
export function setCoreGatePassed(passed: boolean): CoreRenderDecision {
  store.setGatePassedSignal(passed);
  return store.decision();
}

/**
 * Apply a live reduced-motion change (OS media query or the global
 * kill-switch, Req 17.4). Turning it on forces the Core to 2D/static, causing a
 * mounted 3D Core to tear down. Mirrors the lens gate's `setReducedMotion`.
 */
export function setCoreReducedMotion(reducedMotion: boolean): CoreRenderDecision {
  store.setSnapshotSignal({ ...store.snapshot(), prefersReducedMotion: reducedMotion });
  return store.decision();
}

/** Report the low-power / battery-saver posture (Req 20.3 / 20.4). */
export function setCoreLowPower(lowPower: boolean): CoreRenderDecision {
  store.setLowPowerSignal(lowPower);
  return store.decision();
}

/**
 * Report a sustained runtime frame-drop from the Core's frame monitor
 * (design §11.5, Req 20.4). `true` auto-degrades to 2D; `false` clears it (the
 * gate must still pass for `auto` to return to 3D).
 */
export function reportCoreFrameDrop(active: boolean): CoreRenderDecision {
  store.setFrameDropSignal(active);
  return store.decision();
}

/** Reactive hook for components. Returns the live decision accessor. */
export function useCoreRenderMode(): Accessor<CoreRenderDecision> {
  return store.decision;
}
