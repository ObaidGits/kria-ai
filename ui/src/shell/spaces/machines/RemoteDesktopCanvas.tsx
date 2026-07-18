import { createMemo, onCleanup, onMount, Show } from "solid-js";
import { Button, StatusDot } from "../../../kit";
import { Icon } from "../../../components/Icon";
import { machineStore, type RemoteDesktopStatus } from "../../../stores/machineStore";
import { mobileStore } from "../../../mobile/mobileStore";
import RemoteDesktopView from "../../../mobile/RemoteDesktopView";
import { openDetachedSurface, windowPresentation } from "../../../windowing/detachableSurfaces";

export type LinuxSessionKind = "Wayland" | "X11" | "Not reported";

export interface RemoteCapabilityPresentation {
  session: LinuxSessionKind;
  capture: string;
  input: string;
  active: boolean;
  inconsistent: boolean;
}

/** Derive labels only from runtime evidence; never guess Wayland/X11 or grants. */
export function describeRemoteCapability(
  status: RemoteDesktopStatus | null,
): RemoteCapabilityPresentation {
  if (!status) {
    return {
      session: "Not reported",
      capture: "Unknown — runtime status unavailable",
      input: "Unknown — runtime status unavailable",
      active: false,
      inconsistent: false,
    };
  }

  const backend = status.backend.toLowerCase();
  const session: LinuxSessionKind = backend.includes("wayland")
    ? "Wayland"
    : backend.includes("x11")
      ? "X11"
      : "Not reported";
  const active = status.state === "active";
  const inconsistent = active && !status.running;

  if (status.state === "pending_approval") {
    return {
      session,
      capture: "Awaiting desktop-portal consent",
      input: "Awaiting RemoteDesktop permission",
      active: false,
      inconsistent: false,
    };
  }
  if (active && status.running) {
    return {
      session,
      capture: "Granted — portal capture running",
      input: "Granted — portal input enabled",
      active: true,
      inconsistent: false,
    };
  }

  if (inconsistent) {
    return {
      session,
      capture: "Unavailable — session active but capture backend stopped",
      input: "Unavailable — no running input backend",
      active: true,
      inconsistent: true,
    };
  }

  return {
    session,
    capture: "Not granted — no active capture session",
    input: "Not granted — no active input session",
    active: false,
    inconsistent: false,
  };
}

/**
 * Machines remote-desktop surface. KRIA runtime remains authoritative: this
 * component reads status and dispatches only the existing kill command. Session
 * start still follows request → explicit HITL confirm → portal grant inside the
 * proven RemoteDesktopView control plane; transport retries remain bounded.
 */
export default function RemoteDesktopCanvas() {
  let pollId: number | undefined;
  const status = machineStore.remoteDesktopStatus;
  const presentation = createMemo(() => describeRemoteCapability(status()));
  const paired = () => mobileStore.isPaired();

  onMount(() => {
    void machineStore.loadRemoteDesktopStatus();
    pollId = window.setInterval(() => void machineStore.loadRemoteDesktopStatus(), 3_000);
  });
  onCleanup(() => {
    if (pollId !== undefined) window.clearInterval(pollId);
  });

  return (
    <section
      class="kria-remote"
      aria-labelledby="kria-remote-title"
      data-active={presentation().active ? "true" : "false"}
    >
      <div class="kria-remote__head">
        <div>
          <h2 id="kria-remote-title" class="kria-machines__region-title">Remote desktop</h2>
          <p class="kria-remote__summary">
            Portal-backed ScreenCast + RemoteDesktop. Capture and input require explicit OS consent.
          </p>
        </div>
        <div class="kria-remote__head-actions">
          <Show when={windowPresentation.surface() !== "remote-desktop"}>
            <Button variant="secondary" size="sm"
              onClick={() => void openDetachedSurface("remote-desktop")}>
              <Icon name="monitor" size={13} aria-hidden /> Detach
            </Button>
          </Show>
          <Show when={presentation().active} fallback={
          <span class="kria-remote__inactive">
            <StatusDot tone="offline" label="Remote desktop inactive" /> Inactive
          </span>
        }>
          <div class="kria-remote__active" role="status" aria-live="assertive">
            <span><StatusDot tone={presentation().inconsistent ? "error" : "online"} label="Remote desktop active" /> Remote desktop ACTIVE</span>
            <Button
              variant="danger"
              size="sm"
              disabled={machineStore.remoteDesktopBusy()}
              aria-label="Kill remote desktop session"
              onClick={() => void machineStore.killRemoteDesktop()}
            >
              <Icon name="square" size={13} aria-hidden />
              {machineStore.remoteDesktopBusy() ? "Stopping…" : "Kill session"}
            </Button>
          </div>
          </Show>
        </div>
      </div>

      <dl class="kria-remote__capabilities" aria-label="Remote desktop capabilities and permissions">
        <div><dt>Linux session</dt><dd>{presentation().session}</dd></div>
        <div><dt>Capture</dt><dd>{presentation().capture}</dd></div>
        <div><dt>Input</dt><dd>{presentation().input}</dd></div>
        <div><dt>Backend</dt><dd>{status()?.backend || "Not reported by runtime"}</dd></div>
      </dl>

      <Show when={machineStore.remoteDesktopError()}>
        {(message) => <p class="kria-remote__error" role="alert">{message()}</p>}
      </Show>

      <div class="kria-remote__canvas" aria-label="Remote desktop canvas">
        <Show when={paired()} fallback={
          <div class="kria-remote__unavailable" role="status">
            <Icon name="monitor-off" size={24} aria-hidden />
            <h3>Remote canvas unavailable</h3>
            <p>
              No authenticated remote endpoint is configured. Pair this client before capture,
              toolbar, or keyboard controls are offered; unavailable controls are not rendered.
            </p>
          </div>
        }>
          <RemoteDesktopView />
        </Show>
      </div>
    </section>
  );
}
