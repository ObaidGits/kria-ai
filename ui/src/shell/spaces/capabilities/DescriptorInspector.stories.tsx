import type { Meta, StoryObj } from "storybook-solidjs-vite";
import { DescriptorInspector } from "./DescriptorInspector";
import type { CapabilityDescriptor } from "../../../stores";
import type { InspectorTarget } from "../../../stores/shellStore";

/**
 * DescriptorInspector (task 8.1) — the shared Inspector's body for a capability
 * (Req 7.2): descriptor, effects, trust tier, and schema. Stories pass a `fetch`
 * override so the workbench renders without a backend.
 */
function descriptor(over: Partial<CapabilityDescriptor> = {}): CapabilityDescriptor {
  return {
    providerId: "native",
    capabilityId: "web-search",
    name: "Web search",
    description: "Searches the web and returns ranked, cited results.",
    version: "1.4.0",
    schemaVersion: "1",
    tags: ["web", "search", "read-only"],
    ioModality: ["text"],
    inputs: ["query", "max_results"],
    outputs: ["results"],
    effectClasses: ["network.read"],
    reversible: "yes",
    idempotent: true,
    elevated: false,
    trustTier: "verified",
    signed: true,
    inputSchema: {
      type: "object",
      properties: { query: { type: "string" }, max_results: { type: "number" } },
      required: ["query"],
    },
    ...over,
  };
}

const target: InspectorTarget = {
  type: "capability",
  id: "native:web-search",
  data: { providerId: "native", capabilityId: "web-search", name: "Web search" },
};

const meta = {
  title: "Spaces/Capabilities/DescriptorInspector",
  component: DescriptorInspector,
  args: { target, fetch: async () => ({ ok: true, data: descriptor() }) },
  decorators: [
    (Story: () => unknown) => (
      <div class="kria-shell" data-window-mode="standard" style={{ "max-width": "420px", padding: "24px" }}>
        {Story() as never}
      </div>
    ),
  ],
} satisfies Meta<typeof DescriptorInspector>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Verified: Story = {
  render: () => (
    <DescriptorInspector target={target} fetch={async () => ({ ok: true, data: descriptor() })} />
  ),
};

export const ElevatedUnsigned: Story = {
  render: () => (
    <DescriptorInspector
      target={target}
      fetch={async () => ({
        ok: true,
        data: descriptor({
          name: "Shell command",
          description: "Runs an arbitrary shell command on the host.",
          effectClasses: ["process.spawn", "filesystem.write"],
          reversible: "no",
          idempotent: false,
          elevated: true,
          trustTier: null,
          signed: false,
        }),
      })}
    />
  ),
};

export const ErrorState: Story = {
  render: () => (
    <DescriptorInspector
      target={target}
      fetch={async () => ({ ok: false, message: "Descriptor service unavailable." })}
    />
  ),
};
