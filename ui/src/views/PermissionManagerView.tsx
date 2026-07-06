// Task 13.3 — Permission management + developer-mode gating.
//
// Lists active scoped permission grants from the GrantStore and lets the user
// explicitly revoke one (a deliberate state mutation via `openclaw_revoke_grant`
// → frozen `PermissionEngine::revoke`; never auto-revoked). Also hosts the
// Developer Mode toggle (`openclaw_get_developer_mode` / `_set_`), which the app
// uses to gate not-production-ready features (R10.3).

import { Component, createSignal, For, Show, onMount, onCleanup } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { GrantsPayload, ScopedGrantView } from "./openclawIcpTypes";

const PermissionManagerView: Component = () => {
  const [grants, setGrants] = createSignal<ScopedGrantView[]>([]);
  const [status, setStatus] = createSignal("");
  const [devMode, setDevMode] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [busy, setBusy] = createSignal<string | null>(null);
  const unlisteners: UnlistenFn[] = [];

  const refresh = async () => {
    setError(null);
    try {
      const p = await invoke<GrantsPayload>("openclaw_list_grants");
      setGrants(p.grants);
      setStatus(p.status);
    } catch (e: unknown) {
      setError(String(e));
    }
  };

  const loadDevMode = async () => {
    try {
      setDevMode(await invoke<boolean>("openclaw_get_developer_mode"));
    } catch (e: unknown) {
      setError(String(e));
    }
  };

  const revoke = async (grantId: string) => {
    setBusy(grantId);
    setError(null);
    try {
      await invoke("openclaw_revoke_grant", { grantId });
      await refresh();
    } catch (e: unknown) {
      setError(String(e));
    } finally {
      setBusy(null);
    }
  };

  const toggleDevMode = async () => {
    const next = !devMode();
    try {
      await invoke("openclaw_set_developer_mode", { enabled: next });
      setDevMode(next);
    } catch (e: unknown) {
      setError(String(e));
    }
  };

  onMount(async () => {
    await Promise.all([refresh(), loadDevMode()]);
    unlisteners.push(await listen("openclaw:grants_changed", () => void refresh()));
    unlisteners.push(
      await listen<boolean>("openclaw:developer_mode", (ev) => setDevMode(ev.payload)),
    );
  });
  onCleanup(() => unlisteners.forEach((u) => u()));

  return (
    <div class="openclaw-permission-manager" style={{ padding: "12px" }}>
      <h3>Permission Management</h3>

      <div
        style={{
          display: "flex",
          "align-items": "center",
          gap: "8px",
          "margin-bottom": "12px",
        }}
      >
        <label>Developer Mode</label>
        <input type="checkbox" checked={devMode()} onChange={() => void toggleDevMode()} />
        <span class="settings-hint">Reveals not-production-ready OpenClaw surfaces.</span>
      </div>

      <Show when={error()}>
        <p style={{ color: "#ef4444" }}>Error: {error()}</p>
      </Show>

      <div style={{ display: "flex", "align-items": "center", "justify-content": "space-between" }}>
        <p class="settings-hint">{status()}</p>
        <button onClick={() => void refresh()}>Refresh</button>
      </div>

      <Show when={grants().length === 0}>
        <p class="settings-hint">No active permission grants.</p>
      </Show>

      <For each={grants()}>
        {(g: ScopedGrantView) => (
          <div
            style={{
              border: "1px solid rgba(255,255,255,0.1)",
              "border-radius": "8px",
              padding: "10px",
              "margin-bottom": "8px",
              display: "flex",
              "align-items": "center",
              "justify-content": "space-between",
            }}
          >
            <div style={{ "font-size": "12px" }}>
              <div>
                <strong>{g.skill_id}</strong>{" "}
                <span style={{ color: g.decision === "allow" ? "#22c55e" : "#ef4444" }}>
                  {g.decision}
                </span>
              </div>
              <div style={{ color: "#9ca3af" }}>
                scope: {g.scope_kind}
                {g.scope_key ? ` (${g.scope_key})` : ""} · risk: {g.risk}
              </div>
              <div style={{ color: "#6b7280" }}>
                granted: {g.granted_at}
                {g.expires_at ? ` · expires: ${g.expires_at}` : ""}
              </div>
            </div>
            <button disabled={busy() === g.grant_id} onClick={() => void revoke(g.grant_id)}>
              {busy() === g.grant_id ? "Revoking…" : "Revoke"}
            </button>
          </div>
        )}
      </For>
    </div>
  );
};

export default PermissionManagerView;
