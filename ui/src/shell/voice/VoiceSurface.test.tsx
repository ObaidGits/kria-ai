/**
 * VoiceSurface — the compact voice presence (task 5.1, Req 12.1).
 *
 * Asserts the surface: is hidden when voice is inactive; when active renders the
 * Core + ONE transcript line in a polite live region and reflects voiceStore
 * state; is compact (not full-screen) by default; and its Stop routes through
 * the existing voice-stop path (never a tool/orchestration call). The default
 * stop mocks the bridge so no Tauri runtime is required.
 */
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, cleanup } from "@solidjs/testing-library";

// Mock the optional bridge invoke so the default Stop can be asserted without a
// Tauri runtime. Hoisted by Vitest before the component import below.
const bridgeInvokeOptional = vi.fn();
vi.mock("../../bridge/invoke", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../bridge/invoke")>();
  return {
    ...actual,
    bridgeInvokeOptional: (...args: unknown[]) => bridgeInvokeOptional(...args),
  };
});

import { VoiceSurface, voicePhaseToCoreState } from "./VoiceSurface";
import { voiceStore } from "../../stores";

function resetVoice(): void {
  voiceStore.deactivate(); // → inactive, idle, transcripts cleared
  voiceStore.setActive(false);
}

beforeEach(() => {
  cleanup();
  bridgeInvokeOptional.mockClear();
  resetVoice();
});

describe("VoiceSurface — visibility (Req 12.1)", () => {
  it("renders nothing while voice is inactive", () => {
    const { container } = render(() => <VoiceSurface />);
    expect(container.querySelector(".kria-voice")).toBeNull();
  });

  it("appears when voice becomes active", () => {
    render(() => <VoiceSurface />);
    voiceStore.activate();
    expect(screen.getByRole("region", { name: "Voice" })).toBeInTheDocument();
  });
});

describe("VoiceSurface — Core + one transcript line (Req 12.1)", () => {
  it("renders the Core presence and exactly one transcript line", () => {
    voiceStore.activate();
    voiceStore.setState("listening");
    // Rendered via a Portal → query the document, not the render container.
    render(() => <VoiceSurface />);

    // The Core is present (role=img, from CorePresence).
    expect(document.querySelector(".kria-core")).toBeInTheDocument();
    // Exactly ONE transcript line.
    expect(document.querySelectorAll(".kria-voice__transcript")).toHaveLength(1);
  });

  it("shows the transcript in a POLITE live region (Req 12.1 / 17.2)", () => {
    voiceStore.activate();
    voiceStore.setTranscript("hello kria", false);
    render(() => <VoiceSurface />);

    const line = document.querySelector(".kria-voice__transcript")!;
    expect(line.getAttribute("aria-live")).toBe("polite");
    expect(line).toHaveTextContent("hello kria");
  });

  it("prefers the in-flight partial over the last final transcript", () => {
    voiceStore.activate();
    voiceStore.setTranscript("final text", false);
    voiceStore.setTranscript("partial in progress", true);
    render(() => <VoiceSurface />);

    expect(document.querySelector(".kria-voice__transcript")).toHaveTextContent(
      "partial in progress",
    );
  });
});

describe("VoiceSurface — reflects voiceStore state via the Core (Req 12.1)", () => {
  it("maps each voice phase to the Core state it presents", () => {
    expect(voicePhaseToCoreState("idle")).toBe("idle");
    expect(voicePhaseToCoreState("wake_listening")).toBe("listening");
    expect(voicePhaseToCoreState("listening")).toBe("listening");
    expect(voicePhaseToCoreState("interrupt")).toBe("listening");
    expect(voicePhaseToCoreState("transcribing")).toBe("thinking");
    expect(voicePhaseToCoreState("thinking")).toBe("thinking");
    expect(voicePhaseToCoreState("speaking")).toBe("speaking");
    expect(voicePhaseToCoreState("error")).toBe("error");
  });

  it("reflects the live phase on the Core and the phase label", () => {
    voiceStore.activate();
    voiceStore.setState("speaking");
    render(() => <VoiceSurface />);

    expect(document.querySelector(".kria-core")!.getAttribute("data-core-state")).toBe("speaking");
    expect(screen.getByText("Speaking")).toBeInTheDocument();

    // A phase change is reflected reactively.
    voiceStore.setState("listening");
    expect(document.querySelector(".kria-core")!.getAttribute("data-core-state")).toBe("listening");
    expect(screen.getByText("Listening")).toBeInTheDocument();
  });
});

describe("VoiceSurface — compact by default (Req 12.1)", () => {
  it("is compact, not full-screen", () => {
    voiceStore.activate();
    render(() => <VoiceSurface />);
    expect(document.querySelector(".kria-voice")!.getAttribute("data-variant")).toBe("compact");
  });
});

describe("VoiceSurface — Stop routes through the voice-stop path (Req 12.1)", () => {
  it("default Stop calls the existing stop_voice command and deactivates", () => {
    voiceStore.activate();
    render(() => <VoiceSurface />);

    fireEvent.click(screen.getByRole("button", { name: "Stop voice" }));

    // Routes through the EXISTING optional command — no tool call/orchestration.
    expect(bridgeInvokeOptional).toHaveBeenCalledWith("stop_voice");
    // Surface stands down.
    expect(voiceStore.active()).toBe(false);
  });

  it("uses an injected Stop handler when provided (no default fired)", () => {
    const onStop = vi.fn();
    voiceStore.activate();
    render(() => <VoiceSurface onStop={onStop} />);

    fireEvent.click(screen.getByRole("button", { name: "Stop voice" }));
    expect(onStop).toHaveBeenCalledTimes(1);
    expect(bridgeInvokeOptional).not.toHaveBeenCalled();
  });
});

describe("VoiceSurface — barge-in is always honored (Req 12.5)", () => {
  it("always exposes an interrupt control, even while KRIA is speaking", () => {
    voiceStore.activate();
    voiceStore.setState("speaking");
    render(() => <VoiceSurface />);
    // Present and enabled regardless of phase → never blocked.
    const btn = screen.getByRole("button", { name: "Interrupt (barge-in)" });
    expect(btn).toBeInTheDocument();
    expect(btn).not.toBeDisabled();
  });

  it("default interrupt routes through the existing voice_v2_abort path and reflects interrupt", () => {
    voiceStore.activate();
    voiceStore.setState("speaking");
    render(() => <VoiceSurface />);

    fireEvent.click(screen.getByRole("button", { name: "Interrupt (barge-in)" }));

    // Routes through the EXISTING optional abort command (the stop-phrase path).
    expect(bridgeInvokeOptional).toHaveBeenCalledWith("voice_v2_abort");
    // The interrupt state is reflected (never blocked).
    expect(voiceStore.state()).toBe("interrupt");
  });

  it("uses an injected interrupt handler when provided", () => {
    const onInterrupt = vi.fn();
    voiceStore.activate();
    render(() => <VoiceSurface onInterrupt={onInterrupt} />);

    fireEvent.click(screen.getByRole("button", { name: "Interrupt (barge-in)" }));
    expect(onInterrupt).toHaveBeenCalledTimes(1);
    expect(bridgeInvokeOptional).not.toHaveBeenCalled();
  });
});
