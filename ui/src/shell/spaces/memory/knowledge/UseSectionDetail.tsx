/**
 * UseSectionDetail — Focused renderer for the Use inspector section.
 *
 * Renders the three separation-of-concerns fields:
 *   - Why stored    (whyStored)
 *   - Why recalled  (whyRecalled)
 *   - How used      (howUsed)
 *
 * Plus trace injection links (confirmed injections only) and filtered
 * reason counts (count + type only — never the content of filtered items).
 *
 * Invariants (F4.4 / task 4.4.3):
 * - Root: data-testid="use-section-detail"
 * - whyStored null → "No storage rationale available"
 * - whyRecalled null → "Not recalled in current context"
 * - howUsed null → "Not used in current context"
 * - traceInjections list shown only when non-empty
 * - filteredReasons section shown only when non-empty
 * - usedInTraceCount shown only when non-null
 * - Only wasInjected:true items appear in trace list (enforced by type)
 * - Filtered reasons show count + filterType ONLY — never content
 *
 * All copy for null states is exact and non-invented.
 * UI never invents facts; all content comes from backend data.
 *
 * Requirements: F4.4 (task 4.4.3)
 */
import { For, Show } from "solid-js";

// ─── Data types ───────────────────────────────────────────────────────────────

export interface TraceInjectionLink {
  traceId: string;
  traceLabel: string | null;   // human label for the trace (may be null)
  navigationTarget: string;    // e.g. "recall?trace=<traceId>"
  wasInjected: true;           // only confirmed injections may appear here
}

export interface FilteredReason {
  filterType: string;          // e.g. "non-injected", "policy-filtered", "below-threshold"
  count: number;               // how many were filtered
}

export interface UseSectionDetailData {
  whyStored: string | null;
  whyRecalled: string | null;
  howUsed: string | null;
  traceInjections: TraceInjectionLink[];  // only confirmed injected traces
  filteredReasons: FilteredReason[];      // what was filtered out (counts only, no content)
  usedInTraceCount: number | null;
}

// ─── Props ────────────────────────────────────────────────────────────────────

export interface UseSectionDetailProps {
  data: UseSectionDetailData;
  onNavigate: (target: string) => void;
}

// ─── Component ────────────────────────────────────────────────────────────────

export function UseSectionDetail(props: UseSectionDetailProps) {
  const d = () => props.data;

  return (
    <div data-testid="use-section-detail">

      {/* Why stored */}
      <div data-testid="why-stored">
        {d().whyStored !== null ? d().whyStored : "No storage rationale available"}
      </div>

      {/* Why recalled */}
      <div data-testid="why-recalled">
        {d().whyRecalled !== null ? d().whyRecalled : "Not recalled in current context"}
      </div>

      {/* How used */}
      <div data-testid="how-used">
        {d().howUsed !== null ? d().howUsed : "Not used in current context"}
      </div>

      {/* Trace injection links — only shown when non-empty */}
      <Show when={d().traceInjections.length > 0}>
        <ul data-testid="trace-injections-list">
          <For each={d().traceInjections}>
            {(link) => (
              <li data-testid={`trace-injection-${link.traceId}`}>
                <span>{link.traceLabel !== null ? link.traceLabel : link.traceId}</span>
                <button
                  type="button"
                  data-testid={`trace-navigate-${link.traceId}`}
                  onClick={() => props.onNavigate(link.navigationTarget)}
                >
                  {link.traceLabel !== null ? link.traceLabel : link.traceId}
                </button>
              </li>
            )}
          </For>
        </ul>
      </Show>

      {/* Filtered reasons — count + type only, never content */}
      <Show when={d().filteredReasons.length > 0}>
        <ul data-testid="filtered-reasons">
          <For each={d().filteredReasons}>
            {(reason) => (
              <li data-testid={`filtered-reason-${reason.filterType}`}>
                <span data-field="filter-count">{reason.count}</span>
                {" "}
                <span data-field="filter-type">{reason.filterType}</span>
              </li>
            )}
          </For>
        </ul>
      </Show>

      {/* Used in trace count — only shown when non-null */}
      <Show when={d().usedInTraceCount !== null}>
        <span data-testid="used-in-trace-count">{d().usedInTraceCount}</span>
      </Show>

    </div>
  );
}

export default UseSectionDetail;
