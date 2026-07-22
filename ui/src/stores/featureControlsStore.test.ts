import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const bridgeInvoke = vi.hoisted(() => vi.fn());
vi.mock("../bridge/invoke", () => ({ bridgeInvoke }));

import { featureControlsStore, type FeatureControl } from "./featureControlsStore";

const disabled: FeatureControl = {
  id: "voice",
  label: "Voice",
  description: "Voice runtime",
  desiredEnabled: false,
  state: "disabled",
};

beforeEach(() => {
  vi.useFakeTimers();
  bridgeInvoke.mockReset();
  featureControlsStore.dispose();
  featureControlsStore.setControls([]);
});

afterEach(() => {
  featureControlsStore.dispose();
  vi.restoreAllMocks();
  vi.useRealTimers();
});

describe("featureControlsStore", () => {
  it.each([
    {
      name: "missing",
      payload: undefined,
      expectedStatus: "unavailable",
      expectedControls: [],
      expectedDiagnostic: "missing-collection",
      expectedRejected: 0,
    },
    {
      name: "null",
      payload: null,
      expectedStatus: "unavailable",
      expectedControls: [],
      expectedDiagnostic: "null-collection",
      expectedRejected: 0,
    },
    {
      name: "empty",
      payload: [],
      expectedStatus: "empty",
      expectedControls: [],
      expectedDiagnostic: undefined,
      expectedRejected: 0,
    },
    {
      name: "partial",
      payload: [disabled, { ...disabled, state: "unknown" }],
      expectedStatus: "partial",
      expectedControls: [disabled],
      expectedDiagnostic: "invalid-entry",
      expectedRejected: 1,
    },
  ])("normalizes a $name successful payload at the existing store boundary", async ({
    payload,
    expectedStatus,
    expectedControls,
    expectedDiagnostic,
    expectedRejected,
  }) => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => undefined);
    bridgeInvoke.mockResolvedValue({ ok: true, data: payload });

    await featureControlsStore.initialize();

    expect(featureControlsStore.status()).toBe(expectedStatus);
    expect(featureControlsStore.controls()).toEqual(expectedControls);
    expect(featureControlsStore.rejectedCount()).toBe(expectedRejected);
    expect(featureControlsStore.diagnostics()[0]?.code).toBe(expectedDiagnostic);
    expect(warn).toHaveBeenCalledTimes(expectedDiagnostic === undefined ? 0 : 1);
  });

  it("uses typed commands and authoritatively refreshes after a mutation", async () => {
    bridgeInvoke.mockImplementation(async (command: string) => {
      if (command === "list_feature_controls") {
        const running = bridgeInvoke.mock.calls.some(([name]) => name === "set_feature_enabled");
        return { ok: true, data: [{ ...disabled, desiredEnabled: running, state: running ? "running" : "disabled" }] };
      }
      if (command === "set_feature_enabled") {
        return { ok: true, data: { ...disabled, desiredEnabled: true, state: "starting" } };
      }
      throw new Error(`Unexpected command: ${command}`);
    });

    await featureControlsStore.initialize();
    await expect(featureControlsStore.setEnabled("voice", true)).resolves.toBe(true);

    expect(bridgeInvoke).toHaveBeenCalledWith(
      "set_feature_enabled",
      { featureId: "voice", enabled: true },
      { timeoutMs: 35_000 },
    );
    expect(bridgeInvoke.mock.calls.map(([command]) => command)).toEqual([
      "list_feature_controls", "set_feature_enabled", "list_feature_controls",
    ]);
    expect(featureControlsStore.controls()[0]).toMatchObject({ desiredEnabled: true, state: "running" });
  });
  it("polls only while state is transitional", async () => {
    let requests = 0;
    bridgeInvoke.mockImplementation(async () => {
      requests += 1;
      return {
        ok: true,
        data: [{ ...disabled, desiredEnabled: true, state: requests === 1 ? "starting" : "running" }],
      };
    });

    await featureControlsStore.initialize();
    expect(requests).toBe(1);
    await vi.advanceTimersByTimeAsync(1_000);
    expect(requests).toBe(2);
    expect(featureControlsStore.controls()[0].state).toBe("running");

    await vi.advanceTimersByTimeAsync(10_000);
    expect(requests).toBe(2);
  });

  it("bounds polling when a transition never settles", async () => {
    bridgeInvoke.mockResolvedValue({
      ok: true,
      data: [{ ...disabled, desiredEnabled: true, state: "starting" }],
    });

    await featureControlsStore.initialize();
    await vi.runAllTimersAsync();

    expect(bridgeInvoke).toHaveBeenCalledTimes(16);
    expect(vi.getTimerCount()).toBe(0);
  });
});
