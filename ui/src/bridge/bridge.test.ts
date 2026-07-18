/**
 * Tauri Bridge Tests
 *
 * Verifies:
 * - Graceful degradation when services are unavailable (Req 20.4)
 * - Correct classification of errors (unavailable vs regular vs timeout)
 * - Event bus dispatch from Tauri events
 * - Bridge lifecycle (init/dispose idempotency)
 */

import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";

// ─── Tauri runtime presence ─────────────────────────────────────────────────────
// The bridge no-ops when the Tauri runtime is absent. For behavioral tests we
// simulate the runtime being present by injecting the internals marker; the
// dedicated "no-op" tests below delete it to verify graceful degradation.

function setTauriPresent() {
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {
    invoke: () => {},
  };
}

function setTauriAbsent() {
  delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
}

beforeEach(() => {
  setTauriPresent();
});

afterEach(() => {
  setTauriAbsent();
});

// ─── Mock @tauri-apps/api/core ─────────────────────────────────────────────────

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => mockInvoke(...args),
}));

// ─── Mock @tauri-apps/api/event ────────────────────────────────────────────────

type ListenerCallback = (event: { payload: unknown }) => void;
const registeredListeners = new Map<string, ListenerCallback[]>();
const mockListen = vi.fn(async (channel: string, cb: ListenerCallback) => {
  if (!registeredListeners.has(channel)) {
    registeredListeners.set(channel, []);
  }
  registeredListeners.get(channel)!.push(cb);
  return () => {
    const cbs = registeredListeners.get(channel);
    if (cbs) {
      const idx = cbs.indexOf(cb);
      if (idx >= 0) cbs.splice(idx, 1);
    }
  };
});

vi.mock("@tauri-apps/api/event", () => ({
  listen: (channel: string, cb: ListenerCallback) => mockListen(channel, cb),
}));

// ─── Helpers ───────────────────────────────────────────────────────────────────

/** Simulate a Tauri event being emitted to all registered listeners */
function emitTauriEvent(channel: string, payload: unknown) {
  const cbs = registeredListeners.get(channel);
  if (cbs) {
    for (const cb of cbs) {
      cb({ payload });
    }
  }
}

// ─── Tests ─────────────────────────────────────────────────────────────────────

describe("bridge/invoke", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockInvoke.mockReset();
  });

  it("bridgeInvoke returns ok result on success", async () => {
    const { bridgeInvoke } = await import("./invoke");
    mockInvoke.mockResolvedValueOnce({ sessions: [] });

    const result = await bridgeInvoke<{ sessions: unknown[] }>("list_sessions");

    expect(result.ok).toBe(true);
    if (result.ok) {
      expect(result.data).toEqual({ sessions: [] });
    }
  });

  it("bridgeInvoke returns unavailable for optional commands that fail", async () => {
    const { bridgeInvoke } = await import("./invoke");
    mockInvoke.mockRejectedValueOnce(new Error("service not available"));

    const result = await bridgeInvoke("connect_colab_tier");

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe("unavailable");
      expect(result.message).toContain("not available");
    }
  });

  it("bridgeInvoke returns unavailable when error matches unavailability patterns", async () => {
    const { bridgeInvoke } = await import("./invoke");
    mockInvoke.mockRejectedValueOnce(new Error("connection refused"));

    const result = await bridgeInvoke("some_command");

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe("unavailable");
    }
  });

  it("bridgeInvoke returns error for non-unavailability failures", async () => {
    const { bridgeInvoke } = await import("./invoke");
    mockInvoke.mockRejectedValueOnce(new Error("invalid argument: name is required"));

    const result = await bridgeInvoke("create_session");

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe("error");
      expect(result.message).toContain("invalid argument");
    }
  });

  it("bridgeInvoke returns timeout when command exceeds deadline", async () => {
    const { bridgeInvoke } = await import("./invoke");
    // Simulate a never-resolving promise
    mockInvoke.mockImplementationOnce(
      () => new Promise(() => {}) // Never resolves
    );

    const result = await bridgeInvoke("slow_command", undefined, { timeoutMs: 50 });

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe("timeout");
      if (result.code === "timeout") {
        expect(result.command).toBe("slow_command");
        expect(result.timeoutMs).toBe(50);
      }
    }
  });

  it("bridgeInvokeOptional returns null for unavailable services", async () => {
    const { bridgeInvokeOptional } = await import("./invoke");
    mockInvoke.mockRejectedValueOnce(new Error("sidecar not running"));

    const result = await bridgeInvokeOptional("get_telegram_config");

    expect(result).toBeNull();
  });

  it("bridgeInvokeOptional returns defaultValue when provided", async () => {
    const { bridgeInvokeOptional } = await import("./invoke");
    mockInvoke.mockRejectedValueOnce(new Error("not connected"));

    const result = await bridgeInvokeOptional<string[]>("list_mcp_servers", undefined, {
      defaultValue: [],
    });

    expect(result).toEqual([]);
  });

  it("bridgeInvokeOptional returns data on success", async () => {
    const { bridgeInvokeOptional } = await import("./invoke");
    mockInvoke.mockResolvedValueOnce({ enabled: true });

    const result = await bridgeInvokeOptional("get_telegram_config");

    expect(result).toEqual({ enabled: true });
  });

  it("bridgeInvoke no-ops with unavailable result when Tauri runtime is absent", async () => {
    const { bridgeInvoke } = await import("./invoke");
    setTauriAbsent();

    const result = await bridgeInvoke("list_sessions");

    expect(mockInvoke).not.toHaveBeenCalled();
    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe("unavailable");
    }
  });

  it("bridgeInvokeOptional returns null when Tauri runtime is absent", async () => {
    const { bridgeInvokeOptional } = await import("./invoke");
    setTauriAbsent();

    const result = await bridgeInvokeOptional("get_telegram_config");

    expect(mockInvoke).not.toHaveBeenCalled();
    expect(result).toBeNull();
  });
});

describe("bridge/types", () => {
  it("isUnavailableError detects unavailability patterns", async () => {
    const { isUnavailableError } = await import("./types");

    expect(isUnavailableError("connection refused")).toBe(true);
    expect(isUnavailableError("service unavailable")).toBe(true);
    expect(isUnavailableError("not found")).toBe(true);
    expect(isUnavailableError("not initialized")).toBe(true);
    expect(isUnavailableError("sidecar not running")).toBe(true);
    expect(isUnavailableError(new Error("timed out"))).toBe(true);
  });

  it("isUnavailableError returns false for regular errors", async () => {
    const { isUnavailableError } = await import("./types");

    expect(isUnavailableError("invalid argument")).toBe(false);
    expect(isUnavailableError("permission denied")).toBe(false);
    expect(isUnavailableError("validation failed")).toBe(false);
  });

  it("extractErrorMessage handles various error shapes", async () => {
    const { extractErrorMessage } = await import("./types");

    expect(extractErrorMessage("plain string")).toBe("plain string");
    expect(extractErrorMessage(new Error("error object"))).toBe("error object");
    expect(extractErrorMessage({ message: "object with message" })).toBe("object with message");
    expect(extractErrorMessage(42)).toBe("42");
  });
});

describe("bridge/listeners", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockListen.mockClear();
    registeredListeners.clear();
  });

  afterEach(async () => {
    // Reset module state between tests
    const { disposeBridgeListeners } = await import("./listeners");
    disposeBridgeListeners();
    registeredListeners.clear();
  });

  it("initBridgeListeners attaches listeners for all known channels", async () => {
    const { initBridgeListeners, disposeBridgeListeners } = await import("./listeners");
    const { EVENT_CHANNELS } = await import("./types");

    // Ensure clean state
    disposeBridgeListeners();
    registeredListeners.clear();
    mockListen.mockClear();

    const totalChannels = Object.values(EVENT_CHANNELS).flat().length;
    const attached = await initBridgeListeners();

    expect(attached).toBe(totalChannels);
    expect(mockListen).toHaveBeenCalledTimes(totalChannels);

    disposeBridgeListeners();
  });

  it("initBridgeListeners no-ops (0 listeners) when Tauri runtime is absent", async () => {
    const { initBridgeListeners, disposeBridgeListeners } = await import("./listeners");

    disposeBridgeListeners();
    registeredListeners.clear();
    mockListen.mockClear();
    setTauriAbsent();

    const attached = await initBridgeListeners();

    expect(attached).toBe(0);
    expect(mockListen).not.toHaveBeenCalled();

    disposeBridgeListeners();
  });

  it("initBridgeListeners gracefully handles failed listeners", async () => {
    vi.resetModules();
    const failingListen = vi.fn(async (channel: string, cb: ListenerCallback) => {
      if (channel === "voice:state") {
        throw new Error("listen failed for voice:state");
      }
      registeredListeners.set(channel, [cb]);
      return () => {};
    });

    vi.doMock("@tauri-apps/api/core", () => ({
      invoke: (...args: unknown[]) => mockInvoke(...args),
    }));
    vi.doMock("@tauri-apps/api/event", () => ({
      listen: (channel: string, cb: ListenerCallback) => failingListen(channel, cb),
    }));

    const { initBridgeListeners, disposeBridgeListeners } = await import("./listeners");
    const { EVENT_CHANNELS } = await import("./types");

    const totalChannels = Object.values(EVENT_CHANNELS).flat().length;
    const attached = await initBridgeListeners();

    // Should attach all minus the one that failed
    expect(attached).toBe(totalChannels - 1);

    disposeBridgeListeners();
  });

  it("dispatchToEventBus maps voice:transcript to bus", async () => {
    // This test verifies that the listeners module correctly dispatches to
    // the event bus. We use a spy approach: spy on eventBus.emit and verify
    // it's called correctly when a Tauri event fires through the mock listener.
    const { eventBus } = await import("../stores/eventBus");
    const emitSpy = vi.spyOn(eventBus, "emit");

    // The top-level mockListen captures callbacks in registeredListeners
    const { initBridgeListeners, disposeBridgeListeners } = await import("./listeners");

    // Dispose any previous state and reinitialize
    disposeBridgeListeners();
    await initBridgeListeners();

    // Simulate Tauri emitting a transcript event
    const cbs = registeredListeners.get("voice:transcript");
    expect(cbs).toBeDefined();
    expect(cbs!.length).toBeGreaterThan(0);
    for (const cb of cbs!) {
      cb({ payload: { text: "hello world", confidence: 0.95 } });
    }

    expect(emitSpy).toHaveBeenCalledWith("voice:transcript", {
      text: "hello world",
      partial: false,
    });

    emitSpy.mockRestore();
    disposeBridgeListeners();
  });

  it("dispatchToEventBus maps memory://changed to bus", async () => {
    const { eventBus } = await import("../stores/eventBus");
    const emitSpy = vi.spyOn(eventBus, "emit");

    const { initBridgeListeners, disposeBridgeListeners } = await import("./listeners");
    disposeBridgeListeners();
    await initBridgeListeners();

    const cbs = registeredListeners.get("memory://changed");
    expect(cbs).toBeDefined();
    expect(cbs!.length).toBeGreaterThan(0);
    for (const cb of cbs!) {
      cb({ payload: { kind: "created", fact_id: "fact-123" } });
    }

    expect(emitSpy).toHaveBeenCalledWith("memory:updated", {
      factId: "fact-123",
      kind: "created",
    });

    emitSpy.mockRestore();
    disposeBridgeListeners();
  });

  it("dispatchToEventBus ignores unmapped events without error", async () => {
    const { eventBus } = await import("../stores/eventBus");
    const emitSpy = vi.spyOn(eventBus, "emit");

    const { initBridgeListeners, disposeBridgeListeners } = await import("./listeners");
    disposeBridgeListeners();
    await initBridgeListeners();

    // voice:debug has no mapping — should not emit anything
    const cbs = registeredListeners.get("voice:debug");
    expect(cbs).toBeDefined();
    expect(() => {
      for (const cb of cbs!) {
        cb({ payload: { stage: "stt_start" } });
      }
    }).not.toThrow();

    // emit should NOT have been called for unmapped events
    const debugCalls = emitSpy.mock.calls.filter(
      ([name]) => String(name) === "voice:debug"
    );
    expect(debugCalls).toHaveLength(0);

    emitSpy.mockRestore();
    disposeBridgeListeners();
  });
});

describe("bridge/tauriBridge", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    registeredListeners.clear();
  });

  it("init is idempotent — second call returns cached count", async () => {
    const { tauriBridge } = await import("./tauriBridge");
    // Ensure clean state
    tauriBridge.dispose();

    const first = await tauriBridge.init();
    const second = await tauriBridge.init();

    expect(first.listenerCount).toBe(second.listenerCount);
    expect(tauriBridge.isInitialized).toBe(true);

    tauriBridge.dispose();
    expect(tauriBridge.isInitialized).toBe(false);
  });

  it("dispose resets state fully", async () => {
    const { tauriBridge } = await import("./tauriBridge");
    tauriBridge.dispose();

    await tauriBridge.init();
    expect(tauriBridge.isInitialized).toBe(true);
    expect(tauriBridge.listenerCount).toBeGreaterThan(0);

    tauriBridge.dispose();
    expect(tauriBridge.isInitialized).toBe(false);
    expect(tauriBridge.listenerCount).toBe(0);
    expect(tauriBridge.initTimeMs).toBeNull();
  });
});
