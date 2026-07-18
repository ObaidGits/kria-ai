/**
 * Registers the Machines Space's Inspector body for `type: "device"` targets
 * (task 9.1, Req 8.1). Called from MachinesSpace on mount; the returned
 * disposer unregisters on unmount / hot-reload (see inspectorRegistry.ts).
 * Mirrors registerDescriptorInspector (task 8.1) so the Space and stories/tests
 * register the same renderer without duplicating wiring.
 */
import { registerInspectorRenderer, type InspectorContent } from "../../inspectorRegistry";
import type { InspectorTarget } from "../../../stores/shellStore";
import { DeviceInspector } from "./DeviceInspector";

export function registerDeviceInspector(): () => void {
  return registerInspectorRenderer(
    "device",
    (target: InspectorTarget): InspectorContent => ({
      title: "Device",
      body: <DeviceInspector target={target} />,
    }),
  );
}
