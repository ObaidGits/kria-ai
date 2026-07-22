import { describe, it, expect, beforeEach, vi } from "vitest";
import { createSignal } from "solid-js";
import { render, screen, fireEvent, waitFor } from "@solidjs/testing-library";
import { MessageBubble } from "./MessageBubble";
import { converseStore, eventBus, shellStore, type Message } from "../../../stores";
import { currentRoute, navigate } from "../../router";
import { copyAnnouncement, resetCopyAnnouncerForTest } from "./copyAnnouncer";

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
  resetCopyAnnouncerForTest();
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

  it("exposes one persistent, labelled actions trigger at rest — no hover/focus/selection needed (Req 12.2, UIE-M-007)", () => {
    const { container } = render(() => <MessageBubble message={msg()} />);
    // Discoverable without any interaction: the trigger is present in the DOM
    // at rest (not conditionally mounted) and carries an accessible label.
    const triggers = screen.getAllByRole("button", { name: "Message actions" });
    expect(triggers).toHaveLength(1);
    // It lives in the low-emphasis actions container (visible at rest, promoted
    // on focus/selection/hover via CSS — never removed from the DOM).
    const actions = container.querySelector(".kria-msg__actions")!;
    expect(actions).toBeInTheDocument();
    expect(actions).toContainElement(triggers[0]);
  });

  it("adds exactly one action tab stop per message (menu holds all six actions)", () => {
    const { container } = render(() => <MessageBubble message={msg()} />);
    // No per-action tab stop: only the single '⋯' trigger is a button/tab stop
    // at rest; the six actions live inside the menu it opens.
    const buttons = container.querySelectorAll(".kria-msg__actions button");
    expect(buttons).toHaveLength(1);
    expect(buttons[0]).toHaveAttribute("aria-label", "Message actions");
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

describe("MessageBubble — selected-state semantics (Req 12.1, UIE-M-008)", () => {
  // WHY aria-current and NOT aria-selected:
  // The message rows are standalone <article> elements rendered inside a
  // role="log" region (ConverseSpace .kria-converse__stream). role="log" is a
  // live-region/document pattern, NOT a composite widget (listbox / grid /
  // tree). `aria-selected` is only valid on children of composite widgets
  // (option/row/gridcell/tab/treeitem); using it on an <article> in a log is a
  // misuse that AT may ignore or mis-announce. `aria-current="true"` is the
  // valid pattern for "the current item within a set of related elements" and
  // is appropriate for the single-select interaction here — it exposes the
  // selected state programmatically while preserving the article/log reading
  // semantics. This mirrors the visible box-shadow selection ring so the
  // programmatic and visual states stay in lockstep (Req 12.1).

  it("exposes NO programmatic current-state when unselected, keeping article/log semantics", () => {
    render(() => <MessageBubble message={msg()} selected={false} />);
    const article = screen.getByRole("article");
    // Unselected: no aria-current, no visual selection data-attribute.
    expect(article).not.toHaveAttribute("aria-current");
    expect(article).not.toHaveAttribute("data-selected");
    // The row is still a document <article> (log reading semantics), never a
    // composite-widget option — so aria-selected must never appear.
    expect(article).not.toHaveAttribute("aria-selected");
  });

  it("exposes selected state via aria-current='true' AND the visible ring when selected", () => {
    render(() => <MessageBubble message={msg()} selected={true} />);
    const article = screen.getByRole("article");
    // Programmatic selected-state (valid for article-in-log): aria-current.
    expect(article).toHaveAttribute("aria-current", "true");
    // Visible selection state retained (drives the box-shadow ring in CSS).
    expect(article).toHaveAttribute("data-selected", "true");
    // Still an <article> in the log — never the invalid composite semantic.
    expect(article).not.toHaveAttribute("aria-selected");
    // Reading semantics preserved: it remains an article with its label.
    expect(article).toHaveAttribute("aria-label", "assistant message");
  });

  it("toggles aria-current in lockstep with the visible selected state", () => {
    const [sel, setSel] = createSignal(false);
    render(() => <MessageBubble message={msg()} selected={sel()} />);
    const article = screen.getByRole("article");
    expect(article).not.toHaveAttribute("aria-current");
    expect(article).not.toHaveAttribute("data-selected");

    setSel(true);
    expect(article).toHaveAttribute("aria-current", "true");
    expect(article).toHaveAttribute("data-selected", "true");

    setSel(false);
    expect(article).not.toHaveAttribute("aria-current");
    expect(article).not.toHaveAttribute("data-selected");
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

describe("MessageBubble — pointer/touch discoverability & operability (Req 12.2, UIE-M-007)", () => {
  // Discoverable WITHOUT hover: the trigger is mounted, enabled, and labelled
  // at rest — a touch/novice pointer user sees and can reach it with no
  // :hover or :focus-within precondition. (Actual pointer-tap open is a
  // Kobalte/compositor path exercised by the keyboard + right-click activation
  // tests below and the manual pointer/SR review in task 11.8.)
  it("keeps the trigger discoverable and operable at rest — no hover/focus precondition", () => {
    render(() => <MessageBubble message={msg()} />);
    const trigger = screen.getByRole("button", { name: "Message actions" });
    // Present + accessible name (discoverable) …
    expect(trigger).toBeInTheDocument();
    // … enabled and not hidden from AT/pointer (operable) …
    expect(trigger).toBeEnabled();
    expect(trigger).not.toHaveAttribute("aria-hidden", "true");
    expect(trigger).not.toHaveAttribute("disabled");
    // … and in the natural tab order (a real activatable control, not tabindex=-1).
    expect(trigger.tabIndex).toBeGreaterThanOrEqual(0);
  });

  it("is operable by pointer activation without hover — right-click opens the SAME action set", () => {
    render(() => <MessageBubble message={msg()} />);
    // A pointer gesture (context-menu) with no prior hover opens the full menu,
    // proving the actions are pointer-operable, not hover-gated.
    openContextMenu();
    expect(screen.getByRole("menu")).toBeInTheDocument();
    expect(screen.getAllByRole("menuitem").length).toBe(6);
  });
});

describe("MessageBubble — keyboard menu navigation (Req 22.x/23.x, UIE-M-007)", () => {
  it("opens from the keyboard, expands the trigger, and exposes navigable menuitems", async () => {
    render(() => <MessageBubble message={msg()} />);
    const trigger = screen.getByRole("button", { name: "Message actions" });
    trigger.focus();
    fireEvent.keyDown(trigger, { key: "Enter" });
    const menu = await screen.findByRole("menu");
    // Trigger reflects the open state and the six actions are navigable items.
    expect(trigger).toHaveAttribute("aria-expanded", "true");
    expect(screen.getAllByRole("menuitem").length).toBe(6);
    // Arrow navigation is handled by the open menu and does not dismiss it.
    fireEvent.keyDown(menu, { key: "ArrowDown" });
    expect(screen.getByRole("menu")).toBeInTheDocument();
  });

  it("dismisses on Escape and collapses the trigger, running no action", async () => {
    render(() => <MessageBubble message={msg()} />);
    const trigger = screen.getByRole("button", { name: "Message actions" });
    trigger.focus();
    fireEvent.keyDown(trigger, { key: "Enter" });
    const menu = await screen.findByRole("menu");
    fireEvent.keyDown(menu, { key: "Escape" });
    // Kobalte marks the surface dismissed and collapses the trigger; focus
    // return to the trigger is a compositor behaviour verified by the manual
    // keyboard/SR review (task 11.8). No action runs on dismiss.
    await waitFor(() => expect(menu).toHaveAttribute("data-closed"));
    expect(trigger).toHaveAttribute("aria-expanded", "false");
  });
});

describe("MessageBubble — copy outcome announced without moving focus (Req 12.3, UIE-M-009)", () => {
  // A macrotask tick drains the fire-and-forget clipboard promise chain plus
  // the announcer's queueMicrotask re-key.
  const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

  it("announces success after the per-message Copy action resolves", async () => {
    Object.assign(navigator, { clipboard: { writeText: vi.fn().mockResolvedValue(undefined) } });
    render(() => <MessageBubble message={msg({ content: "copy me" })} />);
    openContextMenu();
    selectMenuItem("Copy");
    await tick();
    expect(copyAnnouncement()).toBe("Copied to clipboard");
  });

  it("announces failure when the clipboard write is rejected", async () => {
    Object.assign(navigator, {
      clipboard: { writeText: vi.fn().mockRejectedValue(new Error("denied")) },
    });
    render(() => <MessageBubble message={msg({ content: "copy me" })} />);
    openContextMenu();
    selectMenuItem("Copy");
    await tick();
    expect(copyAnnouncement()).toBe("Copy failed");
  });

  it("announces failure from the code-block copy button when the clipboard is missing", async () => {
    Object.assign(navigator, { clipboard: undefined });
    const { container } = render(() =>
      <MessageBubble message={msg({ content: "```js\nconst a = 1;\n```" })} />,
    );
    const copyBtn = container.querySelector<HTMLButtonElement>(".kria-md-code__copy");
    expect(copyBtn).not.toBeNull();
    fireEvent.click(copyBtn!);
    await tick();
    expect(copyAnnouncement()).toBe("Copy failed");
  });
});
