/**
 * LifecycleActions — Forget / Restore / Hard Delete lifecycle workflow component.
 *
 * Handles three lifecycle action kinds with dependency choices, 30-day restore
 * window, reconciliation status per derived index, plain-language crypto state,
 * and focus containment/restoration.
 *
 * Critical invariants (F4.4 / task 4.4.8):
 * - Root: data-testid="lifecycle-actions-root"
 * - data-testid="lifecycle-action-phase" carries data-phase attribute per phase
 * - idle: renders only root + phase element, no content
 * - All copy comes from backend data; UI never invents facts or labels.
 * - Commit disabled when isStale=true OR any dependency has null choice (for
 *   forget/hard-delete with dependencies). Restore never requires dep choices.
 * - Stale warning includes role="alert".
 * - Error message includes role="alert".
 * - Retry button only rendered when canRetry=true.
 * - CRITICAL: The text "Crypto-shredded" and "Cryptographically erased" may
 *   NEVER appear unless cryptoProof is non-null. No exceptions.
 * - Hard Delete without crypto capability shows "Hard Delete (content marked
 *   for removal)" — NEVER "Crypto-shredded" or equivalent.
 * - Reconciliation list only rendered when non-empty.
 * - Dependency list only rendered when non-empty.
 * - Restore kind never requires dependency choices.
 * - After committed action, returnFocusRef (if provided) is called to restore
 *   keyboard focus to the initiating element.
 *
 * Requirements: F4.4 (task 4.4.8)
 */
import { Show, For, Switch, Match, onMount } from "solid-js";

// ─── Data types ───────────────────────────────────────────────────────────────

export type LifecycleActionKind = 'forget' | 'restore' | 'hard-delete';

export interface DependencyItem {
  itemId: string;
  label: string;
  kind: string;
  choice: 'cascade' | 'keep-independent-evidence' | null;  // null = not yet chosen
}

export interface ReconciliationStatus {
  indexName: string;    // e.g. "fts5", "vector", "graph", "trace", "cache"
  status: 'pending' | 'in-progress' | 'complete' | 'failed';
}

export interface CryptoState {
  hasCryptoCapability: boolean;
  cryptoProof: string | null;   // ONLY set when crypto erasure actually occurred
  pendingErasure: boolean;      // true when scheduled but not yet confirmed
}

export interface LifecyclePreview {
  kind: LifecycleActionKind;
  itemId: string;
  itemLabel: string;
  description: string;        // from backend
  policyLabel: string;
  baseRevision: number;
  isStale: boolean;
  dependencies: DependencyItem[];
  // For forget:
  restoreWindowDays: number | null;  // e.g. 30
  restoreUntil: string | null;       // ISO timestamp
  // For hard-delete:
  cryptoState: CryptoState | null;
  // Reconciliation
  reconciliationStatuses: ReconciliationStatus[];
}

export interface LifecycleResult {
  kind: LifecycleActionKind;
  newRevision: number;
  auditRecordId: string;
  description: string;         // from backend
  // For forget:
  restoreUntil: string | null;
  // For hard-delete:
  cryptoState: CryptoState | null;
  reconciliationStatuses: ReconciliationStatus[];
}

export type LifecycleActionPhase =
  | { phase: 'idle' }
  | { phase: 'preview'; action: LifecyclePreview }
  | { phase: 'committing' }
  | { phase: 'committed'; result: LifecycleResult }
  | { phase: 'error'; message: string; canRetry: boolean };

// ─── Props ────────────────────────────────────────────────────────────────────

export interface LifecycleActionsProps {
  state: LifecycleActionPhase;
  onDependencyChoice: (itemId: string, choice: 'cascade' | 'keep-independent-evidence') => void;
  onCommit: () => void;
  onCancel: () => void;
  // For focus restoration — the ref to return focus to
  returnFocusRef?: () => HTMLElement | null;
}

// ─── Helper: Crypto State View ────────────────────────────────────────────────

interface CryptoStateViewProps {
  cryptoState: CryptoState;
  prefix: string; // "preview" or "result"
}

function CryptoStateView(props: CryptoStateViewProps) {
  return (
    <div>
      {/* Capability indicator */}
      <span
        data-testid="crypto-capability"
        data-has-capability={props.cryptoState.hasCryptoCapability ? "true" : "false"}
      >
        {props.cryptoState.hasCryptoCapability
          ? "Cryptographic erasure available"
          : "Cryptographic erasure not available"}
      </span>

      {/* Proof confirmed — ONLY when cryptoProof is non-null.
          INVARIANT: Never show "Crypto-shredded" / "Cryptographically erased"
          unless cryptoProof is non-null. This is the ONLY place such text may appear. */}
      <Show when={props.cryptoState.cryptoProof !== null}>
        <span data-testid="crypto-proof-confirmed">
          Cryptographic erasure confirmed — proof: {props.cryptoState.cryptoProof}
        </span>
      </Show>

      {/* Pending erasure — content deletion in progress (plain language only) */}
      <Show when={props.cryptoState.pendingErasure && props.cryptoState.cryptoProof === null}>
        <span data-testid="crypto-pending">
          Content deletion in progress
        </span>
      </Show>

      {/* Unavailable — crypto capability absent AND no proof.
          Uses plain language: "Hard Delete (content marked for removal)"
          NEVER "Crypto-shredded" or "Cryptographically erased" here. */}
      <Show when={!props.cryptoState.hasCryptoCapability && props.cryptoState.cryptoProof === null}>
        <span data-testid="crypto-unavailable">
          Hard Delete (content marked for removal)
        </span>
      </Show>
    </div>
  );
}

// ─── Helper: Reconciliation List View ─────────────────────────────────────────

interface ReconciliationListViewProps {
  statuses: ReconciliationStatus[];
  testId: string;
}

function ReconciliationListView(props: ReconciliationListViewProps) {
  return (
    <Show when={props.statuses.length > 0}>
      <ul data-testid={props.testId}>
        <For each={props.statuses}>
          {(item) => (
            <li
              data-testid={`reconciliation-${item.indexName}`}
              data-status={item.status}
            >
              {item.indexName}: {item.status}
            </li>
          )}
        </For>
      </ul>
    </Show>
  );
}

// ─── Helper: is commit disabled? ─────────────────────────────────────────────

function isCommitDisabled(action: LifecyclePreview): boolean {
  if (action.isStale) return true;
  // Restore never requires dependency choices
  if (action.kind === 'restore') return false;
  // Forget / hard-delete: all dependencies must have a choice
  if (action.dependencies.length > 0) {
    return action.dependencies.some((d) => d.choice === null);
  }
  return false;
}

// ─── Component ────────────────────────────────────────────────────────────────

export function LifecycleActions(props: LifecycleActionsProps) {
  return (
    <div data-testid="lifecycle-actions-root">
      <Switch>

        {/* ── Idle phase ────────────────────────────────────────────── */}
        <Match when={props.state.phase === 'idle'}>
          <div data-testid="lifecycle-action-phase" data-phase="idle" />
        </Match>

        {/* ── Preview phase ─────────────────────────────────────────── */}
        <Match when={props.state.phase === 'preview' && props.state}>
          {(s) => {
            const action = () =>
              (s() as Extract<LifecycleActionPhase, { phase: 'preview' }>).action;
            return (
              <div data-testid="lifecycle-action-phase" data-phase="preview">

                {/* Kind / item / description / policy / revision */}
                <span data-testid="lifecycle-action-kind">{action().kind}</span>
                <span data-testid="lifecycle-item-label">{action().itemLabel}</span>
                <span data-testid="lifecycle-description">{action().description}</span>
                <span data-testid="lifecycle-policy-label">{action().policyLabel}</span>
                <span data-testid="lifecycle-base-revision">{action().baseRevision}</span>

                {/* Stale warning */}
                <Show when={action().isStale}>
                  <div data-testid="lifecycle-stale-warning" role="alert">
                    Preview is stale — base revision has changed. Refresh before committing.
                  </div>
                </Show>

                {/* Dependencies list — only when non-empty */}
                <Show when={action().dependencies.length > 0}>
                  <ul data-testid="dependencies-list">
                    <For each={action().dependencies}>
                      {(dep) => (
                        <li
                          data-testid={`dependency-${dep.itemId}`}
                          data-selected-choice={dep.choice ?? 'none'}
                        >
                          <span data-field="dep-label">{dep.label}</span>
                          <span data-field="dep-kind">{dep.kind}</span>
                          <button
                            type="button"
                            data-testid={`dep-cascade-${dep.itemId}`}
                            onClick={() => props.onDependencyChoice(dep.itemId, 'cascade')}
                          >
                            Cascade
                          </button>
                          <button
                            type="button"
                            data-testid={`dep-keep-${dep.itemId}`}
                            onClick={() =>
                              props.onDependencyChoice(dep.itemId, 'keep-independent-evidence')
                            }
                          >
                            Keep independent evidence
                          </button>
                        </li>
                      )}
                    </For>
                  </ul>
                </Show>

                {/* Forget-specific: restore window and restore-until */}
                <Show when={action().kind === 'forget' && action().restoreWindowDays !== null}>
                  <span data-testid="restore-window">{action().restoreWindowDays}</span>
                </Show>
                <Show when={action().kind === 'forget' && action().restoreUntil !== null}>
                  <span data-testid="restore-until">{action().restoreUntil}</span>
                </Show>

                {/* Hard-delete crypto state */}
                <Show when={action().kind === 'hard-delete' && action().cryptoState !== null}>
                  <CryptoStateView cryptoState={action().cryptoState!} prefix="preview" />
                </Show>

                {/* Reconciliation list — only when non-empty */}
                <ReconciliationListView
                  statuses={action().reconciliationStatuses}
                  testId="reconciliation-list"
                />

                {/* Commit button */}
                <button
                  type="button"
                  data-testid="lifecycle-commit-button"
                  disabled={isCommitDisabled(action())}
                  onClick={() => {
                    if (!isCommitDisabled(action())) {
                      props.onCommit();
                    }
                  }}
                >
                  Commit
                </button>

                {/* Cancel button */}
                <button
                  type="button"
                  data-testid="lifecycle-cancel-button"
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
          <div data-testid="lifecycle-action-phase" data-phase="committing">
            <span data-testid="lifecycle-committing" role="status" aria-live="polite">
              Committing…
            </span>
          </div>
        </Match>

        {/* ── Committed phase ───────────────────────────────────────── */}
        <Match when={props.state.phase === 'committed' && props.state}>
          {(s) => {
            const result = () =>
              (s() as Extract<LifecycleActionPhase, { phase: 'committed' }>).result;

            // Restore focus to the initiating element after commit
            onMount(() => {
              const el = props.returnFocusRef?.();
              if (el) {
                el.focus();
              }
            });

            return (
              <div data-testid="lifecycle-action-phase" data-phase="committed">

                <span data-testid="lifecycle-result-revision">{result().newRevision}</span>
                <span data-testid="lifecycle-result-audit-id">{result().auditRecordId}</span>
                <span data-testid="lifecycle-result-description">{result().description}</span>

                {/* restore-until — only when non-null */}
                <Show when={result().restoreUntil !== null}>
                  <span data-testid="lifecycle-result-restore-until">
                    {result().restoreUntil}
                  </span>
                </Show>

                {/* Crypto result — same invariants: no claim without proof */}
                <Show when={result().cryptoState !== null}>
                  <CryptoStateView cryptoState={result().cryptoState!} prefix="result" />
                </Show>

                {/* Reconciliation list */}
                <ReconciliationListView
                  statuses={result().reconciliationStatuses}
                  testId="lifecycle-result-reconciliation-list"
                />

              </div>
            );
          }}
        </Match>

        {/* ── Error phase ───────────────────────────────────────────── */}
        <Match when={props.state.phase === 'error' && props.state}>
          {(s) => {
            const err = () => s() as Extract<LifecycleActionPhase, { phase: 'error' }>;
            return (
              <div data-testid="lifecycle-action-phase" data-phase="error">

                <div data-testid="lifecycle-error" role="alert">
                  {err().message}
                </div>

                {/* Retry button — only when canRetry */}
                <Show when={err().canRetry}>
                  <button
                    type="button"
                    data-testid="lifecycle-retry"
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

export default LifecycleActions;
