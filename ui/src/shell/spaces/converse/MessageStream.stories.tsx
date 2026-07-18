import type { Meta, StoryObj } from "storybook-solidjs-vite";
import MessageStream from "./MessageStream";
import { converseStore, type Message } from "../../../stores";

/**
 * Virtualized MessageStream (task 3.2). Stories seed converseStore before
 * rendering and wrap the stream in a fixed-height, floating surface so the
 * virtualizer + sticky jump control read correctly in the workbench.
 */
const meta = {
  title: "Converse/MessageStream",
  component: MessageStream,
  decorators: [
    (Story: () => unknown) => (
      <div class="kria-shell" data-window-mode="standard" style={{ height: "560px", display: "flex", "flex-direction": "column" }}>
        {Story() as never}
      </div>
    ),
  ],
} satisfies Meta<typeof MessageStream>;

export default meta;
type Story = StoryObj<typeof meta>;

function seed(messages: Message[]): void {
  converseStore.clearMessages();
  for (const m of messages) converseStore.addMessage(m);
}

const now = Date.now();

/** A short conversation with markdown, code, and an inline result card. */
export const Conversation: Story = {
  render: () => {
    seed([
      { id: "m1", threadId: "t1", role: "user", content: "Show me a Rust hello world.", timestamp: now },
      {
        id: "m2",
        threadId: "t1",
        role: "assistant",
        content:
          "Here you go:\n\n```rust\nfn main() {\n    println!(\"Hello, world!\");\n}\n```\n\nRun it with `cargo run`.",
        timestamp: now + 1,
        results: [
          { id: "r1", kind: "tool-result", title: "cargo run", summary: "Compiled and ran in 0.4s → Hello, world!" },
        ],
      },
    ]);
    return <MessageStream />;
  },
};

/** A long thread demonstrating virtualization (only the visible window mounts). */
export const LongThread: Story = {
  render: () => {
    seed(
      Array.from({ length: 400 }, (_, i) => ({
        id: `m${i}`,
        threadId: "t1",
        role: i % 2 === 0 ? "user" : "assistant",
        content: `Message **${i}** in a long, virtualized thread. Scroll to see rows mount on demand.`,
        timestamp: now + i,
      })) as Message[],
    );
    return <MessageStream />;
  },
};
