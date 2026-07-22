import type { Meta, StoryObj } from "storybook-solidjs-vite";
import Room from "./Room";
import { CorePresence } from "../../../components/CorePresence";
import { coreStore } from "../../../stores";

/**
 * Room — the homepage environment (design.md §3 & §14, Requirement 1).
 *
 * These stories exercise the real `Room` component (task 1.1): the four
 * back-to-front environment layers (base gradient, particle field, floor
 * sheen, peripheral darkness) as pure presentation. The Room only *consumes*
 * the shared-light `--core-*` variables; publication from the Core render tick
 * is owned by task 1.2, so here the token defaults render a correct resting
 * Room. A `CorePresence` is placed in the content plane to show how the light
 * pool and floor sheen sit beneath the Core.
 *
 * Wrapped in a fixed-height immersive shell so the full-bleed layout reads
 * correctly in the workbench (matches the HomeSpace story convention).
 *
 * Requirements: 1.1, 1.2, 1.3, 1.6 (and 16.4 — workbench parity).
 */
const meta = {
  title: "Spaces/Home/Room",
  component: Room,
  decorators: [
    (Story: () => unknown) => (
      <div class="kria-shell" data-window-mode="immersive" style={{ height: "600px" }}>
        {Story() as never}
      </div>
    ),
  ],
} satisfies Meta<typeof Room>;

export default meta;
type Story = StoryObj<typeof meta>;

/** Resting Room — full atmosphere, Core idling in the light pool (Req 1.1). */
export const Rest: Story = {
  render: () => {
    coreStore.reset();
    return (
      <Room>
        <div style={{ flex: "1 1 auto", display: "flex", "align-items": "center", "justify-content": "center" }}>
          <CorePresence size="lg" />
        </div>
      </Room>
    );
  },
};

/** Sparse field — the degrade ladder sheds particles first under load (§11.5). */
export const SparseParticles: Story = {
  render: () => {
    coreStore.reset();
    return <Room particleCount={8} />;
  },
};

/** No particles — floor sheen + vignette only. */
export const NoParticles: Story = {
  render: () => {
    coreStore.reset();
    return <Room particleCount={0} />;
  },
};

/** Reduced-motion — static frame, same layout/colors, drift frozen (Req 1.6). */
export const ReducedMotion: Story = {
  render: () => {
    coreStore.reset();
    return <Room reducedMotion />;
  },
};

/** Degraded — flat neutral base only (failure / very-low-capability, §14). */
export const Degraded: Story = {
  render: () => {
    coreStore.reset();
    return <Room degraded />;
  },
};
