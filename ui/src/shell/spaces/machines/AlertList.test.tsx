import { describe, it, expect, afterEach } from "vitest";
import { render, screen, cleanup } from "@solidjs/testing-library";
import { AlertList } from "./AlertList";
import type { DeviceAlertView } from "../../../hooks/useDeviceStatus";

function alert(over: Partial<DeviceAlertView> = {}): DeviceAlertView {
  return {
    category: "clock_drift",
    message: "Clock drift exceeded threshold.",
    targetId: "vm-1",
    leaseId: "lease-9",
    createdAtUnixMs: Date.now() - 30_000,
    ...over,
  };
}

afterEach(cleanup);

describe("AlertList — fleet alerts (task 9.1, Req 8.1/17.3/20.4)", () => {
  it("shows an honest empty state when there are no alerts (Req 20.4)", () => {
    render(() => <AlertList alerts={[]} />);
    expect(screen.getByText("No active alerts")).toBeInTheDocument();
    // No list is rendered when empty.
    expect(screen.queryByRole("list")).not.toBeInTheDocument();
  });

  it("renders alerts as a real semantic list with category as icon + text (Req 17.2/17.3)", () => {
    render(() => (
      <AlertList
        alerts={[alert(), alert({ category: "lease", targetId: null, message: "Lease renewed." })]}
      />
    ));
    const list = screen.getByRole("list", { name: "Fleet alerts" });
    expect(list).toBeInTheDocument();
    expect(screen.getAllByRole("listitem")).toHaveLength(2);
    // Category text present (not color-only).
    expect(screen.getByText("clock_drift")).toBeInTheDocument();
    expect(screen.getByText("lease")).toBeInTheDocument();
    expect(screen.getByText("Clock drift exceeded threshold.")).toBeInTheDocument();
  });

  it("surfaces device + lease metadata when present", () => {
    render(() => <AlertList alerts={[alert()]} />);
    expect(screen.getByText("device vm-1")).toBeInTheDocument();
    expect(screen.getByText("lease lease-9")).toBeInTheDocument();
  });

  it("caps the number of rendered alerts", () => {
    const many = Array.from({ length: 30 }, (_, i) => alert({ message: `m${i}` }));
    render(() => <AlertList alerts={many} max={5} />);
    expect(screen.getAllByRole("listitem")).toHaveLength(5);
  });
});
