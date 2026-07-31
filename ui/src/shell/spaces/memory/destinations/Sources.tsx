/**
 * Sources — Memory Control Center Sources destination (full operational content).
 *
 * Renders sources with policy, trust, consent, version, derivations, and lifecycle.
 * Provides status-gated action buttons: consent grant/revoke, candidate approve/exclude,
 * cancel, resume, and delete. Also shows the current action phase.
 *
 * Invariants (F4.2 / task 4.5.2):
 * - Root: data-testid="sources-destination"
 * - Loading: data-testid="sources-loading" role="status" — only when isLoading
 * - Error:  data-testid="sources-error"   role="alert"  — only when errorMessage non-null
 * - Sources list: data-testid="sources-list" role="list" — only when sources non-empty and not loading
 * - Empty: data-testid="sources-empty" — only when not loading and sources empty
 * - Per source: data-testid="source-{id}" data-status={source.status}
 *   - data-testid="source-label-{id}"
 *   - data-testid="source-kind-{id}"
 *   - data-testid="source-status-{id}"
 *   - data-testid="source-policy-{id}"      — policyLabel (exact)
 *   - data-testid="source-trust-{id}"       — trustLevel
 *   - data-testid="source-consent-{id}"     — consentStatus
 *   - data-testid="source-version-{id}"     — only when version non-null
 *   - data-testid="source-lifecycle-{id}"   — lifecycleLabel
 *   - data-testid="source-last-updated-{id}" — lastUpdated ISO timestamp
 *   - data-testid="source-item-count-{id}"  — only when itemCount non-null
 *   - data-testid="source-derivations-{id}" — only when derivations non-empty
 *     - Each: data-testid="source-derivation-{derivedId}"
 *   - data-testid="source-candidate-preview-{id}" — only when candidatePreview non-null AND status==='candidate'
 *   - Actions gated by status and consentStatus
 * - data-testid="source-action-phase" data-phase={actionPhase.phase} — always present
 * - Committed: data-testid="source-action-revision" and data-testid="source-action-audit"
 * - Error: data-testid="source-action-error" role="alert"
 * - UI never invents facts — all labels come from backend data.
 *
 * Requirements: F4.2 (task 4.5.2)
 */
import { For, Show, Switch, Match } from "solid-js";

// ─── Data types ───────────────────────────────────────────────────────────────

export type SourceStatus =
  | 'candidate'
  | 'active'
  | 'paused'
  | 'cancelled'
  | 'completed'
  | 'deleted';

export type SourceKind =
  | 'filesystem'
  | 'repository'
  | 'conversation'
  | 'shell-history'
  | 'library'
  | 'mcp'
  | 'openclaw'
  | 'sidecar'
  | 'import'
  | 'cloud'
  | string;

export interface SourceDerivation {
  derivedId: string;
  derivedLabel: string;
  derivedKind: string;
}

export interface Source {
  id: string;
  label: string;
  kind: SourceKind;
  status: SourceStatus;
  policyLabel: string;
  trustLevel: string;
  consentStatus: string;
  version: string | null;
  derivations: SourceDerivation[];
  lifecycleLabel: string;
  lastUpdated: string;
  itemCount: number | null;
  candidatePreview: string | null;
}

export type SourceActionPhase =
  | { phase: 'idle' }
  | { phase: 'confirming'; sourceId: string; action: 'consent' | 'revoke-consent' | 'approve-candidate' | 'exclude-candidate' | 'cancel' | 'resume' | 'delete' }
  | { phase: 'committing' }
  | { phase: 'committed'; newRevision: number; auditRecordId: string }
  | { phase: 'error'; message: string };

export interface SourcesState {
  sources: Source[];
  isLoading: boolean;
  errorMessage: string | null;
  actionPhase: SourceActionPhase;
}

// ─── Props ────────────────────────────────────────────────────────────────────

export interface SourcesProps {
  state: SourcesState;
  onConsent: (sourceId: string) => void;
  onRevokeConsent: (sourceId: string) => void;
  onApproveCandidate: (sourceId: string) => void;
  onExcludeCandidate: (sourceId: string) => void;
  onCancel: (sourceId: string) => void;
  onResume: (sourceId: string) => void;
  onDelete: (sourceId: string) => void;
  onActionCommit: () => void;
  onActionCancel: () => void;
}

// ─── Single source item ───────────────────────────────────────────────────────

function SourceItem(props: { source: Source; sourceProps: SourcesProps }) {
  const src = () => props.source;
  const id = () => props.source.id;

  return (
    <li
      data-testid={`source-${id()}`}
      data-status={src().status}
      role="listitem"
    >
      {/* Label */}
      <span data-testid={`source-label-${id()}`}>{src().label}</span>

      {/* Kind */}
      <span data-testid={`source-kind-${id()}`}>{src().kind}</span>

      {/* Status label */}
      <span data-testid={`source-status-${id()}`}>{src().status}</span>

      {/* Policy label — exact from backend */}
      <span data-testid={`source-policy-${id()}`}>{src().policyLabel}</span>

      {/* Trust level */}
      <span data-testid={`source-trust-${id()}`}>{src().trustLevel}</span>

      {/* Consent status */}
      <span data-testid={`source-consent-status-${id()}`}>{src().consentStatus}</span>

      {/* Version — only when non-null */}
      <Show when={src().version !== null}>
        <span data-testid={`source-version-${id()}`}>{src().version}</span>
      </Show>

      {/* Lifecycle label */}
      <span data-testid={`source-lifecycle-${id()}`}>{src().lifecycleLabel}</span>

      {/* Last updated ISO timestamp */}
      <span data-testid={`source-last-updated-${id()}`}>{src().lastUpdated}</span>

      {/* Item count — only when non-null */}
      <Show when={src().itemCount !== null}>
        <span data-testid={`source-item-count-${id()}`}>{src().itemCount}</span>
      </Show>

      {/* Derivations — only when non-empty */}
      <Show when={src().derivations.length > 0}>
        <div data-testid={`source-derivations-${id()}`}>
          <For each={src().derivations}>
            {(derivation) => (
              <div data-testid={`source-derivation-${derivation.derivedId}`}>
                <span>{derivation.derivedLabel}</span>
                <span>{derivation.derivedKind}</span>
              </div>
            )}
          </For>
        </div>
      </Show>

      {/* Candidate preview — only when status==='candidate' AND candidatePreview non-null */}
      <Show when={src().status === 'candidate' && src().candidatePreview !== null}>
        <div data-testid={`source-candidate-preview-${id()}`}>
          {src().candidatePreview}
        </div>
      </Show>

      {/* ── Status-specific action buttons ─────────────────────────── */}

      {/* Candidate: approve + exclude */}
      <Show when={src().status === 'candidate'}>
        <button
          type="button"
          data-testid={`source-approve-${id()}`}
          onClick={() => props.sourceProps.onApproveCandidate(id())}
        >
          Approve
        </button>
        <button
          type="button"
          data-testid={`source-exclude-${id()}`}
          onClick={() => props.sourceProps.onExcludeCandidate(id())}
        >
          Exclude
        </button>
      </Show>

      {/* Active + consentStatus !== "denied": revoke-consent */}
      <Show when={src().status === 'active' && src().consentStatus !== 'denied'}>
        <button
          type="button"
          data-testid={`source-revoke-consent-${id()}`}
          onClick={() => props.sourceProps.onRevokeConsent(id())}
        >
          Revoke Consent
        </button>
      </Show>

      {/* Active + consentStatus === "pending": grant consent */}
      <Show when={src().status === 'active' && src().consentStatus === 'pending'}>
        <button
          type="button"
          data-testid={`source-consent-${id()}`}
          onClick={() => props.sourceProps.onConsent(id())}
        >
          Grant Consent
        </button>
      </Show>

      {/* Active: cancel */}
      <Show when={src().status === 'active'}>
        <button
          type="button"
          data-testid={`source-cancel-${id()}`}
          onClick={() => props.sourceProps.onCancel(id())}
        >
          Cancel
        </button>
      </Show>

      {/* Paused or cancelled: resume */}
      <Show when={src().status === 'paused' || src().status === 'cancelled'}>
        <button
          type="button"
          data-testid={`source-resume-${id()}`}
          onClick={() => props.sourceProps.onResume(id())}
        >
          Resume
        </button>
      </Show>

      {/* Active, paused, or cancelled: delete */}
      <Show when={src().status === 'active' || src().status === 'paused' || src().status === 'cancelled'}>
        <button
          type="button"
          data-testid={`source-delete-${id()}`}
          onClick={() => props.sourceProps.onDelete(id())}
        >
          Delete
        </button>
      </Show>
    </li>
  );
}

// ─── Action phase indicator ───────────────────────────────────────────────────

function ActionPhaseIndicator(props: {
  actionPhase: SourceActionPhase;
  onActionCommit: () => void;
  onActionCancel: () => void;
}) {
  return (
    <Switch>
      <Match when={props.actionPhase.phase === 'idle'}>
        <div data-testid="source-action-phase" data-phase="idle" />
      </Match>

      <Match when={props.actionPhase.phase === 'confirming'}>
        <div data-testid="source-action-phase" data-phase="confirming">
          <button type="button" data-testid="source-action-commit" onClick={props.onActionCommit}>
            Confirm
          </button>
          <button type="button" data-testid="source-action-cancel-btn" onClick={props.onActionCancel}>
            Cancel
          </button>
        </div>
      </Match>

      <Match when={props.actionPhase.phase === 'committing'}>
        <div data-testid="source-action-phase" data-phase="committing">
          <span role="status" aria-live="polite">Committing…</span>
        </div>
      </Match>

      <Match when={props.actionPhase.phase === 'committed' && props.actionPhase}>
        {(s) => {
          const committed = () => s() as Extract<SourceActionPhase, { phase: 'committed' }>;
          return (
            <div data-testid="source-action-phase" data-phase="committed">
              <span data-testid="source-action-revision">{committed().newRevision}</span>
              <span data-testid="source-action-audit">{committed().auditRecordId}</span>
            </div>
          );
        }}
      </Match>

      <Match when={props.actionPhase.phase === 'error' && props.actionPhase}>
        {(s) => {
          const err = () => s() as Extract<SourceActionPhase, { phase: 'error' }>;
          return (
            <div data-testid="source-action-phase" data-phase="error">
              <div data-testid="source-action-error" role="alert">
                {err().message}
              </div>
            </div>
          );
        }}
      </Match>
    </Switch>
  );
}

// ─── Root component ───────────────────────────────────────────────────────────

export function Sources(props: SourcesProps) {
  const state = () => props.state;
  const hasSources = () => state().sources.length > 0;
  const showEmpty = () => !state().isLoading && !hasSources();

  return (
    <section data-testid="sources-destination" aria-label="Sources">

      {/* ── Loading indicator ──────────────────────────────────────────── */}
      <Show when={state().isLoading}>
        <span data-testid="sources-loading" role="status" aria-live="polite">
          Loading sources…
        </span>
      </Show>

      {/* ── Error ──────────────────────────────────────────────────────── */}
      <Show when={state().errorMessage !== null}>
        <div data-testid="sources-error" role="alert">
          {state().errorMessage}
        </div>
      </Show>

      {/* ── Sources list ──────────────────────────────────────────────── */}
      <Show when={!state().isLoading && hasSources()}>
        <ul data-testid="sources-list" role="list" aria-label="Sources list">
          <For each={state().sources}>
            {(source) => <SourceItem source={source} sourceProps={props} />}
          </For>
        </ul>
      </Show>

      {/* ── Empty state ───────────────────────────────────────────────── */}
      <Show when={showEmpty()}>
        <span data-testid="sources-empty">No sources</span>
      </Show>

      {/* ── Action phase ──────────────────────────────────────────────── */}
      <ActionPhaseIndicator
        actionPhase={state().actionPhase}
        onActionCommit={props.onActionCommit}
        onActionCancel={props.onActionCancel}
      />

    </section>
  );
}

export default Sources;
