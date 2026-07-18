import type { Meta, StoryObj } from "storybook-solidjs-vite";
import { TrustReviewDialog } from "./TrustReviewDialog";
import type { RemoteSkillView } from "../../../stores";

/**
 * TrustReviewDialog (task 8.2, Req 7.4) — the trust-review step before a remote
 * skill install. Trust tier is icon + text (Req 17.3); install requires a
 * deliberate press. Stories log the install instead of dispatching.
 */
function skill(over: Partial<RemoteSkillView> = {}): RemoteSkillView {
  return {
    slug: "web-search",
    name: "Web Search",
    description: "Search the web and summarize results.",
    category: "web",
    trustTier: "community",
    version: "1.0.0",
    manifestUrl: "https://hub.example/web-search",
    capabilities: ["network.read", "fs.read"],
    installed: false,
    ...over,
  };
}

const meta = {
  title: "Spaces/Capabilities/TrustReviewDialog",
  component: TrustReviewDialog,
} satisfies Meta<typeof TrustReviewDialog>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Community: Story = {
  args: {
    skill: skill(),
    open: true,
    onOpenChange: () => {},
    onInstall: () => {},
  },
  render: () => (
    <TrustReviewDialog
      skill={skill()}
      open={true}
      onOpenChange={(o) => console.log("open", o)}
      onInstall={(caps) => console.log("install", caps)}
    />
  ),
};

export const NoCapabilities: Story = {
  args: {
    skill: skill({ name: "Clock", capabilities: [], description: "Report the time." }),
    open: true,
    onOpenChange: () => {},
    onInstall: () => {},
  },
  render: () => (
    <TrustReviewDialog
      skill={skill({ name: "Clock", capabilities: [], description: "Report the time." })}
      open={true}
      onOpenChange={(o) => console.log("open", o)}
      onInstall={(caps) => console.log("install", caps)}
    />
  ),
};
