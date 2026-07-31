/**
 * Health — Memory Control Center Health destination.
 *
 * Renders exact state for every system subsystem:
 *   authority, index, model, outbox, backlog, resource, degradation,
 *   recovery, last-verified, remediation, and evidence artifact links.
 *
 * Invariants (F4.2 / task 4.2.7):
 * - Renders <section data-testid="health-shell">.
 * - NO invented wellness scores, "health scores", or inferred states.
 * - Each subsystem shows its exact state independently.
 * - Recovery_Mode indicator disables write descriptions and shows only
 *   diagnostics + verified recovery actions.
 * - Developer details (dev-only sections) shown only when isDevMode=true.
 * - Loading indicator shown while isLoading=true.
 * - Each subsystem section uses data-testid matching its name (e.g. "authority-section").
 * - Degraded subsystems list their exact reason and remediation steps.
 * - Evidence Artifact links rendered per subsystem when provided.
 * - This is a pure display component — no mutations, no policy enforcement.
 *
 * Requirements:
 *   MGR-001, MGR-010–011, MGR-017, MGR-031, MGR-038, MGR-045–046
 *   MGD-001, MGD-005, MGD-030
 *   MG-H12, MG-M08–M10, MG-L02, MG-L06–L07
 *   F4.2 (task 4.2.7) — Health destination.
 */
import { For, Show } from "solid-js";

// ─── Sub-state interfaces ────────────────────────────────────────────────────

/** Operational state of a single subsystem. */
export type SubsystemState =
  | "idle"
  | "loading"
  | "ready"
  | "partial"
  | "stale"
  | "offline"
  | "error";

/** Authority subsystem state (schema version, event count, revision, record counts). */
export interface AuthorityState {
  state: SubsystemState;
  schemaVersion: string;
  eventCount: number;
  graphRevision: number;
  recordCount: number;
  lastVerified: string | null;
  evidenceLink: string | null;
  /** Dev-only: raw SQL event log stats. */
  devSqlStats: string | null;
}

/** Index subsystem state (FTS5 + vector). */
export interface IndexState {
  state: SubsystemState;
  fts5Status: string;
  fts5Version: string;
  vectorPartitionStatus: string;
  vectorModel: string;
  vectorDimensions: number;
  lastRebuildTimestamp: string | null;
  lastVerified: string | null;
  evidenceLink: string | null;
  /** Dev-only: internal cursor positions. */
  devCursorInfo: string | null;
}

/** Model subsystem state (embedder + LLM). */
export interface ModelState {
  state: SubsystemState;
  embedderIdentity: string;
  embedderVersion: string;
  embedderStatus: string;
  llmAvailability: string;
  llmStatus: string;
  modelManifest: string;
  lastVerified: string | null;
  evidenceLink: string | null;
}

/** Outbox subsystem state (pending/retry/dead-letter). */
export interface OutboxState {
  state: SubsystemState;
  pendingCount: number;
  retryCount: number;
  deadLetterCount: number;
  lastProcessedTimestamp: string | null;
  lastVerified: string | null;
  evidenceLink: string | null;
  /** Dev-only: raw pending item IDs. */
  devPendingIds: string[] | null;
}

/** Backlog subsystem state (scheduler queue, P0–P4 counts). */
export interface BacklogState {
  state: SubsystemState;
  queueDepth: number;
  p0Count: number;
  p1Count: number;
  p2Count: number;
  p3Count: number;
  p4Count: number;
  lastDrainTimestamp: string | null;
  lastVerified: string | null;
  evidenceLink: string | null;
}

/** Resource pressure state (memory/CPU/thermal/battery). */
export interface ResourceState {
  state: SubsystemState;
  /** Memory pressure in bytes (exact, no wellness inference). */
  memoryPressureBytes: number;
  /** CPU utilisation percent (exact value). */
  cpuUtilisationPercent: number;
  /** Thermal state label as reported by the OS. */
  thermalState: string;
  /** Battery level as a percent, or null if on AC/not applicable. */
  batteryPercent: number | null;
  lastVerified: string | null;
  evidenceLink: string | null;
}

/** A single degraded or failed capability entry. */
export interface DegradedCapability {
  /** Capability name, e.g. "vector-search". */
  name: string;
  /** Exact reason string from the subsystem — never inferred. */
  reason: string;
  /** Ordered list of remediation steps. */
  remediationSteps: string[];
}

/** Degradation state across all capabilities. */
export interface DegradationState {
  /** Capabilities that are currently degraded or failed. */
  degradedCapabilities: DegradedCapability[];
  lastVerified: string | null;
  evidenceLink: string | null;
}

/** Recovery state and available actions. */
export interface RecoveryState {
  /** True when the system is in Recovery_Mode (writes disabled). */
  recoveryMode: boolean;
  lastVerifiedTimestamp: string | null;
  /** Available recovery actions (only shown in Recovery_Mode). */
  availableRecoveryActions: string[];
  evidenceLink: string | null;
}

// ─── Props ───────────────────────────────────────────────────────────────────

export interface HealthProps {
  /** Authority subsystem state. */
  authority: AuthorityState;
  /** Index subsystem state. */
  index: IndexState;
  /** Model subsystem state. */
  model: ModelState;
  /** Outbox subsystem state. */
  outbox: OutboxState;
  /** Backlog subsystem state. */
  backlog: BacklogState;
  /** Resource pressure state. */
  resource: ResourceState;
  /** Cross-capability degradation state. */
  degradation: DegradationState;
  /** Recovery state and actions. */
  recovery: RecoveryState;
  /**
   * True while a health query is in-flight. Shows the loading indicator
   * and suppresses all subsystem sections.
   */
  isLoading: boolean;
  /**
   * True when running in developer mode. Dev-only details (SQL stats,
   * internal cursors, raw event counts) are local-gated behind this flag.
   */
  isDevMode: boolean;
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

/** Human-readable label for a SubsystemState value. Never infers wellness. */
function subsystemStateLabel(s: SubsystemState): string {
  switch (s) {
    case "idle":    return "Idle";
    case "loading": return "Loading";
    case "ready":   return "Ready";
    case "partial": return "Partial";
    case "stale":   return "Stale";
    case "offline": return "Offline";
    case "error":   return "Error";
  }
}

// ─── Component ───────────────────────────────────────────────────────────────

export function Health(props: HealthProps) {
  const isRecovery = () => props.recovery.recoveryMode;
  const hasDegraded = () => props.degradation.degradedCapabilities.length > 0;

  return (
    <section data-testid="health-shell" aria-label="Health">
      {/* ── Loading indicator ────────────────────────────────────────────── */}
      <Show when={props.isLoading}>
        <span data-testid="loading-indicator" role="status" aria-live="polite">
          Loading health…
        </span>
      </Show>

      {/* ── Recovery Mode banner — writes disabled; only diagnostics shown ── */}
      <Show when={isRecovery()}>
        <div
          data-testid="recovery-mode-banner"
          role="alert"
          aria-live="assertive"
        >
          Recovery Mode active — writes disabled
        </div>
      </Show>

      {/* ── Authority ────────────────────────────────────────────────────── */}
      <section data-testid="authority-section" aria-label="Authority state">
        <span data-field="state" data-state={props.authority.state}>
          {subsystemStateLabel(props.authority.state)}
        </span>
        <span data-field="schema-version">{props.authority.schemaVersion}</span>
        <span data-field="event-count">{props.authority.eventCount}</span>
        <span data-field="graph-revision">{props.authority.graphRevision}</span>
        <span data-field="record-count">{props.authority.recordCount}</span>
        <Show when={props.authority.lastVerified !== null}>
          <span data-field="last-verified">{props.authority.lastVerified}</span>
        </Show>
        <Show when={props.authority.evidenceLink !== null}>
          <a
            data-field="evidence-link"
            href={props.authority.evidenceLink!}
            rel="noopener noreferrer"
          >
            Evidence
          </a>
        </Show>
        {/* Dev-only: SQL stats — local-gated */}
        <Show when={props.isDevMode && props.authority.devSqlStats !== null}>
          <span data-field="dev-sql-stats" data-dev-only="true">
            {props.authority.devSqlStats}
          </span>
        </Show>
      </section>

      {/* ── Index ────────────────────────────────────────────────────────── */}
      <section data-testid="index-section" aria-label="Index state">
        <span data-field="state" data-state={props.index.state}>
          {subsystemStateLabel(props.index.state)}
        </span>
        <span data-field="fts5-status">{props.index.fts5Status}</span>
        <span data-field="fts5-version">{props.index.fts5Version}</span>
        <span data-field="vector-partition-status">{props.index.vectorPartitionStatus}</span>
        <span data-field="vector-model">{props.index.vectorModel}</span>
        <span data-field="vector-dimensions">{props.index.vectorDimensions}</span>
        <Show when={props.index.lastRebuildTimestamp !== null}>
          <span data-field="last-rebuild">{props.index.lastRebuildTimestamp}</span>
        </Show>
        <Show when={props.index.lastVerified !== null}>
          <span data-field="last-verified">{props.index.lastVerified}</span>
        </Show>
        <Show when={props.index.evidenceLink !== null}>
          <a
            data-field="evidence-link"
            href={props.index.evidenceLink!}
            rel="noopener noreferrer"
          >
            Evidence
          </a>
        </Show>
        {/* Dev-only: internal cursor info — local-gated */}
        <Show when={props.isDevMode && props.index.devCursorInfo !== null}>
          <span data-field="dev-cursor-info" data-dev-only="true">
            {props.index.devCursorInfo}
          </span>
        </Show>
      </section>

      {/* ── Model ────────────────────────────────────────────────────────── */}
      <section data-testid="model-section" aria-label="Model state">
        <span data-field="state" data-state={props.model.state}>
          {subsystemStateLabel(props.model.state)}
        </span>
        <span data-field="embedder-identity">{props.model.embedderIdentity}</span>
        <span data-field="embedder-version">{props.model.embedderVersion}</span>
        <span data-field="embedder-status">{props.model.embedderStatus}</span>
        <span data-field="llm-availability">{props.model.llmAvailability}</span>
        <span data-field="llm-status">{props.model.llmStatus}</span>
        <span data-field="model-manifest">{props.model.modelManifest}</span>
        <Show when={props.model.lastVerified !== null}>
          <span data-field="last-verified">{props.model.lastVerified}</span>
        </Show>
        <Show when={props.model.evidenceLink !== null}>
          <a
            data-field="evidence-link"
            href={props.model.evidenceLink!}
            rel="noopener noreferrer"
          >
            Evidence
          </a>
        </Show>
      </section>

      {/* ── Outbox ───────────────────────────────────────────────────────── */}
      <section data-testid="outbox-section" aria-label="Outbox state">
        <span data-field="state" data-state={props.outbox.state}>
          {subsystemStateLabel(props.outbox.state)}
        </span>
        <span data-field="pending-count">{props.outbox.pendingCount}</span>
        <span data-field="retry-count">{props.outbox.retryCount}</span>
        <span data-field="dead-letter-count">{props.outbox.deadLetterCount}</span>
        <Show when={props.outbox.lastProcessedTimestamp !== null}>
          <span data-field="last-processed">{props.outbox.lastProcessedTimestamp}</span>
        </Show>
        <Show when={props.outbox.lastVerified !== null}>
          <span data-field="last-verified">{props.outbox.lastVerified}</span>
        </Show>
        <Show when={props.outbox.evidenceLink !== null}>
          <a
            data-field="evidence-link"
            href={props.outbox.evidenceLink!}
            rel="noopener noreferrer"
          >
            Evidence
          </a>
        </Show>
        {/* Dev-only: raw pending item IDs — local-gated */}
        <Show when={props.isDevMode && props.outbox.devPendingIds !== null && props.outbox.devPendingIds!.length > 0}>
          <ul data-field="dev-pending-ids" data-dev-only="true">
            <For each={props.outbox.devPendingIds!}>
              {(id) => <li>{id}</li>}
            </For>
          </ul>
        </Show>
      </section>

      {/* ── Backlog ───────────────────────────────────────────────────────── */}
      <section data-testid="backlog-section" aria-label="Backlog state">
        <span data-field="state" data-state={props.backlog.state}>
          {subsystemStateLabel(props.backlog.state)}
        </span>
        <span data-field="queue-depth">{props.backlog.queueDepth}</span>
        <span data-field="p0-count">{props.backlog.p0Count}</span>
        <span data-field="p1-count">{props.backlog.p1Count}</span>
        <span data-field="p2-count">{props.backlog.p2Count}</span>
        <span data-field="p3-count">{props.backlog.p3Count}</span>
        <span data-field="p4-count">{props.backlog.p4Count}</span>
        <Show when={props.backlog.lastDrainTimestamp !== null}>
          <span data-field="last-drain">{props.backlog.lastDrainTimestamp}</span>
        </Show>
        <Show when={props.backlog.lastVerified !== null}>
          <span data-field="last-verified">{props.backlog.lastVerified}</span>
        </Show>
        <Show when={props.backlog.evidenceLink !== null}>
          <a
            data-field="evidence-link"
            href={props.backlog.evidenceLink!}
            rel="noopener noreferrer"
          >
            Evidence
          </a>
        </Show>
      </section>

      {/* ── Resource ─────────────────────────────────────────────────────── */}
      <section data-testid="resource-section" aria-label="Resource state">
        <span data-field="state" data-state={props.resource.state}>
          {subsystemStateLabel(props.resource.state)}
        </span>
        <span data-field="memory-pressure-bytes">{props.resource.memoryPressureBytes}</span>
        <span data-field="cpu-utilisation-percent">{props.resource.cpuUtilisationPercent}</span>
        <span data-field="thermal-state">{props.resource.thermalState}</span>
        <Show when={props.resource.batteryPercent !== null}>
          <span data-field="battery-percent">{props.resource.batteryPercent}</span>
        </Show>
        <Show when={props.resource.lastVerified !== null}>
          <span data-field="last-verified">{props.resource.lastVerified}</span>
        </Show>
        <Show when={props.resource.evidenceLink !== null}>
          <a
            data-field="evidence-link"
            href={props.resource.evidenceLink!}
            rel="noopener noreferrer"
          >
            Evidence
          </a>
        </Show>
      </section>

      {/* ── Degradation — only when capabilities are degraded ────────────── */}
      <Show when={hasDegraded()}>
        <section data-testid="degradation-section" aria-label="Degradation state">
          <Show when={props.degradation.lastVerified !== null}>
            <span data-field="last-verified">{props.degradation.lastVerified}</span>
          </Show>
          <Show when={props.degradation.evidenceLink !== null}>
            <a
              data-field="evidence-link"
              href={props.degradation.evidenceLink!}
              rel="noopener noreferrer"
            >
              Evidence
            </a>
          </Show>
          <ul data-testid="degraded-capabilities-list" aria-label="Degraded capabilities">
            <For each={props.degradation.degradedCapabilities}>
              {(cap) => (
                <li data-capability={cap.name}>
                  <span data-field="capability-name">{cap.name}</span>
                  <span data-field="reason">{cap.reason}</span>
                  <Show when={cap.remediationSteps.length > 0}>
                    <ol data-field="remediation-steps" aria-label={`Remediation for ${cap.name}`}>
                      <For each={cap.remediationSteps}>
                        {(step) => <li>{step}</li>}
                      </For>
                    </ol>
                  </Show>
                </li>
              )}
            </For>
          </ul>
        </section>
      </Show>

      {/* ── Recovery ─────────────────────────────────────────────────────── */}
      <section data-testid="recovery-section" aria-label="Recovery state">
        <span
          data-field="recovery-mode"
          data-recovery-mode={String(props.recovery.recoveryMode)}
        >
          {props.recovery.recoveryMode ? "Recovery Mode: active" : "Recovery Mode: inactive"}
        </span>
        <Show when={props.recovery.lastVerifiedTimestamp !== null}>
          <span data-field="last-verified">{props.recovery.lastVerifiedTimestamp}</span>
        </Show>
        {/* Recovery actions — only shown when in Recovery_Mode */}
        <Show when={isRecovery() && props.recovery.availableRecoveryActions.length > 0}>
          <ul data-testid="recovery-actions-list" aria-label="Available recovery actions">
            <For each={props.recovery.availableRecoveryActions}>
              {(action) => <li data-action={action}>{action}</li>}
            </For>
          </ul>
        </Show>
        <Show when={props.recovery.evidenceLink !== null}>
          <a
            data-field="evidence-link"
            href={props.recovery.evidenceLink!}
            rel="noopener noreferrer"
          >
            Evidence
          </a>
        </Show>
      </section>
    </section>
  );
}

export default Health;
