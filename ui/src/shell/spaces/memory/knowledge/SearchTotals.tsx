/**
 * SearchTotals — Honest totals, truncation notice, cursor pagination, and
 * search-vs-filter mode label for the Recall destination.
 *
 * Invariants (F4.3 / task 4.3.3):
 * - Renders the exact qualifier wording from the backend without invention:
 *   "Showing N of N results"           — exact
 *   "Showing N of at least N results"  — at-least
 *   "Showing N of estimated N results" — estimate
 *   "Showing N results"                — unknown total (null)
 * - Truncation is clearly labeled when shown < total (isTruncated=true).
 * - Cursor-based pagination: prev/next buttons with accessible aria-labels.
 *   Both disabled when isLoading=true.
 * - Mode label always visible and unambiguous:
 *   mode="search" → "Full-corpus search"
 *   mode="filter" → "Filter this view"
 * - Strategy info section only rendered when strategyInfo prop is provided.
 * - UI never invents counts, modes, or availability claims.
 *
 * Requirements: MGR-006, MGR-014, MGR-023–024, MGR-031;
 *   MGD-026, MGD-030, MGD-046; MG-H01, MG-H04, MG-O05, MG-O25.
 */
import { For, Show } from "solid-js";

// ─── Types ────────────────────────────────────────────────────────────────────

/** Strategy availability info forwarded from the backend retrieval trace. */
export interface StrategyInfo {
  /** Strategy names that contributed to this result set. */
  used: string[];
  /** Strategy names that were unavailable during this retrieval. */
  unavailable: string[];
}

/** Props for SearchTotals. */
export interface SearchTotalsProps {
  /**
   * "search" = full-corpus backend Recall search.
   * "filter" = local-only visible-list filtering ("Filter this view").
   * The label must always be visible and never ambiguous.
   */
  mode: "search" | "filter";
  /** Number of items currently shown in the result list. */
  shown: number;
  /**
   * Total matching items according to the backend.
   * null when the backend did not report a total (unknown).
   */
  total: number | null;
  /**
   * Qualifier for the total count. Maps directly to backend semantics.
   * null when total is null.
   */
  totalQualifier: "exact" | "at-least" | "estimate" | null;
  /**
   * True when the shown count is less than total and the result set is
   * capped — the user can load more via cursor.
   */
  isTruncated: boolean;
  /** True when there is a previous cursor page available. */
  hasPreviousCursor: boolean;
  /** True when there is a next cursor page available. */
  hasNextCursor: boolean;
  /** True while a retrieval request is in-flight. */
  isLoading: boolean;
  /** Called when the user activates "Load previous results". */
  onPreviousPage: () => void;
  /** Called when the user activates "Load next results". */
  onNextPage: () => void;
  /**
   * Optional strategy availability info from the backend retrieval trace.
   * When omitted, the strategy-info section is not rendered.
   */
  strategyInfo?: StrategyInfo;
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

/**
 * Build the "Showing N of …" text exactly as specified.
 * Never invents qualifier wording.
 */
function buildTotalsLabel(
  shown: number,
  total: number | null,
  qualifier: "exact" | "at-least" | "estimate" | null,
): string {
  if (total === null) {
    return `Showing ${shown} results`;
  }
  switch (qualifier) {
    case "at-least":
      return `Showing ${shown} of at least ${total} results`;
    case "estimate":
      return `Showing ${shown} of estimated ${total} results`;
    case "exact":
    default:
      return `Showing ${shown} of ${total} results`;
  }
}

// ─── Component ───────────────────────────────────────────────────────────────

export function SearchTotals(props: SearchTotalsProps) {
  const totalsLabel = () =>
    buildTotalsLabel(props.shown, props.total, props.totalQualifier);

  const modeText = () =>
    props.mode === "search" ? "Full-corpus search" : "Filter this view";

  return (
    <div data-testid="search-totals-root">
      {/* ── Mode label — always visible, never ambiguous ─────────────── */}
      <span
        data-testid="mode-label"
        data-mode={props.mode}
      >
        {modeText()}
      </span>

      {/* ── Totals label ──────────────────────────────────────────────── */}
      <span data-testid="totals-label">
        {totalsLabel()}
      </span>

      {/* ── Truncation notice — only when isTruncated=true ───────────── */}
      <Show when={props.isTruncated}>
        <div data-testid="truncation-notice">
          {`Results truncated — showing first ${props.shown}. Use cursor to load more.`}
        </div>
      </Show>

      {/* ── Cursor pagination controls ────────────────────────────────── */}
      <button
        type="button"
        data-testid="prev-page-button"
        aria-label="Load previous results"
        disabled={!props.hasPreviousCursor || props.isLoading}
        onClick={() => props.onPreviousPage()}
      >
        Previous
      </button>

      <button
        type="button"
        data-testid="next-page-button"
        aria-label="Load next results"
        disabled={!props.hasNextCursor || props.isLoading}
        onClick={() => props.onNextPage()}
      >
        Next
      </button>

      {/* ── Strategy info — only when strategyInfo prop is provided ──── */}
      <Show when={props.strategyInfo !== undefined}>
        <div data-testid="strategy-info">
          <Show when={(props.strategyInfo?.used.length ?? 0) > 0}>
            <span data-testid="used-strategies">
              {"Used: "}
              <For each={props.strategyInfo!.used}>
                {(s, i) => (
                  <span>
                    {i() > 0 ? ", " : ""}
                    {s}
                  </span>
                )}
              </For>
            </span>
          </Show>
          <Show when={(props.strategyInfo?.unavailable.length ?? 0) > 0}>
            <span data-testid="unavailable-strategies">
              {"Unavailable: "}
              <For each={props.strategyInfo!.unavailable}>
                {(s, i) => (
                  <span>
                    {i() > 0 ? ", " : ""}
                    {s}
                  </span>
                )}
              </For>
            </span>
          </Show>
        </div>
      </Show>
    </div>
  );
}

export default SearchTotals;
