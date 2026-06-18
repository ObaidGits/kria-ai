import { Component, createSignal, For, onCleanup, onMount, Show } from "solid-js";
import { invoke } from "@tauri-apps/api/core";

/**
 * Desktop control panel for Phase 4.5 (mobile prompt-control) and Phase 4.6
 * (remote desktop). Everything — enable toggles, gateway lifecycle, device
 * pairing/revocation, and the remote-desktop kill switch + "remote active"
 * indicator — is driven from here.
 */

interface MobileConfig {
  mobile_enabled: boolean;
  require_device_auth: boolean;
  bind_interface: string;
  remote_desktop_enabled: boolean;
}

interface GatewayStatus {
  mobile_enabled: boolean;
  remote_desktop_enabled: boolean;
  running: boolean;
  bound_addr: string | null;
  device_count: number;
  remote_desktop: RemoteStatus;
}

interface RemoteStatus {
  state: "idle" | "pending_approval" | "active" | "stopped" | "expired";
  session_id: string | null;
  running: boolean;
}

interface DeviceInfo {
  id: string;
  name: string;
  created_at: number;
  last_seen: number;
  revoked: boolean;
}

const MobileRemotePanel: Component = () => {
  const [cfg, setCfg] = createSignal<MobileConfig | null>(null);
  const [status, setStatus] = createSignal<GatewayStatus | null>(null);
  const [devices, setDevices] = createSignal<DeviceInfo[]>([]);
  const [pairing, setPairing] = createSignal<{ code: string; mobile_url: string; server_url: string } | null>(null);
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal("");
  let timer: ReturnType<typeof setInterval> | undefined;

  const loadAll = async () => {
    try {
      setCfg(await invoke<MobileConfig>("get_mobile_config"));
      setStatus(await invoke<GatewayStatus>("mobile_gateway_status"));
      const d = await invoke<{ devices: DeviceInfo[] }>("mobile_list_devices");
      setDevices(d.devices ?? []);
    } catch (e) {
      setError(String(e));
    }
  };

  const refreshStatus = async () => {
    try {
      setStatus(await invoke<GatewayStatus>("mobile_gateway_status"));
    } catch {
      /* ignore transient */
    }
  };

  onMount(() => {
    void loadAll();
    timer = setInterval(refreshStatus, 4000);
  });
  onCleanup(() => timer && clearInterval(timer));

  const patch = (p: Partial<MobileConfig>) => setCfg((c) => (c ? { ...c, ...p } : c));

  const save = async () => {
    const c = cfg();
    if (!c) return;
    setBusy(true);
    setError("");
    try {
      await invoke("set_mobile_config", {
        mobileEnabled: c.mobile_enabled,
        requireDeviceAuth: c.require_device_auth,
        bindInterface: c.bind_interface,
        remoteDesktopEnabled: c.remote_desktop_enabled,
      });
      await loadAll();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const startGateway = async () => {
    setBusy(true);
    setError("");
    try {
      await invoke("mobile_gateway_start");
      await refreshStatus();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const stopGateway = async () => {
    setBusy(true);
    try {
      await invoke("mobile_gateway_stop");
      await refreshStatus();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const generateCode = async () => {
    setError("");
    try {
      const p = await invoke<{ code: string; mobile_url: string; server_url: string }>(
        "mobile_begin_pairing",
      );
      setPairing(p);
    } catch (e) {
      setError(String(e));
    }
  };

  const revoke = async (id: string) => {
    try {
      await invoke("mobile_revoke_device", { deviceId: id });
      await loadAll();
    } catch (e) {
      setError(String(e));
    }
  };

  const killRemote = async () => {
    try {
      await invoke("remote_desktop_kill");
      await refreshStatus();
    } catch (e) {
      setError(String(e));
    }
  };

  const rdState = () => status()?.remote_desktop?.state ?? "idle";

  return (
    <section class="settings-section">
      <h3>Mobile &amp; Remote Desktop</h3>

      <Show when={error()}>
        <div class="settings-error">{error()}</div>
      </Show>

      {/* Remote-active indicator (laptop side) */}
      <Show when={rdState() === "active"}>
        <div class="startup-warning-banner">
          🔴 <strong>Remote desktop session ACTIVE</strong> — your screen is being viewed/controlled.
          <button class="btn-secondary" style={{ "margin-left": "12px" }} onClick={killRemote}>
            Kill session
          </button>
        </div>
      </Show>

      {/* Config */}
      <Show when={cfg()}>
        {(c) => (
          <div class="settings-field-group">
            <label class="settings-toggle">
              <input
                type="checkbox"
                checked={c().mobile_enabled}
                onChange={(e) => patch({ mobile_enabled: e.currentTarget.checked })}
              />
              Enable mobile prompt-control (Phase 4.5)
            </label>
            <label class="settings-toggle">
              <input
                type="checkbox"
                checked={c().require_device_auth}
                onChange={(e) => patch({ require_device_auth: e.currentTarget.checked })}
              />
              Require per-device token (recommended)
            </label>
            <label class="settings-toggle">
              <input
                type="checkbox"
                checked={c().remote_desktop_enabled}
                onChange={(e) => patch({ remote_desktop_enabled: e.currentTarget.checked })}
              />
              Enable remote desktop view &amp; takeover (Phase 4.6 — high risk)
            </label>
            <div class="settings-field">
              <label>Bind interface (empty = server host; use your Tailscale IP for privacy)</label>
              <input
                type="text"
                placeholder="100.x.y.z or 0.0.0.0 for LAN"
                value={c().bind_interface}
                onInput={(e) => patch({ bind_interface: e.currentTarget.value })}
              />
            </div>
            <button class="btn-primary" onClick={save} disabled={busy()}>
              {busy() ? "Saving…" : "Save settings"}
            </button>
          </div>
        )}
      </Show>

      {/* Gateway lifecycle */}
      <div class="settings-field" style={{ "margin-top": "16px" }}>
        <h4>Phone gateway</h4>
        <p class="settings-hint">
          Status:{" "}
          <strong>{status()?.running ? `running @ ${status()?.bound_addr}` : "stopped"}</strong>
        </p>
        <div style={{ display: "flex", gap: "10px" }}>
          <Show when={!status()?.running}>
            <button class="btn-primary" onClick={startGateway} disabled={busy()}>
              Start gateway
            </button>
          </Show>
          <Show when={status()?.running}>
            <button class="btn-secondary" onClick={stopGateway} disabled={busy()}>
              Stop gateway
            </button>
          </Show>
        </div>
      </div>

      {/* Pairing */}
      <div class="settings-field" style={{ "margin-top": "16px" }}>
        <h4>Pair a phone</h4>
        <button class="btn-secondary" onClick={generateCode} disabled={!status()?.running}>
          Generate pairing code
        </button>
        <Show when={!status()?.running}>
          <p class="settings-hint">Start the gateway first.</p>
        </Show>
        <Show when={pairing()}>
          {(p) => (
            <div class="settings-pairing-card">
              <p>On your phone open: <strong>{p().mobile_url}</strong></p>
              <p>Server URL: <code>{p().server_url}</code></p>
              <p>Pairing code:</p>
              <div class="settings-pairing-code">{p().code}</div>
            </div>
          )}
        </Show>
      </div>

      {/* Devices */}
      <div class="settings-field" style={{ "margin-top": "16px" }}>
        <h4>Paired devices ({devices().length})</h4>
        <For each={devices()} fallback={<p class="settings-hint">No devices paired yet.</p>}>
          {(d) => (
            <div class="settings-device-row">
              <span>
                {d.name} {d.revoked ? "(revoked)" : ""}
              </span>
              <Show when={!d.revoked}>
                <button class="btn-secondary" onClick={() => revoke(d.id)}>
                  Revoke
                </button>
              </Show>
            </div>
          )}
        </For>
      </div>
    </section>
  );
};

export default MobileRemotePanel;
