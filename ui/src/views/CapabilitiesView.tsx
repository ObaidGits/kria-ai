// First-class Capabilities area (CPP / Milestone 9).
//
// Provider-neutral desktop surface backed by the `cpp_*` Tauri commands
// (crates/kria-desktop/src/commands/capability.rs). It is organised into tabs:
//   - Providers          : live provider list + health + negotiated version.
//   - Browser            : federated catalog across ALL providers + goal search;
//                          each capability can be inspected (Descriptor Viewer)
//                          and run through the permission gate (Approval Modal).
//   - Marketplace        : installable, not-yet-installed recommendations.
//   - Approval Center    : durable permission grants list + revoke.
//   - Timeline           : the observability event feed (also serves Runtime
//                          Monitor + Recovery — recover/failure stages surface
//                          here with distinct colouring).
//
// No provider is named or special-cased in the UI — everything is driven by the
// descriptors and events the backend federates.

import { Component, createSignal, For, Show, onMount, onCleanup } from "solid-js";
import { invoke } from "@tauri-apps/api/core";

interface CppStatus {
  enabled: boolean;
  provider_count: number;
  healthy_providers: number;
  descriptor_count: number;
}

interface CppProviderView {
  provider_id: string;
  health: string;
  state: string;
  version: string | null;
  descriptor_count: number;
  error: string | null;
}

interface CppCapabilityView {
  provider_id: string;
  capability_id: string;
  name: string;
  description: string;
  tags: string[];
  elevated: boolean;
  score: number;
}

interface CppGrantView {
  grant_id: string;
  provider_id: string;
  capability_id: string;
  scope: string;
  scope_key: string | null;
  effects: string[];
  decision: string;
  granted_at: string;
  expires_at: string | null;
}

interface CppAuthDecision {
  kind: string; // allow | prompt | deny
  tier: string;
  effects: string[];
  risk: string | null;
  reason: string | null;
  grant_id: string | null;
}

interface CppExecuteResult {
  status: string; // ok | declined | needs_approval | denied
  decision: CppAuthDecision | null;
  value: unknown | null;
  reason: string | null;
}

interface CppEventView {
  correlation_id: string;
  provider_id: string;
  capability_id: string | null;
  stage: string;
  outcome: string;
  detail: string;
  timestamp: string;
}

interface CppDescriptorView {
  provider_id: string;
  capability_id: string;
  name: string;
  description: string;
  version: string;
  schema_version: string;
  tags: string[];
  io_modality: string[];
  inputs: string[];
  outputs: string[];
  effect_classes: string[];
  reversible: string;
  idempotent: boolean;
  elevated: boolean;
  trust_tier: string | null;
  signed: boolean;
  guidance: unknown | null;
  expectations: unknown | null;
  input_schema: unknown;
}

const healthColor: Record<string, string> = {
  ready: "#16a34a",
  healthy: "#16a34a",
  busy: "#2563eb",
  degraded: "#d97706",
  offline: "#dc2626",
};

const outcomeColor: Record<string, string> = {
  started: "#2563eb",
  ok: "#16a34a",
  declined: "#d97706",
  failed: "#dc2626",
  degraded: "#d97706",
};

type Tab = "providers" | "browser" | "marketplace" | "approvals" | "timeline";

const CapabilitiesView: Component = () => {
  const [tab, setTab] = createSignal<Tab>("browser");
  const [status, setStatus] = createSignal<CppStatus | null>(null);
  const [providers, setProviders] = createSignal<CppProviderView[]>([]);
  const [caps, setCaps] = createSignal<CppCapabilityView[]>([]);
  const [recs, setRecs] = createSignal<CppCapabilityView[]>([]);
  const [grants, setGrants] = createSignal<CppGrantView[]>([]);
  const [events, setEvents] = createSignal<CppEventView[]>([]);
  const [query, setQuery] = createSignal("");
  const [mktQuery, setMktQuery] = createSignal("");
  const [loading, setLoading] = createSignal(true);
  const [error, setError] = createSignal<string | null>(null);

  // Descriptor Viewer modal.
  const [viewing, setViewing] = createSignal<CppDescriptorView | null>(null);
  // Approval modal state.
  const [approving, setApproving] = createSignal<{
    cap: CppCapabilityView;
    decision: CppAuthDecision;
    args: string;
  } | null>(null);
  const [runResult, setRunResult] = createSignal<{ cap: string; result: CppExecuteResult } | null>(null);

  let pollTimer: ReturnType<typeof setInterval> | undefined;

  const refresh = async () => {
    setLoading(true);
    setError(null);
    try {
      const [st, provs, catalog] = await Promise.all([
        invoke<CppStatus>("cpp_status"),
        invoke<CppProviderView[]>("cpp_list_providers"),
        invoke<CppCapabilityView[]>("cpp_catalog"),
      ]);
      setStatus(st);
      setProviders(provs);
      setCaps(catalog);
    } catch (e: unknown) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  };

  const search = async () => {
    const q = query().trim();
    if (!q) {
      await refresh();
      return;
    }
    setLoading(true);
    setError(null);
    try {
      setCaps(await invoke<CppCapabilityView[]>("cpp_discover", { query: q, k: 25 }));
    } catch (e: unknown) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  };

  const recommend = async () => {
    setError(null);
    try {
      setRecs(await invoke<CppCapabilityView[]>("cpp_recommend", { query: mktQuery().trim(), k: 15 }));
    } catch (e: unknown) {
      setError(String(e));
    }
  };

  const loadGrants = async () => {
    setError(null);
    try {
      setGrants(await invoke<CppGrantView[]>("cpp_list_grants"));
    } catch (e: unknown) {
      setError(String(e));
    }
  };

  const revokeGrant = async (grantId: string) => {
    try {
      await invoke("cpp_revoke_grant", { grantId });
      await loadGrants();
    } catch (e: unknown) {
      setError(String(e));
    }
  };

  const loadTimeline = async () => {
    try {
      setEvents(await invoke<CppEventView[]>("cpp_timeline", { limit: 300 }));
    } catch (e: unknown) {
      setError(String(e));
    }
  };

  const inspect = async (c: CppCapabilityView) => {
    try {
      setViewing(
        await invoke<CppDescriptorView>("cpp_descriptor", {
          providerId: c.provider_id,
          capabilityId: c.capability_id,
        }),
      );
    } catch (e: unknown) {
      setError(String(e));
    }
  };

  // Run a capability through the permission gate. On needs_approval, raise the
  // approval modal; otherwise show the result.
  const run = async (c: CppCapabilityView, argsJson: string) => {
    setError(null);
    let args: unknown = {};
    if (argsJson.trim()) {
      try {
        args = JSON.parse(argsJson);
      } catch {
        setError("Arguments must be valid JSON.");
        return;
      }
    }
    try {
      const res = await invoke<CppExecuteResult>("cpp_execute", {
        providerId: c.provider_id,
        capabilityId: c.capability_id,
        args,
      });
      if (res.status === "needs_approval" && res.decision) {
        setApproving({ cap: c, decision: res.decision, args: argsJson });
      } else {
        setRunResult({ cap: `${c.provider_id}/${c.capability_id}`, result: res });
        void loadTimeline();
      }
    } catch (e: unknown) {
      setError(String(e));
    }
  };

  // Approve (or deny) at a scope, then re-run on approve.
  const decide = async (scope: string, allow: boolean) => {
    const a = approving();
    if (!a) return;
    try {
      await invoke("cpp_approve", {
        providerId: a.cap.provider_id,
        capabilityId: a.cap.capability_id,
        scope,
        allow,
      });
      const cap = a.cap;
      const args = a.args;
      setApproving(null);
      if (allow) {
        await run(cap, args);
      } else {
        await loadGrants();
      }
    } catch (e: unknown) {
      setError(String(e));
    }
  };

  const switchTab = (t: Tab) => {
    setTab(t);
    if (t === "approvals") void loadGrants();
    if (t === "timeline") void loadTimeline();
    if (t === "marketplace") void recommend();
  };

  onMount(() => {
    void refresh();
    // Poll the timeline while it is the active tab (push+poll reconcile).
    pollTimer = setInterval(() => {
      if (tab() === "timeline") void loadTimeline();
    }, 3000);
  });
  onCleanup(() => {
    if (pollTimer) clearInterval(pollTimer);
  });

  const tabs: { id: Tab; label: string }[] = [
    { id: "providers", label: "Providers" },
    { id: "browser", label: "Browser" },
    { id: "marketplace", label: "Marketplace" },
    { id: "approvals", label: "Approval Center" },
    { id: "timeline", label: "Timeline" },
  ];

  return (
    <section class="ironclad-strip capabilities-view" style={{ padding: "16px" }}>
      <div class="ironclad-strip-top">
        <div class="ironclad-strip-title">
          <span>Capabilities</span>
          <span class="ironclad-strip-subtitle">Provider-neutral capability platform (CPP)</span>
        </div>
        <div class="ironclad-strip-actions">
          <button type="button" class="btn-secondary" disabled={loading()} onClick={() => void refresh()}>
            {loading() ? "Loading…" : "Refresh"}
          </button>
        </div>
      </div>

      <Show when={error()}>
        <div class="startup-warning-banner"><strong>Error:</strong> {error()}</div>
      </Show>

      <Show when={status()}>
        {(s) => (
          <div style={{ display: "flex", gap: "16px", "flex-wrap": "wrap", margin: "8px 0 12px" }}>
            <div class="settings-hint">CPP flag: <strong>{s().enabled ? "ON" : "OFF"}</strong></div>
            <div class="settings-hint">Providers: <strong>{s().healthy_providers}/{s().provider_count}</strong></div>
            <div class="settings-hint">Capabilities: <strong>{s().descriptor_count}</strong></div>
          </div>
        )}
      </Show>

      {/* Tab bar */}
      <div style={{ display: "flex", gap: "6px", "margin-bottom": "14px", "border-bottom": "1px solid #e5e7eb" }}>
        <For each={tabs}>
          {(t) => (
            <button
              type="button"
              class="btn-secondary"
              onClick={() => switchTab(t.id)}
              style={{
                "border-bottom": tab() === t.id ? "2px solid #2563eb" : "2px solid transparent",
                "font-weight": tab() === t.id ? "600" : "400",
              }}
            >
              {t.label}
            </button>
          )}
        </For>
      </div>

      {/* ── Providers ─────────────────────────────────────────────── */}
      <Show when={tab() === "providers"}>
        <Show when={providers().length === 0}>
          <p class="settings-hint">No providers registered.</p>
        </Show>
        <div style={{ display: "flex", "flex-direction": "column", gap: "6px" }}>
          <For each={providers()}>
            {(p) => (
              <div style={{ display: "flex", "align-items": "center", gap: "12px", padding: "8px", border: "1px solid #e5e7eb", "border-radius": "8px" }}>
                <span style={{ width: "10px", height: "10px", "border-radius": "50%", background: healthColor[p.health] ?? "#6b7280" }} />
                <strong style={{ "min-width": "160px" }}>{p.provider_id}</strong>
                <span class="settings-hint">{p.state}</span>
                <span class="settings-hint">v{p.version ?? "?"}</span>
                <span class="settings-hint">{p.descriptor_count} capabilities</span>
                <Show when={p.error}><span style={{ color: "#dc2626" }}>{p.error}</span></Show>
              </div>
            )}
          </For>
        </div>
      </Show>

      {/* ── Browser ───────────────────────────────────────────────── */}
      <Show when={tab() === "browser"}>
        <div style={{ display: "flex", gap: "8px", "margin-bottom": "12px" }}>
          <input
            type="text"
            placeholder="Describe a goal to discover capabilities across all providers…"
            value={query()}
            onInput={(e) => setQuery(e.currentTarget.value)}
            onKeyDown={(e) => { if (e.key === "Enter") void search(); }}
            style={{ flex: "1", padding: "8px" }}
          />
          <button type="button" class="btn-secondary" onClick={() => void search()}>Discover</button>
        </div>
        <div style={{ display: "flex", "flex-direction": "column", gap: "6px" }}>
          <For each={caps()}>
            {(c) => <CapabilityRow cap={c} onInspect={() => void inspect(c)} onRun={(a) => void run(c, a)} />}
          </For>
        </div>
      </Show>

      {/* ── Marketplace ───────────────────────────────────────────── */}
      <Show when={tab() === "marketplace"}>
        <div style={{ display: "flex", gap: "8px", "margin-bottom": "12px" }}>
          <input
            type="text"
            placeholder="What capability do you need? (installable recommendations)"
            value={mktQuery()}
            onInput={(e) => setMktQuery(e.currentTarget.value)}
            onKeyDown={(e) => { if (e.key === "Enter") void recommend(); }}
            style={{ flex: "1", padding: "8px" }}
          />
          <button type="button" class="btn-secondary" onClick={() => void recommend()}>Recommend</button>
        </div>
        <Show when={recs().length === 0}><p class="settings-hint">No installable recommendations.</p></Show>
        <div style={{ display: "flex", "flex-direction": "column", gap: "6px" }}>
          <For each={recs()}>
            {(c) => (
              <div style={{ padding: "8px", border: "1px dashed #cbd5e1", "border-radius": "8px" }}>
                <div style={{ display: "flex", "align-items": "center", gap: "8px" }}>
                  <strong>{c.name}</strong>
                  <span class="settings-hint">{c.provider_id} / {c.capability_id}</span>
                  <span style={{ "margin-left": "auto", "font-size": "11px", color: "#6b7280" }}>installable</span>
                </div>
                <div style={{ "font-size": "12px", color: "#4b5563" }}>{c.description}</div>
              </div>
            )}
          </For>
        </div>
      </Show>

      {/* ── Approval Center ───────────────────────────────────────── */}
      <Show when={tab() === "approvals"}>
        <div style={{ display: "flex", "justify-content": "space-between", "align-items": "center", "margin-bottom": "8px" }}>
          <h3 style={{ margin: "0" }}>Active grants</h3>
          <button type="button" class="btn-secondary" onClick={() => void loadGrants()}>Reload</button>
        </div>
        <Show when={grants().length === 0}><p class="settings-hint">No active grants.</p></Show>
        <div style={{ display: "flex", "flex-direction": "column", gap: "6px" }}>
          <For each={grants()}>
            {(g) => (
              <div style={{ display: "flex", "align-items": "center", gap: "10px", padding: "8px", border: "1px solid #e5e7eb", "border-radius": "8px" }}>
                <span style={{ color: g.decision === "deny" ? "#dc2626" : "#16a34a", "font-weight": "600" }}>{g.decision}</span>
                <strong>{g.provider_id} / {g.capability_id}</strong>
                <span class="settings-hint">scope: {g.scope}{g.scope_key ? ` (${g.scope_key})` : ""}</span>
                <span class="settings-hint">{g.effects.join(", ") || "no effects"}</span>
                <button type="button" class="btn-secondary" style={{ "margin-left": "auto" }} onClick={() => void revokeGrant(g.grant_id)}>Revoke</button>
              </div>
            )}
          </For>
        </div>
      </Show>

      {/* ── Timeline / Runtime Monitor / Recovery ─────────────────── */}
      <Show when={tab() === "timeline"}>
        <div style={{ display: "flex", "justify-content": "space-between", "align-items": "center", "margin-bottom": "8px" }}>
          <h3 style={{ margin: "0" }}>Event timeline</h3>
          <button type="button" class="btn-secondary" onClick={() => void loadTimeline()}>Reload</button>
        </div>
        <Show when={events().length === 0}><p class="settings-hint">No events yet. Run a capability from the Browser.</p></Show>
        <div style={{ display: "flex", "flex-direction": "column", gap: "3px", "font-family": "monospace", "font-size": "12px" }}>
          <For each={events()}>
            {(e) => (
              <div style={{ display: "flex", gap: "8px", padding: "3px 6px", "border-radius": "4px", background: "#f8fafc" }}>
                <span style={{ color: "#6b7280" }}>{e.timestamp.slice(11, 19)}</span>
                <span style={{ "min-width": "80px" }}>{e.stage}</span>
                <span style={{ color: outcomeColor[e.outcome] ?? "#6b7280", "min-width": "70px" }}>{e.outcome}</span>
                <span style={{ color: "#2563eb" }}>{e.provider_id}</span>
                <span style={{ color: "#4b5563" }}>{e.capability_id ?? ""}</span>
                <span style={{ color: "#6b7280", "margin-left": "auto" }}>{e.detail}</span>
              </div>
            )}
          </For>
        </div>
      </Show>

      {/* ── Descriptor Viewer modal ───────────────────────────────── */}
      <Show when={viewing()}>
        {(d) => (
          <div class="modal-overlay" onClick={() => setViewing(null)}>
            <div class="modal-content" onClick={(e) => e.stopPropagation()} style={{ "max-width": "640px", "max-height": "80vh", overflow: "auto" }}>
              <h3>{d().name} <span class="settings-hint">({d().schema_version})</span></h3>
              <p class="settings-hint">{d().provider_id} / {d().capability_id} · v{d().version || "?"}</p>
              <p>{d().description}</p>
              <p><strong>Effects:</strong> {d().effect_classes.join(", ") || "none"} · reversible: {d().reversible} · idempotent: {String(d().idempotent)} {d().elevated ? "· ⚠ elevated" : ""}</p>
              <p><strong>I/O:</strong> in [{d().inputs.join(", ")}] → out [{d().outputs.join(", ")}] · modality [{d().io_modality.join(", ")}]</p>
              <p><strong>Trust:</strong> {d().trust_tier ?? "unknown"} {d().signed ? "· signed" : ""}</p>
              <Show when={d().tags.length > 0}><p><strong>Tags:</strong> {d().tags.join(" · ")}</p></Show>
              <details>
                <summary>Input schema</summary>
                <pre style={{ "white-space": "pre-wrap", "font-size": "11px" }}>{JSON.stringify(d().input_schema, null, 2)}</pre>
              </details>
              <Show when={d().guidance}>
                <details><summary>Guidance</summary><pre style={{ "white-space": "pre-wrap", "font-size": "11px" }}>{JSON.stringify(d().guidance, null, 2)}</pre></details>
              </Show>
              <Show when={d().expectations}>
                <details><summary>Expectations</summary><pre style={{ "white-space": "pre-wrap", "font-size": "11px" }}>{JSON.stringify(d().expectations, null, 2)}</pre></details>
              </Show>
              <div style={{ "text-align": "right", "margin-top": "12px" }}>
                <button type="button" class="btn-secondary" onClick={() => setViewing(null)}>Close</button>
              </div>
            </div>
          </div>
        )}
      </Show>

      {/* ── Approval modal ────────────────────────────────────────── */}
      <Show when={approving()}>
        {(a) => (
          <div class="modal-overlay">
            <div class="modal-content" onClick={(e) => e.stopPropagation()} style={{ "max-width": "480px" }}>
              <h3>Approval required</h3>
              <p><strong>{a().cap.name}</strong></p>
              <p class="settings-hint">{a().cap.provider_id} / {a().cap.capability_id}</p>
              <p>
                This capability requests effects:{" "}
                <strong>{a().decision.effects.join(", ") || "unknown"}</strong>
                {a().decision.risk ? ` · risk: ${a().decision.risk}` : ""}
              </p>
              <Show when={a().decision.reason}><p class="settings-hint">{a().decision.reason}</p></Show>
              <p class="settings-hint">Approve for:</p>
              <div style={{ display: "flex", gap: "6px", "flex-wrap": "wrap", "margin-bottom": "10px" }}>
                <button type="button" class="btn-primary" onClick={() => void decide("once", true)}>Once</button>
                <button type="button" class="btn-primary" onClick={() => void decide("session", true)}>This session</button>
                <button type="button" class="btn-primary" onClick={() => void decide("workspace", true)}>This workspace</button>
                <button type="button" class="btn-primary" onClick={() => void decide("persistent", true)}>Always</button>
              </div>
              <div style={{ display: "flex", gap: "6px", "justify-content": "space-between" }}>
                <button type="button" class="btn-secondary" style={{ color: "#dc2626" }} onClick={() => void decide("persistent", false)}>Deny (standing)</button>
                <button type="button" class="btn-secondary" onClick={() => setApproving(null)}>Cancel</button>
              </div>
            </div>
          </div>
        )}
      </Show>

      {/* ── Run result toast ──────────────────────────────────────── */}
      <Show when={runResult()}>
        {(r) => (
          <div class="modal-overlay" onClick={() => setRunResult(null)}>
            <div class="modal-content" onClick={(e) => e.stopPropagation()} style={{ "max-width": "560px", "max-height": "70vh", overflow: "auto" }}>
              <h3>Result — {r().result.status}</h3>
              <p class="settings-hint">{r().cap}</p>
              <Show when={r().result.reason}><p style={{ color: "#d97706" }}>{r().result.reason}</p></Show>
              <Show when={r().result.value !== null && r().result.value !== undefined}>
                <pre style={{ "white-space": "pre-wrap", "font-size": "12px" }}>{JSON.stringify(r().result.value, null, 2)}</pre>
              </Show>
              <div style={{ "text-align": "right", "margin-top": "12px" }}>
                <button type="button" class="btn-secondary" onClick={() => setRunResult(null)}>Close</button>
              </div>
            </div>
          </div>
        )}
      </Show>
    </section>
  );
};

// One capability row in the Browser, with inspect + inline run (args editor).
const CapabilityRow: Component<{
  cap: CppCapabilityView;
  onInspect: () => void;
  onRun: (argsJson: string) => void;
}> = (props) => {
  const [open, setOpen] = createSignal(false);
  const [args, setArgs] = createSignal("{}");
  return (
    <div style={{ padding: "8px", border: "1px solid #e5e7eb", "border-radius": "8px" }}>
      <div style={{ display: "flex", "align-items": "center", gap: "8px" }}>
        <strong>{props.cap.name}</strong>
        <span class="settings-hint">{props.cap.provider_id} / {props.cap.capability_id}</span>
        <Show when={props.cap.elevated}>
          <span style={{ color: "#d97706", "font-size": "11px" }}>⚠ needs approval</span>
        </Show>
        <span style={{ "margin-left": "auto", "font-size": "11px", color: "#6b7280" }}>{props.cap.score.toFixed(3)}</span>
        <button type="button" class="btn-secondary" onClick={props.onInspect}>Inspect</button>
        <button type="button" class="btn-secondary" onClick={() => setOpen(!open())}>Run</button>
      </div>
      <div style={{ "font-size": "12px", color: "#4b5563" }}>{props.cap.description}</div>
      <Show when={props.cap.tags.length > 0}>
        <div style={{ "font-size": "11px", color: "#6b7280", "margin-top": "2px" }}>{props.cap.tags.join(" · ")}</div>
      </Show>
      <Show when={open()}>
        <div style={{ "margin-top": "6px", display: "flex", gap: "6px" }}>
          <textarea
            value={args()}
            onInput={(e) => setArgs(e.currentTarget.value)}
            placeholder='{"expression": "3+3"}'
            style={{ flex: "1", "font-family": "monospace", "font-size": "12px", "min-height": "48px", padding: "6px" }}
          />
          <button type="button" class="btn-primary" onClick={() => props.onRun(args())}>Execute</button>
        </div>
      </Show>
    </div>
  );
};

export default CapabilitiesView;
