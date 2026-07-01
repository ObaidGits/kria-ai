import { Component, For, Show } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { appStore } from "../stores/app";

/**
 * ResourceDashboard — live panel for the Hardware & Resource Authority (HRA).
 *
 * Consumes the additive `resource:hra_status` stream (`appStore.hraStatus`) for the overview and
 * the richer `resource:hra_diagnostics` bundle (`appStore.hraDiagnostics`) for devices, telemetry
 * freshness, recovered crash leases, and SLA. Renders the six HRA UI surfaces (Tasks 20/40):
 * Overview, Explainability, Session Awareness, Forecasting, Recovery, Diagnostics export.
 *
 * Views whose backend data is genuinely not yet streamed (per-decision rationale, session
 * ownership, forecasting) show an explicit "awaiting data" state rather than faking content — the
 * panel stays honest about what the backend currently provides.
 */
const ResourceDashboard: Component = () => {
  const { hraStatus, hraDiagnostics } = appStore;

  const status = () => hraStatus() ?? undefined;
  const diag = () => hraDiagnostics() ?? undefined;
  const metrics = () => (status()?.["metrics"] as Record<string, unknown> | undefined) ?? undefined;
  const devices = () => (diag()?.["devices"] as Array<Record<string, unknown>> | undefined) ?? [];
  const telemetry = () => (diag()?.["telemetry"] as Record<string, unknown> | undefined) ?? undefined;
  const recovered = () =>
    (diag()?.["recovered_open_leases"] as Array<Record<string, unknown>> | undefined) ?? [];
  const residents = () =>
    (diag()?.["residents"] as Array<Record<string, unknown>> | undefined) ?? [];
  const coResidency = () =>
    (diag()?.["co_residency"] as Record<string, unknown> | undefined) ?? undefined;
  const decisions = () =>
    (diag()?.["decisions"] as Array<Record<string, unknown>> | undefined) ?? [];
  const forecast = () =>
    (diag()?.["forecast"] as Record<string, unknown> | undefined) ?? undefined;

  const exportDiagnostics = async () => {
    // Prefer the authoritative backend bundle (on-demand pull); fall back to the streamed copy.
    let bundle: unknown = diag() ?? null;
    try {
      bundle = await invoke("get_hra_diagnostics");
    } catch {
      /* keep streamed copy */
    }
    const payload = {
      exported_at: new Date().toISOString(),
      hra_status: status() ?? null,
      hra_diagnostics: bundle,
    };
    const blob = new Blob([JSON.stringify(payload, null, 2)], { type: "application/json" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = `kria-hra-diagnostics-${Date.now()}.json`;
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
    URL.revokeObjectURL(url);
  };

  return (
    <div class="resource-dashboard" role="status" aria-live="polite">
      <Show
        when={status()}
        fallback={<div class="resource-dashboard-empty">Resource Authority: starting…</div>}
      >
        {/* View 1 — Dashboard / Overview */}
        <div class="resource-view">
          <div class="resource-dashboard-row">
            <span class="resource-dashboard-label">Resource Authority</span>
            <span class="resource-dashboard-mode">
              {status()?.["shadow_only"] ? "shadow" : "enforcing"}
            </span>
          </div>
          <div class="resource-dashboard-row">
            <span>epoch {String(status()?.["epoch"] ?? "—")}</span>
            <span>shadow gate: {status()?.["shadow_gate_passes"] ? "clean ✓" : "diverged ✗"}</span>
          </div>
          <Show when={metrics()}>
            <div class="resource-dashboard-metrics">
              <span>granted {String(metrics()?.["granted"] ?? 0)}</span>
              <span>busy {String(metrics()?.["busy"] ?? 0)}</span>
              <span>shed {String(metrics()?.["shed"] ?? 0)}</span>
              <span>preempt {String(metrics()?.["preemptions"] ?? 0)}</span>
              <span>swaps {String(metrics()?.["swaps"] ?? 0)}</span>
              <span>foreground safe: {metrics()?.["foreground_invariant_ok"] ? "✓" : "✗"}</span>
            </div>
          </Show>
          {/* Live device table (from diagnostics bundle). */}
          <Show
            when={devices().length > 0}
            fallback={<div class="resource-view-note">No GPU devices detected (CPU placement).</div>}
          >
            <div class="resource-device-list">
              <For each={devices()}>
                {(d) => (
                  <div class="resource-device-row">
                    <span class="resource-device-id">{String(d["id"])}</span>
                    <span>
                      free {String(d["effective_free_vram_mb"])}/{String(d["total_vram_mb"])} MB
                    </span>
                    <span>hard {String(d["hard_limit_mb"])} MB</span>
                    <span>{String(d["health"])}</span>
                  </div>
                )}
              </For>
            </div>
          </Show>
          <Show when={telemetry()}>
            <div class="resource-view-note">
              telemetry: {String(telemetry()?.["source"])} · seq {String(telemetry()?.["seq"] ?? "—")}{" "}
              · CPU {String(telemetry()?.["cpu_avg_pct"] ?? "—")}% ({String(telemetry()?.["cpu_cores"] ?? "—")} cores){" "}
              · RAM free {String(telemetry()?.["ram_free_mb"] ?? "—")}/
              {String(telemetry()?.["ram_total_mb"] ?? "—")} MB
            </div>
          </Show>
        </div>

        {/* View 1b — Resource Pressure (derived from live bands) */}
        <div class="resource-view">
          <div class="resource-view-title">Resource Pressure</div>
          <Show
            when={devices().length > 0}
            fallback={<div class="resource-view-note">No GPU — CPU/cloud placement only.</div>}
          >
            <For each={devices()}>
              {(d) => {
                const free = Number(d["effective_free_vram_mb"] ?? 0);
                const soft = Number(d["soft_limit_mb"] ?? 0);
                const hard = Number(d["hard_limit_mb"] ?? 0);
                const emer = Number(d["emergency_limit_mb"] ?? 0);
                const level = () =>
                  free <= emer ? "emergency" : free <= hard ? "hard" : free <= soft ? "soft" : "ok";
                return (
                  <div class="resource-device-row">
                    <span class="resource-device-id">{String(d["id"])}</span>
                    <span class={`resource-pressure resource-pressure-${level()}`}>{level()}</span>
                    <span>free {free} MB</span>
                    <span>soft {soft} · hard {hard} · emerg {emer}</span>
                  </div>
                );
              }}
            </For>
          </Show>
        </div>

        {/* View 2 — Explainability */}
        <div class="resource-view">
          <div class="resource-view-title">Explainability — recent decisions</div>
          <Show
            when={decisions().length > 0}
            fallback={
              <div class="resource-view-note">
                No decisions journaled yet. Every placement/eviction/recovery is recorded here with a
                plain-language reason once the authority makes a call.
              </div>
            }
          >
            <div class="resource-device-list">
              <For each={decisions().slice().reverse()}>
                {(d) => (
                  <div class="resource-device-row">
                    <span class="resource-device-id">{String(d["kind"])}</span>
                    <span>{String(d["detail"])}</span>
                    <span class="resource-view-note">{String(d["why"])}</span>
                  </div>
                )}
              </For>
            </div>
          </Show>
        </div>

        {/* View 3 — Session Awareness (co-residency) */}
        <div class="resource-view">
          <div class="resource-view-title">Session Awareness — Co-Residency</div>
          <div class="resource-view-note">
            Active policy profile: <strong>{String(diag()?.["profile"] ?? "—")}</strong>.
            <Show when={coResidency()}>
              {" "}
              preemptions {String(coResidency()?.["preemptions"] ?? 0)} · dedup{" "}
              {String(coResidency()?.["dedup_hits"] ?? 0)} · rollbacks{" "}
              {String(coResidency()?.["rollbacks"] ?? 0)}
            </Show>
          </div>
          <Show
            when={residents().length > 0}
            fallback={
              <div class="resource-view-note">
                No models currently co-resident on GPU.
              </div>
            }
          >
            <div class="resource-device-list">
              <For each={residents()}>
                {(r) => (
                  <div class="resource-device-row">
                    <span class="resource-device-id">{String(r["model"])}</span>
                    <span>{String(r["class"])}</span>
                    <span>{String(r["device"])}</span>
                    <span>refs {String(r["refs"])}</span>
                    <Show when={r["pinned"]}>
                      <span title="anti-thrash pin active">pinned</span>
                    </Show>
                  </div>
                )}
              </For>
            </div>
          </Show>
        </div>

        {/* View 4 — Forecasting */}
        <div class="resource-view">
          <div class="resource-view-title">Forecasting</div>
          <Show
            when={forecast()}
            fallback={<div class="resource-view-note">Collecting telemetry to build a forecast…</div>}
          >
            {(() => {
              const tte = forecast()?.["time_to_exhaustion_s"];
              const conf = Number(forecast()?.["confidence"] ?? 0);
              return (
                <div class="resource-view-note">
                  VRAM exhaustion forecast:{" "}
                  {tte == null ? (
                    <strong>stable — not trending toward exhaustion</strong>
                  ) : (
                    <strong>~{Math.round(Number(tte))}s to exhaustion</strong>
                  )}{" "}
                  · confidence {(conf * 100).toFixed(0)}%
                </div>
              );
            })()}
          </Show>
        </div>

        {/* View 5 — Recovery */}
        <div class="resource-view">
          <div class="resource-view-title">Recovery</div>
          <div class="resource-view-note">
            Authority epoch {String(status()?.["epoch"] ?? "—")} (bumps fence prior-instance leases).
          </div>
          <Show
            when={recovered().length > 0}
            fallback={
              <div class="resource-view-note">
                No orphaned leases recovered from the journal — clean boot.
              </div>
            }
          >
            <div class="resource-device-list">
              <For each={recovered()}>
                {(l) => (
                  <div class="resource-device-row">
                    <span>recovered lease #{String(l["token"])}</span>
                    <span>{String(l["device"])}</span>
                    <span>{String(l["vram_mb"])} MB</span>
                  </div>
                )}
              </For>
            </div>
          </Show>
        </div>

        {/* View 6 — Diagnostics export */}
        <div class="resource-view">
          <div class="resource-view-title">Diagnostics</div>
          <button type="button" class="resource-diag-btn" onClick={exportDiagnostics}>
            Export diagnostics bundle (JSON)
          </button>
        </div>
      </Show>
    </div>
  );
};

export default ResourceDashboard;
