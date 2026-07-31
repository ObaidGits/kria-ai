/**
 * Timeline — Memory Control Center Timeline destination (full operational content).
 *
 * Renders valid-time / transaction-time snapshot/diff capability with
 * change kind filter toggles, query metadata (range, timezone, revision),
 * and a paginated list of classified changes.
 *
 * Invariants (F4.2 / task 4.5.3 / MGR-010):
 * - When capability === 'unavailable': render only data-testid="timeline-unavailable",
 *   no controls, no filters, no timeline items — nothing else.
 * - When capability is 'full', 'valid-time-only', or 'transaction-time-only':
 *   render full UI under data-testid="timeline-destination".
 * - Capability badge: data-testid="timeline-capability" data-capability={capability}.
 * - Range, timezone, revision shown only when non-null in query.
 * - Filter toggles shown per change kind; data-active reflects inclusion.
 * - Loading: data-testid="timeline-loading" role="status".
 * - Error: data-testid="timeline-error" role="alert".
 * - Changes list: data-testid="timeline-changes-list" role="list" — only when non-empty.
 * - Empty: data-testid="timeline-empty" — only when not loading and changes empty.
 * - Load more: data-testid="timeline-load-more" — only when hasMore=true.
 * - Per-change: data-testid="timeline-change-{id}" data-change-kind={kind}.
 * - UI never invents facts — all labels and descriptions come from backend data.
 *
 * Requirements: MGR-010 (temporal graph correctness), F4.2 task 4.5.3.
 */
import { For, Show } from "solid-js";

// ─── Data types ───────────────────────────────────────────────────────────────

export type TimelineCapability =
  | "full"
  | "valid-time-only"
  | "transaction-time-only"
  | "unavailable";

export type TimelineChangeKind =
  | "addition"
  | "expiry"
  | "contradiction"
  | "supersession"
  | "correction";

export interface TimelineChange {
  id: string;
  kind: TimelineChangeKind;
  /** Human-readable label from backend. */
  label: string;
  validTimeStart: string | null;
  validTimeEnd: string | null;
  transactionRevision: number;
  transactionTime: string;
  /** Human-readable description from backend. */
  description: string;
}

export interface TimelineQuery {
  rangeStart: string | null;
  rangeEnd: string | null;
  timezone: string | null;
  graphRevision: number | null;
  includeChangeKinds: TimelineChangeKind[];
}

export interface TimelineState {
  capability: TimelineCapability;
  isLoading: boolean;
  errorMessage: string | null;
  query: TimelineQuery;
  changes: TimelineChange[];
  hasMore: boolean;
  cursorToken: string | null;
}

// ─── Props ────────────────────────────────────────────────────────────────────

export interface TimelineProps {
  state: TimelineState;
  onLoadMore: () => void;
  onChangeKindToggle: (kind: TimelineChangeKind) => void;
}

// ─── Capability-filtered kinds ────────────────────────────────────────────────

/** All five change kinds are shown for any available capability. */
const ALL_CHANGE_KINDS: TimelineChangeKind[] = [
  "addition",
  "expiry",
  "contradiction",
  "supersession",
  "correction",
];

/** Map a kind to a short human-readable label. */
function changeKindLabel(kind: TimelineChangeKind): string {
  switch (kind) {
    case "addition":
      return "Addition";
    case "expiry":
      return "Expiry";
    case "contradiction":
      return "Contradiction";
    case "supersession":
      return "Supersession";
    case "correction":
      return "Correction";
  }
}

// ─── Single change item ───────────────────────────────────────────────────────

function ChangeItem(props: { change: TimelineChange }) {
  const c = () => props.change;
  const id = () => props.change.id;

  return (
    <li
      data-testid={`timeline-change-${id()}`}
      data-change-kind={c().kind}
      role="listitem"
    >
      {/* Kind label */}
      <span data-testid={`change-kind-${id()}`}>{changeKindLabel(c().kind)}</span>

      {/* Backend label */}
      <span data-testid={`change-label-${id()}`}>{c().label}</span>

      {/* Backend description */}
      <span data-testid={`change-description-${id()}`}>{c().description}</span>

      {/* Transaction time */}
      <time data-testid={`change-transaction-time-${id()}`}>{c().transactionTime}</time>

      {/* Transaction revision */}
      <span data-testid={`change-revision-${id()}`}>{c().transactionRevision}</span>

      {/* Valid time start — only when non-null */}
      <Show when={c().validTimeStart !== null}>
        <time data-testid={`change-valid-start-${id()}`}>{c().validTimeStart}</time>
      </Show>

      {/* Valid time end — only when non-null */}
      <Show when={c().validTimeEnd !== null}>
        <time data-testid={`change-valid-end-${id()}`}>{c().validTimeEnd}</time>
      </Show>
    </li>
  );
}

// ─── Root component ───────────────────────────────────────────────────────────

export function Timeline(props: TimelineProps) {
  const state = () => props.state;
  const capability = () => state().capability;

  // Non-negotiable invariant: unavailable → show only the unavailable notice.
  return (
    <Show
      when={capability() !== "unavailable"}
      fallback={
        <div data-testid="timeline-unavailable">
          Timeline is not available for this context.
        </div>
      }
    >
      <section data-testid="timeline-destination" aria-label="Timeline">

        {/* ── Capability badge ──────────────────────────────────────────── */}
        <div data-testid="timeline-capability" data-capability={capability()} />

        {/* ── Query metadata ────────────────────────────────────────────── */}
        <Show when={state().query.rangeStart !== null}>
          <span data-testid="timeline-range-start">{state().query.rangeStart}</span>
        </Show>

        <Show when={state().query.rangeEnd !== null}>
          <span data-testid="timeline-range-end">{state().query.rangeEnd}</span>
        </Show>

        <Show when={state().query.timezone !== null}>
          <span data-testid="timeline-timezone">{state().query.timezone}</span>
        </Show>

        <Show when={state().query.graphRevision !== null}>
          <span data-testid="timeline-revision">{state().query.graphRevision}</span>
        </Show>

        {/* ── Change kind filter toggles ─────────────────────────────────── */}
        <div role="group" aria-label="Change kind filters">
          <For each={ALL_CHANGE_KINDS}>
            {(kind) => (
              <button
                type="button"
                data-testid={`timeline-filter-${kind}`}
                data-active={
                  state().query.includeChangeKinds.includes(kind) ? "true" : "false"
                }
                onClick={() => props.onChangeKindToggle(kind)}
              >
                {changeKindLabel(kind)}
              </button>
            )}
          </For>
        </div>

        {/* ── Loading ───────────────────────────────────────────────────── */}
        <Show when={state().isLoading}>
          <span data-testid="timeline-loading" role="status" aria-live="polite">
            Loading timeline…
          </span>
        </Show>

        {/* ── Error ─────────────────────────────────────────────────────── */}
        <Show when={state().errorMessage !== null}>
          <div data-testid="timeline-error" role="alert">
            {state().errorMessage}
          </div>
        </Show>

        {/* ── Changes list ──────────────────────────────────────────────── */}
        <Show when={state().changes.length > 0}>
          <ul
            data-testid="timeline-changes-list"
            role="list"
            aria-label="Timeline changes"
          >
            <For each={state().changes}>
              {(change) => <ChangeItem change={change} />}
            </For>
          </ul>
        </Show>

        {/* ── Empty state ───────────────────────────────────────────────── */}
        <Show when={!state().isLoading && state().changes.length === 0}>
          <span data-testid="timeline-empty">No changes in this range</span>
        </Show>

        {/* ── Load more ─────────────────────────────────────────────────── */}
        <Show when={state().hasMore}>
          <button
            type="button"
            data-testid="timeline-load-more"
            onClick={() => props.onLoadMore()}
          >
            Load more
          </button>
        </Show>

      </section>
    </Show>
  );
}

export default Timeline;
