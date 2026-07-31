/**
 * memory/api/client — MemoryApiClient v2
 *
 * One client operation map that normalises Tauri IPC and HTTP transports behind
 * a single `dispatch` call. Callers never branch on transport; the client handles
 * that internally.
 *
 * Design invariants (F4.1):
 *   • No local semantic inference — the client passes results through unchanged;
 *     it never filters, re-ranks, or annotates response items.
 *   • Unsupported capability is explicit — an `UnsupportedCapabilityError` is
 *     thrown when the server returns an Unsupported error code.
 *   • Per-request correlation IDs — generated from crypto.randomUUID() if the
 *     caller does not supply one.
 *   • AbortController deadlines — every request is wrapped in a deadline timeout
 *     (default 5 000 ms); the caller may also supply its own AbortSignal.
 *   • Revision base forwarded — callers may attach a known revision so the
 *     server can reject stale requests early.
 *
 * Transport matrix:
 *   "tauri"  — uses @tauri-apps/api invoke; works in the desktop runtime.
 *   "http"   — uses native fetch with JSON body.
 *
 * Requirements: MGR-007, MGR-008, MGR-020.
 */

import { bridgeInvoke } from "../../../../bridge/invoke";

// ─── Public constants ─────────────────────────────────────────────────────────

/** Default deadline for every dispatch call (ms). */
export const DEFAULT_DEADLINE_MS = 5_000;

// ─── Shared type aliases ──────────────────────────────────────────────────────

/** Discriminated count semantics for `GraphResponseV2.total_count`. */
export interface TotalSemantics {
  kind: "exact" | "at_least" | "estimate";
  value: number;
}

/** A non-fatal advisory from the backend. */
export interface ApiWarning {
  code: string;
  message: string;
}

/** Degradation envelope when one or more strategies are unavailable. */
export interface DegradationInfo {
  level: "partial" | "degraded" | "offline";
  unavailable_strategies: string[];
  reason: string;
}

/**
 * The canonical v2 graph response DTO — mirrors the Rust `GraphResponseV2`
 * struct.  The client never mutates or post-processes this value.
 */
export interface GraphResponseV2 {
  schema_version: string;
  revision: number;
  query_hash: string;
  items: unknown[];
  total_count: TotalSemantics;
  truncated: boolean;
  truncation_reason: string | null;
  recovery_cursor: string | null;
  warnings: ApiWarning[];
  degradation: DegradationInfo | null;
}

// ─── Request options ──────────────────────────────────────────────────────────

/**
 * Options accepted by `MemoryApiClient.dispatch`.
 *
 * Every field is optional; the client applies sensible defaults.
 */
export interface RequestOptions {
  /** An external AbortSignal to honour in addition to the deadline. */
  abortSignal?: AbortSignal;
  /**
   * Caller-supplied correlation ID included in every request for tracing.
   * Defaults to `crypto.randomUUID()`.
   */
  correlationId?: string;
  /**
   * The client's last known graph revision, forwarded so the server can reject
   * stale requests early.
   */
  revisionBase?: number;
  /**
   * Per-request deadline in milliseconds. Overrides `DEFAULT_DEADLINE_MS`.
   * An AbortController is created internally; the caller's `abortSignal` is
   * also respected if both are supplied.
   */
  deadlineMs?: number;
  /** Opaque pagination cursor from a previous response. */
  cursor?: string;
  /** Schema version the caller is prepared to accept. */
  schemaVersion?: string;
}

// ─── Error types ─────────────────────────────────────────────────────────────

/**
 * Thrown when the server responds with an `Unsupported` error code.
 *
 * The `feature` field carries the operation or capability name that the server
 * does not support.  Callers that need to degrade gracefully should catch this
 * error specifically.
 *
 * Requirements: MGR-020 (transport capability parity — explicit unsupported).
 */
export class UnsupportedCapabilityError extends Error {
  readonly feature: string;

  constructor(feature: string) {
    super("Unsupported: " + feature);
    this.name = "UnsupportedCapabilityError";
    this.feature = feature;
  }
}

// ─── Internal helpers ─────────────────────────────────────────────────────────

/**
 * Build the request envelope that is sent on both transports.
 * The shape is kept flat so Tauri's command argument deserializer is happy.
 */
function buildEnvelope(
  operation: string,
  params: unknown,
  options: RequestOptions,
): Record<string, unknown> {
  return {
    operation,
    params,
    correlation_id: options.correlationId ?? crypto.randomUUID(),
    deadline_ms: options.deadlineMs ?? DEFAULT_DEADLINE_MS,
    ...(options.revisionBase !== undefined && { revision_base: options.revisionBase }),
    ...(options.cursor !== undefined && { cursor: options.cursor }),
    ...(options.schemaVersion !== undefined && { schema_version: options.schemaVersion }),
  };
}

/**
 * Inspect a raw response body (or Tauri error string) and throw an
 * `UnsupportedCapabilityError` if the server reported an unsupported operation.
 */
function maybeThrowUnsupported(operation: string, rawError?: unknown): never | void {
  const msg = typeof rawError === "string" ? rawError : String(rawError ?? "");
  // The Rust adapter is expected to include "Unsupported" in the error message
  // for capability-not-present conditions.
  if (/unsupported/i.test(msg)) {
    throw new UnsupportedCapabilityError(operation);
  }
}

// ─── MemoryApiClient ──────────────────────────────────────────────────────────

/**
 * `MemoryApiClient` — single typed entry-point for all Memory API v2 calls.
 *
 * Construct one instance per window (or share a singleton per transport).
 * The client is stateless beyond the `transport` field; reactive state lives in
 * the window session store (`state/windowSession.ts`).
 */
export class MemoryApiClient {
  /** Transport this client uses for all operations. */
  readonly transport: "tauri" | "http";

  /**
   * For the HTTP transport only: the base URL (e.g. `"http://127.0.0.1:3000"`).
   * Ignored when `transport === "tauri"`.
   */
  readonly #baseUrl: string;

  constructor(options: { transport: "tauri" | "http"; baseUrl?: string }) {
    this.transport = options.transport;
    this.#baseUrl = options.baseUrl ?? "";
  }

  /**
   * Dispatch a Memory API v2 operation.
   *
   * Normalises Tauri IPC vs HTTP:
   *   • Both transports send the same `buildEnvelope(...)` shape.
   *   • Both transports detect `Unsupported` errors and throw
   *     `UnsupportedCapabilityError`.
   *   • Both transports respect the deadline and the caller's AbortSignal.
   *
   * Never infers semantic meaning from results — the response is returned as-is.
   *
   * @param operation  The named memory operation (e.g. `"memory_v2_query"`).
   * @param params     Operation-specific parameters, passed through verbatim.
   * @param options    Per-call overrides (deadline, correlation ID, …).
   * @returns          The raw `GraphResponseV2` from the server.
   * @throws           `UnsupportedCapabilityError` on Unsupported server response.
   * @throws           `DOMException` (name `"AbortError"`) on deadline / abort.
   */
  async dispatch(
    operation: string,
    params: unknown,
    options: RequestOptions = {},
  ): Promise<GraphResponseV2> {
    const deadlineMs = options.deadlineMs ?? DEFAULT_DEADLINE_MS;

    // Build a composite AbortSignal that fires on deadline or caller abort.
    const deadlineController = new AbortController();
    const deadlineTimer = setTimeout(
      () => deadlineController.abort(new DOMException("Memory API deadline exceeded", "AbortError")),
      deadlineMs,
    );

    // Combine the deadline signal with any caller-supplied signal.
    const signal: AbortSignal = options.abortSignal
      ? AbortSignal.any
        ? AbortSignal.any([options.abortSignal, deadlineController.signal])
        : deadlineController.signal // fallback for older runtimes
      : deadlineController.signal;

    try {
      if (this.transport === "tauri") {
        return await this.#dispatchTauri(operation, params, options, signal, deadlineController.signal);
      } else {
        return await this.#dispatchHttp(operation, params, options, signal);
      }
    } finally {
      clearTimeout(deadlineTimer);
    }
  }

  // ── Tauri transport ─────────────────────────────────────────────────────────

  async #dispatchTauri(
    operation: string,
    params: unknown,
    options: RequestOptions,
    _compositeSignal: AbortSignal,
    deadlineSignal: AbortSignal,
  ): Promise<GraphResponseV2> {
    const envelope = buildEnvelope(operation, params, options);

    // Build the list of signals to race against (deadline + optional caller).
    const signals: AbortSignal[] = [deadlineSignal];
    if (options.abortSignal) signals.push(options.abortSignal);

    // Check if already aborted before we start.
    for (const s of signals) {
      if (s.aborted) {
        throw s.reason ?? new DOMException("Aborted", "AbortError");
      }
    }

    // Tauri's invoke doesn't accept an AbortSignal, so we race the IPC call
    // against a signal-abort rejection to honour cancellation.
    const abortPromise = new Promise<never>((_resolve, reject) => {
      const onAbort = (signal: AbortSignal) => {
        reject(signal.reason ?? new DOMException("Aborted", "AbortError"));
      };
      for (const s of signals) {
        if (s.aborted) {
          onAbort(s);
          return;
        }
        s.addEventListener("abort", () => onAbort(s), { once: true });
      }
    });

    try {
      // Route desktop IPC through the shared bridge. Abort remains logical:
      // Tauri cannot cancel an already-issued command, so generation/session
      // guards must still reject any late completion.
      const call = bridgeInvoke<GraphResponseV2>(
        "memory_v2_dispatch",
        envelope,
        { timeoutMs: options.deadlineMs ?? DEFAULT_DEADLINE_MS },
      ).then((result) => {
        if (result.ok) return result.data;
        maybeThrowUnsupported(operation, result.message);
        throw new Error(result.message);
      });
      return await Promise.race([call, abortPromise]);
    } catch (err: unknown) {
      maybeThrowUnsupported(operation, err);
      throw err;
    }
  }

  // ── HTTP transport ──────────────────────────────────────────────────────────

  async #dispatchHttp(
    operation: string,
    params: unknown,
    options: RequestOptions,
    signal: AbortSignal,
  ): Promise<GraphResponseV2> {
    const envelope = buildEnvelope(operation, params, options);
    const url = `${this.#baseUrl}/memory/v2/dispatch`;

    let response: Response;
    try {
      response = await fetch(url, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(envelope),
        signal,
      });
    } catch (err: unknown) {
      // fetch throws a DOMException on abort — re-throw as-is.
      throw err;
    }

    if (!response.ok) {
      const text = await response.text().catch(() => "");
      maybeThrowUnsupported(operation, text);
      throw new Error(`Memory API HTTP ${response.status}: ${text}`);
    }

    const body: GraphResponseV2 = await response.json();
    return body;
  }
}
