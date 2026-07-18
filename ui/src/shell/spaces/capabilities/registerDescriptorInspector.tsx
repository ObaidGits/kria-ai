/**
 * Registers the Capabilities Space's Inspector body for `type: "capability"`
 * targets (task 8.1, Req 7.2). Called from CapabilitiesSpace on mount; the
 * returned disposer unregisters on unmount / hot-reload (see
 * inspectorRegistry.ts). Keeping this in a tiny module lets both the Space and
 * stories/tests register the same renderer without duplicating the wiring —
 * mirroring registerMemoryInspector (task 6.2) and registerAutomationNodeInspector
 * (task 7.3).
 */
import { registerInspectorRenderer, type InspectorContent } from "../../inspectorRegistry";
import type { InspectorTarget } from "../../../stores/shellStore";
import { DescriptorInspector } from "./DescriptorInspector";

export function registerDescriptorInspector(): () => void {
  return registerInspectorRenderer(
    "capability",
    (target: InspectorTarget): InspectorContent => ({
      title: "Capability",
      body: <DescriptorInspector target={target} />,
    }),
  );
}
