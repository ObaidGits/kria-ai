import type { Meta, StoryObj } from "storybook-solidjs-vite";
import { Table } from "./Table";

const meta = { title: "Kit/Table", component: Table } satisfies Meta<typeof Table>;
export default meta;
type Story = StoryObj<typeof meta>;

export const Basic: Story = {
  args: {},
  render: () => <Table><caption>Example</caption><tbody><tr><td>KRIA</td><td>Ready</td></tr></tbody></Table>,
};
