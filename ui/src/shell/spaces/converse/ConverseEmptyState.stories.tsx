import type { Meta, StoryObj } from "storybook-solidjs-vite";
import ConverseEmptyState from "./ConverseEmptyState";
import { converseStore } from "../../../stores";

/**
 * ConverseEmptyState — the Core-forward cold/warm empty state (task 3.6, Req 4.6).
 * The component reads converseStore.threads() to decide cold vs warm, so each
 * story seeds the store first. Wrapped in a fixed-height shell so the centered
 * layout reads correctly in the workbench.
 */
const meta = {
  title: "Spaces/Converse/ConverseEmptyState",
  component: ConverseEmptyState,
  decorators: [
    (Story: () => unknown) => (
      <div class="kria-shell" data-window-mode="standard" style={{ height: "560px" }}>
        {Story() as never}
      </div>
    ),
  ],
} satisfies Meta<typeof ConverseEmptyState>;

export default meta;
type Story = StoryObj<typeof meta>;

/** COLD — first ever: Core forward + ≤3 example intents (ask/automate/remember). */
export const Cold: Story = {
  render: () => {
    converseStore.setThreads([]);
    return <ConverseEmptyState />;
  },
};

/** WARM — returning: Core at rest + ≤3 continue-suggestions from recent threads. */
export const Warm: Story = {
  render: () => {
    const now = Date.now();
    converseStore.setThreads([
      { id: "t1", title: "Daily notes", createdAt: now, updatedAt: now, pinned: false, archived: false, temporary: false },
      { id: "t2", title: "Trip plan for March", createdAt: now, updatedAt: now - 1000, pinned: false, archived: false, temporary: false },
      { id: "t3", title: "Q2 budget review", createdAt: now, updatedAt: now - 2000, pinned: false, archived: false, temporary: false },
    ]);
    return <ConverseEmptyState />;
  },
};
