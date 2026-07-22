import type { Meta, StoryObj } from "storybook-solidjs-vite";
import { createSignal } from "solid-js";

import AdaptiveContextSurface from "./AdaptiveContextSurface";
import VoiceLine from "./VoiceLine";
import { Room } from "./Room";
import { CorePresence } from "../../../components/CorePresence";
import { coreStore } from "../../../stores";
import type { FocusAcs, FocusVoiceLine } from "../../../stores/homeFocusStore";
import type { Route } from "../../router";

/**
 * AdaptiveContextSurface — the BODY of the current Focus subject (design.md
 * §6.2, Requirement 8). These stories exercise the real component (task 4.2)
 * with an injected `acs` accessor so the workbench stays deterministic and
 * decoupled from the live domain stores.
 *
 * Shown inside the `Room` beneath a `CorePresence` + `VoiceLine` to match the
 * homepage composition (design §2/§6): one living-glass surface for the one
 * Focus subject. Every action here ROUTES/STAGES ONLY (Req 8.2) — the `run`
 * callback and `onNavigate` are stubbed no-op loggers; nothing sends or
 * executes. At rest the surface DISSOLVES (renders nothing), never an empty box.
 *
 * Requirements: 8.1, 8.2, 8.3, 8.4, 8.5 (and 16.4 — workbench parity).
 */
const meta = {
  title: "Spaces/Home/AdaptiveContextSurface",
  component: AdaptiveContextSurface,
  decorators: [
    (Story: () => unknown) => (
      <div class="kria-shell" data-window-mode="immersive" style={{ height: "600px" }}>
        {Story() as never}
      </div>
    ),
  ],
} satisfies Meta<typeof AdaptiveContextSurface>;

export default meta;
type Story = StoryObj<typeof meta>;

function acs(over: Partial<FocusAcs> = {}): FocusAcs {
  return {
    subjectId: over.subjectId ?? "s1",
    title: over.title ?? "1 approval waiting",
    line: over.line ?? "Send the weekly report to the team channel.",
    action: over.action,
    ownerRoute: over.ownerRoute ?? { space: "converse", segment: "approvals" },
  };
}

/** Wrap the surface in the Room + Core (+ Voice Line) like the real homepage. */
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

/** A subject with body only — deeper detail routes to the owning Space (Req 8.2). */
export const Resting: Story = {
  render: () =>
    stage(
      <AdaptiveContextSurface
        acs={() => acs({ title: "Standup in 20", line: "Your prep notes are ready to resume." })}
        onNavigate={(r) => console.info("[ACS story] route only →", r)}
      />,
    ),
};

/** A subject offering its single action verb (routing/staging only, Req 8.2). */
export const WithAction: Story = {
  render: () =>
    stage(
      <AdaptiveContextSurface
        acs={() =>
          acs({
            action: { label: "Review", run: () => console.info("[ACS story] stage/route only — never sends") },
          })
        }
        onNavigate={(r) => console.info("[ACS story] route only →", r)}
      />,
    ),
};

/**
 * Bound to the Voice Line — the ACS is the SAME subject as the headline (Req
 * 8.4). Both densities share `subjectId`; the ACS is the expansion, never a
 * competing context system.
 */
export const BoundToVoiceLine: Story = {
  render: () => {
    const subjectId = "approval:ap-1";
    const link: Route = { space: "converse", segment: "approvals", entityId: "ap-1" };
    const voice = (): FocusVoiceLine => ({
      subjectId,
      text: "1 approval waiting for you.",
      key: subjectId,
      actionable: true,
      link,
      priority: 100,
      confidence: 1,
      emphasis: "high",
    });
    return stage(
      <div style={{ display: "flex", "flex-direction": "column", "align-items": "center", gap: "16px" }}>
        <VoiceLine line={voice} onNavigate={(r) => console.info("[ACS story] route only →", r)} />
        <AdaptiveContextSurface
          acs={() =>
            acs({
              subjectId,
              title: "Send weekly report",
              line: "Post the weekly report to #team.",
              action: { label: "Review", run: () => console.info("[ACS story] stage/route only") },
              ownerRoute: link,
            })
          }
          onNavigate={(r) => console.info("[ACS story] route only →", r)}
        />
      </div>,
    );
  },
};

/** Empty / rest — the surface DISSOLVES (renders NOTHING; never an empty box). */
export const Empty: Story = {
  render: () => stage(<AdaptiveContextSurface acs={() => undefined} />),
};

/**
 * Live crossfade — a button swaps the subject so the crossfade + once-announce
 * behavior is visible in the workbench (Req 8.5). Clearing the subject makes the
 * surface dissolve.
 */
export const Crossfade: Story = {
  render: () => {
    const subjects: (FocusAcs | undefined)[] = [
      acs({ subjectId: "s1", title: "Download finished", line: "report.pdf is ready in Downloads." }),
      acs({ subjectId: "s2", title: "Standup in 20", line: "Want your prep notes?" }),
      undefined,
    ];
    const [i, setI] = createSignal(0);
    return stage(
      <div style={{ display: "flex", "flex-direction": "column", "align-items": "center", gap: "16px" }}>
        <AdaptiveContextSurface
          acs={() => subjects[i()]}
          onNavigate={(r) => console.info("[ACS story] route only →", r)}
        />
        <button type="button" onClick={() => setI((n) => (n + 1) % subjects.length)}>
          Next subject
        </button>
      </div>,
    );
  },
};
