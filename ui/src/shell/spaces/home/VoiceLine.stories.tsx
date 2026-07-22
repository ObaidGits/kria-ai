import type { Meta, StoryObj } from "storybook-solidjs-vite";
import { createSignal } from "solid-js";

import VoiceLine from "./VoiceLine";
import { Room } from "./Room";
import { CorePresence } from "../../../components/CorePresence";
import { coreStore } from "../../../stores";
import type { FocusVoiceLine } from "../../../stores/homeFocusStore";
import type { Route } from "../../router";

/**
 * VoiceLine — the Focus headline beneath the Core (design.md §6.1, Requirement
 * 3). These stories exercise the real `VoiceLine` component (task 4.1) with an
 * injected `line` accessor so the workbench stays deterministic and decoupled
 * from the live domain stores.
 *
 * Shown inside the `Room` beneath a `CorePresence` to match the homepage
 * composition (design §2): one calm line under the Core. The deep-link story's
 * `onNavigate` is stubbed to a no-op logger — activation ROUTES ONLY (Req 3.6);
 * it never sends or executes.
 *
 * Requirements: 3.1, 3.2, 3.3, 3.4, 3.5, 3.6 (and 16.4 — workbench parity).
 */
const meta = {
  title: "Spaces/Home/VoiceLine",
  component: VoiceLine,
  decorators: [
    (Story: () => unknown) => (
      <div class="kria-shell" data-window-mode="immersive" style={{ height: "600px" }}>
        {Story() as never}
      </div>
    ),
  ],
} satisfies Meta<typeof VoiceLine>;

export default meta;
type Story = StoryObj<typeof meta>;

function subject(over: Partial<FocusVoiceLine> = {}): FocusVoiceLine {
  return {
    subjectId: over.subjectId ?? "s1",
    text: over.text ?? "Evening, Obaid. Standup in 20 — want the notes?",
    key: over.key ?? "k1",
    actionable: over.actionable ?? false,
    link: over.link,
    priority: over.priority ?? 80,
    confidence: over.confidence ?? 0.8,
    emphasis: over.emphasis ?? "high",
  };
}

/** Wrap the line in the Room + Core so it reads like the real homepage. */
function stage(node: unknown) {
  coreStore.reset();
  return (
    <Room>
      <div style={{ flex: "1 1 auto", display: "flex", "flex-direction": "column", "align-items": "center", "justify-content": "center", gap: "24px" }}>
        <CorePresence size="lg" />
        {node as never}
      </div>
    </Room>
  );
}

/** A plain, non-actionable resting line beneath the Core (Req 3.1). */
export const Resting: Story = {
  render: () => stage(<VoiceLine line={() => subject({ text: "Quietly here whenever you need me." })} />),
};

/** An actionable subject: a routing-only deep link to the owning surface (Req 3.6). */
export const DeepLink: Story = {
  render: () => {
    const link: Route = { space: "converse", segment: "thread", entityId: "t-42" };
    return stage(
      <VoiceLine
        line={() => subject({ text: "Resume the notes for standup", actionable: true, link })}
        // Routing only — no send/tool/approval side effect (Req 3.6).
        onNavigate={(r) => console.info("[VoiceLine story] route only →", r)}
      />,
    );
  },
};

/** Empty / rest — the component renders NOTHING (never an empty box, §14). */
export const Empty: Story = {
  render: () => stage(<VoiceLine line={() => undefined} />),
};

/**
 * Live crossfade — a button swaps the subject so the crossfade + announce-once
 * behavior is visible in the workbench (Req 3.3/3.4). Same text is a silent
 * no-op; a new subject cross-dissolves.
 */
export const Crossfade: Story = {
  render: () => {
    const lines = [
      subject({ text: "Download finished.", key: "k1", subjectId: "s1" }),
      subject({ text: "Meeting in 20 — prep notes?", key: "k2", subjectId: "s2" }),
      subject({ text: "1 approval waiting for you.", key: "k3", subjectId: "s3", priority: 100 }),
    ];
    const [i, setI] = createSignal(0);
    return stage(
      <div style={{ display: "flex", "flex-direction": "column", "align-items": "center", gap: "16px" }}>
        <VoiceLine line={() => lines[i()]} />
        <button type="button" onClick={() => setI((n) => (n + 1) % lines.length)}>
          Next subject
        </button>
      </div>,
    );
  },
};
