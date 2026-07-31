/**
 * memory/layout/pointerTargets — Coarse-pointer touch target size enforcement.
 *
 * Pure TypeScript module — no DOM, no side effects.
 *
 * Enforces the ≥44×44px minimum coarse-pointer target size required by
 * WCAG 2.5.5 (Level AA) and platform touch guidelines.
 *
 * IDs: MGR-022, MGR-026; MGD-026; MG-H10, MG-H13.
 */

// ─── Constants ────────────────────────────────────────────────────────────────

/** Minimum dimension in pixels for a coarse-pointer (touch) target. */
export const MIN_COARSE_TARGET_PX = 44;

// ─── Functions ────────────────────────────────────────────────────────────────

/**
 * Returns true when both dimensions meet the minimum coarse-pointer target size.
 * Both width and height must be ≥44px.
 */
export function meetsCoarseTarget(widthPx: number, heightPx: number): boolean {
  return widthPx >= MIN_COARSE_TARGET_PX && heightPx >= MIN_COARSE_TARGET_PX;
}

/**
 * Returns the coarse-pointer safe dimension: max(current, 44).
 * Use this to expand an element's hit target to meet the minimum.
 */
export function getCoarseTargetDimension(current: number): number {
  return Math.max(current, MIN_COARSE_TARGET_PX);
}
