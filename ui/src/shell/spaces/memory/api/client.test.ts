/**
 * memory/api/client — unit tests for MemoryApiClient (task 4.1.2)
 *
 * Validates:
 *   • DEFAULT_DEADLINE_MS is 5 000
 *   • UnsupportedCapabilityError message and feature property
 *   • GraphResponseV2 TypeScript shape has expected fields (type-level assertions)
 *   • dispatch sets correlation_id and deadline_ms in the Tauri envelope
 *   • dispatch throws UnsupportedCapabilityError on Unsupported server responses
 *   • dispatch aborts and rejects on deadline
 *   • HTTP transport sends correct JSON envelope
 *
 * The Tauri `invoke` module is mocked so no desktop runtime is required.
 */

import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";

// ── Mock the shared bridge before importing the module under test ─────────────
// Existing assertions keep inspecting `invokeMock`; the bridge mock wraps raw
// fixture values in the same non-throwing ServiceResult shape used at runtime.
const { invokeMock } = vi.hoisted(() => ({
  invokeMock: vi.fn<(cmd: string, args: Record<string, unknown>) => Promise<unknown>>(),
}));
vi.mock("../../../../bridge/invoke", () => ({
  bridgeInvoke: async (cmd: string, args: Record<string, unknown>) => {
    const value = await invokeMock(cmd, args);
    return typeof value === "object" && value !== null && "ok" in value
      ? value
      : { ok: true, data: value };
  },
}));

import {
  DEFAULT_DEADLINE_MS,
  UnsupportedCapabilityError,
  MemoryApiClient,
} from "./client";
import type { GraphResponseV2 } from "./client";

// ─── Helpers ──────────────────────────────────────────────────────────────────

function makeMinimalResponse(): GraphResponseV2 {
  return {
    schema_version: "2.0.0",
    revision: 1,
    query_hash: "abc123",
    items: [],
    total_count: { kind: "exact", value: 0 },
    truncated: false,
    truncation_reason: null,
    recovery_cursor: null,
    warnings: [],
    degradation: null,
  };
}

// ─── Constant tests ───────────────────────────────────────────────────────────

describe("DEFAULT_DEADLINE_MS", () => {
  it("is 5 000", () => {
    expect(DEFAULT_DEADLINE_MS).toBe(5_000);
  });
});

// ─── UnsupportedCapabilityError ───────────────────────────────────────────────

describe("UnsupportedCapabilityError", () => {
  it("has the correct message format", () => {
    const err = new UnsupportedCapabilityError("temporal_diff");
    expect(err.message).toBe("Unsupported: temporal_diff");
  });

  it("exposes the feature property", () => {
    const err = new UnsupportedCapabilityError("prediction");
    expect(err.feature).toBe("prediction");
  });

  it("is an instance of Error", () => {
    const err = new UnsupportedCapabilityError("x");
    expect(err).toBeInstanceOf(Error);
  });

  it("has name UnsupportedCapabilityError", () => {
    const err = new UnsupportedCapabilityError("x");
    expect(err.name).toBe("UnsupportedCapabilityError");
  });
});

// ─── GraphResponseV2 type-level assertions ────────────────────────────────────
//
// TypeScript type checks run at compile time; the test below exercises a
// runtime shape assertion to confirm the interface contract is respected.

describe("GraphResponseV2 shape", () => {
  it("has schema_version, revision, query_hash, items, total_count, truncated, warnings, degradation", () => {
    const r: GraphResponseV2 = makeMinimalResponse();

    // Field presence assertions (type-level properties exercised at runtime)
    expect(typeof r.schema_version).toBe("string");
    expect(typeof r.revision).toBe("number");
    expect(typeof r.query_hash).toBe("string");
    expect(Array.isArray(r.items)).toBe(true);
    expect(typeof r.total_count.kind).toBe("string");
    expect(typeof r.total_count.value).toBe("number");
    expect(typeof r.truncated).toBe("boolean");
    expect(Array.isArray(r.warnings)).toBe(true);
    // truncation_reason and recovery_cursor are nullable strings
    expect(r.truncation_reason === null || typeof r.truncation_reason === "string").toBe(true);
    expect(r.recovery_cursor === null || typeof r.recovery_cursor === "string").toBe(true);
    // degradation is nullable object
    expect(r.degradation === null || typeof r.degradation === "object").toBe(true);
  });

  it("accepts DegradationInfo with level, unavailable_strategies, reason", () => {
    const r: GraphResponseV2 = {
      ...makeMinimalResponse(),
      degradation: {
        level: "partial",
        unavailable_strategies: ["vector"],
        reason: "embedder offline",
      },
    };
    expect(r.degradation?.level).toBe("partial");
    expect(Array.isArray(r.degradation?.unavailable_strategies)).toBe(true);
    expect(typeof r.degradation?.reason).toBe("string");
  });
});

// ─── MemoryApiClient — Tauri transport ───────────────────────────────────────

describe("MemoryApiClient — Tauri transport", () => {
  let client: MemoryApiClient;

  beforeEach(() => {
    invokeMock.mockReset();
    client = new MemoryApiClient({ transport: "tauri" });
  });

  it("has transport === 'tauri'", () => {
    expect(client.transport).toBe("tauri");
  });

  it("calls invoke('memory_v2_dispatch', …) with operation, params, correlation_id, deadline_ms", async () => {
    invokeMock.mockResolvedValueOnce(makeMinimalResponse());

    await client.dispatch("memory_v2_query", { q: "test" });

    expect(invokeMock).toHaveBeenCalledOnce();
    const [cmd, envelope] = invokeMock.mock.calls[0] as [string, Record<string, unknown>];
    expect(cmd).toBe("memory_v2_dispatch");
    expect(envelope.operation).toBe("memory_v2_query");
    expect(envelope.params).toEqual({ q: "test" });
    expect(typeof envelope.correlation_id).toBe("string");
    expect((envelope.correlation_id as string).length).toBeGreaterThan(0);
    expect(envelope.deadline_ms).toBe(DEFAULT_DEADLINE_MS);
  });

  it("uses caller-supplied correlationId when provided", async () => {
    invokeMock.mockResolvedValueOnce(makeMinimalResponse());

    await client.dispatch("op", {}, { correlationId: "my-trace-id" });

    const [, envelope] = invokeMock.mock.calls[0] as [string, Record<string, unknown>];
    expect(envelope.correlation_id).toBe("my-trace-id");
  });

  it("uses caller-supplied deadlineMs when provided", async () => {
    invokeMock.mockResolvedValueOnce(makeMinimalResponse());

    await client.dispatch("op", {}, { deadlineMs: 1_000 });

    const [, envelope] = invokeMock.mock.calls[0] as [string, Record<string, unknown>];
    expect(envelope.deadline_ms).toBe(1_000);
  });

  it("forwards revisionBase as revision_base", async () => {
    invokeMock.mockResolvedValueOnce(makeMinimalResponse());

    await client.dispatch("op", {}, { revisionBase: 42 });

    const [, envelope] = invokeMock.mock.calls[0] as [string, Record<string, unknown>];
    expect(envelope.revision_base).toBe(42);
  });

  it("does not include revision_base when revisionBase is omitted", async () => {
    invokeMock.mockResolvedValueOnce(makeMinimalResponse());

    await client.dispatch("op", {});

    const [, envelope] = invokeMock.mock.calls[0] as [string, Record<string, unknown>];
    expect("revision_base" in envelope).toBe(false);
  });

  it("returns the response without mutation (no local semantic inference)", async () => {
    const response = makeMinimalResponse();
    response.items = [{ id: "x", label: "Node X" }];
    invokeMock.mockResolvedValueOnce(response);

    const result = await client.dispatch("memory_v2_query", {});

    expect(result).toStrictEqual(response);
    expect(result.items).toHaveLength(1);
  });

  it("throws UnsupportedCapabilityError when invoke rejects with 'Unsupported'", async () => {
    invokeMock.mockRejectedValueOnce(new Error("Unsupported: temporal_diff"));

    await expect(client.dispatch("temporal_diff", {})).rejects.toBeInstanceOf(
      UnsupportedCapabilityError,
    );
  });

  it("throws UnsupportedCapabilityError when invoke rejects with lowercase 'unsupported'", async () => {
    invokeMock.mockRejectedValueOnce("unsupported operation: prediction");

    await expect(client.dispatch("prediction", {})).rejects.toBeInstanceOf(
      UnsupportedCapabilityError,
    );
  });

  it("re-throws non-unsupported errors as-is", async () => {
    const originalError = new Error("network error");
    invokeMock.mockRejectedValueOnce(originalError);

    await expect(client.dispatch("op", {})).rejects.toBe(originalError);
  });

  it("aborts when the caller's AbortSignal fires", async () => {
    // Never resolves — the abort should reject the promise
    invokeMock.mockReturnValueOnce(new Promise(() => { /* never */ }));

    const controller = new AbortController();

    const promise = client.dispatch("op", {}, {
      abortSignal: controller.signal,
      deadlineMs: 30_000, // long deadline so only the caller abort triggers
    });

    // Abort on next microtask tick so the race has started
    await Promise.resolve();
    controller.abort();

    await expect(promise).rejects.toThrow();
  }, 2_000); // short per-test timeout
});

// ─── MemoryApiClient — HTTP transport ────────────────────────────────────────

describe("MemoryApiClient — HTTP transport", () => {
  let client: MemoryApiClient;
  const fetchMock = vi.fn<(input: RequestInfo | URL, init?: RequestInit) => Promise<Response>>();

  beforeEach(() => {
    fetchMock.mockReset();
    vi.stubGlobal("fetch", fetchMock);
    client = new MemoryApiClient({ transport: "http", baseUrl: "http://127.0.0.1:3000" });
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("has transport === 'http'", () => {
    expect(client.transport).toBe("http");
  });

  it("POSTs to /memory/v2/dispatch with correct JSON envelope", async () => {
    const response = makeMinimalResponse();
    fetchMock.mockResolvedValueOnce(
      new Response(JSON.stringify(response), { status: 200, headers: { "Content-Type": "application/json" } }),
    );

    await client.dispatch("memory_v2_query", { q: "hello" }, { correlationId: "test-cid" });

    expect(fetchMock).toHaveBeenCalledOnce();
    const [url, init] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(url).toBe("http://127.0.0.1:3000/memory/v2/dispatch");
    expect(init.method).toBe("POST");

    const body = JSON.parse(init.body as string) as Record<string, unknown>;
    expect(body.operation).toBe("memory_v2_query");
    expect(body.params).toEqual({ q: "hello" });
    expect(body.correlation_id).toBe("test-cid");
    expect(body.deadline_ms).toBe(DEFAULT_DEADLINE_MS);
  });

  it("throws UnsupportedCapabilityError on non-ok response with 'Unsupported' text", async () => {
    fetchMock.mockResolvedValueOnce(
      new Response("Unsupported: temporal_diff", { status: 422 }),
    );

    await expect(client.dispatch("temporal_diff", {})).rejects.toBeInstanceOf(
      UnsupportedCapabilityError,
    );
  });

  it("throws a plain Error on non-ok non-unsupported response", async () => {
    fetchMock.mockResolvedValueOnce(
      new Response("internal server error", { status: 500 }),
    );

    await expect(client.dispatch("op", {})).rejects.toThrow(/HTTP 500/);
  });

  it("returns the response body unchanged (no local semantic inference)", async () => {
    const response = makeMinimalResponse();
    response.items = [{ id: "y" }];
    fetchMock.mockResolvedValueOnce(
      new Response(JSON.stringify(response), { status: 200, headers: { "Content-Type": "application/json" } }),
    );

    const result = await client.dispatch("op", {});
    expect(result).toStrictEqual(response);
  });
});
