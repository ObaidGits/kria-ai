import type { Meta, StoryObj } from "storybook-solidjs-vite";
import { IconButton } from "./IconButton";

const meta = {
  title: "Kit/IconButton",
  component: IconButton,
  argTypes: {
    variant: { control: "select", options: ["ghost", "solid", "danger"] },
    size: { control: "select", options: ["sm", "md", "lg"] },
    disabled: { control: "boolean" },
  },
  args: { icon: "settings", label: "Settings", variant: "ghost", size: "md" },
} satisfies Meta<typeof IconButton>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {};
export const Solid: Story = { args: { variant: "solid", icon: "search", label: "Search" } };
export const Danger: Story = { args: { variant: "danger", icon: "trash-2", label: "Delete" } };
export const Disabled: Story = { args: { disabled: true } };

export const Sizes: Story = {
  render: () => (
    <div style={{ display: "flex", gap: "12px", "align-items": "center" }}>
      <IconButton icon="x" label="Close" size="sm" />
      <IconButton icon="x" label="Close" size="md" />
      <IconButton icon="x" label="Close" size="lg" />
    </div>
  ),
};
