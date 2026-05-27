import { Component, For, Show, createSignal, onCleanup, onMount } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

interface N8nStatusPayload {
  enabled: boolean;
  base_url: string;
  callback_url: string;
  configured_workflows: any[];
  runs: any[];
  dead_letters: any[];
  governance_log: any[];
  hitl_responses: Record<string, any>;
  inbox_path: string;
  audit_path: string;
}

const N8nDashboard: Component = () => {
  const [status, setStatus] = createSignal<N8nStatusPayload | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  const [busy, setBusy] = createSignal(false);
  const [discovery, setDiscovery] = createSignal<any | null>(null);

  const refresh = async () => {
    setBusy(true);
    try {
      const result = await invoke<N8nStatusPayload>("get_n8n_status");
      setStatus(result);
      setError(null);
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  };

  const discover = async () => {
    setBusy(true);
    try {
      const result = await invoke<any>("discover_n8n_workflows");
      setDiscovery(result);
      setError(null);
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  };

  const reconcile = async (correlationId: string) => {
    setBusy(true);
    try {
      await invoke("reconcile_n8n_run", { correlationId });
      await refresh();
      setError(null);
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  };

  onMount(async () => {
    await refresh();
    const unlisten = await listen("n8n:callback", () => {
      void refresh();
    });
    const interval = setInterval(() => void refresh(), 5000);
    onCleanup(() => {
      unlisten();
      clearInterval(interval);
    });
  });

  return (
    <section class="ironclad-strip">
      <div class="ironclad-strip-top">
        <div class="ironclad-strip-title">
          <span>n8n Integration</span>
          <span class="ironclad-strip-subtitle">Bounded external workflow substrate</span>
        </div>
        <div class="ironclad-strip-actions">
          <button class="btn-secondary" disabled={busy()} onClick={() => void refresh()}>
            Refresh
          </button>
          <button class="btn-secondary" disabled={busy() || !status()?.enabled} onClick={() => void discover()}>
            Discover
          </button>
        </div>
      </div>

      <Show when={error()}>
        <div class="startup-warning-banner">
          <strong>n8n:</strong> {error()}
        </div>
      </Show>

      <Show when={status()} fallback={<div class="status-pill subtle">Loading n8n status…</div>}>
        {(s) => (
          <>
            <div class="ironclad-metric-row">
              <div class="ironclad-card">
                <div class="ironclad-card-label">Runtime</div>
                <div class="ironclad-chip-row">
                  <span class={`ironclad-chip ${s().enabled ? "" : "warn"}`}>
                    {s().enabled ? "Enabled" : "Disabled"}
                  </span>
                  <span class="ironclad-chip">Workflows {s().configured_workflows.length}</span>
                  <span class="ironclad-chip">Runs {s().runs.length}</span>
                  <span class="ironclad-chip">Governance {s().governance_log?.length ?? 0}</span>
                  <span class={`ironclad-chip ${s().dead_letters.length > 0 ? "warn" : ""}`}>
                    Dead letters {s().dead_letters.length}
                  </span>
                </div>
                <div class="ironclad-muted">Base URL: {s().base_url || "not configured"}</div>
                <div class="ironclad-muted">Callback URL: {s().callback_url}</div>
              </div>

              <div class="ironclad-card">
                <div class="ironclad-card-label">Authority</div>
                <div class="ironclad-muted">
                  KRIA invokes only configured workflows. n8n callback data is evidence;
                  KRIA verification still decides completion.
                </div>
                <div class="ironclad-muted">Inbox: {s().inbox_path}</div>
                <div class="ironclad-muted">Audit: {s().audit_path}</div>
              </div>
            </div>

            <div class="ironclad-forensics-panel">
              <div class="ironclad-forensics-head">
                <strong>Configured Workflows</strong>
                <span>{s().configured_workflows.length}</span>
              </div>
              <Show when={s().configured_workflows.length > 0} fallback={<div class="ironclad-muted">No n8n workflows configured.</div>}>
                <For each={s().configured_workflows}>
                  {(workflow) => (
                    <div class="ironclad-forensic-entry">
                      <div class="ironclad-forensic-summary">
                        <span class={`ironclad-severity ${workflow.status === "approved" ? "info" : "warn"}`}>
                          {workflow.status}
                        </span>
                        <span>{workflow.display_name || workflow.workflow_id}</span>
                      </div>
                      <div class="ironclad-forensic-meta">
                        <span>{workflow.workflow_id}</span>
                        <span>{workflow.workflow_version}</span>
                        <span>{workflow.endpoint_path}</span>
                      </div>
                    </div>
                  )}
                </For>
              </Show>
            </div>

            <div class="ironclad-forensics-panel">
              <div class="ironclad-forensics-head">
                <strong>Workflow Runs</strong>
                <span>{s().runs.length}</span>
              </div>
              <Show when={s().runs.length > 0} fallback={<div class="ironclad-muted">No callback runs ingested yet.</div>}>
                <For each={s().runs}>
                  {(run) => (
                    <div class="ironclad-forensic-entry">
                      <div class="ironclad-forensic-summary">
                        <span class={`ironclad-severity ${run.terminal ? "info" : "warn"}`}>
                          {run.status}
                        </span>
                        <span>{run.workflow_id}</span>
                      </div>
                      <div class="ironclad-forensic-meta">
                        <span>{run.correlation_id}</span>
                        <span>seq {run.last_sequence_number}</span>
                        <span>evidence {run.evidence_log?.length ?? 0}</span>
                        <button class="btn-secondary" disabled={busy()} onClick={() => void reconcile(run.correlation_id)}>
                          Reconcile
                        </button>
                      </div>
                    </div>
                  )}
                </For>
              </Show>
            </div>

            <div class="ironclad-forensics-panel">
              <div class="ironclad-forensics-head">
                <strong>Governance Decisions</strong>
                <span>{s().governance_log?.length ?? 0}</span>
              </div>
              <Show when={(s().governance_log?.length ?? 0) > 0} fallback={<div class="ironclad-muted">No governance decisions yet.</div>}>
                <For each={s().governance_log ?? []}>
                  {(decision) => (
                    <div class="ironclad-forensic-entry">
                      <div class="ironclad-forensic-summary">
                        <span class={`ironclad-severity ${decision.verification_status === "verified" ? "info" : "warn"}`}>
                          {decision.continuation_action}
                        </span>
                        <span>{decision.workflow_id}</span>
                      </div>
                      <div class="ironclad-forensic-meta">
                        <span>{decision.correlation_id}</span>
                        <span>{decision.verification_status}</span>
                        <span>{decision.run_status}</span>
                      </div>
                      <div class="ironclad-muted">{decision.explanation}</div>
                    </div>
                  )}
                </For>
              </Show>
            </div>

            <Show when={discovery()}>
              <div class="ironclad-forensics-panel">
                <div class="ironclad-forensics-head">
                  <strong>Discovery Result</strong>
                  <span>read-only</span>
                </div>
                <pre>{JSON.stringify(discovery(), null, 2)}</pre>
              </div>
            </Show>
          </>
        )}
      </Show>
    </section>
  );
};

export default N8nDashboard;
