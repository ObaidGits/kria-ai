/**
 * memory/layout/breakpoints — Responsive layout breakpoints and dimension constants.
 *
 * Pure TypeScript module — no DOM, no side effects.
 *
 * Covers tasks 4.8.1 (≥1200px), 4.8.2 (800–1199px), and 4.8.3 (<800px).
 *
 * Layout modes:
 *   large  (≥1200px): 240px nav + flexible workspace min 560px + 360px inspector
 *   medium (800–1199px): 72px rail + flexible workspace + 320px overlay inspector
 *   small  (<800px): single-column, full-height inspector sheet
 *
 * IDs: MGR-013–016, MGR-022, MGR-026; MGD-013–014, MGD-026, MGD-046;
 *      MG-H10–H15, MG-M01–M03, MG-M24–M26.
 */

// ─── Breakpoint constants ─────────────────────────────────────────────────────

export const BREAKPOINT_LARGE = 1200;
export const BREAKPOINT_MEDIUM = 800;

// ─── Dimension constants ──────────────────────────────────────────────────────

export const NAV_WIDTH_LARGE = 240;
export const NAV_WIDTH_MEDIUM = 72;
export const WORKSPACE_MIN_WIDTH = 560;
export const INSPECTOR_WIDTH_LARGE = 360;
export const INSPECTOR_WIDTH_MEDIUM = 320;

// ─── Layout mode ──────────────────────────────────────────────────────────────

/**
 * Layout mode derived from viewport width.
 *   large  — ≥1200px: full three-column layout
 *   medium — 800–1199px: rail + workspace + overlay inspector
 *   small  — <800px: single-column / sheet layout
 */
export type LayoutMode = 'large' | 'medium' | 'small';

/**
 * Returns the layout mode for the given viewport width.
 * Boundaries are inclusive on the lower end:
 *   ≥1200 → large, ≥800 → medium, <800 → small.
 */
export function getLayoutMode(viewportWidth: number): LayoutMode {
  if (viewportWidth >= BREAKPOINT_LARGE) return 'large';
  if (viewportWidth >= BREAKPOINT_MEDIUM) return 'medium';
  return 'small';
}

/**
 * Returns the navigation panel width in pixels for the given layout mode.
 *   large  → 240px full navigation panel
 *   medium → 72px icon rail
 *   small  → 0px (navigation collapses into sheet/overlay)
 */
export function getNavWidth(mode: LayoutMode): number {
  switch (mode) {
    case 'large': return NAV_WIDTH_LARGE;
    case 'medium': return NAV_WIDTH_MEDIUM;
    case 'small': return 0;
  }
}

/**
 * Returns the inspector panel width in pixels for the given layout mode.
 *   large  → 360px side inspector
 *   medium → 320px focus-managed overlay inspector
 *   small  → 0px (inspector renders as full-height sheet)
 */
export function getInspectorWidth(mode: LayoutMode): number {
  switch (mode) {
    case 'large': return INSPECTOR_WIDTH_LARGE;
    case 'medium': return INSPECTOR_WIDTH_MEDIUM;
    case 'small': return 0;
  }
}
