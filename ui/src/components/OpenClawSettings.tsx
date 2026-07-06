import { Component, createSignal, onMount, Show } from "solid-js";
import { invoke } from "@tauri-apps/api/core";

// Mirrors the backend `OpenClawSettingsPayload` (commands/openclaw.rs).
interface OpenClawSettings {
  enabled: boolean;
  image: string;
  warm_per_class: number;
  max_concurrent_invocations: number;
  default_timeout_secs: number;
  max_warm_age_secs: number;
  max_restart_attempts: number;
  rewrite_descriptions: boolean;
  check_updates: boolean;
  registry_index_url: string;
  community_allows_network: boolean;
  verified_skips_hitl: boolean;
  runtime_active: boolean;
}

/**
 * Production OpenClaw settings — everything configurable from the UI, no TOML editing.
 * Enable/disable the substrate, tune the warm pool + runtime limits, skills, registry
 * and trust policy. Changing enable/image requires a KRIA restart (the container pool
 * is wired at boot); the panel says so clearly instead of failing silently.
 */
const OpenClawSettings: Component = () => {
  const [settings, setSettings] = createSignal<OpenClawSettings | null>(null);
  const [loading, setLoading] = createSignal(true);
  const [saving, setSaving] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [notice, setNotice] = createSignal<string | null>(null);

  const load = async () => {
    setLoading(true);
    setError(null);
    try {
      const s = await invoke<OpenClawSettings>("openclaw_get_settings");
      setSettings(s);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  };

  onMount(() => {
    void load();
  });

  const patch = <K extends keyof OpenClawSettings>(key: K, value: OpenClawSettings[K]) => {
    const cur = settings();
    if (!cur) return;
    setSettings({ ...cur, [key]: value });
  };

  const save = async () => {
    const cur = settings();
    if (!cur) return;
    setSaving(true);
    setError(null);
    setNotice(null);
    try {
      const restartRequired = await invoke<boolean>("openclaw_update_settings", { settings: cur });
      setNotice(
        restartRequired
          ? "Saved. Restart KRIA to apply the enable/image change."
          : "Settings saved."
      );
      await load();
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  };

  const row = { display: "flex", "align-items": "center", "justify-content": "space-between", gap: "12px", padding: "6px 0" } as const;
  const numInput = { width: "110px" } as const;

  return (
    <div class="openclaw-settings">
      <h3>OpenClaw Runtime</h3>
      <p class="settings-hint">
        Sandboxed skill substrate. Configure entirely here — no config files.
      </p>

      <Show when={error()}>
        <div class="settings-error" style={{ color: "#991b1b" }}>{error()}</div>
      </Show>
      <Show when={notice()}>
        <div class="settings-notice" style={{ color: "#166534" }}>{notice()}</div>
      </Show>

      <Show when={!loading() && settings()} fallback={<p>Loading OpenClaw settings…</p>}>
        {(() => {
          const s = settings()!;
          return (
            <div>
              {/* General */}
              <h4>General</h4>
              <div style={row}>
                <label>Enable OpenClaw substrate</label>
                <input
                  type="checkbox"
                  checked={s.enabled}
                  onChange={(e) => patch("enabled", e.currentTarget.checked)}
                />
              </div>
              <div style={row}>
                <span class="settings-hint">
                  Runtime status:{" "}
                  <strong>{s.runtime_active ? "running" : s.enabled ? "enabled (restart to start)" : "disabled"}</strong>
                </span>
              </div>

              {/* Runtime */}
              <h4>Runtime</h4>
              <div style={row}>
                <label>Container image</label>
                <input
                  type="text"
                  value={s.image}
                  style={{ width: "260px" }}
                  onChange={(e) => patch("image", e.currentTarget.value)}
                />
              </div>
              <div style={row}>
                <label>Warm containers per class</label>
                <input type="number" min="0" max="16" style={numInput}
                  value={s.warm_per_class}
                  onChange={(e) => patch("warm_per_class", Number(e.currentTarget.value))} />
              </div>
              <div style={row}>
                <label>Max concurrent invocations</label>
                <input type="number" min="1" max="64" style={numInput}
                  value={s.max_concurrent_invocations}
                  onChange={(e) => patch("max_concurrent_invocations", Number(e.currentTarget.value))} />
              </div>
              <div style={row}>
                <label>Invocation timeout (secs)</label>
                <input type="number" min="1" max="3600" style={numInput}
                  value={s.default_timeout_secs}
                  onChange={(e) => patch("default_timeout_secs", Number(e.currentTarget.value))} />
              </div>
              <div style={row}>
                <label>Idle recycle age (secs)</label>
                <input type="number" min="30" style={numInput}
                  value={s.max_warm_age_secs}
                  onChange={(e) => patch("max_warm_age_secs", Number(e.currentTarget.value))} />
              </div>
              <div style={row}>
                <label>Boot retry attempts</label>
                <input type="number" min="1" max="10" style={numInput}
                  value={s.max_restart_attempts}
                  onChange={(e) => patch("max_restart_attempts", Number(e.currentTarget.value))} />
              </div>

              {/* Skills */}
              <h4>Skills</h4>
              <div style={row}>
                <label>Rewrite skill descriptions (local LLM)</label>
                <input type="checkbox" checked={s.rewrite_descriptions}
                  onChange={(e) => patch("rewrite_descriptions", e.currentTarget.checked)} />
              </div>
              <div style={row}>
                <label>Check for skill updates</label>
                <input type="checkbox" checked={s.check_updates}
                  onChange={(e) => patch("check_updates", e.currentTarget.checked)} />
              </div>
              <div style={row}>
                <label>Registry index URL</label>
                <input type="text" value={s.registry_index_url} style={{ width: "260px" }}
                  onChange={(e) => patch("registry_index_url", e.currentTarget.value)} />
              </div>

              {/* Security */}
              <h4>Security & Trust</h4>
              <div style={row}>
                <label>Community skills may use network</label>
                <input type="checkbox" checked={s.community_allows_network}
                  onChange={(e) => patch("community_allows_network", e.currentTarget.checked)} />
              </div>
              <div style={row}>
                <label>Verified skills skip approval (HITL)</label>
                <input type="checkbox" checked={s.verified_skips_hitl}
                  onChange={(e) => patch("verified_skips_hitl", e.currentTarget.checked)} />
              </div>

              <div style={{ "margin-top": "12px" }}>
                <button disabled={saving()} onClick={() => void save()}>
                  {saving() ? "Saving…" : "Save OpenClaw settings"}
                </button>
              </div>
            </div>
          );
        })()}
      </Show>
    </div>
  );
};

export default OpenClawSettings;
