import { Component, For, Show } from "solid-js";
import type { N8nRuntimeStatusPayload, N8nStatusPayload } from "../stores/n8n";

interface Props {
  status: N8nStatusPayload | null;
  runtimeStatus?: N8nRuntimeStatusPayload | null;
}

const N8nDiagnosticsPanel: Component<Props> = (props) => {
  const status = () => props.status;
  const runtime = () => props.runtimeStatus;
  const connection = () => runtime()?.runtime?.last_connection;
  const container = () => runtime()?.runtime?.container;
  const readiness = () => status()?.stage3_readiness;

  return (
    <details class="n8n-diagnostics">
      <summary>Diagnostics</summary>
      <Show when={status()} fallback={<div class="n8n-empty">Diagnostics unavailable.</div>}>
        {(payload) => (
          <div class="n8n-diagnostics-grid">
            <div>
              <span>Runtime</span>
              <strong>{(runtime()?.enabled ?? payload().enabled) ? "Enabled" : "Disabled"}</strong>
              <small>{runtime()?.mode || payload().mode || "mode unknown"}</small>
            </div>
            <div>
              <span>Setup health</span>
              <strong>{connection()?.status || "untested"}</strong>
              <small>{connection()?.message || "Run Test Connection from n8n settings."}</small>
            </div>
            <div classList={{ "n8n-readiness-card": true, "is-ready": !!readiness()?.ready, "is-blocked": !readiness()?.ready }}>
              <span>Stage 3 Readiness</span>
              <strong>{readiness()?.ready ? "Ready" : "Blocked"}</strong>
              <small>
                {readiness()
                  ? `${readiness()?.workflow_metadata_count}/${readiness()?.required_workflow_count} workflows with routing metadata`
                  : "Run Phase 6 readiness gate."}
              </small>
            </div>
            <div>
              <span>Container</span>
              <strong>{container()?.running ? "Running" : container()?.status || "external/not managed"}</strong>
              <small>{container()?.health || container()?.message || "No managed container health."}</small>
            </div>
            <div>
              <span>Base URL</span>
              <strong>{runtime()?.base_url || payload().base_url || "not configured"}</strong>
              <small>{runtime()?.dashboard_url || payload().dashboard_url || "dashboard URL not configured"}</small>
            </div>
            <div>
              <span>Callback URL</span>
              <strong>{runtime()?.callback_url || payload().callback_url}</strong>
              <small>Use this in signed n8n callback nodes.</small>
            </div>
            <div>
              <span>Dead letters</span>
              <strong>{payload().dead_letters.length}</strong>
              <small>{payload().inbox_path}</small>
            </div>

            <Show when={(readiness()?.missing_gates?.length ?? 0) > 0}>
              <div class="n8n-diagnostics-wide n8n-readiness-detail">
                <span>Blocked Gates</span>
                <For each={readiness()?.missing_gates ?? []}>
                  {(gate) => <small>{gate}</small>}
                </For>
              </div>
            </Show>

            <Show when={readiness()?.ready}>
              <div class="n8n-diagnostics-wide n8n-readiness-detail">
                <span>First Allowed Intelligence Slice</span>
                <For each={readiness()?.first_slice ?? []}>
                  {(step) => <small>{step}</small>}
                </For>
              </div>
            </Show>

            <Show when={payload().dead_letters.length > 0}>
              <div class="n8n-diagnostics-wide">
                <span>Dead-letter drilldown</span>
                <For each={payload().dead_letters}>
                  {(deadLetter) => (
                    <small>
                      {deadLetter.workflow_id} · {deadLetter.reason} · seq {deadLetter.sequence_number}
                    </small>
                  )}
                </For>
              </div>
            </Show>
          </div>
        )}
      </Show>
    </details>
  );
};

export default N8nDiagnosticsPanel;
