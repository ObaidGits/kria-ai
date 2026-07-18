import type { Meta, StoryObj } from "storybook-solidjs-vite";
import { Menu } from "./Menu";

const items = [
  { id: "copy", label: "Copy", icon: "copy", onSelect: () => {} },
  { id: "retry", label: "Retry", icon: "refresh-cw", onSelect: () => {} },
  { id: "sep", separator: true },
  { id: "remember", label: "Remember", icon: "brain", onSelect: () => {} },
  { id: "delete", label: "Delete", icon: "trash-2", disabled: true },
];

const meta = {
  title: "Kit/Menu",
  component: Menu,
} satisfies Meta<typeof Menu>;

export default meta;
type Story = StoryObj<typeof meta>;

export const IconTrigger: Story = {
  args: { triggerLabel: "Message actions", items },
  render: () => (
    <Menu
      label="Message actions"
      triggerLabel="Message actions"
      triggerIcon="more-horizontal"
      items={items}
    />
  ),
};

export const TextTrigger: Story = {
  args: { triggerLabel: "Actions", items },
  render: () => <Menu triggerLabel="Actions" items={items} />,
};
