import type { Meta, StoryObj } from "storybook-solidjs-vite";
import { createSignal } from "solid-js";

import ContextualOrbit from "./ContextualOrbit";
import { Room } from "./Room";
import { CorePresence } from "../../../components/CorePresence";
import { coreStore } from "../../../stores";
import type { OrbitPoint } from "../../../stores/homeFocusStore";
import type { Route } from "../../router";

/**
 * ContextualOrbit — partial, temporary capability-awareness light-points around
 * the Core (design.md §6.4, Requirement 6). These stories exercise the real
 * component (task 6.2) with an injected `orbit` accessor + explicit `engaged`
 * so the workbench stays deterministic and decoupled from the live domain
 * stores.
 *
 * Shown inside the `Room` around a `CorePresence` to match the homepage
 * composition (design §2/§6.4): body language around the Core, not a menu or a
 * permanent ring. Actionable points ROUTE ONLY (Req 6.4) — here `onNavigate` is
 * a stubbed no-op logger, so nothing sends or executes. At rest / disengaged
 * the Orbit renders NOTHING (Req 6.1); reduced motion degrades it to static
 * labelled dots (Req 6.6).
 *
 * Requirements: 6.1, 6.2, 6.3, 6.4, 6.5, 6.6 (and 16.4 — workbench parity).
 */
const meta = {
  title: "Spaces/Home/ContextualOrbit",
  component: ContextualOrbit,
  decorators: [
    (Story: () => unknown) => (
      <div class="kria-shell" data-window-mode="immersive" style={{ height: "600px" }}>
        {Story() as never}
      </div>
    ),
  ],
} satisfies Meta<typeof ContextualOrbit>;

export default meta;
type Story = StoryObj<typeof meta>;

/** A realistic partial set: memory (route), automation running (route), meeting (route). */
const SAMPLE: OrbitPoint[] = [
  { id: "orbit:memory", capability: "memory", lit: true, label: "Just learned", route: { space: "memory" } as Route },
  {
    id: "orbit:automation",
    capability: "automation",
    lit: true,
    label: "Automation running",
    route: { space: "automations" } as Route,
  },
  // A non-actionable awareness light (no route) — pure body language.
  { id: "orbit:local", capability: "local", lit: true, label: "Working locally" },
];

/** Wrap the Orbit around the Core like the real homepage. */
function stage(node: unknown) {
  coreStore.reset();
  return (
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
        <div style={{ position: "relative", display: "flex" }}>
          <CorePresence size="lg" />
          {node as never}
        </div>
      </div>
    </Room>
  );
}

/** Routing-only handler that just logs — never sends/executes (Req 6.4). */
const onNavigate = (r: Route) => console.info("[Orbit story] route only →", r);

/** Engaged with a partial set of lit points (Req 6.1/6.2). */
export const Engaged: Story = {
  render: () => stage(<ContextualOrbit orbit={() => SAMPLE} engaged={() => true} onNavigate={onNavigate} />),
};

/** A single lit point — the engine lit only one relevant capability. */
export const SinglePoint: Story = {
  render: () => stage(<ContextualOrbit orbit={() => [SAMPLE[0]]} engaged={() => true} onNavigate={onNavigate} />),
};

/** Disengaged / rest — the Orbit renders NOTHING (never a permanent ring, Req 6.1). */
export const Disengaged: Story = {
  render: () => stage(<ContextualOrbit orbit={() => SAMPLE} engaged={() => false} onNavigate={onNavigate} />),
};

/** Reduced motion — static labelled dots, same labels + routing (Req 6.6). */
export const StaticDots: Story = {
  render: () =>
    stage(<ContextualOrbit orbit={() => SAMPLE} engaged={() => true} reducedMotion onNavigate={onNavigate} />),
};

/**
 * Live — a button toggles engagement so the appear-on-engagement / fade-on-
 * disengage behavior is visible in the workbench (Req 6.1).
 */
export const Live: Story = {
  render: () => {
    const [engaged, setEngaged] = createSignal(true);
    return stage(
      <>
        <ContextualOrbit orbit={() => SAMPLE} engaged={engaged} onNavigate={onNavigate} />
        <button
          type="button"
          style={{ position: "absolute", bottom: "-64px", left: "50%", transform: "translateX(-50%)" }}
          onClick={() => setEngaged((e) => !e)}
        >
          Toggle engagement
        </button>
      </>,
    );
  },
};
