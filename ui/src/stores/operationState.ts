/**
 * Operation State — the ONE shared operation-state vocabulary (UIE-M-013, Req 13.1–13.6).
 *
 * Phase 7 / IU-12 sub-task 12.2. Today loading / error / retry state is scattered
 * across `coreStore` and every per-Space store, and the kit exposes only
 * `Progress` / `StatusDot` / `EmptyState` — there is no single vocabulary a
 * surface can map its authoritative signals onto (gap G1 from the 12.1
 * inventory). This module defines that ONE vocabulary as a READ-ONLY, DERIVED
 * projection, mirroring `currentWorkSummary.ts`:
 *
 *   • It is NOT a store: no signals of its own, no setters, no side effects,
 *     no timers, no lifecycle ownership.
 *   • It NEVER mutates runtime task, route, model, approval, or Safety state
 *     (design.md §20.1 read-only-projection invariant).
 *   • It NEVER adds a backend API and NEVER invents backend progress: a
 *     determinate progress value is surfaced ONLY when an authoritative source
 *     already measured it (matching observatory `Job.progress`, which is omitted
 *     when unmeasured — no fabricated percentage, UIE-M-013).
 *   • Absent / unknown values are OMITTED, never inferred (Req 13.x truthful
 *     omission), reusing the shared {@link nonEmpty} omission discipline instead
 *     of forking a parallel one.
 *
 * ── Scope of THIS sub-task (12.2) ────────────────────────────────────────────
 * 12.2 DEFINES the vocabulary: the {@link OperationState} type, the pure mapping
 * from existing authoritative signals into it, and the omission rules. It does
 * NOT wire presentation copy / loading text (that is 12.6) and does NOT rename
 * Stop controls (that is 12.7). Everything here is a pure derivation with no
 * imports of presentation components.
 *
 * ── The vocabulary (Req 13.6) ────────────────────────────────────────────────
 *   empty                        no operation / no data yet
 *   loading                      operation begun; awaiting first result
 *   active                       operation running/sustained
 *   waiting                      paused, awaiting an external/user/next signal
 *   blocked                      awaiting a decision (approval / policy) to proceed
 *   completed                    operation finished successfully
 *   failed                       operation failed; recovery may be offered
 *   retrying                     a retry / recovery attempt is in progress
 *   recovered                    recovery succeeded (announce once, Req 13.5)
 *   optional-service-unavailable an OPTIONAL backend/service is offline
 *
 * There is intentionally NO `cancelled`/`stopped` term: cancellation SCOPE is a
 * separate concern owned by UIE-M-015 (Stop naming, 12.7). A settled/stopped or
 * otherwise-unknown source status therefore resolves to `empty` (no active
 * operation) rather than being misrepresented as `completed` or `failed`.
 *
 * Requirements: 13.1, 13.2, 13.3, 13.4, 13.5, 13.6; design §17, §20.1; UIE-M-013.
 */
import type { CoreState } from "./coreStore";
import type { WorkBlockStatus } from "./converseStore";
import type { WorkflowStatus } from "./automationStore";
import type { JobStatus } from "./observatoryStore";
import type { WorkflowLifecycle } from "../types/workflowRuntime";
import { nonEmpty } from "./currentWorkSummary";

// ─── The vocabulary type ─────────────────────────────────────────────────────────

/** The ONE shared operation-state vocabulary (Req 13.6). */
export type OperationState =
  | "empty"
  | "loading"
  | "active"
  | "waiting"
  | "blocked"
  | "completed"
  | "failed"
  | "retrying"
  | "recovered"
  | "optional-service-unavailable";

/** Canonical ordering of the vocabulary (stable, for enumeration/tests). */
export const OPERATION_STATES: readonly OperationState[] = [
  "empty",
  "loading",
  "active",
  "waiting",
  "blocked",
  "completed",
  "failed",
  "retrying",
  "recovered",
  "optional-service-unavailable",
] as const;

/**
 * Precedence used ONLY to resolve an ambiguous multi-signal snapshot (several
 * flags true at once). Higher wins. Mirrors the attention-first intent of
 * `coreStore.STATE_PRIORITY`: a real failure or a required decision outranks
 * in-flight work, which outranks terminal/settled facts, which outrank the
 * `empty` resting floor.
 */
export const OPERATION_STATE_PRIORITY: Readonly<Record<OperationState, number>> = {
  failed: 100,
  blocked: 90,
  "optional-service-unavailable": 80,
  retrying: 70,
  waiting: 60,
  loading: 50,
  active: 40,
  recovered: 30,
  completed: 20,
  empty: 0,
};

/** States that need user attention (recovery / decision / offline optional dep). */
export const ATTENTION_OPERATION_STATES: ReadonlySet<OperationState> = new Set([
  "failed",
  "blocked",
  "waiting",
  "optional-service-unavailable",
]);

/** In-flight states where a determinate progress value is meaningful. */
export const PROGRESS_BEARING_OPERATION_STATES: ReadonlySet<OperationState> = new Set([
  "loading",
  "active",
  "retrying",
]);

/** Settled / terminal states (no further action inherent to the state itself). */
export const TERMINAL_OPERATION_STATES: ReadonlySet<OperationState> = new Set([
  "completed",
  "recovered",
  "empty",
]);

/** Whether a state needs user attention. */
export function isAttentionOperation(state: OperationState): boolean {
  return ATTENTION_OPERATION_STATES.has(state);
}

// ─── The projected fact ────────────────────────────────────────────────────────

/**
 * One read-only operation snapshot. Carries the source owner + source-owned id
 * so a presentation surface can deep-link to the real owner. `message` and
 * `progress` are OMITTED whenever the source provides none (never fabricated).
 */
export interface OperationSnapshot {
  readonly state: OperationState;
  /** Source owner verbatim (e.g. "coreStore.state", "settingsStore"). */
  readonly source: string;
  /** Source-owned operation id. Omitted when the source supplies none. */
  readonly operationId?: string;
  /**
   * Source-owned message (cause / block reason / run message). Omitted when the
   * source provides none — never inferred or replaced with a placeholder.
   */
  readonly message?: string;
  /**
   * Determinate progress in the closed interval [0, 1], surfaced ONLY when an
   * authoritative source already measured it AND the resolved state can bear
   * progress. Omitted otherwise — a wait with no measured progress is
   * indeterminate, never a fabricated percentage (UIE-M-013, Req 13.1).
   */
  readonly progress?: number;
}

// ─── Omission primitives ─────────────────────────────────────────────────────────

/**
 * Normalize a measured progress value. Present ONLY when it is a finite number
 * within [0, 1]. Any other input (undefined, null, NaN, Infinity, out-of-range)
 * is OMITTED — never clamped, rounded, or invented, so the UI can fall back to
 * an indeterminate indicator instead of a fabricated percentage.
 */
export function normalizeMeasuredProgress(
  value: number | null | undefined,
): number | undefined {
  if (typeof value !== "number") return undefined;
  if (!Number.isFinite(value)) return undefined;
  if (value < 0 || value > 1) return undefined;
  return value;
}

// ─── Source-enum adapters (existing scattered signals → ONE vocabulary) ──────────

/**
 * `coreStore` activity machine → operation vocabulary. Idle is the empty
 * resting floor; the sustained activity states collapse to `active`; the
 * attention states map to their vocabulary peers. `recovering` is an in-flight
 * recovery attempt → `retrying` (the successful end state returns to idle, which
 * a surface surfaces separately as `recovered`; the Core has no `recovered`).
 */
export function coreStateToOperationState(state: CoreState): OperationState {
  switch (state) {
    case "idle":
      return "empty";
    case "waiting":
      return "waiting";
    case "blocked":
      return "blocked";
    case "error":
      return "failed";
    case "recovering":
      return "retrying";
    default:
      // listening/thinking/planning/speaking/acting/running-automation/
      // watching/remembering/reflecting/learning
      return "active";
  }
}

/**
 * `converseStore` WorkBlock status → operation vocabulary. `pending` is queued
 * (loading), `running` is active, terminal success/failure map directly.
 * `stopped` has no vocabulary term (cancellation scope is UIE-M-015) → `empty`.
 */
export function workBlockStatusToOperationState(
  status: WorkBlockStatus,
): OperationState {
  switch (status) {
    case "pending":
      return "loading";
    case "running":
      return "active";
    case "completed":
      return "completed";
    case "failed":
      return "failed";
    case "stopped":
      return "empty";
  }
}

/**
 * `automationStore` workflow status → operation vocabulary. `idle` (configured,
 * not running) is `empty`; `paused` is awaiting resume → `waiting`.
 */
export function automationStatusToOperationState(
  status: WorkflowStatus,
): OperationState {
  switch (status) {
    case "idle":
      return "empty";
    case "running":
      return "active";
    case "completed":
      return "completed";
    case "failed":
      return "failed";
    case "paused":
      return "waiting";
  }
}

/**
 * Workflow-session lifecycle → operation vocabulary. `created`/`planned` are
 * pre-run (loading); `executing`/`verifying` are active; `hitl_pending` awaits a
 * decision → `blocked`; `finalized` is completed; `cancelled` has no vocabulary
 * term → `empty`.
 */
export function workflowLifecycleToOperationState(
  lifecycle: WorkflowLifecycle,
): OperationState {
  switch (lifecycle) {
    case "created":
    case "planned":
      return "loading";
    case "executing":
    case "verifying":
      return "active";
    case "hitl_pending":
      return "blocked";
    case "finalized":
      return "completed";
    case "cancelled":
      return "empty";
  }
}

/**
 * `observatoryStore` job status → operation vocabulary. `queued` is loading;
 * `timed_out` is a failure; `rolled_back` is a completed recovery → `recovered`;
 * `cancelled`/`unknown` have no vocabulary term → `empty` (never fabricated).
 */
export function jobStatusToOperationState(status: JobStatus): OperationState {
  switch (status) {
    case "queued":
      return "loading";
    case "running":
      return "active";
    case "paused":
      return "waiting";
    case "completed":
      return "completed";
    case "failed":
    case "timed_out":
      return "failed";
    case "rolled_back":
    case "recovered":
      return "recovered";
    case "cancelled":
    case "unknown":
      return "empty";
  }
}

// ─── Generic normalized async-signal derivation ──────────────────────────────────

/**
 * A normalized description of a surface's authoritative async signals. This is
 * how the settings/provisioning-style "loading + error + retryable + optional
 * service" stores map into the ONE vocabulary WITHOUT re-deriving their own
 * ad-hoc vocabulary. Every field is optional; a source supplies only what it
 * actually owns, and absent fields never contribute an invented state.
 */
export interface AsyncSignalInput {
  /** Source owner verbatim (required for deep-linking / provenance). */
  readonly source: string;
  /** Source-owned operation id, if any. */
  readonly operationId?: string | null;
  /** True when this surface depends on an OPTIONAL backend/service. */
  readonly serviceOptional?: boolean;
  /**
   * Availability of the optional service. Only consulted when
   * `serviceOptional` is true; `false` → `optional-service-unavailable`.
   */
  readonly serviceAvailable?: boolean;
  /** An explicit failure flag (independent of whether a message exists). */
  readonly failed?: boolean;
  /** Source-owned failure cause. Its presence also implies failure. */
  readonly error?: string | null;
  /** An explicit blocked flag (awaiting a decision). */
  readonly blocked?: boolean;
  /** Source-owned block reason. Its presence also implies blocked. */
  readonly blockReason?: string | null;
  /** A retry / recovery attempt is currently in progress. */
  readonly retrying?: boolean;
  /** Awaiting an external/user/next signal (paused). */
  readonly waiting?: boolean;
  /** Operation begun; awaiting first result. */
  readonly loading?: boolean;
  /** Operation running/sustained. */
  readonly active?: boolean;
  /** Recovery just succeeded (announce once, Req 13.5). */
  readonly recovered?: boolean;
  /** Operation finished successfully. */
  readonly completed?: boolean;
  /**
   * Whether the surface currently holds data. When explicitly `false` and no
   * other signal is active, the resolved state is `empty`. Left `undefined`
   * when the source does not distinguish empty from settled.
   */
  readonly hasData?: boolean;
  /** Source-owned, state-agnostic message (e.g. a live run message). */
  readonly message?: string | null;
  /**
   * Source-measured determinate progress. Surfaced only via
   * {@link normalizeMeasuredProgress} and only on a progress-bearing state.
   */
  readonly progress?: number | null;
}

/**
 * Resolve the state-appropriate, source-owned message. `failed`/`blocked` prefer
 * their dedicated cause; every state falls back to the generic source message.
 * Absent → omitted (never fabricated).
 */
function resolveMessage(
  state: OperationState,
  input: AsyncSignalInput,
): string | undefined {
  if (state === "failed") return nonEmpty(input.error) ?? nonEmpty(input.message);
  if (state === "blocked") return nonEmpty(input.blockReason) ?? nonEmpty(input.message);
  return nonEmpty(input.message);
}

/**
 * Pure projection: a normalized async-signal snapshot → one {@link OperationSnapshot}.
 *
 * Deterministic, side-effect free, and mutation free. Resolves the single
 * highest-precedence state present (see {@link OPERATION_STATE_PRIORITY}), then
 * attaches only the source-owned `operationId`, `message`, and measured
 * `progress` that actually exist. When no signal is active it resolves to
 * `empty` — the truthful resting state, not a fabricated one.
 */
export function deriveOperationSnapshot(input: AsyncSignalInput): OperationSnapshot {
  const state = resolveOperationState(input);
  const operationId = nonEmpty(input.operationId);
  const message = resolveMessage(state, input);
  const progress = PROGRESS_BEARING_OPERATION_STATES.has(state)
    ? normalizeMeasuredProgress(input.progress)
    : undefined;

  return {
    state,
    source: input.source,
    ...(operationId ? { operationId } : {}),
    ...(message ? { message } : {}),
    ...(progress !== undefined ? { progress } : {}),
  };
}

/**
 * Resolve the single vocabulary state for a normalized snapshot by fixed
 * precedence. Exported for surfaces that only need the state (not the full
 * snapshot) and for exhaustive testing.
 */
export function resolveOperationState(input: AsyncSignalInput): OperationState {
  if (input.serviceOptional === true && input.serviceAvailable === false) {
    return "optional-service-unavailable";
  }
  if (input.failed === true || nonEmpty(input.error) !== undefined) return "failed";
  if (input.blocked === true || nonEmpty(input.blockReason) !== undefined) return "blocked";
  if (input.retrying === true) return "retrying";
  if (input.waiting === true) return "waiting";
  if (input.loading === true) return "loading";
  if (input.active === true) return "active";
  if (input.recovered === true) return "recovered";
  if (input.completed === true) return "completed";
  return "empty";
}
