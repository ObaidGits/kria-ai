import { describe, it, expect, afterEach, vi } from "vitest";
import { render, screen, fireEvent, cleanup } from "@solidjs/testing-library";
import { DeviceRow } from "./DeviceRow";
import type { DeviceTargetView, DeviceTestResultView } from "../../../hooks/useDeviceStatus";

function device(over: Partial<DeviceTargetView> = {}): DeviceTargetView {
  return {
    targetId: "vm-1",
    displayName: "Office VM",
    mode: "ssh_bootstrap",
    state: "ready",
    tainted: false,
    taintReason: null,
    healthScore: 0.95,
    latencyEwmaMs: 40,
    recentFailureRate: 0,
    dockerHealth: "pass",
    dockerPassCount: 3,
    dockerFailCount: 0,
    dockerLastRunAtUnixMs: Date.now(),
    updatedAtUnixMs: Date.now(),
    ...over,
  };
}

function renderRow(props: Parameters<typeof DeviceRow>[0]) {
  return render(() => (
    <table>
      <tbody>
        <DeviceRow {...props} />
      </tbody>
    </table>
  ));
}

afterEach(cleanup);

describe("DeviceRow — fleet matrix row (task 9.1, Req 8.1/17.2/17.3)", () => {
  it("renders health / latency / docker / test signals as icon + text (Req 17.3)", () => {
    const test: DeviceTestResultView = {
      targetId: "vm-1",
      suiteName: "smoke",
      zone: "prod",
      status: "pass",
      timestampUnixMs: Date.now(),
      reportPath: "",
    };
    renderRow({ device: device(), testResult: test, onInspect: () => {} });

    expect(screen.getByText("Office VM")).toBeInTheDocument();
    expect(screen.getByText("vm-1")).toBeInTheDocument();
    expect(screen.getByText("Ready")).toBeInTheDocument();
    expect(screen.getByText("95%")).toBeInTheDocument();
    expect(screen.getByText("40 ms")).toBeInTheDocument();
    // Docker + latest test both read "Pass" (icon + text, never color alone).
    expect(screen.getAllByText("Pass").length).toBeGreaterThanOrEqual(2);
  });

  it("shows an honest 'No runs' test state when there is no result", () => {
    renderRow({ device: device(), testResult: null, onInspect: () => {} });
    expect(screen.getByText("No runs")).toBeInTheDocument();
  });

  it("opens the device Inspector when the name button is activated (Req 1.6/17.1)", () => {
    const onInspect = vi.fn();
    renderRow({ device: device(), onInspect });
    fireEvent.click(screen.getByRole("button", { name: /Office VM/ }));
    expect(onInspect).toHaveBeenCalledTimes(1);
    expect(onInspect.mock.calls[0][0].targetId).toBe("vm-1");
  });

  it("requests a deliberate-confirm delete (never deletes inline, Req 8.4)", () => {
    const onRequestDelete = vi.fn();
    renderRow({ device: device(), onInspect: () => {}, onRequestDelete });
    fireEvent.click(screen.getByRole("button", { name: /Delete Office VM/ }));
    expect(onRequestDelete).toHaveBeenCalledTimes(1);
  });

  it("toggles the terminal via a dispatch-only callback", () => {
    const onToggleTerminal = vi.fn();
    renderRow({ device: device(), onInspect: () => {}, onToggleTerminal });
    fireEvent.click(screen.getByRole("button", { name: "Open terminal" }));
    expect(onToggleTerminal).toHaveBeenCalledWith("vm-1");
  });

  it("disables docker evals with an honest reason when there is no lease", () => {
    const onRunDocker = vi.fn();
    renderRow({
      device: device(),
      onInspect: () => {},
      onRunDocker,
      dockerDisabled: true,
      dockerDisabledReason: "Docker evals need an active fleet lease",
    });
    const btn = screen.getByRole("button", { name: "Docker evals need an active fleet lease" });
    expect(btn).toBeDisabled();
  });

  it("surfaces a taint reason when present", () => {
    renderRow({
      device: device({ state: "tainted", tainted: true, taintReason: "handshake failed" }),
      onInspect: () => {},
    });
    expect(screen.getByText("handshake failed")).toBeInTheDocument();
    expect(screen.getByText("Tainted")).toBeInTheDocument();
  });
});
