import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { DeviceTargetView } from "../../../hooks/useDeviceStatus";
import { FleetMatrix } from "./FleetMatrix";

function device(index: number): DeviceTargetView {
  return {
    targetId: `device-${index}`,
    displayName: `Device ${index}`,
    mode: "ssh",
    state: "ready",
    tainted: false,
    taintReason: null,
    healthScore: 0.9,
    latencyEwmaMs: 10,
    recentFailureRate: 0,
    dockerHealth: "pass",
    dockerPassCount: 1,
    dockerFailCount: 0,
    dockerLastRunAtUnixMs: null,
    updatedAtUnixMs: index,
  };
}

afterEach(() => cleanup());

describe("FleetMatrix virtualization (Req 16.2)", () => {
  it("mounts only visible semantic table rows for a large fleet", () => {
    render(() => <FleetMatrix fleet={Array.from({ length: 500 }, (_, i) => device(i))}
      streamState="online" onInspect={vi.fn()} />);
    const mounted = document.querySelectorAll('[data-virtual-list="fleet-matrix"] [data-target-id]').length;
    expect(mounted).toBeGreaterThan(0);
    expect(mounted).toBeLessThan(500);
    expect(document.querySelector("table")).not.toBeNull();
  });

  it("keeps honest empty fleet state", () => {
    render(() => <FleetMatrix fleet={[]} streamState="degraded" onInspect={vi.fn()} />);
    expect(document.body).toHaveTextContent("Live fleet status is offline");
  });
});
