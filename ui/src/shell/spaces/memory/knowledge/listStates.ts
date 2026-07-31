/**
 * memory/knowledge/listStates — discriminated union + pure helpers for list request states
 *
 * Covers every degraded/error/terminal state the semantic list can enter.
 * All copy is exact and pre-approved — no invented strings, no "something went
 * wrong", no "empty store" claims, no disclosure of hidden items.
 *
 * Key invariants (F4.3 / task 4.3.6):
 *   • Every state has exact copy — getCopyForState never invents text.
 *   • Correlation IDs are propagated on every state that carries one.
 *   • Retry intent is preserved through loading → timeout/error transitions.
 *   • "empty" means no items match this query — never claims the store is empty.
 *   • "unauthorized" never reveals existence of hidden items.
 *   • "recovery" exposes diagnostics only — no write operations.
 *   • "deleted" shows "item no longer available" — never reveals content.
 *
 * Requirements: MGR-006, MGR-014, MGR-031; MGD-026, MGD-030.
 */

// ─── Discriminated union ──────────────────────────────────────────────────────

export type ListRequestState =
  | { kind: "empty" }
  | { kind: "loading"; correlationId: string; intent: string }
  | { kind: "ready"; items: unknown[]; correlationId: string }
  | { kind: "partial"; items: unknown[]; correlationId: string; unavailableStrategies: string[] }
  | { kind: "stale"; items: unknown[]; correlationId: string; staleSince: string }
  | { kind: "offline"; preservedItems: unknown[] | null; lastKnownCorrelationId: string | null }
  | { kind: "unauthorized"; correlationId: string; reason: string | null }
  | { kind: "timeout"; correlationId: string; retryable: boolean; intent: string }
  | { kind: "malformed"; correlationId: string; details: string }
  | { kind: "error"; correlationId: string; message: string; retryable: boolean; intent: string }
  | { kind: "deleted"; itemId: string; correlationId: string }
  | { kind: "recovery"; correlationId: string; diagnostics: string };

// ─── Exact copy constants ─────────────────────────────────────────────────────

/**
 * Exact copy for each list request state kind.
 *
 * Rules applied:
 *   - "empty" → says query found nothing, not that the store is empty.
 *   - "unauthorized" → no hint that hidden items exist.
 *   - "deleted" → "no longer available" only; content never revealed.
 *   - No state uses "something went wrong".
 *   - "ready" → empty string (items speak for themselves).
 */
export const LIST_STATE_COPY: Record<ListRequestState["kind"], string> = {
  empty:        "No items match your search. Your filters are preserved.",
  loading:      "Loading\u2026",
  ready:        "", // no copy for ready — items speak for themselves
  partial:      "Partial results \u2014 some retrieval strategies were unavailable.",
  stale:        "Results may be out of date. Refresh to reload.",
  offline:      "You are offline. Showing last known results.",
  unauthorized: "You do not have permission to view these results.",
  timeout:      "The request timed out. You can retry.",
  malformed:    "The response was unrecognised. Please report the correlation ID.",
  error:        "An error occurred. Please try again.",
  deleted:      "This item is no longer available.",
  recovery:     "System is in recovery mode. Writes are disabled.",
};

// ─── Transition helpers (pure functions) ─────────────────────────────────────

/**
 * Returns the exact copy string for a given state kind.
 * Never invents text — always reads from LIST_STATE_COPY.
 */
export function getCopyForState(kind: ListRequestState["kind"]): string {
  return LIST_STATE_COPY[kind];
}

/**
 * Returns true if this state is retryable.
 *
 * - timeout: always retryable (retryable field is always true per mkTimeout)
 * - error: retryable only when the error's retryable flag is true
 * - all other states: not retryable
 */
export function isRetryable(state: ListRequestState): boolean {
  switch (state.kind) {
    case "timeout": return state.retryable;
    case "error":   return state.retryable;
    default:        return false;
  }
}

/**
 * Returns the correlation ID if the state carries one; null otherwise.
 * "empty" and "offline" (with no lastKnownCorrelationId) return null.
 */
export function getCorrelationId(state: ListRequestState): string | null {
  switch (state.kind) {
    case "empty":         return null;
    case "offline":       return state.lastKnownCorrelationId;
    case "loading":       return state.correlationId;
    case "ready":         return state.correlationId;
    case "partial":       return state.correlationId;
    case "stale":         return state.correlationId;
    case "unauthorized":  return state.correlationId;
    case "timeout":       return state.correlationId;
    case "malformed":     return state.correlationId;
    case "error":         return state.correlationId;
    case "deleted":       return state.correlationId;
    case "recovery":      return state.correlationId;
  }
}

/**
 * Returns the preserved intent string if present (used to rebuild a retry
 * request with the original parameters).
 * Only loading, timeout, and error carry an intent.
 */
export function getPreservedIntent(state: ListRequestState): string | null {
  switch (state.kind) {
    case "loading": return state.intent;
    case "timeout": return state.intent;
    case "error":   return state.intent;
    default:        return null;
  }
}

/**
 * Returns true if the state has items that can be displayed to the user.
 * offline counts as having items only when preservedItems is non-null and non-empty.
 */
export function hasItems(state: ListRequestState): boolean {
  switch (state.kind) {
    case "ready":   return state.items.length > 0;
    case "partial": return state.items.length > 0;
    case "stale":   return state.items.length > 0;
    case "offline": return state.preservedItems !== null && state.preservedItems.length > 0;
    default:        return false;
  }
}

/**
 * Returns true if the state represents a degraded (but not hard-error) view:
 * the user sees results but they are incomplete, stale, or offline.
 */
export function isDegraded(state: ListRequestState): boolean {
  switch (state.kind) {
    case "stale":   return true;
    case "partial": return true;
    case "offline": return true;
    default:        return false;
  }
}

/**
 * Returns true if the state is a hard error from which no results are shown:
 * unauthorized, malformed response, generic error, or recovery mode.
 */
export function isHardError(state: ListRequestState): boolean {
  switch (state.kind) {
    case "unauthorized": return true;
    case "malformed":    return true;
    case "error":        return true;
    case "recovery":     return true;
    default:             return false;
  }
}

// ─── Constructor helpers ──────────────────────────────────────────────────────

/** Create a loading state from an intent string and correlation ID. */
export function mkLoading(correlationId: string, intent: string): ListRequestState {
  return { kind: "loading", correlationId, intent };
}

/** Create a ready state from items and correlationId. */
export function mkReady(items: unknown[], correlationId: string): ListRequestState {
  return { kind: "ready", items, correlationId };
}

/** Create a partial state. */
export function mkPartial(
  items: unknown[],
  correlationId: string,
  unavailableStrategies: string[],
): ListRequestState {
  return { kind: "partial", items, correlationId, unavailableStrategies };
}

/** Create a stale state (items + when they became stale). */
export function mkStale(
  items: unknown[],
  correlationId: string,
  staleSince: string,
): ListRequestState {
  return { kind: "stale", items, correlationId, staleSince };
}

/** Create an offline state (preserving current items if any, or null). */
export function mkOffline(
  preservedItems: unknown[] | null,
  lastKnownCorrelationId: string | null,
): ListRequestState {
  return { kind: "offline", preservedItems, lastKnownCorrelationId };
}

/** Create an unauthorized state. */
export function mkUnauthorized(correlationId: string, reason: string | null): ListRequestState {
  return { kind: "unauthorized", correlationId, reason };
}

/**
 * Create a timeout state.
 * Always retryable — timeout with retryable=false is an error state, not a timeout.
 * The original intent is preserved so the retry request can be reconstructed.
 */
export function mkTimeout(correlationId: string, intent: string): ListRequestState {
  return { kind: "timeout", correlationId, retryable: true, intent };
}

/** Create a malformed state. */
export function mkMalformed(correlationId: string, details: string): ListRequestState {
  return { kind: "malformed", correlationId, details };
}

/** Create an error state. */
export function mkError(
  correlationId: string,
  message: string,
  retryable: boolean,
  intent: string,
): ListRequestState {
  return { kind: "error", correlationId, message, retryable, intent };
}

/** Create a deleted state. */
export function mkDeleted(itemId: string, correlationId: string): ListRequestState {
  return { kind: "deleted", itemId, correlationId };
}

/** Create a recovery state. */
export function mkRecovery(correlationId: string, diagnostics: string): ListRequestState {
  return { kind: "recovery", correlationId, diagnostics };
}
