import type { Meta, StoryObj } from "storybook-solidjs-vite";
import { MemoryCard } from "./MemoryCard";
import type { MemoryFact } from "../../../stores";

/**
 * MemoryCard (task 6.2) — the compact memory tile. Cues (confidence / worth /
 * staleness) are icon+text so meaning is never color-only (Req 17.3). Clicking
 * a card opens the shared Inspector; the stories use an `onOpen` override so the
 * workbench logs the selection instead of mutating the global shell store.
 */
function fact(over: Partial<MemoryFact> = {}): MemoryFact {
  const now = Date.now();
  return {
    id: "f1",
    content: "The user prefers dark mode and a calm, keyboard-first UI.",
    confidence: 0.92,
    worth: 0.7,
    staleness: 0.05,
    source: "conversation",
    createdAt: now,
    updatedAt: now,
    tags: ["preference"],
    ...over,
  };
}

const log = (f: MemoryFact) => console.log("open inspector for", f.id);

const meta = {
  title: "Spaces/Memory/MemoryCard",
  component: MemoryCard,
  args: { fact: fact(), onOpen: log },
  decorators: [
    (Story: () => unknown) => (
      <div class="kria-shell" data-window-mode="standard" style={{ "max-width": "520px", padding: "24px" }}>
        {Story() as never}
      </div>
    ),
  ],
} satisfies Meta<typeof MemoryCard>;

export default meta;
type Story = StoryObj<typeof meta>;

export const HighConfidence: Story = {
  render: () => <MemoryCard fact={fact()} onOpen={log} />,
};

export const LowConfidenceStale: Story = {
  render: () => (
    <MemoryCard
      fact={fact({
        id: "f2",
        content: "The user might live in Berlin (unconfirmed).",
        confidence: 0.25,
        worth: 0.2,
        staleness: 0.85,
        source: "inference",
      })}
      onOpen={log}
    />
  ),
};

export const Selected: Story = {
  render: () => <MemoryCard fact={fact()} selected onOpen={log} />,
};
