/**
 * Session resume tests (task 1.5 / Req 1.4).
 *
 * Covers the additions that complete session resume on top of the typed router:
 *   • active Converse thread persisted + restored round-trip
 *   • last Space / selection / scroll restored on a fresh (relaunched) module
 *   • corrupt / partially-malformed / missing persisted state falls back cleanly
 *   • debounced writes coalesce bursts; flushSession forces an immediate write
 *
 * Because the router keeps session state in module-level signals, "relaunch" is
 * simulated by seeding localStorage, calling vi.resetModules(), and dynamically
 * re-importing the module so the boot-time restore path runs again.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { createRoot } from "solid-js";

const STORAGE_KEY = "kria_router_session";

function seedSession(value: unknown): void {
  window.localStorage.setItem(STORAGE_KEY, JSON.stringify(value));
}

function readSession(): any {
  const raw = window.localStorage.getItem(STORAGE_KEY);
  return raw ? JSON.parse(raw) : null;
}

beforeEach(() => {
  window.localStorage.removeItem(STORAGE_KEY);
  vi.resetModules();
});

// ─── Restore on relaunch ─────────────────────────────────────────────────────

describe("session restore on relaunch", () => {
  it("restores last active Space, thread, selection, and scroll", async () => {
    seedSession({
      route: { space: "memory", segment: "graph", entityId: "node-9" },
      spaceStates: { memory: { scrollTop: 420, selection: "mem-7" } },
      activeThreadId: "thread-42",
    });

    const router = await import("./router");

    expect(router.currentRoute()).toEqual({
      space: "memory",
      segment: "graph",
      entityId: "node-9",
    });
    expect(router.getRestoredThreadId()).toBe("thread-42");
    const state = router.getSpaceState("memory");
    expect(state.scrollTop).toBe(420);
    expect(state.selection).toBe("mem-7");
  });

  it("defaults to converse with no thread when nothing is persisted", async () => {
    const router = await import("./router");
    expect(router.currentRoute()).toEqual({ space: "converse" });
    expect(router.getRestoredThreadId()).toBeNull();
  });
});

// ─── Corrupt / malformed state ───────────────────────────────────────────────

describe("graceful fallback for corrupt or missing state", () => {
  it("falls back to default route when JSON is corrupt", async () => {
    window.localStorage.setItem(STORAGE_KEY, "{not valid json");
    const router = await import("./router");
    expect(router.currentRoute()).toEqual({ space: "converse" });
    expect(router.getRestoredThreadId()).toBeNull();
  });

  it("falls back to default when the persisted space is invalid", async () => {
    seedSession({ route: { space: "not-a-space" }, spaceStates: {} });
    const router = await import("./router");
    expect(router.currentRoute()).toEqual({ space: "converse" });
  });

  it("drops malformed spaceStates entries and bad-typed fields", async () => {
    seedSession({
      route: { space: "converse" },
      spaceStates: {
        converse: { scrollTop: "oops", selection: 123 }, // wrong types
        "bogus-space": { scrollTop: 10, selection: "x" }, // invalid space key
        memory: { scrollTop: 88, selection: "keep" }, // valid
      },
      activeThreadId: 999, // wrong type
    });

    const router = await import("./router");

    // Bad-typed converse fields sanitized to defaults.
    const converse = router.getSpaceState("converse");
    expect(converse.scrollTop).toBe(0);
    expect(converse.selection).toBeNull();

    // Valid memory entry preserved.
    const memory = router.getSpaceState("memory");
    expect(memory.scrollTop).toBe(88);
    expect(memory.selection).toBe("keep");

    // Non-string thread id dropped.
    expect(router.getRestoredThreadId()).toBeNull();
  });

  it("treats an empty-string thread id as absent", async () => {
    seedSession({ route: { space: "converse" }, spaceStates: {}, activeThreadId: "" });
    const router = await import("./router");
    expect(router.getRestoredThreadId()).toBeNull();
  });
});

// ─── Thread persistence round-trip ───────────────────────────────────────────

describe("active thread persistence round-trip", () => {
  it("persists the active thread id into the stored session", async () => {
    vi.useFakeTimers();
    try {
      const router = await import("./router");
      createRoot((dispose) => {
        router.initRouterPersistence(300);
        router.setSessionThreadId("thread-abc");
        router.navigate("converse", "thread", "thread-abc");
        vi.advanceTimersByTime(300);

        const stored = readSession();
        expect(stored.activeThreadId).toBe("thread-abc");
        expect(stored.route).toEqual({
          space: "converse",
          segment: "thread",
          entityId: "thread-abc",
        });
        dispose();
      });
    } finally {
      vi.useRealTimers();
    }
  });
});

// ─── Debounce behavior ───────────────────────────────────────────────────────

describe("debounced session writes", () => {
  it("coalesces rapid changes into a single trailing write", async () => {
    vi.useFakeTimers();
    try {
      const router = await import("./router");
      createRoot((dispose) => {
        router.initRouterPersistence(300);

        // Rapid burst — e.g. scroll updates.
        router.setSpaceState("converse", { scrollTop: 10 });
        router.setSpaceState("converse", { scrollTop: 20 });
        router.setSpaceState("converse", { scrollTop: 30 });

        // Nothing written before the debounce window elapses.
        expect(readSession()).toBeNull();

        vi.advanceTimersByTime(299);
        expect(readSession()).toBeNull();

        vi.advanceTimersByTime(1);
        const stored = readSession();
        expect(stored).not.toBeNull();
        expect(stored.spaceStates.converse.scrollTop).toBe(30);
        dispose();
      });
    } finally {
      vi.useRealTimers();
    }
  });

  it("flushSession writes immediately and cancels the pending debounce", async () => {
    vi.useFakeTimers();
    try {
      const router = await import("./router");
      createRoot((dispose) => {
        router.initRouterPersistence(300);
        router.setSpaceState("memory", { scrollTop: 55, selection: "m-1" });

        // Not yet written.
        expect(readSession()).toBeNull();

        router.flushSession();

        const stored = readSession();
        expect(stored).not.toBeNull();
        expect(stored.spaceStates.memory.scrollTop).toBe(55);
        expect(stored.spaceStates.memory.selection).toBe("m-1");
        dispose();
      });
    } finally {
      vi.useRealTimers();
    }
  });
});
