import type { Meta, StoryObj } from "storybook-solidjs-vite";
import HomeSpace from "./HomeSpace";
import { coreStore } from "../../../stores";

/**
 * HomeSpace — the presence homepage surface (design.md §14, Requirement 22).
 *
 * This story exercises the *scaffold* stage of HomeSpace (task 0.2): a
 * Core-forward, never-blank shell that the home surface routes to when the
 * `home.presence.v2` flag is ON. The Room, Focus UI, unified Composer, hybrid
 * navigation and 3D Core are owned by later tasks (1.x–9.x) and are NOT built
 * here — see the sibling `HomepageScaffolds.stories.tsx` for their placeholder
 * workbench entries.
 *
 * HomeSpace reads global stores (currently `coreStore` via `CorePresence`), so
 * each story seeds `coreStore` before rendering — matching the seeding
 * convention in `ConverseSpace.stories.tsx` / `MemorySpace.stories.tsx`.
 * Wrapped in a fixed-height `.kria-shell` (immersive) so the presence layout
 * reads correctly in the workbench.
 *
 * Requirements: 16.4 (version design-system changes + update the component
 * workbench stories accordingly), 22.1, 22.2.
 */
const meta = {
  title: "Spaces/HomeSpace",
  component: HomeSpace,
  decorators: [
    (Story: () => unknown) => (
      <div class="kria-shell" data-window-mode="immersive" style={{ height: "600px" }}>
        {Story() as never}
      </div>
    ),
  ],
} satisfies Meta<typeof HomeSpace>;

export default meta;
type Story = StoryObj<typeof meta>;

/** Resting calm — the Core idles, no placeholder widgets (Req 1.5). */
export const Rest: Story = {
  render: () => {
    coreStore.reset();
    return <HomeSpace />;
  },
};

/** Working — the Core reflects a live "thinking" activity. */
export const Thinking: Story = {
  render: () => {
    coreStore.reset();
    coreStore.setState("thinking");
    return <HomeSpace />;
  },
};

/** Attention — the Core steps forward and calms while blocked (Req 3.3). */
export const Blocked: Story = {
  render: () => {
    coreStore.reset();
    coreStore.setBlocked("Approval needed to continue");
    return <HomeSpace />;
  },
};
