/**
 * memory/state/patchReducer — patchReducer<T>
 *
 * Pure reducer for per-window query state convergence in the Memory Graph v2 UI.
 * Handles atomic patch application, duplicate/reorder/gap safety, schema and
 * policy mismatch refetch signalling, write confirmation/rollback, and
 * invalidation.
 *
 * Design invariants (F4.1):
 *   • Patch is atomic only when patch.base_revision === state.revision — any
 *     mismatch leaves state unchanged so the caller can trigger a bounded
 *     active-query refetch.
 *   • Schema version mismatch → state unchanged (schema changed; caller must
 *     refetch with the new schema).
 *   • Policy hash mismatch → state unchanged (policy changed; caller must
 *     refetch under the new policy).
 *   • Duplicate patches (same base+target already applied) are no-ops via the
 *     revision guard.
 *   • Reordered / gap patches fail the revision guard and are no-ops — the
 *     caller handles refetch.
 *   • CONFIRM_WRITE removes the matching pending write and advances revision
 *     only when the confirmed revision is strictly higher.
 *   • ROLLBACK_WRITE removes the matching pending write and restores items to
 *     the pre-optimistic snapshot saved in the pending write (if any).
 *   • INVALIDATE keeps items in place but appends a sentinel pending-write
 *     marker so callers can detect the invalidation without extra state fields.
 *   • REFETCH_REQUIRED is a pure signal — state is left unchanged; the caller
 *     handles the actual refetch.
 *   • All operations are pure; no mutation of the input state.
 *
 * Requirements: MGR-008 (revision/patch consistency), MGR-004 (policy isolation),
 * F4.1 (window session isolation and convergence).
 */

// ─── AuthorityPatch ───────────────────────────────────────────────────────────

/**
 * A single change entry within an `AuthorityPatch`.
 *
 * Mirrors the `graph_changes` row emitted by the Rust authority layer.
 */
export interface PatchEntry {
  /** The Cognitive_Record kind affected (e.g. "memory", "entity"). */
  record_kind: string;
  /** Stable canonical identifier of the affected record. */
  record_id: string;
  /** Semantic class of the change. */
  change_kind: "insert" | "update" | "state_change" | "delete" | "invalidate";
  /**
   * Content hash of the record prior to this change.
   * `null` for inserts.
   */
  before_hash: string | null;
  /**
   * Content hash of the record after this change.
   * `null` for deletes/invalidations.
   */
  after_hash: string | null;
  /**
   * Policy partition that owns this record change.
   * Used to detect cross-policy contamination.
   */
  policy_partition: string;
}

/**
 * An incremental patch emitted by the authority after a committed transaction.
 *
 * Mirrors the Rust `AuthorityPatch` type from `graph/patch.rs`.
 */
export interface AuthorityPatch {
  /** Graph revision this patch can be applied on top of. */
  base_revision: number;
  /** The graph revision this patch advances the client to. */
  target_revision: number;
  /** Ordered list of record-level changes. */
  changes: PatchEntry[];
  /** Record IDs that are invalidated by this patch and must be dropped. */
  invalidations: string[];
  /**
   * Encrypted cursor that allows the client to resume a bounded refetch
   * from this exact revision boundary.
   * `null` when recovery is not applicable.
   */
  recovery_cursor: string | null;
  /** DTO schema version the patch was produced under. */
  schema_version: string;
  /** Content hash of the effective policy the patch was produced under. */
  policy_hash: string;
}

// ─── PendingWrite ─────────────────────────────────────────────────────────────

/**
 * Tracks an optimistically-applied write that has not yet been confirmed or
 * rolled back by an authority revision.
 */
export interface PendingWrite {
  /** Stable command identifier forwarded in the authority command envelope. */
  commandId: string;
  /** Human-readable operation name for display in the pending state UI. */
  operationName: string;
  /**
   * Items that were displayed optimistically while this write is pending.
   * Stored so that ROLLBACK_WRITE can restore them.
   */
  optimisticItems?: unknown[];
  /**
   * The authority revision the command was based on.
   * Must equal `state.revision` at the time the optimistic write was applied.
   */
  baseRevision: number;
}

// ─── ReducerState ─────────────────────────────────────────────────────────────

/**
 * Per-window query state for a single active query in the Memory Graph v2 UI.
 *
 * `T` is the item type produced by the active query (e.g. a semantic scene node
 * DTO).  The reducer does not inspect item contents; it receives a new `T[]`
 * from the caller when a patch is applied.
 */
export interface ReducerState<T> {
  /** Current result set for the active query. */
  items: T[];
  /** The authority graph revision this state reflects. */
  revision: number;
  /**
   * In-flight or invalidation-sentinel pending writes.
   *
   * A pending write is appended when an optimistic mutation is initiated and
   * removed when confirmed or rolled back.  An INVALIDATE action also appends
   * a sentinel entry (see action docs).
   */
  pendingWrites: PendingWrite[];
  /** DTO schema version this state was produced under. */
  schemaVersion: string;
  /** Content hash of the effective policy this state was produced under. */
  policyHash: string;
  /** Deterministic hash of the query that produced this state. */
  queryHash: string;
}

// ─── PatchAction ─────────────────────────────────────────────────────────────

/**
 * Discriminated union of all patch reducer actions.
 */
export type PatchAction =
  | {
      type: "APPLY_PATCH";
      /** The authority patch to apply. */
      patch: AuthorityPatch;
    }
  | {
      type: "CONFIRM_WRITE";
      /** The command identifier being confirmed. */
      commandId: string;
      /**
       * The authority revision at which the write was committed.
       * The reducer advances `state.revision` only when this is strictly higher
       * than the current revision.
       */
      revision: number;
    }
  | {
      type: "ROLLBACK_WRITE";
      /** The command identifier being rolled back. */
      commandId: string;
      /** Human-readable reason for the rollback, surfaced in the UI. */
      reason: string;
    }
  | {
      type: "INVALIDATE";
      /**
       * Record IDs that must be considered stale.
       * The reducer keeps all current items but records a sentinel
       * pending-write entry so the orchestrating layer can detect the
       * invalidation and schedule a bounded refetch.
       */
      invalidatedIds: string[];
    }
  | {
      type: "REFETCH_REQUIRED";
      /**
       * The query hash that triggered the refetch signal.
       * Included so the orchestrating layer can match it to the active query.
       */
      queryHash: string;
      /**
       * Human-readable reason for the required refetch (e.g. "schema changed",
       * "gap in patch stream").
       */
      reason: string;
    };

// ─── Invalidation sentinel ────────────────────────────────────────────────────

/**
 * Sentinel `commandId` used for INVALIDATE entries appended to `pendingWrites`.
 * The orchestrating layer can detect these by checking `startsWith("__INVALIDATE__")`.
 */
const INVALIDATE_SENTINEL_PREFIX = "__INVALIDATE__";

/**
 * Build the sentinel commandId for an INVALIDATE action.
 */
function invalidateSentinelId(invalidatedIds: string[]): string {
  return `${INVALIDATE_SENTINEL_PREFIX}${invalidatedIds.join(",")}`;
}

// ─── patchReducer ─────────────────────────────────────────────────────────────

/**
 * Pure reducer for per-window query state convergence.
 *
 * @param state     The current immutable window state.
 * @param action    The action to apply.
 * @param newItems  For `APPLY_PATCH` actions: the caller-supplied items that
 *                  reflect the patched authority state.  Ignored for all other
 *                  action types.  When `undefined` on a matching patch the
 *                  existing items are preserved unchanged.
 * @returns         A new `ReducerState<T>` (or the same reference if the
 *                  action is a no-op).
 */
export function patchReducer<T>(
  state: ReducerState<T>,
  action: PatchAction,
  newItems?: T[],
): ReducerState<T> {
  switch (action.type) {
    case "APPLY_PATCH": {
      const { patch } = action;

      // Guard 1: revision must match — duplicate / reordered / gap patches
      // are no-ops; the caller must trigger a bounded refetch.
      if (patch.base_revision !== state.revision) {
        return state;
      }

      // Guard 2: schema version must match — caller must refetch under the
      // new schema if versions diverge.
      if (patch.schema_version !== state.schemaVersion) {
        return state;
      }

      // Guard 3: policy hash must match — caller must refetch under the new
      // effective policy if hashes diverge.
      if (patch.policy_hash !== state.policyHash) {
        return state;
      }

      // All guards passed — advance revision and apply provided items.
      return {
        ...state,
        revision: patch.target_revision,
        items: newItems !== undefined ? newItems : state.items,
      };
    }

    case "CONFIRM_WRITE": {
      const { commandId, revision } = action;

      // Remove the matching pending write (if any).
      const remainingWrites = state.pendingWrites.filter(
        (pw) => pw.commandId !== commandId,
      );

      // Advance revision only if the confirmed revision is strictly higher
      // than the current one (patches from the authority stream may already
      // have advanced it further).
      const newRevision = revision > state.revision ? revision : state.revision;

      return {
        ...state,
        revision: newRevision,
        pendingWrites: remainingWrites,
      };
    }

    case "ROLLBACK_WRITE": {
      const { commandId } = action;

      // Find the pending write to extract the pre-optimistic items.
      const pending = state.pendingWrites.find(
        (pw) => pw.commandId === commandId,
      );

      // Remove the matching pending write.
      const remainingWrites = state.pendingWrites.filter(
        (pw) => pw.commandId !== commandId,
      );

      // Revert items to the pre-optimistic snapshot if one was saved,
      // otherwise keep the current items unchanged.
      const revertedItems: T[] =
        pending?.optimisticItems !== undefined
          ? (pending.optimisticItems as T[])
          : state.items;

      return {
        ...state,
        items: revertedItems,
        pendingWrites: remainingWrites,
      };
    }

    case "INVALIDATE": {
      // Keep existing items (they remain displayable as stale data while
      // a refetch is in progress), but append a sentinel pending-write entry
      // so the orchestrating layer can detect the invalidation and schedule
      // a bounded refetch without additional state fields.
      const sentinelWrite: PendingWrite = {
        commandId: invalidateSentinelId(action.invalidatedIds),
        operationName: "invalidation",
        baseRevision: state.revision,
      };

      return {
        ...state,
        pendingWrites: [...state.pendingWrites, sentinelWrite],
      };
    }

    case "REFETCH_REQUIRED": {
      // Pure signal — state is left unchanged.
      // The orchestrating layer is responsible for scheduling the actual
      // bounded refetch based on the queryHash and reason in the action.
      return state;
    }
  }
}
