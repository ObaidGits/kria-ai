/**
 * memory/state/windowSession — MemoryWindowSessionV2
 *
 * Tracks per-window query/policy/revision state for the Memory Graph v2 UI.
 * Each window instance owns its own session; there is no shared mutable state.
 *
 * Design invariants (F4.1):
 *   • Per-instance request ownership — each `beginRequest` call increments the
 *     generation counter and cancels the previous in-flight request. Responses
 *     from superseded requests are silently discarded via generation mismatch.
 *   • Query/policy/revision guards — callers must validate the base revision and
 *     policy hash before committing mutations or displaying stale results.
 *   • Detached restore validation — only a session in the `"detached"` state may
 *     be restored; the guard prevents spurious re-attach.
 *   • Cancellation on focus change — `beginRequest` always cancels the previous
 *     AbortController so in-flight fetches are terminated promptly.
 *
 * Requirements: MGR-007, MGR-008, MGR-020.
 */

// ─── State type ───────────────────────────────────────────────────────────────

/**
 * Exact states a window session can occupy.
 *
 * Transitions:
 *   idle      → loading  (beginRequest)
 *   loading   → ready    (completeRequest — matching generation)
 *   loading   → error    (failRequest — matching generation)
 *   loading   → loading  (beginRequest while loading — cancels previous)
 *   ready     → loading  (beginRequest)
 *   ready     → stale    (markStale)
 *   error     → loading  (beginRequest)
 *   stale     → loading  (beginRequest)
 *   *         → detached (markDetached)
 *   detached  → idle     (reset)
 *   *         → idle     (reset)
 */
export type WindowSessionState =
  | "idle"
  | "loading"
  | "ready"
  | "stale"
  | "error"
  | "detached";

// ─── Config ───────────────────────────────────────────────────────────────────

/**
 * Immutable per-window configuration bound at construction time.
 *
 * `instanceId`     — unique window identifier; ties responses back to the
 *                    correct window even when multiple are open.
 * `policyHash`     — content hash of the policy the window was opened with;
 *                    used to reject responses that belong to a different policy.
 * `schemaVersion`  — the DTO schema version this window understands.
 */
export interface WindowSessionConfig {
  instanceId: string;
  policyHash: string;
  schemaVersion: string;
}

// ─── MemoryWindowSessionV2 ────────────────────────────────────────────────────

/**
 * `MemoryWindowSessionV2` — per-window reactive session for Memory Graph v2.
 *
 * One instance lives per open memory window. The orchestrating layer (store /
 * component) holds the reference; the API client is stateless and does not
 * hold a reference back.
 *
 * All mutation methods that operate on in-flight requests (`completeRequest`,
 * `failRequest`) perform a generation check and return `false` when the
 * generation no longer matches, making it safe to call them from async
 * callbacks without manual cancellation tracking.
 */
export class MemoryWindowSessionV2 {
  /** Stable window identifier forwarded to every request envelope. */
  readonly instanceId: string;

  /** Immutable config set at construction; use guards to validate fields. */
  readonly config: WindowSessionConfig;

  /**
   * Monotonically increasing counter — incremented on every `beginRequest`.
   * Returned inside the request token so callers can pass it back to
   * `completeRequest` / `failRequest`.
   */
  #generation: number = 0;

  /** Current lifecycle state of this session. */
  #state: WindowSessionState = "idle";

  /**
   * Last confirmed authority revision from a successful `completeRequest`.
   * Used by `guardRevision` to reject base-revision mismatches before writes.
   */
  #revision: number = 0;

  /**
   * The query string that the current in-flight request was launched with.
   * Cleared on `reset()`.
   */
  #activeQuery: string | null = null;

  /**
   * AbortController for the current in-flight request.
   * Cancelled (and replaced) on each `beginRequest`; cancelled on
   * `markDetached` and `reset`.
   */
  #activeAbortController: AbortController | null = null;

  // ── Constructor ─────────────────────────────────────────────────────────────

  constructor(config: WindowSessionConfig) {
    this.instanceId = config.instanceId;
    this.config = config;
  }

  // ── Read-only accessors ─────────────────────────────────────────────────────

  /** Current lifecycle state. */
  get state(): WindowSessionState {
    return this.#state;
  }

  /**
   * Current generation counter.
   * Starts at `0`; each `beginRequest` call increments it by 1.
   */
  get generation(): number {
    return this.#generation;
  }

  /**
   * Last authority revision confirmed by `completeRequest`.
   * `0` until the first successful request completes.
   */
  get revision(): number {
    return this.#revision;
  }

  // ── Request lifecycle ───────────────────────────────────────────────────────

  /**
   * Begin a new request for the given query string.
   *
   * Side effects:
   *   1. Cancels any current in-flight request via its AbortController.
   *   2. Increments `#generation`.
   *   3. Creates a fresh AbortController for the new request.
   *   4. Transitions state to `"loading"`.
   *   5. Records `query` as `#activeQuery`.
   *
   * Returns an opaque token containing the generation number and the signal
   * the caller must pass to its fetch / dispatch call.  The caller MUST pass
   * the generation back to `completeRequest` / `failRequest`.
   */
  beginRequest(query: string): { generation: number; signal: AbortSignal } {
    // Cancel the previous request if one is in flight.
    this.#activeAbortController?.abort();

    // Increment generation before creating the new controller so that any
    // lingering callbacks from the old request see a mismatched generation.
    this.#generation += 1;

    const controller = new AbortController();
    this.#activeAbortController = controller;
    this.#state = "loading";
    this.#activeQuery = query;

    return { generation: this.#generation, signal: controller.signal };
  }

  /**
   * Mark the request identified by `generation` as successfully completed.
   *
   * If `generation` does not match the current generation the response is
   * stale (a newer request was started) and the method returns `false`
   * without mutating any state.
   *
   * On match: transitions state to `"ready"` and records the new `revision`.
   */
  completeRequest(generation: number, revision: number): boolean {
    if (generation !== this.#generation) {
      return false;
    }
    this.#state = "ready";
    this.#revision = revision;
    return true;
  }

  /**
   * Mark the request identified by `generation` as failed.
   *
   * Returns `false` if the generation is stale (does not mutate state).
   * On match: transitions state to `"error"`.
   */
  failRequest(generation: number): boolean {
    if (generation !== this.#generation) {
      return false;
    }
    this.#state = "error";
    return true;
  }

  // ── Guards ──────────────────────────────────────────────────────────────────

  /**
   * Returns `true` only when `baseRevision` equals the last confirmed
   * authority revision.
   *
   * Use this guard before submitting a write or patch to ensure the caller
   * is working from a current snapshot and has not missed an interleaved
   * authority update.
   */
  guardRevision(baseRevision: number): boolean {
    return baseRevision === this.#revision;
  }

  /**
   * Returns `true` only when `policyHash` equals the policy hash this session
   * was configured with.
   *
   * Use this guard to reject responses or writes that belong to a different
   * policy context (e.g., after a policy reload that opened a new window).
   */
  guardPolicy(policyHash: string): boolean {
    return policyHash === this.config.policyHash;
  }

  // ── Lifecycle transitions ───────────────────────────────────────────────────

  /**
   * Detach this session (e.g. the backing window was unmounted while a request
   * was in flight, or the server reported the session is no longer valid).
   *
   * Cancels the active abort controller and transitions to `"detached"`.
   * The session may not be used for new requests until `reset()` is called,
   * but `validateDetachedRestore()` can be used to guard a restore flow.
   */
  markDetached(): void {
    this.#activeAbortController?.abort();
    this.#activeAbortController = null;
    this.#state = "detached";
  }

  /**
   * Returns `true` only when the session is in the `"detached"` state.
   *
   * Only detached sessions may be restored; this guard prevents an accidental
   * double-restore or a restore of an actively-loading session.
   */
  validateDetachedRestore(): boolean {
    return this.#state === "detached";
  }

  /**
   * Mark this session as stale (e.g. an authority revision bump was observed
   * on the bus while no request is in flight, meaning cached results are no
   * longer current).
   *
   * Only meaningful when the session is in an active (non-detached, non-error)
   * state; callers should check `state !== "detached"` before calling.
   */
  markStale(): void {
    this.#state = "stale";
  }

  /**
   * Reset the session to its initial `"idle"` state.
   *
   * Side effects:
   *   • Cancels any active abort controller.
   *   • Resets generation to `0`.
   *   • Resets revision to `0`.
   *   • Clears `#activeQuery`.
   *   • Transitions state to `"idle"`.
   */
  reset(): void {
    this.#activeAbortController?.abort();
    this.#activeAbortController = null;
    this.#generation = 0;
    this.#revision = 0;
    this.#activeQuery = null;
    this.#state = "idle";
  }
}
