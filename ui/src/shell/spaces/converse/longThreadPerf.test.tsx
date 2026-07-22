/**
 * Long-thread render + rapid-transition PERFORMANCE INVARIANTS (task 9.8;
 * IU-10 / UIE-M-005; design §21 IU-10 "virtualization and shell scroll
 * restoration can compete", §20 Place tolerance, Req 16.4).
 *
 * These are the CHEAP, deterministic invariants that prove the two perf-critical
 * guarantees WITHOUT frame timing (which jsdom cannot provide):
 *
 *   1. Virtualization stays ACTIVE under a long thread — the rendered row window
 *      is BOUNDED and does NOT grow with the thread size (500 vs 5000 → similar,
 *      not proportional). This is the invariant that keeps a long thread cheap.
 *   2. NO duplicate restoration under rapid / overlapping transitions — the
 *      single-owner coordinator (`beginConversationPlace`/`endConversationPlace`)
 *      restores EXACTLY ONCE per settled outermost transition, never more, even
 *      under rapid-fire sequences and nested overlap (the §21 IU-10 risk this
 *      task eliminates).
 *   3. Rapid Width-Profile churn is CSS-only — it never invokes the stream
 *      restoration owner and never remounts the stream (reuses the 9.7
 *      ControlledResizeObserver pattern).
 *
 * Real frame timing / compositor behaviour / landed pixels are deferred to the
 * browser perf spec (`ui/e2e/task-9.8-longthread-perf.spec.ts`) and the
 * phase-gate browser run (9.10/9.11).
 *
 * Validates: Requirements 10.4, 11.6, 11.7, 15.5, 16.4, 21.6, 21.7, 21.8
 * (design §20, §21 IU-10).
 */
import { describe, it, expect, beforeEach, afterEach } from "vitest";
import { render, screen, cleanup } from "@solidjs/testing-library";

import { MessageStream } from "./MessageStream";
import ConverseSpace from "../ConverseSpace";
import { converseStore, type Message, type Thread } from "../../../stores";
import {
  beginConversationPlace,
  captureConversationPlace,
  endConversationPlace,
  __conversationRestoreCount,
  __resetConversationPlace,
} from "./conversationPlace";

// ── helpers ──────────────────────────────────────────────────────────────────

function makeThread(id: string, updatedAt: number): Thread {
  return { id, title: `Thread ${id}`, createdAt: 0, updatedAt, pinned: false, archived: false, temporary: false };
}

/** Seed an active thread with `count` messages (bounded-window subject). */
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
    <div style={{ height: "600px" }}>
      <MessageStream />
    </div>
  ));
}

/** Rendered row window size (bounded virtualized window, not the full thread). */
function renderedRowCount(container: HTMLElement): number {
  return container.querySelectorAll(".kria-stream__row").length;
}

function resetShared(): void {
  __resetConversationPlace();
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
// 1. Virtualization stays active — bounded window, INDEPENDENT of thread size.
// ─────────────────────────────────────────────────────────────────────────────

describe("1. virtualization stays active under a long thread (Req 16.4 / §21 IU-10)", () => {
  it("renders a bounded row window that does NOT grow with the thread (500 vs 5000)", () => {
    // Small representative thread.
    seedThread(500);
    const small = renderStream();
    const rendered500 = renderedRowCount(small.container);
    const articles500 = screen.getAllByRole("article").length;
    // The bounded window matches the rendered rows (1 article per row).
    expect(articles500).toBe(rendered500);
    // Virtualized: a window, never the whole thread.
    expect(rendered500).toBeGreaterThan(0);
    expect(rendered500).toBeLessThan(500);

    cleanup();
    __resetConversationPlace();

    // 10× larger thread.
    seedThread(5000);
    const large = renderStream();
    const rendered5000 = renderedRowCount(large.container);
    expect(rendered5000).toBeGreaterThan(0);
    expect(rendered5000).toBeLessThan(5000);

    // The core perf invariant: a 10× thread does NOT produce a 10× (or even 2×)
    // render window. The window is bounded by the viewport + overscan, not the
    // thread size, so idle/scroll cost stays constant as the thread grows.
    expect(rendered5000).toBeLessThanOrEqual(rendered500 * 2);
    // Absolute ceiling: the window is a small constant (viewport/estimate +
    // overscan*2), never a large fraction of the thread.
    expect(rendered5000).toBeLessThan(80);

    // Virtualization active ⇒ the single owner still captures a resolvable
    // message-id anchor at 5000 messages (no degradation of place at scale).
    const anchor = captureConversationPlace();
    expect(anchor).not.toBeNull();
    expect(anchor!.anchorMessageId).toBeTruthy();
    const index = converseStore.messages().findIndex((m) => m.id === anchor!.anchorMessageId);
    expect(index).toBeGreaterThanOrEqual(0);
  });
});

// ─────────────────────────────────────────────────────────────────────────────
// 2. No duplicate restoration under rapid / overlapping transitions.
// ─────────────────────────────────────────────────────────────────────────────

describe("2. no duplicate restoration under rapid transitions (§21 IU-10 one restoration path)", () => {
  it("restores EXACTLY once per settled sequential transition across a rapid-fire run", () => {
    seedThread(300);
    renderStream(); // registers the real single owner

    const before = __conversationRestoreCount();
    const RAPID = 50;
    // 50 rapid begin/end pairs — each is one settled outermost transition.
    for (let i = 0; i < RAPID; i += 1) {
      beginConversationPlace();
      endConversationPlace();
    }
    // Exactly one restore per settled transition — never doubles, never skips.
    expect(__conversationRestoreCount()).toBe(before + RAPID);
  });

  it("restores exactly once per OUTERMOST transition under nested overlap (never double-restores)", () => {
    seedThread(120);
    renderStream();

    const before = __conversationRestoreCount();

    // Deeply nested overlap (mode change + approval + more, arriving before any
    // settle): a single outermost transition → exactly one restore.
    beginConversationPlace();
    beginConversationPlace();
    beginConversationPlace();
    expect(__conversationRestoreCount()).toBe(before); // nothing restored while open
    endConversationPlace();
    endConversationPlace();
    expect(__conversationRestoreCount()).toBe(before); // still open (1 remaining)
    endConversationPlace();
    expect(__conversationRestoreCount()).toBe(before + 1); // settled → one restore
  });

  it("counts EXACTLY the number of settled outermost transitions across interleaved overlaps", () => {
    seedThread(80);
    renderStream();

    const before = __conversationRestoreCount();

    // A deterministic interleaving of nested overlaps and standalone transitions.
    // Depth returns to 0 exactly THREE times → exactly three restores.
    // Transition A (overlap depth 3):
    beginConversationPlace();
    beginConversationPlace();
    beginConversationPlace();
    endConversationPlace();
    endConversationPlace();
    endConversationPlace(); // settle #1
    // Transition B (overlap depth 2):
    beginConversationPlace();
    beginConversationPlace();
    endConversationPlace();
    endConversationPlace(); // settle #2
    // Transition C (standalone):
    beginConversationPlace();
    endConversationPlace(); // settle #3

    expect(__conversationRestoreCount()).toBe(before + 3);
  });

  it("ignores an unmatched end (never restores below zero depth)", () => {
    seedThread(20);
    renderStream();
    const before = __conversationRestoreCount();
    // A stray end with no open transition must be a no-op (defensive coordinator).
    endConversationPlace();
    expect(__conversationRestoreCount()).toBe(before);
  });
});

// ─────────────────────────────────────────────────────────────────────────────
// 3. Rapid Width-Profile churn is CSS-only — no stream restore, no remount.
// ─────────────────────────────────────────────────────────────────────────────

describe("3. rapid width-profile churn does not restore or remount the stream (§21 IU-10)", () => {
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
      const target = this.observed.values().next().value as Element | undefined;
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

  beforeEach(() => {
    ControlledResizeObserver.instances = [];
    globalThis.ResizeObserver = ControlledResizeObserver as unknown as typeof ResizeObserver;
  });
  afterEach(() => {
    globalThis.ResizeObserver = OriginalRO;
  });

  it("churns focus↔dual↔assisted↔full rapidly with zero stream restores and one stream instance", () => {
    seedThread(200);
    converseStore.updateDraft({ text: "churn draft", mode: "assistant" });
    const { container } = render(() => <ConverseSpace />);
    const root = container.querySelector<HTMLElement>(".kria-converse")!;
    const stream = container.querySelector('[data-region="message-stream-virtual"]');
    expect(stream).not.toBeNull();

    // Select the observer watching the Converse root (the virtualizer builds its
    // own observers on the stream viewport).
    const observer = ControlledResizeObserver.instances.find((o) => o.observed.has(root))!;
    expect(observer).toBeDefined();

    const restoresBefore = __conversationRestoreCount();

    // Rapid width churn across every profile boundary, many times.
    const widths = [700, 900, 1200, 1500, 900, 700, 1500, 1200, 700, 1500];
    for (const w of widths) observer.emit(w);

    // A width profile is a read-only CSS projection: the churn NEVER invokes the
    // stream restoration owner (no double-restore, no restore at all)…
    expect(__conversationRestoreCount()).toBe(restoresBefore);
    // …and NEVER remounts the stream (same instance across the whole churn).
    expect(container.querySelector('[data-region="message-stream-virtual"]')).toBe(stream);
    // Final profile reflects the last emitted width (full ≥ 1440) → CSS updated.
    expect(root.getAttribute("data-width-profile")).toBe("full");
    // Stream still virtualized after churn (bounded window, not the whole thread).
    const rendered = container.querySelectorAll(".kria-stream__row").length;
    expect(rendered).toBeGreaterThan(0);
    expect(rendered).toBeLessThan(200);
    // Draft survived the churn (no remount / state loss).
    expect(converseStore.composerDraft().text).toBe("churn draft");
  });
});
