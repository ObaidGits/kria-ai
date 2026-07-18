import type { Meta, StoryObj } from "storybook-solidjs-vite";
import { onCleanup } from "solid-js";
import { NodeBuilder } from "./NodeBuilder";
import { NodePalette } from "./NodePalette";
import { NodeInspector } from "./NodeInspector";
import { registerAutomationNodeInspector } from "./registerAutomationNodeInspector";
import { InspectorHost } from "../../InspectorHost";
import { resetInspectorRegistry } from "../../inspectorRegistry";
import { automationStore, shellStore } from "../../../stores";

/**
 * Automations · Build — the 2D node builder (task 7.3, Req 6.3/6.4). Components
 * read the global automationStore, so each story seeds it before rendering.
 * The builder is a lightweight in-house DOM+SVG canvas (NOT the 3D graph).
 */
const meta = {
  title: "Spaces/Automations/Build",
  component: NodeBuilder,
  decorators: [
    (Story: () => unknown) => (
      <div class="kria-shell" data-window-mode="standard" style={{ height: "720px", padding: "24px", "overflow-y": "auto" }}>
        {Story() as never}
      </div>
    ),
  ],
} satisfies Meta<typeof NodeBuilder>;

export default meta;
type Story = StoryObj<typeof meta>;

/** Cold builder — empty canvas + the node palette. */
export const Empty: Story = {
  render: () => {
    automationStore.newDraft();
    shellStore.setInspectorTarget(null);
    return <NodeBuilder />;
  },
};

/** A small seeded graph: trigger → HTTP request → send email. */
export const WithGraph: Story = {
  render: () => {
    automationStore.newDraft();
    shellStore.setInspectorTarget(null);
    automationStore.setDraftName("Morning briefing");
    const a = automationStore.addNode("manual-trigger", { x: 40, y: 60 });
    const b = automationStore.addNode("http-request", { x: 260, y: 60 });
    const c = automationStore.addNode("email", { x: 480, y: 60 });
    automationStore.connectNodes(a, b);
    automationStore.connectNodes(b, c);
    automationStore.selectNode(null);
    return <NodeBuilder />;
  },
};

/** The node palette on its own. */
export const Palette: StoryObj = {
  render: () => {
    automationStore.newDraft();
    return <NodePalette />;
  },
};

/** The node Inspector body (as it appears inside the shared Inspector). */
export const Inspector: StoryObj = {
  render: () => {
    automationStore.newDraft();
    const id = automationStore.addNode("http-request");
    automationStore.updateNodeParams(id, { url: "https://api.example.test/news", method: "GET" });
    const dispose = registerAutomationNodeInspector();
    onCleanup(() => {
      dispose();
      resetInspectorRegistry();
    });
    shellStore.setInspectorTarget({ type: "automation-node", id });
    return (
      <div style={{ position: "relative", height: "560px" }}>
        <InspectorHost />
        <NodeInspector target={{ type: "automation-node", id }} />
      </div>
    );
  },
};
