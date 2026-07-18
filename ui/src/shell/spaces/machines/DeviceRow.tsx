/**
 * DeviceRow — one device in the fleet matrix (task 9.1, Req 8.1). Renders a
 * REAL semantic table row (`<tr>` inside FleetMatrix's `<table>`, Req 17.2)
 * with the health / latency / docker / tests columns the requirement calls for.
 *
 * Selecting the device (its name button, keyboard-operable — Req 17.1) opens
 * the device Inspector in the ONE shared Inspector (Req 1.6) via `onInspect`.
 * Every state/health/docker/test signal is icon + text, never color alone
 * (Req 17.3).
 *
 * ── ARCHITECTURE INVARIANT ──────────────────────────────────────────────────
 * Presentation only. The row runs NO substrate action itself — action buttons
 * call back to the Space, which dispatches through the runtime's own commands
 * (dispatch-only). The destructive Delete is routed to a deliberate confirm in
 * the Space (Req 8.4) — this row never deletes inline. Device text is UNTRUSTED
 * and rendered as escaped text (Solid).
 *
 * Requirements: 8.1, 17.1, 17.2, 17.3
 */
import { Show, createMemo } from "solid-js";
import { Badge, IconButton } from "../../../kit";
import { Icon } from "../../../components/Icon";
import type {
  DeviceTargetView,
  DeviceTestResultView,
} from "../../../hooks/useDeviceStatus";
import {
  dockerPresentation,
  formatAbsolute,
  healthPct,
  healthPresentation,
  statePresentation,
  testPresentation,
} from "./fleetPresentation";

export interface DeviceRowProps {
  device: DeviceTargetView;
  /** Latest test result for this device (null → honest "No runs"). */
  testResult?: DeviceTestResultView | null;
  /** Whether this row's terminal is currently focused. */
  terminalOpen?: boolean;
  /** Whether this row is the selected (inspected) device. */
  selected?: boolean;
  /** Open the device Inspector for this device (Req 1.6). */
  onInspect: (device: DeviceTargetView) => void;
  /** Toggle the focused terminal for this device. */
  onToggleTerminal?: (targetId: string) => void;
  /** Request a deliberate-confirm delete (Space owns the confirm, Req 8.4). */
  onRequestDelete?: (device: DeviceTargetView) => void;
  /** Dispatch a docker-eval run through the runtime (dispatch-only). */
  onRunDocker?: (targetId: string) => void;
  /** Disable the docker-eval action (e.g. no active lease) with honest reason. */
  dockerDisabled?: boolean;
  dockerDisabledReason?: string;
  /** Virtualizer index required by TanStack's measurement contract. */
  virtualIndex?: number;
  /** Receives the semantic row for virtualizer measurement. */
  rowRef?: (element: HTMLTableRowElement) => void;
  /** True while a delete is in flight for this device. */
  deleting?: boolean;
}

export function DeviceRow(props: DeviceRowProps) {
  const d = () => props.device;
  const health = createMemo(() => healthPresentation(d()));
  const state = createMemo(() => statePresentation(d().state));
  const docker = createMemo(() => dockerPresentation(d().dockerHealth));
  const test = createMemo(() => testPresentation(props.testResult ?? null));

  const dockerBlocked = () =>
    props.dockerDisabled || d().state === "quarantine" || d().state === "disabled";

  return (
    <tr
      ref={(element) => props.rowRef?.(element)}
      class="kria-fleet__row"
      aria-selected={props.selected ? "true" : "false"}
      data-index={props.virtualIndex}
      data-target-id={d().targetId}
    >
      {/* Device (name + id) — the keyboard-operable inspect trigger. */}
      <td>
        <button
          type="button"
          class="kria-fleet__name-btn kit-focusable"
          onClick={() => props.onInspect(d())}
        >
          <span class="kria-fleet__name">{d().displayName}</span>
          <span class="kria-fleet__id">{d().targetId}</span>
        </button>
        <Show when={d().taintReason}>
          <span class="kria-fleet__taint">
            <Icon name="alert-triangle" size={11} aria-hidden /> {d().taintReason}
          </span>
        </Show>
      </td>

      {/* Mode */}
      <td>{d().mode}</td>

      {/* State */}
      <td>
        <Badge tone={state().tone}>
          <Icon name={state().icon} size={12} aria-hidden /> {state().label}
        </Badge>
      </td>

      {/* Health */}
      <td>
        <div class="kria-fleet__health">
          <span class="kria-fleet__cell-inline">
            <Icon name={health().icon} size={12} aria-hidden />
            <span class="kria-fleet__health-value">{healthPct(d())}%</span>
          </span>
          <progress
            class="kria-fleet__health-meter"
            max={100}
            value={healthPct(d())}
            aria-label={`Health ${health().label}`}
          />
        </div>
      </td>

      {/* Latency */}
      <td>
        <span class="kria-fleet__cell-inline">{Math.round(d().latencyEwmaMs)} ms</span>
        <span class="kria-fleet__cell-meta">
          fail {(d().recentFailureRate * 100).toFixed(1)}%
        </span>
      </td>

      {/* Docker */}
      <td>
        <Badge tone={docker().tone}>
          <Icon name={docker().icon} size={12} aria-hidden /> {docker().label}
        </Badge>
        <span class="kria-fleet__cell-meta">
          pass {d().dockerPassCount} / fail {d().dockerFailCount}
        </span>
        <span class="kria-fleet__cell-meta">{formatAbsolute(d().dockerLastRunAtUnixMs)}</span>
      </td>

      {/* Tests */}
      <td>
        <Badge tone={test().tone}>
          <Icon name={test().icon} size={12} aria-hidden /> {test().label}
        </Badge>
        <Show when={props.testResult}>
          <span class="kria-fleet__cell-meta">{props.testResult!.suiteName}</span>
        </Show>
      </td>

      {/* Actions — all dispatch-only callbacks to the Space. */}
      <td>
        <div class="kria-fleet__actions">
          <Show when={props.onRunDocker}>
            <IconButton
              icon="play"
              label={
                dockerBlocked()
                  ? props.dockerDisabledReason ?? "Docker evals unavailable"
                  : "Run docker evals"
              }
              size="sm"
              disabled={dockerBlocked()}
              onClick={() => props.onRunDocker!(d().targetId)}
            />
          </Show>
          <Show when={props.onToggleTerminal}>
            <IconButton
              icon="terminal"
              label={props.terminalOpen ? "Hide terminal" : "Open terminal"}
              size="sm"
              onClick={() => props.onToggleTerminal!(d().targetId)}
            />
          </Show>
          <Show when={props.onRequestDelete}>
            <IconButton
              icon="trash-2"
              label={`Delete ${d().displayName}`}
              size="sm"
              variant="danger"
              disabled={props.deleting}
              onClick={() => props.onRequestDelete!(d())}
            />
          </Show>
        </div>
      </td>
    </tr>
  );
}

export default DeviceRow;
