import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ServiceResult } from "../../../bridge/types";

const bridgeInvoke = vi.fn();
vi.mock("../../../bridge/invoke", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../../bridge/invoke")>();
  return { ...actual, bridgeInvoke: (...args: unknown[]) => bridgeInvoke(...args) };
});

import MobileDevicesPanel, { isPairingChallengeActive } from "./MobileDevicesPanel";
import {
  machineStore,
  type MobileDeviceInfo,
  type MobileGatewayStatus,
  type MobilePairingChallenge,
} from "../../../stores/machineStore";

function ok<T>(data: T): ServiceResult<T> {
  return { ok: true, data };
}

const gateway: MobileGatewayStatus = {
  mobile_enabled: true,
  remote_desktop_enabled: true,
  running: true,
  bound_addr: "100.64.0.2:7749",
  device_count: 1,
  remote_desktop: {
    state: "idle",
    session_id: null,
    started_at: null,
    last_activity: null,
    idle_timeout_secs: 300,
    running: false,
    backend: "portal",
  },
};

const phone: MobileDeviceInfo = {
  id: "phone-1",
  name: "Owner phone",
  created_at: 1_700_000_000,
  last_seen: 1_700_000_100,
  revoked: false,
};

function challenge(): MobilePairingChallenge {
  return {
    code: "482193",
    qr_payload: "kria://pair/482193",
    expires_at: Math.floor(Date.now() / 1_000) + 300,
    server_url: "http://100.64.0.2:7749",
    mobile_url: "http://100.64.0.2:7749/m",
  };
}

beforeEach(() => {
  bridgeInvoke.mockReset();
  machineStore.resetMobileState();
  bridgeInvoke.mockImplementation((command: string) => {
    if (command === "mobile_gateway_status") return Promise.resolve(ok(gateway));
    if (command === "mobile_list_devices") return Promise.resolve(ok({ devices: [phone] }));
    if (command === "mobile_begin_pairing") return Promise.resolve(ok(challenge()));
    if (command === "mobile_revoke_device") {
      return Promise.resolve(ok({ device_id: phone.id, revoked: true }));
    }
    return Promise.resolve(ok(undefined));
  });
});

afterEach(() => {
  cleanup();
  machineStore.resetMobileState();
  vi.useRealTimers();
});

describe("isPairingChallengeActive", () => {
  it("accepts only finite expiries strictly after now", () => {
    const now = 10_000;
    for (let offset = -100; offset <= 100; offset += 1) {
      expect(isPairingChallengeActive(now + offset, now)).toBe(offset > 0);
    }
    expect(isPairingChallengeActive(Number.NaN, now)).toBe(false);
    expect(isPairingChallengeActive(Number.POSITIVE_INFINITY, now)).toBe(false);
  });
});

describe("MobileDevicesPanel — task 9.3 (Req 8.1/8.4)", () => {
  it("loads gateway + paired devices and generates a single-use pairing code", async () => {
    render(() => <MobileDevicesPanel />);

    expect(await screen.findByText("Gateway running")).toBeInTheDocument();
    expect(screen.getByText("Owner phone")).toBeInTheDocument();
    await machineStore.beginMobilePairing();

    expect(await screen.findByText("482193")).toBeInTheDocument();
    expect(bridgeInvoke).toHaveBeenCalledWith("mobile_begin_pairing");
    expect(screen.getByText("http://100.64.0.2:7749/m")).toBeInTheDocument();
  });

  it("does not dispatch revocation from the destructive trigger itself", async () => {
    render(() => <MobileDevicesPanel />);

    expect(await screen.findByText("Owner phone")).toBeInTheDocument();
    const trigger = screen.getByRole("button", { name: "Revoke access" });
    expect(trigger).toHaveAttribute("aria-haspopup", "dialog");
    expect(bridgeInvoke).not.toHaveBeenCalledWith("mobile_revoke_device", expect.anything());

    // Kobalte dialog portals cannot open under this repo's duplicate-Solid
    // jsdom harness. The structural contract still proves the destructive
    // command is behind the shared Confirm, never bound to this trigger.
    fireEvent.click(trigger);
    expect(bridgeInvoke).not.toHaveBeenCalledWith("mobile_revoke_device", expect.anything());
  });

  it("keeps pairing unavailable while gateway is stopped", async () => {
    bridgeInvoke.mockImplementation((command: string) => {
      if (command === "mobile_gateway_status") {
        return Promise.resolve(ok({ ...gateway, running: false, bound_addr: null }));
      }
      if (command === "mobile_list_devices") return Promise.resolve(ok({ devices: [] }));
      return Promise.resolve(ok(undefined));
    });

    render(() => <MobileDevicesPanel />);
    expect(await screen.findByText("Gateway stopped")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Pair phone" })).toBeDisabled();
    expect(screen.getByText("No mobile devices paired yet.")).toBeInTheDocument();
  });
});
