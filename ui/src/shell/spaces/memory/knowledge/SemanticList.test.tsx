/**
 * Tests for SemanticList (task 4.3.4).
 *
 * Validates:
 * - Root element renders
 * - Loading state shows indicator with role="status", hides list
 * - Empty items shows empty message
 * - Items rendered for visibleStart..visibleEnd range
 * - Node item renders: kind, authorityClass, displayName, evidenceSummary,
 *   evidenceCount, status, truthState
 * - Edge item renders: kind, directionLabel, source→target labels,
 *   authorityClass, evidenceSummary, truthState
 * - Selected state: data-selected="true", aria-selected="true"
 * - Current state: data-current="true", aria-current="true"
 * - Expanded state: data-expanded="true", aria-expanded="true"
 * - onSelect called with itemId on row click
 * - onExpand called with itemId on double-click or Shift+Enter
 * - Action button renders with data-testid and aria-label
 * - Disabled action: disabled attribute and aria-disabled="true"
 * - Dangerous action: data-dangerous="true"
 * - onAction called with (itemId, actionId) on action button click
 * - onScroll callback wired to scroll container
 * - Virtualization: only items in visibleStart..visibleEnd are rendered
 *
 * Requirements: MGR-006, MGR-014, MGR-023–024, MGR-031;
 *   MGD-026, MGD-030, MGD-046; MG-H01, MG-H04, MG-H10–H12, MG-O05, MG-O25.
 */
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { SemanticList } from "./SemanticList";
import type { SemanticListItem, SemanticListProps } from "./SemanticList";

afterEach(() => cleanup());

// ─── Fixtures ─────────────────────────────────────────────────────────────────

function makeNodeItem(overrides: Partial<SemanticListItem> = {}): SemanticListItem {
  return {
    id: "node-1",
    itemType: "node",
    kind: "entity",
    authorityClass: "personal",
    displayName: "Alice",
    sourceId: null,
    sourceLabel: null,
    targetId: null,
    targetLabel: null,
    directionLabel: null,
    evidenceSummary: "Mentioned in conversation log",
    evidenceCount: 3,
    status: "active",
    truthState: "Current",
    isSelected: false,
    isCurrent: false,
    isExpanded: false,
    authorizedActions: [],
    ...overrides,
  };
}

function makeEdgeItem(overrides: Partial<SemanticListItem> = {}): SemanticListItem {
  return {
    id: "edge-1",
    itemType: "edge",
    kind: "relation",
    authorityClass: "work",
    displayName: null,
    sourceId: "node-a",
    sourceLabel: "Alice",
    targetId: "node-b",
    targetLabel: "Acme Corp",
    directionLabel: "worked-at",
    evidenceSummary: "Resume reference",
    evidenceCount: 1,
    status: "active",
    truthState: "Current",
    isSelected: false,
    isCurrent: false,
    isExpanded: false,
    authorizedActions: [],
    ...overrides,
  };
}

function renderList(partial: Partial<SemanticListProps> = {}) {
  const defaults: SemanticListProps = {
    items: [],
    isLoading: false,
    visibleStart: 0,
    visibleEnd: 10,
    totalHeight: 1000,
    itemHeight: 50,
    onSelect: vi.fn(),
    onExpand: vi.fn(),
    onAction: vi.fn(),
    onScroll: vi.fn(),
  };
  return render(() => <SemanticList {...defaults} {...partial} />);
}

// ─── Root element ─────────────────────────────────────────────────────────────

describe("root element", () => {
  it("renders data-testid=semantic-list-root on the root element", () => {
    renderList();
    expect(screen.getByTestId("semantic-list-root")).toBeInTheDocument();
  });
});

// ─── Loading state ────────────────────────────────────────────────────────────

describe("loading state", () => {
  it("shows loading indicator with role=status when isLoading=true", () => {
    renderList({ isLoading: true });
    const indicator = screen.getByTestId("semantic-list-loading");
    expect(indicator).toBeInTheDocument();
    expect(indicator).toHaveAttribute("role", "status");
  });

  it("loading indicator has aria-live=polite", () => {
    renderList({ isLoading: true });
    expect(screen.getByTestId("semantic-list-loading")).toHaveAttribute(
      "aria-live",
      "polite",
    );
  });

  it("hides the scroll container while loading", () => {
    renderList({
      isLoading: true,
      items: [makeNodeItem()],
    });
    expect(screen.queryByTestId("semantic-list-scroll")).not.toBeInTheDocument();
  });

  it("hides the empty message while loading", () => {
    renderList({ isLoading: true, items: [] });
    expect(screen.queryByTestId("semantic-list-empty")).not.toBeInTheDocument();
  });
});

// ─── Empty state ──────────────────────────────────────────────────────────────

describe("empty state", () => {
  it("shows empty message when items is empty and not loading", () => {
    renderList({ items: [], isLoading: false });
    const empty = screen.getByTestId("semantic-list-empty");
    expect(empty).toBeInTheDocument();
    expect(empty).toHaveTextContent("No items to display");
  });

  it("does not show empty message when loading", () => {
    renderList({ items: [], isLoading: true });
    expect(screen.queryByTestId("semantic-list-empty")).not.toBeInTheDocument();
  });

  it("does not show empty message when items exist", () => {
    renderList({
      items: [makeNodeItem()],
      visibleStart: 0,
      visibleEnd: 1,
    });
    expect(screen.queryByTestId("semantic-list-empty")).not.toBeInTheDocument();
  });
});

// ─── Scroll container ─────────────────────────────────────────────────────────

describe("scroll container", () => {
  it("renders semantic-list-scroll when items are present and not loading", () => {
    renderList({
      items: [makeNodeItem()],
      visibleStart: 0,
      visibleEnd: 1,
    });
    expect(screen.getByTestId("semantic-list-scroll")).toBeInTheDocument();
  });

  it("scroll container has role=grid", () => {
    renderList({
      items: [makeNodeItem()],
      visibleStart: 0,
      visibleEnd: 1,
    });
    expect(screen.getByTestId("semantic-list-scroll")).toHaveAttribute(
      "role",
      "grid",
    );
  });

  it("scroll container has aria-label='Memory items'", () => {
    renderList({
      items: [makeNodeItem()],
      visibleStart: 0,
      visibleEnd: 1,
    });
    expect(screen.getByTestId("semantic-list-scroll")).toHaveAttribute(
      "aria-label",
      "Memory items",
    );
  });

  it("calls onScroll with scrollTop when the container is scrolled", () => {
    const onScroll = vi.fn();
    renderList({
      items: [makeNodeItem()],
      visibleStart: 0,
      visibleEnd: 1,
      onScroll,
    });
    const container = screen.getByTestId("semantic-list-scroll");
    // jsdom doesn't support real scroll; simulate via fireEvent
    Object.defineProperty(container, "scrollTop", { value: 200, configurable: true });
    fireEvent.scroll(container);
    expect(onScroll).toHaveBeenCalledWith(200);
  });
});

// ─── Virtualization ───────────────────────────────────────────────────────────

describe("virtualization", () => {
  const allItems = [
    makeNodeItem({ id: "n0" }),
    makeNodeItem({ id: "n1" }),
    makeNodeItem({ id: "n2" }),
    makeNodeItem({ id: "n3" }),
    makeNodeItem({ id: "n4" }),
  ];

  it("renders only items in the visibleStart..visibleEnd range", () => {
    renderList({ items: allItems, visibleStart: 1, visibleEnd: 3 });
    // n1, n2 should be rendered; n0, n3, n4 should not
    expect(screen.queryByTestId("semantic-list-item-n0")).not.toBeInTheDocument();
    expect(screen.getByTestId("semantic-list-item-n1")).toBeInTheDocument();
    expect(screen.getByTestId("semantic-list-item-n2")).toBeInTheDocument();
    expect(screen.queryByTestId("semantic-list-item-n3")).not.toBeInTheDocument();
    expect(screen.queryByTestId("semantic-list-item-n4")).not.toBeInTheDocument();
  });

  it("renders all items when visibleEnd equals items.length", () => {
    renderList({ items: allItems, visibleStart: 0, visibleEnd: 5 });
    for (let i = 0; i < 5; i++) {
      expect(screen.getByTestId(`semantic-list-item-n${i}`)).toBeInTheDocument();
    }
  });

  it("renders no items when visibleStart === visibleEnd", () => {
    renderList({ items: allItems, visibleStart: 2, visibleEnd: 2 });
    for (let i = 0; i < 5; i++) {
      expect(screen.queryByTestId(`semantic-list-item-n${i}`)).not.toBeInTheDocument();
    }
    // The scroll container itself should still be present (items.length > 0)
    expect(screen.getByTestId("semantic-list-scroll")).toBeInTheDocument();
  });
});

// ─── Node item rendering ──────────────────────────────────────────────────────

describe("node item rendering", () => {
  function renderNode(overrides: Partial<SemanticListItem> = {}) {
    const item = makeNodeItem({ id: "nid", ...overrides });
    renderList({ items: [item], visibleStart: 0, visibleEnd: 1 });
    return item;
  }

  it("renders the row with data-item-type=node", () => {
    renderNode();
    expect(screen.getByTestId("semantic-list-item-nid")).toHaveAttribute(
      "data-item-type",
      "node",
    );
  });

  it("renders kind badge", () => {
    renderNode({ kind: "memory" });
    expect(screen.getByTestId("item-kind-nid")).toHaveTextContent("memory");
  });

  it("renders authorityClass", () => {
    renderNode({ authorityClass: "work" });
    expect(screen.getByTestId("item-authority-nid")).toHaveTextContent("work");
  });

  it("renders displayName", () => {
    renderNode({ displayName: "Bob" });
    expect(screen.getByTestId("item-display-name-nid")).toHaveTextContent("Bob");
  });

  it("renders evidenceSummary", () => {
    renderNode({ evidenceSummary: "From meeting notes" });
    expect(screen.getByTestId("item-evidence-summary-nid")).toHaveTextContent(
      "From meeting notes",
    );
  });

  it("renders evidenceCount", () => {
    renderNode({ evidenceCount: 7 });
    expect(screen.getByTestId("item-evidence-count-nid")).toHaveTextContent("7");
  });

  it("renders status", () => {
    renderNode({ status: "pending" });
    expect(screen.getByTestId("item-status-nid")).toHaveTextContent("pending");
  });

  it("renders truthState with data-truth-state attribute", () => {
    renderNode({ truthState: "Stale" });
    const el = screen.getByTestId("item-truth-state-nid");
    expect(el).toHaveTextContent("Stale");
    expect(el).toHaveAttribute("data-truth-state", "Stale");
  });

  it("does not render edge direction elements for a node", () => {
    renderNode();
    expect(screen.queryByTestId("item-direction-nid")).not.toBeInTheDocument();
    expect(screen.queryByTestId("item-direction-label-nid")).not.toBeInTheDocument();
  });
});

// ─── Edge item rendering ──────────────────────────────────────────────────────

describe("edge item rendering", () => {
  function renderEdge(overrides: Partial<SemanticListItem> = {}) {
    const item = makeEdgeItem({ id: "eid", ...overrides });
    renderList({ items: [item], visibleStart: 0, visibleEnd: 1 });
    return item;
  }

  it("renders the row with data-item-type=edge", () => {
    renderEdge();
    expect(screen.getByTestId("semantic-list-item-eid")).toHaveAttribute(
      "data-item-type",
      "edge",
    );
  });

  it("renders kind badge", () => {
    renderEdge({ kind: "relation" });
    expect(screen.getByTestId("item-kind-eid")).toHaveTextContent("relation");
  });

  it("renders directionLabel", () => {
    renderEdge({ directionLabel: "knows" });
    expect(screen.getByTestId("item-direction-label-eid")).toHaveTextContent("knows");
  });

  it("renders source→target labels", () => {
    renderEdge({ sourceLabel: "Alice", targetLabel: "Bob" });
    const dir = screen.getByTestId("item-direction-eid");
    expect(dir.textContent).toContain("Alice");
    expect(dir.textContent).toContain("→");
    expect(dir.textContent).toContain("Bob");
  });

  it("renders authorityClass for edge", () => {
    renderEdge({ authorityClass: "public" });
    expect(screen.getByTestId("item-authority-eid")).toHaveTextContent("public");
  });

  it("renders evidenceSummary for edge", () => {
    renderEdge({ evidenceSummary: "Social graph import" });
    expect(screen.getByTestId("item-evidence-summary-eid")).toHaveTextContent(
      "Social graph import",
    );
  });

  it("renders truthState for edge", () => {
    renderEdge({ truthState: "Contradicted" });
    const el = screen.getByTestId("item-truth-state-eid");
    expect(el).toHaveTextContent("Contradicted");
    expect(el).toHaveAttribute("data-truth-state", "Contradicted");
  });

  it("does not render displayName for an edge", () => {
    renderEdge({ displayName: null });
    expect(screen.queryByTestId("item-display-name-eid")).not.toBeInTheDocument();
  });
});

// ─── Row state attributes ─────────────────────────────────────────────────────

describe("row state attributes", () => {
  it("data-selected=true and aria-selected=true when item is selected", () => {
    renderList({
      items: [makeNodeItem({ id: "sel", isSelected: true })],
      visibleStart: 0,
      visibleEnd: 1,
    });
    const row = screen.getByTestId("semantic-list-item-sel");
    expect(row).toHaveAttribute("data-selected", "true");
    expect(row).toHaveAttribute("aria-selected", "true");
  });

  it("data-selected=false and aria-selected=false when item is not selected", () => {
    renderList({
      items: [makeNodeItem({ id: "notsel", isSelected: false })],
      visibleStart: 0,
      visibleEnd: 1,
    });
    const row = screen.getByTestId("semantic-list-item-notsel");
    expect(row).toHaveAttribute("data-selected", "false");
    expect(row).toHaveAttribute("aria-selected", "false");
  });

  it("data-current=true and aria-current=true when item is current", () => {
    renderList({
      items: [makeNodeItem({ id: "cur", isCurrent: true })],
      visibleStart: 0,
      visibleEnd: 1,
    });
    const row = screen.getByTestId("semantic-list-item-cur");
    expect(row).toHaveAttribute("data-current", "true");
    expect(row).toHaveAttribute("aria-current", "true");
  });

  it("data-current=false when item is not current", () => {
    renderList({
      items: [makeNodeItem({ id: "notcur", isCurrent: false })],
      visibleStart: 0,
      visibleEnd: 1,
    });
    const row = screen.getByTestId("semantic-list-item-notcur");
    expect(row).toHaveAttribute("data-current", "false");
  });

  it("data-expanded=true and aria-expanded=true when item is expanded", () => {
    renderList({
      items: [makeNodeItem({ id: "exp", isExpanded: true })],
      visibleStart: 0,
      visibleEnd: 1,
    });
    const row = screen.getByTestId("semantic-list-item-exp");
    expect(row).toHaveAttribute("data-expanded", "true");
    expect(row).toHaveAttribute("aria-expanded", "true");
  });

  it("data-expanded=false when item is not expanded", () => {
    renderList({
      items: [makeNodeItem({ id: "notexp", isExpanded: false })],
      visibleStart: 0,
      visibleEnd: 1,
    });
    const row = screen.getByTestId("semantic-list-item-notexp");
    expect(row).toHaveAttribute("data-expanded", "false");
  });

  it("each row has role=row", () => {
    renderList({
      items: [makeNodeItem({ id: "r1" })],
      visibleStart: 0,
      visibleEnd: 1,
    });
    expect(screen.getByTestId("semantic-list-item-r1")).toHaveAttribute(
      "role",
      "row",
    );
  });

  it("each row has tabIndex=0", () => {
    renderList({
      items: [makeNodeItem({ id: "t1" })],
      visibleStart: 0,
      visibleEnd: 1,
    });
    expect(screen.getByTestId("semantic-list-item-t1")).toHaveAttribute(
      "tabindex",
      "0",
    );
  });
});

// ─── Row interaction callbacks ────────────────────────────────────────────────

describe("row interaction callbacks", () => {
  it("calls onSelect with itemId on row click", () => {
    const onSelect = vi.fn();
    renderList({
      items: [makeNodeItem({ id: "click-me" })],
      visibleStart: 0,
      visibleEnd: 1,
      onSelect,
    });
    fireEvent.click(screen.getByTestId("semantic-list-item-click-me"));
    expect(onSelect).toHaveBeenCalledWith("click-me");
  });

  it("calls onExpand with itemId on double-click", () => {
    const onExpand = vi.fn();
    renderList({
      items: [makeNodeItem({ id: "dbl-me" })],
      visibleStart: 0,
      visibleEnd: 1,
      onExpand,
    });
    fireEvent.dblClick(screen.getByTestId("semantic-list-item-dbl-me"));
    expect(onExpand).toHaveBeenCalledWith("dbl-me");
  });

  it("calls onExpand with itemId on Shift+Enter", () => {
    const onExpand = vi.fn();
    renderList({
      items: [makeNodeItem({ id: "shift-enter" })],
      visibleStart: 0,
      visibleEnd: 1,
      onExpand,
    });
    fireEvent.keyDown(screen.getByTestId("semantic-list-item-shift-enter"), {
      key: "Enter",
      shiftKey: true,
    });
    expect(onExpand).toHaveBeenCalledWith("shift-enter");
  });

  it("does not call onExpand on plain Enter", () => {
    const onExpand = vi.fn();
    renderList({
      items: [makeNodeItem({ id: "plain-enter" })],
      visibleStart: 0,
      visibleEnd: 1,
      onExpand,
    });
    fireEvent.keyDown(screen.getByTestId("semantic-list-item-plain-enter"), {
      key: "Enter",
      shiftKey: false,
    });
    expect(onExpand).not.toHaveBeenCalled();
  });
});

// ─── Action rendering ─────────────────────────────────────────────────────────

describe("action button rendering", () => {
  function makeItemWithActions(itemId: string): SemanticListItem {
    return makeNodeItem({
      id: itemId,
      authorizedActions: [
        { id: "inspect", label: "Inspect", isEnabled: true, isDangerous: false },
        { id: "delete", label: "Delete", isEnabled: false, isDangerous: true },
        { id: "correct", label: "Correct", isEnabled: true, isDangerous: false },
      ],
    });
  }

  it("renders action buttons with data-testid=action-{itemId}-{actionId}", () => {
    renderList({
      items: [makeItemWithActions("ai1")],
      visibleStart: 0,
      visibleEnd: 1,
    });
    expect(screen.getByTestId("action-ai1-inspect")).toBeInTheDocument();
    expect(screen.getByTestId("action-ai1-delete")).toBeInTheDocument();
    expect(screen.getByTestId("action-ai1-correct")).toBeInTheDocument();
  });

  it("action button has aria-label from the label field", () => {
    renderList({
      items: [makeItemWithActions("ai2")],
      visibleStart: 0,
      visibleEnd: 1,
    });
    expect(screen.getByTestId("action-ai2-inspect")).toHaveAttribute(
      "aria-label",
      "Inspect",
    );
  });

  it("disabled action has disabled attribute", () => {
    renderList({
      items: [makeItemWithActions("ai3")],
      visibleStart: 0,
      visibleEnd: 1,
    });
    expect(screen.getByTestId("action-ai3-delete")).toBeDisabled();
  });

  it("disabled action has aria-disabled=true", () => {
    renderList({
      items: [makeItemWithActions("ai4")],
      visibleStart: 0,
      visibleEnd: 1,
    });
    expect(screen.getByTestId("action-ai4-delete")).toHaveAttribute(
      "aria-disabled",
      "true",
    );
  });

  it("enabled action does not have aria-disabled", () => {
    renderList({
      items: [makeItemWithActions("ai5")],
      visibleStart: 0,
      visibleEnd: 1,
    });
    const btn = screen.getByTestId("action-ai5-inspect");
    expect(btn).not.toHaveAttribute("aria-disabled");
  });

  it("dangerous action has data-dangerous=true", () => {
    renderList({
      items: [makeItemWithActions("ai6")],
      visibleStart: 0,
      visibleEnd: 1,
    });
    expect(screen.getByTestId("action-ai6-delete")).toHaveAttribute(
      "data-dangerous",
      "true",
    );
  });

  it("non-dangerous action has data-dangerous=false", () => {
    renderList({
      items: [makeItemWithActions("ai7")],
      visibleStart: 0,
      visibleEnd: 1,
    });
    expect(screen.getByTestId("action-ai7-inspect")).toHaveAttribute(
      "data-dangerous",
      "false",
    );
  });

  it("calls onAction with (itemId, actionId) when enabled action button is clicked", () => {
    const onAction = vi.fn();
    renderList({
      items: [makeItemWithActions("ai8")],
      visibleStart: 0,
      visibleEnd: 1,
      onAction,
    });
    fireEvent.click(screen.getByTestId("action-ai8-inspect"));
    expect(onAction).toHaveBeenCalledWith("ai8", "inspect");
  });

  it("does not call onAction when disabled action button is clicked", () => {
    const onAction = vi.fn();
    renderList({
      items: [makeItemWithActions("ai9")],
      visibleStart: 0,
      visibleEnd: 1,
      onAction,
    });
    // Disabled button click — browser prevents the click, but even if it fires
    // via fireEvent, our handler checks isEnabled.
    fireEvent.click(screen.getByTestId("action-ai9-delete"));
    expect(onAction).not.toHaveBeenCalled();
  });

  it("action click does not also fire onSelect", () => {
    const onSelect = vi.fn();
    const onAction = vi.fn();
    renderList({
      items: [makeItemWithActions("ai10")],
      visibleStart: 0,
      visibleEnd: 1,
      onSelect,
      onAction,
    });
    fireEvent.click(screen.getByTestId("action-ai10-inspect"));
    expect(onAction).toHaveBeenCalledWith("ai10", "inspect");
    // stopPropagation should prevent onSelect from firing
    expect(onSelect).not.toHaveBeenCalled();
  });
});

// ─── Mixed items ──────────────────────────────────────────────────────────────

describe("mixed node and edge items", () => {
  it("renders both node and edge items in one list", () => {
    const items: SemanticListItem[] = [
      makeNodeItem({ id: "n-mix" }),
      makeEdgeItem({ id: "e-mix" }),
    ];
    renderList({ items, visibleStart: 0, visibleEnd: 2 });
    expect(screen.getByTestId("semantic-list-item-n-mix")).toHaveAttribute(
      "data-item-type",
      "node",
    );
    expect(screen.getByTestId("semantic-list-item-e-mix")).toHaveAttribute(
      "data-item-type",
      "edge",
    );
  });
});

// ─── Absolute positioning ─────────────────────────────────────────────────────

describe("absolute row positioning", () => {
  it("positions row at top = absoluteIndex * itemHeight", () => {
    const items = [
      makeNodeItem({ id: "pos0" }),
      makeNodeItem({ id: "pos1" }),
      makeNodeItem({ id: "pos2" }),
    ];
    renderList({ items, visibleStart: 1, visibleEnd: 3, itemHeight: 60 });
    // Item at absolute index 1 → top: 60px
    const row1 = screen.getByTestId("semantic-list-item-pos1");
    expect(row1).toHaveStyle({ top: "60px" });
    // Item at absolute index 2 → top: 120px
    const row2 = screen.getByTestId("semantic-list-item-pos2");
    expect(row2).toHaveStyle({ top: "120px" });
  });
});
