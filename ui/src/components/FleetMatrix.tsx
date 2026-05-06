import { Component, For, Show, createMemo } from "solid-js";
import {
  FleetAlertView,
  FleetConnectionState,
  FleetTerminalLine,
  FleetTargetView,
} from "../hooks/useFleetHeartbeat";

export interface FleetMatrixProps {
  fleet: FleetTargetView[];
  focusedTerminalTargetId: string | null;
  terminalLines: FleetTerminalLine[];
  alerts: FleetAlertView[];
  streamState: FleetConnectionState;
  lastHeartbeatAtUnixMs: number | null;
  leaseHealthy: boolean;
  lastError: string | null;
  onAddTarget: () => void;
  onReconnectStreams: () => void;
  onFocusTerminal: (targetId: string | null) => void;
  onRunDockerEvals: (targetId: string) => Promise<void> | void;
  dockerActionDisabled?: boolean;
  title?: string;
  class?: string;
}

function formatAgo(unixMs: number | null): string {
  if (!unixMs) {
    return "never";
  }
  const diffMs = Math.max(0, Date.now() - unixMs);
  if (diffMs < 1_000) {
    return "just now";
  }
  if (diffMs < 60_000) {
    return `${Math.floor(diffMs / 1_000)}s ago`;
  }
  if (diffMs < 3_600_000) {
    return `${Math.floor(diffMs / 60_000)}m ago`;
  }
  return `${Math.floor(diffMs / 3_600_000)}h ago`;
}

function healthPct(target: FleetTargetView): number {
  const score = Math.max(0, Math.min(1, target.healthScore));
  const penalty = Math.max(0, Math.min(1, target.recentFailureRate));
  const adjusted = Math.max(0, score * (1 - penalty * 0.5));
  return Math.round(adjusted * 100);
}

function stateClass(state: FleetTargetView["state"]): string {
  return `state-${state}`;
}

function formatRunAt(unixMs: number | null): string {
  if (!unixMs || Number.isNaN(unixMs)) {
    return "never";
  }
  return new Date(unixMs).toLocaleString();
}

function streamStateLabel(state: FleetConnectionState): string {
  if (state === "online") return "Online";
  if (state === "connecting") return "Loading";
  if (state === "degraded") return "Offline";
  if (state === "stopped") return "Stopped";
  return "Idle";
}

function emptyTelemetryLabel(state: FleetConnectionState): string {
  if (state === "connecting") return "Loading fleet telemetry...";
  if (state === "online") return "Connected. Waiting for fleet telemetry events...";
  if (state === "degraded") return "Fleet telemetry is currently offline.";
  if (state === "stopped") return "Fleet telemetry is paused.";
  return "Fleet telemetry is idle.";
}

const FleetMatrix: Component<FleetMatrixProps> = (props) => {
  const selectedTarget = createMemo(() => {
    const selectedId = props.focusedTerminalTargetId;
    if (!selectedId) {
      return null;
    }
    return props.fleet.find((target) => target.targetId === selectedId) ?? null;
  });

  const visibleTerminalLines = createMemo(() => {
    const lines = props.terminalLines;
    if (lines.length <= 500) {
      return lines;
    }
    return lines.slice(lines.length - 500);
  });

  return (
    <section class={`fleet-matrix ${props.class ?? ""}`.trim()}>
      <header class="fleet-matrix-head">
        <div>
          <div class="fleet-matrix-kicker">Ironclad Fleet</div>
          <h3>{props.title ?? "Live Orchestration Matrix"}</h3>
        </div>

        <div class="fleet-matrix-head-metrics">
          <button class="btn-secondary" onClick={props.onAddTarget}>
            Add Soldier
          </button>
          <span class="fleet-stream-status-wrap">
            <span class={`fleet-stream-state ${props.streamState}`}>
              {streamStateLabel(props.streamState)}
            </span>
            <Show when={props.lastError}>
              <span
                class="fleet-stream-info"
                role="img"
                aria-label="Connection detail"
                data-reason={props.lastError ?? ""}
                title={props.lastError ?? ""}
              >
                i
              </span>
            </Show>
          </span>
          <span class="fleet-heartbeat-chip">
            lease heartbeat {formatAgo(props.lastHeartbeatAtUnixMs)}
          </span>
          <span class={`fleet-heartbeat-chip ${props.leaseHealthy ? "healthy" : "stale"}`}>
            {props.leaseHealthy ? "lease healthy" : "lease stale"}
          </span>
          <button class="btn-secondary" onClick={props.onReconnectStreams}>
            Reconnect Streams
          </button>
        </div>
      </header>

      <div class="fleet-matrix-grid">
        <div class="fleet-target-pane">
          <div class="fleet-pane-title">Targets</div>
          <div class="fleet-table-wrap">
            <table class="fleet-table">
              <thead>
                <tr>
                  <th>Target</th>
                  <th>Mode</th>
                  <th>State</th>
                  <th>Health</th>
                  <th>Latency</th>
                  <th>Failures</th>
                  <th>Docker</th>
                  <th>Actions</th>
                </tr>
              </thead>
              <tbody>
                <Show
                  when={props.fleet.length > 0}
                  fallback={
                    <tr>
                      <td colspan={8} class="fleet-empty-row">
                        {emptyTelemetryLabel(props.streamState)}
                      </td>
                    </tr>
                  }
                >
                  <For each={props.fleet}>
                    {(target) => (
                      <tr
                        class={`fleet-target-row ${props.focusedTerminalTargetId === target.targetId ? "selected" : ""}`}
                        onClick={() => props.onFocusTerminal(target.targetId)}
                      >
                        <td>
                          <div class="fleet-target-name">{target.displayName}</div>
                          <div class="fleet-target-id">{target.targetId}</div>
                        </td>
                        <td>{target.mode}</td>
                        <td>
                          <span class={`fleet-state-pill ${stateClass(target.state)} ${target.tainted ? "tainted" : ""}`}>
                            {target.state}
                          </span>
                          <Show when={target.taintReason}>
                            <div class="fleet-taint-reason">{target.taintReason}</div>
                          </Show>
                        </td>
                        <td>
                          <div class="fleet-health-bar">
                            <div class="fleet-health-fill" style={{ width: `${healthPct(target)}%` }} />
                          </div>
                          <div class="fleet-health-value">{healthPct(target)}%</div>
                        </td>
                        <td>{Math.round(target.latencyEwmaMs)}ms</td>
                        <td>{(target.recentFailureRate * 100).toFixed(1)}%</td>
                        <td>
                          <span class={`fleet-docker-pill docker-${target.dockerHealth}`}>
                            {target.dockerHealth}
                          </span>
                          <div class="fleet-docker-meta">
                            pass {target.dockerPassCount} / fail {target.dockerFailCount}
                          </div>
                          <div class="fleet-docker-meta">{formatRunAt(target.dockerLastRunAtUnixMs)}</div>
                        </td>
                        <td class="fleet-actions-cell">
                          <button
                            class="btn-secondary fleet-action-btn"
                            disabled={props.dockerActionDisabled || target.state === "quarantine" || target.state === "disabled"}
                            onClick={(event) => {
                              event.stopPropagation();
                              void props.onRunDockerEvals(target.targetId);
                            }}
                          >
                            Run Docker Evals
                          </button>
                          <button
                            class="btn-secondary fleet-action-btn"
                            onClick={(event) => {
                              event.stopPropagation();
                              props.onFocusTerminal(
                                props.focusedTerminalTargetId === target.targetId ? null : target.targetId,
                              );
                            }}
                          >
                            {props.focusedTerminalTargetId === target.targetId ? "Hide Terminal" : "Open Terminal"}
                          </button>
                        </td>
                      </tr>
                    )}
                  </For>
                </Show>
              </tbody>
            </table>
          </div>
        </div>

        <div class="fleet-live-pane">
          <div class="fleet-pane-title">Focused Terminal</div>
          <Show
            when={selectedTarget()}
            fallback={<div class="fleet-terminal-empty">Select a target to attach terminal stream.</div>}
          >
            <div class="fleet-terminal-head">
              <div>
                <strong>{selectedTarget()!.displayName}</strong>
                <div class="fleet-target-id">{selectedTarget()!.targetId}</div>
              </div>
              <button class="btn-secondary" onClick={() => props.onFocusTerminal(null)}>
                Detach
              </button>
            </div>

            <div class="fleet-terminal-body">
              <For each={visibleTerminalLines()}>
                {(line) => (
                  <div class={`fleet-terminal-line stream-${line.stream}`}>
                    <span class="fleet-terminal-offset">#{line.offset}</span>
                    <span class="fleet-terminal-text">{line.text}</span>
                  </div>
                )}
              </For>
              <Show when={visibleTerminalLines().length === 0}>
                <div class="fleet-terminal-empty">No terminal output yet.</div>
              </Show>
            </div>
          </Show>

          <div class="fleet-pane-title minor">Alerts</div>
          <div class="fleet-alert-list">
            <Show when={props.alerts.length > 0} fallback={<div class="fleet-alert-empty">No active alerts.</div>}>
              <For each={props.alerts.slice(0, 24)}>
                {(alert) => (
                  <div class="fleet-alert-item">
                    <div class="fleet-alert-top">
                      <strong>{alert.category}</strong>
                      <span>{formatAgo(alert.createdAtUnixMs)}</span>
                    </div>
                    <div class="fleet-alert-message">{alert.message}</div>
                    <Show when={alert.targetId || alert.leaseId}>
                      <div class="fleet-alert-meta">
                        <Show when={alert.targetId}>
                          <span>target {alert.targetId}</span>
                        </Show>
                        <Show when={alert.leaseId}>
                          <span>lease {alert.leaseId}</span>
                        </Show>
                      </div>
                    </Show>
                  </div>
                )}
              </For>
            </Show>
          </div>

        </div>
      </div>
    </section>
  );
};

export default FleetMatrix;
