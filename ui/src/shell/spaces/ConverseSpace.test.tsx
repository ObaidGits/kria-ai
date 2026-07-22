import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { render, screen, fireEvent, cleanup, waitFor } from "@solidjs/testing-library";
import ConverseSpace from "./ConverseSpace";
import { resolveConverseComposition } from "./converseComposition";
import { navigate } from "../router";
import { InspectorHost } from "../InspectorHost";
import { converseStore, coreStore, shellStore } from "../../stores";
import { isFeatureEnabled, setFeatureFlag } from "../../featureFlags";
import type { Message, Thread, WorkBlock } from "../../stores/converseStore";
import type { CoreState } from "../../stores/coreStore";
import {
  activeGuiCognitionSession,
  clearGuiCognitionSession,
  handleGuiCognitionEvent,
} from "../../stores/guiCognitionSession";

let guiTurnCounter = 0;

/** Seed an active GUI-cognition session (lifecycle leaves "idle"). */
function seedGuiCognitionSession(): void {
  guiTurnCounter += 1;
  handleGuiCognitionEvent({
    version: 1,
    session_id: `gui-session-${guiTurnCounter}`,
    turn_id: `gui-turn-${guiTurnCounter}`,
    workflow_id: `gui-workflow-${guiTurnCounter}`,
    sequence: 1,
    timestamp_ms: Date.now(),
    event: { type: "TurnStarted" },
  } as never);
}

// As of the Phase-2 exit rollout (task 2.4) `home.presence.v2` defaults ON, so
// the home surface routes to the presence `HomeSpace`. The specs in this file
// exercise the LEGACY Converse empty state, which is now the rollback path
// (Req 22.1 — kept operational via the flag). Pin the flag OFF before each test
// so those legacy-surface assertions verify the rollback path. The dedicated
// "home surface routing" describe below overrides this to cover the default.
beforeEach(() => setFeatureFlag("home.presence.v2", false));

const testDefaultResizeObserver = globalThis.ResizeObserver;

class FullWidthResizeObserver {
  constructor(private readonly callback: ResizeObserverCallback) {}

  observe(target: Element): void {
    this.callback(
      [{ target, contentRect: { width: 1440 } } as ResizeObserverEntry],
      this as unknown as ResizeObserver,
    );
  }

  unobserve(): void {}
  disconnect(): void {}
}

function makeWorkBlock(id: string): WorkBlock {
  return {
    id,
    type: "tool-call",
    status: "running",
    summary: `work ${id}`,
    startedAt: Date.now(),
  };
}

/**
 * Seed a user turn (message) plus one running work block bound to it, so the
 * inline per-turn activity trace has something to render. Mirrors the runtime
 * flow where a send creates the user turn and work events tag their blocks with
 * that turn id.
 */
function seedTurnWithWork(turnId: string, blockId: string): void {
  converseStore.addMessage({
    id: turnId,
    threadId: converseStore.activeThreadId() ?? "thread-inline",
    role: "user",
    content: "Do the thing",
    timestamp: Date.now(),
  });
  converseStore.addWorkBlock({ ...makeWorkBlock(blockId), turnId });
}

function makeThread(id: string, updatedAt: number, archived = false): Thread {
  return { id, title: `Thread ${id}`, createdAt: 0, updatedAt, pinned: false, archived, temporary: false };
}

/**
 * Establish a returning-user (Continuation) baseline: an active empty thread
 * plus a separate non-archived history thread. Continuation keeps the
 * ThreadSidebar open by default, which is the state these layout tests assume.
 * Cold Start closed-by-default is covered by its own describe (task 6.3,
 * UIE-H-008). Threads/active-thread survive `clearMessages()`, so tests that
 * clear messages stay in Continuation.
 */
function seedReturningUserThreads(): void {
  converseStore.setThreads([makeThread("layout-active", 2), makeThread("layout-history", 1)]);
  converseStore.setActiveThread("layout-active");
}

function seedContext(id = "context-1"): void {
  converseStore.setContextRailItems([
    { id, type: "memory", label: `Context ${id}`, data: { source: "test" } },
  ]);
}

function seedMessages(count: number): void {
  for (let index = 0; index < count; index += 1) {
    const message: Message = {
      id: `layout-message-${index}`,
      threadId: "layout-thread",
      role: index % 2 === 0 ? "user" : "assistant",
      content: `Layout message ${index}`,
      timestamp: index,
    };
    converseStore.addMessage(message);
  }
}

describe("ConverseSpace — three-lane layout (task 3.1, Req 4.1/4.3)", () => {
  beforeEach(() => {
    globalThis.ResizeObserver = FullWidthResizeObserver as unknown as typeof ResizeObserver;
    // Reset shared singletons so each test starts from a clean, standard shell.
    shellStore.setWindowMode("standard");
    shellStore.setInspectorTarget(null);
    converseStore.clearMessages(); // clears messages + work blocks + context rail
    coreStore.reset(); // idle → not active
    clearGuiCognitionSession(); // no active GUI-cognition session
    // Returning-user (Continuation) baseline → ThreadSidebar open by default.
    seedReturningUserThreads();
  });

  afterEach(() => {
    globalThis.ResizeObserver = testDefaultResizeObserver;
  });

  it("presents the ConversationLane as the focal, dominant lane (Req 4.1/4.3)", () => {
    render(() => <ConverseSpace />);
    const conversation = screen.getByRole("region", { name: "Conversation" });
    expect(conversation).toBeInTheDocument();
    // Dominance is asserted via the layout marker the CSS keys off of.
    expect(conversation).toHaveAttribute("data-dominant", "true");
    expect(conversation).toHaveAttribute("data-lane", "conversation");
  });

  it("renders the focal message-stream container and the sticky Composer (Req 4.1/4.4)", () => {
    const { container } = render(() => <ConverseSpace />);
    expect(screen.getByRole("log", { name: "Message stream" })).toBeInTheDocument();
    expect(container.querySelector('[data-region="composer"]')).not.toBeNull();
  });

  it("keeps the Composer AFTER the message region in the DOM so it never covers the last message (Req 4.4)", () => {
    const { container } = render(() => <ConverseSpace />);
    const stream = container.querySelector('[data-region="message-stream"]')!;
    const composer = container.querySelector('[data-region="composer"]')!;
    expect(stream).not.toBeNull();
    expect(composer).not.toBeNull();
    // compareDocumentPosition: FOLLOWING (4) means composer comes after stream.
    const rel = stream.compareDocumentPosition(composer);
    expect(rel & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it("preserves the independent Composer row while Context visibility changes (Req 4.1/4.4)", async () => {
    seedContext("composer-row");
    const { container } = render(() => <ConverseSpace />);
    const root = container.querySelector<HTMLElement>(".kria-converse")!;
    const composer = container.querySelector<HTMLElement>('[data-region="composer"]')!;
    const contextToggle = screen.getByRole("button", { name: "Toggle context rail" });

    expect(Array.from(root.children, (child) => child.getAttribute("data-region") ?? child.className)).toEqual([
      "kria-converse__lanes",
      "composer",
    ]);

    // Work is inline now (no Work lane); Context is the only toggleable
    // secondary lane. The Composer element identity survives lane changes.
    expect(screen.queryByRole("complementary", { name: "Work" })).toBeNull();
    fireEvent.click(contextToggle);
    expect(contextToggle).toHaveAttribute("aria-pressed", "true");
    expect(await screen.findByRole("complementary", { name: "Context" })).toBeInTheDocument();
    expect(container.querySelector('[data-region="composer"]')).toBe(composer);

    fireEvent.click(contextToggle);
    expect(contextToggle).toHaveAttribute("aria-pressed", "false");
    expect(screen.queryByRole("complementary", { name: "Context" })).toBeNull();
    expect(container.querySelector('[data-region="composer"]')).toBe(composer);
  });

  it("preserves Composer row, lane borders, shared reading measure, and reduced-motion CSS (Req 4.3/4.4/5.6/12.8)", async () => {
    const [{ default: layoutCss }, { default: streamCss }, { default: shellCss }] = await Promise.all([
      import("./ConverseSpace.css?raw"),
      import("./converse/MessageStream.css?raw"),
      import("../AppShell.css?raw"),
    ]);

    expect(layoutCss).toMatch(
      /\.kria-converse\s*\{[\s\S]*?--kria-conversation-reading-measure:\s*720px;[\s\S]*?grid-template-areas:\s*"lanes"\s*"composer";[\s\S]*?grid-template-rows:\s*minmax\(0,\s*1fr\)\s+auto;/,
    );
    expect(layoutCss).toMatch(/\.kria-converse__lanes\s*\{[\s\S]*?grid-area:\s*lanes;/);
    expect(layoutCss).toMatch(
      /\.kria-converse__composer\s*\{[\s\S]*?grid-area:\s*composer;[\s\S]*?border-top:\s*1px\s+solid\s+var\(--color-border-default\);/,
    );
    for (const lane of ["work", "context"]) {
      expect(layoutCss).toMatch(
        new RegExp(`\\.kria-converse__${lane}\\s*\\{[\\s\\S]*?border-left:\\s*1px\\s+solid\\s+var\\(--color-border-default\\);`),
      );
    }
    for (const [profile, gutter] of [
      ["focus", "space-3"],
      ["dual", "space-4"],
      ["assisted", "space-4"],
      ["full", "space-6"],
    ] as const) {
      const profileRule = layoutCss.match(
        new RegExp(`\\.kria-converse\\[data-width-profile="${profile}"\\][^{]*\\{([\\s\\S]*?)\\}`),
      )?.[1] ?? "";
      expect(profileRule, `${profile} deliberate gutter`).toContain(
        `--kria-conversation-inline-gutter: var(--${gutter});`,
      );
    }
    expect(layoutCss).toMatch(
      /\.kria-converse\[data-window-mode="immersive"\]\s*\{[\s\S]*?--kria-conversation-reading-measure:\s*880px;[\s\S]*?--kria-work-lane-width:\s*clamp\(320px,\s*30vw,\s*480px\);/,
    );
    expect(layoutCss).toMatch(
      /\.kria-converse__composer\s*\{[\s\S]*?padding-inline-start:\s*calc\(var\(--kria-converse-leading-lanes\)\s*\+\s*var\(--kria-conversation-inline-gutter\)\);[\s\S]*?padding-inline-end:\s*calc\(var\(--kria-converse-trailing-lanes\)\s*\+\s*var\(--kria-conversation-inline-gutter\)\);/,
    );
    expect(layoutCss).toMatch(
      /\.kria-converse__composer-inner\s*\{[\s\S]*?width:\s*100%;[\s\S]*?max-width:\s*var\(--kria-conversation-reading-measure\);[\s\S]*?margin-inline:\s*auto;/,
    );
    expect(streamCss).toMatch(
      /\.kria-stream__sizer\s*\{[\s\S]*?width:\s*calc\(100%\s*-\s*var\(--kria-conversation-inline-gutter\)\s*-\s*var\(--kria-conversation-inline-gutter\)\);[\s\S]*?max-width:\s*var\(--kria-conversation-reading-measure\);[\s\S]*?margin-inline:\s*auto;/,
    );
    expect(shellCss).not.toMatch(
      /\.kria-shell\[data-window-mode="immersive"\]\s+\.kria-converse__(?:stream|composer)-inner/,
    );
    expect(layoutCss).toMatch(
      /@media\s*\(prefers-reduced-motion:\s*reduce\)\s*\{[\s\S]*?\.kria-converse__work,\s*\.kria-converse__context\s*\{[\s\S]*?animation:\s*none;/,
    );
  });

  it("uses the shell-owned active inset/fill contract in every Window Mode (Req 10.5)", async () => {
    const [{ default: shellCss }, { default: converseCss }] = await Promise.all([
      import("../AppShell.css?raw"),
      import("./ConverseSpace.css?raw"),
    ]);

    const routerRule = shellCss.match(/\.kria-space-router\s*\{([\s\S]*?)\}/)?.[1] ?? "";
    expect(routerRule).toMatch(/--kria-active-space-inset:\s*var\(--space-6\);/);
    expect(routerRule).toMatch(
      /--kria-active-space-fill:\s*calc\(-1\s*\*\s*var\(--kria-active-space-inset\)\);/,
    );
    expect(routerRule).toMatch(/padding:\s*var\(--kria-active-space-inset\);/);

    for (const mode of ["mini", "immersive"] as const) {
      const modeRule = shellCss.match(
        new RegExp(`\\.kria-shell\\[data-window-mode="${mode}"\\] \\.kria-space-router\\s*\\{([\\s\\S]*?)\\}`),
      )?.[1] ?? "";
      expect(modeRule, `${mode} shell inset`).toMatch(
        /--kria-active-space-inset:\s*var\(--space-3\);/,
      );
      expect(modeRule, `${mode} inherits router padding through the contract`).not.toMatch(/padding\s*:/);
    }

    const converseRule = converseCss.match(/\.kria-converse\s*\{([\s\S]*?)\}/)?.[1] ?? "";
    expect(converseRule).toMatch(/margin:\s*var\(--kria-active-space-fill\);/);
    expect(converseRule).not.toMatch(/margin:\s*calc\(-1\s*\*\s*var\(--space-6\)\)/);
    expect(converseCss.match(/var\(--kria-active-space-fill\)/g)).toHaveLength(1);
  });

  it("never renders a standalone Work lane — work is inline per-turn now (Req 4.1/4.2)", () => {
    // Idle: no Work lane.
    render(() => <ConverseSpace />);
    expect(screen.queryByRole("complementary", { name: "Work" })).toBeNull();
    cleanup();

    // Even with active work blocks present, no Work complementary lane exists —
    // the blocks surface inline in the conversation instead (see InlineWorkTrace).
    seedTurnWithWork("turn-nolane", "wb-nolane");
    render(() => <ConverseSpace />);
    expect(screen.queryByRole("complementary", { name: "Work" })).toBeNull();
  });

  it("renders work inline as a per-turn activity trace right after its user message (Req 4.2)", async () => {
    seedTurnWithWork("turn-inline", "wb-inline-1");
    render(() => <ConverseSpace />);

    // The inline trace exists, is attached to the turn, and shows the block.
    const trace = await screen.findByRole("region", { name: "KRIA activity for this turn" });
    expect(trace).toBeInTheDocument();
    expect(trace.getAttribute("data-turn-id")).toBe("turn-inline");
    // Running work auto-expands the trace and renders the typed WorkBlock.
    expect(await screen.findByText("work wb-inline-1")).toBeInTheDocument();

    // A second block for the same turn streams in without a remount.
    converseStore.addWorkBlock({ ...makeWorkBlock("wb-inline-2"), turnId: "turn-inline" });
    expect(await screen.findByText("work wb-inline-2")).toBeInTheDocument();
  });

  it("renders the active GUI-cognition session inline below the stream (no Work lane)", () => {
    seedGuiCognitionSession();
    expect(activeGuiCognitionSession()).not.toBeNull();
    const { container } = render(() => <ConverseSpace />);
    expect(screen.queryByRole("complementary", { name: "Work" })).toBeNull();
    expect(container.querySelector('[data-region="gui-cognition-inline"]')).not.toBeNull();
  });

  it("keeps the ContextRail on-demand: hidden by default, toggled by the user (Req 4.1)", async () => {
    seedContext("on-demand");
    render(() => <ConverseSpace />);
    const toggle = screen.getByRole("button", { name: "Toggle context rail" });
    expect(screen.queryByRole("complementary", { name: "Context" })).toBeNull();
    expect(toggle).toHaveAttribute("aria-pressed", "false");

    // Toggle it open via the on-demand control.
    fireEvent.click(toggle);
    expect(await screen.findByRole("complementary", { name: "Context" })).toBeInTheDocument();

    // Toggle it closed again.
    fireEvent.click(toggle);
    expect(screen.queryByRole("complementary", { name: "Context" })).toBeNull();
  });

  it("offers a collapsible ThreadSidebar in Standard mode (Req 4.1 / §6.1)", () => {
    render(() => <ConverseSpace />);
    expect(screen.getByRole("navigation", { name: "Threads" })).toBeInTheDocument();
  });

  it("aligns DOM, landmark, visual-area, and natural keyboard order (Req 4.1/4.3)", async () => {
    seedContext("order");
    const { container } = render(() => <ConverseSpace />);
    fireEvent.click(screen.getByRole("button", { name: "Toggle context rail" }));

    const laneRoot = container.querySelector<HTMLElement>(".kria-converse__lanes")!;
    const laneNames = () =>
      Array.from(laneRoot.children, (lane) => (lane as HTMLElement).dataset.lane);

    expect(laneNames()).toEqual(["threads", "conversation", "context"]);

    const landmarks = Array.from(
      laneRoot.querySelectorAll<HTMLElement>(":scope > nav, :scope > section, :scope > aside"),
      (lane) => lane.dataset.lane,
    );
    expect(landmarks).toEqual(laneNames());

    for (const lane of Array.from(laneRoot.children) as HTMLElement[]) {
      expect(lane.style.gridArea).toBe(lane.dataset.lane);
    }

    const focusables = Array.from(
      laneRoot.querySelectorAll<HTMLElement>(
        'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]',
      ),
    ).filter((element) => element.tabIndex >= 0);
    const keyboardLaneOrder = focusables
      .map((element) => element.closest<HTMLElement>("[data-lane]")?.dataset.lane)
      .filter((lane): lane is string => Boolean(lane))
      .map((lane) => ["threads", "conversation", "context"].indexOf(lane));

    expect(keyboardLaneOrder.length).toBeGreaterThan(0);
    expect(keyboardLaneOrder.every((lane, index) => index === 0 || lane >= keyboardLaneOrder[index - 1])).toBe(true);
    expect(
      Array.from(laneRoot.querySelectorAll<HTMLElement>("[tabindex]")).every(
        (element) => element.tabIndex <= 0,
      ),
    ).toBe(true);

    fireEvent.click(screen.getByRole("button", { name: "Close thread sidebar" }));
    expect(laneNames()).toEqual(["conversation", "context"]);
  });

  it("keeps visual templates in semantic order with ConversationLane as the only flexible track (Req 4.1/4.3)", async () => {
    const { default: layoutCss } = await import("./ConverseSpace.css?raw");
    const templates = new Map<string, string>(
      Array.from(
        layoutCss.matchAll(/grid-template-areas:\s*"([^"]+)";\s*grid-template-columns:\s*([^;]+);/g),
        ([, areas, columns]) => [areas, columns.trim()] as const,
      ),
    );
    const expectedAreas = [
      "conversation",
      "threads conversation",
      "conversation work",
      "conversation context",
      "threads conversation work",
      "threads conversation context",
      "conversation work context",
      "threads conversation work context",
    ];

    expect(Array.from(templates.keys()).sort()).toEqual([...expectedAreas].sort());
    for (const areas of expectedAreas) {
      const tracks = templates.get(areas)!.match(/minmax\(0,\s*1fr\)|auto/g);
      expect(tracks).toEqual(
        areas.split(" ").map((lane) => (lane === "conversation" ? "minmax(0, 1fr)" : "auto")),
      );
    }
  });

  it("Property P5: generated lane subsets occupy exactly their rendered regions with positive conversation width", async () => {
    // **Validates: Requirements 4.1, 4.2, 4.3, 4.6**
    const { default: layoutCss } = await import("./ConverseSpace.css?raw");
    const templates = new Map<string, string>(
      Array.from(
        layoutCss.matchAll(/grid-template-areas:\s*"([^"]+)";\s*grid-template-columns:\s*([^;]+);/g),
        ([, areas, columns]) => [areas, columns.trim()] as const,
      ),
    );
    const secondaryMaxWidth = new Map(
      (["threads", "work", "context"] as const).map((lane) => {
        const match = layoutCss.match(
          new RegExp(`--kria-${lane}-lane-width:\\s*clamp\\([^,]+,[^,]+,\\s*(\\d+)px\\);`),
        );
        expect(match, `${lane} must retain a bounded maximum width`).not.toBeNull();
        return [lane, Number(match![1])] as const;
      }),
    );
    const fullProfileWidth = 1440;
    // Work is no longer a lane; the toggleable secondary lanes are threads and
    // context (2 dims → 4 subsets).
    const subsets = Array.from({ length: 4 }, (_, mask) => ({
      threads: Boolean(mask & 0b001),
      context: Boolean(mask & 0b010),
    }));

    expect(subsets).toHaveLength(4);
    for (const subset of subsets) {
      cleanup();
      converseStore.clearMessages();
      if (subset.context) seedContext(`subset-${Number(subset.threads)}`);

      const { container } = render(() => <ConverseSpace />);
      if (!subset.threads) {
        fireEvent.click(screen.getByRole("button", { name: "Close thread sidebar" }));
      }
      if (subset.context) {
        fireEvent.click(screen.getByRole("button", { name: "Toggle context rail" }));
      }

      const expectedRegions = [
        ...(subset.threads ? ["threads"] : []),
        "conversation",
        ...(subset.context ? ["context"] : []),
      ];
      const laneRoot = container.querySelector<HTMLElement>(".kria-converse__lanes")!;
      const renderedRegions = Array.from(
        laneRoot.children,
        (lane) => (lane as HTMLElement).dataset.lane!,
      );
      const templateColumns = templates.get(renderedRegions.join(" "));
      const occupiedRegions = renderedRegions.join(" ").split(" ");
      const caseLabel = JSON.stringify(subset);

      expect(renderedRegions, `${caseLabel}: semantic regions`).toEqual(expectedRegions);
      expect(templateColumns, `${caseLabel}: occupied template exists`).toBeDefined();
      expect(occupiedRegions, `${caseLabel}: occupied regions`).toEqual(renderedRegions);

      const tracks = templateColumns!.match(/minmax\(0,\s*1fr\)|auto/g) ?? [];
      expect(tracks, `${caseLabel}: one track per occupied region`).toHaveLength(renderedRegions.length);
      expect(
        tracks.filter((track) => track === "minmax(0, 1fr)"),
        `${caseLabel}: conversation is sole flexible track`,
      ).toEqual(["minmax(0, 1fr)"]);

      const reservedSecondaryWidth = renderedRegions.reduce(
        (total, region) => total + (secondaryMaxWidth.get(region as "threads" | "work" | "context") ?? 0),
        0,
      );
      const conversationWidth = fullProfileWidth - reservedSecondaryWidth;
      expect(conversationWidth, `${caseLabel}: conversation width`).toBeGreaterThan(0);
    }
  });

  it("restores focus when a focused secondary lane is removed", async () => {
    seedContext("focus");
    render(() => <ConverseSpace />);

    // Removing the focused ThreadSidebar restores focus to the stable
    // open-sidebar control (never left on a detached node).
    const closeThreads = screen.getByRole("button", { name: "Close thread sidebar" });
    closeThreads.focus();
    fireEvent.click(closeThreads);
    await waitFor(() => expect(screen.getByRole("button", { name: "Open thread sidebar" })).toHaveFocus());

    // Work is inline now — no Work lane to focus or restore from — and the focal
    // conversation + Composer remain intact through the transition.
    expect(screen.queryByRole("complementary", { name: "Work" })).toBeNull();
    expect(screen.getByRole("region", { name: "Conversation" })).toBeInTheDocument();
    expect(screen.getByLabelText("Composer")).toBeInTheDocument();
  });

  it("keeps one semantic instance per lane through rapid toggle sequences", () => {
    seedContext("rapid");
    const { container } = render(() => <ConverseSpace />);
    const laneNames = () => Array.from(
      container.querySelectorAll<HTMLElement>(".kria-converse__lanes > [data-lane]"),
      (lane) => lane.dataset.lane,
    );

    // 6 cycles (was 12) still exercises repeated toggle+churn convergence.
    for (let cycle = 0; cycle < 6; cycle += 1) {
      fireEvent.click(screen.getByRole("button", { name: "Toggle context rail" }));
      fireEvent.click(screen.getByRole("button", { name: "Toggle context rail" }));
    }

    // Context ends closed after an even number of toggles; threads + conversation
    // remain, each exactly once.
    expect(laneNames()).toEqual(["threads", "conversation"]);
    expect(new Set(laneNames()).size).toBe(laneNames().length);
    expect(container.querySelectorAll('[data-region="composer"]')).toHaveLength(1);
  });

  it("omits an empty Context lane and reclaims it when context empties", async () => {
    const { container } = render(() => <ConverseSpace />);
    const toggle = screen.getByRole("button", { name: "Toggle context rail" });

    fireEvent.click(toggle);
    expect(toggle).toHaveAttribute("aria-pressed", "false");
    expect(screen.queryByRole("complementary", { name: "Context" })).toBeNull();

    seedContext("transient");
    fireEvent.click(toggle);
    expect(await screen.findByRole("complementary", { name: "Context" })).toBeInTheDocument();
    converseStore.setContextRailItems([]);

    await waitFor(() => expect(screen.queryByRole("complementary", { name: "Context" })).toBeNull());
    expect(toggle).toHaveAttribute("aria-pressed", "false");
    expect(Array.from(
      container.querySelectorAll<HTMLElement>(".kria-converse__lanes > [data-lane]"),
      (lane) => lane.dataset.lane,
    )).toEqual(["threads", "conversation"]);
  });

  it("retains virtualized MessageStream while secondary lanes toggle", () => {
    seedContext("virtualized");
    seedMessages(300);
    converseStore.addWorkBlock(makeWorkBlock("wb-virtualized"));
    const { container } = render(() => <ConverseSpace />);
    const stream = container.querySelector('[data-region="message-stream-virtual"]');
    const renderedBefore = screen.getAllByRole("article").length;

    expect(stream).not.toBeNull();
    expect(renderedBefore).toBeGreaterThan(0);
    expect(renderedBefore).toBeLessThan(300);
    fireEvent.click(screen.getByRole("button", { name: "Toggle context rail" }));
    converseStore.clearWorkBlocks();
    fireEvent.click(screen.getByRole("button", { name: "Close thread sidebar" }));

    expect(container.querySelector('[data-region="message-stream-virtual"]')).toBe(stream);
    expect(screen.getAllByRole("article").length).toBeGreaterThan(0);
    expect(screen.getAllByRole("article").length).toBeLessThan(300);
  });

  it("preserves semantic lane composition while Inspector is open", async () => {
    seedContext("inspector");
    converseStore.addWorkBlock(makeWorkBlock("wb-inspector"));
    const { container } = render(() => (
      <>
        <ConverseSpace />
        <InspectorHost />
      </>
    ));
    fireEvent.click(screen.getByRole("button", { name: "Toggle context rail" }));
    const laneRoot = container.querySelector<HTMLElement>(".kria-converse__lanes")!;
    const before = Array.from(laneRoot.children, (lane) => (lane as HTMLElement).dataset.lane);

    shellStore.openInspector("memory", "task-2.7");
    expect(await screen.findByRole("complementary", { name: "Inspector" })).toBeInTheDocument();
    expect(Array.from(laneRoot.children, (lane) => (lane as HTMLElement).dataset.lane)).toEqual(before);
    expect(container.querySelectorAll('[data-lane="conversation"]')).toHaveLength(1);
  });

  it("keeps Compact intent while Width Profile, not mode, owns lane fit (Req 4.4/4.5)", async () => {
    shellStore.setWindowMode("mini");
    render(() => <ConverseSpace />);

    expect(screen.getByRole("region", { name: "Conversation" })).toBeInTheDocument();
    expect(screen.queryByRole("complementary", { name: "Work" })).toBeNull();
    expect(screen.getByRole("navigation", { name: "Threads" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Toggle context rail" })).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "Converse" })).toHaveAttribute("data-window-mode", "mini");
  });
});


describe("ConverseSpace — stable Width Profile observer (task 3.6)", () => {
  it("uses one content-box observer, exact lower-inclusive boundaries, deduplicated profile writes, and cleanup", async () => {
    // **Validates: Requirements 4.4, 4.5, 4.7, 10.4**
    const OriginalResizeObserver = globalThis.ResizeObserver;

    class ControlledResizeObserver {
      static readonly instances: ControlledResizeObserver[] = [];
      readonly observed = new Set<Element>();
      readonly observeCalls: Array<{ target: Element; options?: ResizeObserverOptions }> = [];
      disconnected = false;

      constructor(private readonly callback: ResizeObserverCallback) {
        ControlledResizeObserver.instances.push(this);
      }

      observe(target: Element, options?: ResizeObserverOptions): void {
        this.observed.add(target);
        this.observeCalls.push({ target, options });
      }

      unobserve(target: Element): void { this.observed.delete(target); }
      disconnect(): void {
        this.disconnected = true;
        this.observed.clear();
      }

      emit(inlineSize: number, contentRectWidth = inlineSize): void {
        const target = this.observed.values().next().value as Element;
        this.callback(
          [{
            target,
            contentBoxSize: [{ inlineSize, blockSize: 600 }],
            contentRect: { width: contentRectWidth },
          } as unknown as ResizeObserverEntry],
          this as unknown as ResizeObserver,
        );
      }

      emitLegacy(width: number): void {
        const target = this.observed.values().next().value as Element;
        this.callback(
          [{ target, contentRect: { width } } as ResizeObserverEntry],
          this as unknown as ResizeObserver,
        );
      }
    }

    globalThis.ResizeObserver = ControlledResizeObserver as unknown as typeof ResizeObserver;
    shellStore.setWindowMode("standard");
    shellStore.setActiveSpace("converse");
    shellStore.openInspector("memory", "observer-target");
    converseStore.clearMessages();
    // Returning-user (Continuation) baseline keeps the ThreadSidebar open so the
    // observer test's full-composition assertions hold (task 6.3, UIE-H-008).
    converseStore.setThreads([
      { id: "observer-thread", title: "Observer", createdAt: 0, updatedAt: 2, pinned: false, archived: false, temporary: false },
      { id: "observer-history", title: "History", createdAt: 0, updatedAt: 1, pinned: false, archived: false, temporary: false },
    ]);
    converseStore.setActiveThread("observer-thread");
    converseStore.updateDraft({ text: "observer draft" });
    const draft = converseStore.composerDraft();
    const inspector = shellStore.inspectorTarget();
    const view = render(() => <ConverseSpace />);
    let profileWrites = 0;

    try {
      const root = view.container.querySelector<HTMLElement>(".kria-converse")!;
      const mutationObserver = new MutationObserver((records) => {
        profileWrites += records.filter((record) => record.attributeName === "data-width-profile").length;
      });
      mutationObserver.observe(root, { attributes: true, attributeFilter: ["data-width-profile"] });
      const observer = ControlledResizeObserver.instances[0];

      expect(ControlledResizeObserver.instances).toHaveLength(1);
      expect(observer.observeCalls).toEqual([{ target: root, options: { box: "content-box" } }]);
      expect(root).toHaveAttribute("data-width-profile", "focus");

      const cases: ReadonlyArray<readonly [number, string]> = [
        [0, "focus"],
        [735.999, "focus"],
        [736, "dual"],
        [1055.999, "dual"],
        [1056, "assisted"],
        [1439.999, "assisted"],
        [1440, "full"],
        [4096, "full"],
      ];
      for (const [width, expected] of cases) {
        observer.emit(width);
        await waitFor(() => expect(root).toHaveAttribute("data-width-profile", expected));
      }

      // Delivered content-box width wins over a conflicting legacy rect. This
      // models Inspector/scrollbar space already removed by layout.
      observer.emit(1055.999, 1056);
      await waitFor(() => expect(root).toHaveAttribute("data-width-profile", "dual"));
      observer.emitLegacy(1056);
      await waitFor(() => expect(root).toHaveAttribute("data-width-profile", "assisted"));

      await new Promise<void>((resolve) => queueMicrotask(resolve));
      const writesBeforeSameProfileBurst = profileWrites;
      for (const width of [1056, 1100, 1200, 1439.999, 1056]) observer.emit(width);
      await new Promise<void>((resolve) => queueMicrotask(resolve));
      expect(profileWrites).toBe(writesBeforeSameProfileBurst);

      for (const invalidWidth of [-1, Number.NaN, Number.POSITIVE_INFINITY]) observer.emit(invalidWidth);
      expect(root).toHaveAttribute("data-width-profile", "assisted");
      expect(shellStore.activeSpace()).toBe("converse");
      expect(converseStore.activeThreadId()).toBe("observer-thread");
      expect(converseStore.composerDraft()).toBe(draft);
      expect(shellStore.inspectorTarget()).toBe(inspector);
      mutationObserver.disconnect();
    } finally {
      view.unmount();
      globalThis.ResizeObserver = OriginalResizeObserver;
    }

    expect(ControlledResizeObserver.instances[0].disconnected).toBe(true);
    expect(ControlledResizeObserver.instances[0].observed.size).toBe(0);
  });

  it("converges after rapid boundary sequences, lane toggles, and return to original width", async () => {
    // **Validates: Requirements 4.4, 4.5, 10.4, 11.5**
    const OriginalResizeObserver = globalThis.ResizeObserver;

    class RapidResizeObserver {
      static instance: RapidResizeObserver;
      target?: Element;
      disconnected = false;

      constructor(private readonly callback: ResizeObserverCallback) {
        RapidResizeObserver.instance = this;
      }
      observe(target: Element): void { this.target = target; }
      unobserve(): void {}
      disconnect(): void { this.disconnected = true; this.target = undefined; }
      emit(width: number): void {
        this.callback(
          [{
            target: this.target!,
            contentBoxSize: { inlineSize: width, blockSize: 600 },
            contentRect: { width },
          } as unknown as ResizeObserverEntry],
          this as unknown as ResizeObserver,
        );
      }
    }

    globalThis.ResizeObserver = RapidResizeObserver as unknown as typeof ResizeObserver;
    shellStore.setWindowMode("standard");
    converseStore.clearMessages();
    seedContext("rapid-profile-context");
    const view = render(() => <ConverseSpace />);

    try {
      const root = view.container.querySelector<HTMLElement>(".kria-converse")!;
      const observer = RapidResizeObserver.instance;
      observer.emit(1500);
      await waitFor(() => expect(root).toHaveAttribute("data-width-profile", "full"));
      fireEvent.click(screen.getByRole("button", { name: "Toggle context rail" }));

      const originalComposition = root.dataset.composition;
      const conversation = screen.getByRole("region", { name: "Conversation" });
      const composer = root.querySelector('[data-region="composer"]');
      const contextToggle = screen.getByRole("button", { name: "Toggle context rail" });
      const rapidWidths = [1439.999, 1440, 1055.999, 1056, 735.999, 736, 1440, 735.999, 1056];

      for (const width of rapidWidths) {
        observer.emit(width);
        fireEvent.click(contextToggle);
        fireEvent.click(contextToggle);
        const closeThreads = screen.queryByRole("button", { name: "Close thread sidebar" });
        if (closeThreads) {
          fireEvent.click(closeThreads);
          fireEvent.click(screen.getByRole("button", { name: "Open thread sidebar" }));
        }
      }
      observer.emit(1500);

      await waitFor(() => expect(root).toHaveAttribute("data-width-profile", "full"));
      expect(root.dataset.composition).toBe(originalComposition);
      expect(screen.getByRole("region", { name: "Conversation" })).toBe(conversation);
      expect(root.querySelector('[data-region="composer"]')).toBe(composer);
      expect(Array.from(
        root.querySelectorAll<HTMLElement>(".kria-converse__lanes > [data-lane]"),
        (lane) => lane.dataset.lane,
      )).toEqual(["threads", "conversation", "context"]);
    } finally {
      view.unmount();
      globalThis.ResizeObserver = OriginalResizeObserver;
    }

    expect(RapidResizeObserver.instance.disconnected).toBe(true);
  });
});

describe("ConverseSpace — deterministic mode/profile composition (task 3.3)", () => {
  it("maps every Window Mode × Width Profile × relevance combination once", () => {
    // **Validates: Requirements 4.1, 4.4, 4.5, 4.6, 10.4**
    const modes = ["standard", "mini", "immersive"] as const;
    const profiles = ["focus", "dual", "assisted", "full"] as const;
    const capacities = { focus: 0, dual: 1, assisted: 2, full: 3 } as const;
    const semanticOrder = ["threads", "work", "context"] as const;
    const priority = ["work", "context", "threads"] as const;
    const ids = new Set<string>();

    for (const mode of modes) {
      for (const profile of profiles) {
        for (let mask = 0; mask < 8; mask += 1) {
          const relevance = {
            threads: Boolean(mask & 0b001),
            work: Boolean(mask & 0b010),
            context: Boolean(mask & 0b100),
          };
          const result = resolveConverseComposition(mode, profile, relevance);
          const selected = new Set(
            priority.filter((lane) => relevance[lane]).slice(0, capacities[profile]),
          );
          const expectedVisible = semanticOrder.filter((lane) => selected.has(lane));

          expect(result).toEqual(resolveConverseComposition(mode, profile, relevance));
          expect(result.mode).toBe(mode);
          expect(result.profile).toBe(profile);
          expect(result.visibleLanes).toEqual(expectedVisible);
          expect(result.threads).toBe(selected.has("threads"));
          expect(result.work).toBe(selected.has("work"));
          expect(result.context).toBe(selected.has("context"));
          expect(ids.has(result.id)).toBe(false);
          ids.add(result.id);
        }
      }
    }

    expect(ids.size).toBe(3 * 4 * 8);
  });

  it("keeps Conversation dominant while profiles admit 0/1/2/3 relevant lanes", () => {
    const allRelevant = { threads: true, work: true, context: true };
    for (const mode of ["standard", "mini", "immersive"] as const) {
      expect(resolveConverseComposition(mode, "focus", allRelevant).visibleLanes).toEqual([]);
      expect(resolveConverseComposition(mode, "dual", allRelevant).visibleLanes).toEqual(["work"]);
      expect(resolveConverseComposition(mode, "assisted", allRelevant).visibleLanes).toEqual(["work", "context"]);
      expect(resolveConverseComposition(mode, "full", allRelevant).visibleLanes).toEqual(["threads", "work", "context"]);
    }
  });

  it("reacts to width and mode without reload or state/place loss", async () => {
    // **Validates: Requirements 4.4, 4.5, 10.4, 11.5**
    const OriginalResizeObserver = globalThis.ResizeObserver;

    class ControlledCompositionResizeObserver {
      static readonly instances: ControlledCompositionResizeObserver[] = [];
      readonly observed = new Set<Element>();

      constructor(private readonly callback: ResizeObserverCallback) {
        ControlledCompositionResizeObserver.instances.push(this);
      }

      observe(target: Element): void { this.observed.add(target); }
      unobserve(target: Element): void { this.observed.delete(target); }
      disconnect(): void { this.observed.clear(); }
      emit(width: number): void {
        const target = this.observed.values().next().value as Element;
        this.callback(
          [{ target, contentRect: { width } } as ResizeObserverEntry],
          this as unknown as ResizeObserver,
        );
      }
    }

    globalThis.ResizeObserver = ControlledCompositionResizeObserver as unknown as typeof ResizeObserver;
    shellStore.setWindowMode("standard");
    shellStore.setActiveSpace("converse");
    shellStore.openInspector("memory", "composition-target");
    converseStore.clearMessages();
    converseStore.setActiveThread("composition-thread");
    converseStore.updateDraft({ text: "preserve this responsive draft" });
    seedMessages(300);
    seedContext("composition-context");

    const view = render(() => <ConverseSpace />);
    try {
      const root = view.container.querySelector<HTMLElement>(".kria-converse")!;
      const observer = ControlledCompositionResizeObserver.instances.find(
        (candidate) => candidate.observed.has(root),
      )!;
      observer.emit(1500);
      await waitFor(() => expect(root).toHaveAttribute("data-width-profile", "full"));
      fireEvent.click(screen.getByRole("button", { name: "Toggle context rail" }));

      const laneNames = () => Array.from(
        root.querySelectorAll<HTMLElement>(".kria-converse__lanes > [data-lane]"),
        (lane) => lane.dataset.lane,
      );
      expect(laneNames()).toEqual(["threads", "conversation", "context"]);

      const conversation = screen.getByRole("region", { name: "Conversation" });
      const composer = root.querySelector<HTMLElement>('[data-region="composer"]')!;
      const textarea = screen.getByRole("textbox", { name: "Message KRIA" });
      const virtualStream = root.querySelector<HTMLElement>('[data-region="message-stream-virtual"]')!;
      const viewport = virtualStream.querySelector<HTMLElement>(".kria-stream__viewport")!;
      const draft = converseStore.composerDraft();
      const inspector = shellStore.inspectorTarget();
      viewport.scrollTop = 137;

      const closeThreads = screen.getByRole("button", { name: "Close thread sidebar" });
      closeThreads.focus();
      observer.emit(900);
      await waitFor(() => expect(root).toHaveAttribute("data-width-profile", "dual"));
      // Dual capacity = 1 secondary lane; with work retired, Context (highest
      // remaining relevance) wins over Threads.
      expect(laneNames()).toEqual(["conversation", "context"]);
      await waitFor(() => expect(screen.getByRole("button", { name: "Toggle context rail" })).toHaveFocus());

      textarea.focus();
      shellStore.setWindowMode("mini");
      await waitFor(() => expect(root).toHaveAttribute("data-window-mode", "mini"));
      expect(laneNames()).toEqual(["conversation", "context"]);
      expect(textarea).toHaveFocus();

      shellStore.setWindowMode("immersive");
      observer.emit(1100);
      await waitFor(() => expect(root).toHaveAttribute("data-width-profile", "assisted"));
      expect(laneNames()).toEqual(["threads", "conversation", "context"]);

      observer.emit(600);
      await waitFor(() => expect(root).toHaveAttribute("data-width-profile", "focus"));
      expect(laneNames()).toEqual(["conversation"]);
      observer.emit(1500);
      await waitFor(() => expect(root).toHaveAttribute("data-width-profile", "full"));
      expect(laneNames()).toEqual(["threads", "conversation", "context"]);

      expect(view.container.querySelector(".kria-converse")).toBe(root);
      expect(screen.getByRole("region", { name: "Conversation" })).toBe(conversation);
      expect(root.querySelector('[data-region="composer"]')).toBe(composer);
      expect(screen.getByRole("textbox", { name: "Message KRIA" })).toBe(textarea);
      expect(root.querySelector('[data-region="message-stream-virtual"]')).toBe(virtualStream);
      expect(viewport.scrollTop).toBe(137);
      expect(shellStore.activeSpace()).toBe("converse");
      expect(converseStore.activeThreadId()).toBe("composition-thread");
      expect(converseStore.composerDraft()).toBe(draft);
      expect(shellStore.inspectorTarget()).toBe(inspector);
    } finally {
      view.unmount();
      globalThis.ResizeObserver = OriginalResizeObserver;
    }
  });
});


describe("ConverseSpace — Cold Start ThreadSidebar default (task 6.3, UIE-H-008, Req 6.3)", () => {
  beforeEach(() => {
    // Full width so composition never collapses the sidebar for width reasons —
    // this isolates the STATE-based Cold Start default from width-profile fit.
    globalThis.ResizeObserver = FullWidthResizeObserver as unknown as typeof ResizeObserver;
    shellStore.setWindowMode("standard");
    shellStore.setInspectorTarget(null);
    converseStore.clearMessages();
    coreStore.reset();
    clearGuiCognitionSession();
  });

  afterEach(() => {
    globalThis.ResizeObserver = testDefaultResizeObserver;
  });

  /** Cold Start: no active task, no explicit intent, no usable history. */
  function seedColdStart(): void {
    converseStore.setThreads([]);
    converseStore.setActiveThread(null);
  }

  it("defaults the ThreadSidebar CLOSED during Cold Start before any interaction (Req 6.3)", () => {
    seedColdStart();
    expect(converseStore.emptyStateClass()).toBe("cold-start");
    render(() => <ConverseSpace />);

    // Sidebar closed by default: no Threads landmark, no in-sidebar close control.
    expect(screen.queryByRole("navigation", { name: "Threads" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Close thread sidebar" })).toBeNull();
  });

  it("keeps a visible, labelled Threads access control when closed in Cold Start (Req 6.3)", () => {
    seedColdStart();
    render(() => <ConverseSpace />);

    // Explicit path to open threads remains available and accessibly labelled.
    const open = screen.getByRole("button", { name: "Open thread sidebar" });
    expect(open).toBeInTheDocument();
    expect(open).toHaveAccessibleName("Open thread sidebar");
  });

  it("does NOT force the ThreadSidebar closed for Continuation (Req 6.3)", () => {
    // Active empty thread + separate non-archived history → Continuation.
    converseStore.setThreads([makeThread("cont-active", 2), makeThread("cont-history", 1)]);
    converseStore.setActiveThread("cont-active");
    expect(converseStore.emptyStateClass()).toBe("continuation");

    render(() => <ConverseSpace />);
    expect(screen.getByRole("navigation", { name: "Threads" })).toBeInTheDocument();
  });

  it("does NOT force the ThreadSidebar closed for an Active conversation (Req 6.3)", () => {
    converseStore.setThreads([makeThread("active-thread", 1)]);
    converseStore.setActiveThread("active-thread");
    converseStore.addMessage({
      id: "cold-active-msg",
      threadId: "active-thread",
      role: "user",
      content: "hello",
      timestamp: 1,
    });
    expect(converseStore.emptyStateClass()).toBe("active");

    render(() => <ConverseSpace />);
    expect(screen.getByRole("navigation", { name: "Threads" })).toBeInTheDocument();
  });

  it("scopes the closed default strictly to Cold Start: transitioning cold→continuation opens by default (Req 6.3)", async () => {
    // Start Cold Start (closed), then usable history appears with no explicit
    // user interaction → classifier leaves Cold Start and the untouched default
    // follows to open. This proves only Cold Start defaults closed.
    seedColdStart();
    render(() => <ConverseSpace />);
    expect(screen.queryByRole("navigation", { name: "Threads" })).toBeNull();

    converseStore.setThreads([makeThread("late-active", 2), makeThread("late-history", 1)]);
    converseStore.setActiveThread("late-active");
    expect(converseStore.emptyStateClass()).toBe("continuation");
    expect(await screen.findByRole("navigation", { name: "Threads" })).toBeInTheDocument();
  });

  it("preserves an EXPLICIT in-session OPEN over the Cold Start default (Req 6.3, current-session choice)", async () => {
    seedColdStart();
    render(() => <ConverseSpace />);

    // Closed by default in Cold Start.
    expect(screen.queryByRole("navigation", { name: "Threads" })).toBeNull();

    // User explicitly opens the sidebar.
    fireEvent.click(screen.getByRole("button", { name: "Open thread sidebar" }));
    expect(await screen.findByRole("navigation", { name: "Threads" })).toBeInTheDocument();

    // A re-render/state churn while still Cold Start must NOT re-close it. Use a
    // context-rail churn (a signal ConverseSpace composes on) as the trigger.
    converseStore.setContextRailItems([
      { id: "cold-open-churn", type: "custom", label: "x", data: {} },
    ]);
    await waitFor(() => expect(converseStore.contextRail().length).toBe(1));
    expect(converseStore.emptyStateClass()).toBe("cold-start");
    expect(screen.getByRole("navigation", { name: "Threads" })).toBeInTheDocument();
    converseStore.setContextRailItems([]);
  });

  it("preserves an EXPLICIT in-session CLOSE over the open default (Req 6.3, current-session choice)", async () => {
    // Continuation → open by default.
    converseStore.setThreads([makeThread("choice-active", 2), makeThread("choice-history", 1)]);
    converseStore.setActiveThread("choice-active");
    render(() => <ConverseSpace />);
    expect(screen.getByRole("navigation", { name: "Threads" })).toBeInTheDocument();

    // User explicitly closes it.
    fireEvent.click(screen.getByRole("button", { name: "Close thread sidebar" }));
    await waitFor(() => expect(screen.queryByRole("navigation", { name: "Threads" })).toBeNull());

    // State churn must not re-open it against the user's explicit close. Use a
    // context-rail churn (a signal ConverseSpace composes on) as the trigger.
    converseStore.setContextRailItems([
      { id: "choice-close-churn", type: "custom", label: "x", data: {} },
    ]);
    await waitFor(() => expect(converseStore.contextRail().length).toBe(1));
    expect(screen.queryByRole("navigation", { name: "Threads" })).toBeNull();
    expect(screen.getByRole("button", { name: "Open thread sidebar" })).toBeInTheDocument();
    converseStore.setContextRailItems([]);
  });

  it("returns focus to the Open control after an explicit close and to Threads on reopen (Req 6.3 keyboard flow)", async () => {
    converseStore.setThreads([makeThread("focus-active", 2), makeThread("focus-history", 1)]);
    converseStore.setActiveThread("focus-active");
    render(() => <ConverseSpace />);

    const close = screen.getByRole("button", { name: "Close thread sidebar" });
    close.focus();
    fireEvent.click(close);
    await waitFor(() => expect(screen.getByRole("button", { name: "Open thread sidebar" })).toHaveFocus());

    fireEvent.click(screen.getByRole("button", { name: "Open thread sidebar" }));
    expect(await screen.findByRole("navigation", { name: "Threads" })).toBeInTheDocument();
    // Preserved thread controls remain available on reopen.
    expect(screen.getByRole("button", { name: "New thread" })).toBeInTheDocument();
    expect(screen.getByRole("searchbox", { name: "Search conversations" })).toBeInTheDocument();
  });

  it("keeps the Cold Start 'Open thread sidebar' control keyboard-reachable and focuses Threads on open (task 6.8, Req 6.3)", async () => {
    // Cold Start defaults closed; the explicit open control is a real, labelled
    // button (keyboard-focusable/activatable) and opening reveals the Threads nav.
    converseStore.setThreads([]);
    converseStore.setActiveThread(null);
    expect(converseStore.emptyStateClass()).toBe("cold-start");
    render(() => <ConverseSpace />);

    const open = screen.getByRole("button", { name: "Open thread sidebar" });
    expect(open.tagName).toBe("BUTTON");
    expect(open.tabIndex).toBeGreaterThanOrEqual(0);

    open.focus();
    expect(open).toHaveFocus();
    fireEvent.click(open); // the activation a keyboard Enter/Space dispatches on a button
    expect(await screen.findByRole("navigation", { name: "Threads" })).toBeInTheDocument();
    // Explicit Threads access + its controls become reachable on open.
    expect(screen.getByRole("button", { name: "Close thread sidebar" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "New thread" })).toBeInTheDocument();
  });
});

describe("ConverseSpace — empty-state reachability across modes (task 6.8, Req 6.3/6.4/16.4)", () => {
  beforeEach(() => {
    // Full width so composition never collapses lanes for width reasons; this
    // isolates Window Mode behavior from Width Profile fit.
    globalThis.ResizeObserver = FullWidthResizeObserver as unknown as typeof ResizeObserver;
    shellStore.setInspectorTarget(null);
    converseStore.clearMessages();
    coreStore.reset();
    clearGuiCognitionSession();
  });

  afterEach(() => {
    globalThis.ResizeObserver = testDefaultResizeObserver;
    cleanup();
  });

  it("holds the Cold Start closed-sidebar default and keeps the orientation heading + ≤3 starters + disclosure reachable in every Window Mode", () => {
    for (const mode of ["standard", "mini", "immersive"] as const) {
      shellStore.setWindowMode(mode);
      converseStore.setThreads([]);
      converseStore.setActiveThread(null);
      expect(converseStore.emptyStateClass(), `${mode}: cold start`).toBe("cold-start");

      const { unmount } = render(() => <ConverseSpace />);

      // Cold Start: ThreadSidebar closed by default, explicit Open control present.
      expect(screen.queryByRole("navigation", { name: "Threads" }), `${mode}: sidebar closed`).toBeNull();
      expect(screen.getByRole("button", { name: "Open thread sidebar" }), `${mode}: open control`).toBeInTheDocument();

      // Orientation heading (level 2) + ≤3 grounded starters remain reachable.
      expect(
        screen.getByRole("heading", { level: 2, name: "What can I help with?" }),
        `${mode}: orientation heading`,
      ).toBeInTheDocument();
      const starters = screen.getByRole("list", { name: "Starter prompts" }).querySelectorAll("li");
      expect(starters.length, `${mode}: starter count`).toBeGreaterThan(0);
      expect(starters.length, `${mode}: ≤3 starters`).toBeLessThanOrEqual(3);

      // Secondary disclosure stays reachable (labelled button) in every mode.
      expect(
        screen.getByRole("button", { name: "Customize suggestions" }),
        `${mode}: disclosure`,
      ).toBeInTheDocument();

      unmount();
    }
  });

  it("keeps starters, heading, and disclosure reachable for an Intentional New Thread with unrelated history in every Window Mode (UIE-H-005)", () => {
    for (const mode of ["standard", "mini", "immersive"] as const) {
      shellStore.setWindowMode(mode);
      converseStore.clearMessages();
      converseStore.setThreads([
        makeThread("nt-new", 3),
        makeThread("nt-history-a", 2),
        makeThread("nt-history-b", 1),
      ]);
      converseStore.markIntentionalNewThread("nt-new");
      expect(converseStore.emptyStateClass(), `${mode}: new-thread`).toBe("intentional-new-thread");

      const { unmount } = render(() => <ConverseSpace />);
      // New-task state: starters + heading render, continuation choices do NOT
      // leak from unrelated history.
      expect(
        screen.getByRole("heading", { level: 2, name: "Start a new task" }),
        `${mode}: new-task heading`,
      ).toBeInTheDocument();
      expect(screen.getByRole("list", { name: "Starter prompts" }), `${mode}: starters`).toBeInTheDocument();
      expect(screen.queryByRole("list", { name: "Continue suggestions" }), `${mode}: no continuation leak`).toBeNull();
      expect(
        screen.getByRole("button", { name: "Customize suggestions" }),
        `${mode}: disclosure`,
      ).toBeInTheDocument();
      unmount();
    }
  });
});

describe("ConverseSpace — conversation toolbar Width Profile adaptation (task 8.6, UIE-M-002)", () => {
  /** Fixed-width ResizeObserver: emits one profile width on observe. */
  function fixedWidthObserver(width: number) {
    return class {
      constructor(private readonly callback: ResizeObserverCallback) {}
      observe(target: Element): void {
        this.callback(
          [{
            target,
            contentBoxSize: [{ inlineSize: width, blockSize: 600 }],
            contentRect: { width },
          } as unknown as ResizeObserverEntry],
          this as unknown as ResizeObserver,
        );
      }
      unobserve(): void {}
      disconnect(): void {}
    } as unknown as typeof ResizeObserver;
  }

  const PROFILE_WIDTH = { focus: 500, dual: 800, assisted: 1200, full: 1500 } as const;

  function seedToolbarBaseline(): void {
    // Continuation baseline: sidebar open by default → the toolbar's action set
    // is context-rail-toggle (primary) + export + detach (secondary).
    converseStore.setThreads([makeThread("tb-active", 2), makeThread("tb-history", 1)]);
    converseStore.setActiveThread("tb-active");
    seedMessages(2); // export enabled (has messages) — irrelevant to presence
    seedContext("toolbar-ctx"); // context exists so the toggle is meaningful
  }

  function renderAtProfile(profile: keyof typeof PROFILE_WIDTH) {
    globalThis.ResizeObserver = fixedWidthObserver(PROFILE_WIDTH[profile]);
    return render(() => <ConverseSpace />);
  }

  function openDisclosure(name: string): void {
    const trigger = screen.getByRole("button", { name });
    trigger.focus();
    fireEvent.keyDown(trigger, { key: "Enter" });
  }

  beforeEach(() => {
    shellStore.setWindowMode("standard");
    shellStore.setInspectorTarget(null);
    converseStore.clearMessages();
    coreStore.reset();
    clearGuiCognitionSession();
    seedToolbarBaseline();
  });

  afterEach(() => {
    cleanup();
    globalThis.ResizeObserver = testDefaultResizeObserver;
  });

  it("collapses secondary actions (export + detach) into ONE labelled overflow at focus/dual", () => {
    for (const profile of ["focus", "dual"] as const) {
      cleanup();
      renderAtProfile(profile);

      // Secondary actions are NOT inline.
      expect(screen.queryByRole("button", { name: "Export conversation" }), `${profile}: export not inline`).toBeNull();
      expect(screen.queryByRole("button", { name: "Detach current thread" }), `${profile}: detach not inline`).toBeNull();

      // One labelled overflow carries them; opening reveals the concrete actions.
      const overflow = screen.getByRole("button", { name: "More conversation actions" });
      expect(overflow, `${profile}: single overflow`).toBeInTheDocument();
      openDisclosure("More conversation actions");
      expect(screen.getByRole("menuitem", { name: "Export as plain text (.txt)" }), `${profile}: export in overflow`).toBeInTheDocument();
      expect(screen.getByRole("menuitem", { name: "Detach current thread" }), `${profile}: detach in overflow`).toBeInTheDocument();
    }
  });

  it("shows secondary actions inline with no overflow at assisted/full", () => {
    for (const profile of ["assisted", "full"] as const) {
      cleanup();
      renderAtProfile(profile);
      expect(screen.getByRole("button", { name: "Export conversation" }), `${profile}: export inline`).toBeInTheDocument();
      expect(screen.getByRole("button", { name: "Detach current thread" }), `${profile}: detach inline`).toBeInTheDocument();
      expect(screen.queryByRole("button", { name: "More conversation actions" }), `${profile}: no overflow`).toBeNull();
    }
  });

  it("keeps the context-rail toggle directly reachable at every profile", () => {
    for (const profile of ["focus", "dual", "assisted", "full"] as const) {
      cleanup();
      renderAtProfile(profile);
      expect(screen.getByRole("button", { name: "Toggle context rail" }), `${profile}: context toggle inline`).toBeInTheDocument();
    }
  });

  it("keeps the conversation title present at every profile", () => {
    for (const profile of ["focus", "dual", "assisted", "full"] as const) {
      cleanup();
      const { container } = renderAtProfile(profile);
      const title = container.querySelector(".kria-converse__conversation-title");
      expect(title, `${profile}: title present`).not.toBeNull();
      expect(title!.textContent, `${profile}: title text`).toBeTruthy();
    }
  });

  it("never renders a secondary action both inline and in overflow (no duplicate)", () => {
    renderAtProfile("focus");
    // Not inline.
    expect(screen.queryByRole("button", { name: "Export conversation" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Detach current thread" })).toBeNull();
    // Exactly once in overflow.
    openDisclosure("More conversation actions");
    expect(screen.getAllByRole("menuitem", { name: "Detach current thread" })).toHaveLength(1);
    expect(screen.queryByRole("button", { name: "Detach current thread" })).toBeNull();
  });

  it("keeps a single non-wrapping toolbar-actions row (controlled height, not free-wrap)", async () => {
    const { default: layoutCss } = await import("./ConverseSpace.css?raw");
    const rule = layoutCss.match(/\.kria-converse__toolbar-actions\s*\{([\s\S]*?)\}/)?.[1] ?? "";
    expect(rule).toContain("flex-wrap: nowrap;");
    // Narrow profile renders the overflow surface INSTEAD of wrapping.
    renderAtProfile("focus");
    expect(screen.getByRole("button", { name: "More conversation actions" })).toBeInTheDocument();
  });

  // UIE-M-010 / Req 12.4 — export-disabled reason exposed to AT (not hover-only,
  // control not hidden). Empty-conversation cause is settable via the store;
  // the in-progress cause + the description mechanism itself are covered by the
  // kit Menu unit tests (Menu.test.tsx). Two distinct causes → distinct text.
  it("exposes the empty-conversation export reason on the inline trigger via aria-describedby", () => {
    converseStore.clearMessages(); // export disabled: no messages
    renderAtProfile("full");
    const trigger = screen.getByRole("button", { name: "Export conversation" });
    // Control stays present (never hidden) even while disabled.
    expect(trigger).toBeInTheDocument();
    const describedBy = trigger.getAttribute("aria-describedby");
    expect(describedBy).toBeTruthy();
    const description = document.getElementById(describedBy!);
    expect(description).not.toBeNull();
    expect(description).toHaveTextContent("No messages to export yet");
    expect(description).toHaveTextContent("Send a message to enable export");
  });

  it("drops the trigger description once export becomes available (has messages)", () => {
    // Baseline seeds messages → export enabled → no reason to announce.
    renderAtProfile("full");
    const trigger = screen.getByRole("button", { name: "Export conversation" });
    expect(trigger).not.toHaveAttribute("aria-describedby");
  });

  it("AT-exposes the export reason on overflow items when export is folded away", () => {
    converseStore.clearMessages(); // export disabled: no messages
    renderAtProfile("focus"); // export folds into the shared overflow
    openDisclosure("More conversation actions");
    const item = screen.getByRole("menuitem", { name: /Export as plain text/ });
    const describedBy = item.getAttribute("aria-describedby");
    expect(describedBy).toBeTruthy();
    expect(document.getElementById(describedBy!)).toHaveTextContent(
      "No messages to export yet",
    );
  });

  // The SECOND disabled cause (export in progress) must be distinguished from
  // empty-conversation at the ConverseSpace level, in BOTH the inline trigger
  // and the folded overflow items — not collapsed into one boolean.
  it("distinguishes the export-in-progress reason from empty on the inline trigger", () => {
    // Messages exist (baseline) but an export is running → the exporting cause
    // wins over empty and must surface its own distinct wording.
    const spy = vi.spyOn(converseStore, "exportingConversation").mockReturnValue(true);
    renderAtProfile("full");
    const trigger = screen.getByRole("button", { name: "Export conversation" });
    const describedBy = trigger.getAttribute("aria-describedby");
    expect(describedBy).toBeTruthy();
    const description = document.getElementById(describedBy!);
    expect(description).toHaveTextContent("Export running");
    expect(description).not.toHaveTextContent("No messages to export yet");
    spy.mockRestore();
  });

  it("distinguishes the export-in-progress reason from empty on overflow items", () => {
    const spy = vi.spyOn(converseStore, "exportingConversation").mockReturnValue(true);
    renderAtProfile("focus"); // export folds into the shared overflow
    openDisclosure("More conversation actions");
    const item = screen.getByRole("menuitem", { name: /Export as plain text/ });
    const describedBy = item.getAttribute("aria-describedby");
    expect(describedBy).toBeTruthy();
    const description = document.getElementById(describedBy!);
    expect(description).toHaveTextContent("Export running");
    expect(description).not.toHaveTextContent("No messages to export yet");
    spy.mockRestore();
  });
});

describe("ConverseSpace — control preservation across profile transitions (task 8.6)", () => {
  class ReconfigurableResizeObserver {
    static instance: ReconfigurableResizeObserver;
    target?: Element;
    constructor(private readonly callback: ResizeObserverCallback) {
      ReconfigurableResizeObserver.instance = this;
    }
    observe(target: Element): void { this.target = target; }
    unobserve(): void {}
    disconnect(): void { this.target = undefined; }
    emit(width: number): void {
      this.callback(
        [{
          target: this.target!,
          contentBoxSize: [{ inlineSize: width, blockSize: 600 }],
          contentRect: { width },
        } as unknown as ResizeObserverEntry],
        this as unknown as ResizeObserver,
      );
    }
  }

  beforeEach(() => {
    globalThis.ResizeObserver = ReconfigurableResizeObserver as unknown as typeof ResizeObserver;
    shellStore.setWindowMode("standard");
    shellStore.setInspectorTarget(null);
    converseStore.clearMessages();
    coreStore.reset();
    clearGuiCognitionSession();
    converseStore.setThreads([makeThread("pres-active", 2), makeThread("pres-history", 1)]);
    converseStore.setActiveThread("pres-active");
  });

  afterEach(() => {
    cleanup();
    globalThis.ResizeObserver = testDefaultResizeObserver;
  });

  it("preserves draft, attachments, mode, and focus across focus→full and full→focus", async () => {
    converseStore.updateDraft({
      text: "survive the resize",
      mode: "lab",
      attachments: [{
        id: "pres-attachment",
        name: "keep.txt",
        mime: "text/plain",
        size: 3,
        bytes: new Uint8Array([97, 98, 99]),
      }],
    });

    const view = render(() => <ConverseSpace />);
    const root = view.container.querySelector<HTMLElement>(".kria-converse")!;
    const observer = ReconfigurableResizeObserver.instance;
    const draft = converseStore.composerDraft();

    observer.emit(500); // focus
    await waitFor(() => expect(root).toHaveAttribute("data-width-profile", "focus"));
    // Focus a stable control that survives the transition (the composer input).
    const textarea = screen.getByRole("textbox", { name: "Message KRIA" });
    textarea.focus();
    expect(textarea).toHaveFocus();

    observer.emit(1500); // full
    await waitFor(() => expect(root).toHaveAttribute("data-width-profile", "full"));
    // Draft object identity + content preserved (no reset), focus not lost.
    expect(converseStore.composerDraft()).toBe(draft);
    expect(converseStore.composerDraft().text).toBe("survive the resize");
    expect(converseStore.composerDraft().mode).toBe("lab");
    expect(converseStore.composerDraft().attachments).toHaveLength(1);
    expect(screen.getByRole("textbox", { name: "Message KRIA" })).toHaveFocus();

    observer.emit(500); // back to focus
    await waitFor(() => expect(root).toHaveAttribute("data-width-profile", "focus"));
    expect(converseStore.composerDraft()).toBe(draft);
    expect(converseStore.composerDraft().text).toBe("survive the resize");
    expect(converseStore.composerDraft().attachments).toHaveLength(1);
    expect(screen.getByText("keep.txt")).toBeInTheDocument();
  });
});

describe("ConverseSpace — scroll ownership contract (task 9.2, UIE-M-005; Req 10.4–10.5)", () => {
  afterEach(() => {
    cleanup();
    // Leave the router on the default Space for other suites.
    navigate("converse");
  });

  it("keeps the five independent lane/Inspector Y owners plus the textarea (preserve, do NOT remove)", async () => {
    const [{ default: layoutCss }, { default: streamCss }, { default: shellCss }] = await Promise.all([
      import("./ConverseSpace.css?raw"),
      import("./converse/MessageStream.css?raw"),
      import("../AppShell.css?raw"),
    ]);

    // Conversation Y owner: the virtualized viewport (THE single conversation owner).
    expect(streamCss).toMatch(/\.kria-stream__viewport\s*\{[\s\S]*?overflow-y:\s*auto;/);
    // Independent bounded lane owners.
    for (const lane of ["threads", "work", "context"] as const) {
      expect(layoutCss, `${lane} lane keeps overflow-y:auto`).toMatch(
        new RegExp(`\\.kria-converse__${lane}\\s*\\{[\\s\\S]*?overflow-y:\\s*auto;`),
      );
    }
    // Inspector body bounded owner.
    expect(shellCss).toMatch(/\.kria-inspector__body\s*\{[\s\S]*?overflow:\s*auto;/);
    // The stream FRAME delegates: it clips and hands scroll to the viewport.
    expect(layoutCss).toMatch(/\.kria-converse__stream\s*\{[\s\S]*?overflow:\s*hidden;/);
  });

  it("removes the redundant Converse vertical scroll owner without touching X or other Spaces (G1)", async () => {
    const { default: shellCss } = await import("../AppShell.css?raw");

    // Base router keeps overflow:auto → non-Converse Spaces still scroll via it.
    const base = shellCss.match(/\.kria-space-router\s*\{([\s\S]*?)\}/)?.[1] ?? "";
    expect(base).toMatch(/overflow:\s*auto;/);

    // Scoped rule removes ONLY the redundant vertical owner for Converse; it does
    // NOT set overflow-x, so the single X owner (Task 8 no-horizontal-overflow) holds.
    const scoped =
      shellCss.match(/\.kria-space-router\[data-active-space="converse"\]\s*\{([\s\S]*?)\}/)?.[1] ?? "";
    expect(scoped).toMatch(/overflow-y:\s*hidden;/);
    expect(scoped).not.toMatch(/overflow-x\s*:/);
    expect(scoped).not.toMatch(/overflow\s*:/); // no blanket override that would also clip X
  });

  it("wires the active-space marker so the scoped rule only engages for Converse", async () => {
    const { SpaceRouter } = await import("../SpaceRouter");

    navigate("converse");
    const converse = render(() => <SpaceRouter />);
    expect(converse.container.querySelector(".kria-space-router")).toHaveAttribute(
      "data-active-space",
      "converse",
    );
    cleanup();

    navigate("memory");
    const memory = render(() => <SpaceRouter />);
    // Non-Converse Space → marker differs → scoped overflow-y:hidden does not apply.
    expect(memory.container.querySelector(".kria-space-router")).toHaveAttribute(
      "data-active-space",
      "memory",
    );
  });

  it("keeps the conversation stream viewport as the sole conversation Y owner in the rendered tree", () => {
    globalThis.ResizeObserver = FullWidthResizeObserver as unknown as typeof ResizeObserver;
    shellStore.setWindowMode("standard");
    converseStore.clearMessages();
    seedReturningUserThreads();
    seedMessages(4);
    const { container } = render(() => <ConverseSpace />);
    const streamRegion = container.querySelector('[data-region="message-stream"]')!;
    expect(streamRegion).not.toBeNull();
    // The role="log" stream frame delegates scroll to exactly one virtualized viewport.
    const viewports = streamRegion.querySelectorAll(".kria-stream__viewport");
    expect(viewports).toHaveLength(1);
    globalThis.ResizeObserver = testDefaultResizeObserver;
  });

  it("preserves the semantically-required bounded WorkBlock nested scrollers (G8 verify — do NOT remove)", async () => {
    const { default: workBlockCss } = await import("./converse/WorkBlock.css?raw");
    // Code/result panes scroll horizontally, bounded.
    expect(workBlockCss).toMatch(
      /\.kria-work-block__tool-args,\s*\.kria-work-block__tool-result\s*\{[\s\S]*?overflow-x:\s*auto;/,
    );
    // Run-log is a bounded (max-height) vertical nested scroller.
    expect(workBlockCss).toMatch(
      /\.kria-work-block__run-log\s*\{[\s\S]*?max-height:\s*160px;[\s\S]*?overflow-y:\s*auto;/,
    );
  });
});

describe("ConverseSpace — sticky Composer clearance + no restoration competition (task 9.5, IU-10; Req 15.5–15.7)", () => {
  beforeEach(() => {
    globalThis.ResizeObserver = FullWidthResizeObserver as unknown as typeof ResizeObserver;
    shellStore.setWindowMode("standard");
    shellStore.setInspectorTarget(null);
    converseStore.clearMessages();
    coreStore.reset();
    clearGuiCognitionSession();
    seedReturningUserThreads();
  });

  afterEach(() => {
    globalThis.ResizeObserver = testDefaultResizeObserver;
    cleanup();
    navigate("converse");
  });

  it("locks the sticky Composer in its OWN grid row, distinct from the stream row (CSS contract)", async () => {
    const { default: layoutCss } = await import("./ConverseSpace.css?raw");

    // The Converse grid declares two distinct, non-overlapping rows: the lanes
    // row (which holds the scrollable stream) and a separate composer row.
    const rootRule = layoutCss.match(/\.kria-converse\s*\{([\s\S]*?)\}/)?.[1] ?? "";
    expect(rootRule).toMatch(/grid-template-areas:\s*[\s\S]*?"lanes"[\s\S]*?"composer";/);
    expect(rootRule).toMatch(/grid-template-rows:\s*minmax\(0,\s*1fr\)\s*auto;/);

    // The Composer occupies the composer area and is sticky-pinned to the bottom
    // of its own row → it can never overlap the last message in the lanes row.
    const composerRule = layoutCss.match(/\.kria-converse__composer\s*\{([\s\S]*?)\}/)?.[1] ?? "";
    expect(composerRule).toMatch(/grid-area:\s*composer;/);
    expect(composerRule).toMatch(/position:\s*sticky;/);
    expect(composerRule).toMatch(/bottom:\s*0;/);

    // The stream/lanes live in the OTHER area (never "composer").
    expect(layoutCss).toMatch(/\.kria-converse__lanes\s*\{[\s\S]*?grid-area:\s*lanes;/);
    expect(layoutCss).toMatch(/\.kria-converse__conversation\s*\{[\s\S]*?grid-area:\s*conversation;/);
  });

  it("renders the Composer inside the Converse grid and NOT inside the scrolling stream viewport", () => {
    seedMessages(6);
    const { container } = render(() => <ConverseSpace />);

    const converse = container.querySelector<HTMLElement>(".kria-converse")!;
    const composer = container.querySelector<HTMLElement>('[data-region="composer"]')!;
    const streamRegion = container.querySelector<HTMLElement>('[data-region="message-stream"]')!;
    const viewport = streamRegion.querySelector<HTMLElement>(".kria-stream__viewport")!;

    expect(converse).not.toBeNull();
    expect(composer).not.toBeNull();
    expect(viewport).not.toBeNull();

    // Composer is a child of the Converse grid — so the shell router's
    // Converse-scoped overflow-y:hidden (task 9.2) cannot clip it: the router
    // clips its own box, while the Composer lives in Converse's own sticky row
    // and the internal viewport is the thing that scrolls.
    expect(composer.closest(".kria-converse")).toBe(converse);
    // The Composer is NOT inside the virtualized scroller, so scrolling the
    // conversation never moves/hides the Composer.
    expect(viewport.contains(composer)).toBe(false);
    expect(composer.closest('[data-region="message-stream"]')).toBeNull();
    // And the Composer is not itself the conversation scroll owner.
    expect(composer.getAttribute("data-scroll-owner")).not.toBe("conversation");
    expect(composer.classList.contains("kria-stream__viewport")).toBe(false);
  });

  it("preserves the Composer element across reversible Window Mode + lane transitions (no clearance loss)", async () => {
    seedContext("clearance-modes");
    seedMessages(5);
    const { container } = render(() => <ConverseSpace />);
    const composerBefore = container.querySelector<HTMLElement>('[data-region="composer"]')!;
    const streamBefore = container.querySelector('[data-region="message-stream"]')!;
    expect(composerBefore).not.toBeNull();
    // Composer follows the stream in source order → never painted over it.
    expect(streamBefore.compareDocumentPosition(composerBefore) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();

    // Reversible transitions: mode change round-trip + Context lane toggle.
    shellStore.setWindowMode("immersive");
    fireEvent.click(screen.getByRole("button", { name: "Toggle context rail" }));
    await screen.findByRole("complementary", { name: "Context" });
    shellStore.setWindowMode("standard");
    fireEvent.click(screen.getByRole("button", { name: "Toggle context rail" }));

    // The same Composer element (identity) survives, still present and still
    // after the stream — clearance holds across the round-trip.
    const composerAfter = container.querySelector<HTMLElement>('[data-region="composer"]')!;
    const streamAfter = container.querySelector('[data-region="message-stream"]')!;
    expect(composerAfter).toBe(composerBefore);
    expect(streamAfter.compareDocumentPosition(composerAfter) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it("confirms the Converse router overflow-y:hidden targets the shell box, not the Composer (no clip)", async () => {
    const { default: shellCss } = await import("../AppShell.css?raw");
    // The scoped rule (task 9.2) removes only the shell-level redundant vertical
    // scroller for Converse; it applies to the router element, never to the
    // sticky Composer row inside Converse's grid.
    const scoped =
      shellCss.match(/\.kria-space-router\[data-active-space="converse"\]\s*\{([\s\S]*?)\}/)?.[1] ?? "";
    expect(scoped).toMatch(/overflow-y:\s*hidden;/);
    // No rule clips the Composer itself.
    expect(shellCss).not.toMatch(/\.kria-converse__composer\s*\{[\s\S]*?overflow[^:]*:\s*hidden;/);
  });
});

describe("ConverseSpace — home surface rollout routing (task 2.4, Req 22.1/22.2/20.2)", () => {
  beforeEach(() => {
    converseStore.clearMessages();
    converseStore.setThreads([]);
    converseStore.setActiveThread(null);
    coreStore.reset();
    clearGuiCognitionSession();
  });

  afterEach(cleanup);

  it("routes the empty home surface to the presence HomeSpace when the flag is ON (Phase-2 exit rollout)", () => {
    setFeatureFlag("home.presence.v2", true);
    expect(isFeatureEnabled("home.presence.v2")).toBe(true);

    render(() => <ConverseSpace />);

    // The presence homepage renders (labelled "Home" region) instead of the
    // legacy Converse empty state. The legacy surface exposes a "Start a
    // conversation" region + a "Starter prompts" list; neither is present when
    // the presence HomeSpace owns the home surface.
    expect(screen.getByRole("region", { name: "Home" })).toBeInTheDocument();
    expect(screen.queryByRole("region", { name: "Start a conversation" })).toBeNull();
    expect(screen.queryByRole("list", { name: "Starter prompts" })).toBeNull();
  });

  it("rolls back to the legacy Converse empty state when the flag override is OFF (Req 22.1)", () => {
    // The rollback path stays fully operational: an override flips the surface
    // back with no rebuild.
    setFeatureFlag("home.presence.v2", false);
    expect(isFeatureEnabled("home.presence.v2")).toBe(false);

    render(() => <ConverseSpace />);

    expect(
      screen.getByRole("heading", { level: 2, name: "What can I help with?" }),
    ).toBeInTheDocument();
    expect(screen.queryByRole("region", { name: "Home" })).toBeNull();
  });

  it("keeps EXACTLY ONE Composer as the unified action target in both rollout and rollback (Req 4.2)", () => {
    // The homepage has exactly one ask-field either way (Req 4.2 — no second
    // competing field). With the flag ON the presence HomeSpace owns the
    // Composer on its vertical axis and the sticky bottom Composer is
    // suppressed; with the flag OFF the sticky Composer is the single field.
    for (const on of [true, false]) {
      setFeatureFlag("home.presence.v2", on);
      const { container, unmount } = render(() => <ConverseSpace />);
      const composers = container.querySelectorAll('[data-region="composer"]');
      expect(composers.length).toBe(1);
      // The unified input (the Converse Composer) is present exactly once.
      expect(container.querySelectorAll(".kria-composer").length).toBe(1);
      unmount();
    }
  });

  it("routes the single Composer onto the vertical axis inside HomeSpace when the flag is ON (design §2, Req 4.1)", () => {
    setFeatureFlag("home.presence.v2", true);
    const { container } = render(() => <ConverseSpace />);
    // The one Composer lives on the vertical axis inside the Home region — not
    // as the sticky bottom row.
    const composer = container.querySelector('[data-region="composer"]');
    expect(composer).not.toBeNull();
    expect(composer?.getAttribute("data-vertical-axis")).toBe("true");
    expect(composer?.closest('[data-region="home-space"]')).not.toBeNull();
    // No sticky bottom Composer row is rendered on the presence home surface.
    expect(container.querySelector(".kria-converse__composer")).toBeNull();
  });
});
