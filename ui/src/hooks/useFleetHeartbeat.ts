import { Accessor, createEffect, createMemo, createSignal, onCleanup } from "solid-js";

const TERMINAL_RING_CAPACITY = 4000;
const DEFAULT_HEARTBEAT_INTERVAL_MS = 15_000;
const DEFAULT_HEARTBEAT_JITTER_PCT = 0.10;
const DEFAULT_RECONNECT_BASE_MS = 800;
const DEFAULT_RECONNECT_MAX_MS = 12_000;
const DEFAULT_MAX_ALERTS = 120;

export type FleetConnectionState = "idle" | "connecting" | "online" | "degraded" | "stopped";

export interface FleetTargetView {
  targetId: string;
  displayName: string;
  mode: string;
  state: "ready" | "leased" | "quarantine" | "tainted" | "disabled" | "unknown";
  tainted: boolean;
  taintReason: string | null;
  healthScore: number;
  latencyEwmaMs: number;
  recentFailureRate: number;
  dockerHealth: "unknown" | "running" | "pass" | "fail";
  dockerPassCount: number;
  dockerFailCount: number;
  dockerLastRunAtUnixMs: number | null;
  updatedAtUnixMs: number;
}

export interface FleetAlertView {
  category: string;
  message: string;
  targetId: string | null;
  leaseId: string | null;
  createdAtUnixMs: number;
}

export interface FleetClockDriftView {
  targetId: string;
  previousBufferMs: number;
  nextBufferMs: number;
  rejectionCount: number;
  createdAtUnixMs: number;
}

export interface FleetDockerUpdateView {
  targetId: string;
  runId: string;
  dockerHealth: "unknown" | "running" | "pass" | "fail";
  dockerPassCount: number;
  dockerFailCount: number;
  dockerLastRunAtUnixMs: number | null;
  updatedAtUnixMs: number;
}

export interface FleetTerminalLine {
  targetId: string;
  offset: number;
  stream: "stdout" | "stderr" | "system";
  text: string;
  tsUnixMs: number;
}

type MaybeAccessor<T> = T | Accessor<T>;

export interface UseFleetHeartbeatOptions {
  commanderBaseUrl?: MaybeAccessor<string | null | undefined>;
  fleetSseUrl?: MaybeAccessor<string>;
  terminalWsBaseUrl?: MaybeAccessor<string>;
  leaseId?: MaybeAccessor<string | null | undefined>;
  heartbeatPostUrl?: MaybeAccessor<string | undefined>;
  heartbeatIntervalMs?: number;
  heartbeatJitterPct?: number;
  reconnectBaseMs?: number;
  reconnectMaxMs?: number;
  maxAlerts?: number;
  autoStart?: boolean;
  eventSourceFactory?: (url: string) => EventSource;
  webSocketFactory?: (url: string) => WebSocket;
}

export interface FleetHeartbeatController {
  targets: Accessor<FleetTargetView[]>;
  focusedTargetId: Accessor<string | null>;
  focusTarget: (targetId: string | null) => void;
  focusedTerminalLines: Accessor<FleetTerminalLine[]>;
  terminalLinesFor: (targetId: string) => FleetTerminalLine[];
  alerts: Accessor<FleetAlertView[]>;
  clockDriftAlerts: Accessor<FleetClockDriftView[]>;
  dockerUpdates: Accessor<FleetDockerUpdateView[]>;
  streamState: Accessor<FleetConnectionState>;
  lastHeartbeatAtUnixMs: Accessor<number | null>;
  leaseHealthy: Accessor<boolean>;
  lastError: Accessor<string | null>;
  reconnectNow: () => void;
  start: () => void;
  stop: () => void;
}

type FleetConnectionIssueSource = "sse" | "terminal_ws" | "heartbeat";

class FixedRingBuffer<T> {
  private readonly storage: Array<T | undefined>;
  private start = 0;
  private count = 0;

  constructor(private readonly capacity: number) {
    this.storage = new Array<T | undefined>(capacity);
  }

  push(value: T): void {
    if (this.count < this.capacity) {
      this.storage[(this.start + this.count) % this.capacity] = value;
      this.count += 1;
      return;
    }

    this.storage[this.start] = value;
    this.start = (this.start + 1) % this.capacity;
  }

  toArray(): T[] {
    const output: T[] = [];
    for (let i = 0; i < this.count; i += 1) {
      const value = this.storage[(this.start + i) % this.capacity];
      if (value !== undefined) {
        output.push(value);
      }
    }
    return output;
  }
}

function parseJsonObject(raw: string): Record<string, unknown> | null {
  try {
    const parsed = JSON.parse(raw) as unknown;
    if (parsed && typeof parsed === "object" && !Array.isArray(parsed)) {
      return parsed as Record<string, unknown>;
    }
    return null;
  } catch {
    return null;
  }
}

function asString(value: unknown): string | null {
  return typeof value === "string" ? value : null;
}

function asNumber(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function asBoolean(value: unknown): boolean | null {
  return typeof value === "boolean" ? value : null;
}

function normalizeEventType(payload: Record<string, unknown>): string {
  const candidate =
    asString(payload.type) ?? asString(payload.event) ?? asString(payload.kind) ?? "";
  return candidate.trim().toLowerCase();
}

function toWsUrl(baseUrl: string, targetId: string, leaseId: string | null): string {
  const url = new URL(baseUrl, window.location.href);
  url.searchParams.set("target_id", targetId);
  if (leaseId) {
    url.searchParams.set("lease_id", leaseId);
  }
  return url.toString();
}

function toSseUrl(baseUrl: string, leaseId: string | null): string {
  const url = new URL(baseUrl, window.location.href);
  if (leaseId) {
    url.searchParams.set("lease_id", leaseId);
  }
  return url.toString();
}

function withJitter(baseMs: number, jitterPct: number): number {
  const swing = Math.round(baseMs * jitterPct);
  const delta = Math.round((Math.random() * 2 - 1) * swing);
  return Math.max(100, baseMs + delta);
}

export function useFleetHeartbeat(options: UseFleetHeartbeatOptions): FleetHeartbeatController {
  const resolveInput = <T,>(value: MaybeAccessor<T> | undefined): T | undefined => {
    if (typeof value === "function") {
      return (value as Accessor<T>)();
    }
    return value;
  };

  const commanderBaseUrl = createMemo(() => {
    const raw = resolveInput(options.commanderBaseUrl);
    if (typeof raw !== "string") {
      return null;
    }
    const trimmed = raw.trim();
    if (!trimmed) {
      return null;
    }
    return trimmed.replace(/\/+$/, "").replace(/\/v1$/i, "");
  });

  const [stableCommanderBaseUrl, setStableCommanderBaseUrl] = createSignal<string | null>(
    commanderBaseUrl(),
  );

  createEffect(() => {
    const latest = commanderBaseUrl();
    if (latest) {
      setStableCommanderBaseUrl(latest);
    }
  });

  const resolvedLeaseId = createMemo(() => {
    const raw = resolveInput(options.leaseId);
    if (typeof raw !== "string") {
      return null;
    }
    const trimmed = raw.trim();
    return trimmed.length > 0 ? trimmed : null;
  });

  const resolvedFleetSseUrl = createMemo(() => {
    const explicit = resolveInput(options.fleetSseUrl);
    if (typeof explicit === "string" && explicit.trim().length > 0) {
      return explicit;
    }
    const base = stableCommanderBaseUrl();
    return base ? `${base}/api/fleet/events` : "/api/fleet/events";
  });

  const hasSseTransport = createMemo(() => {
    const explicit = resolveInput(options.fleetSseUrl);
    if (typeof explicit === "string" && explicit.trim().length > 0) {
      return true;
    }
    return Boolean(stableCommanderBaseUrl());
  });

  const resolvedTerminalWsBaseUrl = createMemo(() => {
    const explicit = resolveInput(options.terminalWsBaseUrl);
    if (typeof explicit === "string" && explicit.trim().length > 0) {
      return explicit;
    }
    const base = stableCommanderBaseUrl();
    return base ? `${base}/api/fleet/terminal` : "/api/fleet/terminal";
  });

  const hasTerminalTransport = createMemo(() => {
    const explicit = resolveInput(options.terminalWsBaseUrl);
    if (typeof explicit === "string" && explicit.trim().length > 0) {
      return true;
    }
    return Boolean(stableCommanderBaseUrl());
  });

  const resolvedHeartbeatPostUrl = createMemo(() => {
    const explicit = resolveInput(options.heartbeatPostUrl);
    if (typeof explicit === "string" && explicit.trim().length > 0) {
      return explicit;
    }
    const base = stableCommanderBaseUrl();
    const lease = resolvedLeaseId();
    if (!base || !lease) {
      return undefined;
    }
    return `${base}/api/fleet/leases/${encodeURIComponent(lease)}/heartbeat`;
  });

  const [streamState, setStreamState] = createSignal<FleetConnectionState>(
    options.autoStart === false ? "idle" : "connecting",
  );
  const [focusedTargetId, setFocusedTargetId] = createSignal<string | null>(null);
  const [targetsMap, setTargetsMap] = createSignal<Map<string, FleetTargetView>>(new Map());
  const [alerts, setAlerts] = createSignal<FleetAlertView[]>([]);
  const [clockDriftAlerts, setClockDriftAlerts] = createSignal<FleetClockDriftView[]>([]);
  const [dockerUpdates, setDockerUpdates] = createSignal<FleetDockerUpdateView[]>([]);
  const [lastHeartbeatAtUnixMs, setLastHeartbeatAtUnixMs] = createSignal<number | null>(null);
  const [lastError, setLastError] = createSignal<string | null>(null);
  const [terminalVersion, setTerminalVersion] = createSignal(0);

  const ringBuffers = new Map<string, FixedRingBuffer<FleetTerminalLine>>();
  const maxAlerts = options.maxAlerts ?? DEFAULT_MAX_ALERTS;
  const heartbeatIntervalMs = Math.max(1000, options.heartbeatIntervalMs ?? DEFAULT_HEARTBEAT_INTERVAL_MS);
  const heartbeatJitterPct = Math.max(0, Math.min(0.4, options.heartbeatJitterPct ?? DEFAULT_HEARTBEAT_JITTER_PCT));
  const reconnectBaseMs = Math.max(250, options.reconnectBaseMs ?? DEFAULT_RECONNECT_BASE_MS);
  const reconnectMaxMs = Math.max(reconnectBaseMs, options.reconnectMaxMs ?? DEFAULT_RECONNECT_MAX_MS);

  let started = options.autoStart !== false;
  let disposed = false;
  let sseRetryTimer: number | null = null;
  let wsRetryTimer: number | null = null;
  let heartbeatTimer: number | null = null;
  let eventSource: EventSource | null = null;
  let terminalSocket: WebSocket | null = null;
  let sseRetryAttempt = 0;
  let wsRetryAttempt = 0;
  let terminalOffsetCounter = 0;
  let lastStatusLog: string | null = null;

  const targets = createMemo(() =>
    Array.from(targetsMap().values()).sort((a, b) => {
      const stateOrder: Record<string, number> = {
        tainted: 0,
        quarantine: 1,
        leased: 2,
        ready: 3,
        disabled: 4,
        unknown: 5,
      };
      const rankA = stateOrder[a.state] ?? 10;
      const rankB = stateOrder[b.state] ?? 10;
      if (rankA !== rankB) {
        return rankA - rankB;
      }
      return a.displayName.localeCompare(b.displayName);
    }),
  );

  const focusedTerminalLines = createMemo(() => {
    terminalVersion();
    const targetId = focusedTargetId();
    if (!targetId) {
      return [];
    }
    const buffer = ringBuffers.get(targetId);
    return buffer ? buffer.toArray() : [];
  });

  const leaseHealthy = createMemo(() => {
    const now = Date.now();
    const lastBeat = lastHeartbeatAtUnixMs();
    if (!lastBeat) {
      return false;
    }
    const healthyWindowMs = Math.round(heartbeatIntervalMs * 2.25);
    return now - lastBeat <= healthyWindowMs;
  });

  const upsertTarget = (targetPatch: Partial<FleetTargetView> & { targetId: string }) => {
    setTargetsMap((current) => {
      const next = new Map(current);
      const existing = next.get(targetPatch.targetId);
      next.set(targetPatch.targetId, {
        targetId: targetPatch.targetId,
        displayName: targetPatch.displayName ?? existing?.displayName ?? targetPatch.targetId,
        mode: targetPatch.mode ?? existing?.mode ?? "unknown",
        state: targetPatch.state ?? existing?.state ?? "unknown",
        tainted: targetPatch.tainted ?? existing?.tainted ?? false,
        taintReason: targetPatch.taintReason ?? existing?.taintReason ?? null,
        healthScore: targetPatch.healthScore ?? existing?.healthScore ?? 0,
        latencyEwmaMs: targetPatch.latencyEwmaMs ?? existing?.latencyEwmaMs ?? 0,
        recentFailureRate: targetPatch.recentFailureRate ?? existing?.recentFailureRate ?? 0,
        dockerHealth: targetPatch.dockerHealth ?? existing?.dockerHealth ?? "unknown",
        dockerPassCount: targetPatch.dockerPassCount ?? existing?.dockerPassCount ?? 0,
        dockerFailCount: targetPatch.dockerFailCount ?? existing?.dockerFailCount ?? 0,
        dockerLastRunAtUnixMs: targetPatch.dockerLastRunAtUnixMs ?? existing?.dockerLastRunAtUnixMs ?? null,
        updatedAtUnixMs: targetPatch.updatedAtUnixMs ?? Date.now(),
      });
      return next;
    });
  };

  const appendAlert = (alert: FleetAlertView) => {
    setAlerts((current) => [alert, ...current].slice(0, maxAlerts));
  };

  const appendClockDriftAlert = (alert: FleetClockDriftView) => {
    setClockDriftAlerts((current) => [alert, ...current].slice(0, maxAlerts));
  };

  const appendDockerUpdate = (update: FleetDockerUpdateView) => {
    setDockerUpdates((current) => [update, ...current].slice(0, maxAlerts));
    upsertTarget({
      targetId: update.targetId,
      dockerHealth: update.dockerHealth,
      dockerPassCount: update.dockerPassCount,
      dockerFailCount: update.dockerFailCount,
      dockerLastRunAtUnixMs: update.dockerLastRunAtUnixMs,
      updatedAtUnixMs: update.updatedAtUnixMs,
    });
  };

  const appendTerminalLines = (targetId: string, lines: FleetTerminalLine[]) => {
    let buffer = ringBuffers.get(targetId);
    if (!buffer) {
      buffer = new FixedRingBuffer<FleetTerminalLine>(TERMINAL_RING_CAPACITY);
      ringBuffers.set(targetId, buffer);
    }

    for (const line of lines) {
      buffer.push(line);
    }

    setTerminalVersion((value) => value + 1);
  };

  const terminalLinesFor = (targetId: string): FleetTerminalLine[] => {
    terminalVersion();
    const buffer = ringBuffers.get(targetId);
    return buffer ? buffer.toArray() : [];
  };

  const setConnectionIssue = (
    source: FleetConnectionIssueSource,
    state: FleetConnectionState,
    reason: string | null,
  ) => {
    setStreamState(state);

    if (reason === null) {
      setLastError(null);
    } else {
      setLastError(reason);
    }

    const signature = `${source}|${state}|${reason ?? ""}`;
    if (signature === lastStatusLog) {
      return;
    }
    lastStatusLog = signature;

    if (state === "online" || state === "idle" || state === "stopped") {
      console.info(`[fleet-heartbeat] ${source} ${state}${reason ? `: ${reason}` : ""}`);
      return;
    }
    console.warn(`[fleet-heartbeat] ${source} ${state}${reason ? `: ${reason}` : ""}`);
  };

  const closeEventSource = () => {
    if (eventSource) {
      eventSource.close();
      eventSource = null;
    }
  };

  const closeTerminalSocket = () => {
    if (terminalSocket) {
      terminalSocket.close(1000, "switch_or_stop");
      terminalSocket = null;
    }
  };

  const clearTimers = () => {
    if (sseRetryTimer !== null) {
      window.clearTimeout(sseRetryTimer);
      sseRetryTimer = null;
    }
    if (wsRetryTimer !== null) {
      window.clearTimeout(wsRetryTimer);
      wsRetryTimer = null;
    }
    if (heartbeatTimer !== null) {
      window.clearTimeout(heartbeatTimer);
      heartbeatTimer = null;
    }
  };

  const scheduleSseReconnect = () => {
    if (!started || disposed || !hasSseTransport()) {
      return;
    }
    sseRetryAttempt += 1;
    const backoff = Math.min(reconnectMaxMs, reconnectBaseMs * (2 ** Math.min(sseRetryAttempt, 6)));
    const waitMs = withJitter(backoff, 0.20);

    if (sseRetryTimer !== null) {
      window.clearTimeout(sseRetryTimer);
    }

    sseRetryTimer = window.setTimeout(() => {
      sseRetryTimer = null;
      connectSse();
    }, waitMs);
  };

  const scheduleWsReconnect = () => {
    if (!started || disposed || !focusedTargetId()) {
      return;
    }
    wsRetryAttempt += 1;
    const backoff = Math.min(reconnectMaxMs, reconnectBaseMs * (2 ** Math.min(wsRetryAttempt, 6)));
    const waitMs = withJitter(backoff, 0.20);

    if (wsRetryTimer !== null) {
      window.clearTimeout(wsRetryTimer);
    }

    wsRetryTimer = window.setTimeout(() => {
      wsRetryTimer = null;
      connectFocusedTerminalWs();
    }, waitMs);
  };

  const handleSsePayload = (payload: Record<string, unknown>) => {
    const eventType = normalizeEventType(payload);

    if (eventType === "targetstatus" || eventType === "target_status") {
      const targetId = asString(payload.target_id) ?? asString(payload.targetId);
      if (!targetId) {
        return;
      }

      upsertTarget({
        targetId,
        displayName: asString(payload.display_name) ?? asString(payload.displayName) ?? targetId,
        mode: asString(payload.mode) ?? "unknown",
        state: (asString(payload.state) ?? asString(payload.status) ?? "unknown") as FleetTargetView["state"],
        tainted: asBoolean(payload.tainted) ?? false,
        taintReason: asString(payload.reason) ?? asString(payload.taint_reason),
        healthScore: asNumber(payload.health_score) ?? asNumber(payload.healthScore) ?? undefined,
        latencyEwmaMs: asNumber(payload.latency_ewma_ms) ?? asNumber(payload.latencyEwmaMs) ?? undefined,
        recentFailureRate:
          asNumber(payload.recent_failure_rate) ?? asNumber(payload.recentFailureRate) ?? undefined,
        dockerHealth:
          ((asString(payload.docker_health) ?? asString(payload.dockerHealth) ?? undefined) as FleetTargetView["dockerHealth"] | undefined),
        dockerPassCount: asNumber(payload.docker_pass_count) ?? asNumber(payload.dockerPassCount) ?? undefined,
        dockerFailCount: asNumber(payload.docker_fail_count) ?? asNumber(payload.dockerFailCount) ?? undefined,
        dockerLastRunAtUnixMs:
          asNumber(payload.docker_last_run_at_unix_ms) ??
          asNumber(payload.dockerLastRunAtUnixMs) ??
          undefined,
        updatedAtUnixMs: Date.now(),
      });

      if (!focusedTargetId()) {
        setFocusedTargetId(targetId);
        if (started) {
          connectFocusedTerminalWs();
        }
      }
      return;
    }

    if (eventType === "fleetalert" || eventType === "fleet_alert") {
      appendAlert({
        category: asString(payload.category) ?? "fleet_alert",
        message: asString(payload.message) ?? "",
        targetId: asString(payload.target_id) ?? asString(payload.targetId),
        leaseId: asString(payload.lease_id) ?? asString(payload.leaseId),
        createdAtUnixMs: asNumber(payload.created_at_unix_ms) ?? Date.now(),
      });
      return;
    }

    if (eventType === "clockdrift" || eventType === "clock_drift") {
      const nested = (payload.alert as Record<string, unknown> | undefined) ?? payload;
      const targetId = asString(nested.target_id) ?? asString(nested.targetId);
      if (!targetId) {
        return;
      }
      appendClockDriftAlert({
        targetId,
        previousBufferMs: asNumber(nested.previous_buffer_ms) ?? 0,
        nextBufferMs: asNumber(nested.next_buffer_ms) ?? 0,
        rejectionCount: asNumber(nested.rejection_count) ?? 0,
        createdAtUnixMs: asNumber(nested.created_at_unix_ms) ?? Date.now(),
      });
      return;
    }

    if (eventType === "dockerevalupdate" || eventType === "docker_eval_update") {
      const targetId = asString(payload.target_id) ?? asString(payload.targetId);
      if (!targetId) {
        return;
      }
      const runId =
        asString(payload.run_id) ??
        asString(payload.runId) ??
        asString(payload.docker_last_run_id) ??
        asString(payload.dockerLastRunId) ??
        `run-${Date.now()}`;
      const updatedAtUnixMs =
        asNumber(payload.updated_at_unix_ms) ??
        asNumber(payload.updatedAtUnixMs) ??
        asNumber(payload.docker_last_run_at_unix_ms) ??
        asNumber(payload.dockerLastRunAtUnixMs) ??
        Date.now();
      appendDockerUpdate({
        targetId,
        runId,
        dockerHealth:
          ((asString(payload.docker_health) ?? asString(payload.dockerHealth) ?? "unknown") as FleetDockerUpdateView["dockerHealth"]),
        dockerPassCount: asNumber(payload.docker_pass_count) ?? asNumber(payload.dockerPassCount) ?? 0,
        dockerFailCount: asNumber(payload.docker_fail_count) ?? asNumber(payload.dockerFailCount) ?? 0,
        dockerLastRunAtUnixMs:
          asNumber(payload.docker_last_run_at_unix_ms) ??
          asNumber(payload.dockerLastRunAtUnixMs) ??
          updatedAtUnixMs,
        updatedAtUnixMs,
      });
      return;
    }

    if (eventType === "terminalgap" || eventType === "terminal_gap") {
      const nested = (payload.marker as Record<string, unknown> | undefined) ?? payload;
      const targetId = asString(nested.target_id) ?? asString(nested.targetId);
      if (!targetId) {
        return;
      }
      appendTerminalLines(targetId, [
        {
          targetId,
          offset: terminalOffsetCounter,
          stream: "system",
          text: `--- terminal gap marker inserted (${asString(nested.message) ?? "gap detected"}) ---`,
          tsUnixMs: asNumber(nested.created_at_unix_ms) ?? Date.now(),
        },
      ]);
      terminalOffsetCounter += 1;
      return;
    }

    if (eventType === "heartbeatack" || eventType === "heartbeat_ack") {
      setLastHeartbeatAtUnixMs(asNumber(payload.ts_unix_ms) ?? Date.now());
      return;
    }

    const snapshotTargets = payload.targets;
    if (Array.isArray(snapshotTargets)) {
      for (const row of snapshotTargets) {
        if (!row || typeof row !== "object") {
          continue;
        }
        const record = row as Record<string, unknown>;
        const targetId = asString(record.target_id) ?? asString(record.targetId);
        if (!targetId) {
          continue;
        }
        upsertTarget({
          targetId,
          displayName: asString(record.display_name) ?? asString(record.displayName) ?? targetId,
          mode: asString(record.mode) ?? "unknown",
          state: (asString(record.state) ?? asString(record.status) ?? "unknown") as FleetTargetView["state"],
          tainted: asBoolean(record.tainted) ?? false,
          taintReason: asString(record.reason) ?? asString(record.taint_reason),
          healthScore: asNumber(record.health_score) ?? asNumber(record.healthScore) ?? undefined,
          latencyEwmaMs: asNumber(record.latency_ewma_ms) ?? asNumber(record.latencyEwmaMs) ?? undefined,
          recentFailureRate:
            asNumber(record.recent_failure_rate) ?? asNumber(record.recentFailureRate) ?? undefined,
          dockerHealth:
            ((asString(record.docker_health) ?? asString(record.dockerHealth) ?? undefined) as FleetTargetView["dockerHealth"] | undefined),
          dockerPassCount: asNumber(record.docker_pass_count) ?? asNumber(record.dockerPassCount) ?? undefined,
          dockerFailCount: asNumber(record.docker_fail_count) ?? asNumber(record.dockerFailCount) ?? undefined,
          dockerLastRunAtUnixMs:
            asNumber(record.docker_last_run_at_unix_ms) ??
            asNumber(record.dockerLastRunAtUnixMs) ??
            undefined,
          updatedAtUnixMs: Date.now(),
        });
      }
    }
  };

  const connectSse = () => {
    if (!started || disposed) {
      return;
    }

    if (!hasSseTransport()) {
      closeEventSource();
      setConnectionIssue(
        "sse",
        "connecting",
        "Waiting for fleet commander endpoint from runtime status.",
      );
      scheduleSseReconnect();
      return;
    }

    closeEventSource();
    setConnectionIssue("sse", "connecting", null);

    const sourceFactory = options.eventSourceFactory ?? ((url: string) => new EventSource(url));
    const url = toSseUrl(resolvedFleetSseUrl(), resolvedLeaseId());
    try {
      eventSource = sourceFactory(url);
    } catch {
      setConnectionIssue(
        "sse",
        "degraded",
        "Failed to initialize fleet SSE stream; retrying automatically.",
      );
      scheduleSseReconnect();
      return;
    }

    eventSource.onopen = () => {
      sseRetryAttempt = 0;
      setConnectionIssue("sse", "online", null);
    };

    eventSource.onmessage = (event) => {
      const parsed = parseJsonObject(event.data);
      if (!parsed) {
        return;
      }
      handleSsePayload(parsed);
    };

    eventSource.onerror = () => {
      if (!started || disposed) {
        return;
      }
      setConnectionIssue(
        "sse",
        "degraded",
        "Fleet SSE stream disconnected; retrying automatically.",
      );
      closeEventSource();
      scheduleSseReconnect();
    };
  };

  const parseWsTerminalMessage = (targetId: string, raw: string): FleetTerminalLine[] => {
    const parsed = parseJsonObject(raw);
    if (!parsed) {
      const line: FleetTerminalLine = {
        targetId,
        offset: terminalOffsetCounter,
        stream: "stdout",
        text: raw,
        tsUnixMs: Date.now(),
      };
      terminalOffsetCounter += 1;
      return [line];
    }

    const kind = normalizeEventType(parsed);
    if (kind === "terminalline" || kind === "terminal_line" || kind === "line") {
      const lineTargetId = asString(parsed.target_id) ?? asString(parsed.targetId) ?? targetId;
      const line: FleetTerminalLine = {
        targetId: lineTargetId,
        offset: asNumber(parsed.offset) ?? terminalOffsetCounter,
        stream: ((asString(parsed.stream) ?? "stdout") as FleetTerminalLine["stream"]),
        text: asString(parsed.text) ?? "",
        tsUnixMs: asNumber(parsed.ts_unix_ms) ?? Date.now(),
      };
      terminalOffsetCounter = Math.max(terminalOffsetCounter + 1, line.offset + 1);
      return [line];
    }

    if (kind === "terminalbatch" || kind === "terminal_batch" || kind === "batch") {
      const linesValue = parsed.lines;
      if (!Array.isArray(linesValue)) {
        return [];
      }
      const lines: FleetTerminalLine[] = [];
      for (const entry of linesValue) {
        if (!entry || typeof entry !== "object") {
          continue;
        }
        const row = entry as Record<string, unknown>;
        const rowOffset = asNumber(row.offset) ?? terminalOffsetCounter;
        const rowTarget = asString(row.target_id) ?? asString(row.targetId) ?? targetId;
        lines.push({
          targetId: rowTarget,
          offset: rowOffset,
          stream: ((asString(row.stream) ?? "stdout") as FleetTerminalLine["stream"]),
          text: asString(row.text) ?? "",
          tsUnixMs: asNumber(row.ts_unix_ms) ?? Date.now(),
        });
        terminalOffsetCounter = Math.max(terminalOffsetCounter + 1, rowOffset + 1);
      }
      return lines;
    }

    if (kind === "terminalgap" || kind === "terminal_gap" || kind === "gap") {
      const lineTargetId = asString(parsed.target_id) ?? asString(parsed.targetId) ?? targetId;
      const line: FleetTerminalLine = {
        targetId: lineTargetId,
        offset: terminalOffsetCounter,
        stream: "system",
        text: `--- terminal gap marker inserted (${asString(parsed.message) ?? "gap detected"}) ---`,
        tsUnixMs: asNumber(parsed.ts_unix_ms) ?? Date.now(),
      };
      terminalOffsetCounter += 1;
      return [line];
    }

    return [];
  };

  const connectFocusedTerminalWs = () => {
    if (!started || disposed) {
      return;
    }

    if (!hasTerminalTransport()) {
      closeTerminalSocket();
      return;
    }

    const currentTargetId = focusedTargetId();
    if (!currentTargetId) {
      closeTerminalSocket();
      return;
    }

    closeTerminalSocket();

    const wsFactory = options.webSocketFactory ?? ((url: string) => new WebSocket(url));
    const leaseId = resolvedLeaseId();
    const wsUrl = toWsUrl(resolvedTerminalWsBaseUrl(), currentTargetId, leaseId);
    terminalSocket = wsFactory(wsUrl);

    terminalSocket.onopen = () => {
      wsRetryAttempt = 0;
      if (streamState() === "online") {
        setLastError(null);
      }
      const subscribeMessage = JSON.stringify({
        kind: "subscribe",
        target_id: currentTargetId,
        lease_id: leaseId,
      });
      terminalSocket?.send(subscribeMessage);
    };

    terminalSocket.onmessage = (message) => {
      const activeTarget = focusedTargetId();
      if (!activeTarget) {
        return;
      }
      const raw = typeof message.data === "string" ? message.data : "";
      const lines = parseWsTerminalMessage(activeTarget, raw);
      if (lines.length === 0) {
        return;
      }

      const grouped = new Map<string, FleetTerminalLine[]>();
      for (const line of lines) {
        const arr = grouped.get(line.targetId) ?? [];
        arr.push(line);
        grouped.set(line.targetId, arr);
      }

      for (const [targetId, targetLines] of grouped.entries()) {
        appendTerminalLines(targetId, targetLines);
      }
    };

    terminalSocket.onerror = () => {
      setConnectionIssue(
        "terminal_ws",
        streamState() === "online" ? "online" : "degraded",
        "Focused terminal stream encountered an error.",
      );
    };

    terminalSocket.onclose = () => {
      if (!started || disposed) {
        return;
      }
      setConnectionIssue(
        "terminal_ws",
        streamState() === "online" ? "online" : "degraded",
        "Focused terminal stream closed; reconnecting.",
      );
      scheduleWsReconnect();
    };
  };

  const heartbeatTick = async () => {
    if (!started || disposed) {
      return;
    }

    const heartbeatPostUrl = resolvedHeartbeatPostUrl();
    const leaseId = resolvedLeaseId();

    if (!heartbeatPostUrl || !leaseId) {
      setLastHeartbeatAtUnixMs(null);
      if (!stableCommanderBaseUrl()) {
        setConnectionIssue(
          "heartbeat",
          streamState() === "online" ? "online" : "connecting",
          "Waiting for commander endpoint before lease heartbeat.",
        );
      }
      scheduleHeartbeat();
      return;
    }

    try {
      const response = await fetch(heartbeatPostUrl, {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
        },
        body: JSON.stringify({
          lease_id: leaseId,
          sent_at_unix_ms: Date.now(),
        }),
      });

      if (!response.ok) {
        throw new Error(`heartbeat request failed (${response.status})`);
      }

      setLastHeartbeatAtUnixMs(Date.now());
      if (streamState() === "online") {
        setLastError(null);
      }
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      setConnectionIssue(
        "heartbeat",
        streamState() === "online" ? "online" : "degraded",
        `Lease heartbeat failed: ${message}`,
      );
      appendAlert({
        category: "heartbeat_error",
        message,
        targetId: focusedTargetId(),
        leaseId,
        createdAtUnixMs: Date.now(),
      });
    }

    scheduleHeartbeat();
  };

  const scheduleHeartbeat = () => {
    if (!started || disposed) {
      return;
    }
    if (heartbeatTimer !== null) {
      window.clearTimeout(heartbeatTimer);
    }
    const waitMs = withJitter(heartbeatIntervalMs, heartbeatJitterPct);
    heartbeatTimer = window.setTimeout(() => {
      heartbeatTimer = null;
      void heartbeatTick();
    }, waitMs);
  };

  const reconnectNow = () => {
    if (!started || disposed) {
      return;
    }
    clearTimers();
    closeEventSource();
    closeTerminalSocket();
    sseRetryAttempt = 0;
    wsRetryAttempt = 0;
    connectSse();
    connectFocusedTerminalWs();
    scheduleHeartbeat();
  };

  const start = () => {
    if (disposed || started) {
      return;
    }
    started = true;
    setStreamState("connecting");
    reconnectNow();
  };

  const stop = () => {
    if (!started) {
      return;
    }
    started = false;
    clearTimers();
    closeEventSource();
    closeTerminalSocket();
    setConnectionIssue("sse", "stopped", null);
  };

  const focusTarget = (targetId: string | null) => {
    if (focusedTargetId() === targetId) {
      return;
    }
    setFocusedTargetId(targetId);
    wsRetryAttempt = 0;
    if (started) {
      connectFocusedTerminalWs();
    }
  };

  const connectionConfigKey = createMemo(
    () => `${resolvedFleetSseUrl()}|${resolvedTerminalWsBaseUrl()}|${resolvedHeartbeatPostUrl() ?? ""}|${resolvedLeaseId() ?? ""}`,
  );
  let lastConnectionConfigKey: string | null = connectionConfigKey();

  createEffect(() => {
    const key = connectionConfigKey();
    if (key === lastConnectionConfigKey) {
      return;
    }
    lastConnectionConfigKey = key;
    if (started && !disposed) {
      reconnectNow();
    }
  });

  if (started) {
    reconnectNow();
  }

  onCleanup(() => {
    disposed = true;
    started = false;
    clearTimers();
    closeEventSource();
    closeTerminalSocket();
  });

  return {
    targets,
    focusedTargetId,
    focusTarget,
    focusedTerminalLines,
    terminalLinesFor,
    alerts,
    clockDriftAlerts,
    dockerUpdates,
    streamState,
    lastHeartbeatAtUnixMs,
    leaseHealthy,
    lastError,
    reconnectNow,
    start,
    stop,
  };
}
