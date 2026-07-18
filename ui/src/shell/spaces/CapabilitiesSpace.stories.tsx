import type { Meta, StoryObj } from "storybook-solidjs-vite";
import { onMount } from "solid-js";
import CapabilitiesSpace from "./CapabilitiesSpace";
import { capabilityStore } from "../../stores";
import type { Capability, SkillView, Provider, ModelView, IntegrationView } from "../../stores";

/**
 * CapabilitiesSpace (task 8.1) — the six-segment Capabilities Space (Req 7.1)
 * with the descriptor Inspector (Req 7.2). Stories seed `capabilityStore` and
 * stub the loaders so the workbench renders without a backend.
 */
function seed() {
  // In the browser workbench there is no Tauri runtime, so the loaders degrade
  // gracefully and never overwrite this seeded data (they only set on success).
  capabilityStore.setCapabilities([
    {
      id: "native:web-search",
      name: "Web search",
      type: "tool",
      status: "active",
      description: "Search the web and return ranked results.",
      source: "native",
      riskLevel: "green",
      providerId: "native",
      capabilityId: "web-search",
      tags: ["web"],
      elevated: false,
    } satisfies Capability,
    {
      id: "shell:exec",
      name: "Shell command",
      type: "tool",
      status: "active",
      description: "Run a shell command on the host.",
      source: "native",
      riskLevel: "yellow",
      providerId: "native",
      capabilityId: "shell-exec",
      tags: ["system"],
      elevated: true,
    } satisfies Capability,
  ]);
  capabilityStore.setSkills([
    {
      slug: "pdf",
      name: "PDF reader",
      description: "Extract text and tables from PDFs.",
      category: "productivity",
      trustTier: "verified",
      installed: true,
      enabled: true,
    } satisfies SkillView,
  ]);
  capabilityStore.setProviders([
    { id: "local", name: "Local llama", type: "local", active: true } satisfies Provider,
  ]);
  capabilityStore.setModels([
    { id: "qwen", name: "Qwen2.5-7B", provider: "local", detail: "32k ctx" } satisfies ModelView,
  ]);
  capabilityStore.setIntegrations([
    {
      id: "mcp:fs",
      name: "Filesystem MCP",
      kind: "mcp",
      status: "connected",
      detail: "4 tools",
    } satisfies IntegrationView,
  ]);
  capabilityStore.setActiveSegment("tools");
}

const meta = {
  title: "Spaces/Capabilities/CapabilitiesSpace",
  component: CapabilitiesSpace,
  decorators: [
    (Story: () => unknown) => {
      seed();
      return (
        <div
          class="kria-shell"
          data-window-mode="standard"
          style={{ "max-width": "900px", padding: "24px" }}
        >
          {(() => {
            onMount(seed);
            return Story() as never;
          })()}
        </div>
      );
    },
  ],
} satisfies Meta<typeof CapabilitiesSpace>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {
  render: () => <CapabilitiesSpace />,
};
