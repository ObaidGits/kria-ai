import type { Meta, StoryObj } from "storybook-solidjs-vite";
import { onCleanup } from "solid-js";
import { CognitionPanel } from "./CognitionPanel";
import { memoryStore, normalizeCognitionResult, type CognitionResult } from "../../../stores";

/**
 * CognitionPanel (task 6.3, Req 5.6) — the Memory Space Cognition segment.
 * Controls trigger the existing cognition commands; the result panel shows
 * WHAT CHANGED persistently (never a toast). Stories cover the three states:
 * idle (no results), running (a job in-flight), and result (history present,
 * including an honest failure).
 */
const meta = {
  title: "Spaces/Memory/CognitionPanel",
  component: CognitionPanel,
  decorators: [
    (Story: () => unknown) => (
      <div class="kria-shell" data-window-mode="standard" style={{ "max-width": "760px", padding: "24px" }}>
        {Story() as never}
      </div>
    ),
  ],
} satisfies Meta<typeof CognitionPanel>;

export default meta;
type Story = StoryObj<typeof meta>;

/** Reset the shared cognition read-model so each story starts clean. */
function reset() {
  memoryStore.seedCognitionResults([]);
  memoryStore.seedCognitionRunning([]);
  onCleanup(reset);
}

export const Idle: Story = {
  render: () => {
    reset();
    return <CognitionPanel />;
  },
};

export const Running: Story = {
  render: () => {
    reset();
    memoryStore.seedCognitionRunning(["reflect", "entity-extraction"]);
    return <CognitionPanel />;
  },
};

export const WithResults: Story = {
  render: () => {
    reset();
    const results: CognitionResult[] = [
      normalizeCognitionResult("reflect", 3),
      normalizeCognitionResult("dream", { procedures: 2, goals_merged: 1, worth_recalibrated: 4 }),
      normalizeCognitionResult("entity-extraction", { processed: 12, entities_linked: 5 }),
      {
        id: "cog-self-improvement-failed",
        job: "self-improvement",
        at: Date.now(),
        ok: false,
        changes: [],
        summary: "",
        message: "Self-improvement engine is offline.",
      },
    ];
    memoryStore.seedCognitionResults(results);
    return <CognitionPanel />;
  },
};
