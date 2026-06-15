import { beforeEach, describe, expect, it, vi } from "vitest";

// Task 10.5 (flag `gui_cog_stream_ux`) — T1 vitest coverage for the app store's
// `gui_cognition:event` listener (streaming), sequential-turn busy handling, and
// the Stop/cancel path. Mirrors the hoisted Tauri mock harness used by
// `app.tool-choice.test.ts` so the listener registry can be driven directly.

const { invokeMock, listenMock, listenerMap } = vi.hoisted(() => {
  const listenerMap = new Map<string, (event: { payload: any }) => void>();

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
      case "switch_session":
        return { session_id: args?.sessionId ?? "mock-session", messages: [] };
      case "get_session_history":
        return [];
      case "cancel_turn":
        return {};
      case "cancel_gui_cognition_turn":
        return { cancelled: true, session_id: args?.sessionId ?? null };
      case "get_settings":
        return { llm: {}, voice: {}, safety: {}, ui: { theme: "dark" }, server: {}, memory: {} };
      case "list_audio_devices":
        return { inputs: [], outputs: [], default_input: null, default_output: null };
      case "get_health":
        return { status: "healthy", services: [{ name: "model_router", status: "healthy" }] };
      default:
        return null;
    }
  });

  const listenMock = vi.fn(async (eventName: string, callback: (event: { payload: any }) => void) => {
    listenerMap.set(eventName, callback);
    return () => listenerMap.delete(eventName);
  });

  vi.stubGlobal("setInterval", vi.fn(() => 1));

  return { invokeMock, listenMock, listenerMap };
});

vi.mock("@tauri-apps/api/core", () => ({ invoke: invokeMock }));
vi.mock("@tauri-apps/api/event", () => ({ listen: listenMock }));
vi.mock("highlight.js/styles/github-dark.css?url", () => ({ default: "dark.css" }));
vi.mock("highlight.js/styles/github.css?url", () => ({ default: "light.css" }));

import { appStore } from "./app";
import {
  __resetGuiCognitionSessionForTests,
  activeGuiCognitionSession,
} from "./guiCognitionSession";

const TURN_BUSY_MESSAGE =
  "A request is already running. Wait for it to finish or press Stop before sending another prompt.";

function emit(eventName: string, payload: any) {
  const callback = listenerMap.get(eventName);
  if (!callback) throw new Error(`Missing listener for ${eventName}`);
  callback({ payload });
}

let sequence = 0;
function envelope(event: any, turnId = "turn-1", sessionId = "session-1") {
  sequence += 1;
  return {
    version: 1,
    session_id: sessionId,
    turn_id: turnId,
    workflow_id: "workflow-1",
    sequence,
    timestamp_ms: Date.now() + sequence,
    event,
  };
}

async function flushAsync(cycles = 4) {
  for (let i = 0; i < cycles; i += 1) {
    await Promise.resolve();
  }
}

async function activateAssistant(sessionId: string) {
  appStore.setCurrentEnvironment("assistant");
  await flushAsync();
  await appStore.switchSession(sessionId);
  await flushAsync();
}

describe("appStore gui_cognition streaming listener (Req 16.1)", () => {
  beforeEach(() => {
    invokeMock.mockClear();
    sequence = 0;
    __resetGuiCognitionSessionForTests();
    emit("agent:done", {});
    appStore.setManualToolMode("auto");
  });

  it("streams envelopes progressively into the session store during the turn", () => {
    appStore.setManualToolMode("gui_cognition");

    emit("gui_cognition:event", envelope({ type: "TurnStarted", mode_id: "gui_cognition" }));
    expect(activeGuiCognitionSession()?.lifecycle).toBe("planning");

    emit("gui_cognition:event", envelope({ type: "ObservationStarted" }));
    expect(activeGuiCognitionSession()?.lifecycle).toBe("observing");

    emit("gui_cognition:event", envelope({ type: "ObservationCompleted", active_window: "Editor", visible_control_count: 5 }));
    expect(activeGuiCognitionSession()?.lifecycle).toBe("planning");
    expect(activeGuiCognitionSession()?.observation.activeWindow).toBe("Editor");
    expect(activeGuiCognitionSession()?.observation.visibleControlCount).toBe(5);

    emit("gui_cognition:event", envelope({ type: "ActionStarted", action_kind: "ClickControl", target: "Search" }));
    expect(activeGuiCognitionSession()?.lifecycle).toBe("executing");

    // Each envelope mutated the store as it arrived — never one end batch.
    emit("gui_cognition:event", envelope({ type: "VerificationCompleted", status: "verified", confidence: 0.9 }));
    expect(activeGuiCognitionSession()?.verification?.status).toBe("verified");

    emit("gui_cognition:event", envelope({ type: "TurnCompleted", status: "ok" }));
    expect(activeGuiCognitionSession()?.lifecycle).toBe("completed");
  });

  it("clears the thinking indicator on a terminal streamed envelope even without agent:done", () => {
    emit("agent:thinking", { status: "processing", mode: "gui_cognition" });
    expect(appStore.isThinking()).toBe(true);

    emit("gui_cognition:event", envelope({ type: "TurnStarted", mode_id: "gui_cognition" }));
    emit("gui_cognition:event", envelope({ type: "ObservationStarted" }));
    expect(appStore.isThinking()).toBe(true); // still running mid-stream

    emit("gui_cognition:event", envelope({ type: "TurnCompleted", status: "ok" }));
    expect(activeGuiCognitionSession()?.lifecycle).toBe("completed");
    expect(appStore.isThinking()).toBe(false);
  });

  it("keeps the thinking indicator running while awaiting approval (non-terminal)", () => {
    emit("agent:thinking", { status: "processing", mode: "gui_cognition" });
    emit("gui_cognition:event", envelope({ type: "TurnStarted", mode_id: "gui_cognition" }));
    emit("gui_cognition:event", envelope({ type: "HitlRequired", reason: "Submit needs approval", risk_level: "high" }));

    expect(activeGuiCognitionSession()?.lifecycle).toBe("awaiting_approval");
    // Paused-for-approval is intentionally NOT cleared — the user must act.
    expect(appStore.isThinking()).toBe(true);

    emit("agent:done", {});
  });

  it("rejects a stale out-of-order streamed envelope without clobbering progress", () => {
    emit("gui_cognition:event", envelope({ type: "TurnStarted", mode_id: "gui_cognition" }));
    emit("gui_cognition:event", envelope({ type: "ObservationCompleted", active_window: "fresh" }));
    const fresh = activeGuiCognitionSession()?.lastSequence ?? 0;

    // Hand-craft a lower-sequence envelope for the same turn (stale).
    emit("gui_cognition:event", {
      version: 1,
      session_id: "session-1",
      turn_id: "turn-1",
      workflow_id: "workflow-1",
      sequence: 1,
      timestamp_ms: Date.now(),
      event: { type: "ObservationCompleted", active_window: "stale" },
    });

    expect(activeGuiCognitionSession()?.observation.activeWindow).toBe("fresh");
    expect(activeGuiCognitionSession()?.lastSequence).toBe(fresh);
  });
});

describe("appStore sequential-turn busy handling (Req 16.2/16.3)", () => {
  beforeEach(async () => {
    invokeMock.mockClear();
    sequence = 0;
    __resetGuiCognitionSessionForTests();
    emit("agent:done", {});
    appStore.setManualToolMode("auto");
    await activateAssistant("gui-busy-session");
  });

  it("notifies 'busy' and does not dispatch or interleave an overlapping prompt", async () => {
    appStore.setManualToolMode("gui_cognition");

    // First turn occupies the assistant scope.
    await appStore.sendMessage("Open the Calculator");
    await flushAsync(8);
    emit("agent:thinking", { status: "processing", mode: "gui_cognition" });
    expect(appStore.isThinking()).toBe(true);

    invokeMock.mockClear();

    // Second prompt arrives while the first turn is still active.
    await appStore.sendMessage("Open the Screenshot tool");
    await flushAsync(4);

    // It is NOT dispatched and NOT recorded as a user turn.
    expect(invokeMock).not.toHaveBeenCalledWith(
      "send_manual_tool_message",
      expect.objectContaining({ message: "Open the Screenshot tool" }),
    );
    expect(
      appStore.messages().some((m) => m.role === "user" && m.content === "Open the Screenshot tool"),
    ).toBe(false);

    // The user is explicitly told the assistant is busy.
    const busyMessages = appStore
      .messages()
      .filter((m) => m.role === "system" && m.content === TURN_BUSY_MESSAGE);
    expect(busyMessages.length).toBe(1);

    // A third overlapping attempt is de-duplicated (no transcript spam).
    await appStore.sendMessage("Open the Files app");
    await flushAsync(4);
    expect(
      appStore.messages().filter((m) => m.role === "system" && m.content === TURN_BUSY_MESSAGE).length,
    ).toBe(1);

    emit("agent:done", {});
  });
});

describe("appStore Stop/cancel for GUI Cognition (Req 16.6 / 21.1)", () => {
  beforeEach(async () => {
    invokeMock.mockClear();
    sequence = 0;
    __resetGuiCognitionSessionForTests();
    emit("agent:done", {});
    appStore.setManualToolMode("auto");
    await activateAssistant("gui-stop-session");
  });

  it("Stop cancels the active turn via cancel_gui_cognition_turn and clears thinking", async () => {
    emit("agent:thinking", { status: "processing", mode: "gui_cognition" });
    emit("gui_cognition:event", envelope({ type: "TurnStarted", mode_id: "gui_cognition" }));
    emit("gui_cognition:event", envelope({ type: "ObservationStarted" }));
    expect(appStore.isThinking()).toBe(true);
    expect(activeGuiCognitionSession()?.lifecycle).toBe("observing");

    invokeMock.mockClear();
    await appStore.cancelGuiCognitionTurn();
    await flushAsync(4);

    // Cancellation flows through the Task 1 cancel command (session-keyed token).
    expect(invokeMock).toHaveBeenCalledWith("cancel_gui_cognition_turn", {
      sessionId: "session-1",
      reason: "Turn cancelled by you.",
    });
    // UI returns to idle and the panel shows a clear cancelled state.
    expect(appStore.isThinking()).toBe(false);
    expect(activeGuiCognitionSession()?.lifecycle).toBe("cancelled");
    expect(activeGuiCognitionSession()?.blocker?.reason).toBe("Turn cancelled by you.");
  });

  it("cancelTurn also aborts an active GUI Cognition turn through the cancel path", async () => {
    emit("agent:thinking", { status: "processing", mode: "gui_cognition" });
    emit("gui_cognition:event", envelope({ type: "TurnStarted", mode_id: "gui_cognition" }));
    expect(appStore.isThinking()).toBe(true);

    invokeMock.mockClear();
    await appStore.cancelTurn();
    await flushAsync(4);

    // Both the chat/agent loop AND the GUI Cognition workflow loop are halted.
    expect(invokeMock).toHaveBeenCalledWith(
      "cancel_gui_cognition_turn",
      expect.objectContaining({ reason: "Turn cancelled by you." }),
    );
    expect(invokeMock).toHaveBeenCalledWith("cancel_turn", expect.objectContaining({}));
    expect(appStore.isThinking()).toBe(false);
    expect(activeGuiCognitionSession()?.lifecycle).toBe("cancelled");
  });

  it("Stop degrades gracefully to a cancelled panel state when no turn is streaming", async () => {
    // No active GUI Cognition session (streaming OFF / nothing running).
    expect(activeGuiCognitionSession()).toBeNull();

    await appStore.cancelGuiCognitionTurn();
    await flushAsync(4);

    // It never throws and leaves the assistant idle.
    expect(appStore.isThinking()).toBe(false);
    // With no active turn, marking cancelled is a no-op (no phantom session).
    expect(activeGuiCognitionSession()).toBeNull();
  });
});
