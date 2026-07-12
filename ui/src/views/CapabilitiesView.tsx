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

interface CppQuarantineView {
  provider_id: string;
  capability_id: string;
  reason: string;
}

interface CppHealthView {
  provider_id: string;
  capability_id: string;
  family: string;
  status: string;
  success_rate: number | null;
  total: number;
  consecutive_failures: number;
  last_failure: string | null;
}

interface CppProposalView {
  id: string;
  kind: string;
  provider_id: string;
  capability_id: string;
  replacement: [string, string] | null;
  rationale: string;
  confidence: number;
  requires_approval: boolean;
  status: string;
  created_at: string;
}

type Tab = "providers" | "browser" | "marketplace" | "generate" | "discovery" | "jobs" | "quarantine" | "evolution" | "approvals" | "timeline";

// Wave 11: a durable job record (Execution Monitor).
interface CppJob {
  id: string;
  provider_id: string;
  capability_id: string;
  state: string;
  attempts: number;
  priority: number;
  created_at: string;
  updated_at: string;
  last_error: string | null;
}

// Wave 10: continuous discovery loop status.
interface CppDiscoveryStatus {
  enabled: boolean;
  running: boolean;
  total_scans: number;
  last_scan_at: string | null;
  next_scan_at: string | null;
  last_scan_findings: number;
  last_scan_skipped_quiet: boolean;
  pending_proposals: number;
  consecutive_errors: number;
  last_error: string | null;
}

// Wave 9 (W9-R12): a dry-run preview of the capability KRIA would synthesize.
interface CppSynthesisPreview {
  synthesizable: boolean;
  capability_id: string | null;
  name: string | null;
  pipeline: string[];
  node_count: number;
  ir_hash: string | null;
  golden_input: string | null;
  golden_output: string | null;
  message: string | null;
}

const CapabilitiesView: Component = () => {
  const [tab, setTab] = createSignal<Tab>("browser");
  const [status, setStatus] = createSignal<CppStatus | null>(null);
  const [providers, setProviders] = createSignal<CppProviderView[]>([]);
  const [caps, setCaps] = createSignal<CppCapabilityView[]>([]);
  const [recs, setRecs] = createSignal<CppCapabilityView[]>([]);
  const [quarantined, setQuarantined] = createSignal<CppQuarantineView[]>([]);
  const [health, setHealth] = createSignal<CppHealthView[]>([]);
  const [proposals, setProposals] = createSignal<CppProposalView[]>([]);
  const [autonomy, setAutonomy] = createSignal<string>("propose_only");
  const [grants, setGrants] = createSignal<CppGrantView[]>([]);
  const [events, setEvents] = createSignal<CppEventView[]>([]);
  const [query, setQuery] = createSignal("");
  const [mktQuery, setMktQuery] = createSignal("");
  // Generate (synthesis) tab state.
  const [synGoal, setSynGoal] = createSignal("");
  const [synPreview, setSynPreview] = createSignal<CppSynthesisPreview | null>(null);
  const [synResult, setSynResult] = createSignal<CppCapabilityView | null>(null);
  const [synBusy, setSynBusy] = createSignal(false);
  const [synLog, setSynLog] = createSignal<CppEventView[]>([]);
  // Discovery tab state.
  const [discovery, setDiscovery] = createSignal<CppDiscoveryStatus | null>(null);
  const [scanning, setScanning] = createSignal(false);
  // Jobs tab state.
  const [jobs, setJobs] = createSignal<CppJob[]>([]);
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

  const loadQuarantine = async () => {
    setError(null);
    try {
      setQuarantined(await invoke<CppQuarantineView[]>("cpp_quarantined"));
    } catch (e: unknown) {
      setError(String(e));
    }
  };

  const releaseQuarantine = async (providerId: string, capabilityId: string) => {
    try {
      await invoke<boolean>("cpp_release_quarantine", {
        providerId,
        capabilityId,
      });
      await loadQuarantine();
    } catch (e: unknown) {
      setError(String(e));
    }
  };

  const loadEvolution = async () => {
    setError(null);
    try {
      setHealth(await invoke<CppHealthView[]>("cpp_health"));
      setProposals(await invoke<CppProposalView[]>("cpp_proposals"));
      setAutonomy(await invoke<string>("cpp_get_autonomy"));
    } catch (e: unknown) {
      setError(String(e));
    }
  };

  const applyProposal = async (id: string) => {
    try {
      await invoke<CppProposalView>("cpp_proposal_apply", { id });
      await loadEvolution();
    } catch (e: unknown) {
      setError(String(e));
    }
  };

  const undoProposal = async (id: string) => {
    try {
      await invoke<CppProposalView>("cpp_proposal_undo", { id });
      await loadEvolution();
    } catch (e: unknown) {
      setError(String(e));
    }
  };

  const changeAutonomy = async (level: string) => {
    try {
      setAutonomy(await invoke<string>("cpp_set_autonomy", { level }));
    } catch (e: unknown) {
      setError(String(e));
    }
  };

  // Wave 9 (W9-R12): preview then synthesize a capability from a goal.
  const previewSynthesis = async () => {
    const g = synGoal().trim();
    setSynResult(null);
    if (!g) {
      setSynPreview(null);
      return;
    }
    try {
      setSynPreview(await invoke<CppSynthesisPreview>("cpp_synthesis_preview", { goal: g }));
    } catch (e: unknown) {
      setError(String(e));
    }
  };

  const doSynthesize = async () => {
    const g = synGoal().trim();
    if (!g) return;
    setSynBusy(true);
    setError(null);
    try {
      const cap = await invoke<CppCapabilityView>("cpp_synthesize", { goal: g });
      setSynResult(cap);
      // Pull the granular synthesis timeline (progress + logs) as evidence.
      try {
        const events = await invoke<CppEventView[]>("cpp_timeline", { limit: 100 });
        setSynLog(
          events.filter(
            (e) =>
              e.stage === "synthesize" ||
              (e.capability_id === cap.capability_id &&
                (e.stage === "acquire" || e.stage === "execute" || e.stage === "failure")),
          ),
        );
      } catch {
        /* timeline is best-effort */
      }
      await refresh();
    } catch (e: unknown) {
      setError(String(e));
    } finally {
      setSynBusy(false);
    }
  };

  // Wave 10: discovery dashboard.
  const loadDiscovery = async () => {
    try {
      setDiscovery(await invoke<CppDiscoveryStatus>("cpp_discovery_status"));
    } catch (e: unknown) {
      setError(String(e));
    }
  };
  const runDiscoveryScan = async () => {
    setScanning(true);
    setError(null);
    try {
      await invoke("cpp_discovery_scan");
      await loadDiscovery();
    } catch (e: unknown) {
      setError(String(e));
    } finally {
      setScanning(false);
    }
  };

  // Wave 11: execution monitor.
  const loadJobs = async () => {
    try {
      setJobs(await invoke<CppJob[]>("cpp_jobs", { limit: 200 }));
    } catch (e: unknown) {
      setError(String(e));
    }
  };
  const cancelJob = async (id: string) => {
    try {
      await invoke("cpp_job_control", { id, action: "cancel" });
      await loadJobs();
    } catch (e: unknown) {
      setError(String(e));
    }
  };

  const switchTab = (t: Tab) => {
    setTab(t);
    if (t === "jobs") void loadJobs();
    if (t === "discovery") void loadDiscovery();
    if (t === "evolution") void loadEvolution();
    if (t === "quarantine") void loadQuarantine();
    if (t === "approvals") void loadGrants();
    if (t === "timeline") void loadTimeline();
    if (t === "marketplace") void recommend();
  };

  onMount(() => {
    void refresh();
    // Poll the timeline while it is the active tab (push+poll reconcile).
    pollTimer = setInterval(() => {
      if (tab() === "timeline") void loadTimeline();
      if (tab() === "jobs") void loadJobs();
    }, 3000);
  });
  onCleanup(() => {
    if (pollTimer) clearInterval(pollTimer);
  });

  const tabs: { id: Tab; label: string }[] = [
    { id: "providers", label: "Providers" },
    { id: "browser", label: "Browser" },
    { id: "marketplace", label: "Marketplace" },
    { id: "generate", label: "Generate" },
    { id: "discovery", label: "Discovery" },
    { id: "jobs", label: "Execution Monitor" },
    { id: "quarantine", label: "Quarantine" },
    { id: "evolution", label: "Evolution" },
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

      {/* ── Quarantine (trust/integrity gate, R8.3) ───────────────── */}
      <Show when={tab() === "quarantine"}>
        <div style={{ display: "flex", "justify-content": "space-between", "align-items": "center", "margin-bottom": "8px" }}>
          <h3 style={{ margin: "0" }}>Quarantined capabilities</h3>
          <button type="button" class="btn-secondary" onClick={() => void loadQuarantine()}>Reload</button>
        </div>
        <p class="settings-hint">
          Capabilities that failed the Brain's trust / integrity gate on acquisition.
          They cannot execute until reviewed and released.
        </p>
        <Show when={quarantined().length === 0}><p class="settings-hint">Nothing quarantined.</p></Show>
        <div style={{ display: "flex", "flex-direction": "column", gap: "6px" }}>
          <For each={quarantined()}>
            {(q) => (
              <div style={{ padding: "8px", border: "1px solid #dc2626", "border-radius": "8px" }}>
                <div style={{ display: "flex", "align-items": "center", gap: "8px" }}>
                  <strong>{q.capability_id}</strong>
                  <span class="settings-hint">{q.provider_id}</span>
                  <button
                    type="button"
                    class="btn-secondary"
                    style={{ "margin-left": "auto" }}
                    onClick={() => void releaseQuarantine(q.provider_id, q.capability_id)}
                  >
                    Release
                  </button>
                </div>
                <div style={{ "font-size": "12px", color: "#b91c1c" }}>{q.reason}</div>
              </div>
            )}
          </For>
        </div>
      </Show>

      {/* ── Generate: synthesize a capability from a goal (Wave 9, R7/R27) ── */}
      <Show when={tab() === "generate"}>
        <h3 style={{ margin: "0 0 6px" }}>Generate a capability</h3>
        <p class="settings-hint">
          KRIA engineers a new capability from a goal by composing audited primitives into a
          validated Capability-Graph IR — no code generation. It runs at the lowest trust tier,
          must pass a golden smoke test before activation, and honestly declines goals it cannot
          express.
        </p>
        <div style={{ display: "flex", gap: "8px", "margin-bottom": "10px" }}>
          <input
            type="text"
            placeholder="e.g. trim then uppercase then reverse"
            value={synGoal()}
            onInput={(e) => setSynGoal(e.currentTarget.value)}
            onChange={() => void previewSynthesis()}
            style={{ flex: "1", padding: "6px 10px" }}
          />
          <button type="button" class="btn-secondary" onClick={() => void previewSynthesis()}>
            Preview
          </button>
          <button
            type="button"
            class="btn-primary"
            disabled={synBusy() || !(synPreview()?.synthesizable ?? false)}
            onClick={() => void doSynthesize()}
          >
            {synBusy() ? "Synthesizing…" : "Synthesize"}
          </button>
        </div>

        <Show when={synPreview()}>
          {(p) => (
            <div style={{ padding: "10px", border: "1px solid #e5e7eb", "border-radius": "8px", "margin-bottom": "10px" }}>
              <Show
                when={p().synthesizable}
                fallback={<p class="settings-hint" style={{ color: "#b45309" }}>{p().message}</p>}
              >
                <div style={{ display: "grid", "grid-template-columns": "auto 1fr", gap: "4px 12px", "font-size": "13px" }}>
                  <strong>Capability id</strong><span>{p().capability_id}</span>
                  <strong>Name</strong><span>{p().name}</span>
                  <strong>Pipeline</strong><span>{p().pipeline.join("  →  ")}</span>
                  <strong>IR nodes</strong><span>{p().node_count}</span>
                  <strong>IR hash</strong><span style={{ "font-family": "monospace" }}>{(p().ir_hash ?? "").slice(0, 16)}</span>
                  <strong>Golden case</strong><span>{JSON.stringify(p().golden_input)} → {JSON.stringify(p().golden_output)}</span>
                </div>
              </Show>
            </div>
          )}
        </Show>

        <Show when={synResult()}>
          {(r) => (
            <div style={{ padding: "10px", border: "1px solid #10b981", "border-radius": "8px" }}>
              <strong style={{ color: "#059669" }}>Synthesized + activated:</strong>{" "}
              {r().provider_id}/{r().capability_id}
              <p class="settings-hint" style={{ margin: "4px 0 0" }}>
                Now discoverable in the Browser tab and executable through the permission gate.
              </p>
            </div>
          )}
        </Show>

        {/* Synthesis progress / logs (granular capability:synthesize events). */}
        <Show when={synLog().length > 0}>
          <h4 style={{ margin: "12px 0 4px" }}>Synthesis log</h4>
          <div style={{ display: "flex", "flex-direction": "column", gap: "2px", "font-family": "monospace", "font-size": "12px" }}>
            <For each={synLog()}>
              {(e) => (
                <div style={{ display: "flex", gap: "8px", padding: "3px 6px", "border-radius": "4px", background: "#f8fafc" }}>
                  <span style={{ color: "#6b7280" }}>{e.timestamp.slice(11, 19)}</span>
                  <span style={{ "min-width": "80px" }}>{e.stage}</span>
                  <span style={{ color: outcomeColor[e.outcome] ?? "#6b7280", "min-width": "70px" }}>{e.outcome}</span>
                  <span style={{ color: "#4b5563" }}>{e.detail}</span>
                </div>
              )}
            </For>
          </div>
        </Show>
      </Show>

      {/* ── Execution Monitor: durable long-running jobs (Wave 11) ────────── */}
      <Show when={tab() === "jobs"}>
        <div style={{ display: "flex", "align-items": "center", gap: "8px", "margin-bottom": "10px" }}>
          <h3 style={{ margin: "0" }}>Execution Monitor</h3>
          <button type="button" class="btn-secondary" style={{ "margin-left": "auto" }} onClick={() => void loadJobs()}>Refresh</button>
        </div>
        <p class="settings-hint">
          Durable, resumable jobs run through the reliable execution path (timeout + bounded retry +
          cancellation). State survives restart. Live-updating.
        </p>
        <Show when={jobs().length === 0}><p class="settings-hint">No jobs.</p></Show>
        <For each={jobs()}>
          {(j) => (
            <div style={{ display: "flex", "align-items": "center", gap: "8px", padding: "6px", "border-bottom": "1px solid #f1f5f9", "font-size": "13px" }}>
              <span style={{ width: "10px", height: "10px", "border-radius": "50%", background: outcomeColor[j.state === "completed" || j.state === "recovered" ? "ok" : j.state === "failed" || j.state === "timed_out" ? "failed" : j.state === "cancelled" || j.state === "rolled_back" ? "declined" : "started"] || "#9ca3af" }} />
              <strong>{j.provider_id}/{j.capability_id}</strong>
              <span style={{ padding: "1px 6px", "border-radius": "4px", background: "#f1f5f9" }}>{j.state}</span>
              <span class="settings-hint">attempts {j.attempts}</span>
              <Show when={j.last_error}><span style={{ color: "#b45309" }}>{j.last_error}</span></Show>
              <span class="settings-hint" style={{ "margin-left": "auto" }}>{j.updated_at.slice(11, 19)}</span>
              <Show when={!["completed", "failed", "cancelled", "rolled_back"].includes(j.state)}>
                <button type="button" class="btn-secondary" onClick={() => void cancelJob(j.id)}>Cancel</button>
              </Show>
            </div>
          )}
        </For>
      </Show>

      {/* ── Discovery: continuous discovery/maintenance dashboard (Wave 10) ── */}
      <Show when={tab() === "discovery"}>
        <div style={{ display: "flex", "align-items": "center", gap: "8px", "margin-bottom": "10px" }}>
          <h3 style={{ margin: "0" }}>Continuous Discovery</h3>
          <button type="button" class="btn-secondary" style={{ "margin-left": "auto" }} onClick={() => void loadDiscovery()}>Refresh</button>
          <button type="button" class="btn-primary" disabled={scanning() || !(discovery()?.enabled ?? false)} onClick={() => void runDiscoveryScan()}>
            {scanning() ? "Scanning…" : "Scan now"}
          </button>
        </div>
        <p class="settings-hint">
          Background maintenance scans provider health + the marketplace and writes reversible
          proposals (Upgrade / Replace / Repair / Retire) to the Evolution feed under the autonomy
          level. Off unless <code>continuous_discovery</code> is enabled.
        </p>
        <Show when={discovery()} fallback={<p class="settings-hint">Loading…</p>}>
          {(s) => (
            <Show
              when={s().enabled}
              fallback={<p class="settings-hint" style={{ color: "#b45309" }}>Continuous discovery is disabled.</p>}
            >
              <div style={{ display: "grid", "grid-template-columns": "auto 1fr", gap: "4px 12px", "font-size": "13px", "max-width": "560px" }}>
                <strong>Running</strong><span>{s().running ? "yes (background loop active)" : "no"}</span>
                <strong>Total scans</strong><span>{s().total_scans}</span>
                <strong>Last scan</strong><span>{s().last_scan_at ?? "—"}{s().last_scan_skipped_quiet ? " (skipped: quiet hours)" : ""}</span>
                <strong>Next scan</strong><span>{s().next_scan_at ?? "—"}</span>
                <strong>Last findings</strong><span>{s().last_scan_findings}</span>
                <strong>Pending proposals</strong><span>{s().pending_proposals} (see Evolution tab)</span>
                <strong>Errors</strong><span>{s().consecutive_errors}{s().last_error ? ` · ${s().last_error}` : ""}</span>
              </div>
            </Show>
          )}
        </Show>
      </Show>

      {/* ── Evolution: health + proposals oversight (R6/R29) ──────── */}
      <Show when={tab() === "evolution"}>
        <div style={{ display: "flex", "align-items": "center", gap: "8px", "margin-bottom": "10px" }}>
          <h3 style={{ margin: "0" }}>Autonomy</h3>
          <select value={autonomy()} onChange={(e) => void changeAutonomy(e.currentTarget.value)}>
            <option value="manual">Manual</option>
            <option value="propose_only">Propose only</option>
            <option value="auto_with_notice">Auto with notice</option>
            <option value="full_auto">Full auto</option>
          </select>
          <button type="button" class="btn-secondary" style={{ "margin-left": "auto" }} onClick={() => void loadEvolution()}>Refresh</button>
        </div>

        <h3 style={{ margin: "10px 0 6px" }}>Evolution proposals</h3>
        <p class="settings-hint">Auditable, reversible proposals from capability health. Elevated actions require approval.</p>
        <Show when={proposals().length === 0}><p class="settings-hint">No proposals — ecosystem healthy.</p></Show>
        <div style={{ display: "flex", "flex-direction": "column", gap: "6px" }}>
          <For each={proposals()}>
            {(p) => (
              <div style={{ padding: "8px", border: "1px solid #cbd5e1", "border-radius": "8px" }}>
                <div style={{ display: "flex", "align-items": "center", gap: "8px" }}>
                  <strong style={{ "text-transform": "uppercase", "font-size": "11px", color: "#2563eb" }}>{p.kind}</strong>
                  <span>{p.capability_id}</span>
                  <Show when={p.replacement}><span class="settings-hint">→ {p.replacement![1]}</span></Show>
                  <span style={{ "margin-left": "auto", "font-size": "11px", color: "#6b7280" }}>{p.status} · conf {(p.confidence * 100).toFixed(0)}%</span>
                </div>
                <div style={{ "font-size": "12px", color: "#4b5563", margin: "4px 0" }}>{p.rationale}</div>
                <Show when={p.status === "pending" || p.status === "approved"}>
                  <div style={{ display: "flex", gap: "6px" }}>
                    <button type="button" class="btn-secondary" onClick={() => void applyProposal(p.id)}>Apply</button>
                    <button type="button" class="btn-secondary" onClick={() => void undoProposal(p.id)}>Dismiss</button>
                  </div>
                </Show>
                <Show when={p.status === "applied"}>
                  <button type="button" class="btn-secondary" onClick={() => void undoProposal(p.id)}>Undo</button>
                </Show>
              </div>
            )}
          </For>
        </div>

        <h3 style={{ margin: "14px 0 6px" }}>Capability health</h3>
        <Show when={health().length === 0}><p class="settings-hint">No health data yet.</p></Show>
        <div style={{ display: "flex", "flex-direction": "column", gap: "4px" }}>
          <For each={health()}>
            {(h) => (
              <div style={{ display: "flex", "align-items": "center", gap: "8px", padding: "6px", "border-bottom": "1px solid #f1f5f9" }}>
                <span style={{ width: "10px", height: "10px", "border-radius": "50%", background: outcomeColor[h.status === "healthy" ? "ok" : h.status === "warning" ? "declined" : h.status === "critical" || h.status === "quarantined" ? "failed" : "started"] || "#9ca3af" }} />
                <strong>{h.capability_id}</strong>
                <span class="settings-hint">{h.provider_id} · {h.family}</span>
                <span style={{ "margin-left": "auto", "font-size": "12px" }}>
                  {h.status}{h.success_rate !== null ? ` · ${(h.success_rate * 100).toFixed(0)}% (${h.total})` : ""}{h.consecutive_failures > 0 ? ` · ${h.consecutive_failures}✗` : ""}
                </span>
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
