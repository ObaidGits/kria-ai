/**
 * Mobile pairing + device governance inside Machines (task 9.3, Req 8.1/8.4).
 * All effects dispatch through KRIA-owned Tauri commands; this surface never
 * contacts the phone gateway directly. Device revocation is confirm-gated.
 */
import { createMemo, createSignal, For, onCleanup, onMount, Show } from "solid-js";
import { Button, Confirm, IconButton, StatusDot } from "../../../kit";
import { Icon } from "../../../components/Icon";
import { machineStore } from "../../../stores/machineStore";
import { formatAbsolute } from "./fleetPresentation";
import "./machines.css";

const POLL_INTERVAL_MS = 4_000;

export function isPairingChallengeActive(expiresAt: number, now: number): boolean {
  return Number.isFinite(expiresAt) && Number.isFinite(now) && expiresAt > now;
}

function formatUnixSeconds(timestamp: number): string {
  return timestamp > 0 ? formatAbsolute(timestamp * 1_000) : "Never";
}

export default function MobileDevicesPanel() {
  let pollId: number | undefined;
  let refreshInFlight = false;
  const [nowSeconds, setNowSeconds] = createSignal(Math.floor(Date.now() / 1_000));

  const status = machineStore.mobileGatewayStatus;
  const pairing = machineStore.mobilePairing;
  const gatewayRunning = () => status()?.running === true;
  const pairingActive = createMemo(() => {
    const challenge = pairing();
    return !!challenge && isPairingChallengeActive(challenge.expires_at, nowSeconds());
  });

  async function refresh(): Promise<void> {
    if (refreshInFlight) return;
    refreshInFlight = true;
    try {
      await machineStore.loadMobileState();
    } finally {
      refreshInFlight = false;
    }
  }

  onMount(() => {
    void refresh();
    pollId = window.setInterval(() => {
      setNowSeconds(Math.floor(Date.now() / 1_000));
      if (!machineStore.mobileBusy()) void refresh();
    }, POLL_INTERVAL_MS);
  });

  onCleanup(() => {
    if (pollId !== undefined) window.clearInterval(pollId);
  });

  return (
    <section class="kria-mobile" aria-labelledby="mobile-devices-title">
      <div class="kria-mobile__head">
        <div>
          <h2 id="mobile-devices-title" class="kria-machines__region-title">Mobile devices</h2>
          <p class="kria-mobile__summary">
            Pair phones through KRIA's authenticated gateway. Each device receives its own revocable token.
          </p>
        </div>
        <div class="kria-mobile__gateway-state">
          <StatusDot
            tone={gatewayRunning() ? "online" : status() ? "offline" : "busy"}
            label={gatewayRunning() ? "Gateway running" : status() ? "Gateway stopped" : "Checking gateway"}
          />
          <Show when={status()?.bound_addr}>
            {(address) => <code>{address()}</code>}
          </Show>
        </div>
      </div>

      <Show when={machineStore.mobileError()}>
        {(message) => (
          <p class="kria-mobile__error" role="alert">
            <Icon name="alert-circle" size={15} aria-hidden />
            <span>{message()}</span>
          </p>
        )}
      </Show>

      <div class="kria-mobile__actions">
        <Show
          when={gatewayRunning()}
          fallback={
            <Button
              variant="secondary"
              size="sm"
              disabled={machineStore.mobileBusy()}
              onClick={() => void machineStore.startMobileGateway()}
            >
              <Icon name="play" size={13} aria-hidden /> Start gateway
            </Button>
          }
        >
          <Button
            variant="secondary"
            size="sm"
            disabled={machineStore.mobileBusy()}
            onClick={() => void machineStore.stopMobileGateway()}
          >
            <Icon name="square" size={13} aria-hidden /> Stop gateway
          </Button>
        </Show>
        <Button
          variant="primary"
          size="sm"
          disabled={!gatewayRunning() || machineStore.mobileBusy()}
          onClick={() => void machineStore.beginMobilePairing()}
        >
          <Icon name="smartphone" size={13} aria-hidden /> Pair phone
        </Button>
        <IconButton
          icon="refresh-cw"
          label="Refresh mobile devices"
          size="sm"
          disabled={machineStore.mobileBusy()}
          onClick={() => void refresh()}
        />
      </div>

      <Show when={gatewayRunning() && pairingActive() && pairing()}>
        {(challenge) => (
          <div class="kria-mobile__pairing" role="status" aria-label="Active mobile pairing code">
            <div class="kria-mobile__pairing-copy">
              <span class="kria-mobile__eyebrow">Single-use pairing code</span>
              <strong class="kria-mobile__code">{challenge().code}</strong>
              <span>Expires {formatUnixSeconds(challenge().expires_at)}</span>
            </div>
            <dl class="kria-mobile__pairing-meta">
              <dt>Open on phone</dt>
              <dd><code>{challenge().mobile_url}</code></dd>
              <dt>KRIA server</dt>
              <dd><code>{challenge().server_url}</code></dd>
            </dl>
          </div>
        )}
      </Show>
      <Show when={pairing() && !pairingActive()}>
        <p class="kria-mobile__notice" role="status">
          Pairing code expired. Generate a new single-use code.
        </p>
      </Show>

      <div class="kria-mobile__devices">
        <h3>Paired devices ({machineStore.mobileDevices().length})</h3>
        <Show
          when={machineStore.mobileDevices().length > 0}
          fallback={<p class="kria-mobile__empty">No mobile devices paired yet.</p>}
        >
          <div class="kria-mobile__table-wrap">
            <table class="kria-mobile__table">
              <caption>Phones authorized to connect to this KRIA instance</caption>
              <thead>
                <tr>
                  <th scope="col">Device</th>
                  <th scope="col">Access</th>
                  <th scope="col">Paired</th>
                  <th scope="col">Last seen</th>
                  <th scope="col"><span class="kit-visually-hidden">Actions</span></th>
                </tr>
              </thead>
              <tbody>
                <For each={machineStore.mobileDevices()}>
                  {(device) => (
                    <tr>
                      <td>
                        <strong>{device.name}</strong>
                        <span class="kria-mobile__device-id">{device.id}</span>
                      </td>
                      <td>
                        <StatusDot
                          tone={device.revoked ? "offline" : "online"}
                          label={device.revoked ? "Revoked" : "Authorized"}
                        />
                      </td>
                      <td>{formatUnixSeconds(device.created_at)}</td>
                      <td>{formatUnixSeconds(device.last_seen)}</td>
                      <td class="kria-mobile__row-action">
                        <Show when={!device.revoked}>
                          <Confirm
                            triggerLabel="Revoke access"
                            title="Revoke mobile device?"
                            message={`This invalidates “${device.name}” immediately. It must pair again before it can send prompts or request remote access.`}
                            confirmLabel="Revoke device"
                            risk="danger"
                            onConfirm={() => void machineStore.revokeMobileDevice(device.id)}
                          />
                        </Show>
                      </td>
                    </tr>
                  )}
                </For>
              </tbody>
            </table>
          </div>
        </Show>
      </div>
    </section>
  );
}
