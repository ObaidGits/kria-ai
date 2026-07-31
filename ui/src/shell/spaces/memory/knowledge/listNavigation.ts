/**
 * memory/knowledge/listNavigation — pure list navigation state management
 *
 * Handles sort/filter/page/expand/path/trace navigation for the semantic list
 * without ever loading full adjacency. All operations are pure reducers that
 * return new state objects.
 *
 * Key invariants (F4.3 / task 4.3.5):
 *   • applyFilter resets cursor to null (filter change = back to page 1).
 *   • applySort resets cursor to null (sort change = back to page 1).
 *   • expand/path/trace intents change intent.type only — no full adjacency load.
 *   • reconcileAfterRefetch preserves selection/focus when IDs remain available;
 *     returns selectionLost/focusLost flags when they drop out of the result set.
 *   • All functions are pure: no mutations, always return new objects.
 *
 * Requirements: MGR-006, MGR-014, MGR-023–024, MGR-031;
 *   MGD-026, MGD-030, MGD-046; MG-H01, MG-H04, MG-H10–H12, MG-O05, MG-O25.
 */

// ─── Types ────────────────────────────────────────────────────────────────────

/** Fields available for sorting the semantic list. */
export type SortField =
  | "name"
  | "kind"
  | "truth-state"
  | "revision"
  | "valid-time-start";

/** Sort direction. */
export type SortDirection = "asc" | "desc";

/** Current sort configuration. */
export interface SortState {
  field: SortField;
  direction: SortDirection;
}

/** Current filter configuration. Empty arrays mean "no filter applied" (all pass). */
export interface FilterState {
  /** Filter by item kinds (empty = all). */
  kinds: string[];
  /** Filter by truth states (empty = all). */
  truthStates: string[];
  /** Filter by authority class (empty = all). */
  authorityClasses: string[];
  /** Text filter query. */
  query: string;
}

/**
 * Describes what navigation operation the UI intends to perform.
 * Changing the intent signals the backend to issue the appropriate bounded
 * query — no full adjacency scan ever occurs on the client.
 */
export interface NavigationIntent {
  /** The navigation mode. */
  type: "list" | "expand" | "path" | "trace";
  /** For expand: the item to expand. */
  itemId?: string;
  /** For path: the source node. */
  sourceId?: string;
  /** For path: the target node. */
  targetId?: string;
  /** For trace: the retrieval trace ID. */
  traceId?: string;
}

/** Complete navigation state for the semantic list. */
export interface ListNavigationState {
  sort: SortState;
  filter: FilterState;
  /** Opaque cursor token for pagination (null = first page). */
  cursor: string | null;
  /** ID of the currently selected item, or null if nothing is selected. */
  selectedId: string | null;
  /** ID of the currently focused item, or null if nothing is focused. */
  focusedId: string | null;
  /** Current navigation intent. */
  intent: NavigationIntent;
  /** True while a backend refetch is in-flight (for optimistic UI). */
  pendingRefetch: boolean;
}

// ─── Default values ───────────────────────────────────────────────────────────

const DEFAULT_SORT: SortState = {
  field: "name",
  direction: "asc",
};

const DEFAULT_FILTER: FilterState = {
  kinds: [],
  truthStates: [],
  authorityClasses: [],
  query: "",
};

const DEFAULT_INTENT: NavigationIntent = {
  type: "list",
};

// ─── Reducer functions ────────────────────────────────────────────────────────

/**
 * Create the initial (default) list navigation state.
 * Sort: name asc, no filter, no cursor, nothing selected/focused, list intent.
 */
export function initialListNavigationState(): ListNavigationState {
  return {
    sort: { ...DEFAULT_SORT },
    filter: { ...DEFAULT_FILTER },
    cursor: null,
    selectedId: null,
    focusedId: null,
    intent: { ...DEFAULT_INTENT },
    pendingRefetch: false,
  };
}

/**
 * Apply a new sort. Resets cursor to null (new sort = start at page 1).
 */
export function applySort(
  state: ListNavigationState,
  sort: SortState,
): ListNavigationState {
  return {
    ...state,
    sort: { ...sort },
    cursor: null,
  };
}

/**
 * Apply a new filter. Resets cursor to null (new filter = start at page 1).
 */
export function applyFilter(
  state: ListNavigationState,
  filter: FilterState,
): ListNavigationState {
  return {
    ...state,
    filter: { ...filter },
    cursor: null,
  };
}

/**
 * Advance to the next page using the provided cursor token.
 * Keeps sort and filter unchanged.
 */
export function applyNextPage(
  state: ListNavigationState,
  cursor: string,
): ListNavigationState {
  return {
    ...state,
    cursor,
  };
}

/**
 * Go back to a previous page using the provided cursor token.
 * Keeps sort and filter unchanged.
 */
export function applyPreviousPage(
  state: ListNavigationState,
  cursor: string,
): ListNavigationState {
  return {
    ...state,
    cursor,
  };
}

/**
 * Set the selected item. Pass null to clear selection.
 * Does not affect focusedId.
 */
export function applySelection(
  state: ListNavigationState,
  itemId: string | null,
): ListNavigationState {
  return {
    ...state,
    selectedId: itemId,
  };
}

/**
 * Set the focused item. Pass null to clear focus.
 * Does not affect selectedId.
 */
export function applyFocus(
  state: ListNavigationState,
  itemId: string | null,
): ListNavigationState {
  return {
    ...state,
    focusedId: itemId,
  };
}

/**
 * Set the current navigation intent.
 *
 * Changing intent to "expand", "path", or "trace" signals the backend to issue
 * a bounded query for that specific operation. The client never loads full
 * adjacency — that remains the backend's responsibility.
 */
export function applyIntent(
  state: ListNavigationState,
  intent: NavigationIntent,
): ListNavigationState {
  return {
    ...state,
    intent: { ...intent },
  };
}

/**
 * Reconcile selection and focus after a bounded refetch completes.
 *
 * For each of selectedId and focusedId:
 *   - If the ID is null, it stays null (nothing to reconcile).
 *   - If the ID is present in availableIds, it is kept.
 *   - If the ID is absent from availableIds, it is cleared and the
 *     corresponding lost flag is set to true.
 *
 * @returns nextState  The reconciled navigation state.
 * @returns selectionLost  true when selectedId was non-null but is no longer available.
 * @returns focusLost  true when focusedId was non-null but is no longer available.
 */
export function reconcileAfterRefetch(
  state: ListNavigationState,
  availableIds: string[],
): { nextState: ListNavigationState; selectionLost: boolean; focusLost: boolean } {
  const idSet = new Set(availableIds);

  let selectedId = state.selectedId;
  let selectionLost = false;
  if (selectedId !== null && !idSet.has(selectedId)) {
    selectedId = null;
    selectionLost = true;
  }

  let focusedId = state.focusedId;
  let focusLost = false;
  if (focusedId !== null && !idSet.has(focusedId)) {
    focusedId = null;
    focusLost = true;
  }

  const nextState: ListNavigationState = {
    ...state,
    selectedId,
    focusedId,
    pendingRefetch: false,
  };

  return { nextState, selectionLost, focusLost };
}

/**
 * Mark a refetch as in-flight (pendingRefetch = true).
 * Use this to show optimistic UI while the backend responds.
 */
export function markPendingRefetch(
  state: ListNavigationState,
): ListNavigationState {
  return {
    ...state,
    pendingRefetch: true,
  };
}

/**
 * Clear the pending refetch flag (pendingRefetch = false).
 * Call this after the refetch response has been applied.
 */
export function clearPendingRefetch(
  state: ListNavigationState,
): ListNavigationState {
  return {
    ...state,
    pendingRefetch: false,
  };
}
