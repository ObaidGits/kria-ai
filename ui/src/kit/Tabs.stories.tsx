import type { Meta, StoryObj } from "storybook-solidjs-vite";
import { Tabs } from "./Tabs";

const items = [
  { value: "run", label: "Run", content: () => <p>Run workflows here.</p> },
  { value: "build", label: "Build", content: () => <p>Build the node canvas.</p> },
  { value: "history", label: "History", content: () => <p>Past runs.</p> },
  { value: "locked", label: "Locked", content: () => <p>Nope.</p>, disabled: true },
];

const meta = {
  title: "Kit/Tabs",
  component: Tabs,
  args: { items },
} satisfies Meta<typeof Tabs>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {};
export const SecondSelected: Story = { args: { defaultValue: "build" } };
