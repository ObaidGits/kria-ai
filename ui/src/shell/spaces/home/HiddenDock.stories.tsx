import type { Meta, StoryObj } from "storybook-solidjs-vite";
import { onCleanup } from "solid-js";

import { HiddenDock } from "../../HiddenDock";
import { Room } from "./Room";
import { CorePresence } from "../../../components/CorePresence";
import { coreStore, homeStore } from "../../../stores";

/**
 * HiddenDock — the presence-homepage navigation rail (design.md §7.1,
 * Requirement 7). Exercises the REAL component (task 6.1) inside the Room +
 * Core, matching the homepage composition (design §2): the rail is invisible at
 * rest and reveals over a dimmed Room only on intent (left-edge cursor, Alt,
 * ⌘K, pin, or keyboard/AT focus entering).
 *
 * Try it in the workbench: move the cursor to the LEFT EDGE, hold ALT, or press
 * TAB to focus into the rail — each reveals it over the dimmed Room. Escape /
 * blur dismisses it (unless pinned). The seven Spaces, their canonical order,
 * and `aria-current` are inherited verbatim from `Dock`.
 *
 * Requirements: 7.1, 7.2, 7.3, 7.4, 7.5, 14.4, 14.5 (and 16.4 — workbench parity).
 */
const meta = {
  title: "Spaces/Home/HiddenDock",
  component: HiddenDock,
  decorators: [
    (Story: () => unknown) => {
      // Reset dock state so each story starts hidden + unpinned.
      homeStore.reset();
      coreStore.reset();
      onCleanup(() => homeStore.reset());
      return (
        <div class="kria-shell" data-window-mode="standard" style={{ height: "600px", position: "relative", overflow: "hidden" }}>
          <Room>
            <div
              style={{
                flex: "1 1 auto",
                display: "flex",
                "align-items": "center",
                "justify-content": "center",
                padding: "24px",
              }}
            >
              <CorePresence size="lg" />
            </div>
          </Room>
          {Story() as never}
        </div>
      );
    },
  ],
} satisfies Meta<typeof HiddenDock>;

export default meta;
type Story = StoryObj<typeof meta>;

/** Hidden at rest — reveal by left-edge cursor, Alt, ⌘K, or Tab focus (Req 7.1). */
export const Hidden: Story = {
  render: () => <HiddenDock />,
};

/** Pinned open — the rail stays revealed until an explicit unpin (Req 7.1). */
export const Pinned: Story = {
  render: () => {
    homeStore.setDockPinned(true);
    return <HiddenDock />;
  },
};
