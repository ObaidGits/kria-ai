import type { Meta, StoryObj } from "storybook-solidjs-vite";
import { Popover } from "./Popover";

const meta = {
  title: "Kit/Popover",
  component: Popover,
} satisfies Meta<typeof Popover>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {
  args: { triggerLabel: "Open popover", children: <span>Popover content</span> },
  render: () => (
    <Popover triggerLabel="Open popover" title="Details">
      <p style={{ margin: 0, color: "var(--color-text-secondary)" }}>
        Popover content lives on a floating aura-glass layer.
      </p>
    </Popover>
  ),
};

export const IconTrigger: Story = {
  args: { triggerLabel: "Info", children: <span>About</span> },
  render: () => (
    <Popover triggerLabel="Info" triggerIcon="info" title="About">
      <p style={{ margin: 0 }}>An icon-triggered popover.</p>
    </Popover>
  ),
};
