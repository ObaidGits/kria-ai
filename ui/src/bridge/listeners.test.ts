import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { eventBus } from "../stores/eventBus";

const handlers = new Map<string, (event: { payload: unknown }) => void>();
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (channel: string, handler: (event: { payload: unknown }) => void) => {
    handlers.set(channel, handler);
    return () => handlers.delete(channel);
  }),
}));

import { disposeBridgeListeners, initBridgeListeners } from "./listeners";

describe("bridge HRA diagnostics mapping", () => {
  beforeEach(async () => {
    Object.defineProperty(window, "__TAURI_INTERNALS__", {
      configurable: true, value: {},
    });
    await initBridgeListeners();
  });

  afterEach(() => {
    disposeBridgeListeners();
    delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
    handlers.clear();
  });

  it("forwards only object diagnostics payloads", () => {
    const received: unknown[] = [];
    const disconnect = eventBus.on("observatory:hra-diagnostics", (payload) => received.push(payload));
    const dispatch = handlers.get("resource:hra_diagnostics");
    expect(dispatch).toBeDefined();

    const valid = { telemetry: { source: "unified_hub" } };
    dispatch!({ payload: valid });
    dispatch!({ payload: null });
    dispatch!({ payload: [] });
    dispatch!({ payload: "invalid" });

    expect(received).toEqual([valid]);
    disconnect();
  });
});
