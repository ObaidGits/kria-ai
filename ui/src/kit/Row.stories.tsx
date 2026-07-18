import type { Meta, StoryObj } from "storybook-solidjs-vite";
import { Row } from "./Row";
import { Icon } from "../components/Icon";
import { Badge } from "./Badge";

const meta = {
  title: "Kit/Row",
  component: Row,
} satisfies Meta<typeof Row>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Static: Story = {
  render: () => <Row title="Static row" subtitle="Not interactive" />,
};

export const Selectable: Story = {
  render: () => (
    <div style={{ display: "flex", "flex-direction": "column", gap: "4px", width: "320px" }}>
      <Row
        title="Converse"
        subtitle="AI workspace"
        leading={<Icon name="message-circle" />}
        onSelect={() => {}}
        selected
      />
      <Row
        title="Memory"
        subtitle="What KRIA knows"
        leading={<Icon name="brain" />}
        trailing={<Badge tone="info">12</Badge>}
        onSelect={() => {}}
      />
    </div>
  ),
};

export const Disabled: Story = {
  render: () => <Row title="Disabled" onSelect={() => {}} disabled />,
};
