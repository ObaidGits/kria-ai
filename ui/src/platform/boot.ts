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
  if (doc) {
    applyBootAttributes(doc, currentSnapshot);
    doc.documentElement.setAttribute("data-render-mode", state.mode);
  }
}

/** Force all motion off; false returns control to the OS preference. */
export function setGlobalReducedMotion(enabled: boolean): void {
  motionController?.setKillSwitch(enabled);
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
  if (doc) applyBootAttributes(doc, caps);
  motionController = createReducedMotionController({
    document: doc,
    window: win,
    initialReducedMotion: caps.prefersReducedMotion,
    onChange: (reduced) => applyReducedMotion(reduced, doc),
  });
  return caps;
}
