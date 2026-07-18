import type { Meta, StoryObj } from "storybook-solidjs-vite";
import MemorySpace from "./MemorySpace";
import { memoryStore } from "../../stores";
import type { MemoryFact, KnowledgeDocument } from "../../stores";
import { navigate } from "../router";

/**
 * Memory Space landing + segment scaffolding (task 6.1). The Space reads the
 * global memoryStore + router, so each story seeds them before rendering.
 * Wrapped in a fixed-height `.kria-shell` so the tablist + region scroll read
 * correctly in the workbench.
 */
const meta = {
  title: "Spaces/MemorySpace",
  component: MemorySpace,
  decorators: [
    (Story: () => unknown) => (
      <div
        class="kria-shell"
        data-window-mode="standard"
        style={{ height: "600px", padding: "24px" }}
      >
        {Story() as never}
      </div>
    ),
  ],
} satisfies Meta<typeof MemorySpace>;

export default meta;
type Story = StoryObj<typeof meta>;

function seedFacts(): MemoryFact[] {
  const now = Date.now();
  return [
    {
      id: "f1",
      content: "The user prefers dark mode and a calm, keyboard-first UI.",
      confidence: 0.92,
      worth: 0.7,
      staleness: 0,
      source: "conversation",
      createdAt: now - 3600_000,
      updatedAt: now - 60_000,
      tags: ["preference", "ui"],
    },
    {
      id: "f2",
      content: "KRIA runs locally on the owner's laptop; no fleet yet.",
      confidence: 0.8,
      worth: 0.6,
      staleness: 0.1,
      source: "document",
      createdAt: now - 7200_000,
      updatedAt: now - 7200_000,
      tags: ["context"],
    },
  ];
}

function seedDocs(): KnowledgeDocument[] {
  return [
    { id: "d1", title: "UI Redesign Masterplan", type: "markdown", indexedAt: Date.now(), size: 42000 },
  ];
}

/** Cold landing — honest empty state, nothing learned yet. */
export const EmptyLanding: Story = {
  render: () => {
    memoryStore.setFacts([]);
    memoryStore.setDocuments([]);
    memoryStore.setSearchQuery("");
    memoryStore.setLoading(false);
    navigate("memory");
    return <MemorySpace />;
  },
};

/** Landing with overview stats + recent memories. */
export const LandingWithData: Story = {
  render: () => {
    memoryStore.setFacts(seedFacts());
    memoryStore.setDocuments(seedDocs());
    memoryStore.setSearchQuery("");
    memoryStore.setLoading(false);
    navigate("memory");
    return <MemorySpace />;
  },
};

/** Explorer lens with a basic MemoryCard list (full card is task 6.2). */
export const Explorer: Story = {
  render: () => {
    memoryStore.setFacts(seedFacts());
    memoryStore.setDocuments(seedDocs());
    memoryStore.setSearchQuery("");
    memoryStore.setLoading(false);
    navigate("memory", "explorer");
    return <MemorySpace />;
  },
};

/** A lens whose body arrives in a later task — honest placeholder. */
export const KnowledgeGraphPlaceholder: Story = {
  render: () => {
    memoryStore.setFacts(seedFacts());
    memoryStore.setLoading(false);
    navigate("memory", "knowledgegraph");
    return <MemorySpace />;
  },
};
