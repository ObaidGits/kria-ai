/**
 * VoiceSurface — the compact voice presence (task 5.1, Req 12.1).
 *
 * Asserts the surface: is hidden when voice is inactive; when active renders the
 * Core + ONE transcript line in a polite live region and reflects voiceStore
 * state; is compact (not full-screen) by default; and its Stop routes through
 * the existing voice-stop path (never a tool/orchestration call). The default
 * stop mocks the bridge so no Tauri runtime is required.
 */
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, fireEvent, cleanup } from "@solidjs/testing-library";
import voiceSurfaceCss from "./VoiceSurface.css?raw";

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
import { voiceStore, approvalStore } from "../../stores";
import { initOverlayInertness } from "../overlayLayers";
import type { ApprovalRequest } from "../../stores/approvalStore";

function makeRequest(overrides: Partial<ApprovalRequest> = {}): ApprovalRequest {
  return {
    id: "req-1",
    type: "tool-hitl",
    title: "Send the drafted email",
    description: "why",
    risk: "yellow",
    effects: ["Sends 1 email"],
    payload: {},
    createdAt: Date.now(),
    status: "pending",
    ...overrides,
  };
}

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

describe("VoiceSurface — no focus theft (task 8.7, §20.3 Focus_Return_Owner, Req 12.5)", () => {
  afterEach(() => {
    document.querySelectorAll("[data-test-external]").forEach((n) => n.remove());
  });

  it("does not move focus away from a pre-focused element when voice activates", () => {
    const input = document.createElement("input");
    input.setAttribute("data-test-external", "");
    document.body.appendChild(input);
    input.focus();
    expect(document.activeElement).toBe(input);

    voiceStore.activate();
    render(() => <VoiceSurface />);

    // Surface mounted (Portal → document), but focus stays put — no auto-seize.
    expect(document.querySelector(".kria-voice")).toBeInTheDocument();
    expect(document.activeElement).toBe(input);
  });
});

describe("VoiceSurface — Escape is internal + non-modal (task 8.7, Req 12.1/12.5)", () => {
  afterEach(() => {
    document.querySelectorAll("[data-test-external]").forEach((n) => n.remove());
  });

  it("dismisses (Stop) when Escape fires with focus INSIDE the surface, without preventDefault", () => {
    const onStop = vi.fn();
    voiceStore.activate();
    render(() => <VoiceSurface onStop={onStop} />);

    const stopBtn = screen.getByRole("button", { name: "Stop voice" });
    stopBtn.focus();

    const event = new KeyboardEvent("keydown", {
      key: "Escape",
      bubbles: true,
      cancelable: true,
    });
    stopBtn.dispatchEvent(event);

    // Surface dismiss fired…
    expect(onStop).toHaveBeenCalledTimes(1);
    // …but it did NOT preventDefault (one-layer; global Escape unaffected).
    expect(event.defaultPrevented).toBe(false);
  });

  it("does NOT dismiss when Escape fires with focus OUTSIDE the surface", () => {
    const onStop = vi.fn();
    const outside = document.createElement("input");
    outside.setAttribute("data-test-external", "");
    document.body.appendChild(outside);

    voiceStore.activate();
    render(() => <VoiceSurface onStop={onStop} />);

    outside.focus();
    outside.dispatchEvent(
      new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }),
    );

    expect(onStop).not.toHaveBeenCalled();
  });
});

describe("VoiceSurface — scoped Stop + barge-in always reachable across phases (task 8.7, Req 12.5)", () => {
  const PHASES = ["idle", "listening", "speaking", "error"] as const;
  for (const p of PHASES) {
    it(`keeps Stop and Interrupt present + enabled in phase "${p}"`, () => {
      voiceStore.activate();
      voiceStore.setState(p);
      render(() => <VoiceSurface />);

      const stopBtn = screen.getByRole("button", { name: "Stop voice" });
      const interruptBtn = screen.getByRole("button", { name: "Interrupt (barge-in)" });
      expect(stopBtn).toBeInTheDocument();
      expect(stopBtn).not.toBeDisabled();
      expect(interruptBtn).toBeInTheDocument();
      expect(interruptBtn).not.toBeDisabled();
    });
  }
});

describe("VoiceSurface — yields to a blocking approval (task 8.7, §20.3, Req 11.3/11.13)", () => {
  let disposeInertness: (() => void) | undefined;

  afterEach(() => {
    disposeInertness?.();
    disposeInertness = undefined;
    approvalStore.setQueue([]);
  });

  it("is inert + aria-hidden while an approval is pending (outranked, not covering)", () => {
    approvalStore.setQueue([]);
    voiceStore.activate();
    render(() => <VoiceSurface />);
    disposeInertness = initOverlayInertness();

    const surface = document.querySelector<HTMLElement>(".kria-voice")!;
    // No blocking layer yet → interactive.
    expect(surface.hasAttribute("inert")).toBe(false);

    // A blocking approval outranks the floating voice surface.
    approvalStore.setQueue([makeRequest()]);
    expect(surface.hasAttribute("inert")).toBe(true);
    expect(surface.getAttribute("aria-hidden")).toBe("true");

    // Clearing the queue restores interactivity (never a permanent block).
    approvalStore.setQueue([]);
    expect(surface.hasAttribute("inert")).toBe(false);
  });
});

describe("VoiceSurface.css — safe-area bounds + reserved bottom band (task 8.7, Req 11.3/11.4)", () => {
  const css = voiceSurfaceCss;

  it("uses env(safe-area-inset* so the pill never sits under OS chrome", () => {
    expect(css).toContain("env(safe-area-inset-bottom");
    expect(css).toContain("env(safe-area-inset-left");
    expect(css).toContain("env(safe-area-inset-right");
  });

  it("lifts off the raw bottom edge via a reserved bottom band (clears the Composer)", () => {
    expect(css).toContain("--kria-voice-reserved-bottom");
    // The bottom offset composes the reserved band + the safe-area inset — it is
    // NOT the raw `bottom: var(--space-5)` that collided with the Composer.
    expect(css).toMatch(/bottom:\s*calc\(var\(--kria-voice-reserved-bottom\)/);
    expect(css).not.toMatch(/bottom:\s*var\(--space-5\);/);
  });
});
