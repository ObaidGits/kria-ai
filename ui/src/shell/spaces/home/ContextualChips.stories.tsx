import type { Meta, StoryObj } from "storybook-solidjs-vite";
import { createSignal } from "solid-js";

import ContextualChips from "./ContextualChips";
import { Room } from "./Room";
import { CorePresence } from "../../../components/CorePresence";
import { coreStore } from "../../../stores";
import type { Chip } from "../../../stores/homeFocusStore";
import type { Route } from "../../router";

/**
 * ContextualChips — ≤3 live next-action affordances beneath the Composer
 * (design.md §6.3, Requirement 5). These stories exercise the real component
 * (task 4.3) with an injected `chips` accessor so the workbench stays
 * deterministic and decoupled from the live domain stores.
 *
 * Shown inside the `Room` beneath a `CorePresence` to match the homepage
 * composition (design §2): a calm row of chips beneath the Composer. Every chip
 * either STAGES a reviewable draft or ROUTES (Req 5.3) — here `onStage` and
 * `onNavigate` are stubbed no-op loggers, so nothing sends or executes. At rest
 * (no real action) the row renders NOTHING — never generic filler (Req 5.2).
 *
 * Requirements: 5.1, 5.2, 5.3, 5.4 (and 16.4 — workbench parity).
 */
const meta = {
  title: "Spaces/Home/ContextualChips",
  component: ContextualChips,
  decorators: [
    (Story: () => unknown) => (
      <div class="kria-shell" data-window-mode="immersive" style={{ height: "600px" }}>
        {Story() as never}
      </div>
    ),
  ],
} satisfies Meta<typeof ContextualChips>;

export default meta;
type Story = StoryObj<typeof meta>;

const approvalsRoute: Route = { space: "converse", segment: "approvals", entityId: "ap-1" };

/** A realistic mixed set: one approval (route), one resume (stage), one memory (route). */
const SAMPLE: Chip[] = [
  { id: "approval:ap-1", label: "1 approval", icon: "shield-alert", kind: "route", payload: approvalsRoute },
  {
    id: "resume:t-42",
    label: "Resume draft",
    icon: "pencil",
    kind: "stage",
    payload: "Draft: finish the weekly report for #team.",
  },
  { id: "memory", label: "Timeline", icon: "brain", kind: "route", payload: { space: "memory" } as Route },
];

/** Wrap the chips in the Room + Core like the real homepage. */
function stage(node: unknown) {
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
        {node as never}
      </div>
    </Room>
  );
}

/** Route + stage handlers that only log — never send/execute (Req 5.3). */
const handlers = {
  onStage: (text: string) => console.info("[Chips story] STAGE draft only (never sends) →", text),
  onNavigate: (r: Route) => console.info("[Chips story] route only →", r),
};

/** Three ranked chips — the maximum (Req 5.1). */
export const ThreeChips: Story = {
  render: () => stage(<ContextualChips chips={() => SAMPLE} {...handlers} />),
};

/** A single chip — the engine surfaced only one real action. */
export const OneChip: Story = {
  render: () => stage(<ContextualChips chips={() => [SAMPLE[0]]} {...handlers} />),
};

/** Empty / rest — no real action, so the row renders NOTHING (never filler, Req 5.2). */
export const Empty: Story = {
  render: () => stage(<ContextualChips chips={() => []} {...handlers} />),
};

/**
 * Live — a button swaps the chip set (including empty) so the omit-when-empty
 * behavior is visible in the workbench (Req 5.2).
 */
export const Live: Story = {
  render: () => {
    const sets: Chip[][] = [SAMPLE, [SAMPLE[1]], []];
    const [i, setI] = createSignal(0);
    return stage(
      <div style={{ display: "flex", "flex-direction": "column", "align-items": "center", gap: "16px" }}>
        <ContextualChips chips={() => sets[i()]} {...handlers} />
        <button type="button" onClick={() => setI((n) => (n + 1) % sets.length)}>
          Next chip set
        </button>
      </div>,
    );
  },
};
