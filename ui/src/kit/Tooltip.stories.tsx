import type { Meta, StoryObj } from "storybook-solidjs-vite";
import { Tooltip } from "./Tooltip";
import { IconButton } from "./IconButton";

const meta = {
  title: "Kit/Tooltip",
  component: Tooltip,
} satisfies Meta<typeof Tooltip>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {
  args: { content: "Open settings", children: <span>Settings</span> },
  render: () => (
    <div style={{ padding: "60px" }}>
      <Tooltip content="Open settings">
        <IconButton icon="settings" label="Settings" />
      </Tooltip>
    </div>
  ),
};

export const OpenByDefault: Story = {
  args: { content: "Always visible in this story", children: <span>Info</span> },
  render: () => (
    <div style={{ padding: "60px" }}>
      <Tooltip content="Always visible in this story" defaultOpen openDelay={0}>
        <IconButton icon="info" label="Info" />
      </Tooltip>
    </div>
  ),
};
