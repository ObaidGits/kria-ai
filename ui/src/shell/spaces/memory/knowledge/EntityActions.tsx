/**
 * EntityActions — Entity-specific mutation workflow component.
 *
 * Handles seven entity action kinds with preview, stale detection, policy
 * label, commit, committed result, and error states:
 *   rename, type-change, add-alias, remove-alias, accept-proposal,
 *   reject-proposal, merge, split, reverse.
 *
 * Invariants (F4.4 / task 4.4.5):
 * - Root: data-testid="entity-actions-root"
 * - data-testid="entity-action-phase" carries data-phase attribute per phase
 * - idle: renders only root + phase element, no content
 * - All copy comes from backend data; UI never invents facts or labels.
 * - Commit button is disabled when isStale=true.
 * - Stale warning includes role="alert".
 * - Committing indicator includes role="status".
 * - Error message includes role="alert".
 * - Retry button only rendered when canRetry=true.
 * - Kind-specific fields only rendered when present.
 *
 * Requirements: F4.4 (task 4.4.5)
 */
import { Show, Switch, Match } from "solid-js";

// ─── Data types ───────────────────────────────────────────────────────────────

export type EntityActionKind =
  | 'rename'
  | 'type-change'
  | 'add-alias'
  | 'remove-alias'
  | 'accept-proposal'
  | 'reject-proposal'
  | 'merge'
  | 'split'
  | 'reverse';

export interface EntityActionPreview {
  kind: EntityActionKind;
  itemId: string;
  label: string;               // human label for this action type
  description: string;         // what will change (from backend)
  policyLabel: string;         // exact policy context
  baseRevision: number;
  isStale: boolean;
  // kind-specific optional fields
  currentName?: string;        // for rename
  proposedName?: string;       // for rename
  currentType?: string;        // for type-change
  proposedType?: string;       // for type-change
  aliasValue?: string;         // for add/remove-alias
  proposalId?: string;         // for accept/reject-proposal
  mergeTargetId?: string;      // for merge
  mergeTargetLabel?: string;   // for merge
  splitField?: string;         // for split
}

export interface EntityActionResult {
  kind: EntityActionKind;
  newRevision: number;
  auditRecordId: string;
  description: string;         // from backend
}

export type EntityActionPhase =
  | { phase: 'idle' }
  | { phase: 'preview'; action: EntityActionPreview }
  | { phase: 'committing' }
  | { phase: 'committed'; result: EntityActionResult }
  | { phase: 'error'; message: string; canRetry: boolean };

// ─── Props ────────────────────────────────────────────────────────────────────

export interface EntityActionsProps {
  state: EntityActionPhase;
  onCommit: () => void;
  onCancel: () => void;
}

// ─── Component ────────────────────────────────────────────────────────────────

export function EntityActions(props: EntityActionsProps) {
  return (
    <div data-testid="entity-actions-root">
      <Switch>

        {/* ── Idle phase ────────────────────────────────────────────── */}
        <Match when={props.state.phase === 'idle'}>
          <div data-testid="entity-action-phase" data-phase="idle" />
        </Match>

        {/* ── Preview phase ─────────────────────────────────────────── */}
        <Match when={props.state.phase === 'preview' && props.state}>
          {(s) => {
            const action = () =>
              (s() as Extract<EntityActionPhase, { phase: 'preview' }>).action;
            return (
              <div data-testid="entity-action-phase" data-phase="preview">

                {/* Common fields */}
                <span data-testid="action-label">{action().label}</span>
                <span data-testid="action-description">{action().description}</span>
                <span data-testid="action-policy-label">{action().policyLabel}</span>
                <span data-testid="action-base-revision">{action().baseRevision}</span>

                {/* Stale warning — only when stale */}
                <Show when={action().isStale}>
                  <div data-testid="action-stale-warning" role="alert">
                    Preview is stale — base revision has changed. Refresh before committing.
                  </div>
                </Show>

                {/* Kind-specific fields */}

                {/* rename */}
                <Show when={action().currentName !== undefined}>
                  <span data-testid="rename-current">{action().currentName}</span>
                </Show>
                <Show when={action().proposedName !== undefined}>
                  <span data-testid="rename-proposed">{action().proposedName}</span>
                </Show>

                {/* type-change */}
                <Show when={action().currentType !== undefined}>
                  <span data-testid="type-current">{action().currentType}</span>
                </Show>
                <Show when={action().proposedType !== undefined}>
                  <span data-testid="type-proposed">{action().proposedType}</span>
                </Show>

                {/* add-alias / remove-alias */}
                <Show when={action().aliasValue !== undefined}>
                  <span data-testid="alias-value">{action().aliasValue}</span>
                </Show>

                {/* accept-proposal / reject-proposal */}
                <Show when={action().proposalId !== undefined}>
                  <span data-testid="proposal-id">{action().proposalId}</span>
                </Show>

                {/* merge */}
                <Show when={action().mergeTargetId !== undefined}>
                  <span data-testid="merge-target-id">{action().mergeTargetId}</span>
                </Show>
                <Show when={action().mergeTargetLabel !== undefined}>
                  <span data-testid="merge-target-label">{action().mergeTargetLabel}</span>
                </Show>

                {/* split */}
                <Show when={action().splitField !== undefined}>
                  <span data-testid="split-field">{action().splitField}</span>
                </Show>

                {/* Commit button — disabled when stale */}
                <button
                  type="button"
                  data-testid="action-commit-button"
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
                  data-testid="action-cancel-button"
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
          <div data-testid="entity-action-phase" data-phase="committing">
            <span data-testid="action-committing" role="status" aria-live="polite">
              Committing…
            </span>
          </div>
        </Match>

        {/* ── Committed phase ───────────────────────────────────────── */}
        <Match when={props.state.phase === 'committed' && props.state}>
          {(s) => {
            const result = () =>
              (s() as Extract<EntityActionPhase, { phase: 'committed' }>).result;
            return (
              <div data-testid="entity-action-phase" data-phase="committed">
                <span data-testid="action-result-revision">{result().newRevision}</span>
                <span data-testid="action-result-audit-id">{result().auditRecordId}</span>
                <span data-testid="action-result-description">{result().description}</span>
              </div>
            );
          }}
        </Match>

        {/* ── Error phase ───────────────────────────────────────────── */}
        <Match when={props.state.phase === 'error' && props.state}>
          {(s) => {
            const err = () => s() as Extract<EntityActionPhase, { phase: 'error' }>;
            return (
              <div data-testid="entity-action-phase" data-phase="error">

                <div data-testid="action-error" role="alert">
                  {err().message}
                </div>

                {/* Retry button — only when canRetry */}
                <Show when={err().canRetry}>
                  <button
                    type="button"
                    data-testid="action-retry"
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

export default EntityActions;
