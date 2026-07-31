/**
 * Inspector — Knowledge item inspector for the Memory space.
 *
 * Renders one operational claim with seven independent lazy sections:
 *   Identity, Truth, Evidence, Relationships, Use, History, Actions.
 *
 * Invariants (F4.4 / task 4.4.1):
 * - Root: data-testid="inspector-root" aria-label="Inspector"
 * - itemId===null → shows data-testid="inspector-empty" "No item selected"
 * - Seven sections: <section data-testid="inspector-section-{sectionId}">
 * - Each section has data-section-state={state} and aria-label={sectionId}
 * - Each section is independently lazy with its own state/retry/correlationId
 * - Loading: data-testid="section-loading-{sectionId}" role="status"
 * - Error: data-testid="section-error-{sectionId}" role="alert"
 * - Retry: data-testid="section-retry-{sectionId}" calls onRetrySection
 * - CorrelationId: data-testid="section-correlation-{sectionId}" when non-null
 * - GraphRevision: data-testid="section-revision-{sectionId}" when non-null
 * - All labels come from the backend; UI never invents text.
 * - Section failures are isolated — one error does not affect others.
 *
 * Requirements:
 *   MGR-005, MGR-018–019, MGR-024–025, MGR-040–041
 *   MGD-001, MGD-027, MGD-036
 *   MG-C02, MG-C04, MG-C07, MG-H11, MG-O01–O04, MG-O08, MG-O10–O13
 */
import { For, Show } from "solid-js";

// ─── State types ──────────────────────────────────────────────────────────────

export type InspectorSectionState =
  | 'idle'
  | 'loading'
  | 'ready'
  | 'empty'
  | 'partial'
  | 'stale'
  | 'offline'
  | 'error';

// ─── Per-section metadata ─────────────────────────────────────────────────────

export interface InspectorSectionMeta {
  state: InspectorSectionState;
  correlationId: string | null;
  graphRevision: number | null;
  lastLoadedAt: string | null;
  errorMessage: string | null;
  isRetrying: boolean;
}

// ─── Section data types ───────────────────────────────────────────────────────

export interface IdentitySection extends InspectorSectionMeta {
  sectionId: 'identity';
  itemId: string | null;
  kind: string | null;
  displayName: string | null;
  aliases: string[];
  authorityClass: string | null;
  policyLabel: string | null;
  validTimeStart: string | null;
  validTimeEnd: string | null;
  transactionTime: string | null;
}

export interface TruthSection extends InspectorSectionMeta {
  sectionId: 'truth';
  truthState: string | null;
  truthReason: string | null;
  contradictionCount: number | null;
  lastVerified: string | null;
  provenanceLabel: string | null;
}

export interface EvidenceSection extends InspectorSectionMeta {
  sectionId: 'evidence';
  evidenceItems: EvidenceItem[];
  totalCount: number | null;
  hasMore: boolean;
}

export interface EvidenceItem {
  id: string;
  source: string;
  locator: string | null;
  method: string;
  version: string;
  polarity: 'support' | 'contradict';
  score: number | null;
  semanticsLabel: string;
  policyLabel: string | null;
}

export interface RelationshipsSection extends InspectorSectionMeta {
  sectionId: 'relationships';
  relations: RelationItem[];
  totalCount: number | null;
  hasMore: boolean;
}

export interface RelationItem {
  id: string;
  direction: 'outgoing' | 'incoming' | 'symmetric';
  registryLabel: string;
  sourceLabel: string;
  targetLabel: string;
  evidenceCount: number;
  validity: string;
}

export interface UseSection extends InspectorSectionMeta {
  sectionId: 'use';
  whyStored: string | null;
  whyRecalled: string | null;
  howUsed: string | null;
  usedInTraceCount: number | null;
}

export interface HistorySection extends InspectorSectionMeta {
  sectionId: 'history';
  events: HistoryEvent[];
  totalCount: number | null;
  hasMore: boolean;
}

export interface HistoryEvent {
  id: string;
  eventType: string;
  timestamp: string;
  actor: string | null;
  description: string;
}

export interface ActionsSection extends InspectorSectionMeta {
  sectionId: 'actions';
  availableActions: InspectorAction[];
}

export interface InspectorAction {
  id: string;
  label: string;
  isEnabled: boolean;
  isDangerous: boolean;
  requiresPreview: boolean;
}

// ─── Complete inspector state ─────────────────────────────────────────────────

export interface InspectorState {
  itemId: string | null;
  identity: IdentitySection;
  truth: TruthSection;
  evidence: EvidenceSection;
  relationships: RelationshipsSection;
  use: UseSection;
  history: HistorySection;
  actions: ActionsSection;
}

// ─── Props ────────────────────────────────────────────────────────────────────

export interface InspectorProps {
  state: InspectorState;
  onRetrySection: (sectionId: string) => void;
  onAction: (actionId: string) => void;
  onNavigate: (target: string) => void;
}

// ─── Section shell helpers ────────────────────────────────────────────────────

/**
 * Renders the common section header chrome: loading indicator, error banner,
 * retry button, correlation ID, graph revision.
 * All data fields come from meta — UI never invents text.
 */
function SectionMeta(props: {
  sectionId: string;
  meta: InspectorSectionMeta;
  onRetry: () => void;
}) {
  return (
    <>
      {/* Loading */}
      <Show when={props.meta.state === 'loading'}>
        <span
          data-testid={`section-loading-${props.sectionId}`}
          role="status"
          aria-live="polite"
        >
          Loading…
        </span>
      </Show>

      {/* Error */}
      <Show when={props.meta.state === 'error'}>
        <div
          data-testid={`section-error-${props.sectionId}`}
          role="alert"
        >
          <span>{props.meta.errorMessage}</span>
          <button
            type="button"
            data-testid={`section-retry-${props.sectionId}`}
            onClick={props.onRetry}
          >
            Retry
          </button>
        </div>
      </Show>

      {/* Correlation ID — only when present */}
      <Show when={props.meta.correlationId !== null}>
        <span data-testid={`section-correlation-${props.sectionId}`}>
          {props.meta.correlationId}
        </span>
      </Show>

      {/* Graph revision — only when present */}
      <Show when={props.meta.graphRevision !== null}>
        <span data-testid={`section-revision-${props.sectionId}`}>
          {props.meta.graphRevision}
        </span>
      </Show>
    </>
  );
}

// ─── Component ────────────────────────────────────────────────────────────────

export function Inspector(props: InspectorProps) {
  const s = () => props.state;

  return (
    <div data-testid="inspector-root" aria-label="Inspector">

      {/* ── Empty: no item selected ───────────────────────────────────── */}
      <Show when={s().itemId === null}>
        <div data-testid="inspector-empty">No item selected</div>
      </Show>

      {/* ── Seven sections — always rendered so each is independently lazy ─ */}

      {/* 1. Identity ──────────────────────────────────────────────────── */}
      <section
        data-testid="inspector-section-identity"
        data-section-state={s().identity.state}
        aria-label="identity"
      >
        <SectionMeta
          sectionId="identity"
          meta={s().identity}
          onRetry={() => props.onRetrySection('identity')}
        />
        <Show when={s().identity.state === 'ready' || s().identity.state === 'partial' || s().identity.state === 'stale'}>
          <Show when={s().identity.itemId !== null}>
            <span data-field="item-id">{s().identity.itemId}</span>
          </Show>
          <Show when={s().identity.kind !== null}>
            <span data-field="kind">{s().identity.kind}</span>
          </Show>
          <Show when={s().identity.displayName !== null}>
            <span data-field="display-name">{s().identity.displayName}</span>
          </Show>
          <Show when={s().identity.authorityClass !== null}>
            <span data-field="authority-class">{s().identity.authorityClass}</span>
          </Show>
          <Show when={s().identity.policyLabel !== null}>
            <span data-field="policy-label">{s().identity.policyLabel}</span>
          </Show>
          <Show when={s().identity.validTimeStart !== null || s().identity.validTimeEnd !== null}>
            <span data-field="valid-time">
              {s().identity.validTimeStart ?? ''}
              {' / '}
              {s().identity.validTimeEnd ?? ''}
            </span>
          </Show>
          <Show when={s().identity.transactionTime !== null}>
            <span data-field="transaction-time">{s().identity.transactionTime}</span>
          </Show>
        </Show>
      </section>

      {/* 2. Truth ─────────────────────────────────────────────────────── */}
      <section
        data-testid="inspector-section-truth"
        data-section-state={s().truth.state}
        aria-label="truth"
      >
        <SectionMeta
          sectionId="truth"
          meta={s().truth}
          onRetry={() => props.onRetrySection('truth')}
        />
        <Show when={s().truth.state === 'ready' || s().truth.state === 'partial' || s().truth.state === 'stale'}>
          <Show when={s().truth.truthState !== null}>
            <span
              data-field="truth-state"
              data-truth-state={s().truth.truthState!}
            >
              {s().truth.truthState}
            </span>
          </Show>
          <Show when={s().truth.truthReason !== null}>
            <span data-field="truth-reason">{s().truth.truthReason}</span>
          </Show>
          <Show when={s().truth.contradictionCount !== null}>
            <span data-field="contradiction-count">{s().truth.contradictionCount}</span>
          </Show>
          <Show when={s().truth.provenanceLabel !== null}>
            <span data-field="provenance-label">{s().truth.provenanceLabel}</span>
          </Show>
        </Show>
      </section>

      {/* 3. Evidence ──────────────────────────────────────────────────── */}
      <section
        data-testid="inspector-section-evidence"
        data-section-state={s().evidence.state}
        aria-label="evidence"
      >
        <SectionMeta
          sectionId="evidence"
          meta={s().evidence}
          onRetry={() => props.onRetrySection('evidence')}
        />
        <Show when={s().evidence.state === 'ready' || s().evidence.state === 'partial' || s().evidence.state === 'stale'}>
          <ul data-testid="evidence-list" aria-label="Evidence items">
            <For each={s().evidence.evidenceItems}>
              {(item) => (
                <li
                  data-testid={`evidence-item-${item.id}`}
                  data-field="polarity"
                  data-polarity={item.polarity}
                >
                  <span data-field="source">{item.source}</span>
                  <span data-field="method">{item.method}</span>
                  <span data-field="version">{item.version}</span>
                  <span data-field="semantics-label">{item.semanticsLabel}</span>
                  <Show when={item.policyLabel !== null}>
                    <span data-field="policy-label">{item.policyLabel}</span>
                  </Show>
                  <Show when={item.score !== null}>
                    <span data-field="score">{item.score}</span>
                  </Show>
                </li>
              )}
            </For>
          </ul>
        </Show>
      </section>

      {/* 4. Relationships ─────────────────────────────────────────────── */}
      <section
        data-testid="inspector-section-relationships"
        data-section-state={s().relationships.state}
        aria-label="relationships"
      >
        <SectionMeta
          sectionId="relationships"
          meta={s().relationships}
          onRetry={() => props.onRetrySection('relationships')}
        />
        <Show when={s().relationships.state === 'ready' || s().relationships.state === 'partial' || s().relationships.state === 'stale'}>
          <ul data-testid="relations-list" aria-label="Relationship items">
            <For each={s().relationships.relations}>
              {(rel) => (
                <li
                  data-testid={`relation-item-${rel.id}`}
                  data-field="direction"
                  data-direction={rel.direction}
                >
                  <span data-field="registry-label">{rel.registryLabel}</span>
                  <span data-field="source-label">{rel.sourceLabel}</span>
                  <span data-field="target-label">{rel.targetLabel}</span>
                  <span data-field="evidence-count">{rel.evidenceCount}</span>
                  <span data-field="validity">{rel.validity}</span>
                </li>
              )}
            </For>
          </ul>
        </Show>
      </section>

      {/* 5. Use ───────────────────────────────────────────────────────── */}
      <section
        data-testid="inspector-section-use"
        data-section-state={s().use.state}
        aria-label="use"
      >
        <SectionMeta
          sectionId="use"
          meta={s().use}
          onRetry={() => props.onRetrySection('use')}
        />
        <Show when={s().use.state === 'ready' || s().use.state === 'partial' || s().use.state === 'stale'}>
          <Show when={s().use.whyStored !== null}>
            <span data-field="why-stored">{s().use.whyStored}</span>
          </Show>
          <Show when={s().use.whyRecalled !== null}>
            <span data-field="why-recalled">{s().use.whyRecalled}</span>
          </Show>
          <Show when={s().use.howUsed !== null}>
            <span data-field="how-used">{s().use.howUsed}</span>
          </Show>
          <Show when={s().use.usedInTraceCount !== null}>
            <span data-field="used-in-trace-count">{s().use.usedInTraceCount}</span>
          </Show>
        </Show>
      </section>

      {/* 6. History ───────────────────────────────────────────────────── */}
      <section
        data-testid="inspector-section-history"
        data-section-state={s().history.state}
        aria-label="history"
      >
        <SectionMeta
          sectionId="history"
          meta={s().history}
          onRetry={() => props.onRetrySection('history')}
        />
        <Show when={s().history.state === 'ready' || s().history.state === 'partial' || s().history.state === 'stale'}>
          <ul data-testid="history-list" aria-label="History events">
            <For each={s().history.events}>
              {(event) => (
                <li
                  data-testid={`history-event-${event.id}`}
                  data-field="event-type"
                  data-event-type={event.eventType}
                >
                  <span data-field="description">{event.description}</span>
                  <span data-field="timestamp">{event.timestamp}</span>
                  <Show when={event.actor !== null}>
                    <span data-field="actor">{event.actor}</span>
                  </Show>
                </li>
              )}
            </For>
          </ul>
        </Show>
      </section>

      {/* 7. Actions ───────────────────────────────────────────────────── */}
      <section
        data-testid="inspector-section-actions"
        data-section-state={s().actions.state}
        aria-label="actions"
      >
        <SectionMeta
          sectionId="actions"
          meta={s().actions}
          onRetry={() => props.onRetrySection('actions')}
        />
        <Show when={s().actions.state === 'ready' || s().actions.state === 'partial' || s().actions.state === 'stale'}>
          <ul data-testid="actions-list" aria-label="Available actions">
            <For each={s().actions.availableActions}>
              {(action) => (
                <li>
                  <button
                    type="button"
                    data-testid={`inspector-action-${action.id}`}
                    aria-label={action.label}
                    disabled={!action.isEnabled}
                    aria-disabled={!action.isEnabled ? "true" : undefined}
                    data-dangerous={String(action.isDangerous)}
                    onClick={() => {
                      if (action.isEnabled) {
                        props.onAction(action.id);
                      }
                    }}
                  >
                    {action.label}
                  </button>
                </li>
              )}
            </For>
          </ul>
        </Show>
      </section>

    </div>
  );
}

export default Inspector;
