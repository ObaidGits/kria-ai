import type { Meta, StoryObj } from "storybook-solidjs-vite";
import { createSignal } from "solid-js";
import { SegmentBar } from "./SegmentBar";

const options = [
  { value: "calm", label: "Calm" },
  { value: "focused", label: "Focused" },
  { value: "dense", label: "Dense" },
];

const meta = {
  title: "Kit/SegmentBar",
  component: SegmentBar,
  args: { label: "Density", options },
} satisfies Meta<typeof SegmentBar>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {
  render: () => {
    const [v, setV] = createSignal("focused");
    return <SegmentBar label="Density" options={options} value={v()} onChange={setV} />;
  },
};

export const Disabled: Story = {
  args: { disabled: true, defaultValue: "calm" },
};
