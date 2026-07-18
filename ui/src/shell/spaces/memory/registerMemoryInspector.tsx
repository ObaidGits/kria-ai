/**
 * Registers the Memory Space's Inspector body for `type: "memory"` targets
 * (task 6.2, Req 5.2). Called from MemorySpace on mount; the returned disposer
 * unregisters on unmount / hot-reload (see inspectorRegistry.ts). Keeping this
 * in a tiny module lets both the Space and stories/tests register the same
 * renderer without duplicating the wiring.
 */
import { registerInspectorRenderer, type InspectorContent } from "../../inspectorRegistry";
import type { InspectorTarget } from "../../../stores/shellStore";
import { MemoryInspector } from "./MemoryInspector";

export function registerMemoryInspector(): () => void {
  return registerInspectorRenderer("memory", (target: InspectorTarget): InspectorContent => ({
    title: "Memory",
    body: <MemoryInspector target={target} />,
  }));
}
