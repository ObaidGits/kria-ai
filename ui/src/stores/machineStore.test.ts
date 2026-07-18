import { describe, expect, it } from "vitest";
import {
  sameMobileDevices,
  sameMobileGatewayStatus,
  sameRemoteDesktopStatus,
  type MobileDeviceInfo,
  type MobileGatewayStatus,
  type RemoteDesktopStatus,
} from "./machineStore";

const remote: RemoteDesktopStatus = {
  state: "idle", session_id: null, started_at: null, last_activity: null,
  idle_timeout_secs: 300, running: false, backend: "wayland-portal",
};
const gateway: MobileGatewayStatus = {
  mobile_enabled: true, remote_desktop_enabled: true, running: true,
  bound_addr: "127.0.0.1:9443", device_count: 1, remote_desktop: remote,
};
const devices: MobileDeviceInfo[] = [
  { id: "phone-1", name: "Phone", created_at: 10, last_seen: 20, revoked: false },
];

describe("machine poll snapshot equality", () => {
  it("treats value-identical remote and gateway snapshots as unchanged", () => {
    expect(sameRemoteDesktopStatus(remote, { ...remote })).toBe(true);
    expect(sameMobileGatewayStatus(gateway, {
      ...gateway,
      remote_desktop: { ...remote },
    })).toBe(true);
  });

  it("detects changed remote and gateway fields", () => {
    expect(sameRemoteDesktopStatus(remote, { ...remote, running: true })).toBe(false);
    expect(sameMobileGatewayStatus(gateway, { ...gateway, device_count: 2 })).toBe(false);
  });

  it("treats cloned device lists as unchanged and detects row changes", () => {
    expect(sameMobileDevices(devices, devices.map((device) => ({ ...device })))).toBe(true);
    expect(sameMobileDevices(devices, [{ ...devices[0], revoked: true }])).toBe(false);
    expect(sameMobileDevices(devices, [])).toBe(false);
  });
});
