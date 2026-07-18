import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ServiceResult } from "../../../bridge/types";

const bridgeInvoke = vi.fn();
vi.mock("../../../bridge/invoke", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../../bridge/invoke")>();
  return { ...actual, bridgeInvoke: (...args: unknown[]) => bridgeInvoke(...args) };
});
vi.mock("../../../mobile/RemoteDesktopView", () => ({
  default: () => <div data-testid="remote-desktop-view">Live remote view</div>,
}));

import RemoteDesktopCanvas, { describeRemoteCapability } from "./RemoteDesktopCanvas";
import { machineStore, type RemoteDesktopStatus } from "../../../stores/machineStore";
import { mobileStore } from "../../../mobile/mobileStore";

function ok<T>(data: T): ServiceResult<T> {
  return { ok: true, data };
}

function status(over: Partial<RemoteDesktopStatus> = {}): RemoteDesktopStatus {
  return {
    state: "idle",
    session_id: null,
    started_at: null,
    last_activity: null,
    idle_timeout_secs: 300,
    running: false,
    backend: "WebRTC · portal ScreenCast · unknown session",
    ...over,
  };
}

beforeEach(() => {
  bridgeInvoke.mockReset();
  bridgeInvoke.mockResolvedValue(ok(status()));
  machineStore.setRemoteDesktopStatus(null);
  machineStore.setRemoteDesktopError(null);
  mobileStore.clear();
  mobileStore.setServerUrl("");
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

describe("describeRemoteCapability — honest Linux signaling (Req 8.3)", () => {
  it.each([
    {
      name: "does not guess when runtime status is absent",
      value: null,
      session: "Not reported",
      capture: "Unknown — runtime status unavailable",
      active: false,
    },
    {
      name: "reports pending X11 portal consent",
      value: status({ state: "pending_approval", backend: "WebRTC · portal ScreenCast · x11 session" }),
      session: "X11",
      capture: "Awaiting desktop-portal consent",
      active: false,
    },
    {
      name: "reports granted Wayland capture only while backend runs",
      value: status({ state: "active", running: true, backend: "WebRTC · portal ScreenCast · wayland session" }),
      session: "Wayland",
      capture: "Granted — portal capture running",
      active: true,
    },
    {
      name: "surfaces inconsistent active state instead of claiming success",
      value: status({ state: "active", running: false }),
      session: "Not reported",
      capture: "Unavailable — session active but capture backend stopped",
      active: true,
    },
  ])("$name", ({ value, session, capture, active }) => {
    const result = describeRemoteCapability(value);
    expect(result.session).toBe(session);
    expect(result.capture).toBe(capture);
    expect(result.active).toBe(active);
  });
});

describe("RemoteDesktopCanvas — task 9.2 (Req 8.2/8.3)", () => {
  it("keeps an unmistakable active indicator and dispatches one-action kill", async () => {
    bridgeInvoke.mockImplementation((command: string) =>
      Promise.resolve(command === "remote_desktop_kill"
        ? ok(undefined)
        : ok(status({ state: "active", running: true, session_id: "rd-1", backend: "WebRTC · portal ScreenCast · wayland session" }))),
    );

    render(() => <RemoteDesktopCanvas />);

    expect(await screen.findByText("Remote desktop ACTIVE")).toBeInTheDocument();
    expect(screen.getByText("Granted — portal capture running")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Kill remote desktop session" }));

    await waitFor(() => expect(bridgeInvoke).toHaveBeenCalledWith("remote_desktop_kill"));
    await waitFor(() => expect(screen.getByText("Inactive")).toBeInTheDocument());
  });

  it("does not render capture/input controls without an authenticated endpoint", async () => {
    render(() => <RemoteDesktopCanvas />);
    expect(await screen.findByText("Remote canvas unavailable")).toBeInTheDocument();
    expect(screen.queryByTestId("remote-desktop-view")).toBeNull();
    expect(screen.getByText(/unavailable controls are not rendered/)).toBeInTheDocument();
  });

  it("mounts the proven canvas/toolbar/keyboard session view once paired", async () => {
    mobileStore.setServerUrl("https://kria.local");
    mobileStore.setToken("device-token");
    render(() => <RemoteDesktopCanvas />);
    expect(await screen.findByTestId("remote-desktop-view")).toBeInTheDocument();
  });
});
