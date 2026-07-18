import type { Meta, StoryObj } from "storybook-solidjs-vite";
import { createSignal } from "solid-js";
import { Chip } from "./Chip";

const meta = {
  title: "Kit/Chip",
  component: Chip,
} satisfies Meta<typeof Chip>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Static: Story = { render: () => <Chip>Label</Chip> };

export const Toggle: Story = {
  render: () => {
    const [on, setOn] = createSignal(false);
    return (
      <Chip selected={on()} onToggle={() => setOn((v) => !v)}>
        Filter
      </Chip>
    );
  },
};

export const Removable: Story = {
  render: () => (
    <Chip onRemove={() => alert("removed")} removeLabel="Remove tag">
      Tag
    </Chip>
  ),
};

export const Disabled: Story = {
  render: () => (
    <Chip onToggle={() => {}} disabled>
      Disabled
    </Chip>
  ),
};
