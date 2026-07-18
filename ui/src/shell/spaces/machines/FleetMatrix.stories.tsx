import type { Meta, StoryObj } from "storybook-solidjs-vite";
import { FleetMatrix } from "./FleetMatrix";
import { TerminalPane } from "./TerminalPane";
import { AlertList } from "./AlertList";
import type {
  DeviceTargetView,
  DeviceTerminalLine,
  DeviceAlertView,
} from "../../../hooks/useDeviceStatus";

/**
 * Machines fleet components (task 9.1, Req 8.1) — the fleet matrix (real table),
 * the keyboard-accessible terminal pane, and the alert list, seeded with sample
 * data so the workbench renders without a backend.
 */
const DEVICES: DeviceTargetView[] = [
  {
    targetId: "office-vm",
    displayName: "Office Ubuntu VM",
    mode: "ssh_bootstrap",
    state: "ready",
    tainted: false,
    taintReason: null,
    healthScore: 0.96,
    latencyEwmaMs: 38,
    recentFailureRate: 0.0,
    dockerHealth: "pass",
    dockerPassCount: 12,
    dockerFailCount: 0,
    dockerLastRunAtUnixMs: Date.now() - 60_000,
    updatedAtUnixMs: Date.now(),
  },
  {
    targetId: "edge-node",
    displayName: "Edge Node",
    mode: "remote_docker",
    state: "degraded",
    tainted: false,
    taintReason: null,
    healthScore: 0.62,
    latencyEwmaMs: 240,
    recentFailureRate: 0.18,
    dockerHealth: "fail",
    dockerPassCount: 4,
    dockerFailCount: 3,
    dockerLastRunAtUnixMs: Date.now() - 3_600_000,
    updatedAtUnixMs: Date.now(),
  },
  {
    targetId: "quarantined-box",
    displayName: "Quarantined Box",
    mode: "ssh_bootstrap",
    state: "tainted",
    tainted: true,
    taintReason: "host key changed",
    healthScore: 0.2,
    latencyEwmaMs: 900,
    recentFailureRate: 0.6,
    dockerHealth: "unknown",
    dockerPassCount: 0,
    dockerFailCount: 0,
    dockerLastRunAtUnixMs: null,
    updatedAtUnixMs: Date.now(),
  },
];

const LINES: DeviceTerminalLine[] = [
  { targetId: "office-vm", offset: 1, stream: "system", text: "-- attached --", tsUnixMs: Date.now() },
  { targetId: "office-vm", offset: 2, stream: "stdout", text: "$ docker ps", tsUnixMs: Date.now() },
  { targetId: "office-vm", offset: 3, stream: "stdout", text: "CONTAINER   IMAGE   STATUS", tsUnixMs: Date.now() },
  { targetId: "office-vm", offset: 4, stream: "stderr", text: "warning: low disk space", tsUnixMs: Date.now() },
];

const ALERTS: DeviceAlertView[] = [
  {
    category: "clock_drift",
    message: "Clock drift exceeded threshold; heartbeat buffer widened.",
    targetId: "edge-node",
    leaseId: "lease-42",
    createdAtUnixMs: Date.now() - 30_000,
  },
  {
    category: "lease",
    message: "Lease renewal succeeded.",
    targetId: null,
    leaseId: "lease-42",
    createdAtUnixMs: Date.now() - 120_000,
  },
];

const meta = {
  title: "Spaces/Machines/FleetComponents",
  component: FleetMatrix,
  decorators: [
    (Story: () => unknown) => (
      <div class="kria-shell" data-window-mode="standard" style={{ padding: "24px", "max-width": "980px" }}>
        {Story() as never}
      </div>
    ),
  ],
} satisfies Meta<typeof FleetMatrix>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Matrix: Story = {
  args: {
    fleet: DEVICES,
    streamState: "online",
    focusedTerminalTargetId: "office-vm",
    selectedTargetId: "office-vm",
    testResultFor: (id) =>
      id === "office-vm"
        ? {
            targetId: id,
            suiteName: "smoke",
            zone: "prod",
            status: "pass",
            timestampUnixMs: Date.now(),
            reportPath: "",
          }
        : null,
    onInspect: () => {},
    onToggleTerminal: () => {},
    onRequestDelete: () => {},
    onRunDocker: () => {},
  },
  render: (props) => <FleetMatrix {...props} />,
};

export const EmptyOffline: Story = {
  args: {
    fleet: [],
    streamState: "degraded",
    onInspect: () => {},
  },
  render: (props) => <FleetMatrix {...props} />,
};

export const Terminal: Story = {
  args: { fleet: DEVICES, streamState: "online", onInspect: () => {} },
  render: () => <TerminalPane device={DEVICES[0]} lines={LINES} onDetach={() => {}} />,
};

export const Alerts: Story = {
  args: { fleet: DEVICES, streamState: "online", onInspect: () => {} },
  render: () => <AlertList alerts={ALERTS} />,
};
