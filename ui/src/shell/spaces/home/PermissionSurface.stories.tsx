import type { Meta, StoryObj } from "storybook-solidjs-vite";

import PermissionSurface from "./PermissionSurface";
import { Room } from "./Room";
import { CorePresence } from "../../../components/CorePresence";
import { coreStore } from "../../../stores";
import { resolvePermissionMode, type OverlayState, type PermissionSubject } from "./permissionUx";

/**
 * PermissionSurface — the homepage Permission UX (design.md §10.4, Requirement
 * 10). These stories exercise the real component (task 8.5) with an injected
 * subject/overlay/handlers so the workbench stays deterministic and decoupled
 * from the live `approvalStore`.
 *
 * Each tier renders its presence style: GREEN → report + Undo (non-blocking);
 * YELLOW → intent + halt window; RED/BLACK → single-line Allow/Deny with
 * what/why visible + a "Review in Approval Center" route. Every control here is
 * a stubbed no-op logger — nothing sends, executes, or mutates a store. When a
 * blocking overlay is open the surface DEFERS (renders nothing) — no
 * modal-on-modal.
 *
 * Requirements: 10.1, 10.2, 10.3, 10.4 (and 16.4 — workbench parity).
 */
const meta = {
  title: "Spaces/Home/PermissionSurface",
  component: PermissionSurface,
  decorators: [
    (Story: () => unknown) => (
      <div class="kria-shell" data-window-mode="immersive" style={{ height: "600px" }}>
        {Story() as never}
      </div>
    ),
  ],
} satisfies Meta<typeof PermissionSurface>;

export default meta;
type Story = StoryObj<typeof meta>;

const CLOSED: OverlayState = { approvalCenterOpen: false, modalOpen: false };

function subject(over: Partial<PermissionSubject> = {}): PermissionSubject {
  const risk = over.risk ?? "red";
  return {
    requestId: over.requestId ?? "s1",
    risk,
    mode: over.mode ?? resolvePermissionMode(risk),
    what: over.what ?? "Delete build output",
    why: over.why ?? "you asked me to clean up",
    reversible: over.reversible ?? true,
    createdAt: over.createdAt ?? 1_000,
  };
}

const log = (label: string) => (id: string) => console.info(`[Permission story] ${label} (route/stage only) →`, id);

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

/** GREEN — KRIA already acted; reports it + offers Undo. Non-blocking (Req 10.1). */
export const GreenReport: Story = {
  render: () =>
    stage(
      <PermissionSurface
        subject={() => subject({ risk: "green", what: "Archived 3 old notes", reversible: true })}
        overlay={() => CLOSED}
        blockedContext={() => false}
        onUndo={log("undo")}
      />,
    ),
};

/** YELLOW — narrates intent + a brief halt window with a Stop control (Req 10.2). */
export const YellowIntent: Story = {
  render: () =>
    stage(
      <PermissionSurface
        subject={() => subject({ risk: "yellow", what: "Sending the weekly report", why: "draft is ready" })}
        overlay={() => CLOSED}
        blockedContext={() => false}
        onHalt={log("halt")}
        onProceed={log("proceed")}
      />,
    ),
};

/** RED — single-line Allow/Deny, what/why visible, routes detail to the Center (Req 10.3/10.4). */
export const RedDecision: Story = {
  render: () =>
    stage(
      <PermissionSurface
        subject={() => subject({ risk: "red", what: "Overwrite production config", why: "you approved the rollout" })}
        overlay={() => CLOSED}
        blockedContext={() => false}
        onAllow={log("allow")}
        onDeny={log("deny")}
        onReviewDetail={log("review")}
      />,
    ),
};

/** RED in an interruptibility-blocked context — surfaces calmly (Req 26.3). */
export const RedBlockedContext: Story = {
  render: () =>
    stage(
      <PermissionSurface
        subject={() => subject({ risk: "red", what: "Share your screen with the call" })}
        overlay={() => CLOSED}
        blockedContext={() => true}
        onAllow={log("allow")}
        onDeny={log("deny")}
      />,
    ),
};

/** Deferred — a blocking overlay is already open, so the surface renders nothing. */
export const DeferredNoStacking: Story = {
  render: () =>
    stage(
      <PermissionSurface
        subject={() => subject({ risk: "red" })}
        overlay={() => ({ approvalCenterOpen: true, modalOpen: false })}
        blockedContext={() => false}
      />,
    ),
};
