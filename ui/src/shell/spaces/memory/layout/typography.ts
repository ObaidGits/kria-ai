/**
 * memory/layout/typography — Typography accessibility constants and validators.
 *
 * Pure TypeScript module — no DOM, no side effects.
 *
 * Enforces minimum font sizes and focus ring widths per WCAG and the
 * Memory Control Center design contract.
 *
 * Minimums:
 *   - Body text: ≥14px
 *   - Map/canvas labels: ≥12px (minimum readable LOD)
 *   - Focus ring: ≥2px (WCAG 2.4.11 / AA)
 *
 * IDs: MGR-013–016, MGR-022; MGD-026, MGD-046; MG-H13–H14.
 */

// ─── Constants ────────────────────────────────────────────────────────────────

/** Minimum font size in px for body text (≥14px per design contract). */
export const MIN_BODY_FONT_SIZE_PX = 14;

/** Minimum font size in px for map/canvas item labels (≥12px readable LOD). */
export const MIN_MAP_LABEL_FONT_SIZE_PX = 12;

/** Minimum focus ring width in px (≥2px per WCAG 2.4.11 AA). */
export const MIN_FOCUS_RING_PX = 2;

// ─── Validators ───────────────────────────────────────────────────────────────

/**
 * Returns true when the given font size meets the minimum for the context.
 *   'body'      — must be ≥ MIN_BODY_FONT_SIZE_PX (14px)
 *   'map-label' — must be ≥ MIN_MAP_LABEL_FONT_SIZE_PX (12px)
 */
export function meetsFontSizeRequirement(
  sizePx: number,
  context: 'body' | 'map-label',
): boolean {
  switch (context) {
    case 'body':
      return sizePx >= MIN_BODY_FONT_SIZE_PX;
    case 'map-label':
      return sizePx >= MIN_MAP_LABEL_FONT_SIZE_PX;
  }
}

/**
 * Returns true when the given focus ring width meets the minimum (≥2px).
 */
export function meetsFocusRingRequirement(ringWidthPx: number): boolean {
  return ringWidthPx >= MIN_FOCUS_RING_PX;
}
