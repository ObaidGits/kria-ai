/**
 * FleetMatrix — the fleet matrix as a REAL semantic table (task 9.1, Req 8.1 /
 * 17.2). Renders one DeviceRow per enrolled device with the health / latency /
 * docker / tests columns. Honest loading / empty states (Req 20.4) — never a
 * silent blank.
 *
 * ARCHITECTURE: presentation only. Row actions are dispatch-only callbacks the
 * Space forwards to the runtime's own commands.
 *
 * Requirements: 8.1, 17.2, 20.4
 */
import { createMemo, For, Show } from "solid-js";
import { createVirtualizer } from "@tanstack/solid-virtual";
import type {
  DeviceConnectionState,
  DeviceTargetView,
  DeviceTestResultView,
} from "../../../hooks/useDeviceStatus";
import { DeviceRow } from "./DeviceRow";
import "./machines.css";

export interface FleetMatrixProps {
  fleet: DeviceTargetView[];
  streamState: DeviceConnectionState;
  focusedTerminalTargetId?: string | null;
  selectedTargetId?: string | null;
  testResultFor?: (targetId: string) => DeviceTestResultView | null;
  onInspect: (device: DeviceTargetView) => void;
  onToggleTerminal?: (targetId: string) => void;
  onRequestDelete?: (device: DeviceTargetView) => void;
  onRunDocker?: (targetId: string) => void;
  dockerDisabled?: boolean;
  dockerDisabledReason?: string;
  deletingTargetIds?: Set<string>;
}

/** Honest empty-table message keyed on the live-stream state (Req 20.4). */
function emptyLabel(state: DeviceConnectionState): string {
  switch (state) {
    case "connecting":
      return "Connecting to the fleet — loading device status…";
    case "online":
      return "Connected. No devices are enrolled yet — enroll one to begin.";
    case "degraded":
      return "Live fleet status is offline. No devices in the local registry.";
    case "stopped":
      return "Live fleet status is paused. No devices to show.";
    default:
      return "No fleet controller configured. Enroll a device to begin.";
  }
}

export function FleetMatrix(props: FleetMatrixProps) {
  let scrollEl: HTMLDivElement | undefined;
  const columns = 8;
  const virtualizer = createVirtualizer({
    get count() { return props.fleet.length; },
    getScrollElement: () => scrollEl ?? null,
    estimateSize: () => 74,
    overscan: 5,
    getItemKey: (index) => props.fleet[index]?.targetId ?? index,
    initialRect: { width: 960, height: 480 },
  });
  const rows = createMemo(() => {
    const measured = virtualizer.getVirtualItems();
    if (measured.length > 0 || props.fleet.length === 0) return measured;

    // TanStack initializes after its scroll element is mounted. Render one
    // bounded viewport immediately so first paint, SSR, and zero-layout test
    // environments never show a false-empty table while measurement starts.
    const estimate = 74;
    const fallbackCount = Math.min(props.fleet.length, Math.ceil(480 / estimate) + 10);
    return Array.from({ length: fallbackCount }, (_, index) => ({
      index,
      key: props.fleet[index]?.targetId ?? index,
      start: index * estimate,
      end: (index + 1) * estimate,
      size: estimate,
      lane: 0,
    }));
  });
  const topPadding = createMemo(() => rows()[0]?.start ?? 0);
  const bottomPadding = createMemo(() => {
    const last = rows()[rows().length - 1];
    return last ? Math.max(0, virtualizer.getTotalSize() - last.end) : 0;
  });

  return (
    <div ref={scrollEl} class="kria-fleet" data-virtual-list="fleet-matrix">
      <table class="kria-fleet__table">
        <caption>Enrolled devices — health, latency, docker, and test status.</caption>
        <thead>
          <tr>
            <th scope="col">Device</th>
            <th scope="col">Mode</th>
            <th scope="col">State</th>
            <th scope="col">Health</th>
            <th scope="col">Latency</th>
            <th scope="col">Docker</th>
            <th scope="col">Tests</th>
            <th scope="col">Actions</th>
          </tr>
        </thead>
        <tbody>
          <Show
            when={props.fleet.length > 0}
            fallback={
              <tr>
                <td class="kria-fleet__empty" colspan={columns}>
                  {emptyLabel(props.streamState)}
                </td>
              </tr>
            }
          >
            <Show when={topPadding() > 0}>
              <tr class="kria-fleet__spacer" aria-hidden="true">
                <td colspan={columns} style={{ height: `${topPadding()}px` }} />
              </tr>
            </Show>
            <For each={rows()}>
              {(row) => {
                const device = () => props.fleet[row.index];
                return (
                  <Show when={device()}>
                    <DeviceRow
                      virtualIndex={row.index}
                      rowRef={(el) => queueMicrotask(() => virtualizer.measureElement(el))}
                      device={device()!}
                      testResult={props.testResultFor?.(device()!.targetId) ?? null}
                      terminalOpen={props.focusedTerminalTargetId === device()!.targetId}
                      selected={props.selectedTargetId === device()!.targetId}
                      onInspect={props.onInspect}
                      onToggleTerminal={props.onToggleTerminal}
                      onRequestDelete={props.onRequestDelete}
                      onRunDocker={props.onRunDocker}
                      dockerDisabled={props.dockerDisabled}
                      dockerDisabledReason={props.dockerDisabledReason}
                      deleting={props.deletingTargetIds?.has(device()!.targetId)}
                    />
                  </Show>
                );
              }}
            </For>
            <Show when={bottomPadding() > 0}>
              <tr class="kria-fleet__spacer" aria-hidden="true">
                <td colspan={columns} style={{ height: `${bottomPadding()}px` }} />
              </tr>
            </Show>
          </Show>
        </tbody>
      </table>
    </div>
  );
}

export default FleetMatrix;
