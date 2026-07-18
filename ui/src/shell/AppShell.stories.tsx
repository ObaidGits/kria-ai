import type { Meta, StoryObj } from "storybook-solidjs-vite";
import { AppShell } from "./AppShell";
import { Dock } from "./Dock";
import { InspectorHost } from "./InspectorHost";
import { PresenceBar } from "./PresenceBar";
import { StatusLine } from "./StatusLine";
import { shellStore, coreStore, approvalStore } from "../stores";

const meta = {
  title: "Shell/AppShell",
  component: AppShell,
} satisfies Meta<typeof AppShell>;

export default meta;
type Story = StoryObj<typeof meta>;

/** The full shell: PresenceBar · Dock · SpaceRouter · InspectorHost · StatusLine. */
export const FullShell: Story = {
  render: () => <AppShell />,
};

/** The Dock alone — the 7-Space navigation rail. */
export const DockRail: Story = {
  render: () => (
    <div style={{ width: "220px", height: "480px", display: "flex" }}>
      <Dock />
    </div>
  ),
};

/** PresenceBar with a pending high-risk approval badge + active Core. */
export const PresenceWithApprovals: Story = {
  render: () => {
    coreStore.setState("thinking");
    approvalStore.setQueue([
      {
        id: "a1",
        type: "tool-hitl",
        title: "Run shell command",
        description: "",
        risk: "red",
        payload: null,
        createdAt: Date.now(),
        status: "pending",
      },
    ]);
    return <PresenceBar />;
  },
};

/** The single shared Inspector, populated with a demo target. */
export const InspectorPanel: Story = {
  render: () => {
    shellStore.setInspectorTarget({ type: "memory", id: "fact-42" });
    return (
      <div style={{ height: "480px", display: "flex", "justify-content": "flex-end" }}>
        <InspectorHost
          renderers={{
            memory: (t) => ({
              title: `Memory ${t.id}`,
              body: <p>Confidence 0.82 · verified · source: conversation.</p>,
            }),
          }}
        />
      </div>
    );
  },
};

/** The single status line reflecting Core state. */
export const StatusLineOnly: Story = {
  render: () => {
    coreStore.setState("acting");
    return <StatusLine />;
  },
};
