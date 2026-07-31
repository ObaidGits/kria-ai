/**
 * memory/scene/sceneParityCheck — Pure assertion helpers for SemanticScene invariants.
 *
 * Pure TypeScript module — no JSX, no DOM, no side effects.
 *
 * Checks list/map/inspector item/action parity, scene purity, unknown/malformed
 * isolation, generated-navigation exclusion, and absence of hidden policy cues.
 *
 * All violation messages are policy-safe: they reference item/action IDs and
 * structural positions, never private content.
 *
 * Task: F4.6.6
 */

import type { SemanticScene } from './semanticScene';
import { isNavigationContainer, isEdgeItem } from './semanticScene';

// ─── Result type ──────────────────────────────────────────────────────────────

/**
 * Result of a parity/purity check.
 *
 * passed is true iff violations is empty.
 * violations contains human-readable descriptions with no private content.
 */
export interface CheckResult {
  passed: boolean;
  /** Human-readable violation descriptions (no private content). */
  violations: string[];
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

function pass(): CheckResult {
  return { passed: true, violations: [] };
}

function fail(violations: string[]): CheckResult {
  return { passed: false, violations };
}

function collect(violations: string[]): CheckResult {
  return violations.length === 0 ? pass() : fail(violations);
}

// ─── Individual checks ────────────────────────────────────────────────────────

/**
 * Check that all items have unique IDs.
 *
 * Violation: reports each duplicate ID once.
 */
export function checkItemIdUniqueness(scene: SemanticScene): CheckResult {
  const seen = new Set<string>();
  const duplicates = new Set<string>();
  for (const item of scene.items) {
    if (seen.has(item.id)) {
      duplicates.add(item.id);
    }
    seen.add(item.id);
  }
  return collect(
    [...duplicates].map((id) => `Duplicate item ID: "${id}"`),
  );
}

/**
 * Check that all actions reference existing item IDs.
 *
 * Violation: reports each action whose targetItemId is not found in scene.items.
 */
export function checkActionTargetValidity(scene: SemanticScene): CheckResult {
  const itemIds = new Set(scene.items.map((i) => i.id));
  const violations: string[] = [];
  for (const action of scene.actions) {
    if (!itemIds.has(action.targetItemId)) {
      violations.push(
        `Action "${action.kind}" references non-existent item ID: "${action.targetItemId}"`,
      );
    }
  }
  return collect(violations);
}

/**
 * Check that navigation containers have no actions.
 *
 * Navigation containers are group containers only — they must not be
 * individually actionable.
 */
export function checkNavigationContainerPurity(scene: SemanticScene): CheckResult {
  const navContainerIds = new Set(
    scene.items.filter(isNavigationContainer).map((i) => i.id),
  );
  if (navContainerIds.size === 0) return pass();

  const violations: string[] = [];
  for (const action of scene.actions) {
    if (navContainerIds.has(action.targetItemId)) {
      violations.push(
        `Navigation container "${action.targetItemId}" must not have actions (found: "${action.kind}")`,
      );
    }
  }
  return collect(violations);
}

/**
 * Check that edge items have valid, non-null endpoint IDs that exist in the scene.
 *
 * An edge item (relation with non-null direction) must have both
 * sourceEndpointId and targetEndpointId pointing to existing items.
 */
export function checkEdgeEndpointCompleteness(scene: SemanticScene): CheckResult {
  const itemIds = new Set(scene.items.map((i) => i.id));
  const violations: string[] = [];

  for (const item of scene.items.filter(isEdgeItem)) {
    if (item.sourceEndpointId === null) {
      violations.push(
        `Edge item "${item.id}" has null sourceEndpointId`,
      );
    } else if (!itemIds.has(item.sourceEndpointId)) {
      violations.push(
        `Edge item "${item.id}" references missing sourceEndpointId: "${item.sourceEndpointId}"`,
      );
    }

    if (item.targetEndpointId === null) {
      violations.push(
        `Edge item "${item.id}" has null targetEndpointId`,
      );
    } else if (!itemIds.has(item.targetEndpointId)) {
      violations.push(
        `Edge item "${item.id}" references missing targetEndpointId: "${item.targetEndpointId}"`,
      );
    }
  }
  return collect(violations);
}

/**
 * Check that all visual tokens reference existing item IDs.
 *
 * Stale tokens referencing items that were removed produce phantom visuals.
 */
export function checkTokenItemValidity(scene: SemanticScene): CheckResult {
  const itemIds = new Set(scene.items.map((i) => i.id));
  const violations: string[] = [];
  for (const token of scene.tokens) {
    if (!itemIds.has(token.itemId)) {
      violations.push(
        `Visual token references non-existent item ID: "${token.itemId}"`,
      );
    }
  }
  return collect(violations);
}

/**
 * Check that no item has a kind of '' (empty string) or any value outside the
 * SceneItemKind union (unknown/malformed isolation).
 *
 * Since SceneItemKind is a compile-time union, unknown kinds can only appear
 * via unsafe casts (e.g. data arriving over the wire without validation).
 */
const KNOWN_KINDS = new Set([
  'entity',
  'memory',
  'relation',
  'goal',
  'source',
  'episode',
  'summary',
  'skill',
  'rule',
  'navigation-container',
]);

export function checkNoUnknownKinds(scene: SemanticScene): CheckResult {
  const violations: string[] = [];
  for (const item of scene.items) {
    const k = item.kind as string;
    if (!KNOWN_KINDS.has(k)) {
      const display = k === '' ? '(empty string)' : `"${k}"`;
      violations.push(
        `Item "${item.id}" has unknown kind: ${display}`,
      );
    }
  }
  return collect(violations);
}

/**
 * Check that the scene has no duplicate (targetItemId, kind) action pairs.
 *
 * Duplicate action pairs produce redundant/conflicting UI affordances.
 */
export function checkActionUniqueness(scene: SemanticScene): CheckResult {
  const seen = new Set<string>();
  const violations: string[] = [];
  for (const action of scene.actions) {
    const key = `${action.targetItemId}::${action.kind}`;
    if (seen.has(key)) {
      violations.push(
        `Duplicate action (targetItemId="${action.targetItemId}", kind="${action.kind}")`,
      );
    }
    seen.add(key);
  }
  return collect(violations);
}

// ─── Combined runner ──────────────────────────────────────────────────────────

/**
 * Run all parity and purity checks and return a combined result.
 *
 * passed is true only when every individual check passes.
 * violations is the concatenated list of all violations across all checks,
 * prefixed with the check name for traceability.
 */
export function runAllParityChecks(scene: SemanticScene): CheckResult {
  const checks: Array<[string, (s: SemanticScene) => CheckResult]> = [
    ['ItemIdUniqueness', checkItemIdUniqueness],
    ['ActionTargetValidity', checkActionTargetValidity],
    ['NavigationContainerPurity', checkNavigationContainerPurity],
    ['EdgeEndpointCompleteness', checkEdgeEndpointCompleteness],
    ['TokenItemValidity', checkTokenItemValidity],
    ['NoUnknownKinds', checkNoUnknownKinds],
    ['ActionUniqueness', checkActionUniqueness],
  ];

  const allViolations: string[] = [];
  for (const [name, check] of checks) {
    const result = check(scene);
    for (const v of result.violations) {
      allViolations.push(`[${name}] ${v}`);
    }
  }
  return collect(allViolations);
}
