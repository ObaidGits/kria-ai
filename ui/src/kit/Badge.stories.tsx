import type { Meta, StoryObj } from "storybook-solidjs-vite";
import { Badge } from "./Badge";

const meta = {
  title: "Kit/Badge",
  component: Badge,
  argTypes: {
    tone: {
      control: "select",
      options: ["neutral", "accent", "success", "warning", "danger", "info"],
    },
  },
  args: { children: "Badge", tone: "neutral" },
} satisfies Meta<typeof Badge>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {};

export const Tones: Story = {
  render: () => (
    <div style={{ display: "flex", gap: "8px" }}>
      <Badge tone="neutral">Neutral</Badge>
      <Badge tone="accent">Accent</Badge>
      <Badge tone="success">Success</Badge>
      <Badge tone="warning">Warning</Badge>
      <Badge tone="danger">Danger</Badge>
      <Badge tone="info">Info</Badge>
    </div>
  ),
};
