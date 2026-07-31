/**
 * memory/scene/selectionManager — SelectionManager
 *
 * Pure reducer and types for disjoint single-click selection vs double-click
 * expand/fit, safe selection removal on scene refresh, and accessibility
 * announcements on re-resolution and close.
 *
 * Design invariants (F4.1):
 *   • Single click ONLY selects — it never triggers expansion or fit.
 *   • Double click ONLY expands/fits — it NEVER changes selectedId.
 *   • REFRESH removes stale selections (id not present in newNodeIds) with
 *     an accessible announcement; retains valid ones with a re-resolution
 *     announcement.
 *   • CLOSE always clears the selection with a reason-specific announcement.
 *   • The reducer is a pure function — no mutation of input state.
 *
 * Requirements: MGR-013 (focus concurrency), MGR-014 (accessible graph
 * composite), MGR-016 (responsive input model); F4.1 window session invariants.
 */

// ─── Constants ────────────────────────────────────────────────────────────────

/**
 * Maximum milliseconds between the first and second click for the pair to be
 * treated as a double-click by the canvas layer.
 *
 * The canvas layer (not this module) is responsible for click-timing
 * detection; this constant is exported so the canvas implementation and
 * tests share the same threshold.
 */
export const DOUBLE_CLICK_THRESHOLD_MS = 300;

// ─── SelectionEvent ───────────────────────────────────────────────────────────

/**
 * Discriminated union of all events that can affect selection state.
 *
 * Invariants:
 *   SINGLE_CLICK  — select only; no expansion or fit.
 *   DOUBLE_CLICK  — expand/fit only; no selection change.
 *   REFRESH       — scene content refreshed; stale selections removed.
 *   CLOSE         — window or panel closed; selection cleared entirely.
 */
export type SelectionEvent =
  | {
      /** Select a node. Does NOT expand or fit the view. */
      type: "SINGLE_CLICK";
      nodeId: string;
    }
  | {
      /** Expand/fit the view on a node. Does NOT change selectedId. */
      type: "DOUBLE_CLICK";
      nodeId: string;
    }
  | {
      /**
       * The visible scene was refreshed and now contains exactly these node
       * IDs.  Any selected node absent from `newNodeIds` is de-selected with
       * an appropriate announcement.
       */
      type: "REFRESH";
      newNodeIds: string[];
    }
  | {
      /** The graph view was closed.  Selection is always cleared entirely. */
      type: "CLOSE";
      reason: "user" | "navigation" | "error";
    };

// ─── SelectionState ───────────────────────────────────────────────────────────

/**
 * Current selection state for one graph view window.
 *
 * `selectedId`       — the currently selected node ID, or `null` when nothing
 *                      is selected.
 * `lastAnnouncement` — the most recent accessible announcement text to be read
 *                      by a screen reader (via an `aria-live` region); `null`
 *                      until the first state-changing event.
 */
export interface SelectionState {
  selectedId: string | null;
  lastAnnouncement: string | null;
}

// ─── selectionReducer ─────────────────────────────────────────────────────────

/**
 * Pure reducer for graph-view selection state.
 *
 * Each `SelectionEvent` kind maps to exactly one policy:
 *
 * | Event         | selectedId                                 | lastAnnouncement                              |
 * |---------------|--------------------------------------------|-----------------------------------------------|
 * | SINGLE_CLICK  | set to `event.nodeId`                      | "Selected: <nodeId>"                          |
 * | DOUBLE_CLICK  | unchanged (expand/fit is canvas-layer only)| unchanged                                     |
 * | REFRESH       | retained if in `newNodeIds`, else `null`   | "Selection re-resolved" / "Selection removed: node no longer in view" |
 * | CLOSE (user)  | `null`                                     | "Graph view closed"                           |
 * | CLOSE (nav)   | `null`                                     | "Graph view closed: navigated away"           |
 * | CLOSE (error) | `null`                                     | "Graph view closed: error"                    |
 *
 * @param state  The current immutable selection state.
 * @param event  The selection event to process.
 * @returns      A new `SelectionState` (or the same reference when the event
 *               is a true no-op, e.g. DOUBLE_CLICK).
 */
export function selectionReducer(
  state: SelectionState,
  event: SelectionEvent,
): SelectionState {
  switch (event.type) {
    case "SINGLE_CLICK": {
      // Select the node. Expansion/fit is handled exclusively by the canvas
      // layer; this reducer does not trigger it.
      return {
        selectedId: event.nodeId,
        lastAnnouncement: `Selected: ${event.nodeId}`,
      };
    }

    case "DOUBLE_CLICK": {
      // Expand/fit is the canvas layer's responsibility.
      // This reducer MUST NOT change selectedId or the announcement.
      return state;
    }

    case "REFRESH": {
      const { newNodeIds } = event;

      if (state.selectedId === null) {
        // Nothing was selected — no announcement needed; return same ref.
        return state;
      }

      const retained = newNodeIds.includes(state.selectedId);

      if (retained) {
        // Selection is still valid in the refreshed scene.
        return {
          selectedId: state.selectedId,
          lastAnnouncement: "Selection re-resolved",
        };
      } else {
        // The selected node is no longer in the visible scene.
        return {
          selectedId: null,
          lastAnnouncement: "Selection removed: node no longer in view",
        };
      }
    }

    case "CLOSE": {
      const announcement = closeAnnouncement(event.reason);
      return {
        selectedId: null,
        lastAnnouncement: announcement,
      };
    }
  }
}

// ─── Internal helpers ─────────────────────────────────────────────────────────

/**
 * Returns the accessible announcement text for a CLOSE event, varying by
 * reason so screen-reader users know why the view closed.
 */
function closeAnnouncement(reason: "user" | "navigation" | "error"): string {
  switch (reason) {
    case "user":
      return "Graph view closed";
    case "navigation":
      return "Graph view closed: navigated away";
    case "error":
      return "Graph view closed: error";
  }
}
