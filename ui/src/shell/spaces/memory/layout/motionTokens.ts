/**
 * memory/layout/motionTokens — Motion duration constants and validators.
 *
 * Pure TypeScript module — no DOM, no side effects.
 *
 * Defines exact animation durations for all memory UI transitions.
 * Hard max is 400ms; reduced motion must use immediate/≤80ms crossfade.
 *
 * No ambient animation, particles, glow, breathing, orbit, or edge flow
 * are permitted — this table covers only purposeful transitions.
 *
 * IDs: MGR-013–016, MGR-022; MGD-026, MGD-046; MG-H15, MG-M24–M26.
 */

// ─── Motion token table ───────────────────────────────────────────────────────

/**
 * Exact motion durations in milliseconds for each animation context.
 *
 *   FOCUS_MS           — keyboard focus ring appearance: 80ms linear
 *   SELECTION_MS       — item selection feedback: 120ms ease-out
 *   INSPECTOR_MS       — inspector panel open/close: 180ms
 *   CAMERA_MS          — camera pan/zoom: 220ms cubic-bezier
 *   SCENE_MS           — scene load/swap: 300ms
 *   TEMPORAL_MS        — temporal lane transitions: 320ms
 *   INFERRED_STORED_MS — inferred→stored state change: 240ms once
 *   STATUS_MS          — status/badge updates: ≤120ms
 *   HARD_MAX_MS        — absolute maximum for any single transition: 400ms
 *   REDUCED_MOTION_MAX_MS — maximum under prefers-reduced-motion: 80ms crossfade
 */
export const MOTION = {
  FOCUS_MS: 80,
  SELECTION_MS: 120,
  INSPECTOR_MS: 180,
  CAMERA_MS: 220,
  SCENE_MS: 300,
  TEMPORAL_MS: 320,
  INFERRED_STORED_MS: 240,
  STATUS_MS: 120,
  HARD_MAX_MS: 400,
  REDUCED_MOTION_MAX_MS: 80,
} as const;

// ─── Type helpers ─────────────────────────────────────────────────────────────

/** Union of all motion token key names. */
export type MotionToken = keyof typeof MOTION;

// ─── Functions ────────────────────────────────────────────────────────────────

/**
 * Returns the duration in milliseconds for the given motion token.
 */
export function getMotionDuration(token: MotionToken): number {
  return MOTION[token];
}

/**
 * Returns true when the given duration is within the hard maximum (≤400ms).
 */
export function isWithinHardMax(durationMs: number): boolean {
  return durationMs <= MOTION.HARD_MAX_MS;
}

/**
 * Returns true when the given duration is within the reduced-motion maximum (≤80ms).
 * Use this when `prefers-reduced-motion: reduce` is active.
 */
export function isWithinReducedMotionMax(durationMs: number): boolean {
  return durationMs <= MOTION.REDUCED_MOTION_MAX_MS;
}
