import type { Meta, StoryObj } from "storybook-solidjs-vite";
import ConverseSpace from "./ConverseSpace";
import { converseStore, coreStore } from "../../stores";
import type { Message, WorkBlock } from "../../stores/converseStore";

/**
 * Converse Space three-lane layout (task 3.1). The Space reads global stores,
 * so each story seeds converseStore/coreStore before rendering. Wrapped in a
 * fixed-height `.kria-shell` so the grid + sticky Composer read correctly in
 * the workbench.
 */
const meta = {
  title: "Spaces/ConverseSpace",
  component: ConverseSpace,
  decorators: [
    (Story: () => unknown) => (
      <div class="kria-shell" data-window-mode="standard" style={{ height: "600px" }}>
        {Story() as never}
      </div>
    ),
  ],
} satisfies Meta<typeof ConverseSpace>;

export default meta;
type Story = StoryObj<typeof meta>;

function seedMessages(): Message[] {
  const now = Date.now();
  return [
    { id: "m1", threadId: "t1", role: "user", content: "Summarize today's notes.", timestamp: now },
    {
      id: "m2",
      threadId: "t1",
      role: "assistant",
      content:
        "Here's a summary of today's notes. The conversation stays visually dominant over the work and context lanes.",
      timestamp: now + 1,
    },
  ];
}

function seedWorkBlocks(): WorkBlock[] {
  const now = Date.now();
  return [
    { id: "wb1", type: "reasoning", status: "completed", summary: "Reviewed 3 notes", startedAt: now },
    { id: "wb2", type: "tool-call", status: "running", summary: "Searching memory…", startedAt: now },
  ];
}

/** Cold empty state — Core-forward placeholder + sticky Composer. */
export const Empty: Story = {
  render: () => {
    converseStore.clearMessages();
    coreStore.reset();
    return <ConverseSpace />;
  },
};

/** Active conversation with the WorkLane revealed by work activity. */
export const WithWorkLane: Story = {
  render: () => {
    converseStore.clearMessages();
    coreStore.reset();
    converseStore.setThreads([
      { id: "t1", title: "Daily notes", createdAt: Date.now(), updatedAt: Date.now(), pinned: false, archived: false, temporary: false },
    ]);
    for (const m of seedMessages()) converseStore.addMessage(m);
    for (const b of seedWorkBlocks()) converseStore.addWorkBlock(b);
    return <ConverseSpace />;
  },
};
