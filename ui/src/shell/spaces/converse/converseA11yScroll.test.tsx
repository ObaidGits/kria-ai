import { describe, it, expect, beforeEach, afterEach } from "vitest";
import { render, screen, cleanup, fireEvent } from "@solidjs/testing-library";
import ConverseSpace from "../ConverseSpace";
import { InspectorHost } from "../../InspectorHost";
import { converseStore, coreStore, shellStore } from "../../../stores";
import type { Message } from "../../../stores/converseStore";
import { resolveTraversalIntent } from "./streamTraversal";

/**
 * Task 9.9 — CHEAP jsdom-verifiable a11y + scroll-ownership validations for the
 * Phase-5 Converse Space (IU-10 / UIE-M-005). Covers the four validations that
 * do NOT need a real compositor:
 *
 *   1. Keyboard      — viewport focusable (tabindex=0) + a scroll-key aria-label,
 *      traversal keys act while Tab / editable-child keys pass through (no trap).
 *   2. SR context    — exactly ONE conversation live region (role=log /
 *      aria-live=polite on `.kria-converse__stream`); the viewport is NOT a
 *      second live region; message rows are <article>; lane landmarks
 *      (Threads/Conversation/Work/Context/Inspector/Composer) are present and
 *      uniquely labelled (Req 12.11).
 *   3. Reduced motion — CSS-string proof that stream/overlay/inspector/lane
 *      entrance motion is frozen under BOTH `@media (prefers-reduced-motion:
 *      reduce)` AND `:root[data-reduced-motion="on"]`, and that stream
 *      traversal/restoration uses instant scroll (no smooth-scroll) (Req 16.4).
 *   4. Mode × profile — the single-live-region / one-scroll-owner-per-axis /
 *      one-Inspector / no-focus-trap invariants hold across ALL
 *      (standard|compact|immersive) × (focus|dual|assisted|full) combinations.
 *
 * Real GNOME/KDE wheel/touchpad momentum, nested Work-lane scroll chaining,
 * scrollbar reserve, and axe are AUTHORED as browser specs
 * (`task-9.9-scroll-a11y.spec.ts`) and executed at phase-gate 9.10/9.11.
 */

const testDefaultResizeObserver = globalThis.ResizeObserver;

/** Width profile boundaries: focus <736, dual 736–1055, assisted 1056–1439, full ≥1440. */
const PROFILE_WIDTH = { focus: 500, dual: 900, assisted: 1200, full: 1500 } as const;
type Profile = keyof typeof PROFILE_WIDTH;

/** ResizeObserver whose emitted width is set per test (drives data-width-profile). */
let observedWidth = 1440;
class ParamWidthResizeObserver {
  constructor(private readonly callback: ResizeObserverCallback) {}
  observe(target: Element): void {
    this.callback(
      [{ target, contentRect: { width: observedWidth } } as ResizeObserverEntry],
      this as unknown as ResizeObserver,
    );
  }
  unobserve(): void {}
  disconnect(): void {}
}

function seedMessages(count: number): void {
  converseStore.clearMessages();
  for (let index = 0; index < count; index += 1) {
    const message: Message = {
      id: `a11y-message-${index}`,
      threadId: "a11y-thread",
      role: index % 2 === 0 ? "user" : "assistant",
      content: `A11y message ${index}`,
      timestamp: index,
    };
    converseStore.addMessage(message);
  }
}

function seedLanes(): void {
  converseStore.addWorkBlock({
    id: "a11y-work",
    type: "tool-call",
    status: "running",
    summary: "work a11y",
    startedAt: Date.now(),
  });
  converseStore.setContextRailItems([
    { id: "a11y-context", type: "memory", label: "Context a11y", data: { source: "test" } },
  ]);
}

describe("Task 9.9 — Converse keyboard + SR-visible context (Req 12.11, 19.1–19.7)", () => {
  beforeEach(() => {
    observedWidth = 1440; // full profile → all lanes admitted
    globalThis.ResizeObserver = ParamWidthResizeObserver as unknown as typeof ResizeObserver;
    shellStore.setWindowMode("standard");
    shellStore.setInspectorTarget(null);
    converseStore.clearMessages();
    coreStore.reset();
    converseStore.setThreads([
      { id: "a11y-active", title: "Active", createdAt: 0, updatedAt: 2, pinned: false, archived: false, temporary: false },
    ]);
    converseStore.setActiveThread("a11y-active");
  });

  afterEach(() => {
    cleanup();
    globalThis.ResizeObserver = testDefaultResizeObserver;
    shellStore.setInspectorTarget(null);
  });

  it("makes the stream viewport keyboard-focusable with a scroll-key aria-label and no live-region duplication", () => {
    seedMessages(8);
    const { container } = render(() => <ConverseSpace />);
    const viewport = container.querySelector<HTMLElement>(".kria-stream__viewport")!;
    expect(viewport).not.toBeNull();
    // Focusable so keyboard users can drive Page/Home/End scroll.
    expect(viewport.getAttribute("tabindex")).toBe("0");
    // Label names the scroll keys for AT users (Req 12.11).
    const label = viewport.getAttribute("aria-label") ?? "";
    expect(label).toMatch(/Page Up/i);
    expect(label).toMatch(/Home and End/i);
    // The viewport is NOT a second live region (the wrapper owns the sole one).
    expect(viewport.getAttribute("aria-live")).toBeNull();
    expect(viewport.getAttribute("role")).not.toBe("log");
    // It carries the single conversation scroll-owner marker.
    expect(viewport.getAttribute("data-scroll-owner")).toBe("conversation");
  });

  it("acts only on Page/Home/End and lets Tab + editable-child keys pass through (no focus trap)", () => {
    // Traversal contract (the wiring MessageStream applies): only scroll keys
    // act (preventDefault → handled); Tab and editable-child keys → none, so
    // focus/caret leave the region naturally.
    expect(resolveTraversalIntent({ key: "Home", editableFocus: false }).kind).toBe("top");
    expect(resolveTraversalIntent({ key: "End", editableFocus: false }).kind).toBe("bottom");
    expect(resolveTraversalIntent({ key: "PageUp", editableFocus: false }).kind).toBe("page");
    expect(resolveTraversalIntent({ key: "PageDown", editableFocus: false }).kind).toBe("page");
    expect(resolveTraversalIntent({ key: "Tab", editableFocus: false }).kind).toBe("none");
    // Any key while focus is in an editable control is deferred (never hijacked).
    expect(resolveTraversalIntent({ key: "Home", editableFocus: true }).kind).toBe("none");
    expect(resolveTraversalIntent({ key: "PageDown", editableFocus: true }).kind).toBe("none");
  });

  it("exposes exactly ONE conversation live region and keeps the viewport out of it (Req 12.11)", () => {
    seedMessages(6);
    const { container } = render(() => <ConverseSpace />);
    // Single role=log region for the conversation.
    const logs = container.querySelectorAll('[role="log"]');
    expect(logs).toHaveLength(1);
    const log = logs[0] as HTMLElement;
    expect(log.classList.contains("kria-converse__stream")).toBe(true);
    expect(log.getAttribute("aria-live")).toBe("polite");
    // Only ONE aria-live region in the rendered Converse tree (the stream).
    const liveRegions = container.querySelectorAll("[aria-live]");
    expect(liveRegions).toHaveLength(1);
    expect(liveRegions[0]).toBe(log);
    // The focusable viewport lives INSIDE the log but is not itself live.
    const viewport = log.querySelector<HTMLElement>(".kria-stream__viewport")!;
    expect(viewport).not.toBeNull();
    expect(viewport.getAttribute("aria-live")).toBeNull();
  });

  it("renders message rows as <article> so AT can enumerate turns", () => {
    seedMessages(6);
    render(() => <ConverseSpace />);
    const articles = screen.getAllByRole("article");
    expect(articles.length).toBeGreaterThan(0);
  });

  it("exposes a polite copy-outcome status region WITHOUT adding a second [aria-live] (Req 12.3, UIE-M-009)", () => {
    seedMessages(6);
    const { container } = render(() => <ConverseSpace />);
    // The copy announcer is a dedicated status region for success/failure.
    const announcer = container.querySelector<HTMLElement>('[data-region="copy-announcer"]')!;
    expect(announcer).not.toBeNull();
    // role="status" carries IMPLICIT aria-live=polite → concise, non-interrupting.
    expect(announcer.getAttribute("role")).toBe("status");
    // It deliberately does NOT set an explicit aria-live, so it never becomes a
    // second [aria-live] region — the single conversation live region (the log)
    // stays the sole [aria-live] node.
    expect(announcer.getAttribute("aria-live")).toBeNull();
    const liveRegions = container.querySelectorAll("[aria-live]");
    expect(liveRegions).toHaveLength(1);
    expect((liveRegions[0] as HTMLElement).classList.contains("kria-converse__stream")).toBe(true);
    // The announcer is separate from (outside) the conversation log region.
    const log = container.querySelector<HTMLElement>('[role="log"]')!;
    expect(log.contains(announcer)).toBe(false);
  });

  it("presents uniquely-labelled lane landmarks (Threads/Conversation/Context/Inspector/Composer)", async () => {
    seedMessages(6);
    seedLanes();
    const { container } = render(() => (
      <>
        <ConverseSpace />
        <InspectorHost />
      </>
    ));
    shellStore.openInspector("memory", "a11y-inspector");
    // Reveal the on-demand Context rail so its landmark is present.
    fireEvent.click(screen.getByRole("button", { name: "Toggle context rail" }));

    // One nav landmark for Threads.
    expect(screen.getAllByRole("navigation", { name: "Threads" })).toHaveLength(1);
    // One focal Conversation region.
    expect(screen.getAllByRole("region", { name: "Conversation" })).toHaveLength(1);
    // Work is inline per-turn now (no Work lane landmark). The remaining
    // complementary landmarks are each UNIQUELY labelled (no duplicates).
    expect(screen.queryByRole("complementary", { name: "Work" })).toBeNull();
    expect(await screen.findByRole("complementary", { name: "Context" })).toBeInTheDocument();
    expect(await screen.findByRole("complementary", { name: "Inspector" })).toBeInTheDocument();
    // Complementary labels are unique: exactly one per name.
    for (const name of ["Context", "Inspector"]) {
      expect(screen.getAllByRole("complementary", { name })).toHaveLength(1);
    }
    // Composer region is present and labelled (div landmark via aria-label).
    const composer = container.querySelector<HTMLElement>('[data-region="composer"]')!;
    expect(composer).not.toBeNull();
    expect(composer.getAttribute("aria-label")).toBe("Composer");
    // Exactly one Inspector panel (non-stacking).
    expect(container.querySelectorAll(".kria-inspector")).toHaveLength(1);
  });
});

describe("Task 9.9 — reduced motion freezes entrance + uses instant scroll (Req 16.4)", () => {
  it("freezes stream/overlay/inspector/lane motion under BOTH reduced-motion gates", async () => {
    const [
      { default: motionCss },
      { default: layoutCss },
      { default: streamCss },
      { default: shellCss },
    ] = await Promise.all([
      import("../../../styles/motion.css?raw"),
      import("../ConverseSpace.css?raw"),
      import("./MessageStream.css?raw"),
      import("../../AppShell.css?raw"),
    ]);

    // Global kill-switch: media query AND data-attr both freeze animation +
    // transition + smooth scroll for EVERY element (covers the stream surface).
    expect(motionCss).toMatch(/@media \(prefers-reduced-motion: reduce\)/);
    expect(motionCss).toMatch(/:root\[data-reduced-motion="on"\] \*[\s\S]*?animation: none !important;[\s\S]*?transition: none !important;/);
    expect(motionCss).toMatch(/:root\[data-reduced-motion="on"\][\s\S]*?scroll-behavior: auto !important;/);
    expect(motionCss).toMatch(/@media \(prefers-reduced-motion: reduce\)[\s\S]*?\*,[\s\S]*?animation: none !important;/);

    // Lane reveal entrance is explicitly frozen for the Converse lanes.
    expect(layoutCss).toMatch(
      /@media \(prefers-reduced-motion: reduce\)[\s\S]*?\.kria-converse__work,[\s\S]*?\.kria-converse__context[\s\S]*?animation: none;/,
    );
    // Inspector entrance frozen at the shell layer.
    expect(shellCss).toMatch(
      /@media \(prefers-reduced-motion: reduce\)[\s\S]*?\.kria-inspector[\s\S]*?animation: none;/,
    );

    // The stream viewport adds NO smooth-scroll of its own (restoration +
    // traversal use instant scroll), so reduced motion has nothing to undo.
    expect(streamCss).not.toMatch(/scroll-behavior:\s*smooth/);
  });

  it("uses instant scroll in stream traversal/restoration source (no smooth-scroll behavior)", async () => {
    const [{ default: streamSource }, { default: placeSource }] = await Promise.all([
      import("./MessageStream.tsx?raw"),
      import("./conversationPlace.ts?raw"),
    ]);
    // No smooth scroll requested anywhere in the conversation scroll path.
    expect(streamSource).not.toMatch(/behavior:\s*["']smooth["']/);
    expect(streamSource).not.toMatch(/scroll-behavior:\s*smooth/);
    expect(placeSource).not.toMatch(/behavior:\s*["']smooth["']/);
  });
});

describe("Task 9.9 — a11y + scroll invariants across ALL Window Mode × Width Profile combos", () => {
  const MODES = ["standard", "mini", "immersive"] as const;
  const PROFILES = ["focus", "dual", "assisted", "full"] as const;

  beforeEach(() => {
    globalThis.ResizeObserver = ParamWidthResizeObserver as unknown as typeof ResizeObserver;
    coreStore.reset();
    converseStore.clearMessages();
    converseStore.setThreads([
      { id: "matrix-active", title: "Active", createdAt: 0, updatedAt: 2, pinned: false, archived: false, temporary: false },
    ]);
    converseStore.setActiveThread("matrix-active");
  });

  afterEach(() => {
    cleanup();
    globalThis.ResizeObserver = testDefaultResizeObserver;
    shellStore.setInspectorTarget(null);
  });

  for (const mode of MODES) {
    for (const profile of PROFILES) {
      it(`holds single-live-region / one-scroll-owner / one-Inspector / no-trap in ${mode} × ${profile}`, async () => {
        // **Validates: Requirements 10.4, 10.5, 12.11, 16.4, 19.1–19.7**
        observedWidth = PROFILE_WIDTH[profile as Profile];
        shellStore.setWindowMode(mode);
        shellStore.setInspectorTarget(null);
        seedMessages(6);
        seedLanes();

        const { container } = render(() => (
          <>
            <ConverseSpace />
            <InspectorHost />
          </>
        ));
        shellStore.openInspector("memory", `matrix-${mode}-${profile}`);
        await screen.findByRole("complementary", { name: "Inspector" });

        // 1. Single conversation live region (role=log / aria-live=polite).
        const logs = container.querySelectorAll('[role="log"]');
        expect(logs, `${mode}/${profile}: one log region`).toHaveLength(1);
        expect(container.querySelectorAll("[aria-live]"), `${mode}/${profile}: one live region`).toHaveLength(1);

        // 2. Exactly one conversation scroll owner (one Y owner for the stream).
        const owners = container.querySelectorAll('[data-scroll-owner="conversation"]');
        expect(owners, `${mode}/${profile}: one conversation scroll owner`).toHaveLength(1);
        expect(container.querySelectorAll(".kria-stream__viewport"), `${mode}/${profile}: one stream viewport`).toHaveLength(1);

        // 3. Exactly one Inspector panel (non-stacking).
        expect(container.querySelectorAll(".kria-inspector"), `${mode}/${profile}: one Inspector`).toHaveLength(1);

        // 4. No focus trap: viewport is focusable (tabindex=0) and the Inspector
        // is a non-modal complementary (no aria-modal, tabindex=-1 programmatic).
        const viewport = owners[0] as HTMLElement;
        expect(viewport.getAttribute("tabindex"), `${mode}/${profile}: viewport focusable`).toBe("0");
        const inspector = container.querySelector<HTMLElement>(".kria-inspector")!;
        expect(inspector.getAttribute("aria-modal"), `${mode}/${profile}: Inspector non-modal`).toBeNull();
        expect(inspector.getAttribute("role")).toBe("complementary");

        cleanup();
        shellStore.setInspectorTarget(null);
      });
    }
  }
});
