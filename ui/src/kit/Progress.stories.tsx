import type { Meta, StoryObj } from "storybook-solidjs-vite";
import { Progress } from "./Progress";

const meta = {
  title: "Kit/Progress",
  component: Progress,
  argTypes: {
    tone: { control: "select", options: ["accent", "success", "warning", "danger"] },
    indeterminate: { control: "boolean" },
  },
  args: { label: "Uploading", value: 45, minValue: 0, maxValue: 100 },
} satisfies Meta<typeof Progress>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Determinate: Story = {};
export const Success: Story = { args: { tone: "success", value: 100, label: "Complete" } };
export const Warning: Story = { args: { tone: "warning", value: 80, label: "Disk" } };
export const Indeterminate: Story = { args: { indeterminate: true, label: "Working" } };
