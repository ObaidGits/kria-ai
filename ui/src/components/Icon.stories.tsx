import type { Meta, StoryObj } from "storybook-solidjs-vite";
import { Icon } from "./Icon";

/**
 * Example workbench story wiring an existing primitive (Icon) so the workbench
 * runs (design.md §1.20). Kit primitives added in task 0.4 follow this pattern.
 */
const meta = {
  title: "Primitives/Icon",
  component: Icon,
  argTypes: {
    name: { control: "text" },
    size: { control: "number" },
    title: { control: "text" },
  },
  args: {
    name: "check",
    size: 24,
  },
} satisfies Meta<typeof Icon>;

export default meta;
type Story = StoryObj<typeof meta>;

/** Default decorative icon (aria-hidden). */
export const Default: Story = {};

/** Icon with an accessible name (role="img"). */
export const Labeled: Story = {
  args: { name: "settings", title: "Settings", size: 24 },
};

/** Larger sizing via the `size` prop. */
export const Large: Story = {
  args: { name: "activity", size: 48 },
};

/** A small gallery to sanity-check sprite wiring across several icons. */
export const Gallery: Story = {
  render: () => (
    <div style={{ display: "flex", gap: "16px", "align-items": "center", color: "var(--color-text-primary)" }}>
      <Icon name="check" size={28} />
      <Icon name="settings" size={28} />
      <Icon name="activity" size={28} />
      <Icon name="search" size={28} />
      <Icon name="x" size={28} />
    </div>
  ),
};
