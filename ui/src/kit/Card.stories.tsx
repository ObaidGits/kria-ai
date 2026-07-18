import type { Meta, StoryObj } from "storybook-solidjs-vite";
import { Card } from "./Card";

const meta = {
  title: "Kit/Card",
  component: Card,
} satisfies Meta<typeof Card>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {
  render: () => (
    <Card>
      <h3 style={{ margin: 0, color: "var(--color-text-primary)" }}>Title</h3>
      <p style={{ color: "var(--color-text-secondary)" }}>Some card content.</p>
    </Card>
  ),
};

export const Elevated: Story = {
  render: () => <Card variant="elevated">Elevated surface</Card>,
};

export const Interactive: Story = {
  render: () => (
    <Card interactive aria-label="Open details" onClick={() => alert("clicked")}>
      Click or focus me
    </Card>
  ),
};
