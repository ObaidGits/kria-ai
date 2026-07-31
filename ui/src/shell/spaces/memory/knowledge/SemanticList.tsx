/**
 * SemanticList — Virtualized semantic list/table for the Knowledge destination.
 *
 * Renders a window of node/edge items from the knowledge graph. Only the rows
 * within the visible window (visibleStart..visibleEnd) are mounted; the
 * remaining space is established via a spacer div so the scroll container has
 * the correct scrollable height.
 *
 * Invariants (F4.3 / task 4.3.4):
 * - Every rendered item exposes kind, authorityClass, status, truthState,
 *   evidenceSummary, evidenceCount, and all authorized actions.
 * - Node items: kind badge, authorityClass, displayName, evidenceSummary,
 *   evidenceCount, status, truthState, actions.
 * - Edge items: kind badge, directionLabel, source→target labels,
 *   authorityClass, evidenceSummary, evidenceCount, status, truthState, actions.
 * - Virtualization: only items in [visibleStart, visibleEnd) are rendered;
 *   each row is positioned absolutely at top = index * itemHeight.
 * - No invented fields: every value is passed through from the caller unchanged.
 * - Actions: disabled when isEnabled=false; marked dangerous when isDangerous=true.
 * - Loading state shows role="status" indicator.
 * - Empty list shows "No items to display".
 *
 * Requirements: MGR-006, MGR-014, MGR-023–024, MGR-031;
 *   MGD-026, MGD-030, MGD-046; MG-H01, MG-H04, MG-H10–H12, MG-O05, MG-O25.
 */
import { For, Show } from "solid-js";

// ─── Data types ───────────────────────────────────────────────────────────────

/** A single authorized action that can be performed on a list item. */
export interface SemanticAction {
  /** Action identifier, e.g. "inspect", "correct", "forget", "delete", "relate". */
  id: string;
  /** Human-readable action label. */
  label: string;
  /** false = capability not authorized; action is rendered but visually disabled. */
  isEnabled: boolean;
  /** true = destructive action (styled differently). */
  isDangerous: boolean;
}

/** One item (node or edge) displayed in the semantic list. */
export interface SemanticListItem {
  /** Stable semantic ID. */
  id: string;
  /** node = entity/memory/goal/source/etc; edge = relation/link. */
  itemType: "node" | "edge";
  /** e.g. "entity", "memory", "relation", "goal", "source". */
  kind: string;
  /** e.g. "personal", "work", "public". */
  authorityClass: string;
  // Node-specific (null for edges)
  displayName: string | null;
  // Edge-specific (null for nodes)
  sourceId: string | null;
  sourceLabel: string | null;
  targetId: string | null;
  targetLabel: string | null;
  /** e.g. "knows", "worked-at", "contradicts". */
  directionLabel: string | null;
  // Common
  /** Short evidence summary. */
  evidenceSummary: string;
  /** Number of supporting evidence items. */
  evidenceCount: number;
  /** e.g. "active", "pending", "expired", "deleted". */
  status: string;
  /** e.g. "Current", "Stale", "Contradicted". */
  truthState: string;
  isSelected: boolean;
  /** true = currently focused/highlighted item. */
  isCurrent: boolean;
  /** true = row is expanded to show more details. */
  isExpanded: boolean;
  /** All authorized actions for this item. */
  authorizedActions: SemanticAction[];
}

// ─── Props ────────────────────────────────────────────────────────────────────

export interface SemanticListProps {
  /** All items in the list (only visibleStart..visibleEnd are rendered). */
  items: SemanticListItem[];
  /** True while a load or query is in-flight. */
  isLoading: boolean;
  // Virtualization: visible window
  /** Index of first visible item (inclusive). */
  visibleStart: number;
  /** Index of last visible item (exclusive). */
  visibleEnd: number;
  /** Total scrollable height in pixels. */
  totalHeight: number;
  /** Height per row in pixels (fixed-height virtualization). */
  itemHeight: number;
  // Callbacks
  /** Called when the user selects an item (single click). */
  onSelect: (itemId: string) => void;
  /** Called when the user expands an item (double-click or Shift+Enter). */
  onExpand: (itemId: string) => void;
  /** Called when the user activates an action button. */
  onAction: (itemId: string, actionId: string) => void;
  /** Called on scroll to synchronize virtualization state. */
  onScroll: (scrollTop: number) => void;
}

// ─── Component ───────────────────────────────────────────────────────────────

export function SemanticList(props: SemanticListProps) {
  // Derive the slice of items that should actually be rendered.
  const visibleItems = () => {
    const start = Math.max(0, props.visibleStart);
    const end = Math.min(props.items.length, props.visibleEnd);
    return props.items.slice(start, end).map((item, sliceIdx) => ({
      item,
      absoluteIndex: start + sliceIdx,
    }));
  };

  const isEmpty = () => !props.isLoading && props.items.length === 0;

  function handleScroll(e: Event & { currentTarget: HTMLDivElement }) {
    props.onScroll(e.currentTarget.scrollTop);
  }

  function handleRowKeyDown(
    e: KeyboardEvent & { currentTarget: HTMLElement },
    itemId: string,
  ) {
    if (e.key === "Enter" && e.shiftKey) {
      e.preventDefault();
      props.onExpand(itemId);
    }
  }

  return (
    <div data-testid="semantic-list-root">
      {/* ── Loading indicator ──────────────────────────────────────── */}
      <Show when={props.isLoading}>
        <div
          data-testid="semantic-list-loading"
          role="status"
          aria-live="polite"
          aria-label="Loading memory items"
        >
          Loading…
        </div>
      </Show>

      {/* ── Empty state ────────────────────────────────────────────── */}
      <Show when={isEmpty()}>
        <div data-testid="semantic-list-empty">No items to display</div>
      </Show>

      {/* ── Virtualized scroll container ───────────────────────────── */}
      <Show when={!props.isLoading && props.items.length > 0}>
        <div
          data-testid="semantic-list-scroll"
          role="grid"
          aria-label="Memory items"
          style={{ position: "relative", overflow: "auto" }}
          onScroll={handleScroll}
        >
          {/* Spacer that establishes the full scrollable height */}
          <div
            style={{
              height: `${props.totalHeight}px`,
              position: "relative",
              "pointer-events": "none",
            }}
            aria-hidden="true"
          />

          {/* Visible rows — absolutely positioned */}
          <For each={visibleItems()}>
            {({ item, absoluteIndex }) => (
              <div
                data-testid={`semantic-list-item-${item.id}`}
                data-item-type={item.itemType}
                data-selected={String(item.isSelected)}
                data-current={String(item.isCurrent)}
                data-expanded={String(item.isExpanded)}
                role="row"
                tabIndex={0}
                aria-selected={item.isSelected}
                aria-current={item.isCurrent ? "true" : undefined}
                aria-expanded={item.isExpanded ? "true" : undefined}
                style={{
                  position: "absolute",
                  top: `${absoluteIndex * props.itemHeight}px`,
                  height: `${props.itemHeight}px`,
                  width: "100%",
                }}
                onClick={() => props.onSelect(item.id)}
                onDblClick={() => props.onExpand(item.id)}
                onKeyDown={(e) => handleRowKeyDown(e, item.id)}
              >
                {/* ── Kind badge ──────────────────────────────────── */}
                <span
                  data-testid={`item-kind-${item.id}`}
                  data-field="kind"
                >
                  {item.kind}
                </span>

                {/* ── Authority class ──────────────────────────────── */}
                <span
                  data-testid={`item-authority-${item.id}`}
                  data-field="authority-class"
                >
                  {item.authorityClass}
                </span>

                {/* ── Node-specific: displayName ───────────────────── */}
                <Show when={item.itemType === "node" && item.displayName !== null}>
                  <span
                    data-testid={`item-display-name-${item.id}`}
                    data-field="display-name"
                  >
                    {item.displayName}
                  </span>
                </Show>

                {/* ── Edge-specific: directionLabel + source→target ── */}
                <Show when={item.itemType === "edge"}>
                  <Show when={item.directionLabel !== null}>
                    <span
                      data-testid={`item-direction-label-${item.id}`}
                      data-field="direction-label"
                    >
                      {item.directionLabel}
                    </span>
                  </Show>
                  <span
                    data-testid={`item-direction-${item.id}`}
                    data-field="direction"
                  >
                    {item.sourceLabel ?? item.sourceId ?? ""}
                    {" → "}
                    {item.targetLabel ?? item.targetId ?? ""}
                  </span>
                </Show>

                {/* ── Evidence summary ─────────────────────────────── */}
                <span
                  data-testid={`item-evidence-summary-${item.id}`}
                  data-field="evidence-summary"
                >
                  {item.evidenceSummary}
                </span>

                {/* ── Evidence count ───────────────────────────────── */}
                <span
                  data-testid={`item-evidence-count-${item.id}`}
                  data-field="evidence-count"
                >
                  {item.evidenceCount}
                </span>

                {/* ── Status ───────────────────────────────────────── */}
                <span
                  data-testid={`item-status-${item.id}`}
                  data-field="status"
                >
                  {item.status}
                </span>

                {/* ── Truth state ──────────────────────────────────── */}
                <span
                  data-testid={`item-truth-state-${item.id}`}
                  data-field="truth-state"
                  data-truth-state={item.truthState}
                >
                  {item.truthState}
                </span>

                {/* ── Authorized actions ───────────────────────────── */}
                <For each={item.authorizedActions}>
                  {(action) => (
                    <button
                      type="button"
                      data-testid={`action-${item.id}-${action.id}`}
                      aria-label={action.label}
                      disabled={!action.isEnabled}
                      aria-disabled={!action.isEnabled ? "true" : undefined}
                      data-dangerous={String(action.isDangerous)}
                      onClick={(e) => {
                        e.stopPropagation();
                        if (action.isEnabled) {
                          props.onAction(item.id, action.id);
                        }
                      }}
                    >
                      {action.label}
                    </button>
                  )}
                </For>
              </div>
            )}
          </For>
        </div>
      </Show>
    </div>
  );
}

export default SemanticList;
