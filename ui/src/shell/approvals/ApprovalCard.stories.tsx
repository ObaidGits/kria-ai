import type { Meta, StoryObj } from "storybook-solidjs-vite";
import { ApprovalCard } from "./ApprovalCard";
import { ModalHost } from "../ModalHost";
import type { ApprovalRequest, ApprovalScope } from "../../stores/approvalStore";

/**
 * ApprovalCard — the consequential-decision card (Req 11.2/11.3). Stories show
 * the risk ramp across low/medium/high/critical and the deliberate-approve +
 * high-risk-confirm behaviour. ModalHost is mounted so the high-risk confirm
 * (routed through the one-at-a-time modal host) is visible in the stories.
 */
function base(overrides: Partial<ApprovalRequest> = {}): ApprovalRequest {
  return {
    id: "story-1",
    type: "tool-hitl",
    title: "Delete 42 cached files",
    description: "Frees ~1.2 GB so the next build has room to run.",
    risk: "green",
    effects: ["Removes ~1.2 GB from the build cache", "The next build is slower once"],
    evidence: "Target: /tmp/kria-build-cache (42 files, 1.2 GB)",
    payload: {},
    createdAt: Date.now(),
    status: "pending",
    ...overrides,
  };
}

const handlers = {
  onApprove: (scope?: ApprovalScope) => console.log("approve", scope),
  onDeny: () => console.log("deny"),
  onKeepPaused: () => console.log("keep-paused"),
};

const meta = {
  title: "Shell/ApprovalCard",
  component: ApprovalCard,
  // Meta-level args satisfy the component's required props; each story overrides
  // `render` to show a specific risk level.
  args: { request: base(), ...handlers },
} satisfies Meta<typeof ApprovalCard>;

export default meta;
type Story = StoryObj<typeof meta>;

/** Low risk (green) — approve is a single deliberate press, no extra confirm. */
export const LowRisk: Story = {
  render: () => (
    <div style={{ "max-width": "440px" }}>
      <ApprovalCard request={base({ risk: "green", scopeOptions: ["once", "session"] })} {...handlers} />
      <ModalHost />
    </div>
  ),
};

/** Medium risk (yellow). */
export const MediumRisk: Story = {
  render: () => (
    <div style={{ "max-width": "440px" }}>
      <ApprovalCard
        request={base({
          risk: "yellow",
          title: "Send the drafted email to Sam",
          description: "You asked KRIA to reply once the report was ready.",
          effects: ["Sends 1 email to sam@example.com"],
          evidence: "Subject: Q3 report — ready for review",
        })}
        {...handlers}
      />
      <ModalHost />
    </div>
  ),
};

/** High risk (red) — Approve opens an explicit confirm via the modal host. */
export const HighRisk: Story = {
  render: () => (
    <div style={{ "max-width": "440px" }}>
      <ApprovalCard
        request={base({
          risk: "red",
          title: "Push 3 commits to main",
          description: "The release workflow wants to publish the built artifacts.",
          effects: ["Pushes to origin/main", "Triggers the deploy pipeline"],
          evidence: "3 commits ahead of origin/main",
        })}
        {...handlers}
      />
      <ModalHost />
    </div>
  ),
};

/** Critical / irreversible (black) — danger confirm, cannot be undone. */
export const Critical: Story = {
  render: () => (
    <div style={{ "max-width": "440px" }}>
      <ApprovalCard
        request={base({
          risk: "black",
          irreversible: true,
          title: "Drop the production database",
          description: "A destructive migration requested this. This cannot be undone.",
          effects: ["Deletes all rows in 12 tables", "No automatic backup exists"],
          evidence: "Host: db-prod-1 · schema: kria",
        })}
        {...handlers}
      />
      <ModalHost />
    </div>
  ),
};
