/**
 * ContextRail enrichment tests (Task 10.4 / IU-07; UIE-M-011).
 *
 * Verifies the field-only enrichment of `ContextRailItem` (source / use /
 * detail) renders through the existing rail item pattern with strict omission,
 * that available-vs-used is distinguished by accessible TEXT (not colour alone,
 * Req 17.3), that an empty rail is never auto-opened and fabricates nothing, and
 * that rendering the rail issues NO backend/bridge request (pure presentation).
 *
 * The bridge invoke module is mocked here (isolated to this file) so the
 * "no backend request" assertion is exact; the existing ConverseSpace.test.tsx
 * regression suite stays untouched.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { render, screen, fireEvent, cleanup } from "@solidjs/testing-library";

vi.mock("../../bridge/invoke", () => ({
  bridgeInvoke: vi.fn(async () => undefined),
  bridgeInvokeOptional: vi.fn(async () => undefined),
}));

import ConverseSpace from "./ConverseSpace";
import { InspectorHost } from "../InspectorHost";
import {
  registerInspectorRenderer,
  resetInspectorRegistry,
} from "../inspectorRegistry";
import { bridgeInvoke, bridgeInvokeOptional } from "../../bridge/invoke";
import { converseStore, coreStore, shellStore } from "../../stores";
import type { ContextRailItem } from "../../stores/converseStore";
import { clearGuiCognitionSession } from "../../stores/guiCognitionSession";
import { createSignal } from "solid-js";

const defaultResizeObserver = globalThis.ResizeObserver;

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

function seedThreads(): void {
  converseStore.setThreads([
    { id: "rail-active", title: "Active", createdAt: 0, updatedAt: 2, pinned: false, archived: false, temporary: false },
    { id: "rail-history", title: "History", createdAt: 0, updatedAt: 1, pinned: false, archived: false, temporary: false },
  ]);
  converseStore.setActiveThread("rail-active");
}

/** Open the on-demand rail via its toolbar toggle. */
function openRail(): void {
  fireEvent.click(screen.getByRole("button", { name: "Toggle context rail" }));
}

describe("ConverseSpace — ContextRail enrichment (Task 10.4, UIE-M-011)", () => {
  beforeEach(() => {
    globalThis.ResizeObserver = FullWidthResizeObserver as unknown as typeof ResizeObserver;
    shellStore.setWindowMode("standard");
    shellStore.setInspectorTarget(null);
    converseStore.clearMessages();
    coreStore.reset();
    clearGuiCognitionSession();
    seedThreads();
    vi.mocked(bridgeInvoke).mockClear();
    vi.mocked(bridgeInvokeOptional).mockClear();
  });

  afterEach(() => {
    cleanup();
    globalThis.ResizeObserver = defaultResizeObserver;
  });

  it("renders type (icon + text), source, use-state, and detail when a writer provides them", () => {
    converseStore.setContextRailItems([
      {
        id: "enriched",
        type: "document",
        label: "Q3 report",
        data: null,
        source: "quarterly-report.pdf",
        use: "used",
        detail: "Pages 3-5 summarized",
      },
    ]);
    render(() => <ConverseSpace />);
    openRail();

    const rail = screen.getByRole("complementary", { name: "Context" });
    // type surfaced as text (not icon/colour alone, Req 17.3)
    expect(rail).toHaveTextContent("Document");
    // label
    expect(rail).toHaveTextContent("Q3 report");
    // source
    expect(rail).toHaveTextContent("Source");
    expect(rail).toHaveTextContent("quarterly-report.pdf");
    // use-state as text
    expect(rail).toHaveTextContent("Used");
    // concise detail
    expect(rail).toHaveTextContent("Pages 3-5 summarized");
  });

  it("omits absent enrichment: a label-only item shows just its type + label", () => {
    converseStore.setContextRailItems([
      { id: "bare", type: "memory", label: "Just a label", data: null },
    ]);
    render(() => <ConverseSpace />);
    openRail();

    const item = document.querySelector<HTMLElement>('[data-context-id="bare"]')!;
    expect(item).not.toBeNull();
    expect(item).toHaveTextContent("Memory"); // type text
    expect(item).toHaveTextContent("Just a label"); // label
    // No source/use/detail rendered when absent.
    expect(item.textContent).not.toContain("Source");
    expect(item.querySelector(".kria-converse__context-item-use")).toBeNull();
    expect(item.querySelector(".kria-converse__context-item-detail")).toBeNull();
    expect(item.querySelector(".kria-converse__context-item-meta")).toBeNull();
  });

  it("omits blank source/detail (nonEmpty discipline — never a placeholder)", () => {
    converseStore.setContextRailItems([
      { id: "blank", type: "custom", label: "Blank meta", data: null, source: "   ", detail: "" },
    ]);
    render(() => <ConverseSpace />);
    openRail();

    const item = document.querySelector<HTMLElement>('[data-context-id="blank"]')!;
    expect(item.querySelector(".kria-converse__context-item-meta")).toBeNull();
    expect(item.querySelector(".kria-converse__context-item-detail")).toBeNull();
  });

  it("distinguishes available vs used by accessible text, not colour alone (Req 17.3)", () => {
    converseStore.setContextRailItems([
      { id: "avail", type: "memory", label: "Available item", data: null, use: "available" },
      { id: "used", type: "memory", label: "Used item", data: null, use: "used" },
    ]);
    render(() => <ConverseSpace />);
    openRail();

    const avail = document.querySelector<HTMLElement>('[data-context-id="avail"] .kria-converse__context-item-use')!;
    const used = document.querySelector<HTMLElement>('[data-context-id="used"] .kria-converse__context-item-use')!;
    expect(avail).toHaveTextContent("Available");
    expect(used).toHaveTextContent("Used");
    // Redundant machine-readable cue in addition to the text.
    expect(avail).toHaveAttribute("data-use", "available");
    expect(used).toHaveAttribute("data-use", "used");
  });

  it("never auto-opens an empty rail and fabricates no placeholder item", () => {
    converseStore.setContextRailItems([]);
    render(() => <ConverseSpace />);
    const toggle = screen.getByRole("button", { name: "Toggle context rail" });
    // Not open by default.
    expect(toggle).toHaveAttribute("aria-pressed", "false");
    expect(screen.queryByRole("complementary", { name: "Context" })).toBeNull();
    // User toggles with no items → stays closed, nothing fabricated.
    fireEvent.click(toggle);
    expect(toggle).toHaveAttribute("aria-pressed", "false");
    expect(screen.queryByRole("complementary", { name: "Context" })).toBeNull();
    expect(document.querySelector(".kria-converse__context-item")).toBeNull();
  });

  it("issues no backend/bridge request when rendering the enriched rail", () => {
    converseStore.setContextRailItems([
      { id: "no-fetch", type: "tool-result", label: "Tool output", data: null, source: "web.search", use: "used" },
    ]);
    render(() => <ConverseSpace />);
    openRail();

    expect(screen.getByRole("complementary", { name: "Context" })).toBeInTheDocument();
    expect(bridgeInvoke).not.toHaveBeenCalled();
    expect(bridgeInvokeOptional).not.toHaveBeenCalled();
  });

  it("links a memory item to the shared Memory Inspector via a real keyboard button (Task 10.5)", () => {
    converseStore.setContextRailItems([
      { id: "rail-item-1", type: "memory", label: "Recalled fact", data: null, source: "mem-77", use: "used" },
    ]);
    render(() => <ConverseSpace />);
    openRail();

    // The memory item is a REAL focusable button (not a click-only div).
    const control = document.querySelector<HTMLButtonElement>(
      'button[data-context-id="rail-item-1"]',
    )!;
    expect(control).not.toBeNull();
    expect(control.tagName).toBe("BUTTON");
    expect(control).toHaveAttribute("data-context-type", "memory");
    // Accessible name states the destination.
    expect(control.getAttribute("aria-label")).toContain("Memory");

    // Activation opens the ONE shared Inspector on the memory id (source-owned).
    fireEvent.click(control);
    expect(shellStore.inspectorTarget()).toEqual({
      type: "memory",
      id: "mem-77",
      data: undefined,
    });
    // Read-only: only the Inspector opened — no backend request.
    expect(bridgeInvoke).not.toHaveBeenCalled();
    expect(bridgeInvokeOptional).not.toHaveBeenCalled();
  });

  it("falls back to the item id when a memory item carries no source", () => {
    converseStore.setContextRailItems([
      { id: "mem-only-id", type: "memory", label: "No source", data: null },
    ]);
    render(() => <ConverseSpace />);
    openRail();

    const control = document.querySelector<HTMLButtonElement>(
      'button[data-context-id="mem-only-id"]',
    )!;
    expect(control).not.toBeNull();
    fireEvent.click(control);
    expect(shellStore.inspectorTarget()).toEqual({
      type: "memory",
      id: "mem-only-id",
      data: undefined,
    });
  });

  it("does NOT link non-memory items (no registered Inspector owner → static item, no fabrication)", () => {
    converseStore.setContextRailItems([
      { id: "doc-1", type: "document", label: "A doc", data: null, source: "report.pdf" },
      { id: "tool-1", type: "tool-result", label: "A tool result", data: null },
      { id: "custom-1", type: "custom", label: "Custom", data: null },
    ]);
    render(() => <ConverseSpace />);
    openRail();

    // Rendered as static divs, never buttons.
    for (const idAttr of ["doc-1", "tool-1", "custom-1"]) {
      expect(document.querySelector(`button[data-context-id="${idAttr}"]`)).toBeNull();
      const el = document.querySelector<HTMLElement>(`[data-context-id="${idAttr}"]`)!;
      expect(el.tagName).toBe("DIV");
    }
    // No Inspector opened.
    expect(shellStore.inspectorTarget()).toBeNull();
  });

  it("accepts existing writers that set no new fields (shape stays backward-compatible)", () => {
    // Compile-time + runtime: an item with only the original fields is valid.
    const legacy: ContextRailItem = { id: "legacy", type: "custom", label: "Legacy", data: { any: 1 } };
    expect(legacy.source).toBeUndefined();
    expect(legacy.use).toBeUndefined();
    expect(legacy.detail).toBeUndefined();
    converseStore.setContextRailItems([legacy]);
    render(() => <ConverseSpace />);
    openRail();
    expect(document.querySelector('[data-context-id="legacy"]')).not.toBeNull();
  });
});

// ── Task 10.7 — truthful bounded / edge-case presentation (UIE-H-002, M-011) ──
describe("ConverseSpace — ContextRail bounded/edge cases (Task 10.7)", () => {
  beforeEach(() => {
    globalThis.ResizeObserver = FullWidthResizeObserver as unknown as typeof ResizeObserver;
    shellStore.setWindowMode("standard");
    shellStore.setInspectorTarget(null);
    converseStore.clearMessages();
    coreStore.reset();
    clearGuiCognitionSession();
    resetInspectorRegistry();
    seedThreads();
    vi.mocked(bridgeInvoke).mockClear();
    vi.mocked(bridgeInvokeOptional).mockClear();
  });

  afterEach(() => {
    cleanup();
    resetInspectorRegistry();
    globalThis.ResizeObserver = defaultResizeObserver;
  });

  const LONG_LABEL =
    "An extraordinarily long context label that would otherwise expand the lane and force horizontal overflow across the layout";
  const LONG_SOURCE =
    "extremely-long-source-provenance-identifier-that-must-not-break-the-layout.document.v3.final";
  const LONG_DETAIL =
    "A verbose concise-detail string repeated enough times to exceed three lines of clamped text so bounded presentation is exercised end to end without breaking the lane width invariant.";

  it("bounds long label/source/detail with the shared clamp classes (no horizontal overflow)", () => {
    converseStore.setContextRailItems([
      { id: "long", type: "document", label: LONG_LABEL, data: null, source: LONG_SOURCE, detail: LONG_DETAIL },
    ]);
    render(() => <ConverseSpace />);
    openRail();

    const item = document.querySelector<HTMLElement>('[data-context-id="long"]')!;
    // Shared bounded-text utility classes applied (task 10.7 consolidation).
    expect(item.querySelector(".kria-converse__context-item-label")).toHaveClass("kria-bounded--2");
    expect(item.querySelector(".kria-converse__context-item-meta-value")).toHaveClass("kria-bounded");
    expect(item.querySelector(".kria-converse__context-item-detail")).toHaveClass("kria-bounded--3");
  });

  it("keeps the FULL value accessible: full text stays in the DOM + is offered via title", () => {
    converseStore.setContextRailItems([
      { id: "full", type: "document", label: LONG_LABEL, data: null, source: LONG_SOURCE, detail: LONG_DETAIL },
    ]);
    render(() => <ConverseSpace />);
    openRail();

    const label = document.querySelector<HTMLElement>(".kria-converse__context-item-label")!;
    const source = document.querySelector<HTMLElement>(".kria-converse__context-item-meta-value")!;
    const detail = document.querySelector<HTMLElement>(".kria-converse__context-item-detail")!;
    // DOM retains the untruncated text (assistive tech reads it in full).
    expect(label.textContent).toBe(LONG_LABEL);
    expect(source.textContent).toBe(LONG_SOURCE);
    expect(detail.textContent).toBe(LONG_DETAIL);
    // Sighted users recover it on hover via the full-value title.
    expect(label).toHaveAttribute("title", LONG_LABEL);
    expect(source).toHaveAttribute("title", LONG_SOURCE);
    expect(detail).toHaveAttribute("title", LONG_DETAIL);
  });

  it("renders no placeholder / 'undefined' text for a missing-field item", () => {
    converseStore.setContextRailItems([
      { id: "sparse", type: "memory", label: "Recalled note", data: null },
    ]);
    render(() => <ConverseSpace />);
    openRail();

    const item = document.querySelector<HTMLElement>('[data-context-id="sparse"]')!;
    const text = item.textContent ?? "";
    expect(text).not.toMatch(/undefined|null|N\/A|—\s*$/i);
    // No empty title attribute is emitted for the absent-source/detail path.
    expect(item.querySelector(".kria-converse__context-item-meta")).toBeNull();
    expect(item.querySelector(".kria-converse__context-item-detail")).toBeNull();
  });

  it("STALE memory id: the fact-link degrades gracefully — Inspector auto-closes, never a dangling entity (reuses 9.4)", async () => {
    // A registered memory renderer that reports its target removed (null) once
    // the entity id is no longer live — the decoupled §20.4 removal signal.
    const [live, setLive] = createSignal(new Set<string>(["mem-live"]));
    registerInspectorRenderer("memory", (t) =>
      live().has(t.id) ? { title: `Memory ${t.id}`, body: <p>alive</p> } : null,
    );
    // A rail item referencing a STALE memory id (its fact was deleted).
    converseStore.setContextRailItems([
      { id: "stale-item", type: "memory", label: "Deleted fact", data: null, source: "mem-stale" },
    ]);
    render(() => (
      <>
        <ConverseSpace />
        <InspectorHost />
      </>
    ));
    openRail();

    // The link is a REAL button to a registered Inspector type — never a broken
    // or fabricated dead link.
    const control = document.querySelector<HTMLButtonElement>(
      'button[data-context-id="stale-item"]',
    )!;
    expect(control.tagName).toBe("BUTTON");
    fireEvent.click(control);

    // Opening a stale target routes to the registered memory renderer, which
    // returns null → the host auto-closes exactly once and returns focus
    // (§20.4). No dangling entity is ever rendered.
    await new Promise<void>((r) => setTimeout(r, 0));
    expect(shellStore.inspectorTarget()).toBeNull();
    expect(screen.queryByRole("complementary", { name: "Inspector" })).toBeNull();
    // Purely presentation/navigation — no backend request to validate the id.
    expect(bridgeInvoke).not.toHaveBeenCalled();
    expect(bridgeInvokeOptional).not.toHaveBeenCalled();

    // A live id, by contrast, resolves and stays open.
    setLive(new Set<string>(["mem-live"]));
    shellStore.openInspector("memory", "mem-live");
    expect(screen.getByRole("complementary", { name: "Inspector" })).toBeInTheDocument();
  });
});
