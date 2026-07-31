/**
 * memory/layout/visualAliases — Redundant semantic visual aliases for scene item kinds.
 *
 * Pure TypeScript module — no DOM, no side effects.
 *
 * Defines shape, color token, icon ID, text fallback, and authority indicator
 * for every SceneItemKind. All encodings are redundant (shape + color +
 * icon + text) so that no meaning depends on color alone.
 *
 * Design invariants:
 *   - Every SceneItemKind has an entry — no kind is implicitly hidden.
 *   - textFallback is always present (plain text abbreviation, never unicode).
 *   - colorToken always references a CSS custom property (starts with '--color-').
 *   - iconId is a text identifier, never a unicode glyph or emoji.
 *   - navigation-container has authorityIndicator: 'none' (it is a group, not an entity).
 *   - No hidden policy indicators — absence of an alias for a kind must never
 *     imply hidden scope; all kinds are listed.
 *
 * IDs: MGR-013–016, MGR-022, MGR-026; MGD-026, MGD-046; MG-H10–H15.
 */

import type { SceneItemKind } from '../scene/semanticScene';

// ─── Types ────────────────────────────────────────────────────────────────────

/** Visual shape used to render the item in a map/canvas. */
export type VisualShape = 'circle' | 'rect' | 'diamond' | 'hexagon' | 'triangle' | 'line';

/**
 * How authority class is indicated visually.
 *   badge  — a small badge overlaid on the shape
 *   line   — a colored stroke/border on the shape
 *   none   — no authority indicator (group containers)
 */
export type AuthorityIndicator = 'badge' | 'line' | 'none';

/** Complete visual alias for one SceneItemKind. */
export interface VisualAlias {
  /** Shape used in the map renderer. */
  shape: VisualShape;
  /** CSS custom property for the item's color, e.g. '--color-entity'. */
  colorToken: string;
  /** Text icon identifier (not a unicode char — safe for any renderer). */
  iconId: string;
  /** Plain text abbreviation always present (used when icon/color unavailable). */
  textFallback: string;
  /** How authority class is visually indicated on this kind. */
  authorityIndicator: AuthorityIndicator;
}

// ─── Alias table ──────────────────────────────────────────────────────────────

/**
 * Complete visual alias table for all ten SceneItemKind values.
 *
 * No hidden policy indicators: every kind has an explicit entry. Absence from
 * this table would be a bug, not an intended hidden-scope signal.
 */
export const VISUAL_ALIASES: Record<SceneItemKind, VisualAlias> = {
  entity: {
    shape: 'circle',
    colorToken: '--color-entity',
    iconId: 'icon-entity',
    textFallback: 'ENT',
    authorityIndicator: 'badge',
  },
  memory: {
    shape: 'rect',
    colorToken: '--color-memory',
    iconId: 'icon-memory',
    textFallback: 'MEM',
    authorityIndicator: 'badge',
  },
  evidence: {
    shape: 'diamond',
    colorToken: '--color-success-solid',
    iconId: 'icon-evidence',
    textFallback: 'EVD',
    authorityIndicator: 'badge',
  },
  aggregate: {
    shape: 'hexagon',
    colorToken: '--color-accent-secondary',
    iconId: 'icon-aggregate',
    textFallback: 'AGG',
    authorityIndicator: 'badge',
  },
  relation: {
    shape: 'line',
    colorToken: '--color-relation',
    iconId: 'icon-relation',
    textFallback: 'REL',
    authorityIndicator: 'line',
  },
  goal: {
    shape: 'diamond',
    colorToken: '--color-goal',
    iconId: 'icon-goal',
    textFallback: 'GOAL',
    authorityIndicator: 'badge',
  },
  source: {
    shape: 'hexagon',
    colorToken: '--color-source',
    iconId: 'icon-source',
    textFallback: 'SRC',
    authorityIndicator: 'badge',
  },
  episode: {
    shape: 'rect',
    colorToken: '--color-episode',
    iconId: 'icon-episode',
    textFallback: 'EP',
    authorityIndicator: 'badge',
  },
  summary: {
    shape: 'rect',
    colorToken: '--color-summary',
    iconId: 'icon-summary',
    textFallback: 'SUM',
    authorityIndicator: 'badge',
  },
  skill: {
    shape: 'hexagon',
    colorToken: '--color-skill',
    iconId: 'icon-skill',
    textFallback: 'SKL',
    authorityIndicator: 'badge',
  },
  rule: {
    shape: 'triangle',
    colorToken: '--color-rule',
    iconId: 'icon-rule',
    textFallback: 'RULE',
    authorityIndicator: 'badge',
  },
  'navigation-container': {
    shape: 'rect',
    colorToken: '--color-navigation-container',
    iconId: 'icon-navigation-container',
    textFallback: 'GRP',
    // Navigation containers are group containers, not entities —
    // no authority indicator applies.
    authorityIndicator: 'none',
  },
};

// ─── Accessor ─────────────────────────────────────────────────────────────────

/**
 * Returns the VisualAlias for the given SceneItemKind.
 * Always returns a defined alias — every kind has an entry.
 */
export function getVisualAlias(kind: SceneItemKind): VisualAlias {
  return VISUAL_ALIASES[kind];
}
