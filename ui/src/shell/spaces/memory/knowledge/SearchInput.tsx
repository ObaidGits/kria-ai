/**
 * SearchInput — Full-corpus search input for the Recall destination.
 *
 * Invariants (F4.3 / task 4.3.1):
 * - Hard 512-character limit: submit is blocked if query exceeds 512 chars.
 * - Character count indicator is shown; screen reader announces at/over limit.
 * - Debounce of ~300ms before firing onSubmit; creates a fresh AbortController
 *   per submit and aborts the previous one when a new query begins.
 * - Platform-correct shortcut labels: ⌘K on Mac, Ctrl+K on others.
 *   Shortcuts are functional — the onKeyDown handler fires onSubmit.
 * - Active filters rendered as dismissible chip tokens with accessible
 *   remove buttons and keyboard navigation (arrow keys, Delete/Backspace).
 * - Saved-filter seam: onSaveFilter prop typed but no UI rendered unless provided
 *   AND the feature is explicitly gated — currently the seam is NOT rendered.
 * - Announced result state via role="status" aria-live="polite" region.
 * - Focus returns to input after chip removal.
 * - No "Filter this view" label confusion: this component is always full-corpus
 *   search. Any local-only filtering must be labeled "Filter this view" in the
 *   caller — this component never performs local filtering.
 *
 * Requirements: MGR-006, MGR-014, MGR-023–024, MGR-031;
 *   MGD-026, MGD-030, MGD-046; MG-H01, MG-H04, MG-H10–H12, MG-O05, MG-O25.
 */
import {
  createEffect,
  createMemo,
  createSignal,
  For,
  onCleanup,
  Show,
} from "solid-js";

// ─── Max limit constant ───────────────────────────────────────────────────────

export const QUERY_MAX_LENGTH = 512;

// ─── Types ────────────────────────────────────────────────────────────────────

/** A single active filter chip shown in the input area. */
export interface SearchFilter {
  /** Stable identifier for this filter instance. */
  id: string;
  /** Human-readable label shown in the chip. */
  label: string;
  /** The filter value. */
  value: string;
  /**
   * The filter dimension kind, e.g. "kind", "truth-state", "source".
   * Used for display grouping by callers; SearchInput renders it as-is.
   */
  kind: string;
}

/** Result state communicated by the caller for accessible announcements. */
export type ResultState =
  | "idle"
  | "searching"
  | "results"
  | "no-results"
  | "error"
  | "partial";

/** Props for SearchInput. */
export interface SearchInputProps {
  /** Controlled query value. */
  query: string;
  /** Called whenever the user changes the input text. */
  onQueryChange: (q: string) => void;
  /**
   * Called (after debounce) when the user commits a search.
   * The signal parameter allows the caller to cancel the in-flight request.
   */
  onSubmit: (query: string, filters: SearchFilter[], signal: AbortSignal) => void;
  /** Active filter chips currently applied. */
  activeFilters: SearchFilter[];
  /** Called when the user removes a filter chip. */
  onRemoveFilter: (filterId: string) => void;
  /** True while a search request is in flight. */
  isSearching: boolean;
  /** Current announced result state. */
  resultState: ResultState;
  /** Number of results; shown in announcement when resultState === "results". */
  resultCount?: number;
  /**
   * Qualifier for the result count. Maps to MGR-006 AC-3 wording.
   * - "exact"    → "N results found"
   * - "at-least" → "At least N results found"
   * - "estimate" → "Estimate N results found"
   */
  resultCountQualifier?: "exact" | "at-least" | "estimate";
  /** Error message shown when resultState === "error". */
  errorMessage?: string;
  /**
   * Saved-filter seam — not implemented, just typed for future extensibility.
   * If provided, a save-filter affordance WOULD be surfaced; currently the
   * feature is not shipped so the prop is accepted but ignored in the render.
   */
  onSaveFilter?: () => void;
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

/**
 * Returns true when the current platform is macOS.
 * Uses navigator.platform (deprecated but still universally available) with a
 * userAgent fallback for environments where platform is empty.
 */
function isMac(): boolean {
  const platform =
    typeof navigator !== "undefined" ? navigator.platform ?? "" : "";
  const ua =
    typeof navigator !== "undefined" ? navigator.userAgent ?? "" : "";
  return /mac/i.test(platform) || /macintosh|mac os x/i.test(ua);
}

/** Human-readable label for a result state. */
function resultStateAnnouncement(
  state: ResultState,
  count?: number,
  qualifier?: "exact" | "at-least" | "estimate",
  errorMessage?: string,
): string {
  switch (state) {
    case "idle":
      return "";
    case "searching":
      return "Searching…";
    case "results": {
      if (count === undefined) return "Results found";
      switch (qualifier) {
        case "at-least":
          return `At least ${count} results found`;
        case "estimate":
          return `Estimate ${count} results found`;
        default:
          return `${count} results found`;
      }
    }
    case "no-results":
      return "No results";
    case "error":
      return errorMessage ? `Search error: ${errorMessage}` : "Search error";
    case "partial":
      return count !== undefined ? `${count} results found (partial)` : "Results found (partial)";
  }
}

// ─── Component ───────────────────────────────────────────────────────────────

export function SearchInput(props: SearchInputProps) {
  // ── Refs ──────────────────────────────────────────────────────────────────
  let inputRef: HTMLInputElement | undefined;

  // ── Platform detection ────────────────────────────────────────────────────
  const mac = isMac();
  const shortcutLabel = mac ? "⌘K" : "Ctrl+K";
  const shortcutAriaLabel = mac ? "Command K" : "Control K";

  // ── Character count ───────────────────────────────────────────────────────
  const charCount = () => props.query.length;
  const isOverLimit = () => charCount() > QUERY_MAX_LENGTH;
  const isAtLimit = () => charCount() === QUERY_MAX_LENGTH;
  const charCountAnnouncement = createMemo(() => {
    if (isOverLimit()) {
      return `Query too long: ${charCount()} of ${QUERY_MAX_LENGTH} characters`;
    }
    if (isAtLimit()) {
      return `${QUERY_MAX_LENGTH} of ${QUERY_MAX_LENGTH} characters — limit reached`;
    }
    return "";
  });

  // ── AbortController management ────────────────────────────────────────────
  let activeController: AbortController | null = null;

  function fireSubmit(query: string) {
    // Abort any previous in-flight request.
    if (activeController) {
      activeController.abort();
    }
    activeController = new AbortController();
    props.onSubmit(query, props.activeFilters, activeController.signal);
  }

  // ── Debounce ──────────────────────────────────────────────────────────────
  createEffect(() => {
    const q = props.query;
    if (isOverLimit()) return; // do not fire while over limit

    const timer = setTimeout(() => {
      fireSubmit(q);
    }, 300);

    onCleanup(() => clearTimeout(timer));
  });

  // ── Input handler ─────────────────────────────────────────────────────────
  function handleInput(e: InputEvent & { currentTarget: HTMLInputElement }) {
    props.onQueryChange(e.currentTarget.value);
  }

  // ── Explicit submit (form onSubmit or shortcut) ───────────────────────────
  function handleExplicitSubmit(e?: Event) {
    e?.preventDefault();
    if (isOverLimit()) return;
    fireSubmit(props.query);
  }

  // ── Keyboard shortcut on the input ────────────────────────────────────────
  function handleInputKeyDown(e: KeyboardEvent & { currentTarget: HTMLInputElement }) {
    const isTrigger = mac
      ? e.metaKey && e.key === "k"
      : e.ctrlKey && e.key === "k";
    if (isTrigger) {
      e.preventDefault();
      handleExplicitSubmit();
    }
  }

  // ── Chip keyboard navigation ──────────────────────────────────────────────
  function handleChipKeyDown(
    e: KeyboardEvent & { currentTarget: HTMLElement },
    filterId: string,
    chipIndex: number,
  ) {
    const chips = props.activeFilters;

    if (e.key === "Delete" || e.key === "Backspace") {
      e.preventDefault();
      removeFilterAndFocus(filterId, chipIndex);
      return;
    }

    if (e.key === "ArrowRight" || e.key === "ArrowDown") {
      e.preventDefault();
      const nextIndex = chipIndex + 1;
      if (nextIndex < chips.length) {
        focusChip(nextIndex);
      } else {
        inputRef?.focus();
      }
      return;
    }

    if (e.key === "ArrowLeft" || e.key === "ArrowUp") {
      e.preventDefault();
      const prevIndex = chipIndex - 1;
      if (prevIndex >= 0) {
        focusChip(prevIndex);
      }
      return;
    }
  }

  function focusChip(index: number) {
    const chipList = document.querySelector<HTMLElement>("[data-testid='filter-chips-list']");
    if (!chipList) return;
    const items = chipList.querySelectorAll<HTMLElement>("[data-chip-index]");
    const target = Array.from(items).find(
      (el) => el.getAttribute("data-chip-index") === String(index),
    );
    target?.focus();
  }

  function removeFilterAndFocus(filterId: string, removedIndex: number) {
    props.onRemoveFilter(filterId);
    // Focus: move to adjacent chip or fall back to input.
    // Since the chip is being removed we schedule focus on the next tick.
    setTimeout(() => {
      const remaining = props.activeFilters.length - 1; // length after removal
      if (remaining <= 0) {
        inputRef?.focus();
        return;
      }
      const targetIndex = removedIndex < remaining ? removedIndex : remaining - 1;
      focusChip(targetIndex);
    }, 0);
  }

  // ── Result state announcement ─────────────────────────────────────────────
  const announcement = createMemo(() =>
    resultStateAnnouncement(
      props.resultState,
      props.resultCount,
      props.resultCountQualifier,
      props.errorMessage,
    )
  );

  return (
    <div data-testid="search-input-root" class="search-input-root">
      {/* ── Search form ─────────────────────────────────────────────────── */}
      <form
        data-testid="search-input-form"
        onSubmit={handleExplicitSubmit}
        aria-label="Full-corpus search"
        role="search"
      >
        {/* ── Filter chips area ──────────────────────────────────────────── */}
        <Show when={props.activeFilters.length > 0}>
          <ul
            data-testid="filter-chips-list"
            role="list"
            aria-label="Active filters"
          >
            <For each={props.activeFilters}>
              {(filter, index) => (
                <li
                  role="listitem"
                  data-chip-index={index()}
                  data-chip-id={filter.id}
                  tabIndex={0}
                  onKeyDown={(e) => handleChipKeyDown(e, filter.id, index())}
                  data-testid={`filter-chip-${filter.id}`}
                >
                  <span data-chip-label={filter.label}>
                    {filter.label}: {filter.value}
                  </span>
                  <button
                    type="button"
                    aria-label={`Remove filter: ${filter.label}`}
                    data-testid={`remove-chip-${filter.id}`}
                    onClick={() => {
                      props.onRemoveFilter(filter.id);
                      // Return focus to input after removal.
                      setTimeout(() => inputRef?.focus(), 0);
                    }}
                  >
                    ✕
                  </button>
                </li>
              )}
            </For>
          </ul>
        </Show>

        {/* ── Input row ──────────────────────────────────────────────────── */}
        <div data-testid="search-input-row">
          <label for="search-input-field">Search memory</label>
          <input
            id="search-input-field"
            ref={inputRef}
            type="search"
            data-testid="search-input-field"
            value={props.query}
            onInput={handleInput}
            onKeyDown={handleInputKeyDown}
            placeholder="Search across all memory…"
            aria-label="Search query"
            aria-describedby="search-char-count search-shortcut-hint"
            autocomplete="off"
            disabled={props.isSearching}
          />

          {/* ── Submit button ────────────────────────────────────────────── */}
          <button
            type="submit"
            data-testid="search-submit-button"
            aria-label={`Submit search (${shortcutAriaLabel})`}
            disabled={props.isSearching || isOverLimit()}
          >
            Search
          </button>
        </div>

        {/* ── Character count ─────────────────────────────────────────────── */}
        <div
          id="search-char-count"
          data-testid="search-char-count"
          aria-live="polite"
          aria-atomic="true"
          role="status"
        >
          <span data-testid="char-count-text">
            {charCount()} / {QUERY_MAX_LENGTH}
          </span>
          {/* Announce when at or over limit */}
          <Show when={isOverLimit() || isAtLimit()}>
            <span
              data-testid="char-limit-announcement"
              aria-live="assertive"
              aria-atomic="true"
            >
              {charCountAnnouncement()}
            </span>
          </Show>
        </div>

        {/* ── Shortcut hint ────────────────────────────────────────────────── */}
        <span
          id="search-shortcut-hint"
          data-testid="search-shortcut-hint"
          aria-label={`Keyboard shortcut: ${shortcutAriaLabel}`}
        >
          {shortcutLabel}
        </span>
      </form>

      {/* ── Result state announcement region ──────────────────────────────── */}
      <div
        data-testid="result-state-announcement"
        role="status"
        aria-live="polite"
        aria-atomic="true"
      >
        {announcement()}
      </div>

      {/* ── Saved-filter seam ─────────────────────────────────────────────── */}
      {/*
        Saved-filter feature is NOT shipped. The onSaveFilter prop is accepted
        for future extensibility. When the feature is approved, render a
        "Save filter" button here conditioned on props.onSaveFilter !== undefined.
        Currently nothing is rendered so no stale UI ships.
      */}
    </div>
  );
}

export default SearchInput;
