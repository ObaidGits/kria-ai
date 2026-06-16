import { beforeEach, describe, expect, it, vi } from "vitest";

const { invokeMock, listenMock, state } = vi.hoisted(() => {
  const state = {
    sessionRows: [] as Array<Record<string, unknown>>,
    history: [] as any[],
    nextSessionId: 1,
  };
  const invokeMock = vi.fn(async (command: string, _args?: Record<string, unknown>) => {
    switch (command) {
      case "list_sessions":
        return state.sessionRows;
      case "create_session":
        return { session_id: `created-${state.nextSessionId++}` };
      case "switch_session":
        return { session_id: _args?.sessionId, messages: [] };
      case "get_session_history":
        return state.history;
      case "delete_session":
        return { deleted_session_id: _args?.sessionId, replacement_session_id: null };
      case "get_memory_enabled":
        return true;
      default:
        return null;
    }
  });
  const listenMock = vi.fn(async () => () => {});
  vi.stubGlobal("setInterval", vi.fn(() => 1));
  return { invokeMock, listenMock, state };
});

vi.mock("@tauri-apps/api/core", () => ({ invoke: invokeMock }));
vi.mock("@tauri-apps/api/event", () => ({ listen: listenMock }));
vi.mock("highlight.js/styles/github-dark.css?url", () => ({ default: "dark.css" }));
vi.mock("highlight.js/styles/github.css?url", () => ({ default: "light.css" }));

import { appStore } from "./app";

describe("session management — dedup + reuse-empty", () => {
  beforeEach(() => {
    state.sessionRows = [];
    state.history = [];
    state.nextSessionId = 1;
    invokeMock.mockClear();
  });

  it("loadSessions yields one row per id even when the active session is also in the backend list", async () => {
    state.sessionRows = [
      { id: "a", title: "A", turn_count: 2, last_active: new Date().toISOString() },
      { id: "b", title: "B", turn_count: 1, last_active: new Date().toISOString() },
    ];
    // Make "a" the active scoped session, then reload.
    await appStore.switchSession("a");
    await appStore.loadSessions();

    const ids = appStore.sessions().map((s) => s.id);
    const countA = ids.filter((id) => id === "a").length;
    expect(countA).toBe(1);
    // No duplicate ids overall.
    expect(new Set(ids).size).toBe(ids.length);
  });

  it("createSession reuses an empty current chat instead of creating a new session", async () => {
    state.sessionRows = [
      { id: "empty", title: "New chat", turn_count: 0, last_active: new Date().toISOString() },
    ];
    await appStore.switchSession("empty"); // current = "empty", messages = []
    await appStore.loadSessions();

    const before = appStore.sessions().length;
    invokeMock.mockClear();
    await appStore.createSession();

    const createCalls = invokeMock.mock.calls.filter((c) => c[0] === "create_session");
    expect(createCalls).toHaveLength(0);
    expect(appStore.sessions().length).toBe(before);
  });

  it("createSession creates a new session when the current chat has turns", async () => {
    state.sessionRows = [
      { id: "busy", title: "Busy", turn_count: 3, last_active: new Date().toISOString() },
    ];
    await appStore.switchSession("busy");
    await appStore.loadSessions();

    invokeMock.mockClear();
    await appStore.createSession();

    const createCalls = invokeMock.mock.calls.filter((c) => c[0] === "create_session");
    expect(createCalls.length).toBeGreaterThanOrEqual(1);
  });
});
