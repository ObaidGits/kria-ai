import type { Meta, StoryObj } from "storybook-solidjs-vite";
import MachinesSpace from "./MachinesSpace";

/**
 * MachinesSpace (task 9.1) — the Machines Space shell: fleet matrix, terminal,
 * alerts, and the enrollment wizard (Req 8.1). In the workbench there is no
 * fleet controller, so the Space renders its honest empty / idle states
 * (Req 20.4). Populated fleet components live in the FleetComponents stories.
 */
const meta = {
  title: "Spaces/Machines/MachinesSpace",
  component: MachinesSpace,
  decorators: [
    (Story: () => unknown) => (
      <div class="kria-shell" data-window-mode="standard" style={{ "max-width": "980px", padding: "24px" }}>
        {Story() as never}
      </div>
    ),
  ],
} satisfies Meta<typeof MachinesSpace>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {
  render: () => <MachinesSpace />,
};
