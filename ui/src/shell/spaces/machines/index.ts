/**
 * Machines Space component barrel (task 9.1). Re-exports the fleet matrix,
 * terminal, alerts, enroll wizard, and device Inspector wiring so
 * `MachinesSpace` imports from one place.
 */
export { DeviceRow } from "./DeviceRow";
export type { DeviceRowProps } from "./DeviceRow";

export { FleetMatrix } from "./FleetMatrix";
export type { FleetMatrixProps } from "./FleetMatrix";

export { TerminalPane } from "./TerminalPane";
export type { TerminalPaneProps } from "./TerminalPane";

export { AlertList } from "./AlertList";
export type { AlertListProps } from "./AlertList";

export { default as MobileDevicesPanel, isPairingChallengeActive } from "./MobileDevicesPanel";

export { default as RemoteDesktopCanvas, describeRemoteCapability } from "./RemoteDesktopCanvas";
export type { LinuxSessionKind, RemoteCapabilityPresentation } from "./RemoteDesktopCanvas";

export { EnrollWizard, EnrollWizardBody } from "./EnrollWizard";
export type {
  EnrollWizardProps,
  EnrollWizardBodyProps,
  EnrollRequest,
  EnrollResult,
} from "./EnrollWizard";

export { DeviceInspector } from "./DeviceInspector";
export type { DeviceInspectorProps, DeviceInspectorTargetData } from "./DeviceInspector";

export { registerDeviceInspector } from "./registerDeviceInspector";

export {
  deriveControllerBaseUrl,
  deriveFleetLeaseId,
  mapRegistryTargets,
} from "./fleetWiring";

export {
  healthPct,
  healthPresentation,
  statePresentation,
  dockerPresentation,
  testPresentation,
  formatAgo,
  formatAbsolute,
} from "./fleetPresentation";
export type { SignalPresentation } from "./fleetPresentation";
