import type { Meta, StoryObj } from "storybook-solidjs-vite";
import { Textarea } from "./Textarea";

const meta = {
  title: "Kit/Textarea",
  component: Textarea,
  args: { label: "Notes", placeholder: "Write something…", rows: 4 },
} satisfies Meta<typeof Textarea>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {};
export const Disabled: Story = { args: { disabled: true, defaultValue: "Locked" } };
export const Invalid: Story = { args: { errorMessage: "This field is required" } };
export const AutoResize: Story = { args: { autoResize: true, label: "Grows" } };
