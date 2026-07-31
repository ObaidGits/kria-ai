/**
 * ContradictionActions — Contradiction resolution workflow component.
 *
 * Handles three contradiction resolution kinds with capability-gated actions,
 * evidence-bearing path explanation, stale detection, policy label, commit,
 * committed result, and error states:
 *   confirm, supersede, keep-both
 *
 * Invariants (F4.4 / task 4.4.7):
 * - Root: data-testid="contradiction-actions-root"
 * - data-testid="contradiction-action-phase" carries data-phase attribute per phase
 * - idle: renders only root + phase element, no content
 * - All copy comes from backend data; UI never invents facts or labels.
 * - Commit button is disabled when isStale=true OR selectedAction=null.
 * - Stale warning includes role="alert".
 * - Committing indicator includes role="status".
 * - Error message includes role="alert".
 * - Retry button only rendered when canRetry=true.
 * - Action buttons only rendered when the corresponding cap flag is true.
 *   When cap flag is false, the button is ENTIRELY ABSENT — no disabled state,
 *   no locked icon, no gap, no hint of its existence.
 * - Evidence path section only rendered when evidencePath is non-null.
 * - supersededBy indicator only rendered when supersededBy is present.
 *
 * Requirements: F4.4 (task 4.4.7)
 */
import { Show, For, Switch, Match } from "solid-js";

// ─── Data types ───────────────────────────────────────────────────────────────

export type ContradictionActionKind = 'confirm' | 'supersede' | 'keep-both';

export interface EvidencePath {
  pathId: string;
  steps: string[];           // ordered path labels from backend
  evidenceSummary: string;   // brief evidence summary
}

export interface ContradictionItem {
  itemId: string;
  label: string;
  value: string;
  truthState: string;
  evidenceSummary: string | null;
}

export interface ContradictionPreview {
  itemA: ContradictionItem;
  itemB: ContradictionItem;
  description: string;           // what the contradiction is (from backend)
  policyLabel: string;           // exact policy context
  baseRevision: number;
  isStale: boolean;
  // Authorized actions — only present when capability exists
  canConfirm: boolean;
  canSupersede: boolean;
  canKeepBoth: boolean;
  // Evidence path — only shown when capability authorizes it
  evidencePath: EvidencePath | null;
  // For supersede: which value is authoritative
  supersededBy?: 'a' | 'b';
}

export interface ContradictionActionResult {
  kind: ContradictionActionKind;
  newRevision: number;
  auditRecordId: string;
  description: string;
}

export type ContradictionActionPhase =
  | { phase: 'idle' }
  | { phase: 'preview'; action: ContradictionPreview }
  | { phase: 'committing'; kind: ContradictionActionKind }
  | { phase: 'committed'; result: ContradictionActionResult }
  | { phase: 'error'; message: string; canRetry: boolean };

// ─── Props ────────────────────────────────────────────────────────────────────

export interface ContradictionActionsProps {
  state: ContradictionActionPhase;
  selectedAction: ContradictionActionKind | null;
  onSelectAction: (kind: ContradictionActionKind) => void;
  onCommit: () => void;
  onCancel: () => void;
}

// ─── Sub-component: ContradictionItemView ─────────────────────────────────────

interface ContradictionItemViewProps {
  testId: string;
  item: ContradictionItem;
}

function ContradictionItemView(props: ContradictionItemViewProps) {
  return (
    <div data-testid={props.testId} data-item-id={props.item.itemId}>
      <span data-testid={`${props.testId}-field`} data-field="item-label">
        {props.item.label}
      </span>
      <span data-testid={`${props.testId}-field`} data-field="item-value">
        {props.item.value}
      </span>
      <span data-testid={`${props.testId}-field`} data-field="truth-state">
        {props.item.truthState}
      </span>
      <Show when={props.item.evidenceSummary !== null}>
        <span data-testid={`${props.testId}-field`} data-field="evidence-summary">
          {props.item.evidenceSummary}
        </span>
      </Show>
    </div>
  );
}

// ─── Component ────────────────────────────────────────────────────────────────

export function ContradictionActions(props: ContradictionActionsProps) {
  return (
    <div data-testid="contradiction-actions-root">
      <Switch>

        {/* ── Idle phase ────────────────────────────────────────────── */}
        <Match when={props.state.phase === 'idle'}>
          <div data-testid="contradiction-action-phase" data-phase="idle" />
        </Match>

        {/* ── Preview phase ─────────────────────────────────────────── */}
        <Match when={props.state.phase === 'preview' && props.state}>
          {(s) => {
            const action = () =>
              (s() as Extract<ContradictionActionPhase, { phase: 'preview' }>).action;
            return (
              <div data-testid="contradiction-action-phase" data-phase="preview">

                {/* Contradiction items */}
                <ContradictionItemView
                  testId="contradiction-item-a"
                  item={action().itemA}
                />
                <ContradictionItemView
                  testId="contradiction-item-b"
                  item={action().itemB}
                />

                {/* Common fields */}
                <span data-testid="contradiction-description">{action().description}</span>
                <span data-testid="contradiction-policy-label">{action().policyLabel}</span>
                <span data-testid="contradiction-base-revision">{action().baseRevision}</span>

                {/* Stale warning — only when stale */}
                <Show when={action().isStale}>
                  <div data-testid="contradiction-stale-warning" role="alert">
                    Preview is stale — base revision has changed. Refresh before committing.
                  </div>
                </Show>

                {/* Evidence path — only when capability authorizes it (non-null) */}
                <Show when={action().evidencePath !== null}>
                  <div data-testid="evidence-path">
                    <span data-testid="evidence-path-summary">
                      {action().evidencePath!.evidenceSummary}
                    </span>
                    <div data-testid="evidence-path-steps">
                      <For each={action().evidencePath!.steps}>
                        {(step) => <span>{step}</span>}
                      </For>
                    </div>
                  </div>
                </Show>

                {/* supersededBy — only when present */}
                <Show when={action().supersededBy !== undefined}>
                  <span data-testid="superseded-by">{action().supersededBy}</span>
                </Show>

                {/* Action buttons — each only rendered when its cap flag is true.
                    ABSENT entirely when unauthorized — no disabled, no hint. */}

                <Show when={action().canConfirm}>
                  <button
                    type="button"
                    data-testid="action-confirm-button"
                    data-selected={props.selectedAction === 'confirm' ? 'true' : 'false'}
                    onClick={() => props.onSelectAction('confirm')}
                  >
                    Confirm
                  </button>
                </Show>

                <Show when={action().canSupersede}>
                  <button
                    type="button"
                    data-testid="action-supersede-button"
                    data-selected={props.selectedAction === 'supersede' ? 'true' : 'false'}
                    onClick={() => props.onSelectAction('supersede')}
                  >
                    Supersede
                  </button>
                </Show>

                <Show when={action().canKeepBoth}>
                  <button
                    type="button"
                    data-testid="action-keep-both-button"
                    data-selected={props.selectedAction === 'keep-both' ? 'true' : 'false'}
                    onClick={() => props.onSelectAction('keep-both')}
                  >
                    Keep Both
                  </button>
                </Show>

                {/* Commit button — disabled when no action selected OR stale */}
                <button
                  type="button"
                  data-testid="contradiction-commit-button"
                  disabled={props.selectedAction === null || action().isStale}
                  onClick={() => {
                    if (props.selectedAction !== null && !action().isStale) {
                      props.onCommit();
                    }
                  }}
                >
                  Commit
                </button>

                {/* Cancel button */}
                <button
                  type="button"
                  data-testid="contradiction-cancel-button"
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
          <div data-testid="contradiction-action-phase" data-phase="committing">
            <span data-testid="contradiction-committing" role="status" aria-live="polite">
              Committing…
            </span>
          </div>
        </Match>

        {/* ── Committed phase ───────────────────────────────────────── */}
        <Match when={props.state.phase === 'committed' && props.state}>
          {(s) => {
            const result = () =>
              (s() as Extract<ContradictionActionPhase, { phase: 'committed' }>).result;
            return (
              <div data-testid="contradiction-action-phase" data-phase="committed">
                <span data-testid="contradiction-result-revision">{result().newRevision}</span>
                <span data-testid="contradiction-result-audit-id">{result().auditRecordId}</span>
                <span data-testid="contradiction-result-description">{result().description}</span>
                <span data-testid="contradiction-result-kind">{result().kind}</span>
              </div>
            );
          }}
        </Match>

        {/* ── Error phase ───────────────────────────────────────────── */}
        <Match when={props.state.phase === 'error' && props.state}>
          {(s) => {
            const err = () => s() as Extract<ContradictionActionPhase, { phase: 'error' }>;
            return (
              <div data-testid="contradiction-action-phase" data-phase="error">

                <div data-testid="contradiction-error" role="alert">
                  {err().message}
                </div>

                {/* Retry button — only when canRetry */}
                <Show when={err().canRetry}>
                  <button
                    type="button"
                    data-testid="contradiction-retry"
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

export default ContradictionActions;
