/**
 * Conversation scroll-ownership + place-preservation + Inspector ROUND-TRIP
 * matrix (task 9.7; IU-10 / UIE-M-005; design §20 Place tolerance, §20.3/§20.4
 * Inspector, §21 IU-10 "one restoration path").
 *
 * This is the COHESIVE integration/round-trip suite over everything built in
 * 9.2–9.6. It does NOT re-derive the unit proofs already established:
 *   • anchor math + coordinator restore-exactly-once → conversationPlace.test.ts
 *   • keyboard/wheel traversal contract           → streamTraversal.test.ts +
 *                                                    MessageStream.test.tsx
 *   • Inspector non-stacking + removal auto-close  → InspectorHost.test.tsx +
 *                                                    overlayInterruption.test.tsx
 *   • viewport exclusion from shell place capture  → placePreservation.test.ts
 *   • scroll owners + composer clearance           → ConverseSpace.test.tsx
 * It instead asserts the SEVEN cross-cutting round-trip scenarios hold together.
 *
 * jsdom has no real layout, so — per the task — we assert the DETERMINISTIC
 * contract (anchor computed/restored via the single owner, stick flags, the
 * coordinator restore-count, exactly one `<aside>`, focus target), stubbing
 * virtualizer/scroll dimensions where needed. Real pixel-landing is deferred to
 * 9.10 E2E; perf/virtualization to 9.8; a11y/Wayland to 9.9.
 *
 * Validates: Requirements 10.4, 10.5, 10.11, 11.5, 11.6, 11.7, 11.12, 15.5,
 * 15.6, 15.7, 16.4, 21.6, 21.7, 21.8 (design §20, §20.3/§20.4, §21 IU-10).
 */
import { describe, it, expect, beforeEach, afterEach } from "vitest";
import { render, screen, fireEvent, cleanup } from "@solidjs/testing-library";
import { createSignal } from "solid-js";

import { MessageStream } from "./MessageStream";
import ConverseSpace from "../ConverseSpace";
import { InspectorHost } from "../../InspectorHost";
import { registerInspectorRenderer, resetInspectorRegistry } from "../../inspectorRegistry";
import { converseStore, shellStore, type Message, type Thread } from "../../../stores";
import { navigate, currentRoute } from "../../router";
import { isConversationOwnedScroller } from "../../placePreservation";
import { whyDidKriaAnswer } from "./messageActions";
import {
  beginConversationPlace,
  captureConversationPlace,
  endConversationPlace,
  resolveConversationRestore,
  restoreConversationPlace,
  __conversationRestoreCount,
  __resetConversationPlace,
} from "./conversationPlace";
import { resolveTraversalIntent, shouldRevealFocusedRow } from "./streamTraversal";

// ── helpers ──────────────────────────────────────────────────────────────────

/** Flush queued microtasks (focus return / auto-focus are microtask-deferred). */
const flush = async () => {
  await Promise.resolve();
  await Promise.resolve();
};
/** Flush the macrotask queue (reactive effects + microtasks). */
const tick = () => new Promise<void>((r) => setTimeout(r, 0));

function makeThread(id: string, updatedAt: number): Thread {
  return { id, title: `Thread ${id}`, createdAt: 0, updatedAt, pinned: false, archived: false, temporary: false };
}

/** Seed an active thread `t1` with `count` messages. */
function seedThread(count: number, threadId = "t1"): void {
  converseStore.clearMessages();
  converseStore.setThreads([makeThread(threadId, 2), makeThread("history", 1)]);
  converseStore.setActiveThread(threadId);
  for (let i = 0; i < count; i += 1) {
    const m: Message = {
      id: `m${i}`,
      threadId,
      role: i % 2 === 0 ? "user" : "assistant",
      content: `Message number ${i}`,
      timestamp: Date.now() + i,
    };
    converseStore.addMessage(m);
  }
}

/** Render MessageStream inside a fixed-height viewport so the virtualizer has a window. */
function renderStream() {
  return render(() => (
    <div style={{ height: "400px" }}>
      <MessageStream />
    </div>
  ));
}

function viewportOf(container: HTMLElement): HTMLElement {
  const el = container.querySelector<HTMLElement>(".kria-stream__viewport");
  if (!el) throw new Error("stream viewport not found");
  return el;
}

function resetShared(): void {
  __resetConversationPlace();
  resetInspectorRegistry();
  shellStore.setWindowMode("standard");
  shellStore.setInspectorTarget(null);
  shellStore.setActiveSpace("converse");
  converseStore.clearMessages();
  converseStore.setThreads([]);
  converseStore.setActiveThread(null);
}

beforeEach(resetShared);
afterEach(() => {
  cleanup();
  resetShared();
  document.body.innerHTML = "";
});

// ─────────────────────────────────────────────────────────────────────────────
// 1. Long virtualized thread — virtualization stays active; the single owner can
//    capture/restore a message-id anchor.
// ─────────────────────────────────────────────────────────────────────────────

describe("1. long virtualized thread (Req 16.2 / §21 IU-10)", () => {
  it("mounts only a bounded window of a huge thread and the owner captures a resolvable message-id anchor", () => {
    const total = 2000;
    seedThread(total);
    const { container } = renderStream();

    // Virtualization: rendered rows are a bounded window << total.
    const rendered = screen.getAllByRole("article").length;
    expect(rendered).toBeGreaterThan(0);
    expect(rendered).toBeLessThan(total);
    expect(container.querySelectorAll(".kria-stream__row").length).toBe(rendered);

    // The registered owner (MessageStream) captures an anchor whose message id
    // resolves back to a real index in the store (the round-trip identity §20).
    const anchor = captureConversationPlace();
    expect(anchor).not.toBeNull();
    expect(anchor!.anchorMessageId).toBeTruthy();
    const index = converseStore.messages().findIndex((m) => m.id === anchor!.anchorMessageId);
    expect(index).toBeGreaterThanOrEqual(0);
    // Restore by id resolves (bottom/anchor) and never throws against the owner.
    const plan = resolveConversationRestore(anchor, (id) =>
      converseStore.messages().findIndex((m) => m.id === id),
    );
    expect(plan.kind === "bottom" || plan.kind === "anchor").toBe(true);
    expect(() => restoreConversationPlace(anchor)).not.toThrow();
  });
});

// ─────────────────────────────────────────────────────────────────────────────
// 2. Near-bottom follow — at bottom follows tail (no jump control); scrolled up
//    does NOT yank + jump-to-latest appears; End re-engages follow.
// ─────────────────────────────────────────────────────────────────────────────

describe("2. near-bottom follow / jump-to-latest (Req 11.5–11.7 / §21)", () => {
  it("follows at bottom, holds place when scrolled up (no yank), and End re-engages follow", async () => {
    seedThread(50);
    const { container } = renderStream();
    const el = viewportOf(container);

    // At bottom (stick=true, default + onMount): no jump control offered.
    expect(screen.queryByRole("button", { name: "Jump to latest message" })).toBeNull();

    // Scroll up (Home releases stick) → jump-to-latest appears.
    fireEvent.keyDown(el, { key: "Home" });
    const jump = await screen.findByRole("button", { name: "Jump to latest message" });
    expect(jump).toBeInTheDocument();

    // A new message arriving while scrolled up does NOT yank to bottom: the jump
    // control stays (stick is still released), position is held.
    converseStore.addMessage({
      id: "m50",
      threadId: "t1",
      role: "assistant",
      content: "new tail message",
      timestamp: Date.now() + 999,
    });
    expect(screen.getByRole("button", { name: "Jump to latest message" })).toBeInTheDocument();

    // End re-engages the follow tail → jump control disappears.
    fireEvent.keyDown(el, { key: "End" });
    expect(screen.queryByRole("button", { name: "Jump to latest message" })).toBeNull();

    // Jump control also re-engages follow when clicked (scroll up again first).
    fireEvent.keyDown(el, { key: "Home" });
    fireEvent.click(await screen.findByRole("button", { name: "Jump to latest message" }));
    expect(screen.queryByRole("button", { name: "Jump to latest message" })).toBeNull();
  });
});

// ─────────────────────────────────────────────────────────────────────────────
// 3. Focused offscreen action — an action outside the visible band is revealed;
//    a fully-visible action does not scroll. (Traversal wiring proven in 9.6;
//    here we assert the reveal DECISION contract used by the round-trip.)
// ─────────────────────────────────────────────────────────────────────────────

describe("3. focused offscreen action reveal (Req 10.11 / §21 UIE-M-005)", () => {
  it("reveals an offscreen / unrendered focused row but leaves a fully-visible one alone", () => {
    // Offscreen below the viewport band → reveal.
    expect(shouldRevealFocusedRow({ start: 5000, end: 5100 }, 0, 600)).toBe(true);
    // Offscreen above → reveal.
    expect(shouldRevealFocusedRow({ start: 0, end: 100 }, 400, 600)).toBe(true);
    // Not currently rendered (no geometry) → reveal by index.
    expect(shouldRevealFocusedRow(null, 0, 600)).toBe(true);
    // Fully within the visible band → NO scroll (must not fight follow/stick).
    expect(shouldRevealFocusedRow({ start: 100, end: 200 }, 0, 600)).toBe(false);
  });

  it("keeps the viewport keyboard-focusable so an offscreen action can be reached without a trap", () => {
    seedThread(80);
    const { container } = renderStream();
    expect(viewportOf(container).getAttribute("tabindex")).toBe("0");
    // End maps to the follow-tail re-engage intent (used after focus reveal).
    expect(resolveTraversalIntent({ key: "End", editableFocus: false })).toEqual({ kind: "bottom" });
  });
});

// ─────────────────────────────────────────────────────────────────────────────
// 4. Lane toggle — toggling Work/Context/ThreadSidebar preserves the conversation
//    place (viewport delegated to the single owner, never captured by the shell)
//    and never triggers a competing stream restoration; composer keeps focus.
// ─────────────────────────────────────────────────────────────────────────────

describe("4. lane toggle preserves place, no competing restore (Req 10.4/10.5/15.5)", () => {
  const OriginalRO = globalThis.ResizeObserver;
  class FullWidthResizeObserver {
    constructor(private readonly cb: ResizeObserverCallback) {}
    observe(target: Element): void {
      this.cb([{ target, contentRect: { width: 1440 } } as ResizeObserverEntry], this as unknown as ResizeObserver);
    }
    unobserve(): void {}
    disconnect(): void {}
  }

  beforeEach(() => {
    globalThis.ResizeObserver = FullWidthResizeObserver as unknown as typeof ResizeObserver;
  });
  afterEach(() => {
    globalThis.ResizeObserver = OriginalRO;
  });

  it("keeps the same virtualized stream + delegated viewport while lanes toggle, with zero stream restores", () => {
    seedThread(300);
    converseStore.setContextRailItems([
      { id: "ctx-1", type: "memory", label: "Context 1", data: {} },
    ]);
    converseStore.addWorkBlock({ id: "wb-1", type: "tool-call", status: "running", summary: "work", startedAt: Date.now() });

    const { container } = render(() => <ConverseSpace />);
    const stream = container.querySelector('[data-region="message-stream-virtual"]');
    const viewport = viewportOf(container);
    expect(stream).not.toBeNull();

    // The conversation viewport is delegated to the single owner: the shell's
    // place preservation must treat it as conversation-owned (excluded).
    expect(isConversationOwnedScroller(viewport)).toBe(true);

    const restoresBefore = __conversationRestoreCount();
    const renderedBefore = screen.getAllByRole("article").length;
    expect(renderedBefore).toBeGreaterThan(0);
    expect(renderedBefore).toBeLessThan(300);

    // Focus the composer, then toggle lanes that are NOT the focused region:
    // focus must not be stolen by the lane churn.
    const composer = container.querySelector<HTMLElement>(".kria-composer__textarea");
    expect(composer).not.toBeNull();
    composer!.focus();
    expect(document.activeElement).toBe(composer);

    fireEvent.click(screen.getByRole("button", { name: "Toggle context rail" }));
    fireEvent.click(screen.getByRole("button", { name: "Close thread sidebar" }));
    converseStore.clearWorkBlocks();

    // Same stream instance (never remounted), virtualization still active.
    expect(container.querySelector('[data-region="message-stream-virtual"]')).toBe(stream);
    expect(screen.getAllByRole("article").length).toBeGreaterThan(0);
    expect(screen.getAllByRole("article").length).toBeLessThan(300);
    // Composer kept focus (lane toggles that don't remove the focused region
    // never steal it via focusStableConversationControl).
    expect(document.activeElement).toBe(composer);
    // No conversation restore was triggered by lane toggling — no shell/stream
    // competition (only mode + approval transitions restore the stream).
    expect(__conversationRestoreCount()).toBe(restoresBefore);
  });
});

// ─────────────────────────────────────────────────────────────────────────────
// 5. Inspector replace / close / target removal — one non-stacking <aside>,
//    forward focus on replace, close returns to the original opener, and a
//    registered renderer → null (target removed while open) auto-closes exactly
//    once with §20.4 region focus and no stray target.
// ─────────────────────────────────────────────────────────────────────────────

describe("5. Inspector replace / close / target removal (§20.1/§20.3/§20.4)", () => {
  it("replace keeps one panel + forward focus; close returns to the original opener", async () => {
    render(() => <InspectorHost />);
    const opener = document.createElement("button");
    document.body.appendChild(opener);
    opener.focus();

    shellStore.openInspector("memory", "A", undefined, { opener });
    shellStore.openInspector("capability", "B"); // REPLACE (no stacking)
    const panels = screen.getAllByRole("complementary", { name: "Inspector" });
    expect(panels).toHaveLength(1);
    expect(panels[0]).toHaveAttribute("data-inspector-type", "capability");
    // Forward focus into the fresh panel, not back to the opener.
    expect(panels[0].contains(document.activeElement)).toBe(true);
    expect(document.activeElement).not.toBe(opener);

    shellStore.setInspectorTarget(null); // close → returnFocus to the original opener
    await flush();
    expect(document.activeElement).toBe(opener);
    expect(screen.queryByRole("complementary", { name: "Inspector" })).toBeNull();
    opener.remove();
  });

  it("target removal (registered renderer → null) auto-closes EXACTLY once with §20.4 region focus, no stray target", async () => {
    const [live, setLive] = createSignal(true);
    registerInspectorRenderer("device", () => (live() ? { title: "Device", body: null } : null));

    const region = document.createElement("section");
    region.setAttribute("data-space", "machines");
    const stray = document.createElement("button");
    document.body.append(stray, region);
    stray.focus();

    // Count closes to prove "exactly once".
    let closes = 0;
    const origClose = shellStore.closeInspector;
    (shellStore as { closeInspector: () => void }).closeInspector = () => {
      closes += 1;
      origClose();
    };
    try {
      render(() => <InspectorHost />);
      shellStore.openInspector("device", "dev-1", undefined, { region });
      expect(screen.getByRole("complementary", { name: "Inspector" })).toBeInTheDocument();

      setLive(false); // entity deleted from its source store while open
      await tick();

      expect(shellStore.inspectorTarget()).toBeNull(); // no stray target
      expect(screen.queryByRole("complementary", { name: "Inspector" })).toBeNull();
      expect(closes).toBe(1); // auto-closed exactly once
      expect(document.activeElement).toBe(region); // §20.4 stable-region fallback
      expect(document.activeElement).not.toBe(stray);
    } finally {
      (shellStore as { closeInspector: () => void }).closeInspector = origClose;
      region.remove();
      stray.remove();
    }
  });
});

// ─────────────────────────────────────────────────────────────────────────────
// 6. Route change — whyDidKriaAnswer opens the Inspector AND changes route (the
//    invoking control unmounts); close returns focus to #space-root; the route
//    change does not reset draft / active thread / selection.
// ─────────────────────────────────────────────────────────────────────────────

describe("6. route-change Inspector open (whyDidKriaAnswer, §20.4)", () => {
  it("changes route, opens the Inspector, preserves draft/thread, and closes back to #space-root", async () => {
    seedThread(3);
    converseStore.updateDraft({ text: "half-written question", mode: "assistant" });
    const restoresBefore = __conversationRestoreCount();

    // Stable primary-workspace landmark + a stray element that holds focus after
    // the invoking Converse control unmounts on the route change.
    const spaceRoot = document.createElement("main");
    spaceRoot.id = "space-root";
    spaceRoot.tabIndex = -1;
    const stray = document.createElement("button");
    document.body.append(stray, spaceRoot);
    stray.focus();

    render(() => <InspectorHost />);

    const answer: Message = {
      id: "a1",
      threadId: "t1",
      role: "assistant",
      content: "answer",
      timestamp: Date.now(),
      usedMemoryIds: ["mem-42"],
    };
    whyDidKriaAnswer(answer);

    // Route changed to the Memory Space; Inspector opened on the memory id.
    expect(currentRoute().space).toBe("memory");
    expect(shellStore.inspectorTarget()).toEqual({ type: "memory", id: "mem-42", data: undefined });
    expect(screen.getByRole("complementary", { name: "Inspector" })).toBeInTheDocument();

    // Route change did NOT reset the composer draft, active thread, or messages.
    expect(converseStore.composerDraft().text).toBe("half-written question");
    expect(converseStore.activeThreadId()).toBe("t1");
    expect(converseStore.messages()).toHaveLength(3);
    // …and did not double-restore the stream (only mode/approval restore it).
    expect(__conversationRestoreCount()).toBe(restoresBefore);

    // Close → focus returns to the stable #space-root landmark, not the stray.
    shellStore.setInspectorTarget(null);
    await flush();
    expect(document.activeElement).toBe(spaceRoot);
    expect(document.activeElement).not.toBe(stray);

    spaceRoot.remove();
    stray.remove();
    navigate("converse");
  });
});

// ─────────────────────────────────────────────────────────────────────────────
// 7. Mode / profile round-trip — Window Mode standard→immersive→standard and
//    Width Profile focus↔full round-trips preserve the conversation anchor
//    (restore EXACTLY once per transition via the coordinator), Composer draft,
//    active thread, and Inspector target, and never double-restore the stream.
// ─────────────────────────────────────────────────────────────────────────────

describe("7. mode / profile round-trip (§21 IU-10 one restoration path)", () => {
  it("Window Mode standard→immersive→standard restores the stream exactly once per transition, preserving draft/thread/inspector", () => {
    seedThread(50);
    renderStream(); // registers the real single owner
    converseStore.updateDraft({ text: "draft to keep", mode: "assistant" });
    shellStore.openInspector("memory", "target-keep");

    expect(__conversationRestoreCount()).toBe(0);

    // Each Window Mode transition = one coordinated begin/end pair (what AppShell
    // wires to shell:mode-changing / shell:mode-changed).
    const transition = () => {
      beginConversationPlace();
      endConversationPlace();
    };
    transition(); // standard → immersive
    transition(); // immersive → standard

    // Exactly one restore per transition — never double-restored, never skipped.
    expect(__conversationRestoreCount()).toBe(2);

    // Round-trip preserved the domain state the coordinator never owns.
    expect(converseStore.composerDraft().text).toBe("draft to keep");
    expect(converseStore.activeThreadId()).toBe("t1");
    expect(shellStore.inspectorTarget()?.id).toBe("target-keep");
  });

  it("a mode change coinciding with a pending approval restores the stream exactly once (overlap)", () => {
    seedThread(20);
    renderStream();
    const before = __conversationRestoreCount();

    // Overlap: mode-changing begins, approval becomes pending (nested), mode
    // settles, then the approval queue clears — a single coordinated restore.
    beginConversationPlace(); // P-A mode-changing
    beginConversationPlace(); // P-B approval pending
    endConversationPlace(); // P-A settled (one transition still open)
    expect(__conversationRestoreCount()).toBe(before); // not restored yet
    endConversationPlace(); // P-B cleared → the single restore

    expect(__conversationRestoreCount()).toBe(before + 1);
  });

  it("Width Profile focus↔full round-trip does NOT restore/double-restore the stream and preserves the draft", () => {
    const OriginalRO = globalThis.ResizeObserver;

    class ControlledResizeObserver {
      static instances: ControlledResizeObserver[] = [];
      readonly observed = new Set<Element>();
      constructor(private readonly cb: ResizeObserverCallback) {
        ControlledResizeObserver.instances.push(this);
      }
      observe(target: Element): void {
        this.observed.add(target);
      }
      unobserve(target: Element): void {
        this.observed.delete(target);
      }
      disconnect(): void {
        this.observed.clear();
      }
      emit(inlineSize: number): void {
        const target = this.observed.values().next().value as Element;
        if (!target) return;
        this.cb(
          [
            {
              target,
              contentBoxSize: [{ inlineSize, blockSize: 600 }],
              contentRect: { width: inlineSize },
            } as unknown as ResizeObserverEntry,
          ],
          this as unknown as ResizeObserver,
        );
      }
    }

    globalThis.ResizeObserver = ControlledResizeObserver as unknown as typeof ResizeObserver;
    try {
      seedThread(120);
      converseStore.updateDraft({ text: "profile draft", mode: "assistant" });
      const { container } = render(() => <ConverseSpace />);
      const root = container.querySelector<HTMLElement>(".kria-converse")!;
      const stream = container.querySelector('[data-region="message-stream-virtual"]');
      // Pick the observer watching the Converse root (others watch the stream).
      const observer = ControlledResizeObserver.instances.find((o) => o.observed.has(root))!;
      expect(observer).toBeDefined();

      const restoresBefore = __conversationRestoreCount();

      observer.emit(700); // Focus (<720)
      expect(root.getAttribute("data-width-profile")).toBe("focus");
      observer.emit(1500); // Full (>=1440)
      expect(root.getAttribute("data-width-profile")).toBe("full");
      observer.emit(700); // back to Focus — round trip
      expect(root.getAttribute("data-width-profile")).toBe("focus");

      // Profile flips are CSS-only: they never invoke the stream restoration
      // owner (no double-restore), and the same stream instance is preserved.
      expect(__conversationRestoreCount()).toBe(restoresBefore);
      expect(container.querySelector('[data-region="message-stream-virtual"]')).toBe(stream);
      // Draft survives the profile round-trip.
      expect(converseStore.composerDraft().text).toBe("profile draft");
    } finally {
      globalThis.ResizeObserver = OriginalRO;
    }
  });
});
