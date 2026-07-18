/**
 * Stories for the single shared Inspector (task 4.4, Req 1.6 / 5.2 / 7.2).
 * Shows the content-typed body registry, the titled fallback, and the
 * one-at-a-time replace behaviour.
 */
import type { Meta, StoryObj } from "storybook-solidjs-vite";
import { onCleanup } from "solid-js";
import { InspectorHost } from "./InspectorHost";
import { registerInspectorRenderer, resetInspectorRegistry } from "./inspectorRegistry";
import { Badge, Row } from "../kit";
import { shellStore } from "../stores";

const meta = {
  title: "Shell/InspectorHost",
  component: InspectorHost,
} satisfies Meta<typeof InspectorHost>;

export default meta;
type Story = StoryObj<typeof meta>;

function frame(child: unknown) {
  return (
    <div style={{ height: "520px", display: "flex", "justify-content": "flex-end" }}>
      {child as never}
    </div>
  );
}

/** A memory body contributed via a per-type renderer (what task 6.2 registers). */
export const MemoryBody: Story = {
  render: () => {
    shellStore.setInspectorTarget({ type: "memory", id: "fact-42" });
    return frame(
      <InspectorHost
        renderers={{
          memory: (t) => ({
            title: `Memory ${t.id}`,
            body: (
              <div>
                <Row>
                  <Badge tone="success">verified</Badge>
                  <Badge tone="neutral">confidence 0.82</Badge>
                </Row>
                <p>Source: conversation · worth 0.7 · no conflicts.</p>
              </div>
            ),
          }),
        }}
      />,
    );
  },
};

/** A capability descriptor body (what task 8.1 registers). */
export const CapabilityBody: Story = {
  render: () => {
    shellStore.setInspectorTarget({ type: "capability", id: "shell.run" });
    return frame(
      <InspectorHost
        renderers={{
          capability: (t) => ({
            title: `Capability ${t.id}`,
            body: (
              <div>
                <Row>
                  <Badge tone="warning">trust: review</Badge>
                </Row>
                <p>Runs a shell command on the host. Effects: filesystem, process.</p>
              </div>
            ),
          }),
        }}
      />,
    );
  },
};

/** An unregistered type falls back to a titled placeholder. */
export const UnregisteredFallback: Story = {
  render: () => {
    shellStore.setInspectorTarget({ type: "device", id: "node-3" });
    return frame(<InspectorHost />);
  },
};

/** Module-level registration (how a Space plugs in without prop wiring). */
export const ModuleRegistration: Story = {
  render: () => {
    const dispose = registerInspectorRenderer("automation-node", (t) => ({
      title: `Node ${t.id}`,
      body: <p>HTTP request → transform → notify. Draft, untested.</p>,
    }));
    onCleanup(() => {
      dispose();
      resetInspectorRegistry();
    });
    shellStore.setInspectorTarget({ type: "automation-node", id: "n8" });
    return frame(<InspectorHost />);
  },
};
