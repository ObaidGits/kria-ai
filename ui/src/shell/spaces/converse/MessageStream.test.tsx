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

describe("MessageStream — keyboard traversal (task 9.6, Req 10.11/12.11/19.x)", () => {
  function viewport(container: HTMLElement): HTMLElement {
    const el = container.querySelector<HTMLElement>(".kria-stream__viewport");
    if (!el) throw new Error("viewport not found");
    return el;
  }

  function keydown(el: HTMLElement, key: string): KeyboardEvent {
    const ev = new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true });
    el.dispatchEvent(ev);
    return ev;
  }

  it("makes the viewport keyboard-focusable (tabindex=0) without trapping", () => {
    seed(20);
    const { container } = renderInViewport();
    expect(viewport(container).getAttribute("tabindex")).toBe("0");
  });

  it("acts on Home/End/PageUp/PageDown (preventDefault) for scroll context", () => {
    seed(50);
    const { container } = renderInViewport();
    const el = viewport(container);
    for (const key of ["Home", "End", "PageUp", "PageDown"]) {
      expect(keydown(el, key).defaultPrevented).toBe(true);
    }
  });

  it("does NOT intercept Tab (no focus trap)", () => {
    seed(50);
    const { container } = renderInViewport();
    expect(keydown(viewport(container), "Tab").defaultPrevented).toBe(false);
  });

  it("does NOT hijack keys aimed at a child editable control", () => {
    seed(50);
    const { container } = renderInViewport();
    const el = viewport(container);
    const input = document.createElement("input");
    el.appendChild(input);
    const ev = new KeyboardEvent("keydown", { key: "Home", bubbles: true, cancelable: true });
    input.dispatchEvent(ev);
    expect(ev.defaultPrevented).toBe(false);
  });

  it("uses native overflow scrolling — no wheel handler that could trap", () => {
    seed(50);
    const { container } = renderInViewport();
    // Wheel is not intercepted: a cancelable wheel event is never preventDefault'd.
    const ev = new WheelEvent("wheel", { deltaY: 120, bubbles: true, cancelable: true });
    viewport(container).dispatchEvent(ev);
    expect(ev.defaultPrevented).toBe(false);
  });
});

describe("MessageStream — selection & focus preservation (task 11.6, Req 16.4/12.11)", () => {
  function articleFor(text: string): HTMLElement {
    const node = screen.getByText(text).closest("article");
    if (!node) throw new Error(`article for "${text}" not found`);
    return node as HTMLElement;
  }

  it("exposes programmatic selected state (aria-current) on exactly the selected message", () => {
    seed(3);
    renderInViewport();
    const target = articleFor("Message number 1");
    target.click();
    expect(target.getAttribute("aria-current")).toBe("true");
    // Only the selected message carries the state.
    expect(articleFor("Message number 0").getAttribute("aria-current")).toBeNull();
    expect(articleFor("Message number 2").getAttribute("aria-current")).toBeNull();
  });

  it("keeps selection bound to the message id when the list changes (row reuse)", () => {
    seed(3);
    renderInViewport();
    articleFor("Message number 1").click();
    // Growing the thread reuses row slots; aria-current must follow the message,
    // never a recycled DOM slot.
    converseStore.addMessage({
      id: "m3",
      threadId: "t1",
      role: "user",
      content: "Message number 3",
      timestamp: Date.now() + 3,
    });
    expect(articleFor("Message number 1").getAttribute("aria-current")).toBe("true");
    expect(articleFor("Message number 3").getAttribute("aria-current")).toBeNull();
  });

  it("wires onFocusOut without hijacking an intentional focus move (relatedTarget present)", () => {
    seed(3);
    const { container } = renderInViewport();
    const vp = container.querySelector<HTMLElement>(".kria-stream__viewport");
    if (!vp) throw new Error("viewport not found");
    const article = articleFor("Message number 1");
    article.focus();
    const other = document.createElement("button");
    document.body.appendChild(other);
    // Focus intentionally moves elsewhere → the handler must not throw and must
    // not steal focus back (queue-restore is gated on relatedTarget == null).
    const ev = new FocusEvent("focusout", {
      bubbles: true,
      cancelable: true,
      relatedTarget: other,
    });
    expect(() => vp.dispatchEvent(ev)).not.toThrow();
    expect(ev.defaultPrevented).toBe(false);
    other.remove();
  });
});
