import type { Meta, StoryObj } from "storybook-solidjs-vite";
import { StatusDot } from "./StatusDot";

const meta = {
  title: "Kit/StatusDot",
  component: StatusDot,
  argTypes: {
    tone: { control: "select", options: ["online", "busy", "error", "info", "offline"] },
    hideLabel: { control: "boolean" },
    pulse: { control: "boolean" },
  },
  args: { tone: "online", label: "Online" },
} satisfies Meta<typeof StatusDot>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {};
export const DotOnly: Story = { args: { hideLabel: true } };
export const Pulse: Story = { args: { pulse: true, tone: "busy", label: "Running" } };

export const AllTones: Story = {
  render: () => (
    <div style={{ display: "flex", "flex-direction": "column", gap: "8px" }}>
      <StatusDot tone="online" label="Online" />
      <StatusDot tone="busy" label="Busy" />
      <StatusDot tone="error" label="Error" />
      <StatusDot tone="info" label="Info" />
      <StatusDot tone="offline" label="Offline" />
    </div>
  ),
};
