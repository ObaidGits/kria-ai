import type { Meta, StoryObj } from "storybook-solidjs-vite";

import TrustIndicator from "./TrustIndicator";
import { Room } from "./Room";
import { CorePresence } from "../../../components/CorePresence";
import { coreStore } from "../../../stores";
import type { CoreState } from "../../../stores/coreStore";

/**
 * TrustIndicator — the muted on-device / local-first trust affordance (design.md
 * §9, Requirement 9). These stories drive the real component (task 8.6) with an
 * injected connectivity + Core state so the workbench stays deterministic.
 *
 *   • Online / Offline — the confirmation STAYS LIT either way (offline is
 *     healthy for a local-first app, never an error — Req 9.1).
 *   • Reaching — while KRIA acts on the device (`acting` / `running-automation`)
 *     a directed Core→edge reach cue lights (Req 9.1).
 *   • The cue is always MUTED/non-emerald (Req 9.2); activation routes to the
 *     Memory & Privacy Settings group (Req 9.3, logged here — routing only).
 *
 * Requirements: 9.1, 9.2, 9.3 (and 16.4 — workbench parity).
 */
const meta = {
  title: "Spaces/Home/TrustIndicator",
  component: TrustIndicator,
  decorators: [
    (Story: () => unknown) => (
      <div class="kria-shell" data-window-mode="immersive" style={{ height: "600px" }}>
        {Story() as never}
      </div>
    ),
  ],
} satisfies Meta<typeof TrustIndicator>;

export default meta;
type Story = StoryObj<typeof meta>;

const log = (route: unknown) => console.info("[Trust story] route only →", route);

function stage(online: boolean, state: CoreState, reducedMotion = false) {
  coreStore.reset();
  return (
    <Room>
      <div
        style={{
          flex: "1 1 auto",
          display: "flex",
          "flex-direction": "column",
          "align-items": "center",
          "justify-content": "center",
          gap: "24px",
          padding: "24px",
        }}
      >
        <CorePresence size="lg" />
        <TrustIndicator
          online={() => online}
          coreState={() => state}
          reducedMotion={reducedMotion}
          onNavigate={log}
        />
      </div>
    </Room>
  );
}

/** Online + at rest — lit, muted, no reach. */
export const OnlineResting: Story = {
  render: () => stage(true, "idle"),
};

/** Offline — STILL LIT + a calm "Offline" hint (never an error state, Req 9.1). */
export const OfflineStillLit: Story = {
  render: () => stage(false, "idle"),
};

/** Acting on the device — the Core→edge reach cue lights (Req 9.1). */
export const ReachingWhileActing: Story = {
  render: () => stage(true, "acting"),
};

/** Running an automation on the device — reach cue lights. */
export const ReachingWhileAutomating: Story = {
  render: () => stage(true, "running-automation"),
};

/** Reduced motion — the reach degrades to a static cue (Req 17.4/21.4). */
export const ReducedMotionStatic: Story = {
  render: () => stage(true, "acting", true),
};
