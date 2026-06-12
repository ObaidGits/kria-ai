import { beforeEach, describe, expect, it, vi } from "vitest";

const { invokeMock, listenMock, listenerMap, setSessionHistory } = vi.hoisted(() => {
  const listenerMap = new Map<string, (event: { payload: any }) => void>();
  let sessionHistory: any[] = [];

  const invokeMock = vi.fn(async (command: string, args?: Record<string, unknown>) => {
    switch (command) {
      case "send_message":
        return { status: "ok", message: args?.message };
      case "send_manual_tool_message":
        return { status: "ok", message: args?.message, profile: args?.profile };
      case "create_session":
        return { session_id: "mock-created-session" };
      case "list_sessions":
        return [];
      case "clear_all_chat_sessions":
        return {
          deleted_session_count: 2,
          deleted_turn_count: 5,
          replacement_session_id: "mock-cleared-session",
        };
      case "switch_session":
        return { session_id: args?.sessionId ?? "mock-session", messages: [] };
      case "delete_session":
        return { deleted_session_id: args?.sessionId, replacement_session_id: null };
      case "get_session_history":
        return sessionHistory;
      case "cancel_turn":
        return {};
      case "get_settings":
        return {
          llm: {},
          voice: {},
          safety: {},
          ui: { theme: "dark" },
          server: {},
          memory: {},
        };
      case "list_audio_devices":
        return {
          inputs: [],
          outputs: [],
          default_input: null,
          default_output: null,
        };
      case "get_health":
        return {
          status: "healthy",
          services: [{ name: "model_router", status: "healthy" }],
        };
      case "set_google_workspace_account":
        return { account: args?.account ?? "personal", updated: true };
      case "reconcile_mcp_runtime":
        return { reconciled: true };
      case "restart_mcp_server_runtime":
        return { restarted: true, name: args?.name ?? null };
      default:
        return null;
    }
  });

  const listenMock = vi.fn(async (eventName: string, callback: (event: { payload: any }) => void) => {
    listenerMap.set(eventName, callback);
    return () => listenerMap.delete(eventName);
  });

  vi.stubGlobal("setInterval", vi.fn(() => 1));

  return {
    invokeMock,
    listenMock,
    listenerMap,
    setSessionHistory: (history: any[]) => {
      sessionHistory = history;
    },
  };
});

vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: listenMock,
}));

vi.mock("highlight.js/styles/github-dark.css?url", () => ({ default: "dark.css" }));
vi.mock("highlight.js/styles/github.css?url", () => ({ default: "light.css" }));

import { appStore } from "./app";
import {
  __resetGuiCognitionSessionForTests,
  activeGuiCognitionSession,
} from "./guiCognitionSession";

function emit(eventName: string, payload: any) {
  const callback = listenerMap.get(eventName);
  if (!callback) {
    throw new Error(`Missing listener for ${eventName}`);
  }
  callback({ payload });
}

async function flushAsync(cycles = 2) {
  for (let i = 0; i < cycles; i += 1) {
    await Promise.resolve();
  }
}

describe("appStore low-confidence tool choice flow", () => {
  beforeEach(() => {
    invokeMock.mockClear();
    setSessionHistory([]);
    appStore.dismissToolChoice();
    appStore.setManualToolMode("auto");
    __resetGuiCognitionSessionForTests();
  });

  it("captures tool-choice event and clears thinking state", () => {
    emit("agent:thinking", { status: "planning" });
    expect(appStore.isThinking()).toBe(true);

    const payload = {
      query: "check my unread emails",
      confidence: 0.46,
      minConfidence: 0.55,
      candidates: [
        {
          name: "gw_gmail_inbox",
          label: "Gmail",
          reason: "Primary match from intent classifier",
          confidence: 0.46,
        },
      ],
    };

    emit("agent:tool_choice_required", payload);

    expect(appStore.toolChoiceRequest()).toEqual(payload);
    expect(appStore.isThinking()).toBe(false);
  });

  it("submits a forced-tool continuation prompt", async () => {
    emit("agent:tool_choice_required", {
      query: "check my unread emails",
      confidence: 0.46,
      minConfidence: 0.55,
      candidates: [
        {
          name: "gw_gmail_inbox",
          label: "Gmail",
          reason: "Primary match from intent classifier",
          confidence: 0.46,
        },
      ],
    });

    appStore.submitToolChoice("gw_gmail_inbox");
    await flushAsync(8);

    expect(invokeMock).toHaveBeenCalledWith("send_message", {
      message: "#tool:gw_gmail_inbox check my unread emails",
    });
    expect(appStore.toolChoiceRequest()).toBeNull();
  });

  it("dismisses pending tool choice without sending", () => {
    emit("agent:tool_choice_required", {
      query: "find files",
      confidence: 0.42,
      minConfidence: 0.55,
      candidates: [],
    });

    appStore.dismissToolChoice();

    expect(appStore.toolChoiceRequest()).toBeNull();
    expect(invokeMock).not.toHaveBeenCalledWith(
      "send_message",
      expect.objectContaining({ message: expect.stringContaining("#tool:") }),
    );
  });
});

describe("appStore manual tool selection mode", () => {
  beforeEach(() => {
    invokeMock.mockClear();
    setSessionHistory([]);
    emit("agent:done", {});
    appStore.dismissToolChoice();
    appStore.setManualToolMode("auto");
    __resetGuiCognitionSessionForTests();
    window.localStorage.clear();
  });

  it("builds manual profiles directly from the manual mode catalog", () => {
    for (const mode of appStore.manualToolModes) {
      const profile = appStore.buildManualToolProfile(mode.id);

      if (mode.id === "auto") {
        expect(profile).toBeNull();
      } else {
        expect(profile).toEqual({
          mode_id: mode.id,
          label: mode.label,
          app_lock: mode.appLock,
          tool_lock: mode.toolLock,
          strategy: mode.strategy,
        });
      }
    }
  });

  it("keeps auto mode on the normal assistant command path", async () => {
    appStore.setManualToolMode("auto");

    await appStore.sendMessage("check the weather");
    await flushAsync(8);

    expect(invokeMock).toHaveBeenCalledWith("send_message", {
      message: "check the weather",
    });
    expect(invokeMock).not.toHaveBeenCalledWith(
      "send_manual_tool_message",
      expect.anything(),
    );
  });

  it("sends the Desktop chat turn after session activation is settled", async () => {
    appStore.setManualToolMode("auto");

    await appStore.sendMessage("Update workflow exact-id so it accepts title from prompt");
    await flushAsync(8);

    const switchSessionCall = invokeMock.mock.calls.findIndex(
      ([command]) => command === "switch_session",
    );
    const sendMessageCall = invokeMock.mock.calls.findIndex(
      ([command, args]) =>
        command === "send_message" &&
        args?.message === "Update workflow exact-id so it accepts title from prompt",
    );

    expect(sendMessageCall).toBeGreaterThanOrEqual(0);
    if (switchSessionCall >= 0) {
      expect(invokeMock.mock.invocationCallOrder[switchSessionCall]).toBeLessThan(
        invokeMock.mock.invocationCallOrder[sendMessageCall],
      );
    }
    expect(
      appStore.messages().some(
        (message) =>
          message.role === "user" &&
          message.content === "Update workflow exact-id so it accepts title from prompt",
      ),
    ).toBe(true);
  });

  it("keeps a newly created chat visible when the backend session list is temporarily stale", async () => {
    appStore.setCurrentEnvironment("assistant");

    await appStore.createSession();
    await flushAsync(8);

    expect(appStore.currentSession()).toBe("mock-created-session");
    expect(
      appStore.sessions().some((session) => session.id === "mock-created-session"),
    ).toBe(true);
  });

  it("keeps the second prompt and assistant response visible in the active chat", async () => {
    appStore.setCurrentEnvironment("assistant");
    await appStore.switchSession("second-prompt-session");
    await flushAsync(4);

    await appStore.sendMessage("first n8n prompt");
    emit("agent:token", { text: "first response" });
    emit("agent:done", {});

    await appStore.sendMessage("second n8n prompt");
    await flushAsync(4);

    expect(
      appStore.messages().some(
        (message) => message.role === "user" && message.content === "second n8n prompt",
      ),
    ).toBe(true);

    emit("agent:token", { text: "second response" });
    emit("agent:done", {});

    expect(
      appStore.messages().some(
        (message) => message.role === "assistant" && message.content.includes("second response"),
      ),
    ).toBe(true);
  });

  it("renders sequential GUI Cognition prompts and replies in the active chat", async () => {
    appStore.setCurrentEnvironment("assistant");
    appStore.setManualToolMode("gui_cognition");
    await appStore.switchSession("gui-cognition-seq-session");
    await flushAsync(4);

    // ── Turn 1 ──────────────────────────────────────────────────────────
    await appStore.sendMessage("Open the Calculator");
    await flushAsync(8);
    // Backend (gui_cognition path) emits this batch once the turn completes.
    emit("agent:thinking", { status: "processing", mode: "gui_cognition" });
    emit("agent:token", { text: "Workflow completed 2 verified step(s) safely." });
    emit("agent:done", {});
    await flushAsync(4);

    expect(appStore.isThinking()).toBe(false);
    expect(
      appStore.messages().some(
        (m) => m.role === "user" && m.content === "Open the Calculator",
      ),
    ).toBe(true);
    expect(
      appStore.messages().some(
        (m) => m.role === "assistant" && m.content.includes("Workflow completed"),
      ),
    ).toBe(true);

    // ── Turn 2 (same chat) ──────────────────────────────────────────────
    await appStore.sendMessage("Open the Screenshot tool");
    await flushAsync(8);
    emit("agent:thinking", { status: "processing", mode: "gui_cognition" });
    emit("agent:token", { text: "Screenshot tool opened and verified." });
    emit("agent:done", {});
    await flushAsync(4);

    expect(appStore.isThinking()).toBe(false);
    const userContents = appStore
      .messages()
      .filter((m) => m.role === "user")
      .map((m) => m.content);
    expect(userContents).toContain("Open the Calculator");
    expect(userContents).toContain("Open the Screenshot tool");
    expect(
      appStore.messages().some(
        (m) => m.role === "assistant" && m.content.includes("Screenshot tool opened"),
      ),
    ).toBe(true);
  });

  it("does not submit or clear a new prompt while the assistant turn is still active", async () => {
    emit("agent:thinking", { status: "processing" });

    await appStore.sendMessage("second prompt while busy");
    await flushAsync(4);

    expect(invokeMock).not.toHaveBeenCalledWith("send_message", {
      message: "second prompt while busy",
    });
    expect(
      appStore.messages().some(
        (message) => message.role === "user" && message.content === "second prompt while busy",
      ),
    ).toBe(false);

    emit("agent:done", {});
  });

  it("keeps unsaved active-session messages when backend history is temporarily stale", async () => {
    appStore.setCurrentEnvironment("assistant");
    await appStore.switchSession("stale-history-session");
    await flushAsync(4);

    await appStore.sendMessage("first persisted prompt");
    emit("agent:token", { text: "first persisted response" });
    emit("agent:done", {});

    await appStore.sendMessage("second local prompt");
    await flushAsync(4);

    setSessionHistory([
      {
        role: "user",
        content: "first persisted prompt",
        timestamp: "2026-06-02T10:00:00.000Z",
      },
      {
        role: "assistant",
        content: "first persisted response",
        timestamp: "2026-06-02T10:00:01.000Z",
      },
    ]);

    await appStore.switchSession("stale-history-session");
    await flushAsync(4);

    expect(
      appStore.messages().some(
        (message) => message.role === "user" && message.content === "second local prompt",
      ),
    ).toBe(true);

    emit("agent:done", {});
  });

  it("new chat cancels an active turn and becomes visible immediately", async () => {
    emit("agent:thinking", { status: "processing" });

    await appStore.createSession();
    await flushAsync(8);

    expect(invokeMock).toHaveBeenCalledWith("cancel_turn", {
      sessionId: expect.any(String),
    });
    expect(appStore.isThinking()).toBe(false);
    expect(appStore.currentSession()).toBe("mock-created-session");
    expect(
      appStore.sessions().some((session) => session.id === "mock-created-session"),
    ).toBe(true);
  });

  it("clears all chat sessions and switches to the replacement session", async () => {
    appStore.setCurrentEnvironment("assistant");
    await appStore.switchSession("old-session");
    await flushAsync(4);

    await appStore.sendMessage("local prompt before clear");
    await flushAsync(4);

    const result = await appStore.clearAllChatSessions();
    await flushAsync(8);

    expect(invokeMock).toHaveBeenCalledWith("clear_all_chat_sessions");
    expect(result).toEqual({ deletedSessionCount: 2, deletedTurnCount: 5 });
    expect(appStore.currentSession()).toBe("mock-cleared-session");
    expect(appStore.messages()).toEqual([]);
    expect(appStore.sessions().some((session) => session.id === "mock-cleared-session")).toBe(true);
  });

  it("sends manual n8n mode through the manual profile command", async () => {
    appStore.setManualToolMode("n8n");

    await appStore.sendMessage("run test_workflow");
    await flushAsync(8);

    expect(invokeMock).toHaveBeenCalledWith("send_manual_tool_message", {
      message: "run test_workflow",
      profile: {
        mode_id: "n8n",
        label: "n8n",
        app_lock: null,
        tool_lock: "n8n_invoke_workflow",
        strategy: "direct",
      },
    });
    expect(invokeMock).not.toHaveBeenCalledWith("send_message", {
      message: "run test_workflow",
    });
  });

  it("sends GUI Cognition mode through the dedicated manual profile command", async () => {
    appStore.setManualToolMode("gui_cognition");

    await appStore.sendMessage("observe the current screen");
    await flushAsync(8);

    expect(invokeMock).toHaveBeenCalledWith("send_manual_tool_message", {
      message: "observe the current screen",
      profile: {
        mode_id: "gui_cognition",
        label: "GUI Cognition",
        app_lock: "gui_cognition",
        tool_lock: null,
        strategy: "routed_within_lock",
      },
    });
    expect(invokeMock).not.toHaveBeenCalledWith("send_message", {
      message: "observe the current screen",
    });
  });

  it("sends every non-auto manual mode through the manual profile command", async () => {
    const manualModes = appStore.manualToolModes.filter((mode) => mode.id !== "auto");

    for (const mode of manualModes) {
      invokeMock.mockClear();
      emit("agent:done", {});
      appStore.setManualToolMode(mode.id);

      await appStore.sendMessage(`route through ${mode.id}`);
      await flushAsync(8);

      expect(invokeMock).toHaveBeenCalledWith("send_manual_tool_message", {
        message: `route through ${mode.id}`,
        profile: {
          mode_id: mode.id,
          label: mode.label,
          app_lock: mode.appLock,
          tool_lock: mode.toolLock,
          strategy: mode.strategy,
        },
      });
      expect(invokeMock).not.toHaveBeenCalledWith("send_message", {
        message: `route through ${mode.id}`,
      });
    }
  });

  it("uses the latest selected manual profile when modes switch", async () => {
    const sequence = [
      { mode: "n8n" as const, message: "n8n route check" },
      { mode: "gui_cognition" as const, message: "gui route check" },
      { mode: "browser" as const, message: "browser route check" },
    ];

    for (const item of sequence) {
      appStore.setManualToolMode(item.mode);
      await appStore.sendMessage(item.message);
      await flushAsync(8);
      emit("agent:done", {});
    }

    const manualCalls = invokeMock.mock.calls.filter(
      ([command]) => command === "send_manual_tool_message"
    );
    expect(manualCalls.map(([, args]) => (args as any)?.profile?.mode_id)).toEqual([
      "n8n",
      "gui_cognition",
      "browser",
    ]);
    expect(manualCalls[1]?.[1]).toEqual({
      message: "gui route check",
      profile: {
        mode_id: "gui_cognition",
        label: "GUI Cognition",
        app_lock: "gui_cognition",
        tool_lock: null,
        strategy: "routed_within_lock",
      },
    });
  });

  it("normalizes unknown manual mode values back to auto", () => {
    appStore.setManualToolMode("not-a-real-mode" as any);

    expect(appStore.manualToolMode()).toBe("auto");
    expect(window.localStorage.getItem("kria_manual_tool_mode")).toBeNull();
    expect(appStore.buildManualToolProfile(appStore.manualToolMode())).toBeNull();
  });

  it("does not send another message while the assistant route is already thinking", async () => {
    appStore.setManualToolMode("gui_cognition");

    await appStore.sendMessage("first gui prompt");
    await flushAsync(8);
    await appStore.sendMessage("second gui prompt should be ignored");
    await flushAsync(4);

    const manualCalls = invokeMock.mock.calls.filter(
      ([command]) => command === "send_manual_tool_message"
    );
    expect(manualCalls).toHaveLength(1);
    expect(manualCalls[0]?.[1]?.message).toBe("first gui prompt");
  });

  it("updates GUI Cognition session state from canonical events", () => {
    appStore.setManualToolMode("gui_cognition");
    emit("gui_cognition:event", {
      version: 1,
      session_id: "session-1",
      turn_id: "turn-1",
      workflow_id: "workflow-1",
      sequence: 1,
      timestamp_ms: Date.now(),
      event: { type: "TurnStarted", mode_id: "gui_cognition" },
    });
    emit("gui_cognition:event", {
      version: 1,
      session_id: "session-1",
      turn_id: "turn-1",
      workflow_id: "workflow-1",
      sequence: 2,
      timestamp_ms: Date.now(),
      event: {
        type: "ObservationCompleted",
        active_window: "Kria",
        visible_control_count: 4,
        ocr_available: false,
        accessibility_available: true,
      },
    });

    expect(activeGuiCognitionSession()?.observation.activeWindow).toBe("Kria");
    expect(activeGuiCognitionSession()?.observation.visibleControlCount).toBe(4);
    expect(appStore.manualToolMode()).toBe("gui_cognition");
  });

  it("persists the last selected manual mode", () => {
    appStore.setManualToolMode("browser");

    expect(appStore.manualToolMode()).toBe("browser");
    expect(window.localStorage.getItem("kria_manual_tool_mode")).toBe("browser");

    appStore.setManualToolMode("auto");
    expect(window.localStorage.getItem("kria_manual_tool_mode")).toBeNull();
  });
});

describe("appStore Google runtime command wiring", () => {
  beforeEach(() => {
    invokeMock.mockClear();
    setSessionHistory([]);
  });

  it("routes Google account/runtime actions to backend commands", async () => {
    await appStore.setGoogleAccount("work");
    await appStore.reconcileMcpRuntime();
    await appStore.restartMcpServerRuntime("gworkspace");

    expect(invokeMock).toHaveBeenCalledWith("set_google_workspace_account", { account: "work" });
    expect(invokeMock).toHaveBeenCalledWith("reconcile_mcp_runtime");
    expect(invokeMock).toHaveBeenCalledWith("restart_mcp_server_runtime", { name: "gworkspace" });
  });
});

describe("appStore session history hydration", () => {
  beforeEach(() => {
    invokeMock.mockClear();
    setSessionHistory([]);
  });

  it("rehydrates persisted tool turns into assistant toolCalls", async () => {
    setSessionHistory([
      {
        role: "assistant",
        content: "I retrieved your latest unread emails.",
        timestamp: "2026-04-18T10:00:00Z",
      },
      {
        role: "tool",
        content: "Tool 'gw_gmail_inbox' returned 3 Gmail message(s).",
        tool_name: "gw_gmail_inbox",
        tool_result: JSON.stringify({
          name: "gw_gmail_inbox",
          args: { query: "in:inbox is:unread", max_results: 3 },
          success: true,
          result: {
            provider: "google_workspace",
            kind: "gmail",
            data: {
              returned_count: 3,
              messages: [
                { subject: "Invoice", from: "billing@example.com" },
                { subject: "Security alert", from: "security@example.com" },
              ],
            },
          },
          metadata: {
            confidence: 0.8,
            source_count: 3,
            freshness_age_hours: null,
            region_match: null,
          },
        }),
        timestamp: "2026-04-18T10:00:01Z",
      },
    ]);

    await appStore.switchSession("session-1");

    const hydrated = appStore.messages();
    expect(hydrated).toHaveLength(1);
    expect(hydrated[0].role).toBe("assistant");
    expect(hydrated[0].content).toBe("I retrieved your latest unread emails.");
    expect(hydrated[0].toolCalls).toHaveLength(1);
    expect(hydrated[0].toolCalls?.[0]).toMatchObject({
      name: "gw_gmail_inbox",
      status: "done",
      args: { query: "in:inbox is:unread", max_results: 3 },
    });
    expect(hydrated[0].toolCalls?.[0].metadata?.sourceCount).toBe(3);
  });

  it("rehydrates assistant-role tool rows in the new persistence format", async () => {
    setSessionHistory([
      {
        role: "assistant",
        content: "Tool 'generate_image' generated 1 image. Saved to: /tmp/kria-cat.png",
        tool_name: "generate_image",
        tool_result: JSON.stringify({
          name: "generate_image",
          args: { prompt: "cat astronaut" },
          success: true,
          result: {
            images: [{ path: "/tmp/kria-cat.png" }],
          },
          metadata: {
            confidence: 0.78,
            source_count: null,
            freshness_age_hours: null,
            region_match: null,
          },
        }),
        timestamp: "2026-04-18T10:00:02Z",
      },
    ]);

    await appStore.switchSession("session-2");

    const hydrated = appStore.messages();
    expect(hydrated).toHaveLength(1);
    expect(hydrated[0].role).toBe("assistant");
    expect(hydrated[0].toolCalls).toHaveLength(1);
    expect(hydrated[0].toolCalls?.[0]).toMatchObject({
      name: "generate_image",
      status: "done",
      args: { prompt: "cat astronaut" },
    });
    expect((hydrated[0].toolCalls?.[0].result as any)?.images?.[0]?.path).toBe(
      "/tmp/kria-cat.png",
    );
  });
});

describe("appStore stream scope parity", () => {
  beforeEach(async () => {
    invokeMock.mockClear();
    setSessionHistory([]);

    appStore.setCurrentEnvironment("assistant");
    await flushAsync();
    await appStore.switchSession("assistant-reset");

    appStore.setCurrentEnvironment("prompt_lab");
    await flushAsync();
    await appStore.switchSession("prompt-lab-reset");

    appStore.setCurrentEnvironment("assistant");
    await flushAsync();
  });

  it("keeps agent and prompt_lab token streams isolated", async () => {
    emit("agent:token", { text: "assistant token" });
    expect(appStore.messages()).toHaveLength(1);
    expect(appStore.messages()[0].content).toBe("assistant token");

    appStore.setCurrentEnvironment("prompt_lab");
    await flushAsync();
    expect(appStore.messages()).toHaveLength(0);

    emit("prompt_lab:token", { text: "lab token" });
    expect(appStore.messages()).toHaveLength(1);
    expect(appStore.messages()[0].content).toBe("lab token");

    appStore.setCurrentEnvironment("assistant");
    await flushAsync();
    expect(appStore.messages()).toHaveLength(1);
    expect(appStore.messages()[0].content).toBe("assistant token");
  });

  it("tracks current session independently per environment", async () => {
    appStore.setCurrentEnvironment("assistant");
    await flushAsync();
    await appStore.switchSession("assistant-session-1");
    expect(appStore.currentSession()).toBe("assistant-session-1");

    appStore.setCurrentEnvironment("prompt_lab");
    await flushAsync();
    await appStore.switchSession("prompt-lab-session-1");
    expect(appStore.currentSession()).toBe("prompt-lab-session-1");

    appStore.setCurrentEnvironment("assistant");
    await flushAsync();
    expect(appStore.currentSession()).toBe("assistant-session-1");
  });
});

describe("appStore n8n chat result formatting", () => {
  beforeEach(async () => {
    invokeMock.mockClear();
    setSessionHistory([]);
    appStore.setCurrentEnvironment("assistant");
    appStore.setManualToolMode("auto");
    await flushAsync();
  });

  it("renders workflow evidence details instead of only the result line", () => {
    const before = appStore.messages().length;

    emit("n8n:chat_result", {
      success: true,
      display_name: "Inbox Digest",
      workflow_id: "gmail_inbox_digest",
      status: "completed",
      evidence: {
        result: "Found 2 unread Gmail message(s).",
        message_count: 2,
        messages: [
          {
            message_ref: "gmail-1",
            from: "team@example.com",
            subject: "Build passed",
            preview: "Build passed and deployment starts at 5 PM.",
          },
          {
            message_ref: "gmail-2",
            from: "ops@example.com",
            subject: "",
            preview: "Please review the latest deployment notes.",
          },
        ],
      },
    });

    const inserted = appStore.messages().slice(before);
    expect(inserted).toHaveLength(1);
    expect(inserted[0].content).toContain('Workflow "Inbox Digest" completed');
    expect(inserted[0].content).toContain("Found 2 unread Gmail message(s).");
    expect(inserted[0].content).toContain("Messages found: 2");
    expect(inserted[0].content).toContain("Build passed");
    expect(inserted[0].content).toContain("Ref: gmail-1");
  });
});

describe("appStore colab stage visibility", () => {
  beforeEach(() => {
    invokeMock.mockClear();
    setSessionHistory([]);
  });

  it("captures colab fallback stage details into warning state", () => {
    emit("agent:stage", {
      step: "colab_dispatch_fallback_local",
      message: "Colab tier requirements were not satisfied; using local fallback",
      detail: {
        reason: "missing capabilities: cell_execution",
        requested_mode: "colab",
        effective_mode: "local",
        runtime_state: "awaiting_browser_connection",
      },
      ts: "2026-04-20T00:00:00Z",
    });

    expect(appStore.latestAgentStage()?.step).toBe("colab_dispatch_fallback_local");
    expect(appStore.colabDispatchWarning()).toContain("colab -> local");
    expect(appStore.colabDispatchWarning()).toContain("missing capabilities: cell_execution");
  });

  it("clears colab warning when ready stage is emitted", () => {
    emit("agent:stage", {
      step: "colab_dispatch_fallback_local",
      message: "fallback",
      detail: {
        reason: "runtime_state=awaiting_browser_connection",
        requested_mode: "colab",
        effective_mode: "local",
      },
      ts: "2026-04-20T00:00:00Z",
    });

    expect(appStore.colabDispatchWarning()).not.toBeNull();

    emit("agent:stage", {
      step: "colab_dispatch_ready",
      message: "Colab tier requirements are satisfied",
      detail: {
        requested_mode: "colab",
        effective_mode: "colab",
      },
      ts: "2026-04-20T00:00:01Z",
    });

    expect(appStore.colabDispatchWarning()).toBeNull();
  });
});
