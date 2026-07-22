/**
 * platform/boot — one-shot platform initialization run at app boot.
 *
 * Establishes the Linux rendering baseline (design.md §11.2) on the DOM before
 * surfaces mount:
 *   - seeds the lens render-mode gate (renderMode.ts) from live capability
 *     detection (2D-default; 3D stays off until a lens runs its §11.3 G2 probe),
 *   - sets root data attributes that CSS keys off:
 *       data-blur="on|off"        aura-glass backdrop-filter vs solid fallback
 *       data-render-mode="2d|3d"  current lens default (diagnostics/telemetry)
 *       data-reduced-motion="on"  present when the user prefers reduced motion
 *
 * Aura-glass blur (design.md §4.3 / §11.2 / §11.3 G8) is applied ONLY when the
 * device actually supports backdrop-filter AND the user has not requested
 * reduced motion; otherwise floating surfaces degrade to the solid-translucent
 * fallback (see kit/floating.css). The visual language must survive without
 * blur on WebKitGTK.
 */
import { detectCapabilities, type CapabilitySnapshot } from "./capabilities";
import {
  createReducedMotionController,
  type ReducedMotionController,
} from "./motion";
import { initRenderMode, setReducedMotion } from "./renderMode";
import {
  applyCoreGateResult,
  initCoreRenderMode,
  setCoreReducedMotion,
  type CoreRenderDecision,
} from "./coreRenderMode";
import type { ProbeResult } from "./capabilities";
import { runCoreGateProbe } from "../prototypes/gateProbes";

/** Blur treatment applied to floating surfaces at boot. */
export type BlurTreatment = "on" | "off";

let motionController: ReducedMotionController | null = null;
let currentSnapshot: CapabilitySnapshot | null = null;

/**
 * Decide whether aura-glass blur is enabled. Blur requires real
 * backdrop-filter support AND no reduced-motion preference (blur is a
 * compositing cost we don't pay when motion is minimized). Pure + testable.
 */
export function decideBlurTreatment(caps: CapabilitySnapshot): BlurTreatment {
  return caps.supportsBackdropFilter && !caps.prefersReducedMotion ? "on" : "off";
}

/** Apply platform attributes used by CSS and diagnostics. */
export function applyBootAttributes(doc: Document, caps: CapabilitySnapshot): void {
  const root = doc.documentElement;
  if (!root) return;
  root.setAttribute("data-blur", decideBlurTreatment(caps));
  root.setAttribute("data-render-mode", "2d");
  if (caps.prefersReducedMotion) root.setAttribute("data-reduced-motion", "on");
  else root.removeAttribute("data-reduced-motion");
}

function applyReducedMotion(reducedMotion: boolean, doc?: Document): void {
  if (!currentSnapshot) return;
  currentSnapshot = { ...currentSnapshot, prefersReducedMotion: reducedMotion };
  const state = setReducedMotion(reducedMotion);
  // Keep the homepage Core gate in sync with the lens gate: reduced-motion
  // forces the Core to its 2D/static path too (Req 17.4 / 20.3).
  setCoreReducedMotion(reducedMotion);
  if (doc) {
    applyBootAttributes(doc, currentSnapshot);
    doc.documentElement.setAttribute("data-render-mode", state.mode);
  }
}

/** Force all motion off; false returns control to the OS preference. */
export function setGlobalReducedMotion(enabled: boolean): void {
  motionController?.setKillSwitch(enabled);
}

/**
 * Run the on-device Core-3D gate probe (design §13.3) and feed the result into
 * the render-mode resolver. This is the ONLY path that satisfies the resolver's
 * "failed-gate" trigger for `auto`: the homepage Core stays on its permanent 2D
 * path until a probe passes (Req 20.2 / 20.3). The probe returns `null` when
 * WebGL is unavailable (WebKitGTK software-raster / jsdom) → the gate stays
 * failed → 2D, an ACCEPTED outcome. `probeRunner` is injectable for tests.
 */
export async function runCoreGateAndApply(
  probeRunner: () => Promise<ProbeResult | null> = runCoreGateProbe,
): Promise<CoreRenderDecision> {
  let probe: ProbeResult | null = null;
  try {
    probe = await probeRunner();
  } catch {
    probe = null; // any probe failure = failed gate = first-class 2D path
  }
  return applyCoreGateResult(probe);
}

/** Remove platform listeners. Intended for tests/window teardown. */
export function disposePlatform(): void {
  motionController?.dispose();
  motionController = null;
  currentSnapshot = null;
}

/** Initialize capabilities and live reduced-motion synchronization. */
export function initPlatform(
  doc: Document | undefined = globalThis.document,
  win: Window | undefined = globalThis.window,
): CapabilitySnapshot {
  disposePlatform();
  const caps = detectCapabilities();
  currentSnapshot = caps;
  initRenderMode(caps);
  // Seed the homepage Core-3D gate/resolver from the same snapshot. 2D-first:
  // the Core stays on its permanent 2D path until a Core-3D gate probe passes
  // (design §13.3 / §13.4, Req 20.3). Task 7.x runs the probe + wires degrade.
  initCoreRenderMode(caps);
  // Kick off the async Core-3D gate probe only when it could plausibly pass
  // (WebGL present + motion allowed). On pass the resolver flips `auto` to 3D;
  // otherwise the Core stays on its permanent 2D path (Req 20.2 / 20.3). Fire-
  // and-forget: the 2D Core renders immediately and the 3D Core (CoreShell3D)
  // reactively upgrades if/when the gate passes — no reload.
  if (caps.hasWebGL && !caps.prefersReducedMotion) {
    void runCoreGateAndApply().catch(() => {});
  }
  if (doc) applyBootAttributes(doc, caps);
  motionController = createReducedMotionController({
    document: doc,
    window: win,
    initialReducedMotion: caps.prefersReducedMotion,
    onChange: (reduced) => applyReducedMotion(reduced, doc),
  });
  return caps;
}
