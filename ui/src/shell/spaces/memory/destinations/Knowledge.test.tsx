/**
 * Tests for Knowledge destination (task 4.2.4).
 *
 * Validates:
 * - knowledge-shell wrapper is always rendered
 * - List view is always rendered (complete fallback, even with no items)
 * - Map view shown only when mapParityReady=true
 * - Map view NOT rendered when mapParityReady=false (no placeholder)
 * - map-canvas inside map-view when mapParityReady=true
 * - Inspector button rendered on each item when inspectorAvailable=true
 * - Inspector button NOT rendered when inspectorAvailable=false
 * - onOpenInspector called with correct id when inspector button clicked
 * - Path button shown only when pathAvailable=true AND selectedId !== item.id
 * - Path button NOT shown when pathAvailable=false
 * - Path button NOT shown when pathAvailable=true but no item is selected
 * - Path button NOT shown when pathAvailable=true but selectedId === item.id
 * - onRequestPath called with (selectedId, itemId) when path button clicked
 * - Correction status shown when correctionAvailable=true
 * - Correction status NOT shown when correctionAvailable=false
 * - Loading indicator shown when isLoading=true
 * - Loading indicator NOT shown when isLoading=false
 * - Empty state shown when no items and isLoading=false
 * - Empty state NOT shown when items are present
 * - Empty state NOT shown while loading
 * - onSelectItem called with correct id when select button clicked
 * - Each item renders label, kind, authorityClass, truthState attributes
 *
 * Requirements: F4.2 (task 4.2.4) — Knowledge destination.
 *               MGR-002, MGR-012, MGR-014, MGR-015, MGR-023.
 */
import { describe, it, expect, afterEach, vi } from "vitest";
import { render, screen, cleanup, fireEvent } from "@solidjs/testing-library";
import {
  Knowledge,
  type KnowledgeProps,
  type KnowledgeItem,
} from "./Knowledge";
import type { SemanticScene } from "../scene/semanticScene";

afterEach(() => cleanup());

// ─── Helpers ─────────────────────────────────────────────────────────────────

function makeItem(overrides: Partial<KnowledgeItem> = {}): KnowledgeItem {
  return {
    id: "item-1",
    kind: "entity",
    authorityClass: "stored",
    label: "Test Entity",
    truthState: "Current",
    revision: 1,
    ...overrides,
  };
}

function makeScene(): SemanticScene {
  return {
    sceneHash: "knowledge-test-scene",
    graphRevision: 1,
    items: [],
    actions: [],
    tokens: [],
    layoutHint: {
      seed: 1,
      strategy: "search-treemap-grid",
      primaryItemId: null,
      maxDepth: null,
    },
    diagnostics: [],
  };
}

function renderKnowledge(props: Partial<KnowledgeProps> = {}) {
  const defaults: KnowledgeProps = {
    items: [],
    scene: makeScene(),
    selectedId: null,
    focusTrail: [],
    loadedNodeCount: 0,
    snapshotItemCount: null,
    graphRevision: null,
    snapshotTruncated: false,
    filterQuery: "",
    inspectorAvailable: false,
    pathAvailable: false,
    correctionAvailable: false,
    mapParityReady: false,
    isLoading: false,
    onFilterQuery: vi.fn(),
    onSelectItem: vi.fn(),
    onOpenInspector: vi.fn(),
    onRequestPath: vi.fn(),
    onBack: vi.fn(),
    onReset: vi.fn(),
  };
  return render(() => <Knowledge {...defaults} {...props} />);
}

// ─── knowledge-shell wrapper ──────────────────────────────────────────────────

describe("knowledge-shell wrapper", () => {
  it("always renders the knowledge-shell section", () => {
    renderKnowledge();
    expect(screen.getByTestId("knowledge-shell")).toBeInTheDocument();
  });
});

// ─── List view (always rendered) ──────────────────────────────────────────────

describe("list view", () => {
  it("always renders the list-view section with items present", () => {
    renderKnowledge({ items: [makeItem()] });
    expect(screen.getByTestId("list-view")).toBeInTheDocument();
  });

  it("always renders the list-view section even with no items", () => {
    renderKnowledge({ items: [] });
    expect(screen.getByTestId("list-view")).toBeInTheDocument();
  });

  it("renders items in the list view", () => {
    const items = [
      makeItem({ id: "a1", label: "Alpha" }),
      makeItem({ id: "b2", label: "Beta" }),
    ];
    renderKnowledge({ items });
    const listView = screen.getByTestId("list-view");
    expect(listView).toHaveTextContent("Alpha");
    expect(listView).toHaveTextContent("Beta");
  });

  it("renders item with correct data attributes", () => {
    const item = makeItem({
      id: "e1",
      kind: "memory",
      authorityClass: "derived",
      truthState: "Stale",
    });
    renderKnowledge({ items: [item] });
    const li = document.querySelector("[data-item-id='e1']");
    expect(li).not.toBeNull();
    expect(li).toHaveAttribute("data-kind", "memory");
    expect(li).toHaveAttribute("data-authority-class", "derived");
    expect(li).toHaveAttribute("data-truth-state", "Stale");
  });

  it("renders item label in a labeled span", () => {
    const item = makeItem({ id: "x1", label: "My Label" });
    renderKnowledge({ items: [item] });
    const labelEl = document.querySelector(
      "[data-item-id='x1'] [data-field='label']",
    );
    expect(labelEl).not.toBeNull();
    expect(labelEl?.textContent).toBe("My Label");
  });
});

// ─── Map view ─────────────────────────────────────────────────────────────────

describe("map view", () => {
  it("shows map-view section when mapParityReady=true", () => {
    renderKnowledge({ mapParityReady: true });
    expect(screen.getByTestId("map-view")).toBeInTheDocument();
  });

  it("keeps the prototype workspace mounted when parity is unavailable", () => {
    renderKnowledge({ mapParityReady: false });
    expect(screen.getByTestId("map-view")).toBeInTheDocument();
  });

  it("renders map-canvas inside map-view when mapParityReady=true", () => {
    renderKnowledge({ mapParityReady: true });
    expect(screen.getByTestId("map-canvas")).toBeInTheDocument();
  });

  it("keeps the prototype canvas mounted for honest empty-state rendering", () => {
    renderKnowledge({ mapParityReady: false });
    expect(screen.getByTestId("map-canvas")).toBeInTheDocument();
  });

  it("list-view is still rendered alongside map-view when mapParityReady=true", () => {
    renderKnowledge({ mapParityReady: true, items: [makeItem()] });
    expect(screen.getByTestId("list-view")).toBeInTheDocument();
    expect(screen.getByTestId("map-view")).toBeInTheDocument();
  });
});

// ─── Inspector button ─────────────────────────────────────────────────────────

describe("inspector button", () => {
  it("shows inspector-btn on each item when inspectorAvailable=true", () => {
    const items = [makeItem({ id: "a" }), makeItem({ id: "b" })];
    renderKnowledge({ items, inspectorAvailable: true });
    const buttons = screen.getAllByTestId("inspector-btn");
    expect(buttons.length).toBe(2);
  });

  it("does NOT show inspector-btn when inspectorAvailable=false", () => {
    renderKnowledge({
      items: [makeItem({ id: "a" })],
      inspectorAvailable: false,
    });
    expect(screen.queryByTestId("inspector-btn")).not.toBeInTheDocument();
  });

  it("calls onOpenInspector with the item id when inspector button clicked", () => {
    const onOpenInspector = vi.fn();
    renderKnowledge({
      items: [makeItem({ id: "item-x" })],
      inspectorAvailable: true,
      onOpenInspector,
    });
    fireEvent.click(screen.getByTestId("inspector-btn"));
    expect(onOpenInspector).toHaveBeenCalledWith("item-x");
  });

  it("calls onOpenInspector with the correct id for each item", () => {
    const onOpenInspector = vi.fn();
    const items = [makeItem({ id: "id-1" }), makeItem({ id: "id-2" })];
    renderKnowledge({ items, inspectorAvailable: true, onOpenInspector });
    const buttons = screen.getAllByTestId("inspector-btn");
    fireEvent.click(buttons[1]);
    expect(onOpenInspector).toHaveBeenCalledWith("id-2");
  });
});

// ─── Path button ──────────────────────────────────────────────────────────────

describe("path button", () => {
  it("shows path-btn when pathAvailable=true AND a different item is selected", () => {
    const items = [makeItem({ id: "a" }), makeItem({ id: "b" })];
    // selectedId = "a", so item "b" should show the path button; item "a" should not
    renderKnowledge({ items, pathAvailable: true, selectedId: "a" });
    const pathButtons = screen.getAllByTestId("path-btn");
    expect(pathButtons.length).toBe(1);
  });

  it("does NOT show path-btn when pathAvailable=false", () => {
    const items = [makeItem({ id: "a" }), makeItem({ id: "b" })];
    renderKnowledge({ items, pathAvailable: false, selectedId: "a" });
    expect(screen.queryByTestId("path-btn")).not.toBeInTheDocument();
  });

  it("does NOT show path-btn when pathAvailable=true but selectedId=null", () => {
    renderKnowledge({
      items: [makeItem({ id: "a" })],
      pathAvailable: true,
      selectedId: null,
    });
    expect(screen.queryByTestId("path-btn")).not.toBeInTheDocument();
  });

  it("does NOT show path-btn for the currently selected item itself", () => {
    // Only one item, and it IS the selected item — no path button
    renderKnowledge({
      items: [makeItem({ id: "self" })],
      pathAvailable: true,
      selectedId: "self",
    });
    expect(screen.queryByTestId("path-btn")).not.toBeInTheDocument();
  });

  it("shows path-btn only for items different from selectedId", () => {
    const items = [
      makeItem({ id: "x1" }),
      makeItem({ id: "x2" }),
      makeItem({ id: "x3" }),
    ];
    // selectedId = "x2": path buttons appear on x1 and x3, not x2
    renderKnowledge({ items, pathAvailable: true, selectedId: "x2" });
    const pathButtons = screen.getAllByTestId("path-btn");
    expect(pathButtons.length).toBe(2);
  });

  it("calls onRequestPath with (selectedId, itemId) when path button clicked", () => {
    const onRequestPath = vi.fn();
    const items = [makeItem({ id: "from" }), makeItem({ id: "to" })];
    renderKnowledge({
      items,
      pathAvailable: true,
      selectedId: "from",
      onRequestPath,
    });
    fireEvent.click(screen.getByTestId("path-btn"));
    expect(onRequestPath).toHaveBeenCalledWith("from", "to");
  });
});

// ─── Correction status ────────────────────────────────────────────────────────

describe("correction status", () => {
  it("shows correction-status when correctionAvailable=true", () => {
    renderKnowledge({ correctionAvailable: true });
    expect(screen.getByTestId("correction-status")).toBeInTheDocument();
  });

  it("does NOT show correction-status when correctionAvailable=false", () => {
    renderKnowledge({ correctionAvailable: false });
    expect(screen.queryByTestId("correction-status")).not.toBeInTheDocument();
  });
});

// ─── Loading indicator ────────────────────────────────────────────────────────

describe("loading indicator", () => {
  it("shows loading-indicator when isLoading=true", () => {
    renderKnowledge({ isLoading: true });
    expect(screen.getByTestId("loading-indicator")).toBeInTheDocument();
  });

  it("does NOT show loading-indicator when isLoading=false", () => {
    renderKnowledge({ isLoading: false });
    expect(screen.queryByTestId("loading-indicator")).not.toBeInTheDocument();
  });
});

// ─── Empty state ──────────────────────────────────────────────────────────────

describe("empty state", () => {
  it("shows empty-state when no items and isLoading=false", () => {
    renderKnowledge({ items: [], isLoading: false });
    expect(screen.getByTestId("empty-state")).toBeInTheDocument();
  });

  it("does NOT show empty-state when items are present", () => {
    renderKnowledge({ items: [makeItem()], isLoading: false });
    expect(screen.queryByTestId("empty-state")).not.toBeInTheDocument();
  });

  it("does NOT show empty-state while isLoading=true (even with no items)", () => {
    renderKnowledge({ items: [], isLoading: true });
    expect(screen.queryByTestId("empty-state")).not.toBeInTheDocument();
  });
});

// ─── onSelectItem callback ────────────────────────────────────────────────────

describe("onSelectItem callback", () => {
  it("calls onSelectItem with the item id when select button is clicked", () => {
    const onSelectItem = vi.fn();
    renderKnowledge({
      items: [makeItem({ id: "sel-1" })],
      onSelectItem,
    });
    fireEvent.click(screen.getByTestId("select-btn-sel-1"));
    expect(onSelectItem).toHaveBeenCalledWith("sel-1");
  });

  it("calls onSelectItem with the correct id for each item", () => {
    const onSelectItem = vi.fn();
    const items = [makeItem({ id: "p1" }), makeItem({ id: "p2" })];
    renderKnowledge({ items, onSelectItem });
    fireEvent.click(screen.getByTestId("select-btn-p2"));
    expect(onSelectItem).toHaveBeenCalledWith("p2");
  });
});

// ─── No global hairball ───────────────────────────────────────────────────────

describe("no global hairball", () => {
  it("renders only the items explicitly provided (bounded rendering)", () => {
    const items = [
      makeItem({ id: "n1" }),
      makeItem({ id: "n2" }),
      makeItem({ id: "n3" }),
    ];
    renderKnowledge({ items });
    const listView = screen.getByTestId("list-view");
    const rows = listView.querySelectorAll("[role='listitem'][data-item-id]");
    expect(rows.length).toBe(3);
  });

  it("renders zero items when items array is empty (no synthetic nodes)", () => {
    renderKnowledge({ items: [] });
    const listView = screen.getByTestId("list-view");
    const lis = listView.querySelectorAll("li[data-item-id]");
    expect(lis.length).toBe(0);
  });
});

// ─── All node kinds and authority classes render correctly ────────────────────

describe("item kinds and authority classes", () => {
  const kinds: KnowledgeItem["kind"][] = [
    "entity",
    "memory",
    "evidence",
    "source",
    "aggregate",
  ];
  const authorityClasses: KnowledgeItem["authorityClass"][] = [
    "stored",
    "derived",
    "inferred",
    "navigation",
  ];

  for (const kind of kinds) {
    it(`renders kind="${kind}" with correct data-kind attribute`, () => {
      renderKnowledge({ items: [makeItem({ id: "k1", kind })] });
      expect(document.querySelector(`[data-kind='${kind}']`)).not.toBeNull();
    });
  }

  for (const ac of authorityClasses) {
    it(`renders authorityClass="${ac}" with correct data-authority-class attribute`, () => {
      renderKnowledge({
        items: [makeItem({ id: "ac1", authorityClass: ac })],
      });
      expect(
        document.querySelector(`[data-authority-class='${ac}']`),
      ).not.toBeNull();
    });
  }
});
