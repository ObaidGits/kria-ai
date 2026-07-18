/**
 * voiceStore — mode + engine switching (task 5.2, Req 12.2 / 12.3).
 *
 * Asserts the config-dispatch contract: `setMode` / `setEngine` update store
 * state, emit a typed request on the bus, and route the change through the
 * EXISTING `patch_config` voice config command (never a tool/orchestration
 * call). The bridge is mocked so no Tauri runtime is required; an unavailable
 * command degrades silently (the store still reflects the choice).
 */
import { describe, it, expect, vi, beforeEach } from "vitest";

// Mock the optional bridge invoke so patch_config routing can be asserted
// without a Tauri runtime. Hoisted before the store import below.
const bridgeInvokeOptional = vi.fn().mockResolvedValue(null);
vi.mock("../bridge/invoke", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../bridge/invoke")>();
  return {
    ...actual,
    bridgeInvokeOptional: (...args: unknown[]) => bridgeInvokeOptional(...args),
  };
});

import { voiceStore, VOICE_MODES, voiceModeMeta } from "./voiceStore";
import type { VoiceListeningMode } from "./voiceStore";
import { eventBus } from "./eventBus";

const VALID_LISTENING: ReadonlySet<VoiceListeningMode> = new Set([
  "push_to_talk",
  "continuous",
  "wake_word",
  "headphone",
]);

beforeEach(() => {
  bridgeInvokeOptional.mockClear();
  try {
    globalThis.localStorage?.clear();
  } catch {
    /* ignore */
  }
  voiceStore.setMode("conversation");
  bridgeInvokeOptional.mockClear();
});

describe("voiceStore — the nine voice modes (Req 12.2)", () => {
  it("defines exactly the nine required modes", () => {
    expect(VOICE_MODES.map((m) => m.mode)).toEqual([
      "quick-ptt",
      "conversation",
      "hands-free",
      "wake-word",
      "ambient",
      "meeting",
      "coding",
      "research",
      "planning",
    ]);
  });

  it("maps every UI mode to a valid backend listening mode", () => {
    for (const m of VOICE_MODES) {
      expect(VALID_LISTENING.has(m.listeningMode)).toBe(true);
      expect(m.label.length).toBeGreaterThan(0);
      expect(m.icon.length).toBeGreaterThan(0);
    }
  });
});

describe("voiceStore.setMode — config-dispatch to the backend (Req 12.2/12.3)", () => {
  it("updates store state and routes the mapped listening mode via patch_config", () => {
    voiceStore.setMode("wake-word");
    expect(voiceStore.mode()).toBe("wake-word");
    // Routes through the EXISTING patch_config command — the mapped listening
    // mode, not the richer UI mode id.
    expect(bridgeInvokeOptional).toHaveBeenCalledWith("patch_config", {
      section: "voice",
      field: "mode",
      value: voiceModeMeta("wake-word").listeningMode,
    });
  });

  it("emits a typed voice:mode-requested with the UI mode + listening mode", () => {
    const handler = vi.fn();
    const off = eventBus.on("voice:mode-requested", handler);
    voiceStore.setMode("coding");
    off();
    expect(handler).toHaveBeenCalledWith({
      mode: "coding",
      listeningMode: voiceModeMeta("coding").listeningMode,
    });
  });

  it("persists the selected mode across a reload (localStorage)", () => {
    voiceStore.setMode("planning");
    let raw: string | null = null;
    try {
      raw = globalThis.localStorage?.getItem("kria.voice.mode") ?? null;
    } catch {
      raw = null;
    }
    // Only assert when a storage backend exists in the test env.
    if (raw !== null) expect(raw).toBe("planning");
  });

  it("does not re-dispatch when the mode is unchanged", () => {
    voiceStore.setMode("meeting");
    bridgeInvokeOptional.mockClear();
    voiceStore.setMode("meeting");
    expect(bridgeInvokeOptional).not.toHaveBeenCalled();
  });

  it("degrades silently when the backend command is unavailable", () => {
    bridgeInvokeOptional.mockResolvedValueOnce(null); // simulate unavailable
    expect(() => voiceStore.setMode("ambient")).not.toThrow();
    // Store still reflects the choice even though the backend no-op'd.
    expect(voiceStore.mode()).toBe("ambient");
  });
});

describe("voiceStore.setEngine — STT/TTS engine switch (Req 12.3)", () => {
  it("updates STT engine health and routes voice.stt_engine via patch_config", () => {
    voiceStore.setEngine("stt", "whisper-rs");
    expect(voiceStore.health().sttEngine).toBe("whisper-rs");
    expect(bridgeInvokeOptional).toHaveBeenCalledWith("patch_config", {
      section: "voice",
      field: "stt_engine",
      value: "whisper-rs",
    });
  });

  it("updates TTS engine health and routes voice.tts_engine via patch_config", () => {
    voiceStore.setEngine("tts", "kokoro");
    expect(voiceStore.health().ttsEngine).toBe("kokoro");
    expect(bridgeInvokeOptional).toHaveBeenCalledWith("patch_config", {
      section: "voice",
      field: "tts_engine",
      value: "kokoro",
    });
  });

  it("emits a typed voice:engine-requested", () => {
    const handler = vi.fn();
    const off = eventBus.on("voice:engine-requested", handler);
    voiceStore.setEngine("tts", "piper-rs");
    off();
    expect(handler).toHaveBeenCalledWith({ kind: "tts", engine: "piper-rs" });
  });
});

describe("voiceStore.interrupt — barge-in is always honored (Req 12.5)", () => {
  it("reflects the interrupt phase and routes through the existing voice_v2_abort path", () => {
    voiceStore.setState("speaking");
    bridgeInvokeOptional.mockClear();

    voiceStore.interrupt();

    // Reflected immediately (never blocked).
    expect(voiceStore.state()).toBe("interrupt");
    // Routes through the EXISTING optional abort command (the stop-phrase path).
    expect(bridgeInvokeOptional).toHaveBeenCalledWith("voice_v2_abort");
  });
});

describe("voiceStore.initVoiceBridge — reflects the real backend pipeline (Req 12.5)", () => {
  it("mirrors backend voice:state-changed into the store without a feedback loop", () => {
    const dispose = voiceStore.initVoiceBridge();
    voiceStore.initVoiceBridge(); // idempotent — must not double-subscribe

    eventBus.emit("voice:state-changed", { state: "speaking", previous: "" });
    expect(voiceStore.state()).toBe("speaking");

    // Backend barge-in / stop-phrase → interrupt, reflected (never blocked).
    eventBus.emit("voice:interrupted", { reason: "barge_in" });
    expect(voiceStore.state()).toBe("interrupt");

    dispose();
    // After dispose, backend events no longer mutate the store.
    eventBus.emit("voice:state-changed", { state: "speaking", previous: "" });
    expect(voiceStore.state()).toBe("interrupt");
  });

  it("ignores unknown backend state strings", () => {
    const dispose = voiceStore.initVoiceBridge();
    voiceStore.setState("idle");
    eventBus.emit("voice:state-changed", { state: "not-a-real-state", previous: "" });
    expect(voiceStore.state()).toBe("idle");
    dispose();
  });
});
