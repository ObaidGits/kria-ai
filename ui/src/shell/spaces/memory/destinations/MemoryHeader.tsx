/**
 * MemoryHeader — shared header for all Memory Control Center destinations.
 *
 * Renders the exact destination name, Graph Revision, policy context,
 * capability/degradation/offline/recovery status, stale timestamp, and an
 * evidence link. It intentionally never infers health from missing data, never
 * uses editorial copy like "brain" / "mind" / "sentience" / "emotion", and
 * never reveals hidden policy scope beyond what is passed as a prop.
 *
 * This is a pure display component; it performs no mutations and enforces no
 * policy (KRIA runtime-authority invariant).
 *
 * Requirements: F4.2 — truthful revision/policy/capability header.
 */
import { Show } from "solid-js";

// ─── Types ───────────────────────────────────────────────────────────────────

export type MemoryDestination =
  | "Overview"
  | "Recall"
  | "Knowledge"
  | "Timeline"
  | "Goals"
  | "Sources"
  | "Health";

export type MemoryStatus =
  | "ready"
  | "stale"
  | "offline"
  | "recovery"
  | "degraded"
  | "partial";

export interface MemoryHeaderProps {
  /** The exact destination name — rendered as-is, no editorial copy added. */
  destination: MemoryDestination;
  /** Graph Revision number (e.g. 42). */
  revision: number;
  /**
   * Policy context string shown verbatim (e.g. "personal:default").
   * Contains only the fields the caller is authorised to surface — no hidden
   * scope is appended by this component.
   */
  policyContext: string;
  /** Operational status of the Memory Control Center. */
  status: MemoryStatus;
  /**
   * Strategies that are currently unavailable. Present when status is
   * "degraded" or "partial" and at least one strategy is down.
   */
  degradedStrategies?: string[];
  /**
   * Timestamp of the last successful data refresh. Only rendered when
   * status === "stale" and a non-null value is provided.
   */
  staleTimestamp?: Date | null;
  /**
   * URL to the Evidence Artifact for this revision (local-gated).
   * When non-null a link is rendered; when null/undefined the link is omitted.
   */
  evidenceLink?: string | null;
}

// ─── Status label mapping ────────────────────────────────────────────────────

/**
 * Maps each status value to a concise human-readable label.
 * Only these six strings are ever produced — no dynamic or interpolated text.
 */
export function statusLabel(status: MemoryStatus): string {
  switch (status) {
    case "ready":
      return "Ready";
    case "stale":
      return "Stale";
    case "offline":
      return "Offline";
    case "recovery":
      return "Recovery Mode";
    case "degraded":
      return "Degraded";
    case "partial":
      return "Partial";
  }
}

// ─── Component ───────────────────────────────────────────────────────────────

export function MemoryHeader(props: MemoryHeaderProps) {
  const hasStrategies = () =>
    Array.isArray(props.degradedStrategies) && props.degradedStrategies.length > 0;

  return (
    <header role="banner" aria-label="Memory Control Center">
      {/* Exact destination name — no editorial copy (brain/mind/etc. prohibited). */}
      <h1>{props.destination}</h1>

      {/* Graph Revision — stable identifier for cache/diff reasoning. */}
      <span data-testid="revision">Rev. {props.revision}</span>

      {/* Policy context — verbatim from caller; no hidden fields appended. */}
      <span data-testid="policy">{props.policyContext}</span>

      {/* Status indicator with machine-readable attribute for styling/testing. */}
      <span data-testid="status" data-status={props.status}>
        {statusLabel(props.status)}
      </span>

      {/* Stale timestamp — only shown when status is stale AND a date exists. */}
      <Show when={props.status === "stale" && props.staleTimestamp != null}>
        <span data-testid="stale-ts">
          Stale since {props.staleTimestamp!.toLocaleTimeString()}
        </span>
      </Show>

      {/* Degraded strategy list — shown when one or more strategies are down. */}
      <Show when={hasStrategies()}>
        <span data-testid="degraded-strategies">
          Unavailable: {props.degradedStrategies!.join(", ")}
        </span>
      </Show>

      {/* Evidence link — only rendered when a non-null URL is provided. */}
      <Show when={props.evidenceLink != null}>
        <a
          href={props.evidenceLink!}
          data-testid="evidence-link"
          rel="noopener noreferrer"
        >
          Evidence
        </a>
      </Show>

      {/* Recovery banner — writes-disabled alert, shown only in recovery mode. */}
      <Show when={props.status === "recovery"}>
        <span data-testid="recovery-banner" role="alert">
          System in Recovery Mode — writes disabled
        </span>
      </Show>
    </header>
  );
}

export default MemoryHeader;
