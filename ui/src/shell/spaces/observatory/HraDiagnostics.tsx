import { createSignal, For, Show } from "solid-js";
import { Badge, Button, Card, EmptyState, Table, type BadgeTone } from "../../../kit";
import { observatoryStore, type DataAuthority } from "../../../stores/observatoryStore";
import type {
  HraDevice,
  HraDiagnosticsEvent,
} from "../../../stores/eventBus";

interface HraPanelProps {
  snapshot: HraDiagnosticsEvent | null;
  authority: DataAuthority;
}

function activeSnapshot(snapshot: HraDiagnosticsEvent | null): HraDiagnosticsEvent | null {
  return snapshot?.available === false ? null : snapshot;
}

function unavailableCopy(authority: DataAuthority): { title: string; description: string } {
  if (authority === "error") return {
    title: "Resource Authority diagnostics failed",
    description: "KRIA could not read the live HRA diagnostics bundle.",
  };
  if (authority === "shadow-mode") return {
    title: "Resource Authority unavailable",
    description: "HRA has not been constructed. CPU and cloud placement can continue without authority diagnostics.",
  };
  return {
    title: "Awaiting Resource Authority",
    description: "No HRA diagnostics snapshot has arrived yet.",
  };
}

function UnavailableState(props: { authority: DataAuthority }) {
  const copy = () => unavailableCopy(props.authority);
  return <EmptyState icon="activity" title={copy().title} description={copy().description} />;
}

function pressure(device: HraDevice): { label: string; tone: BadgeTone } {
  if (device.effective_free_vram_mb <= device.emergency_limit_mb) return { label: "Emergency", tone: "danger" };
  if (device.effective_free_vram_mb <= device.hard_limit_mb) return { label: "Hard", tone: "danger" };
  if (device.effective_free_vram_mb <= device.soft_limit_mb) return { label: "Soft", tone: "warning" };
  return { label: "Nominal", tone: "success" };
}

function count(value: number): string {
  return value.toLocaleString();
}

export function HraNowPanel(props: HraPanelProps) {
  const snapshot = () => activeSnapshot(props.snapshot);
  return <section class="kria-observatory__hra" aria-labelledby="hra-now-heading">
    <div class="kria-observatory__region-head">
      <div>
        <h2 id="hra-now-heading">Hardware &amp; Resource Authority</h2>
        <p>Live admission mode, pressure, co-residency, and capacity forecast.</p>
      </div>
      <Badge tone={props.authority === "live" ? "success" : props.authority === "error" ? "danger" : "neutral"}>
        {props.authority === "live" ? "Live" : props.authority === "error" ? "Error" : "Unavailable"}
      </Badge>
    </div>
    <Show when={snapshot()} fallback={<UnavailableState authority={props.authority} />}>
      {(data) => <>
        <Show when={data().status} fallback={<EmptyState title="HRA status not reported" description="Snapshot contains no authority mode or epoch." />}>
          {(status) => <Card class="kria-observatory__hra-card">
            <div class="kria-observatory__card-head">
              <h3>Authority state</h3>
              <div class="kria-observatory__badges">
                <Badge tone={status().shadow_only ? "warning" : "success"}>
                  {status().shadow_only ? "Shadow" : "Enforcing"}
                </Badge>
                <Badge tone={status().shadow_gate_passes ? "success" : "danger"}>
                  Shadow gate {status().shadow_gate_passes ? "clean" : "diverged"}
                </Badge>
              </div>
            </div>
            <dl class="kria-observatory__hra-stats">
              <div><dt>Epoch</dt><dd>{count(status().epoch)}</dd></div>
              <div><dt>Granted</dt><dd>{count(status().metrics.granted)}</dd></div>
              <div><dt>Busy</dt><dd>{count(status().metrics.busy)}</dd></div>
              <div><dt>Shed</dt><dd>{count(status().metrics.shed)}</dd></div>
              <div><dt>Preemptions</dt><dd>{count(status().metrics.preemptions)}</dd></div>
              <div><dt>Swaps</dt><dd>{count(status().metrics.swaps)}</dd></div>
              <div><dt>OOM events</dt><dd>{count(status().metrics.oom_events)}</dd></div>
              <div><dt>Foreground invariant</dt><dd>{status().metrics.foreground_invariant_ok ? "Safe" : "Violated"}</dd></div>
            </dl>
          </Card>}
        </Show>

        <Card class="kria-observatory__hra-card">
          <div class="kria-observatory__card-head"><h3>GPU devices &amp; pressure bands</h3></div>
          <Show when={data().devices} fallback={<EmptyState title="GPU inventory not reported" description="Snapshot contains no device table." />}>
            {(devices) => <Show when={devices().length > 0}
              fallback={<EmptyState icon="cpu" title="No GPU devices detected" description="HRA reports CPU or cloud placement only." />}>
              <div class="kria-observatory__table-scroll"><Table>
                <caption class="kria-observatory__sr-only">GPU resource pressure</caption>
                <thead><tr><th>Device</th><th>Pressure</th><th>Effective free</th><th>Total</th><th>Bands: soft / hard / emergency</th><th>Health</th><th>Breaker</th></tr></thead>
                <tbody><For each={devices()}>{(device) => {
                  const band = () => pressure(device);
                  return <tr>
                    <th scope="row">{device.id}</th>
                    <td><Badge tone={band().tone}>{band().label}</Badge></td>
                    <td>{count(device.effective_free_vram_mb)} MB</td>
                    <td>{count(device.total_vram_mb)} MB</td>
                    <td>{count(device.soft_limit_mb)} / {count(device.hard_limit_mb)} / {count(device.emergency_limit_mb)} MB</td>
                    <td>{device.health}</td><td>{device.breaker}</td>
                  </tr>;
                }}</For></tbody>
              </Table></div>
            </Show>}
          </Show>
        </Card>

        <div class="kria-observatory__hra-grid">
          <Card class="kria-observatory__hra-card">
            <div class="kria-observatory__card-head"><h3>Co-residency</h3><Badge>{data().profile ?? "Profile unavailable"}</Badge></div>
            <Show when={data().co_residency} fallback={<p>Co-residency metrics not reported.</p>}>
              {(metrics) => <dl class="kria-observatory__hra-stats kria-observatory__hra-stats--compact">
                <div><dt>Preemptions</dt><dd>{count(metrics().preemptions)}</dd></div>
                <div><dt>Dedup hits</dt><dd>{count(metrics().dedup_hits)}</dd></div>
                <div><dt>Rollbacks</dt><dd>{count(metrics().rollbacks)}</dd></div>
              </dl>}
            </Show>
            <Show when={data().residents} fallback={<p>Resident inventory not reported.</p>}>
              {(residents) => <Show when={residents().length > 0}
                fallback={<EmptyState title="No GPU residents" description="No models are currently co-resident." />}>
                <ul class="kria-observatory__hra-list"><For each={residents()}>{(resident) => <li>
                  <div><strong>{resident.model}</strong><span>{resident.class} · {resident.device}</span></div>
                  <div class="kria-observatory__badges"><Badge>{resident.refs} refs</Badge><Show when={resident.pinned}><Badge tone="info">Pinned</Badge></Show></div>
                </li>}</For></ul>
              </Show>}
            </Show>
          </Card>

          <Card class="kria-observatory__hra-card">
            <div class="kria-observatory__card-head"><h3>VRAM forecast</h3></div>
            <Show when={data().forecast} fallback={<EmptyState title="Forecast unavailable" description="HRA has not reported a capacity forecast." />}>
              {(forecast) => <div class="kria-observatory__forecast">
                <strong>{forecast().time_to_exhaustion_s === null
                  ? "Stable — not trending toward exhaustion"
                  : `~${Math.round(forecast().time_to_exhaustion_s!)}s to exhaustion`}</strong>
                <span>Confidence {(forecast().confidence * 100).toFixed(0)}% · {forecast().resource}</span>
              </div>}
            </Show>
          </Card>
        </div>
      </>}
    </Show>
  </section>;
}

export function HraForensicsPanel(props: HraPanelProps) {
  const snapshot = () => activeSnapshot(props.snapshot);
  return <section class="kria-observatory__hra" aria-labelledby="hra-forensics-heading">
    <div class="kria-observatory__region-head"><div><h2 id="hra-forensics-heading">Resource decisions &amp; recovery</h2><p>Authority rationale and crash-fenced leases from live journal state.</p></div></div>
    <Show when={snapshot()} fallback={<UnavailableState authority={props.authority} />}>
      {(data) => <div class="kria-observatory__hra-grid">
        <Card class="kria-observatory__hra-card">
          <div class="kria-observatory__card-head"><h3>Recent decisions</h3></div>
          <Show when={data().decisions} fallback={<EmptyState title="Decision journal unavailable" description="Snapshot contains no explainability journal." />}>
            {(decisions) => <Show when={decisions().length > 0}
              fallback={<EmptyState title="No decisions journaled" description="HRA has not made a placement, eviction, or recovery decision yet." />}>
              <ol class="kria-observatory__decision-list"><For each={decisions().slice().reverse()}>{(decision) => <li>
                <div class="kria-observatory__card-head"><Badge tone="info">{decision.kind}</Badge><span>#{count(decision.seq)} · turn {decision.turn_id}</span></div>
                <strong>{decision.detail || "No additional detail"}</strong><p>{decision.why}</p>
              </li>}</For></ol>
            </Show>}
          </Show>
        </Card>

        <Card class="kria-observatory__hra-card">
          <div class="kria-observatory__card-head"><h3>Recovered leases</h3></div>
          <Show when={data().recovered_open_leases} fallback={<EmptyState title="Recovery journal unavailable" description="Snapshot contains no recovery state." />}>
            {(leases) => <Show when={leases().length > 0}
              fallback={<EmptyState title="Clean boot" description="No orphaned resource leases were recovered." />}>
              <ul class="kria-observatory__hra-list"><For each={leases()}>{(lease) => <li>
                <div><strong>Lease #{count(lease.token)}</strong><span>{lease.device}</span></div><Badge>{count(lease.vram_mb)} MB</Badge>
              </li>}</For></ul>
            </Show>}
          </Show>
        </Card>
      </div>}
    </Show>
  </section>;
}

export function HraDiagnosticsPanel(props: HraPanelProps) {
  const snapshot = () => activeSnapshot(props.snapshot);
  const [exporting, setExporting] = createSignal(false);

  const exportDiagnostics = async () => {
    setExporting(true);
    try {
      const pulled = await observatoryStore.refreshHraDiagnostics();
      const bundle = pulled ?? props.snapshot;
      if (!bundle) return;
      const blob = new Blob([JSON.stringify({
        exported_at: new Date().toISOString(),
        hra_diagnostics: bundle,
      }, null, 2)], { type: "application/json" });
      const url = URL.createObjectURL(blob);
      const anchor = document.createElement("a");
      anchor.href = url;
      anchor.download = `kria-hra-diagnostics-${Date.now()}.json`;
      document.body.appendChild(anchor);
      anchor.click();
      anchor.remove();
      URL.revokeObjectURL(url);
    } finally {
      setExporting(false);
    }
  };

  return <section class="kria-observatory__hra" aria-labelledby="hra-diagnostics-heading">
    <div class="kria-observatory__region-head">
      <div><h2 id="hra-diagnostics-heading">Resource diagnostics</h2><p>Unified telemetry detail, SLA configuration, and authoritative JSON export.</p></div>
      <Button variant="secondary" size="sm" disabled={exporting()} onClick={() => void exportDiagnostics()}>
        {exporting() ? "Preparing…" : "Export diagnostics JSON"}
      </Button>
    </div>
    <Show when={snapshot()} fallback={<UnavailableState authority={props.authority} />}>
      {(data) => <div class="kria-observatory__hra-grid">
        <Card class="kria-observatory__hra-card">
          <div class="kria-observatory__card-head"><h3>Telemetry detail</h3></div>
          <Show when={data().telemetry} fallback={<EmptyState title="Telemetry unavailable" description="HRA snapshot contains no telemetry sample." />}>
            {(telemetry) => <Show when={telemetry().source === "unified_hub"}
              fallback={<EmptyState title="Unified telemetry unavailable" description={`Reported source: ${telemetry().source}`} />}>
              <dl class="kria-observatory__diagnostic-list">
                <div><dt>Source</dt><dd>{telemetry().source}</dd></div>
                <div><dt>Sequence</dt><dd>{telemetry().seq == null ? "Not reported" : count(telemetry().seq!)}</dd></div>
                <div><dt>GPU count</dt><dd>{telemetry().gpu_count == null ? "Not reported" : count(telemetry().gpu_count!)}</dd></div>
                <div><dt>CPU average</dt><dd>{telemetry().cpu_avg_pct == null ? "Not reported" : `${telemetry().cpu_avg_pct}%`}</dd></div>
                <div><dt>CPU cores</dt><dd>{telemetry().cpu_cores == null ? "Not reported" : count(telemetry().cpu_cores!)}</dd></div>
                <div><dt>Per-core CPU</dt><dd>{telemetry().cpu_per_core_pct?.length ? telemetry().cpu_per_core_pct!.map((value) => `${value}%`).join(", ") : "Not reported"}</dd></div>
                <div><dt>RAM free</dt><dd>{telemetry().ram_free_mb == null || telemetry().ram_total_mb == null ? "Not reported" : `${count(telemetry().ram_free_mb!)} / ${count(telemetry().ram_total_mb!)} MB`}</dd></div>
              </dl>
            </Show>}
          </Show>
        </Card>

        <Card class="kria-observatory__hra-card">
          <div class="kria-observatory__card-head"><h3>SLA configuration</h3></div>
          <Show when={data().sla} fallback={<EmptyState title="SLA state unavailable" description="Snapshot contains no SLA configuration." />}>
            {(sla) => <dl class="kria-observatory__diagnostic-list">
              <For each={Object.entries(sla())}>{([key, value]) => <div><dt>{key}</dt><dd>{String(value)}</dd></div>}</For>
            </dl>}
          </Show>
        </Card>
      </div>}
    </Show>
  </section>;
}
