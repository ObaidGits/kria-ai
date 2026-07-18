import type { Meta, StoryObj } from "storybook-solidjs-vite";
import { CapabilityRow } from "./CapabilityRow";
import type { Capability } from "../../../stores";

/**
 * CapabilityRow (task 8.1) — a tool/capability row in the Tools segment.
 * Selecting it opens the descriptor in the shared Inspector; these stories use
 * an `onInspect` override so the workbench logs the selection instead of
 * mutating the global shell store. Risk is icon + text (Req 17.3).
 */
function cap(over: Partial<Capability> = {}): Capability {
  return {
    id: "prov:web-search",
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
    ...over,
  };
}

const log = (c: Capability) => console.log("inspect", c.id);

const meta = {
  title: "Spaces/Capabilities/CapabilityRow",
  component: CapabilityRow,
  args: { capability: cap(), onInspect: log },
  decorators: [
    (Story: () => unknown) => (
      <ul
        class="kria-shell kria-capabilities__list"
        data-window-mode="standard"
        style={{ "max-width": "560px", padding: "24px", "list-style": "none" }}
      >
        {Story() as never}
      </ul>
    ),
  ],
} satisfies Meta<typeof CapabilityRow>;

export default meta;
type Story = StoryObj<typeof meta>;

export const LowRisk: Story = {
  render: () => <CapabilityRow capability={cap()} onInspect={log} />,
};

export const Elevated: Story = {
  render: () => (
    <CapabilityRow
      capability={cap({
        id: "shell:exec",
        name: "Shell command",
        description: "Run a shell command on the host.",
        riskLevel: "yellow",
        elevated: true,
      })}
      onInspect={log}
    />
  ),
};

export const HighRisk: Story = {
  render: () => (
    <CapabilityRow
      capability={cap({
        id: "fs:delete",
        name: "Delete files",
        description: "Permanently remove files from disk.",
        riskLevel: "red",
        elevated: true,
      })}
      onInspect={log}
    />
  ),
};
