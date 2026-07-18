import type { Meta, StoryObj } from "storybook-solidjs-vite";
import { QuarantineReview } from "./QuarantineReview";
import { ModalHost } from "../../ModalHost";
import type { QuarantineToolView } from "../../../stores";

/**
 * QuarantineReview (task 8.4, Req 20.3) — the revived QuarantineQueue folded
 * into the Governance segment. Compiled/discovered skills are reviewed and
 * approved/rejected via a deliberate confirm (Req 11.3). Risk + status are
 * icon + text, never color alone (Req 17.3). Stories log decisions instead of
 * dispatching to the runtime.
 */
function tool(over: Partial<QuarantineToolView> = {}): QuarantineToolView {
  return {
    id: "t1",
    name: "PDF summarizer",
    description: "Summarize a PDF into key points.",
    riskLevel: "yellow",
    status: "PendingApproval",
    source: "SkillCompiler",
    successCount: 3,
    consecutiveFailures: 1,
    totalExecutions: 4,
    createdAt: "2024-01-01",
    lastTested: "2024-01-02",
    reviewNotes: null,
    parametersSchema: null,
    ...over,
  };
}

const handlers = {
  onApprove: (id: string) => console.log("approve", id),
  onReject: (id: string) => console.log("reject", id),
  onReload: () => console.log("reload"),
};

const meta = {
  title: "Spaces/Capabilities/QuarantineReview",
  component: QuarantineReview,
} satisfies Meta<typeof QuarantineReview>;

export default meta;
type Story = StoryObj<typeof meta>;

/** Base args satisfy the typed story contract; each story overrides via render. */
const baseArgs = { loading: false, tools: [], ...handlers };

export const PendingApproval: Story = {
  args: baseArgs,
  render: () => (
    <>
      <QuarantineReview
        loading={false}
        tools={[
          tool(),
          tool({ id: "t2", name: "Shell runner", riskLevel: "red", description: "Run a shell command." }),
          tool({ id: "t3", name: "Clock", riskLevel: "green", status: "Testing", description: "Report the time." }),
        ]}
        {...handlers}
      />
      {/* Mounted so the deliberate approve/reject confirm is visible. */}
      <ModalHost />
    </>
  ),
};

export const Empty: Story = {
  args: baseArgs,
  render: () => <QuarantineReview loading={false} tools={[]} {...handlers} />,
};

export const Loading: Story = {
  args: baseArgs,
  render: () => <QuarantineReview loading={true} tools={[]} {...handlers} />,
};
