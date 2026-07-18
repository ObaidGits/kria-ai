import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@solidjs/testing-library";

vi.mock("../bridge/invoke", () => ({
  bridgeInvoke: vi.fn(async () => ({ ok: true, data: {} })),
  bridgeInvokeOptional: vi.fn(async () => null),
}));

import { bridgeInvoke, bridgeInvokeOptional } from "../bridge/invoke";
import { converseStore, coreStore, observatoryStore } from "../stores";
import {
  KriaMiniCompanion,
  MINI_INTENT_MAX_LENGTH,
  normalizeMiniIntent,
  NowMiniCompanion,
  NOW_MINI_JOB_CAP,
} from "./MiniCompanions";

const invoke = bridgeInvoke as unknown as ReturnType<typeof vi.fn>;
const invokeOptional = bridgeInvokeOptional as unknown as ReturnType<typeof vi.fn>;

beforeEach(() => {
  invoke.mockClear();
  invokeOptional.mockClear();
  invoke.mockResolvedValue({ ok: true, data: {} });
  converseStore.clearMessages();
  converseStore.setActiveThread(null);
  converseStore.updateDraft({ text: "", attachments: [], mode: "assistant" });
  observatoryStore.setJobs([]);
  observatoryStore.setResourceMetrics({});
  observatoryStore.setTelemetryAuthority("awaiting-data");
  coreStore.reset();
});

/** Validates: Requirements 15.7 */
describe("KRIA Mini", () => {
  it("normalizes every input to a deterministic bounded intent", () => {
    const samples = ["", "  hello  ", "x".repeat(MINI_INTENT_MAX_LENGTH + 50)];
    for (const sample of samples) {
      const normalized = normalizeMiniIntent(sample);
      expect(normalized).toBe(normalized.trim());
      expect(normalized.length).toBeLessThanOrEqual(MINI_INTENT_MAX_LENGTH);
    }
  });

  it("submits through Converse runtime authority and preserves draft", async () => {
    converseStore.updateDraft({ text: "main-window draft", mode: "lab" });
    const { container } = render(() => <KriaMiniCompanion />);

    fireEvent.input(screen.getByRole("textbox", { name: "Intent for KRIA" }), {
      target: { value: "check current state" },
    });
    fireEvent.submit(container.querySelector("form")!);

    await vi.waitFor(() => expect(invoke).toHaveBeenCalledWith(
      "send_message", { message: "check current state" },
    ));
    expect(converseStore.composerDraft()).toMatchObject({ text: "main-window draft", mode: "lab" });
  });

  it("routes Stop through existing cancellation", async () => {
    coreStore.setState("acting");
    render(() => <KriaMiniCompanion />);
    fireEvent.click(screen.getByRole("button", { name: "Stop" }));
    await vi.waitFor(() => expect(invokeOptional).toHaveBeenCalledWith(
      "cancel_turn", { sessionId: "" },
    ));
  });
});

/** Validates: Requirements 15.7 */
describe("Now mini", () => {
  it("uses shared HRA telemetry for metrics and releases its consumer", () => {
    observatoryStore.applyHraDiagnostics({ telemetry: {
      source: "unified_hub", cpu_avg_pct: 25,
      ram_total_mb: 100, ram_free_mb: 25,
    } });
    const disconnect = vi.fn();
    const connect = vi.spyOn(observatoryStore, "connectTelemetry").mockReturnValue(disconnect);
    const view = render(() => <NowMiniCompanion />);

    expect(connect).toHaveBeenCalledTimes(1);
    expect(screen.getByText("25%")).toBeInTheDocument();
    expect(screen.getByText("75%")).toBeInTheDocument();
    view.unmount();
    expect(disconnect).toHaveBeenCalledTimes(1);
    connect.mockRestore();
  });

  it("caps active job presentation while keeping cancellation in JobRow", () => {
    observatoryStore.setJobs(Array.from({ length: NOW_MINI_JOB_CAP + 2 }, (_, index) => ({
      id: `job-${index}`,
      name: `Job ${index}`,
      status: "running" as const,
      progress: index * 10,
      startedAt: index,
      cancelKind: "capability" as const,
    })));

    render(() => <NowMiniCompanion />);

    expect(screen.getAllByRole("listitem")).toHaveLength(NOW_MINI_JOB_CAP);
    expect(screen.getByText("+2 more in Observatory")).toBeInTheDocument();
    expect(screen.getAllByRole("button", { name: "Cancel" })).toHaveLength(NOW_MINI_JOB_CAP);
  });
});