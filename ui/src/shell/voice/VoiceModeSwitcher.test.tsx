/**
 * VoiceModeSwitcher — in-surface mode + engine switching (task 5.2,
 * Req 12.2 / 12.3).
 *
 * Asserts: the switcher shows the current mode as a chip; opening it lists ALL
 * nine voice modes as labelled, keyboard-operable options with the active one
 * announced (aria-pressed); selecting a mode calls voiceStore.setMode (which
 * routes the backend command via the mocked bridge) and updates the current
 * chip; STT/TTS engine pickers are present; and an unavailable backend command
 * degrades silently. The bridge is mocked so no Tauri runtime is required.
 */
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, cleanup, within } from "@solidjs/testing-library";

const bridgeInvokeOptional = vi.fn().mockResolvedValue(null);
vi.mock("../../bridge/invoke", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../bridge/invoke")>();
  return {
    ...actual,
    bridgeInvokeOptional: (...args: unknown[]) => bridgeInvokeOptional(...args),
  };
});

import { VoiceModeSwitcher } from "./VoiceModeSwitcher";
import { voiceStore, VOICE_MODES } from "../../stores";

function openSwitcher(): void {
  fireEvent.click(screen.getByRole("button", { name: "Change voice mode and engine" }));
}

beforeEach(() => {
  cleanup();
  // Kobalte overlays (Popover/Select) portal onto document.body; ensure a clean
  // slate so reopened overlays in later tests remain uniquely queryable.
  document.body.innerHTML = "";
  bridgeInvokeOptional.mockClear();
  try {
    globalThis.localStorage?.clear();
  } catch {
    /* ignore */
  }
  voiceStore.setMode("conversation");
  voiceStore.setHealth({ sttHealthy: null, ttsHealthy: null, sttEngine: "", ttsEngine: "" });
  bridgeInvokeOptional.mockClear();
});

describe("VoiceModeSwitcher — current mode chip (Req 12.2/12.3)", () => {
  it("shows the current mode label as a compact chip", () => {
    render(() => <VoiceModeSwitcher />);
    const switcher = screen.getByTestId("voice-mode-switcher");
    expect(within(switcher).getByText("Conversation")).toBeInTheDocument();
  });

  it("exposes a labelled trigger to reach mode/engine switching from the surface", () => {
    render(() => <VoiceModeSwitcher />);
    expect(
      screen.getByRole("button", { name: "Change voice mode and engine" }),
    ).toBeInTheDocument();
  });
});

describe("VoiceModeSwitcher — lists all nine modes (Req 12.2)", () => {
  it("renders every voice mode as a keyboard-operable option once opened", () => {
    render(() => <VoiceModeSwitcher />);
    openSwitcher();
    for (const m of VOICE_MODES) {
      const option = screen.getByRole("button", { name: m.label });
      expect(option).toBeInTheDocument();
      // Toggle button semantics → keyboard-operable + current announced.
      expect(option).toHaveAttribute("aria-pressed");
    }
  });

  it("announces the active mode via aria-pressed", () => {
    render(() => <VoiceModeSwitcher />);
    openSwitcher();
    expect(screen.getByRole("button", { name: "Conversation" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    expect(screen.getByRole("button", { name: "Wake word" })).toHaveAttribute(
      "aria-pressed",
      "false",
    );
  });
});

describe("VoiceModeSwitcher — selecting a mode (Req 12.2/12.3)", () => {
  it("routes the backend command via the bridge and updates the current chip", () => {
    render(() => <VoiceModeSwitcher />);
    openSwitcher();

    fireEvent.click(screen.getByRole("button", { name: "Wake word" }));

    // Store updated.
    expect(voiceStore.mode()).toBe("wake-word");
    // Routed through the EXISTING patch_config voice command (mapped listening mode).
    expect(bridgeInvokeOptional).toHaveBeenCalledWith("patch_config", {
      section: "voice",
      field: "mode",
      value: "wake_word",
    });
    // Current chip reflects the new mode.
    const switcher = screen.getByTestId("voice-mode-switcher");
    expect(within(switcher).getByText("Wake word")).toBeInTheDocument();
  });

  it("degrades silently when the backend command is unavailable", () => {
    bridgeInvokeOptional.mockResolvedValueOnce(null);
    render(() => <VoiceModeSwitcher />);
    openSwitcher();

    expect(() =>
      fireEvent.click(screen.getByRole("button", { name: "Coding" })),
    ).not.toThrow();
    expect(voiceStore.mode()).toBe("coding");
  });
});

describe("VoiceModeSwitcher — engine switching (Req 12.3)", () => {
  it("renders STT and TTS engine pickers", () => {
    render(() => <VoiceModeSwitcher />);
    openSwitcher();
    expect(screen.getByText("Speech-to-text")).toBeInTheDocument();
    expect(screen.getByText("Text-to-speech")).toBeInTheDocument();
  });
});
