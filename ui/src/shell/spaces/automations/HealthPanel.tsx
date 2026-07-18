/**
 * HealthPanel — the n8n Build/Health diagnostics, folded into the Automations
 * Build segment (kria-ui-redesign task 7.5, Req 20.2).
 *
 * The orphaned legacy `N8nDiagnosticsPanel` (never mounted — inventory §II) is
 * folded here: setup health, runtime enable/mode, Stage-3 readiness, managed
 * container, base/dashboard/callback URLs, and the dead-letter drilldown —
 * rebuilt on the design kit + tokens (zero raw color, Req 14.2) with honest
 * loading/empty states.
 *
 * Read-only diagnostics: reads `n8nStore` only (fed by existing n8n commands +
 * events). No orchestration. Untrusted strings (URLs, messages, dead-letter
 * reasons) render as escaped text.
 *
 * Requirements: 20.2
 */
import { createMemo, createSignal, For, Show, onMount } from "solid-js";
import { Badge, Button, Card, Confirm, EmptyState, StatusDot } from "../../../kit";
import type { BadgeTone, StatusTone } from "../../../kit";
import { Icon } from "../../../components/Icon";
import { n8nStore } from "../../../stores/n8n";
import "./build-panels.css";

/** A single labelled diagnostic fact (label + value + optional sub-note). */
function Fact(props: { label: string; value: string; note?: string }) {
  return (
    <div class="kria-health__fact">
      <span class="kria-health__fact-label">{props.label}</span>
      <strong class="kria-health__fact-value">{props.value}</strong>
      <Show when={props.note}>
        <span class="kria-health__fact-note">{props.note}</span>
      </Show>
    </div>
  );
}

function auditTone(value: string | undefined): BadgeTone {
  const normalized = (value ?? "").toLowerCase();
  if (normalized.includes("ready") || normalized.includes("pass") || normalized === "info") return "success";
  if (normalized.includes("critical") || normalized.includes("blocked") || normalized === "high") return "danger";
  if (normalized.includes("warn") || normalized.includes("fix") || normalized.includes("degraded")) return "warning";
  return "neutral";
}

export function HealthPanel() {
  // Best-effort refresh on mount; degrades gracefully when n8n is unavailable
  // (Req 20.4) — the honest empty state below covers the "no status" case.
  onMount(() => void n8nStore.initialize().catch(() => undefined));

  const status = () => n8nStore.status();
  const runtime = () => n8nStore.runtimeStatus();
  const connection = createMemo(() => runtime()?.runtime?.last_connection);
  const container = createMemo(() => runtime()?.runtime?.container);
  const readiness = createMemo(() => status()?.stage3_readiness);
  const deadLetters = createMemo(() => status()?.dead_letters ?? []);
  const audit = () => n8nStore.productionAudit();
  const lifecycleReports = () => n8nStore.workflowLifecycleReports();
  const pendingCopyOperations = createMemo(() =>
    n8nStore.copyLifecycleOperations().filter((operation) => operation.status !== "complete"),
  );
  const [actionMessage, setActionMessage] = createSignal("");

  async function runAudit() {
    setActionMessage("");
    try {
      const result = await n8nStore.runProductionAudit();
      setActionMessage(`Production audit complete: ${result.overall_status.replace(/_/g, " ")}.`);
    } catch (error) {
      setActionMessage(error instanceof Error ? error.message : String(error));
    }
  }

  async function auditLifecycle() {
    setActionMessage("");
    try {
      const reports = await n8nStore.auditWorkflowLifecycle();
      setActionMessage(`Lifecycle audit checked ${reports.length} workflow(s).`);
    } catch (error) {
      setActionMessage(error instanceof Error ? error.message : String(error));
    }
  }

  async function exportAudit() {
    setActionMessage("");
    try {
      const result = await n8nStore.exportProductionAuditBundle(false);
      setActionMessage(result?.message || `Redacted audit bundle exported to ${result?.bundle_path || "eval reports"}.`);
    } catch (error) {
      setActionMessage(error instanceof Error ? error.message : String(error));
    }
  }

  async function repairFinding(finding: NonNullable<ReturnType<typeof audit>>["findings"][number]) {
    setActionMessage("");
    try {
      const result = await n8nStore.repairAuditFinding(finding);
      setActionMessage(result?.message || "Safe audit repair completed.");
    } catch (error) {
      setActionMessage(error instanceof Error ? error.message : String(error));
    }
  }

  async function continueCopyOperation(operationId: string) {
    setActionMessage("");
    try {
      const result = await n8nStore.continuePendingCopyOperation(operationId);
      setActionMessage(result?.message || "Pending generated-copy setup recovered.");
    } catch (error) {
      setActionMessage(error instanceof Error ? error.message : String(error));
    }
  }

  async function cleanupCopy(workflowId: string, deleteFromN8n: boolean) {
    setActionMessage("");
    try {
      const result = await n8nStore.cleanupGeneratedCopy(workflowId, deleteFromN8n);
      setActionMessage(result?.message || "Generated-copy lifecycle cleaned up.");
    } catch (error) {
      setActionMessage(error instanceof Error ? error.message : String(error));
    }
  }

  const enabled = createMemo(() => runtime()?.enabled ?? status()?.enabled ?? false);
  const connTone = createMemo<StatusTone>(() => {
    const c = (connection()?.status ?? "").toLowerCase();
    if (c.includes("ok") || c.includes("connected") || c.includes("healthy")) return "online";
    if (c.includes("fail") || c.includes("error")) return "error";
    if (!c || c.includes("untested")) return "offline";
    return "info";
  });
  const readyTone = createMemo<BadgeTone>(() => (readiness()?.ready ? "success" : "warning"));

  return (
    <section class="kria-health" aria-label="n8n health &amp; diagnostics">
      <h2 class="kria-automations__region-title">Health &amp; diagnostics</h2>

      <Show
        when={status() || runtime()}
        fallback={
          <EmptyState
            icon="activity"
            title="Diagnostics unavailable"
            description="Connect n8n in Settings → Connections to see setup health, readiness, and runtime diagnostics here."
          />
        }
      >
        <>
        <Card class="kria-health__card">
          {/* Setup health + runtime */}
          <div class="kria-health__row">
            <StatusDot tone={connTone()} label={`Setup: ${connection()?.status ?? "untested"}`} />
            <span class="kria-health__row-label">
              Setup health: {connection()?.status ?? "untested"}
            </span>
          </div>
          <Show when={connection()?.message}>
            <p class="kria-health__hint">{connection()!.message}</p>
          </Show>

          <div class="kria-health__facts">
            <Fact
              label="Runtime"
              value={enabled() ? "Enabled" : "Disabled"}
              note={runtime()?.mode || status()?.mode || "mode unknown"}
            />
            <Fact
              label="Container"
              value={
                container()?.running
                  ? "Running"
                  : container()?.status || "external / not managed"
              }
              note={container()?.health || container()?.message || "No managed container health."}
            />
            <Fact
              label="Base URL"
              value={runtime()?.base_url || status()?.base_url || "not configured"}
              note={runtime()?.dashboard_url || status()?.dashboard_url || "dashboard URL not set"}
            />
            <Fact
              label="Callback URL"
              value={runtime()?.callback_url || status()?.callback_url || "not configured"}
              note="Use this in signed n8n callback nodes."
            />
          </div>

          {/* Stage-3 readiness */}
          <Show when={readiness()}>
            {(r) => (
              <div class="kria-health__readiness">
                <div class="kria-health__readiness-head">
                  <span class="kria-health__fact-label">Stage 3 readiness</span>
                  <Badge tone={readyTone()}>{r().ready ? "Ready" : "Blocked"}</Badge>
                  <span class="kria-health__fact-note">
                    {r().workflow_metadata_count}/{r().required_workflow_count} workflows with
                    routing metadata
                  </span>
                </div>
                <Show when={(r().missing_gates?.length ?? 0) > 0}>
                  <div class="kria-health__list">
                    <span class="kria-health__fact-label">Blocked gates</span>
                    <ul>
                      <For each={r().missing_gates}>{(gate) => <li>{gate}</li>}</For>
                    </ul>
                  </div>
                </Show>
                <Show when={r().ready && (r().first_slice?.length ?? 0) > 0}>
                  <div class="kria-health__list">
                    <span class="kria-health__fact-label">First allowed slice</span>
                    <ul>
                      <For each={r().first_slice}>{(step) => <li>{step}</li>}</For>
                    </ul>
                  </div>
                </Show>
              </div>
            )}
          </Show>

          {/* Dead-letter drilldown */}
          <div class="kria-health__deadletters">
            <div class="kria-health__readiness-head">
              <span class="kria-health__fact-label">Dead letters</span>
              <Badge tone={deadLetters().length > 0 ? "danger" : "neutral"}>
                {deadLetters().length}
              </Badge>
              <Show when={status()?.inbox_path}>
                <span class="kria-health__fact-note">{status()!.inbox_path}</span>
              </Show>
            </div>
            <Show when={deadLetters().length > 0}>
              <ul class="kria-health__list">
                <For each={deadLetters()}>
                  {(dl) => (
                    <li>
                      {dl.workflow_id} · {dl.reason} · seq {dl.sequence_number}
                    </li>
                  )}
                </For>
              </ul>
            </Show>
          </div>
        </Card>

        <Card class="kria-health__card" aria-label="n8n production audit and lifecycle">
          <div class="kria-health__audit-head">
            <div>
              <span class="kria-health__row-label">Production audit &amp; lifecycle</span>
              <p class="kria-health__hint">Checks security, reliability, adapters, and workflow drift without running workflows.</p>
            </div>
            <div class="kria-health__audit-actions">
              <Button
                variant="secondary"
                size="sm"
                disabled={n8nStore.managementBusyKey() === "production-audit:run"}
                onClick={() => void runAudit()}
              >
                <Icon name="shield-check" size={14} /> Run audit
              </Button>
              <Button
                variant="secondary"
                size="sm"
                disabled={n8nStore.managementBusyKey() === "lifecycle:audit"}
                onClick={() => void auditLifecycle()}
              >
                <Icon name="refresh-cw" size={14} /> Check lifecycle
              </Button>
              <Confirm
                triggerLabel="Export redacted bundle"
                triggerIcon="download"
                title="Export redacted n8n audit bundle?"
                message="KRIA will write a privacy-filtered diagnostics bundle to the local evaluation reports folder. Workflow labels remain excluded."
                confirmLabel="Export bundle"
                risk="warning"
                onConfirm={() => void exportAudit()}
              />
            </div>
          </div>

          <Show when={actionMessage()}>
            <p class="kria-health__message" role="status">{actionMessage()}</p>
          </Show>

          <Show
            when={audit()}
            fallback={<p class="kria-health__hint">Run an audit for current production-readiness findings.</p>}
          >
            {(report) => (
              <>
                <div class="kria-health__facts">
                  <Fact
                    label="Overall"
                    value={report().overall_status.replace(/_/g, " ")}
                    note={`${report().findings.length} finding(s)`}
                  />
                  <Fact
                    label="Security"
                    value={report().security_status.replace(/_/g, " ")}
                    note={`${report().summary_counts.critical ?? 0} critical · ${report().summary_counts.high ?? 0} high`}
                  />
                  <Fact
                    label="Reliability"
                    value={report().reliability_status.replace(/_/g, " ")}
                    note={new Date(report().generated_at_ms).toLocaleString()}
                  />
                </div>
                <Show when={report().adapter_readiness.length > 0}>
                  <div class="kria-health__audit-grid">
                    <For each={report().adapter_readiness}>
                      {(adapter) => (
                        <div class="kria-health__fact">
                          <span class="kria-health__fact-label">{adapter.adapter.replace(/_/g, " ")}</span>
                          <Badge tone={auditTone(adapter.status)}>{adapter.status.replace(/_/g, " ")}</Badge>
                          <span class="kria-health__fact-note">{adapter.reason}</span>
                        </div>
                      )}
                    </For>
                  </div>
                </Show>
                <Show when={report().findings.length > 0}>
                  <ul class="kria-health__findings">
                    <For each={report().findings}>
                      {(finding) => (
                        <li>
                          <div class="kria-health__finding-main">
                            <div class="kria-registry__tags">
                              <Badge tone={auditTone(finding.severity)}>{finding.severity}</Badge>
                              <Badge tone="neutral">{finding.category}</Badge>
                            </div>
                            <strong>{finding.title}</strong>
                            <span>{finding.message}</span>
                            <small>{finding.next_action}</small>
                          </div>
                          <Show when={finding.safe_to_auto_fix && finding.repair_kind}>
                            <Confirm
                              triggerLabel={finding.repair_kind!.replace(/_/g, " ")}
                              triggerIcon="wrench"
                              title={`Apply safe repair: ${finding.title}?`}
                              message={finding.message}
                              confirmLabel="Apply repair"
                              risk="warning"
                              onConfirm={() => void repairFinding(finding)}
                            />
                          </Show>
                        </li>
                      )}
                    </For>
                  </ul>
                </Show>
              </>
            )}
          </Show>

          <Show when={pendingCopyOperations().length > 0}>
            <div class="kria-health__lifecycle">
              <span class="kria-health__fact-label">Pending generated-copy recovery</span>
              <ul class="kria-health__findings">
                <For each={pendingCopyOperations()}>
                  {(operation) => (
                    <li>
                      <div class="kria-health__finding-main">
                        <strong>{operation.copy_workflow_id || operation.operation_id}</strong>
                        <div class="kria-registry__tags">
                          <Badge tone={auditTone(operation.status)}>{operation.status.replace(/_/g, " ")}</Badge>
                          <Badge tone="neutral">{operation.stage.replace(/_/g, " ")}</Badge>
                        </div>
                        <Show when={operation.source_workflow_id}>
                          <span>Source: {operation.source_workflow_id}</span>
                        </Show>
                        <Show when={operation.last_error}>
                          <span>{operation.last_error}</span>
                        </Show>
                        <Show when={(operation.recovery_actions?.length ?? 0) > 0}>
                          <small>Recovery: {operation.recovery_actions!.join("; ")}</small>
                        </Show>
                      </div>
                      <div class="kria-health__audit-actions">
                        <Button
                          variant="secondary"
                          size="sm"
                          disabled={n8nStore.managementBusyKey() === `lifecycle:continue:${operation.operation_id}`}
                          onClick={() => void continueCopyOperation(operation.operation_id)}
                        >
                          Continue setup
                        </Button>
                        <Confirm
                          triggerLabel="Remove KRIA copy"
                          triggerIcon="trash-2"
                          title={`Remove generated copy ${operation.copy_workflow_id}?`}
                          message="This removes the generated copy from KRIA runtime profiles and registry. The n8n workflow remains intact."
                          confirmLabel="Remove from KRIA"
                          risk="warning"
                          onConfirm={() => void cleanupCopy(operation.copy_workflow_id, false)}
                        />
                        <Confirm
                          triggerLabel="Delete n8n copy"
                          triggerIcon="trash-2"
                          title={`Delete generated copy ${operation.copy_workflow_id} from n8n?`}
                          message="This removes the generated copy from KRIA and permanently deletes its n8n workflow. The source workflow is not changed."
                          confirmLabel="Delete copy"
                          risk="danger"
                          onConfirm={() => void cleanupCopy(operation.copy_workflow_id, true)}
                        />
                      </div>
                    </li>
                  )}
                </For>
              </ul>
            </div>
          </Show>

          <Show when={lifecycleReports().length > 0}>
            <div class="kria-health__lifecycle">
              <span class="kria-health__fact-label">Workflow lifecycle</span>
              <ul class="kria-health__findings">
                <For each={lifecycleReports()}>
                  {(report) => (
                    <li>
                      <div class="kria-health__finding-main">
                        <strong>{report.workflow_id}</strong>
                        <Badge tone={auditTone(report.lifecycle_severity || report.lifecycle_status)}>
                          {report.lifecycle_status.replace(/_/g, " ")}
                        </Badge>
                        <Show when={(report.blockers?.length ?? 0) > 0}>
                          <span>{report.blockers!.join("; ")}</span>
                        </Show>
                        <Show when={(report.warnings?.length ?? 0) > 0}>
                          <small>{report.warnings!.join("; ")}</small>
                        </Show>
                      </div>
                      <Button
                        variant="secondary"
                        size="sm"
                        disabled={n8nStore.managementBusyKey() === `lifecycle:refresh:${report.workflow_id}`}
                        onClick={() => void n8nStore.refreshLifecycleItem(report.workflow_id).catch((error) => setActionMessage(String(error)))}
                      >
                        Refresh
                      </Button>
                    </li>
                  )}
                </For>
              </ul>
            </div>
          </Show>
        </Card>
        </>
      </Show>
    </section>
  );
}

export default HealthPanel;
