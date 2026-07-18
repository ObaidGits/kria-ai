import type { Meta, StoryObj } from "storybook-solidjs-vite";
import { Search } from "./Search";

const meta = {
  title: "Kit/Search",
  component: Search,
  args: { placeholder: "Search everything…" },
} satisfies Meta<typeof Search>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {};
export const WithVisibleLabel: Story = { args: { label: "Find", showLabel: true } };
export const Disabled: Story = { args: { disabled: true, defaultValue: "query" } };
