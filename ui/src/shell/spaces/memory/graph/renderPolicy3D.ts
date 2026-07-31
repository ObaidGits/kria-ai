/**
 * renderPolicy3D.ts — F6 isolated technical spike: render policy state machine.
 *
 * Pure TypeScript module — no JSX, no DOM, no WebGL, no side effects.
 *
 * This module implements the render-policy state machine for the 3D spike
 * (task 6.2.5). All state is immutable; every function returns a new value.
 *
 * Exports:
 *   • MotionMode             — 'full' | 'reduced' | 'none'
 *   • QualityLevel           — 6-step quality ladder (design.md §4.7.8)
 *   • DegradationReason      — typed reasons for quality degradation
 *   • degradeQuality         — step down the ladder by one level
 *   • shouldRender3D         — false only for 'list-first'
 *
 *   • IdleState              — { lastInputMs, lastSceneChangeMs, nowMs }
 *   • shouldStopRendering    — true when idle >2000ms (design §4.8.9)
 *
 *   • ContextLossState       — 'healthy' | 'lost' | 'recovering' | 'fallback'
 *   • ContextLossRecord      — state + 2D fallback context
 *   • onContextLoss          — healthy|recovering → lost
 *   • onContextRestored      — recovering → healthy
 *   • onFallbackActivated    — lost → fallback
 *
 *   • RenderPolicyState      — combined policy (motion + quality + idle + context)
 *   • RenderPolicyEvent      — discriminated union of policy events
 *   • applyEvent             — pure reducer: (policy, event) → policy
 *
 * Quality ladder order (design.md §4.7.8, frozen):
 *   full → no-decoration → no-labels → no-analytics → reduced-scene → list-first
 *
 * Idle threshold (design §4.8.9): stop render when no input/scene change for >2000ms.
 *
 * Context-loss recovery paths:
 *   healthy → lost → recovering → healthy  (GPU context restored)
 *   healthy → lost → fallback → healthy    (recovery failed; fall back to 2D/list)
 *
 * Design invariants (frozen per task 6.2.5):
 *   • All functions are pure — no side effects, no globals, no DOM.
 *   • degradeQuality never wraps; 'list-first' → 'list-first' (floor).
 *   • shouldRender3D is false ONLY for 'list-first'.
 *   • shouldStopRendering threshold is exactly >2000ms (strict greater-than).
 *   • In MotionMode 'none': animationEnabled is false, frameRequested is false.
 *   • In MotionMode 'reduced': animationEnabled is false, staticFramePending is true
 *     when a scene change arrives.
 *   • ContextLossRecord.state transitions are one-way per event; no backward jump
 *     except through the full recovery path.
 *   • onContextRestored is a no-op (returns record unchanged) unless state is
 *     'recovering'.
 *   • onFallbackActivated is a no-op unless state is 'lost'.
 *   • Multiple context-loss events accumulate — each call to onContextLoss
 *     overwrites lostAtMs, fallback2DQuery, fallback2DFocusId, and
 *     pendingActionKind with the latest values and forces state to 'lost'.
 *
 * IDs: MGR-001, MGR-004, MGR-012, MGR-015, MGR-026; MGD-003, MGD-026, MGD-046;
 *      task 6.2.5 (F6 pre-production spike only — not a shipped renderer path).
 */

// ─── Motion mode ──────────────────────────────────────────────────────────────

/**
 * Motion mode for the 3D renderer.
 *
 *   'full'    — animations enabled; continuous rendering on motion.
 *   'reduced' — no animations; one static frame per scene change.
 *   'none'    — rendering disabled entirely (Recovery_Mode or OS-level
 *               prefers-reduced-motion: no-preference overridden).
 */
export type MotionMode = 'full' | 'reduced' | 'none';

// ─── Quality ladder ───────────────────────────────────────────────────────────

/**
 * Six-step quality ladder from design.md §4.7.8.
 *
 * Ordered from highest to lowest (degradation order):
 *   full            → all decorations, labels, analytics
 *   no-decoration   → no edge glow, particle effects
 *   no-labels       → labels hidden
 *   no-analytics    → analytics panel hidden
 *   reduced-scene   → 120 node / 180 edge scene cap
 *   list-first      → 3D disabled entirely, list shown
 */
export type QualityLevel =
  | 'full'
  | 'no-decoration'
  | 'no-labels'
  | 'no-analytics'
  | 'reduced-scene'
  | 'list-first';

/**
 * Reason for a quality degradation step.
 *
 * Used only for intent documentation; degradeQuality always steps down by
 * exactly one level regardless of reason.
 */
export type DegradationReason =
  | 'frame-budget-exceeded'
  | 'memory-pressure'
  | 'thermal-throttle'
  | 'context-loss'
  | 'scene-cap-exceeded'
  | 'user-preference'
  | 'forced';

/**
 * Ordered quality levels, highest to lowest.
 * Frozen per task 6.2.5.
 */
const QUALITY_ORDER: QualityLevel[] = [
  'full',
  'no-decoration',
  'no-labels',
  'no-analytics',
  'reduced-scene',
  'list-first',
];

/**
 * Pure function: step down the quality ladder by exactly one level.
 *
 * Already at 'list-first' → returns 'list-first' (floor, no wrap).
 * The `reason` parameter is accepted for documentation/tracing but does not
 * affect the result.
 *
 * @param current — current quality level
 * @param reason  — reason for degradation (unused in computation)
 * @returns next lower quality level, or 'list-first' if already at floor
 */
export function degradeQuality(current: QualityLevel, reason: DegradationReason): QualityLevel {
  const idx = QUALITY_ORDER.indexOf(current);
  // indexOf should always return ≥0 for a valid QualityLevel, but guard defensively.
  if (idx < 0 || idx >= QUALITY_ORDER.length - 1) {
    return 'list-first';
  }
  return QUALITY_ORDER[idx + 1]!;
}

/**
 * Pure function: returns true when 3D rendering is active.
 *
 * Returns false ONLY for 'list-first' (3D disabled entirely).
 * All other quality levels permit 3D rendering (possibly degraded).
 *
 * @param quality — current quality level
 * @returns true if 3D is rendered, false if list-first fallback is active
 */
export function shouldRender3D(quality: QualityLevel): boolean {
  return quality !== 'list-first';
}

// ─── Idle render stop ─────────────────────────────────────────────────────────

/**
 * Idle state descriptor for shouldStopRendering.
 *
 *   lastInputMs       — timestamp (ms) of the last user input event.
 *   lastSceneChangeMs — timestamp (ms) of the last scene data change.
 *   nowMs             — current timestamp (ms) to compare against.
 */
export interface IdleState {
  lastInputMs: number;
  lastSceneChangeMs: number;
  nowMs: number;
}

/**
 * Idle threshold from design.md §4.8.9: stop render when no input or scene
 * change for > 2000ms.
 *
 * Frozen per task 6.2.5.
 */
export const IDLE_THRESHOLD_MS = 2000;

/**
 * Pure function: returns true when the render loop should stop due to idleness.
 *
 * Returns true when BOTH of the following hold:
 *   • nowMs - lastInputMs > IDLE_THRESHOLD_MS (2000 ms)
 *   • nowMs - lastSceneChangeMs > IDLE_THRESHOLD_MS (2000 ms)
 *
 * The comparison is strict greater-than (not ≥), matching design §4.8.9.
 *
 * @param idle — IdleState with timestamps in milliseconds
 * @returns true to stop rendering, false to continue
 */
export function shouldStopRendering(idle: IdleState): boolean {
  const sinceInput = idle.nowMs - idle.lastInputMs;
  const sinceScene = idle.nowMs - idle.lastSceneChangeMs;
  return sinceInput > IDLE_THRESHOLD_MS && sinceScene > IDLE_THRESHOLD_MS;
}

// ─── Context loss recovery state machine ─────────────────────────────────────

/**
 * Context-loss state machine states.
 *
 *   healthy    — GPU context is active and healthy.
 *   lost       — GPU context was lost; fallback data captured.
 *   recovering — Context restore in progress; waiting for confirmation.
 *   fallback   — Recovery failed; 2D/list fallback activated with restored
 *                query/focus/pending state.
 */
export type ContextLossState = 'healthy' | 'lost' | 'recovering' | 'fallback';

/**
 * Context-loss record carrying the current state and 2D fallback context.
 *
 *   state            — current recovery state
 *   lostAtMs         — timestamp when context was lost (null if never lost)
 *   fallback2DQuery  — the active query to restore in 2D (null if none)
 *   fallback2DFocusId — the focused item id to restore in 2D (null if none)
 *   pendingActionKind — any in-flight action kind at the time of loss (null if none)
 */
export interface ContextLossRecord {
  state: ContextLossState;
  lostAtMs: number | null;
  fallback2DQuery: string | null;
  fallback2DFocusId: string | null;
  pendingActionKind: string | null;
}

/**
 * Canonical initial (healthy) context-loss record.
 */
export const INITIAL_CONTEXT_LOSS_RECORD: ContextLossRecord = {
  state: 'healthy',
  lostAtMs: null,
  fallback2DQuery: null,
  fallback2DFocusId: null,
  pendingActionKind: null,
};

/**
 * Pure function: record a context-loss event.
 *
 * Transitions from any state → 'lost', capturing the current query/focus/
 * pending state for 2D fallback restoration.
 *
 * Multiple calls accumulate: each call overwrites lostAtMs, fallback2DQuery,
 * fallback2DFocusId, and pendingActionKind with the latest values, and forces
 * state to 'lost'.
 *
 * @param record        — current context-loss record
 * @param query         — active 2D query string at time of loss (may be null)
 * @param focusId       — focused item id at time of loss (may be null)
 * @param pendingAction — in-flight action kind at time of loss (may be null)
 * @param nowMs         — current timestamp in milliseconds
 * @returns new ContextLossRecord with state='lost' and fallback context captured
 */
export function onContextLoss(
  record: ContextLossRecord,
  query: string | null,
  focusId: string | null,
  pendingAction: string | null,
  nowMs: number,
): ContextLossRecord {
  return {
    state: 'lost',
    lostAtMs: nowMs,
    fallback2DQuery: query,
    fallback2DFocusId: focusId,
    pendingActionKind: pendingAction,
  };
}

/**
 * Pure function: signal that the GPU context has been restored.
 *
 * Transitions:
 *   recovering → healthy  (clears lostAtMs; retains fallback data for reference)
 *   any other state → no-op (returns record unchanged)
 *
 * Note: only 'recovering' is accepted because the contract requires
 *   healthy → lost → recovering → healthy
 * Calling onContextRestored from 'lost' directly (without a recovering step)
 * is a no-op.
 *
 * @param record — current context-loss record
 * @returns new ContextLossRecord with state='healthy', or unchanged if not recovering
 */
export function onContextRestored(record: ContextLossRecord): ContextLossRecord {
  if (record.state !== 'recovering') {
    return record;
  }
  return {
    ...record,
    state: 'healthy',
    lostAtMs: null,
  };
}

/**
 * Pure function: activate the 2D/list fallback after recovery failure.
 *
 * Transitions:
 *   lost → fallback  (fallback data is already captured from onContextLoss)
 *   any other state → no-op (returns record unchanged)
 *
 * After fallback is activated, the caller is responsible for restoring the
 * 2D view using fallback2DQuery, fallback2DFocusId, and pendingActionKind.
 *
 * @param record — current context-loss record
 * @returns new ContextLossRecord with state='fallback', or unchanged if not lost
 */
export function onFallbackActivated(record: ContextLossRecord): ContextLossRecord {
  if (record.state !== 'lost') {
    return record;
  }
  return {
    ...record,
    state: 'fallback',
  };
}

/**
 * Pure function: begin recovery attempt (lost → recovering).
 *
 * Only valid from 'lost' state; no-op otherwise.
 *
 * @param record — current context-loss record
 * @returns new ContextLossRecord with state='recovering', or unchanged
 */
export function onRecoveryStarted(record: ContextLossRecord): ContextLossRecord {
  if (record.state !== 'lost') {
    return record;
  }
  return {
    ...record,
    state: 'recovering',
  };
}

// ─── Combined render policy state ─────────────────────────────────────────────

/**
 * Combined render policy state for the 3D renderer.
 *
 *   motionMode       — current motion mode
 *   quality          — current quality level
 *   contextLoss      — context-loss recovery record
 *   animationEnabled — true when motion='full' and context is healthy
 *   frameRequested   — true when a new frame should be rendered
 *   staticFramePending — true in 'reduced' mode when a scene change arrived
 */
export interface RenderPolicyState {
  motionMode: MotionMode;
  quality: QualityLevel;
  contextLoss: ContextLossRecord;
  animationEnabled: boolean;
  frameRequested: boolean;
  staticFramePending: boolean;
}

/**
 * Canonical initial render policy state.
 * Full quality, full motion, healthy context.
 */
export const INITIAL_RENDER_POLICY: RenderPolicyState = {
  motionMode: 'full',
  quality: 'full',
  contextLoss: INITIAL_CONTEXT_LOSS_RECORD,
  animationEnabled: true,
  frameRequested: true,
  staticFramePending: false,
};

// ─── Render policy events ─────────────────────────────────────────────────────

/**
 * Discriminated union of all events that can be applied to a RenderPolicyState.
 */
export type RenderPolicyEvent =
  | { kind: 'set-motion-mode'; mode: MotionMode }
  | { kind: 'degrade-quality'; reason: DegradationReason }
  | { kind: 'reset-quality' }
  | { kind: 'scene-changed' }
  | { kind: 'context-lost'; query: string | null; focusId: string | null; pendingAction: string | null; nowMs: number }
  | { kind: 'recovery-started' }
  | { kind: 'context-restored' }
  | { kind: 'fallback-activated' }
  | { kind: 'static-frame-rendered' };

// ─── Helper: derive animation/frame flags from state ─────────────────────────

/**
 * Derive animation flags from the current motion mode and context-loss state.
 *
 *   animationEnabled: true only when motionMode='full' AND context is healthy.
 *   frameRequested:   true when motionMode is 'full' or 'reduced' AND context
 *                     is not 'lost' or 'fallback'.
 *   staticFramePending: only relevant in 'reduced' mode; set by scene-changed event.
 */
function deriveFlags(
  motionMode: MotionMode,
  contextLoss: ContextLossRecord,
  staticFramePending: boolean,
): Pick<RenderPolicyState, 'animationEnabled' | 'frameRequested' | 'staticFramePending'> {
  const contextOk = contextLoss.state === 'healthy' || contextLoss.state === 'recovering';
  const animationEnabled = motionMode === 'full' && contextOk;
  const frameRequested = motionMode !== 'none' && contextOk;
  return { animationEnabled, frameRequested, staticFramePending };
}

// ─── Pure reducer ─────────────────────────────────────────────────────────────

/**
 * Pure reducer: apply a RenderPolicyEvent to the current RenderPolicyState.
 *
 * Returns a new RenderPolicyState. The input is never mutated.
 *
 * Event semantics:
 *
 *   set-motion-mode        — change motion mode; re-derive flags.
 *   degrade-quality        — step down quality ladder one level.
 *   reset-quality          — restore quality to 'full'.
 *   scene-changed          — in 'reduced' mode, set staticFramePending=true.
 *   context-lost           — record loss, capture fallback 2D context.
 *   recovery-started       — transition lost → recovering.
 *   context-restored       — transition recovering → healthy.
 *   fallback-activated     — transition lost → fallback.
 *   static-frame-rendered  — clear staticFramePending (reduced mode frame done).
 *
 * @param policy — current policy state
 * @param event  — event to apply
 * @returns new policy state after applying the event
 */
export function applyEvent(
  policy: RenderPolicyState,
  event: RenderPolicyEvent,
): RenderPolicyState {
  switch (event.kind) {
    case 'set-motion-mode': {
      const motionMode = event.mode;
      // In 'none' mode, clear staticFramePending since rendering is disabled.
      const staticFramePending = motionMode === 'reduced' ? policy.staticFramePending : false;
      return {
        ...policy,
        motionMode,
        ...deriveFlags(motionMode, policy.contextLoss, staticFramePending),
      };
    }

    case 'degrade-quality': {
      return {
        ...policy,
        quality: degradeQuality(policy.quality, event.reason),
      };
    }

    case 'reset-quality': {
      return {
        ...policy,
        quality: 'full',
      };
    }

    case 'scene-changed': {
      // In 'reduced' mode, request a single static frame.
      const staticFramePending = policy.motionMode === 'reduced';
      return {
        ...policy,
        ...deriveFlags(policy.motionMode, policy.contextLoss, staticFramePending),
      };
    }

    case 'context-lost': {
      const contextLoss = onContextLoss(
        policy.contextLoss,
        event.query,
        event.focusId,
        event.pendingAction,
        event.nowMs,
      );
      return {
        ...policy,
        contextLoss,
        ...deriveFlags(policy.motionMode, contextLoss, false),
      };
    }

    case 'recovery-started': {
      const contextLoss = onRecoveryStarted(policy.contextLoss);
      return {
        ...policy,
        contextLoss,
        ...deriveFlags(policy.motionMode, contextLoss, policy.staticFramePending),
      };
    }

    case 'context-restored': {
      const contextLoss = onContextRestored(policy.contextLoss);
      return {
        ...policy,
        contextLoss,
        ...deriveFlags(policy.motionMode, contextLoss, policy.staticFramePending),
      };
    }

    case 'fallback-activated': {
      const contextLoss = onFallbackActivated(policy.contextLoss);
      return {
        ...policy,
        contextLoss,
        ...deriveFlags(policy.motionMode, contextLoss, false),
      };
    }

    case 'static-frame-rendered': {
      // Clear the pending static frame flag; animation stays disabled in 'reduced'.
      return {
        ...policy,
        staticFramePending: false,
      };
    }
  }
}
