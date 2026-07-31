/**
 * RelationActions — Relation-specific mutation workflow component.
 *
 * Handles nine relation action kinds with preview, stale detection, policy
 * label, commit, committed result (with undo), and error states:
 *   create, edit, type-change, direction-change, add-evidence,
 *   confirm, expire, delete, undo
 *
 * Also handles:
 *   - Prediction materialization: when a predicted relation is materialized
 *     as a confirmed relation (isPendingConfirmation + predictionInfo).
 *   - Pending confirmation state: shows predicted score/rationale with
 *     confirm/reject actions.
 *
 * Invariants (F4.4 / task 4.4.6):
 * - Root: data-testid="relation-actions-root"
 * - data-testid="relation-action-phase" carries data-phase attribute per phase
 * - idle: renders only root + phase element, no content
 * - Relative score is ALWAYS shown as "Rank: X%" (never "confidence",
 *   "certainty", or "probability")
 * - Commit button is disabled when isStale=true.
 * - Stale warning includes role="alert".
 * - Committing indicator includes role="status".
 * - Error message includes role="alert".
 * - Retry button only rendered when canRetry=true.
 * - Undo button only rendered when isPendingUndo=true.
 * - Kind-specific fields only rendered when present.
 * - All copy comes from backend data; UI never invents facts or labels.
 *
 * Requirements: F4.4 (task 4.4.6)
 */
import { Show, Switch, Match } from "solid-js";

// ─── Data types ───────────────────────────────────────────────────────────────

export type RelationActionKind =
  | 'create'
  | 'edit'
  | 'type-change'
  | 'direction-change'
  | 'add-evidence'
  | 'confirm'
  | 'expire'
  | 'delete'
  | 'undo';

export interface PredictionInfo {
  relativeScore: number;   // 0.0-1.0 — shown as "Rank: X%" never "confidence"
  rationale: string;       // from backend
  profileId: string;       // retrieval profile that generated this prediction
}

export interface RelationActionPreview {
  kind: RelationActionKind;
  relationId: string | null;   // null for create
  label: string;               // human label from backend
  description: string;         // what will change
  policyLabel: string;         // exact policy context
  baseRevision: number;
  isStale: boolean;
  isPendingConfirmation: boolean;  // true when relation is in pending-confirmation state
  predictionInfo: PredictionInfo | null;  // only when isPendingConfirmation=true
  // kind-specific
  currentType?: string;        // for type-change
  proposedType?: string;
  currentDirection?: string;   // for direction-change
  proposedDirection?: string;
  evidenceSummary?: string;    // for add-evidence
  undoTargetRevision?: number; // for undo
}

export interface RelationActionResult {
  kind: RelationActionKind;
  newRevision: number;
  auditRecordId: string;
  description: string;
  isPendingUndo: boolean;      // true if undo is available
}

export type RelationActionPhase =
  | { phase: 'idle' }
  | { phase: 'preview'; action: RelationActionPreview }
  | { phase: 'committing' }
  | { phase: 'committed'; result: RelationActionResult }
  | { phase: 'error'; message: string; canRetry: boolean };

// ─── Props ────────────────────────────────────────────────────────────────────

export interface RelationActionsProps {
  state: RelationActionPhase;
  onCommit: () => void;
  onCancel: () => void;
  onUndo: () => void;
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

/** Format a 0.0-1.0 relative score as "Rank: X%" — NEVER "confidence". */
function formatRelativeScore(score: number): string {
  return `Rank: ${Math.round(score * 100)}%`;
}

// ─── Component ────────────────────────────────────────────────────────────────

export function RelationActions(props: RelationActionsProps) {
  return (
    <div data-testid="relation-actions-root">
      <Switch>

        {/* ── Idle phase ────────────────────────────────────────────── */}
        <Match when={props.state.phase === 'idle'}>
          <div data-testid="relation-action-phase" data-phase="idle" />
        </Match>

        {/* ── Preview phase ─────────────────────────────────────────── */}
        <Match when={props.state.phase === 'preview' && props.state}>
          {(s) => {
            const action = () =>
              (s() as Extract<RelationActionPhase, { phase: 'preview' }>).action;
            return (
              <div data-testid="relation-action-phase" data-phase="preview">

                {/* Common fields */}
                <span data-testid="relation-action-label">{action().label}</span>
                <span data-testid="relation-action-description">{action().description}</span>
                <span data-testid="relation-action-policy-label">{action().policyLabel}</span>
                <span data-testid="relation-action-base-revision">{action().baseRevision}</span>

                {/* Stale warning — only when stale */}
                <Show when={action().isStale}>
                  <div data-testid="relation-stale-warning" role="alert">
                    Preview is stale — base revision has changed. Refresh before committing.
                  </div>
                </Show>

                {/* Pending confirmation banner */}
                <Show when={action().isPendingConfirmation}>
                  <div data-testid="pending-confirmation-banner">
                    This relation is pending confirmation.
                  </div>
                </Show>

                {/* Prediction info — only when predictionInfo is present */}
                <Show when={action().predictionInfo !== null}>
                  <span data-testid="prediction-score">
                    {formatRelativeScore(action().predictionInfo!.relativeScore)}
                  </span>
                  <span data-testid="prediction-rationale">
                    {action().predictionInfo!.rationale}
                  </span>
                  <span data-testid="prediction-profile">
                    {action().predictionInfo!.profileId}
                  </span>
                </Show>

                {/* Kind-specific: type-change */}
                <Show when={action().currentType !== undefined}>
                  <span data-testid="relation-type-current">{action().currentType}</span>
                </Show>
                <Show when={action().proposedType !== undefined}>
                  <span data-testid="relation-type-proposed">{action().proposedType}</span>
                </Show>

                {/* Kind-specific: direction-change */}
                <Show when={action().currentDirection !== undefined}>
                  <span data-testid="relation-dir-current">{action().currentDirection}</span>
                </Show>
                <Show when={action().proposedDirection !== undefined}>
                  <span data-testid="relation-dir-proposed">{action().proposedDirection}</span>
                </Show>

                {/* Kind-specific: add-evidence */}
                <Show when={action().evidenceSummary !== undefined}>
                  <span data-testid="relation-evidence-summary">{action().evidenceSummary}</span>
                </Show>

                {/* Kind-specific: undo */}
                <Show when={action().undoTargetRevision !== undefined}>
                  <span data-testid="undo-target-revision">{action().undoTargetRevision}</span>
                </Show>

                {/* Commit button — disabled when stale */}
                <button
                  type="button"
                  data-testid="relation-commit-button"
                  disabled={action().isStale}
                  onClick={() => {
                    if (!action().isStale) {
                      props.onCommit();
                    }
                  }}
                >
                  Commit
                </button>

                {/* Cancel button */}
                <button
                  type="button"
                  data-testid="relation-cancel-button"
                  onClick={props.onCancel}
                >
                  Cancel
                </button>

              </div>
            );
          }}
        </Match>

        {/* ── Committing phase ──────────────────────────────────────── */}
        <Match when={props.state.phase === 'committing'}>
          <div data-testid="relation-action-phase" data-phase="committing">
            <span data-testid="relation-committing" role="status" aria-live="polite">
              Committing…
            </span>
          </div>
        </Match>

        {/* ── Committed phase ───────────────────────────────────────── */}
        <Match when={props.state.phase === 'committed' && props.state}>
          {(s) => {
            const result = () =>
              (s() as Extract<RelationActionPhase, { phase: 'committed' }>).result;
            return (
              <div data-testid="relation-action-phase" data-phase="committed">
                <span data-testid="relation-result-revision">{result().newRevision}</span>
                <span data-testid="relation-result-audit-id">{result().auditRecordId}</span>
                <span data-testid="relation-result-description">{result().description}</span>

                {/* Undo button — only when isPendingUndo */}
                <Show when={result().isPendingUndo}>
                  <button
                    type="button"
                    data-testid="relation-undo-button"
                    onClick={props.onUndo}
                  >
                    Undo
                  </button>
                </Show>
              </div>
            );
          }}
        </Match>

        {/* ── Error phase ───────────────────────────────────────────── */}
        <Match when={props.state.phase === 'error' && props.state}>
          {(s) => {
            const err = () => s() as Extract<RelationActionPhase, { phase: 'error' }>;
            return (
              <div data-testid="relation-action-phase" data-phase="error">

                <div data-testid="relation-error" role="alert">
                  {err().message}
                </div>

                {/* Retry button — only when canRetry */}
                <Show when={err().canRetry}>
                  <button
                    type="button"
                    data-testid="relation-retry"
                    onClick={props.onCommit}
                  >
                    Retry
                  </button>
                </Show>

              </div>
            );
          }}
        </Match>

      </Switch>
    </div>
  );
}

export default RelationActions;
