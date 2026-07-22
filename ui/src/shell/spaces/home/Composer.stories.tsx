import type { Meta, StoryObj } from "storybook-solidjs-vite";

import HomeComposer from "./Composer";
import { Room } from "./Room";
import { CorePresence } from "../../../components/CorePresence";
import { createSharedLightPublisher } from "./sharedLight";
import { converseStore, coreStore } from "../../../stores";
import { createRoot } from "solid-js";

/**
 * Composer (homepage) — the unified action target on the true vertical center
 * axis (design.md §2, task 5.1). These stories exercise the real component
 * inside the `Room` beneath a `CorePresence`, matching the homepage composition:
 * the Core offset in the upper third, the Composer centered on the vertical
 * axis (design §2).
 *
 * The stories mount the shared-light publisher so the Composer's rim-light
 * reacts to the live Core hue/intensity, and the ⌘K hint is stubbed to a no-op
 * logger (routing/discoverability only — it never sends or executes, Req 4.2).
 *
 * Requirements: 4.1, 4.2, 4.3 (and 16.4 — workbench parity).
 */
const meta = {
  title: "Spaces/Home/Composer",
  component: HomeComposer,
  decorators: [
    (Story: () => unknown) => (
      <div class="kria-shell" data-window-mode="immersive" style={{ height: "640px" }}>
        {Story() as never}
      </div>
    ),
  ],
} satisfies Meta<typeof HomeComposer>;

export default meta;
type Story = StoryObj<typeof meta>;

/** Wrap the Composer in the Room + Core like the real homepage vertical axis. */
function stage(node: unknown) {
  coreStore.reset();
  // Publish the shared light so the Composer rim reacts to the Core.
  createRoot(() => createSharedLightPublisher());
  return (
    <Room>
      <div
        style={{
          flex: "1 1 auto",
          display: "flex",
          "flex-direction": "column",
          "align-items": "center",
          "justify-content": "center",
          gap: "48px",
          padding: "24px",
          width: "100%",
        }}
      >
        <CorePresence size="lg" interactive />
        {node as never}
      </div>
    </Room>
  );
}

const onOpenPalette = () => console.info("[Composer story] open Command Palette (routing only)");

/** Resting homepage: one unified Composer on the vertical axis + ⌘K hint. */
export const Rest: Story = {
  render: () => {
    converseStore.updateDraft({ text: "", attachments: [] });
    return stage(<HomeComposer onOpenPalette={onOpenPalette} />);
  },
};

/** With a staged draft — e.g. a Contextual Chip populated the shared draft. */
export const WithStagedDraft: Story = {
  render: () => {
    converseStore.updateDraft({ text: "Draft: summarize today's meeting notes." });
    return stage(<HomeComposer onOpenPalette={onOpenPalette} />);
  },
};
