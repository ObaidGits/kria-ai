import { describe, it, expect, afterEach, vi } from "vitest";
import { render, screen, fireEvent, cleanup } from "@solidjs/testing-library";
import { TerminalPane } from "./TerminalPane";
import type { DeviceTargetView, DeviceTerminalLine } from "../../../hooks/useDeviceStatus";

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
    recentFailureRate: 0,
    dockerHealth: "pass",
    dockerPassCount: 1,
    dockerFailCount: 0,
    dockerLastRunAtUnixMs: Date.now(),
    updatedAtUnixMs: Date.now(),
    ...over,
  };
}

function line(offset: number, text: string, stream: DeviceTerminalLine["stream"] = "stdout"): DeviceTerminalLine {
  return { targetId: "vm-1", offset, stream, text, tsUnixMs: Date.now() };
}

afterEach(cleanup);

describe("TerminalPane — focused device terminal (task 9.1, Req 8.1/17.1)", () => {
  it("shows a prompt to select a device when none is focused", () => {
    render(() => <TerminalPane device={null} lines={[]} />);
    expect(screen.getByText("Select a device to attach its terminal stream.")).toBeInTheDocument();
  });

  it("exposes a keyboard-focusable log live region (Req 17.1)", () => {
    render(() => <TerminalPane device={device()} lines={[line(1, "$ ls")]} />);
    const log = screen.getByRole("log", { name: /Terminal output for Office VM/ });
    expect(log).toBeInTheDocument();
    // Focusable so keyboard users can scroll it.
    expect(log.getAttribute("tabindex")).toBe("0");
    expect(log).toHaveAttribute("aria-live", "polite");
  });

  it("renders terminal lines as escaped text (untrusted substrate output)", () => {
    render(() => <TerminalPane device={device()} lines={[line(1, "<script>alert(1)</script>")]} />);
    expect(screen.getByText("<script>alert(1)</script>")).toBeInTheDocument();
  });

  it("shows an honest empty state when there is no output yet", () => {
    render(() => <TerminalPane device={device()} lines={[]} />);
    expect(screen.getByText("No terminal output yet.")).toBeInTheDocument();
  });

  it("caps rendered lines to the last N (maxLines)", () => {
    const lines = Array.from({ length: 10 }, (_, i) => line(i + 1, `line-${i}`));
    render(() => <TerminalPane device={device()} lines={lines} maxLines={3} />);
    expect(screen.queryByText("line-6")).not.toBeInTheDocument();
    expect(screen.getByText("line-9")).toBeInTheDocument();
  });

  it("detaches via a labelled dispatch-only callback", () => {
    const onDetach = vi.fn();
    render(() => <TerminalPane device={device()} lines={[]} onDetach={onDetach} />);
    fireEvent.click(screen.getByRole("button", { name: "Detach terminal" }));
    expect(onDetach).toHaveBeenCalledTimes(1);
  });
});


describe("TerminalPane — log virtualization (Req 16.2)", () => {
  it("mounts only visible lines from a large bounded ring", () => {
    const lines = Array.from({ length: 500 }, (_, i) => line(i + 1, `heavy-log-${i}`));
    render(() => <TerminalPane device={device()} lines={lines} maxLines={500} />);
    const mounted = document.querySelectorAll('[data-virtual-list="terminal-log"] [data-offset]').length;
    expect(mounted).toBeGreaterThan(0);
    expect(mounted).toBeLessThan(500);
  });
});
