/**
 * Recall — Memory Control Center Recall destination.
 *
 * Provides full-corpus ranked search across all authorized memory content.
 * Never filters the corpus silently — this is the whole-corpus view, not a
 * filtered sub-view.
 *
 * Invariants (F4.2 / MGR-006):
 * - Search is always full-corpus; local filtering is a separate labeled operation.
 * - Result totals are labeled "Showing N results", "Showing at least N results",
 *   or "Showing estimate N results" per response semantics — never bare numbers.
 * - Scores are always labeled "Relative score" — never "confidence" or "probability".
 * - When one or more retrieval strategies are unavailable the result set is
 *   labeled partial and the unavailable strategies are named explicitly.
 * - A "Why this answer?" trace button is shown only when traceId is non-null;
 *   it is never rendered for results without a trace.
 * - When no results exist and the component is not loading, a "No results" state
 *   is shown that preserves active context without implying an empty store.
 * - This is a pure display component — no mutations, no policy enforcement.
 *
 * Requirements: F4.2 (task 4.2.3) — Recall destination.
 */
import { For, Show } from "solid-js";
import { createSignal } from "solid-js";

// ─── Supporting interfaces ────────────────────────────────────────────────────

export interface RecallResult {
  /** Stable record identifier. */
  id: string;
  /** Record kind (e.g. "memory", "entity", "summary"). */
  kind: string;
  /** Which field of the record matched the query. */
  matchedField: string;
  /** Human-readable rank rationale from the backend. */
  rationale: string;
  /**
   * Relative score value (e.g. "0.87"). This is a relative rank metric, not a
   * probability or calibrated confidence value. Always labeled "Relative score".
   */
  relativeScore: string;
  /** Truth state of this record (e.g. "Current", "Stale", "Contradicted"). */
  truthState: string;
  /** Authority revision at which this result was retrieved. */
  revision: number;
  /**
   * Retrieval_Trace ID proving context injection for this result.
   * null = no trace available for this result; the trace button is omitted.
   */
  traceId: string | null;
}

/**
 * Semantics of the total result count returned by the backend.
 * - "exact"    → the backend counted all matching records precisely.
 * - "at_least" → the backend can confirm at least this many results exist.
 * - "estimate" → the backend returned an approximated total.
 */
export type TotalSemantics = {
  kind: "exact" | "at_least" | "estimate";
  value: number;
};

// ─── Props ───────────────────────────────────────────────────────────────────

export interface RecallProps {
  /** Current search query string (controlled). */
  query: string;
  /** Results returned by the Retrieval Engine. */
  results: RecallResult[];
  /**
   * Total count with its semantics. null = loading or not yet requested —
   * the total-count element is not rendered until a value is provided.
   */
  totalCount: TotalSemantics | null;
  /**
   * Names of retrieval strategies that are currently unavailable.
   * Empty array means all strategies are available (no partial warning shown).
   */
  unavailableStrategies: string[];
  /** True while a search request is in-flight. */
  isLoading: boolean;
  /** Called when the user submits a search query. */
  onSearch: (query: string) => void;
  /**
   * Called when the user clicks "Why this answer?" on a result.
   * traceId is always non-null when this handler is invoked.
   */
  onOpenTrace: (traceId: string) => void;
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

/**
 * Formats the total count label according to the response semantics.
 * Uses the exact wording required by MGR-006:
 * - "Showing N results" (exact)
 * - "Showing at least N results" (at_least)
 * - "Showing estimate N results" (estimate)
 */
function formatTotalLabel(total: TotalSemantics): string {
  switch (total.kind) {
    case "exact":
      return `Showing ${total.value} results`;
    case "at_least":
      return `Showing at least ${total.value} results`;
    case "estimate":
      return `Showing estimate ${total.value} results`;
  }
}

// ─── Component ───────────────────────────────────────────────────────────────

export function Recall(props: RecallProps) {
  // Local controlled input mirrors the incoming query prop for the search form.
  const [inputValue, setInputValue] = createSignal(props.query);

  function handleSubmit(e: Event) {
    e.preventDefault();
    const q = inputValue().trim();
    // Allow empty submit — caller decides how to handle it.
    props.onSearch(q);
  }

  const hasUnavailableStrategies = () =>
    props.unavailableStrategies.length > 0;

  const hasResults = () => props.results.length > 0;

  const showNoResults = () =>
    !props.isLoading && !hasResults();

  return (
    <section data-testid="recall-shell" aria-label="Recall">
      {/* ── Search form ──────────────────────────────────────────────────── */}
      <form
        data-testid="search-form"
        onSubmit={handleSubmit}
        aria-label="Full-corpus search"
      >
        <label for="recall-search-input">Search memory</label>
        <input
          id="recall-search-input"
          type="search"
          value={inputValue()}
          onInput={(e) => setInputValue(e.currentTarget.value)}
          placeholder="Search across all memory…"
          maxLength={512}
          autocomplete="off"
          aria-label="Search query"
        />
        <button type="submit" disabled={props.isLoading}>
          Search
        </button>
      </form>

      {/* ── Loading indicator ─────────────────────────────────────────────── */}
      <Show when={props.isLoading}>
        <span data-testid="loading-indicator" role="status" aria-live="polite">
          Searching…
        </span>
      </Show>

      {/* ── Partial strategies warning ────────────────────────────────────── */}
      <Show when={hasUnavailableStrategies()}>
        <span data-testid="partial-strategies" role="status">
          Partial: {props.unavailableStrategies.join(", ")} unavailable
        </span>
      </Show>

      {/* ── Total count ───────────────────────────────────────────────────── */}
      <Show when={props.totalCount !== null}>
        <span data-testid="total-count">
          {formatTotalLabel(props.totalCount!)}
        </span>
      </Show>

      {/* ── Results list ──────────────────────────────────────────────────── */}
      <Show when={hasResults()}>
        <ul data-testid="results-list" aria-label="Search results">
          <For each={props.results}>
            {(result) => (
              <li data-result-id={result.id}>
                {/* Kind */}
                <span data-field="kind">{result.kind}</span>
                {" — "}
                {/* Matched field */}
                <span data-field="matched-field">{result.matchedField}</span>

                {/* Rationale */}
                <p data-field="rationale">{result.rationale}</p>

                {/* Relative score — never labeled as confidence or probability */}
                <span data-field="relative-score">
                  Relative score: {result.relativeScore}
                </span>

                {/* Truth state */}
                <span data-field="truth-state">{result.truthState}</span>

                {/* "Why this answer?" trace navigation — only when traceId exists */}
                <Show when={result.traceId !== null}>
                  <button
                    type="button"
                    data-testid={`trace-button-${result.id}`}
                    onClick={() => props.onOpenTrace(result.traceId!)}
                  >
                    Why this answer?
                  </button>
                </Show>
              </li>
            )}
          </For>
        </ul>
      </Show>

      {/* ── No results state ──────────────────────────────────────────────── */}
      <Show when={showNoResults()}>
        <span data-testid="no-results">
          No results
        </span>
      </Show>
    </section>
  );
}

export default Recall;
