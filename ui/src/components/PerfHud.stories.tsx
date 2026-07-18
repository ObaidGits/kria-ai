import type { Meta, StoryObj } from "storybook-solidjs-vite";
import PerfHud from "./PerfHud";
import { clearMeasures, measureSince } from "../utils/perf";

/**
 * Workbench story for the dev-gated perf HUD. Seeds a few measures (one over
 * budget) so the HUD has content to display in the canvas.
 */
const meta = {
  title: "Diagnostics/PerfHud",
  component: PerfHud,
} satisfies Meta<typeof PerfHud>;

export default meta;
type Story = StoryObj<typeof meta>;

export const WithMeasures: Story = {
  render: () => {
    clearMeasures();
    const now = performance.now();
    measureSince("palette-open", now - 40); // within budget
    measureSince("first-token", now - 30); // within budget
    measureSince("space-switch", now - 500); // over the 150ms budget
    return <PerfHud />;
  },
};
