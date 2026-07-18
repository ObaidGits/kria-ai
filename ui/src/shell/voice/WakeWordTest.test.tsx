/**
 * WakeWordTest — a REAL, functional wake-word test (task 5.3, Req 12.4).
 *
 * These tests prove the test stays REAL and never fakes a pass:
 *   • it starts REAL listening via the existing `start_voice` command;
 *   • a genuine `voice:wake-detected` bus event → pass feedback + score;
 *   • timeout / no-detection → an HONEST failure (never a pass);
 *   • missing models / no-mic / unavailable voice → an HONEST error (not a pass).
 *
 * The bridge is mocked so no Tauri runtime is needed; detections are driven
 * through the typed event bus exactly as the real bridge would map them.
 */
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, fireEvent, cleanup, waitFor } from "@solidjs/testing-library";
import type { ServiceResult } from "../../bridge/types";

// Controllable bridge mocks (hoisted before the component import).
const bridgeInvoke = vi.fn();
const bridgeInvokeOptional = vi.fn();
vi.mock("../../bridge/invoke", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../bridge/invoke")>();
  return {
    ...actual,
    bridgeInvoke: (...args: unknown[]) => bridgeInvoke(...args),
    bridgeInvokeOptional: (...args: unknown[]) => bridgeInvokeOptional(...args),
  };
});

import { WakeWordTest, wakeTestStatusMeta } from "./WakeWordTest";
import { eventBus } from "../../stores";
import { voiceStore } from "../../stores";

// ─── Helpers ────────────────────────────────────────────────────────────────

function ok<T>(data: T): ServiceResult<T> {
  return { ok: true, data };
}
function unavailable<T>(command: string): ServiceResult<T> {
  return { ok: false, code: "unavailable", message: "not available", command };
}

/** wake_word readiness where the models are present + feature compiled. */
const READY = {
  wake_word: {
    enabled_in_config: true,
    feature_compiled: true,
    all_models_present: true,
    keyword_path: "/models/wake/hey_ria.onnx",
  },
};

/** Route bridgeInvoke by command name for a test. */
function routeInvoke(map: Record<string, () => ServiceResult<unknown>>): void {
  bridgeInvoke.mockImplementation((command: string) => {
    const handler = map[command];
    return Promise.resolve(handler ? handler() : unavailable(command));
  });
}

beforeEach(() => {
  cleanup();
  bridgeInvoke.mockReset();
  bridgeInvokeOptional.mockReset();
  bridgeInvokeOptional.mockResolvedValue(null);
  voiceStore.deactivate();
});

afterEach(() => {
  eventBus.clear();
});

// ─── Pure status mapping ──────────────────────────────────────────────────────

describe("wakeTestStatusMeta — status is never conveyed by color alone (Req 17.3)", () => {
  it("gives every status a distinct label + icon", () => {
    const labels = new Set<string>();
    const icons: Record<string, string> = {};
    for (const s of ["idle", "checking", "listening", "detected", "failed", "unavailable"] as const) {
      const meta = wakeTestStatusMeta(s);
      expect(meta.label.length).toBeGreaterThan(0);
      expect(meta.icon.length).toBeGreaterThan(0);
      labels.add(meta.label);
      icons[s] = meta.icon;
    }
    // Distinct phrasing per state (state is never color-only, Req 17.3).
    expect(labels.size).toBe(6);
    expect(icons.detected).toBe("check-circle");
    expect(icons.failed).toBe("alert-circle");
    expect(icons.unavailable).toBe("mic-off");
  });
});

// ─── Real listening + detection = pass ────────────────────────────────────────

describe("WakeWordTest — a genuine detection is the only way to pass (Req 12.4)", () => {
  it("starts REAL listening then passes on a real voice:wake-detected event", async () => {
    routeInvoke({ voice_v2_status: () => ok(READY), start_voice: () => ok(null) });
    const onDetected = vi.fn();

    render(() => <WakeWordTest onDetected={onDetected} />);
    fireEvent.click(screen.getByRole("button", { name: "Start wake word test" }));

    // REAL listening was started via the existing command (not faked).
    await waitFor(() => expect(bridgeInvoke).toHaveBeenCalledWith("start_voice"));
    await screen.findByText("Listening — say the wake word");

    // A genuine detection arrives on the bus (as the bridge would map it).
    eventBus.emit("voice:wake-detected", { score: 0.92, source: "pipeline" });

    await screen.findByText("Wake word detected");
    expect(onDetected).toHaveBeenCalledWith(0.92);
    // Listening was torn down (stop routed through the existing command).
    expect(bridgeInvokeOptional).toHaveBeenCalledWith("stop_voice");
  });
});

// ─── Honest timeout ───────────────────────────────────────────────────────────

describe("WakeWordTest — no detection fails honestly (never a fake pass) (Req 12.4)", () => {
  it("reports an honest failure when nothing is heard within the window", async () => {
    routeInvoke({ voice_v2_status: () => ok(READY), start_voice: () => ok(null) });

    render(() => <WakeWordTest timeoutMs={20} />);
    fireEvent.click(screen.getByRole("button", { name: "Start wake word test" }));

    await screen.findByText("No wake word detected");
    // A failure is NOT a pass.
    expect(screen.queryByText("Wake word detected")).toBeNull();
  });
});

// ─── Honest unavailability (not a fake pass) ──────────────────────────────────

describe("WakeWordTest — unavailability is honest, never a fake pass (Req 12.4)", () => {
  it("reports unavailable when the wake-word models are missing and never listens", async () => {
    routeInvoke({
      voice_v2_status: () =>
        ok({ wake_word: { feature_compiled: true, all_models_present: false, keyword_path: "/m/hey_ria.onnx" } }),
    });

    render(() => <WakeWordTest />);
    fireEvent.click(screen.getByRole("button", { name: "Start wake word test" }));

    await screen.findByText("Wake word unavailable");
    // It must NOT have started listening or claimed a pass.
    expect(bridgeInvoke).not.toHaveBeenCalledWith("start_voice");
    expect(screen.queryByText("Wake word detected")).toBeNull();
  });

  it("reports honest error (not a pass) when start_voice is unavailable (no mic)", async () => {
    routeInvoke({
      voice_v2_status: () => ok(READY),
      start_voice: () => unavailable("start_voice"),
    });

    render(() => <WakeWordTest />);
    fireEvent.click(screen.getByRole("button", { name: "Start wake word test" }));

    await screen.findByText("Wake word unavailable");
    expect(screen.queryByText("Wake word detected")).toBeNull();
  });

  it("reports unavailable when voice diagnostics themselves are unavailable", async () => {
    routeInvoke({ voice_v2_status: () => unavailable("voice_v2_status") });

    render(() => <WakeWordTest />);
    fireEvent.click(screen.getByRole("button", { name: "Start wake word test" }));

    await screen.findByText("Wake word unavailable");
    expect(bridgeInvoke).not.toHaveBeenCalledWith("start_voice");
  });
});

// ─── Accessibility ────────────────────────────────────────────────────────────

describe("WakeWordTest — accessible (Req 17.1/17.2)", () => {
  it("exposes a labelled group and a polite live status region", () => {
    routeInvoke({ voice_v2_status: () => ok(READY), start_voice: () => ok(null) });
    render(() => <WakeWordTest />);

    expect(screen.getByRole("group", { name: "Wake word test" })).toBeInTheDocument();
    const status = document.querySelector(".kria-wake-test__status")!;
    expect(status.getAttribute("aria-live")).toBe("polite");
  });
});
