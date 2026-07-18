import type { Meta, StoryObj } from "storybook-solidjs-vite";
import Composer from "./Composer";
import { converseStore, coreStore } from "../../../stores";

/**
 * Composer (task 3.4) — grow-then-scroll input, attachments, Assistant/Lab mode
 * chip, voice entry, and the single Send⇄Stop action. Stories seed the draft /
 * Core state before rendering; Send/Stop/voice are stubbed so the workbench
 * never touches the pipeline.
 */
const meta = {
  title: "Converse/Composer",
  component: Composer,
  decorators: [
    (Story: () => unknown) => (
      <div
        class="kria-shell"
        data-window-mode="standard"
        style={{ width: "720px", padding: "var(--space-4)", background: "var(--color-surface-3)" }}
      >
        {Story() as never}
      </div>
    ),
  ],
} satisfies Meta<typeof Composer>;

export default meta;
type Story = StoryObj<typeof meta>;

const noop = () => {};

/** Idle, empty — Send is present but disabled until there's text. */
export const Empty: Story = {
  render: () => {
    coreStore.reset();
    converseStore.setActiveThread("story-thread");
    converseStore.updateDraft({ text: "", attachments: [], mode: "assistant" });
    return <Composer onSend={noop} onStop={noop} onVoiceStart={noop} />;
  },
};

/** With a multi-line draft + staged attachments. */
export const WithDraftAndAttachments: Story = {
  render: () => {
    coreStore.reset();
    converseStore.setActiveThread("story-thread");
    converseStore.updateDraft({
      text: "Summarize these two files\nand compare their conclusions.",
      attachments: [
        { id: "q3", name: "q3-report.pdf", mime: "application/pdf", size: 0, bytes: new Uint8Array() },
        { id: "notes", name: "notes.md", mime: "text/markdown", size: 0, bytes: new Uint8Array() },
      ],
      mode: "assistant",
    });
    return <Composer onSend={noop} onStop={noop} onVoiceStart={noop} />;
  },
};

/** Lab mode (tool-locked) — a mode of the thread, not a hidden environment. */
export const LabMode: Story = {
  render: () => {
    coreStore.reset();
    converseStore.setActiveThread("story-thread");
    converseStore.updateDraft({ text: "grep the repo for TODOs", attachments: [], mode: "lab" });
    return <Composer onSend={noop} onStop={noop} onVoiceStart={noop} />;
  },
};

/** Working — the single Send becomes a prominent Stop while KRIA works. */
export const Working: Story = {
  render: () => {
    converseStore.setActiveThread("story-thread");
    converseStore.updateDraft({ text: "", attachments: [], mode: "assistant" });
    coreStore.setState("thinking");
    return <Composer onSend={noop} onStop={noop} onVoiceStart={noop} />;
  },
};
