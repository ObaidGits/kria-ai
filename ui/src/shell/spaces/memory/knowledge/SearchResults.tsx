/**
 * SearchResults — Ranked result list for the Recall destination.
 *
 * Renders a mixed list of search result items from any kind (Entity, Memory,
 * Relation, Source, Goal, etc.). Each item surfaces every required field from
 * the backend DTO without invention or inference.
 *
 * Invariants (F4.3 / task 4.3.2):
 * - relativeScore is always labeled "Rank: X%" — never "confidence",
 *   "certainty", or "probability".
 * - policySummary is shown as-is from the backend — no synthetic gap/count
 *   labels are rendered.
 * - truthState is always rendered per item, with a data-truth-state attribute.
 * - navigationTarget is rendered as a button that calls onNavigate.
 * - No invented stats: no "health score", "wellness", "certainty", or
 *   fabricated topology claims.
 * - Loading shows a role="status" indicator.
 * - idle state renders nothing (empty).
 * - no-results state: exact copy "No results found. Your filters are preserved."
 *   — never claims the store is empty.
 * - error state shows the error message without claiming the store is empty.
 * - partial state shows results with a notice about partial availability.
 *
 * Requirements: MGR-006, MGR-014, MGR-023–024, MGR-031;
 *   MGD-026, MGD-030, MGD-046; MG-H01, MG-H04, MG-H10–H12, MG-O05, MG-O25.
 */
import { For, Show } from "solid-js";

// ─── Data types ───────────────────────────────────────────────────────────────

/**
 * One ranked result item from the Retrieval Engine v2 API.
 *
 * All fields are passed through from the backend DTO unchanged; the UI never
 * invents or infers values for any of them.
 */
export interface SearchResultItem {
  /** Stable semantic ID for this result. */
  id: string;
  /** Record kind: "memory" | "entity" | "relation" | "source" | "goal" | … */
  kind: string;
  /** Which field matched the query, e.g. "title", "body", "alias", "name". */
  matchedField: string;
  /** Human-readable retrieval rationale from the backend — never invented. */
  rationale: string;
  /**
   * Relative rank score in 0.0–1.0. This is a rank-based relative metric, not
   * a raw confidence float or calibrated probability. Always shown as "Rank: X%".
   */
  relativeScore: number;
  /** ID of the retrieval profile used (e.g. "balanced-v1", "rrf-general-v1"). */
  profileId: string;
  /**
   * Policy context string — exact text from the backend, shown verbatim.
   * No synthetic gap/hidden-scope labels or color/count indicators are added.
   */
  policySummary: string;
  /**
   * Truth state for this result item, e.g. "Current", "Stale", "Contradicted",
   * "Unverified", "Superseded", "Inferred", "Confirmed", "Forgotten", "Deleted",
   * "Unavailable". Always rendered; never suppressed.
   */
  truthState: string;
  /** Graph revision from which this result was drawn. */
  graphRevision: number;
  /** Source identifier, if applicable. */
  sourceId: string | null;
  /** Human-readable label for the source, if applicable. */
  sourceLabel: string | null;
  /** ISO timestamp for valid time start, if applicable. */
  validTimeStart: string | null;
  /** ISO timestamp for valid time end, if applicable. */
  validTimeEnd: string | null;
  /** ISO timestamp when this record was stored (transaction time). */
  transactionTime: string;
  /**
   * Navigation destination for this result, e.g.
   * "knowledge?inspect=<id>". Used as the argument to onNavigate.
   */
  navigationTarget: string;
  /** Short display summary of the item's content. */
  summary: string;
}

// ─── Props ────────────────────────────────────────────────────────────────────

export interface SearchResultsProps {
  /** The ranked result items to display. */
  items: SearchResultItem[];
  /** True while a search request is in-flight. */
  isLoading: boolean;
  /** Current state of the result set. */
  resultState: "idle" | "searching" | "results" | "no-results" | "error" | "partial";
  /** Error message shown when resultState === "error". */
  errorMessage?: string;
  /** Called when the user clicks the navigation link for a result. */
  onNavigate: (target: string) => void;
  /** Called when the user activates a result (Enter key or click). */
  onSelect: (item: SearchResultItem) => void;
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

/**
 * Format a 0.0–1.0 relative score as "Rank: X%".
 * Clamps to [0, 100] and rounds to the nearest integer.
 * Never uses the word "confidence", "certainty", or "probability".
 */
function formatRelativeScore(score: number): string {
  const pct = Math.round(Math.max(0, Math.min(1, score)) * 100);
  return `Rank: ${pct}%`;
}

/**
 * Format a valid time range for display.
 * Returns null if both bounds are null (omit the section entirely).
 */
function formatValidTime(
  start: string | null,
  end: string | null,
): string | null {
  if (start === null && end === null) return null;
  const s = start ?? "—";
  const e = end ?? "ongoing";
  return `${s} → ${e}`;
}

// ─── Component ───────────────────────────────────────────────────────────────

export function SearchResults(props: SearchResultsProps) {
  const hasItems = () => props.items.length > 0;

  return (
    <div data-testid="search-results-root">
      {/* ── Loading indicator ─────────────────────────────────────────── */}
      <Show when={props.isLoading}>
        <div
          data-testid="search-results-loading"
          role="status"
          aria-live="polite"
          aria-label="Searching for results"
        >
          Searching…
        </div>
      </Show>

      {/* ── Idle: render nothing ─────────────────────────────────────── */}
      {/* Intentionally empty when resultState is idle: no element needed */}

      {/* ── Error state ───────────────────────────────────────────────── */}
      <Show when={props.resultState === "error" && !props.isLoading}>
        <div
          data-testid="search-results-error"
          role="alert"
          aria-live="assertive"
        >
          {props.errorMessage
            ? `Search error: ${props.errorMessage}`
            : "Search error. Please try again."}
        </div>
      </Show>

      {/* ── No-results state ─────────────────────────────────────────── */}
      <Show when={props.resultState === "no-results" && !props.isLoading}>
        <div
          data-testid="search-results-empty"
          role="status"
        >
          No results found. Your filters are preserved.
        </div>
      </Show>

      {/* ── Partial results notice ────────────────────────────────────── */}
      <Show when={props.resultState === "partial" && !props.isLoading}>
        <div
          data-testid="search-results-partial-notice"
          role="status"
          aria-live="polite"
        >
          Partial results — some strategies unavailable
        </div>
      </Show>

      {/* ── Results list ─────────────────────────────────────────────── */}
      <Show when={hasItems() && !props.isLoading}>
        <ul
          data-testid="search-results-list"
          role="list"
          aria-label="Search results"
        >
          <For each={props.items}>
            {(item) => {
              const validTime = formatValidTime(
                item.validTimeStart,
                item.validTimeEnd,
              );

              function handleKeyDown(
                e: KeyboardEvent & { currentTarget: HTMLElement },
              ) {
                if (e.key === "Enter") {
                  e.preventDefault();
                  props.onSelect(item);
                }
              }

              return (
                <li
                  role="listitem"
                  data-testid={`result-item-${item.id}`}
                  data-result-id={item.id}
                  tabIndex={0}
                  aria-label={`${item.kind}: ${item.summary}`}
                  onKeyDown={handleKeyDown}
                >
                  {/* ── Kind badge ─────────────────────────────────── */}
                  <span
                    data-testid={`result-kind-${item.id}`}
                    data-field="kind"
                  >
                    {item.kind}
                  </span>

                  {/* ── Summary (primary content) ─────────────────── */}
                  <p
                    data-testid={`result-summary-${item.id}`}
                    data-field="summary"
                  >
                    {item.summary}
                  </p>

                  {/* ── Matched field ──────────────────────────────── */}
                  <span
                    data-testid={`result-matched-field-${item.id}`}
                    data-field="matched-field"
                  >
                    matched in: {item.matchedField}
                  </span>

                  {/* ── Rationale ─────────────────────────────────── */}
                  <p
                    data-testid={`result-rationale-${item.id}`}
                    data-field="rationale"
                  >
                    {item.rationale}
                  </p>

                  {/* ── Relative score — always "Rank: X%", never "confidence" */}
                  <span
                    data-testid={`result-score-${item.id}`}
                    data-field="relative-score"
                  >
                    {formatRelativeScore(item.relativeScore)}
                  </span>

                  {/* ── Profile ───────────────────────────────────── */}
                  <span
                    data-testid={`result-profile-${item.id}`}
                    data-field="profile-id"
                  >
                    Profile: {item.profileId}
                  </span>

                  {/* ── Policy summary — exact backend text, no synthesis */}
                  <span
                    data-testid={`result-policy-${item.id}`}
                    data-field="policy-summary"
                  >
                    {item.policySummary}
                  </span>

                  {/* ── Truth state — always shown, with data attribute */}
                  <span
                    data-testid={`result-truth-state-${item.id}`}
                    data-field="truth-state"
                    data-truth-state={item.truthState}
                  >
                    {item.truthState}
                  </span>

                  {/* ── Graph revision ────────────────────────────── */}
                  <span
                    data-testid={`result-revision-${item.id}`}
                    data-field="graph-revision"
                  >
                    Rev: {item.graphRevision}
                  </span>

                  {/* ── Source context (when present) ─────────────── */}
                  <Show
                    when={
                      item.sourceLabel !== null || item.sourceId !== null
                    }
                  >
                    <span
                      data-testid={`result-source-${item.id}`}
                      data-field="source"
                    >
                      Source:{" "}
                      {item.sourceLabel ?? item.sourceId}
                    </span>
                  </Show>

                  {/* ── Valid time range (when present) ───────────── */}
                  <Show when={validTime !== null}>
                    <span
                      data-testid={`result-valid-time-${item.id}`}
                      data-field="valid-time"
                    >
                      Valid: {validTime}
                    </span>
                  </Show>

                  {/* ── Transaction time ──────────────────────────── */}
                  <span
                    data-testid={`result-transaction-time-${item.id}`}
                    data-field="transaction-time"
                  >
                    Stored: {item.transactionTime}
                  </span>

                  {/* ── Navigation link ───────────────────────────── */}
                  <button
                    type="button"
                    data-testid={`result-navigate-${item.id}`}
                    data-field="navigation-target"
                    aria-label={`Open ${item.kind}: ${item.summary}`}
                    onClick={() => props.onNavigate(item.navigationTarget)}
                  >
                    Open
                  </button>
                </li>
              );
            }}
          </For>
        </ul>
      </Show>
    </div>
  );
}

export default SearchResults;
