/**
 * platform/capabilities — runtime capability detection + the 3D-enable gate.
 *
 * Design source: design.md §11.2 (WebKitGTK Correction) and §11.3 gate G2.
 *
 * KRIA is 2D-first on Linux/WebKitGTK. The 2D Memory graph and Capability
 * constellation are the DEFAULT; 3D is an opt-in enhancement that is enabled
 * ONLY when BOTH conditions hold on the current device:
 *   (a) capability detection passes (WebGL present, not reduced-motion), AND
 *   (b) an on-device performance probe (§11.3 G2) reports a viable result.
 *
 * This module exposes:
 *   - pure detection functions (WebGL / reduced-motion / backdrop-filter),
 *   - a snapshot aggregator,
 *   - the `decideRenderMode` / `shouldEnable3D` gate consumed by task 0.6.
 *
 * Every function accepts optional injected globals so the logic is unit-testable
 * under jsdom (which lacks WebGL, matchMedia, and CSS.supports).
 */

/** WebGL support tiers detected on the device. */
export type WebGLTier = "none" | "webgl1" | "webgl2";

/** Result of the §11.3 G2 on-device 3D viability probe, if one has run. */
export interface ProbeResult {
  /** Median interaction frame time in ms while the scene animates. */
  interactionFrameMs: number;
  /** Estimated frames-per-second derived from the interaction frame time. */
  interactionFps: number;
  /** Whether the scene reached ~0 cost when frozen/idle (frame loop stopped). */
  idleQuiet: boolean;
  /** Node count exercised by the probe. */
  nodeCount: number;
}

/** Aggregated capability snapshot for the current device. */
export interface CapabilitySnapshot {
  webglTier: WebGLTier;
  hasWebGL: boolean;
  prefersReducedMotion: boolean;
  supportsBackdropFilter: boolean;
  /** Probe result if a G2 probe has been run this session, else null. */
  probe: ProbeResult | null;
}

/** Chosen lens render mode for graph/constellation surfaces. */
export type RenderMode = "2d" | "3d";

/** Explained gate decision for the 3D-enable question (§11.3 G2). */
export interface RenderModeDecision {
  mode: RenderMode;
  /** True only when 3D is enabled. */
  enable3D: boolean;
  /** Human-readable reason, surfaced in diagnostics / a11y notice. */
  reason: string;
}

/** G2 viability thresholds (design.md §11.3): interaction ≥30 fps AND idle ~0. */
export const G2_MIN_INTERACTION_FPS = 30;

// --- detection primitives ---------------------------------------------------

/**
 * Detect the WebGL tier available on the device. Attempts a real context on a
 * throwaway canvas. Returns "none" when WebGL is unavailable (the common
 * WebKitGTK/software-rasterizer outcome), "webgl2" or "webgl1" otherwise.
 */
export function detectWebGLTier(doc: Document | undefined = globalThis.document): WebGLTier {
  if (!doc || typeof doc.createElement !== "function") return "none";
  let canvas: HTMLCanvasElement;
  try {
    canvas = doc.createElement("canvas");
  } catch {
    return "none";
  }
  if (typeof canvas.getContext !== "function") return "none";
  try {
    if (canvas.getContext("webgl2")) return "webgl2";
  } catch {
    /* fall through to webgl1 attempt */
  }
  try {
    if (canvas.getContext("webgl") || canvas.getContext("experimental-webgl")) {
      return "webgl1";
    }
  } catch {
    /* no webgl */
  }
  return "none";
}

/**
 * Detect whether the user has requested reduced motion. Defaults to `true`
 * (the safe, motion-off posture) when matchMedia is unavailable.
 */
export function detectReducedMotion(win: Window | undefined = globalThis.window): boolean {
  if (!win || typeof win.matchMedia !== "function") return true;
  try {
    return win.matchMedia("(prefers-reduced-motion: reduce)").matches;
  } catch {
    return true;
  }
}

/**
 * Detect backdrop-filter (aura-glass blur) support. Checks both the standard
 * and the `-webkit-` prefixed property (WebKitGTK). Returns false when the
 * CSS.supports API is unavailable.
 */
export function detectBackdropFilter(
  cssApi: typeof CSS | undefined = typeof CSS !== "undefined" ? CSS : undefined,
): boolean {
  if (!cssApi || typeof cssApi.supports !== "function") return false;
  try {
    return (
      cssApi.supports("backdrop-filter", "blur(1px)") ||
      cssApi.supports("-webkit-backdrop-filter", "blur(1px)")
    );
  } catch {
    return false;
  }
}

// --- aggregation ------------------------------------------------------------

/** Build a full capability snapshot for the current device. */
export function detectCapabilities(probe: ProbeResult | null = null): CapabilitySnapshot {
  const webglTier = detectWebGLTier();
  return {
    webglTier,
    hasWebGL: webglTier !== "none",
    prefersReducedMotion: detectReducedMotion(),
    supportsBackdropFilter: detectBackdropFilter(),
    probe,
  };
}

// --- the gate ---------------------------------------------------------------

/**
 * Evaluate a G2 probe result against the viability thresholds (§11.3):
 * interaction ≥30 fps AND idle ~0 (frame loop actually stopped).
 */
export function probePasses(probe: ProbeResult): boolean {
  return probe.interactionFps >= G2_MIN_INTERACTION_FPS && probe.idleQuiet;
}

/**
 * The 3D-enable gate (design.md §11.2 / §11.3 G2). 2D is the DEFAULT; 3D is
 * enabled only when capability detection passes AND a G2 probe has run and
 * passed. Any failing/missing precondition resolves to "2d" — which per §11.2
 * is an ACCEPTED outcome, not a failure.
 */
export function decideRenderMode(snapshot: CapabilitySnapshot): RenderModeDecision {
  if (snapshot.prefersReducedMotion) {
    return { mode: "2d", enable3D: false, reason: "reduced-motion: 2D default (static)" };
  }
  if (!snapshot.hasWebGL) {
    return { mode: "2d", enable3D: false, reason: "no WebGL context: 2D default" };
  }
  if (!snapshot.probe) {
    return {
      mode: "2d",
      enable3D: false,
      reason: "3D viability probe not yet run: 2D default until G2 passes",
    };
  }
  if (!probePasses(snapshot.probe)) {
    return {
      mode: "2d",
      enable3D: false,
      reason: `G2 probe below threshold (${snapshot.probe.interactionFps.toFixed(
        0,
      )} fps, idleQuiet=${snapshot.probe.idleQuiet}): 2D default on this device`,
    };
  }
  return {
    mode: "3d",
    enable3D: true,
    reason: `G2 probe passed (${snapshot.probe.interactionFps.toFixed(0)} fps, idle quiet): 3D enabled`,
  };
}

/** Convenience boolean gate consumed by task 0.6 lens mounting. */
export function shouldEnable3D(snapshot: CapabilitySnapshot): boolean {
  return decideRenderMode(snapshot).enable3D;
}
