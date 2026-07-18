import { describe, it, expect, beforeEach } from "vitest";
import { render, screen } from "@solidjs/testing-library";
import { MessageStream } from "./MessageStream";
import { converseStore, type Message } from "../../../stores";

function seed(count: number): void {
  converseStore.clearMessages();
  for (let i = 0; i < count; i++) {
    const m: Message = {
      id: `m${i}`,
      threadId: "t1",
      role: i % 2 === 0 ? "user" : "assistant",
      content: `Message number ${i}`,
      timestamp: Date.now() + i,
    };
    converseStore.addMessage(m);
  }
}

// Give the viewport a real height so the virtualizer has a window to fill.
function renderInViewport() {
  return render(() => (
    <div style={{ height: "400px" }}>
      <MessageStream />
    </div>
  ));
}

beforeEach(() => {
  converseStore.clearMessages();
});

describe("MessageStream — virtualization (Req 16.2)", () => {
  it("renders messages from the store", () => {
    seed(3);
    renderInViewport();
    expect(screen.getByText("Message number 0")).toBeInTheDocument();
    expect(screen.getAllByRole("article").length).toBeGreaterThan(0);
  });

  it("mounts only a subset of a large list (does not render every row)", () => {
    const total = 500;
    seed(total);
    renderInViewport();
    // Virtualization: far fewer DOM bubbles than messages in the store.
    const rendered = screen.getAllByRole("article").length;
    expect(rendered).toBeGreaterThan(0);
    expect(rendered).toBeLessThan(total);
  });

  it("renders nothing when the thread is empty", () => {
    renderInViewport();
    expect(screen.queryAllByRole("article").length).toBe(0);
  });
});
