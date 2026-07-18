import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { render, screen, cleanup, waitFor } from "@solidjs/testing-library";
import MachinesSpace from "./MachinesSpace";
import { machineStore, shellStore } from "../../stores";
import { navigate } from "../router";
import { getInspectorRenderer, resetInspectorRegistry } from "../inspectorRegistry";

describe("MachinesSpace — fleet, mobile, remote, terminal + alerts (tasks 9.1–9.3, Req 8.1/8.4)", () => {
  beforeEach(() => {
    vi.spyOn(machineStore, "loadFleetStatus").mockResolvedValue(undefined);
    vi.spyOn(machineStore, "loadMobileState").mockResolvedValue(undefined);
    machineStore.setIroncladStatus(null);
    machineStore.setSettings(null);
    shellStore.setInspectorTarget(null);
    navigate("machines");
    resetInspectorRegistry();
  });

  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it("renders the Machines header", () => {
    render(() => <MachinesSpace />);
    expect(screen.getByRole("heading", { level: 1, name: "Machines" })).toBeInTheDocument();
  });

  it("renders the fleet matrix as a REAL table with the required columns (Req 17.2)", () => {
    render(() => <MachinesSpace />);
    expect(screen.getByRole("table")).toBeInTheDocument();
    for (const col of ["Device", "State", "Health", "Latency", "Docker", "Tests"]) {
      expect(screen.getByRole("columnheader", { name: col })).toBeInTheDocument();
    }
  });

  it("shows an honest empty state when no fleet controller is configured (Req 20.4)", () => {
    render(() => <MachinesSpace />);
    expect(screen.getByText(/No fleet controller configured/)).toBeInTheDocument();
  });

  it("registers the device Inspector renderer on mount (Req 1.6)", () => {
    render(() => <MachinesSpace />);
    expect(getInspectorRenderer("device")).toBeTypeOf("function");
  });

  it("consumes a device deep link after fleet targets arrive", async () => {
    machineStore.setIroncladStatus({
      fleet: {
        enrolled_targets: [
          { target_id: "device-1", display_name: "Office VM", mode: "ssh_bootstrap" },
        ],
      },
    });
    navigate("machines", "device", "device-1");
    render(() => <MachinesSpace />);

    await waitFor(() => {
      expect(shellStore.inspectorTarget()?.type).toBe("device");
      expect(shellStore.inspectorTarget()?.id).toBe("device-1");
      expect(document.activeElement).toBe(
        document.querySelector<HTMLElement>('[data-target-id="device-1"] button'),
      );
    });
  });

  it("exposes an enroll-device affordance in the toolbar (Req 8.1)", () => {
    // The wizard's Kobalte modal cannot be opened via a controlled prop under
    // jsdom; the wizard body itself is covered by EnrollWizardBody tests.
    render(() => <MachinesSpace />);
    expect(screen.getByRole("button", { name: /Enroll device/ })).toBeInTheDocument();
  });

  it("integrates mobile pairing/devices inside Machines (Req 8.1)", () => {
    render(() => <MachinesSpace />);
    expect(screen.getByRole("heading", { name: "Mobile devices" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Pair phone" })).toBeDisabled();
  });

  it("shows the honest terminal + alert empty states", () => {
    render(() => <MachinesSpace />);
    expect(screen.getByText(/Select a device to attach/)).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "No active alerts" })).toBeInTheDocument();
  });
});
