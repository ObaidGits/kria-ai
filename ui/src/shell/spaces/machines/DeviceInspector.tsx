/**
 * DeviceInspector — the shared Inspector's body for `type: "device"` targets
 * (task 9.1, Req 8.1). Registered via `registerInspectorRenderer("device", …)`
 * so selecting a DeviceRow opens THIS body in the ONE shared Inspector
 * (Req 1.6). Discloses the device's identity, state, health, latency, docker,
 * and latest test result — every signal as icon + text, never color alone
 * (Req 17.3).
 *
 * Read-only legibility: the destructive delete + remote-desktop controls live
 * in the matrix / later tasks (9.2/9.3). This body wires NO substrate action —
 * it only discloses the device read-model (architecture invariant).
 *
 * HONEST STATE: if the target carries no device data (e.g. opened before the
 * stream populated it) an explicit message is shown, never a silent blank
 * (Req 20.4).
 *
 * SECURITY: device fields are UNTRUSTED — rendered as escaped text (Solid).
 *
 * Requirements: 8.1, 17.2, 17.3, 20.4
 */
import { Show } from "solid-js";
import { Badge } from "../../../kit";
import { Icon } from "../../../components/Icon";
import type { InspectorTarget } from "../../../stores/shellStore";
import type {
  DeviceTargetView,
  DeviceTestResultView,
} from "../../../hooks/useDeviceStatus";
import {
  dockerPresentation,
  formatAbsolute,
  formatAgo,
  healthPct,
  healthPresentation,
  statePresentation,
  testPresentation,
} from "./fleetPresentation";
import "./machines.css";

export interface DeviceInspectorTargetData {
  device: DeviceTargetView;
  testResult?: DeviceTestResultView | null;
}

export interface DeviceInspectorProps {
  target: InspectorTarget;
}

export function DeviceInspector(props: DeviceInspectorProps) {
  const data = () => (props.target.data ?? null) as DeviceInspectorTargetData | null;
  const device = () => data()?.device ?? null;

  return (
    <div class="kria-device" data-testid="device-inspector">
      <Show
        when={device()}
        fallback={
          <p class="kria-device__desc" role="status">
            Device details are unavailable right now. It may be re-connecting to the fleet.
          </p>
        }
      >
        {(d) => {
          const health = healthPresentation(d());
          const state = statePresentation(d().state);
          const docker = dockerPresentation(d().dockerHealth);
          const test = testPresentation(data()?.testResult ?? null);
          return (
            <>
              <section class="kria-device__section" aria-label="Identity">
                <h3 class="kria-device__section-title">Identity</h3>
                <dl class="kria-device__meta">
                  <dt>Name</dt>
                  <dd>{d().displayName}</dd>
                  <dt>Target id</dt>
                  <dd>{d().targetId}</dd>
                  <dt>Mode</dt>
                  <dd>{d().mode}</dd>
                  <dt>Updated</dt>
                  <dd>{formatAgo(d().updatedAtUnixMs)}</dd>
                </dl>
              </section>

              <section class="kria-device__section" aria-label="Status">
                <h3 class="kria-device__section-title">Status</h3>
                <div class="kria-device__tags">
                  <Badge tone={state.tone}>
                    <Icon name={state.icon} size={12} aria-hidden /> {state.label}
                  </Badge>
                  <Badge tone={health.tone}>
                    <Icon name={health.icon} size={12} aria-hidden /> {health.label}
                  </Badge>
                </div>
                <dl class="kria-device__meta">
                  <dt>Health</dt>
                  <dd>{healthPct(d())}%</dd>
                  <dt>Latency</dt>
                  <dd>{Math.round(d().latencyEwmaMs)} ms</dd>
                  <dt>Failure rate</dt>
                  <dd>{(d().recentFailureRate * 100).toFixed(1)}%</dd>
                </dl>
                <Show when={d().taintReason}>
                  <p class="kria-device__desc">
                    <Icon name="alert-triangle" size={13} aria-hidden /> {d().taintReason}
                  </p>
                </Show>
              </section>

              <section class="kria-device__section" aria-label="Docker evals">
                <h3 class="kria-device__section-title">Docker evals</h3>
                <div class="kria-device__tags">
                  <Badge tone={docker.tone}>
                    <Icon name={docker.icon} size={12} aria-hidden /> {docker.label}
                  </Badge>
                </div>
                <dl class="kria-device__meta">
                  <dt>Pass / fail</dt>
                  <dd>
                    {d().dockerPassCount} / {d().dockerFailCount}
                  </dd>
                  <dt>Last run</dt>
                  <dd>{formatAbsolute(d().dockerLastRunAtUnixMs)}</dd>
                </dl>
              </section>

              <section class="kria-device__section" aria-label="Latest test">
                <h3 class="kria-device__section-title">Latest test</h3>
                <div class="kria-device__tags">
                  <Badge tone={test.tone}>
                    <Icon name={test.icon} size={12} aria-hidden /> {test.label}
                  </Badge>
                </div>
                <Show when={data()?.testResult}>
                  {(result) => (
                    <dl class="kria-device__meta">
                      <dt>Suite</dt>
                      <dd>{result().suiteName}</dd>
                      <dt>Zone</dt>
                      <dd>{result().zone}</dd>
                      <dt>When</dt>
                      <dd>{formatAgo(result().timestampUnixMs)}</dd>
                    </dl>
                  )}
                </Show>
              </section>
            </>
          );
        }}
      </Show>
    </div>
  );
}

export default DeviceInspector;
