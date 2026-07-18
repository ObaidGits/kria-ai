import { describe, it, expect, afterEach } from "vitest";
import { render, screen, cleanup } from "@solidjs/testing-library";
import { DeviceInspector } from "./DeviceInspector";
import type { InspectorTarget } from "../../../stores/shellStore";
import type { DeviceTargetView } from "../../../hooks/useDeviceStatus";

function device(over: Partial<DeviceTargetView> = {}): DeviceTargetView {
  return {
    targetId: "vm-1",
    displayName: "Office VM",
    mode: "ssh_bootstrap",
    state: "ready",
    tainted: false,
    taintReason: null,
    healthScore: 0.9,
    latencyEwmaMs: 40,
    recentFailureRate: 0.05,
    dockerHealth: "pass",
    dockerPassCount: 4,
    dockerFailCount: 1,
    dockerLastRunAtUnixMs: Date.now(),
    updatedAtUnixMs: Date.now(),
    ...over,
  };
}

afterEach(cleanup);

describe("DeviceInspector — device Inspector body (task 9.1, Req 8.1/17.3)", () => {
  it("discloses identity, status, docker, and test sections from target data", () => {
    const target: InspectorTarget = {
      type: "device",
      id: "vm-1",
      data: { device: device() },
    };
    render(() => <DeviceInspector target={target} />);

    expect(screen.getByText("Office VM")).toBeInTheDocument();
    expect(screen.getByText("vm-1")).toBeInTheDocument();
    expect(screen.getByText("Ready")).toBeInTheDocument();
    expect(screen.getByText("40 ms")).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Docker evals" })).toBeInTheDocument();
    expect(screen.getByText("4 / 1")).toBeInTheDocument();
  });

  it("shows an honest fallback when the target carries no device data (Req 20.4)", () => {
    const target: InspectorTarget = { type: "device", id: "vm-x" };
    render(() => <DeviceInspector target={target} />);
    expect(screen.getByText(/Device details are unavailable/)).toBeInTheDocument();
  });
});
