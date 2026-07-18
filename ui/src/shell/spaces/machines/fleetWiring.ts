/**
 * Fleet wiring — pure derivations that translate the existing `get_ironclad_status`
 * snapshot + settings into the inputs the live fleet stream (`useDeviceStatus`)
 * needs (task 9.1, Req 8.1 / 20.4).
 *
 * These are pure functions (no Solid, no I/O) so they are unit-testable and so
 * the Space stays a thin wiring layer. They consume EXISTING backend shapes and
 * NEVER change command/event names. When the fleet service is absent the
 * derivations return null / [] so the Space degrades to honest empty/idle
 * states (Req 20.4) rather than throwing.
 *
 * ── ARCHITECTURE INVARIANT ──────────────────────────────────────────────────
 * Read-model only. KRIA is the authoritative orchestrator; enrolled targets are
 * execution substrates surfaced here for legibility. These helpers derive
 * connection metadata for the read stream — they carry no authority and issue
 * no commands.
 */
import type { DeviceTargetView } from "../../../hooks/useDeviceStatus";

type AnyRecord = Record<string, unknown>;

function asRecord(value: unknown): AnyRecord | null {
  return value && typeof value === "object" && !Array.isArray(value) ? (value as AnyRecord) : null;
}

function firstNonEmptyString(candidates: unknown[]): string | null {
  for (const candidate of candidates) {
    if (typeof candidate === "string" && candidate.trim().length > 0) {
      return candidate.trim();
    }
  }
  return null;
}

/**
 * Derive the fleet commander base URL from the ironclad status snapshot and
 * settings. Trailing slash + `/v1` suffix are stripped so `useDeviceStatus`
 * can append `/api/fleet/...`. Returns null when nothing is configured (→ the
 * stream stays idle, no false "offline" alarms, Req 20.4).
 */
export function deriveControllerBaseUrl(status: unknown, settings: unknown): string | null {
  const s = asRecord(status);
  const cfg = asRecord(settings);
  const fleet = asRecord(s?.fleet);
  const pool = asRecord(fleet?.pool_packet);
  const server = asRecord(cfg?.server);
  const ironcladCfg = asRecord(cfg?.ironclad);
  const fleetCfg = asRecord(cfg?.fleet);

  const direct = firstNonEmptyString([
    pool?.controller_base_url,
    pool?.controllerBaseUrl,
    fleet?.controller_base_url,
    fleet?.controllerBaseUrl,
    s?.controller_base_url,
    s?.controllerBaseUrl,
    ironcladCfg?.controller_url,
    ironcladCfg?.controllerUrl,
    ironcladCfg?.controller_base_url,
    ironcladCfg?.controllerBaseUrl,
    fleetCfg?.controller_base_url,
    fleetCfg?.controllerBaseUrl,
    fleetCfg?.controller_url,
    fleetCfg?.controllerUrl,
    server?.controller_base_url,
    server?.controllerBaseUrl,
    server?.base_url,
    server?.baseUrl,
  ]);
  if (direct) {
    return direct.replace(/\/+$/, "").replace(/\/v1$/i, "");
  }

  // Fall back to the local server host/port if present.
  const host = firstNonEmptyString([server?.host, server?.local_host, cfg?.local_host]);
  const portRaw = server?.port ?? server?.local_port ?? cfg?.local_port;
  const port =
    typeof portRaw === "number"
      ? portRaw
      : typeof portRaw === "string"
        ? Number.parseInt(portRaw, 10)
        : Number.NaN;

  if (host && Number.isFinite(port)) {
    const normalizedHost = host === "0.0.0.0" ? "127.0.0.1" : host;
    return `http://${normalizedHost}:${Math.trunc(port)}`;
  }

  return null;
}

/**
 * Derive the active fleet lease id from the status snapshot / settings.
 * Returns null when no lease is active (→ terminal/heartbeat degrade to
 * read-only "local only", Req 20.4).
 */
export function deriveFleetLeaseId(status: unknown, settings: unknown): string | null {
  const s = asRecord(status);
  const cfg = asRecord(settings);
  const fleet = asRecord(s?.fleet);
  const pool = asRecord(fleet?.pool_packet);
  const ironcladCfg = asRecord(cfg?.ironclad);
  const fleetCfg = asRecord(cfg?.fleet);

  return firstNonEmptyString([
    pool?.active_lease_id,
    pool?.activeLeaseId,
    pool?.lease_id,
    pool?.leaseId,
    fleet?.active_lease_id,
    fleet?.activeLeaseId,
    fleet?.lease_id,
    fleet?.leaseId,
    s?.active_lease_id,
    s?.activeLeaseId,
    s?.lease_id,
    s?.leaseId,
    ironcladCfg?.active_lease_id,
    ironcladCfg?.lease_id,
    fleetCfg?.active_lease_id,
    fleetCfg?.lease_id,
  ]);
}

const VALID_STATES: readonly DeviceTargetView["state"][] = [
  "ready",
  "leased",
  "quarantine",
  "tainted",
  "disabled",
  "degraded",
  "unreachable",
  "unknown",
];

const VALID_DOCKER: readonly DeviceTargetView["dockerHealth"][] = [
  "unknown",
  "running",
  "pass",
  "fail",
];

function num(value: unknown, fallback: number): number {
  return typeof value === "number" && Number.isFinite(value) ? value : fallback;
}

/**
 * Map the registry / connection-control targets in an ironclad status snapshot
 * into `DeviceTargetView` rows the fleet matrix renders BEFORE the live stream
 * connects (seed rows so the table is never blank when devices are enrolled).
 * Live data (`connection_control_targets`) wins over plain `enrolled_targets`.
 */
export function mapRegistryTargets(status: unknown): DeviceTargetView[] {
  const s = asRecord(status);
  const fleet = asRecord(s?.fleet);
  if (!fleet) return [];

  const rows: unknown[] = [];
  if (Array.isArray(fleet.enrolled_targets)) rows.push(...fleet.enrolled_targets);
  if (Array.isArray(fleet.connection_control_targets)) rows.push(...fleet.connection_control_targets);

  const map = new Map<string, DeviceTargetView>();
  for (const entry of rows) {
    const row = asRecord(entry);
    if (!row) continue;

    const targetId = firstNonEmptyString([row.target_id, row.targetId, row.id]);
    if (!targetId || map.has(targetId)) continue;

    const isLive = typeof row.state === "string" && row.state !== "unknown";
    const rawState = isLive ? String(row.state) : "unknown";
    const state = (VALID_STATES as readonly string[]).includes(rawState)
      ? (rawState as DeviceTargetView["state"])
      : "unknown";
    const rawDocker = typeof row.docker_health === "string" ? row.docker_health : "unknown";
    const dockerHealth = (VALID_DOCKER as readonly string[]).includes(rawDocker)
      ? (rawDocker as DeviceTargetView["dockerHealth"])
      : "unknown";

    map.set(targetId, {
      targetId,
      displayName: firstNonEmptyString([row.display_name, row.displayName]) ?? targetId,
      mode: firstNonEmptyString([row.mode]) ?? "ssh_bootstrap",
      state,
      tainted: Boolean(row.tainted ?? row.taint_reason),
      taintReason:
        firstNonEmptyString([row.taint_reason, row.reason]) ?? null,
      healthScore: num(row.health_score ?? row.healthScore, 1),
      latencyEwmaMs: num(row.latency_ewma_ms ?? row.latencyEwmaMs, 50),
      recentFailureRate: num(row.recent_failure_rate ?? row.recentFailureRate, 0),
      dockerHealth,
      dockerPassCount: num(row.docker_pass_count ?? row.dockerPassCount, 0),
      dockerFailCount: num(row.docker_fail_count ?? row.dockerFailCount, 0),
      dockerLastRunAtUnixMs:
        typeof row.docker_last_run_at_unix_ms === "number"
          ? row.docker_last_run_at_unix_ms
          : null,
      updatedAtUnixMs: num(row.updated_at_unix_ms, Date.now()),
    });
  }

  return Array.from(map.values());
}
