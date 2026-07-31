/**
 * CorrectionPreview — Two-phase correction workflow component.
 *
 * Renders a governed correction preview/commit flow:
 *
 * Phase 1 (preview): Shows current vs proposed value, evidence, scope,
 *   affected count, reversibility, base revision, audit consequence, and
 *   commit/cancel controls. Commit is disabled when preview is stale.
 *
 * Phase 2 (committed): Shows new revision, audit record ID, affected
 *   count, and undo button (when reversible and within window).
 *
 * Additional phases:
 *   committing — in-flight indicator.
 *   error — error message with optional retry.
 *
 * Invariants (F4.4 / task 4.4.4):
 * - Root: data-testid="correction-preview"
 * - data-testid="correction-phase" carries data-phase attribute per phase
 * - All copy comes from backend data; UI never invents facts or labels.
 * - Commit button is disabled when isStale=true or phase is 'committing'.
 * - Stale warning includes role="alert".
 * - Committing indicator includes role="status".
 * - Error message includes role="alert".
 * - Undo button only rendered when canUndo=true.
 * - Undo expiry only rendered when undoWindowExpiry is non-null.
 * - Evidence note only rendered when evidence is non-null.
 * - Reversibility window only rendered when isReversible and reversibilityWindow non-null.
 *
 * Requirements: F4.4 (task 4.4.4)
 */
import { Show, Switch, Match } from "solid-js";

// ─── Data types ───────────────────────────────────────────────────────────────

export interface CorrectionPreviewData {
  itemId: string;
  fieldName: string;             // which field is being corrected
  currentValue: string;          // current value (from backend)
  proposedValue: string;         // proposed new value
  evidence: string | null;       // evidence for the correction
  scope: string;                 // e.g. "this item only", "this item and 3 derivations"
  affectedCount: number;
  isReversible: boolean;
  reversibilityWindow: string | null;  // e.g. "30 days" — only when isReversible
  baseRevision: number;          // revision when preview was generated
  auditConsequence: string;      // exact text from backend
  isStale: boolean;              // true when base revision is outdated
}

export interface CorrectionCommitResult {
  newRevision: number;
  auditRecordId: string;
  affectedCount: number;
  canUndo: boolean;
  undoWindowExpiry: string | null;  // ISO timestamp when undo expires
}

export type CorrectionPhase =
  | { phase: 'preview'; data: CorrectionPreviewData }
  | { phase: 'committed'; result: CorrectionCommitResult }
  | { phase: 'committing' }
  | { phase: 'error'; message: string; canRetry: boolean };

// ─── Props ────────────────────────────────────────────────────────────────────

export interface CorrectionPreviewProps {
  state: CorrectionPhase;
  onCommit: () => void;   // fires when user confirms commit
  onCancel: () => void;   // fires when user cancels
  onUndo: () => void;     // fires when user activates undo after commit
}

// ─── Component ────────────────────────────────────────────────────────────────

export function CorrectionPreview(props: CorrectionPreviewProps) {
  return (
    <div data-testid="correction-preview">
      <Switch>

        {/* ── Preview phase ─────────────────────────────────────────── */}
        <Match when={props.state.phase === 'preview' && props.state}>
          {(s) => {
            const data = () => (s() as Extract<CorrectionPhase, { phase: 'preview' }>).data;
            return (
              <div data-testid="correction-phase" data-phase="preview">

                {/* Current vs proposed */}
                <span data-testid="current-value">{data().currentValue}</span>
                <span data-testid="proposed-value">{data().proposedValue}</span>

                {/* Evidence — only when non-null */}
                <Show when={data().evidence !== null}>
                  <span data-testid="evidence-note">{data().evidence}</span>
                </Show>

                {/* Scope */}
                <span data-testid="scope-label">{data().scope}</span>

                {/* Affected count */}
                <span data-testid="affected-count">{data().affectedCount}</span>

                {/* Reversibility */}
                <span
                  data-testid="reversibility"
                  data-reversible={data().isReversible ? "true" : "false"}
                >
                  {data().isReversible
                    ? `Reversible: ${data().reversibilityWindow ?? ""}`
                    : "Irreversible"}
                </span>

                {/* Reversibility window — only when reversible and window non-null */}
                <Show when={data().isReversible && data().reversibilityWindow !== null}>
                  <span data-testid="reversibility-window">{data().reversibilityWindow}</span>
                </Show>

                {/* Base revision */}
                <span data-testid="base-revision">{data().baseRevision}</span>

                {/* Audit consequence */}
                <span data-testid="audit-consequence">{data().auditConsequence}</span>

                {/* Stale warning — only when stale */}
                <Show when={data().isStale}>
                  <div data-testid="stale-warning" role="alert">
                    Preview is stale — base revision has changed. Refresh before committing.
                  </div>
                </Show>

                {/* Commit button — disabled when stale or while committing */}
                <button
                  type="button"
                  data-testid="commit-button"
                  disabled={data().isStale || props.state.phase === 'committing'}
                  onClick={() => {
                    if (!data().isStale) {
                      props.onCommit();
                    }
                  }}
                >
                  Commit
                </button>

                {/* Cancel button */}
                <button
                  type="button"
                  data-testid="cancel-button"
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
          <div data-testid="correction-phase" data-phase="committing">
            <span data-testid="committing-indicator" role="status" aria-live="polite">
              Committing…
            </span>
          </div>
        </Match>

        {/* ── Committed phase ───────────────────────────────────────── */}
        <Match when={props.state.phase === 'committed' && props.state}>
          {(s) => {
            const result = () => (s() as Extract<CorrectionPhase, { phase: 'committed' }>).result;
            return (
              <div data-testid="correction-phase" data-phase="committed">

                <span data-testid="new-revision">{result().newRevision}</span>
                <span data-testid="audit-record-id">{result().auditRecordId}</span>
                <span data-testid="committed-affected-count">{result().affectedCount}</span>

                {/* Undo button — only when canUndo */}
                <Show when={result().canUndo}>
                  <button
                    type="button"
                    data-testid="undo-button"
                    onClick={props.onUndo}
                  >
                    Undo
                  </button>
                </Show>

                {/* Undo expiry — only when non-null */}
                <Show when={result().undoWindowExpiry !== null}>
                  <span data-testid="undo-expiry">{result().undoWindowExpiry}</span>
                </Show>

              </div>
            );
          }}
        </Match>

        {/* ── Error phase ───────────────────────────────────────────── */}
        <Match when={props.state.phase === 'error' && props.state}>
          {(s) => {
            const err = () => s() as Extract<CorrectionPhase, { phase: 'error' }>;
            return (
              <div data-testid="correction-phase" data-phase="error">

                <div data-testid="correction-error" role="alert">
                  {err().message}
                </div>

                {/* Retry button — only when canRetry */}
                <Show when={err().canRetry}>
                  <button
                    type="button"
                    data-testid="correction-retry"
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

export default CorrectionPreview;
