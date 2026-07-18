import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, screen, fireEvent } from "@solidjs/testing-library";
import { MessageBubble } from "./MessageBubble";
import { converseStore, eventBus, shellStore, type Message } from "../../../stores";
import { currentRoute, navigate } from "../../router";

function msg(overrides: Partial<Message> = {}): Message {
  return {
    id: "m1",
    threadId: "t1",
    role: "assistant",
    content: "Hello **world**",
    timestamp: Date.UTC(2024, 0, 1, 12, 0),
    ...overrides,
  };
}

// Open the right-click context menu on the bubble.
function openContextMenu() {
  const article = screen.getByRole("article");
  fireEvent.contextMenu(article);
}

// Select a menu item the same reliable way the kit Menu test does.
function selectMenuItem(name: string | RegExp) {
  const item = screen.getByRole("menuitem", { name });
  fireEvent.keyDown(item, { key: "Enter" });
  fireEvent.keyUp(item, { key: "Enter" });
}

beforeEach(() => {
  eventBus.clear();
  // Provide a clipboard stub (jsdom has none by default).
  Object.assign(navigator, { clipboard: { writeText: vi.fn().mockResolvedValue(undefined) } });
});

describe("MessageBubble — rendering & provenance (Req 20.5)", () => {
  it("renders sanitized markdown (bold) as HTML", () => {
    const { container } = render(() => <MessageBubble message={msg()} />);
    const body = container.querySelector(".kria-msg__body")!;
    expect(body.querySelector("strong")?.textContent).toBe("world");
  });

  it("shows an AI-provenance cue on assistant turns and 'You' on user turns", () => {
    const { unmount } = render(() => <MessageBubble message={msg({ role: "assistant" })} />);
    expect(screen.getByRole("article")).toHaveAttribute("data-provenance", "kria");
    expect(screen.getByText("KRIA")).toBeInTheDocument();
    unmount();

    render(() => <MessageBubble message={msg({ id: "m2", role: "user" })} />);
    expect(screen.getByRole("article")).toHaveAttribute("data-provenance", "user");
    expect(screen.getByText("You")).toBeInTheDocument();
  });

  it("applies role styling via data-role", () => {
    render(() => <MessageBubble message={msg({ role: "system" })} />);
    expect(screen.getByRole("article")).toHaveAttribute("data-role", "system");
  });
});

describe("MessageBubble — sanitization (security critical, §1.17)", () => {
  it("strips <script> and inline event handlers from AI content", () => {
    const malicious =
      "Safe text\n\n<script>window.__pwned=1</script>\n\n<img src=x onerror=\"window.__pwned=1\">";
    const { container } = render(() => <MessageBubble message={msg({ content: malicious })} />);
    const body = container.querySelector(".kria-msg__body")!;
    expect(body.querySelector("script")).toBeNull();
    expect(body.innerHTML).not.toContain("onerror");
    expect(body.innerHTML).not.toContain("window.__pwned");
  });

  it("sanitizes untrusted result-card HTML", () => {
    const message = msg({
      results: [
        { id: "r1", kind: "tool-result", title: "Shell output", html: "<b>ok</b><script>evil()</script>" },
      ],
    });
    const { container } = render(() => <MessageBubble message={message} />);
    const resultBody = container.querySelector(".kria-msg__result-body")!;
    expect(resultBody.querySelector("script")).toBeNull();
    expect(resultBody.querySelector("b")?.textContent).toBe("ok");
  });
});

describe("MessageBubble — inline result cards (Req 4.3)", () => {
  it("renders attached result cards", () => {
    const message = msg({
      results: [
        { id: "r1", kind: "memory", title: "Recalled fact", summary: "User prefers dark mode" },
      ],
    });
    render(() => <MessageBubble message={message} />);
    expect(screen.getByText("Recalled fact")).toBeInTheDocument();
    expect(screen.getByText("User prefers dark mode")).toBeInTheDocument();
  });
});

describe("MessageBubble — per-message actions (Req 4.8)", () => {
  it("shows the six actions on right-click", () => {
    render(() => <MessageBubble message={msg()} />);
    openContextMenu();
    const items = screen.getAllByRole("menuitem");
    expect(items.map((i) => i.textContent?.trim())).toEqual([
      "Copy",
      "Retry",
      "Explain",
      "Remember",
      "Branch",
      "Feedback",
    ]);
  });

  it("opens the actions menu via the keyboard", () => {
    render(() => <MessageBubble message={msg()} />);
    const trigger = screen.getByRole("button", { name: "Message actions" });
    trigger.focus();
    fireEvent.keyDown(trigger, { key: "Enter" });
    expect(screen.getByRole("menu")).toBeInTheDocument();
    expect(screen.getAllByRole("menuitem").length).toBe(6);
  });

  it("copy writes the message content to the clipboard (local UI action)", () => {
    render(() => <MessageBubble message={msg({ content: "copy me" })} />);
    openContextMenu();
    selectMenuItem("Copy");
    expect(navigator.clipboard.writeText).toHaveBeenCalledWith("copy me");
  });

  it("retry dispatches through converseStore (not a tool call)", () => {
    const spy = vi.spyOn(converseStore, "retryMessage");
    render(() => <MessageBubble message={msg({ id: "mX" })} />);
    openContextMenu();
    selectMenuItem("Retry");
    expect(spy).toHaveBeenCalledWith("mX");
    spy.mockRestore();
  });
});

describe("MessageBubble — 'Why did KRIA answer this?' deep-link (Req 5.7)", () => {
  beforeEach(() => {
    // Reset navigation + Inspector so each assertion starts clean.
    navigate("converse");
    shellStore.closeInspector();
  });

  it("hides the affordance on an assistant answer with no memory provenance", () => {
    render(() => <MessageBubble message={msg({ role: "assistant" })} />);
    openContextMenu();
    expect(screen.queryByRole("menuitem", { name: "Why did KRIA answer this?" })).toBeNull();
  });

  it("hides the affordance on user messages even if ids are present (assistant-only)", () => {
    render(() =>
      <MessageBubble message={msg({ id: "u9", role: "user", usedMemoryIds: ["mem-1"] })} />,
    );
    openContextMenu();
    expect(screen.queryByRole("menuitem", { name: "Why did KRIA answer this?" })).toBeNull();
  });

  it("hides the affordance when usedMemoryIds is empty (no fake link)", () => {
    render(() => <MessageBubble message={msg({ role: "assistant", usedMemoryIds: [] })} />);
    openContextMenu();
    expect(screen.queryByRole("menuitem", { name: "Why did KRIA answer this?" })).toBeNull();
  });

  it("shows the affordance on an assistant answer that carries memory provenance", () => {
    render(() =>
      <MessageBubble message={msg({ role: "assistant", usedMemoryIds: ["mem-42"] })} />,
    );
    openContextMenu();
    expect(
      screen.getByRole("menuitem", { name: "Why did KRIA answer this?" }),
    ).toBeInTheDocument();
  });

  it("clicking deep-links to the memory (nav) and opens the shared Inspector on it", () => {
    render(() =>
      <MessageBubble
        message={msg({ role: "assistant", usedMemoryIds: ["mem-42", "mem-99"] })}
      />,
    );
    openContextMenu();
    selectMenuItem("Why did KRIA answer this?");

    // Deep-link to Memory Space at the primary (first) memory id.
    expect(currentRoute().space).toBe("memory");
    expect(currentRoute().segment).toBe("explorer");
    expect(currentRoute().entityId).toBe("mem-42");

    // Shared Inspector is open on that memory (Req 5.7).
    const target = shellStore.inspectorTarget();
    expect(target?.type).toBe("memory");
    expect(target?.id).toBe("mem-42");
  });
});
