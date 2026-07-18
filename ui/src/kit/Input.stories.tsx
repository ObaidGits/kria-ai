import type { Meta, StoryObj } from "storybook-solidjs-vite";
import { Input } from "./Input";

const meta = {
  title: "Kit/Input",
  component: Input,
  args: { label: "Name", placeholder: "Type here…" },
} satisfies Meta<typeof Input>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {};
export const WithValue: Story = { args: { defaultValue: "KRIA" } };
export const Disabled: Story = { args: { disabled: true, defaultValue: "Locked" } };
export const Invalid: Story = {
  args: { defaultValue: "bad", errorMessage: "Value is not valid" },
};
export const HiddenLabel: Story = {
  args: { label: "Search query", hideLabel: true, placeholder: "Search" },
};
