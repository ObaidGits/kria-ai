import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { EventBus, MAX_RAF_EVENTS_PER_FRAME } from "./eventBus";

describe("EventBus", () => {
  let bus: EventBus;

  beforeEach(() => {
    bus = new EventBus();
  });

  afterEach(() => {
    bus.clear();
  });

  describe("basic pub/sub", () => {
    it("delivers events to subscribers synchronously (coalesce: none)", () => {
      const handler = vi.fn();
      bus.on("shell:space-changed", handler, "none");

      bus.emit("shell:space-changed", { space: "memory", previous: "converse" });

      expect(handler).toHaveBeenCalledTimes(1);
      expect(handler).toHaveBeenCalledWith({ space: "memory", previous: "converse" });
    });

    it("supports multiple subscribers for the same event", () => {
      const h1 = vi.fn();
      const h2 = vi.fn();
      bus.on("core:state-changed", h1, "none");
      bus.on("core:state-changed", h2, "none");

      bus.emit("core:state-changed", { state: "thinking", previous: "idle" });

      expect(h1).toHaveBeenCalledTimes(1);
      expect(h2).toHaveBeenCalledTimes(1);
    });

    it("does not fire after unsubscribe", () => {
      const handler = vi.fn();
      const unsub = bus.on("shell:theme-changed", handler, "none");

      bus.emit("shell:theme-changed", { theme: "dark" });
      expect(handler).toHaveBeenCalledTimes(1);

      unsub();
      bus.emit("shell:theme-changed", { theme: "light" });
      expect(handler).toHaveBeenCalledTimes(1);
    });

    it("once() fires handler only once then unsubscribes", () => {
      const handler = vi.fn();
      bus.once("notification:push", handler);

      bus.emit("notification:push", { id: "1", level: "info", message: "hi" });
      bus.emit("notification:push", { id: "2", level: "warn", message: "again" });

      expect(handler).toHaveBeenCalledTimes(1);
      expect(handler).toHaveBeenCalledWith({ id: "1", level: "info", message: "hi" });
    });

    it("does nothing when emitting to an event with no subscribers", () => {
      // Should not throw
      expect(() => {
        bus.emit("memory:updated", { factId: "abc" });
      }).not.toThrow();
    });
  });

  describe("hasSubscribers", () => {
    it("returns false when no subscribers", () => {
      expect(bus.hasSubscribers("shell:space-changed")).toBe(false);
    });

    it("returns true after subscribing", () => {
      bus.on("shell:space-changed", () => {}, "none");
      expect(bus.hasSubscribers("shell:space-changed")).toBe(true);
    });

    it("returns false after unsubscribing last listener", () => {
      const unsub = bus.on("shell:space-changed", () => {}, "none");
      unsub();
      expect(bus.hasSubscribers("shell:space-changed")).toBe(false);
    });
  });

  describe("clear()", () => {
    it("removes all subscriptions", () => {
      bus.on("shell:space-changed", () => {}, "none");
      bus.on("core:state-changed", () => {}, "none");

      bus.clear();

      expect(bus.hasSubscribers("shell:space-changed")).toBe(false);
      expect(bus.hasSubscribers("core:state-changed")).toBe(false);
    });
  });

  describe("rAF coalescing", () => {
    it("batches high-freq events and delivers on next frame", async () => {
      const handler = vi.fn();
      // Explicitly set raf mode
      bus.on("converse:token", handler, "raf");

      // Emit multiple tokens rapidly
      bus.emit("converse:token", { sessionId: "s1", token: "a" });
      bus.emit("converse:token", { sessionId: "s1", token: "b" });
      bus.emit("converse:token", { sessionId: "s1", token: "c" });

      // Not yet delivered (batched)
      expect(handler).not.toHaveBeenCalled();

      // Wait for rAF (jsdom uses setTimeout fallback ~16ms)
      await new Promise((r) => setTimeout(r, 50));

      // All three delivered in one flush
      expect(handler).toHaveBeenCalledTimes(3);
      expect(handler).toHaveBeenNthCalledWith(1, { sessionId: "s1", token: "a" });
      expect(handler).toHaveBeenNthCalledWith(2, { sessionId: "s1", token: "b" });
      expect(handler).toHaveBeenNthCalledWith(3, { sessionId: "s1", token: "c" });
    });

    it("auto-detects high-freq events for rAF coalescing", async () => {
      const handler = vi.fn();
      // No explicit coalesce mode — should auto-detect "converse:token" as high-freq
      bus.on("converse:token", handler);

      bus.emit("converse:token", { sessionId: "s1", token: "x" });

      // Should be batched, not immediate
      expect(handler).not.toHaveBeenCalled();

      await new Promise((r) => setTimeout(r, 50));
      expect(handler).toHaveBeenCalledTimes(1);
    });
  });

  describe("microtask coalescing", () => {
    it("batches events and delivers on next microtask", async () => {
      const handler = vi.fn();
      bus.on("observatory:telemetry", handler, "microtask");

      bus.emit("observatory:telemetry", { metric: "cpu", value: 50, ts: 1 });
      bus.emit("observatory:telemetry", { metric: "cpu", value: 55, ts: 2 });

      // Not yet delivered
      expect(handler).not.toHaveBeenCalled();

      // Wait for microtask to flush
      await Promise.resolve();
      await Promise.resolve(); // Extra tick for safety

      expect(handler).toHaveBeenCalledTimes(2);
    });
  });

  describe("mixed coalesce modes on the same event", () => {
    it("immediate subscriber fires immediately, raf subscriber deferred", async () => {
      const immediate = vi.fn();
      const deferred = vi.fn();

      bus.on("shell:space-changed", immediate, "none");
      bus.on("shell:space-changed", deferred, "raf");

      bus.emit("shell:space-changed", { space: "memory", previous: "converse" });

      // immediate fires now
      expect(immediate).toHaveBeenCalledTimes(1);
      // deferred not yet
      expect(deferred).not.toHaveBeenCalled();

      await new Promise((r) => setTimeout(r, 50));
      expect(deferred).toHaveBeenCalledTimes(1);
    });
  });
});


describe("EventBus — heavy model load interactivity (Req 16.5/16.6)", () => {
  it("bounds each token drain while Stop remains synchronous", () => {
    const callbacks: FrameRequestCallback[] = [];
    vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => {
      callbacks.push(cb);
      return callbacks.length;
    });
    const bus = new EventBus();
    const tokens = vi.fn();
    const stop = vi.fn();
    bus.on("converse:token", tokens);
    bus.on("converse:work-cancel-requested", stop);

    const total = MAX_RAF_EVENTS_PER_FRAME * 4;
    for (let i = 0; i < total; i += 1) {
      bus.emit("converse:token", { sessionId: "heavy-model", token: String(i) });
    }
    bus.emit("converse:work-cancel-requested", { blockId: "work-1", blockType: "tool-call" });

    expect(stop).toHaveBeenCalledTimes(1);
    expect(tokens).not.toHaveBeenCalled();
    callbacks.shift()!(0);
    expect(tokens).toHaveBeenCalledTimes(MAX_RAF_EVENTS_PER_FRAME);
    expect(callbacks.length).toBe(1);

    while (callbacks.length > 0) callbacks.shift()!(0);
    expect(tokens).toHaveBeenCalledTimes(total);
    bus.clear();
    vi.unstubAllGlobals();
  });

  it("queues each event once when multiple rAF subscribers exist", () => {
    const callbacks: FrameRequestCallback[] = [];
    vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => {
      callbacks.push(cb);
      return callbacks.length;
    });
    const bus = new EventBus();
    const first = vi.fn();
    const second = vi.fn();
    bus.on("converse:token", first);
    bus.on("converse:token", second);

    bus.emit("converse:token", { sessionId: "s1", token: "x" });
    callbacks.shift()!(0);

    expect(first).toHaveBeenCalledTimes(1);
    expect(second).toHaveBeenCalledTimes(1);
    bus.clear();
    vi.unstubAllGlobals();
  });
});
