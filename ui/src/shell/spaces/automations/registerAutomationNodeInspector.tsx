/**
 * Registers the node builder's Inspector body for `type: "automation-node"`
 * targets (task 7.3, Req 6.3 / 5.2). Called from NodeBuilder on mount; the
 * returned disposer unregisters on unmount / hot-reload (see
 * inspectorRegistry.ts). Keeping this tiny module separate lets both the
 * builder and stories/tests register the same renderer without duplicating the
 * wiring — mirroring registerMemoryInspector (task 6.2).
 */
import { registerInspectorRenderer, type InspectorContent } from "../../inspectorRegistry";
import type { InspectorTarget } from "../../../stores/shellStore";
import { NodeInspector } from "./NodeInspector";

export function registerAutomationNodeInspector(): () => void {
  return registerInspectorRenderer(
    "automation-node",
    (target: InspectorTarget): InspectorContent => ({
      title: "Node",
      body: <NodeInspector target={target} />,
    }),
  );
}
