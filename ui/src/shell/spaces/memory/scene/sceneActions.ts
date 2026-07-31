/**
 * memory/scene/sceneActions — Centralized typed scene actions with capability authorization.
 *
 * Pure TypeScript module — no JSX, no DOM, no side effects.
 *
 * Centralizes all typed select/expand/inspect/path/correct/merge/split/relate/
 * forget/restore/delete/fit/back/forward actions and capability authorization.
 *
 * Design invariants (F4.6 / task 4.6.5):
 *   - Capabilities gate which actions appear — unauthorized actions are entirely absent.
 *   - select, expand, inspect are always authorized regardless of capabilities.
 *   - fit, back, forward are scene-level actions, not item-level.
 *   - correct, merge, split, forget, delete are dangerous and require preview.
 *   - Only authorized actions appear; disabled items are excluded entirely.
 */

import type { SceneActionKind, SemanticSceneAction } from './semanticScene';

// ─── Capabilities ─────────────────────────────────────────────────────────────

/**
 * Capabilities that govern which scene actions are available to the user.
 * Each flag gates one or more action kinds.
 */
export interface SceneCapabilities {
  /** Correction workflow — gates the 'correct' action. */
  canCorrect: boolean;
  /** Entity merge — gates the 'merge' action. */
  canMerge: boolean;
  /** Entity split — gates the 'split' action. */
  canSplit: boolean;
  /** Relation management — gates the 'relate' action. */
  canRelate: boolean;
  /** Forget lifecycle — gates the 'forget' action. */
  canForget: boolean;
  /** Restore lifecycle — gates the 'restore' action. */
  canRestore: boolean;
  /** Hard delete lifecycle — gates the 'delete' action. */
  canDelete: boolean;
  /** Path navigation — gates the 'path' action. */
  canNavigatePath: boolean;
  /** Camera fit — gates the 'fit' action. */
  canFitView: boolean;
  /** Back/forward history navigation — gates 'back' and 'forward' actions. */
  canNavigateHistory: boolean;
}

/**
 * Default capability set — all capabilities enabled except none are disabled.
 * Destructive actions (correct, merge, split, forget, delete) are ON by default
 * because the spec's "default" is a fully-capable user; callers narrow as needed.
 */
export const DEFAULT_CAPABILITIES: SceneCapabilities = {
  canCorrect: true,
  canMerge: true,
  canSplit: true,
  canRelate: true,
  canForget: true,
  canRestore: true,
  canDelete: true,
  canNavigatePath: true,
  canFitView: true,
  canNavigateHistory: true,
};

// ─── Action kinds ─────────────────────────────────────────────────────────────

/**
 * All action kinds in canonical order.
 * Order is preserved in buildAuthorizedActions output.
 */
export const ALL_ACTION_KINDS: readonly SceneActionKind[] = [
  'select',
  'expand',
  'inspect',
  'path',
  'correct',
  'merge',
  'split',
  'relate',
  'forget',
  'restore',
  'delete',
  'fit',
  'back',
  'forward',
] as const;

// ─── Authorization map ────────────────────────────────────────────────────────

/**
 * Maps each action kind to its capability predicate.
 * select, expand, inspect are unconditionally authorized.
 */
const ACTION_CAPABILITY_MAP: Record<SceneActionKind, (caps: SceneCapabilities) => boolean> = {
  select:   () => true,
  expand:   () => true,
  inspect:  () => true,
  path:     (caps) => caps.canNavigatePath,
  correct:  (caps) => caps.canCorrect,
  merge:    (caps) => caps.canMerge,
  split:    (caps) => caps.canSplit,
  relate:   (caps) => caps.canRelate,
  forget:   (caps) => caps.canForget,
  restore:  (caps) => caps.canRestore,
  delete:   (caps) => caps.canDelete,
  fit:      (caps) => caps.canFitView,
  back:     (caps) => caps.canNavigateHistory,
  forward:  (caps) => caps.canNavigateHistory,
};

// ─── Labels ───────────────────────────────────────────────────────────────────

const ACTION_LABELS: Record<SceneActionKind, string> = {
  select:  'Select',
  expand:  'Expand',
  inspect: 'Inspect',
  path:    'Find path',
  correct: 'Correct',
  merge:   'Merge',
  split:   'Split',
  relate:  'Relate',
  forget:  'Forget',
  restore: 'Restore',
  delete:  'Delete',
  fit:     'Fit view',
  back:    'Back',
  forward: 'Forward',
};

// ─── Dangerous / preview flags ────────────────────────────────────────────────

/** Action kinds that are flagged as dangerous and require a preview confirmation. */
const DANGEROUS_KINDS = new Set<SceneActionKind>([
  'correct',
  'merge',
  'split',
  'forget',
  'delete',
]);

/** Scene-level action kinds — not tied to any specific item. */
const SCENE_LEVEL_KINDS = new Set<SceneActionKind>(['fit', 'back', 'forward']);

// ─── Authorization helpers ────────────────────────────────────────────────────

/**
 * Returns true if the given action kind is authorized by the capability set.
 */
export function isActionAuthorized(kind: SceneActionKind, caps: SceneCapabilities): boolean {
  return ACTION_CAPABILITY_MAP[kind](caps);
}

// ─── Item-level action builder ────────────────────────────────────────────────

export interface BuildAuthorizedActionsOptions {
  /** Action kinds to exclude even if authorized by capabilities. */
  excludeKinds?: SceneActionKind[];
}

/**
 * Builds the authorized action list for a specific scene item.
 *
 * - Iterates ALL_ACTION_KINDS in order.
 * - Excludes fit, back, forward (scene-level — use buildSceneLevelActions instead).
 * - Skips any kind not authorized by the capability set.
 * - Skips any kind in options.excludeKinds.
 * - isEnabled=true for all included actions (disabled items are excluded entirely).
 * - isDangerous=true and requiresPreview=true for: correct, merge, split, forget, delete.
 */
export function buildAuthorizedActions(
  itemId: string,
  caps: SceneCapabilities,
  options?: BuildAuthorizedActionsOptions,
): SemanticSceneAction[] {
  const excludeSet = new Set<SceneActionKind>(options?.excludeKinds ?? []);
  const actions: SemanticSceneAction[] = [];

  for (const kind of ALL_ACTION_KINDS) {
    // Scene-level actions are not item-level
    if (SCENE_LEVEL_KINDS.has(kind)) continue;

    // Capability authorization check
    if (!isActionAuthorized(kind, caps)) continue;

    // Caller-supplied exclusion list
    if (excludeSet.has(kind)) continue;

    const dangerous = DANGEROUS_KINDS.has(kind);

    actions.push({
      targetItemId: itemId,
      kind,
      label: ACTION_LABELS[kind],
      isEnabled: true,
      isDangerous: dangerous,
      requiresPreview: dangerous,
    });
  }

  return actions;
}

// ─── Scene-level action builder ───────────────────────────────────────────────

/**
 * Builds scene-level actions not tied to any specific item.
 * Scene-level actions are: fit, back, forward.
 *
 * Returns actions whose targetItemId is omitted (scene-scope).
 */
export function buildSceneLevelActions(
  caps: SceneCapabilities,
): Array<Omit<SemanticSceneAction, 'targetItemId'>> {
  const actions: Array<Omit<SemanticSceneAction, 'targetItemId'>> = [];

  for (const kind of ALL_ACTION_KINDS) {
    if (!SCENE_LEVEL_KINDS.has(kind)) continue;
    if (!isActionAuthorized(kind, caps)) continue;

    actions.push({
      kind,
      label: ACTION_LABELS[kind],
      isEnabled: true,
      isDangerous: false,
      requiresPreview: false,
    });
  }

  return actions;
}

// ─── Typed action dispatch event ──────────────────────────────────────────────

/**
 * A typed action dispatch event fired when a user activates a scene action.
 *
 * timestamp is set at dispatch time via performance.now() for latency tracking.
 */
export interface SceneActionEvent {
  /** ID of the scene item the action targets. */
  itemId: string;
  kind: SceneActionKind;
  /** performance.now() at the time the action was dispatched. */
  timestamp: number;
}
