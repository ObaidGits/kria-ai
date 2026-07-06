// Task 13.1 — Capability Manager + generated-skills provenance surface.
//
// Lists installed OpenClaw skills from the frozen registry with their
// provenance (including A9-generated skills) and, when `openclaw_icp_enabled`
// is ON, their derived capability profile (provides/consumes/inputs/outputs
// from the `capability_profiles` view). Honest degraded banner when the flag is
// OFF or the derived view is unavailable. Pushes update via `openclaw:capabilities`.

import { Component, createSignal, For, Show, onMount, onCleanup } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { CapabilityManagerPayload, CapabilitySkillCard } from "./openclawIcpTypes";

const provenanceColor: Record<string, string> = {
  generated: "#a855f7",
  bundled: "#3b82f6",
  installed_bundle: "#0ea5e9",
  clawhub: "#f59e0b",
  workspace: "#22c55e",
  developer: "#ef4444",
};

const CapabilityManagerView: Component = () => {
  const [payload, setPayload] = createSignal<CapabilityManagerPayload | null>(null);
  const [loading, setLoading] = createSignal(true);
  const [error, setError] = createSignal<string | null>(null);
  let unlisten: UnlistenFn | undefined;

  const refresh = async () => {
    setError(null);
    try {
      const p = await invoke<CapabilityManagerPayload>("openclaw_capability_manager");
      setPayload(p);
    } catch (e: unknown) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  };

  onMount(async () => {
    await refresh();
    // Push-sync: reconcile on the additive `openclaw:capabilities` event.
    unlisten = await listen<CapabilityManagerPayload>("openclaw:capabilities", (ev) => {
      setPayload(ev.payload);
    });
  });
  onCleanup(() => unlisten?.());

  return (
    <div class="openclaw-capability-manager" style={{ padding: "12px" }}>
      <div style={{ display: "flex", "align-items": "center", "justify-content": "space-between" }}>
        <h3>Capability Manager</h3>
        <button disabled={loading()} onClick={() => void refresh()}>
          {loading() ? "Loading…" : "Refresh"}
        </button>
      </div>

      <Show when={error()}>
        <p style={{ color: "#ef4444" }}>Error: {error()}</p>
      </Show>

      <Show when={payload()}>
        {(p) => (
          <>
            <Show when={p().degraded}>
              <p class="settings-hint" style={{ color: "#f59e0b" }}>
                {p().status}
              </p>
            </Show>
            <Show when={!p().degraded}>
              <p class="settings-hint">{p().status}</p>
            </Show>

            <Show when={p().skills.length === 0}>
              <p class="settings-hint">No installed skills.</p>
            </Show>

            <For each={p().skills}>
              {(skill: CapabilitySkillCard) => (
                <div
                  style={{
                    border: "1px solid rgba(255,255,255,0.1)",
                    "border-radius": "8px",
                    padding: "10px",
                    "margin-bottom": "8px",
                  }}
                >
                  <div style={{ display: "flex", "align-items": "center", gap: "8px" }}>
                    <strong>{skill.name}</strong>
                    <span
                      style={{
                        "font-size": "11px",
                        padding: "2px 6px",
                        "border-radius": "4px",
                        background: provenanceColor[skill.provenance] ?? "#6b7280",
                        color: "#fff",
                      }}
                    >
                      {skill.provenance}
                    </span>
                    <Show when={skill.generated_workflow_id}>
                      <span style={{ "font-size": "11px", color: "#a855f7" }}>
                        workflow: {skill.generated_workflow_id}
                      </span>
                    </Show>
                    <span style={{ "font-size": "11px", color: skill.enabled ? "#22c55e" : "#9ca3af" }}>
                      {skill.state}
                    </span>
                  </div>
                  <div style={{ "font-size": "12px", color: "#9ca3af" }}>{skill.description}</div>
                  <div style={{ "font-size": "11px", color: "#9ca3af", "margin-top": "4px" }}>
                    {skill.category} · trust: {skill.trust_tier} · risk: {skill.risk_level}
                  </div>

                  <Show
                    when={skill.profile.has_profile}
                    fallback={
                      <div style={{ "font-size": "11px", color: "#6b7280", "margin-top": "4px" }}>
                        No capability profile indexed.
                      </div>
                    }
                  >
                    <div style={{ "font-size": "11px", "margin-top": "6px" }}>
                      <Show when={skill.profile.provides.length > 0}>
                        <div>provides: {skill.profile.provides.join(", ")}</div>
                      </Show>
                      <Show when={skill.profile.consumes.length > 0}>
                        <div>consumes: {skill.profile.consumes.join(", ")}</div>
                      </Show>
                      <Show when={skill.profile.inputs.length > 0}>
                        <div>inputs: {skill.profile.inputs.join(", ")}</div>
                      </Show>
                      <Show when={skill.profile.outputs.length > 0}>
                        <div>outputs: {skill.profile.outputs.join(", ")}</div>
                      </Show>
                    </div>
                  </Show>
                </div>
              )}
            </For>
          </>
        )}
      </Show>
    </div>
  );
};

export default CapabilityManagerView;
