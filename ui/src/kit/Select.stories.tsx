import type { Meta, StoryObj } from "storybook-solidjs-vite";
import { Select } from "./Select";

const options = [
  { value: "assistant", label: "Assistant" },
  { value: "lab", label: "Lab" },
  { value: "coding", label: "Coding" },
  { value: "locked", label: "Locked", disabled: true },
];

const meta = {
  title: "Kit/Select",
  component: Select,
  args: { label: "Mode", options, placeholder: "Choose a mode" },
} satisfies Meta<typeof Select>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {};
export const WithValue: Story = { args: { defaultValue: "coding" } };
export const Disabled: Story = { args: { disabled: true } };
export const Invalid: Story = { args: { errorMessage: "Please choose a mode" } };
