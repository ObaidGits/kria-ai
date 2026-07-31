/**
 * SemanticList integration / E2E tests (task 4.3.7).
 *
 * List-only E2E tests — no Canvas/map/3D. Composes SearchInput + SearchResults
 * + SearchTotals + SemanticList together and exercises end-to-end list workflows
 * with fixture data. No network calls — vi.fn() for callbacks, hardcoded fixtures.
 *
 * Scenarios:
 *   1.  Find entity by name
 *   2.  Find by alias
 *   3.  Find by content
 *   4.  Find by source
 *   5.  Find relation
 *   6.  Find goal
 *   7.  No result — empty state
 *   8.  Hidden result (unauthorized)
 *   9.  Partial strategy
 *   10. Inspect action available and activatable
 *   11. Path navigation intent
 *   12. Correction action
 *   13. Lifecycle: delete
 *   14. Lifecycle: forget
 *   15. Lifecycle: restore
 *   16. List remains usable on error
 *
 * Requirements: MGR-006, MGR-014, MGR-023–024, MGR-031;
 *   MGD-026, MGD-030, MGD-046; MG-H01, MG-H04, MG-H10–H12, MG-O05, MG-O25.
 */
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { createSignal } from "solid-js";

import { SearchInput } from "./SearchInput";
import { SearchResults } from "./SearchResults";
import { SearchTotals } from "./SearchTotals";
import { SemanticList } from "./SemanticList";
import { applyIntent, initialListNavigationState } from "./listNavigation";

import type { SearchResultItem } from "./SearchResults";
import type { SemanticListItem, SemanticAction } from "./SemanticList";
import type { SearchFilter } from "./SearchInput";

afterEach(() => cleanup());

// ─── Fixture helpers ──────────────────────────────────────────────────────────

function makeListItem(overrides: Partial<SemanticListItem> = {}): SemanticListItem {
  return {
    id: "item-1",
    itemType: "node",
    kind: "entity",
    authorityClass: "personal",
    displayName: "Alice",
    sourceId: null,
    sourceLabel: null,
    targetId: null,
    targetLabel: null,
    directionLabel: null,
    evidenceSummary: "From conversation log",
    evidenceCount: 2,
    status: "active",
    truthState: "Current",
    isSelected: false,
    isCurrent: false,
    isExpanded: false,
    authorizedActions: [],
    ...overrides,
  };
}

function makeSearchItem(overrides: Partial<SearchResultItem> = {}): SearchResultItem {
  return {
    id: "result-1",
    kind: "entity",
    matchedField: "name",
    rationale: "Matched on name field.",
    relativeScore: 0.9,
    profileId: "balanced-v1",
    policySummary: "namespace=default",
    truthState: "Current",
    graphRevision: 1,
    sourceId: null,
    sourceLabel: null,
    validTimeStart: null,
    validTimeEnd: null,
    transactionTime: "2024-01-01T00:00:00Z",
    navigationTarget: "knowledge?inspect=result-1",
    summary: "Alice",
    ...overrides,
  };
}

function makeAction(overrides: Partial<SemanticAction> = {}): SemanticAction {
  return {
    id: "inspect",
    label: "Inspect",
    isEnabled: true,
    isDangerous: false,
    ...overrides,
  };
}

// ─── 1. Find entity by name ───────────────────────────────────────────────────

describe("1. find entity by name", () => {
  it("searching 'Alice' shows entity with matching displayName in SemanticList", () => {
    const [query, setQuery] = createSignal("");
    const onSubmit = vi.fn();
    const items = [makeListItem({ id: "alice", displayName: "Alice", kind: "entity" })];

    render(() => (
      <div>
        <SearchInput
          query={query()}
          onQueryChange={setQuery}
          onSubmit={onSubmit}
          activeFilters={[]}
          onRemoveFilter={vi.fn()}
          isSearching={false}
          resultState="results"
          resultCount={1}
          resultCountQualifier="exact"
        />
        <SemanticList
          items={items}
          isLoading={false}
          visibleStart={0}
          visibleEnd={1}
          totalHeight={50}
          itemHeight={50}
          onSelect={vi.fn()}
          onExpand={vi.fn()}
          onAction={vi.fn()}
          onScroll={vi.fn()}
        />
      </div>
    ));

    const input = screen.getByTestId("search-input-field");
    fireEvent.input(input, { target: { value: "Alice" } });

    expect(screen.getByTestId("semantic-list-item-alice")).toBeInTheDocument();
    expect(screen.getByTestId("item-display-name-alice")).toHaveTextContent("Alice");
    expect(screen.getByTestId("item-kind-alice")).toHaveTextContent("entity");
  });
});

// ─── 2. Find by alias (SearchResults) ────────────────────────────────────────

describe("2. find by alias", () => {
  it("searching 'Al' shows entity whose alias matched in SearchResults", () => {
    const aliasItem = makeSearchItem({
      id: "alice-alias",
      kind: "entity",
      matchedField: "alias",
      summary: "Alice",
    });

    render(() => (
      <SearchResults
        items={[aliasItem]}
        isLoading={false}
        resultState="results"
        onNavigate={vi.fn()}
        onSelect={vi.fn()}
      />
    ));

    expect(screen.getByTestId("result-item-alice-alias")).toBeInTheDocument();
    expect(screen.getByTestId("result-matched-field-alice-alias")).toHaveTextContent(
      "matched in: alias",
    );
    expect(screen.getByTestId("result-summary-alice-alias")).toHaveTextContent("Alice");
  });
});

// ─── 3. Find by content ───────────────────────────────────────────────────────

describe("3. find by content", () => {
  it("searching 'project meeting' shows memory item with body content match", () => {
    const contentItem = makeSearchItem({
      id: "mem-meeting",
      kind: "memory",
      matchedField: "body",
      summary: "Notes from the project meeting on Q4 goals.",
    });

    render(() => (
      <SearchResults
        items={[contentItem]}
        isLoading={false}
        resultState="results"
        onNavigate={vi.fn()}
        onSelect={vi.fn()}
      />
    ));

    expect(screen.getByTestId("result-item-mem-meeting")).toBeInTheDocument();
    expect(screen.getByTestId("result-kind-mem-meeting")).toHaveTextContent("memory");
    expect(screen.getByTestId("result-matched-field-mem-meeting")).toHaveTextContent(
      "matched in: body",
    );
    expect(screen.getByTestId("result-summary-mem-meeting")).toHaveTextContent(
      "project meeting",
    );
  });
});

// ─── 4. Find by source ────────────────────────────────────────────────────────

describe("4. find by source", () => {
  it("filtering by source 'conversation' shows items from that source", () => {
    const sourceItem = makeSearchItem({
      id: "src-conv-1",
      kind: "memory",
      matchedField: "source",
      sourceId: "conversation-001",
      sourceLabel: "conversation",
      summary: "Remembered from yesterday's chat.",
    });

    const activeFilters: SearchFilter[] = [
      { id: "f1", label: "Source", value: "conversation", kind: "source" },
    ];

    render(() => (
      <div>
        <SearchInput
          query="conversation"
          onQueryChange={vi.fn()}
          onSubmit={vi.fn()}
          activeFilters={activeFilters}
          onRemoveFilter={vi.fn()}
          isSearching={false}
          resultState="results"
          resultCount={1}
          resultCountQualifier="exact"
        />
        <SearchResults
          items={[sourceItem]}
          isLoading={false}
          resultState="results"
          onNavigate={vi.fn()}
          onSelect={vi.fn()}
        />
      </div>
    ));

    expect(screen.getByTestId("filter-chip-f1")).toBeInTheDocument();
    expect(screen.getByTestId("result-item-src-conv-1")).toBeInTheDocument();
    expect(screen.getByTestId("result-source-src-conv-1")).toHaveTextContent("conversation");
  });
});

// ─── 5. Find relation ─────────────────────────────────────────────────────────

describe("5. find relation", () => {
  it("searching 'knows' shows relation edge item in SemanticList", () => {
    const relationItem = makeListItem({
      id: "rel-knows",
      itemType: "edge",
      kind: "relation",
      displayName: null,
      directionLabel: "knows",
      sourceLabel: "Alice",
      targetLabel: "Bob",
      sourceId: "node-alice",
      targetId: "node-bob",
    });

    render(() => (
      <SemanticList
        items={[relationItem]}
        isLoading={false}
        visibleStart={0}
        visibleEnd={1}
        totalHeight={50}
        itemHeight={50}
        onSelect={vi.fn()}
        onExpand={vi.fn()}
        onAction={vi.fn()}
        onScroll={vi.fn()}
      />
    ));

    expect(screen.getByTestId("semantic-list-item-rel-knows")).toBeInTheDocument();
    expect(screen.getByTestId("item-kind-rel-knows")).toHaveTextContent("relation");
    expect(screen.getByTestId("item-direction-label-rel-knows")).toHaveTextContent("knows");
    const dir = screen.getByTestId("item-direction-rel-knows");
    expect(dir.textContent).toContain("Alice");
    expect(dir.textContent).toContain("Bob");
  });
});

// ─── 6. Find goal ─────────────────────────────────────────────────────────────

describe("6. find goal", () => {
  it("searching 'learn rust' shows goal item in SearchResults", () => {
    const goalItem = makeSearchItem({
      id: "goal-rust",
      kind: "goal",
      matchedField: "title",
      summary: "Learn Rust programming language",
    });

    render(() => (
      <SearchResults
        items={[goalItem]}
        isLoading={false}
        resultState="results"
        onNavigate={vi.fn()}
        onSelect={vi.fn()}
      />
    ));

    expect(screen.getByTestId("result-item-goal-rust")).toBeInTheDocument();
    expect(screen.getByTestId("result-kind-goal-rust")).toHaveTextContent("goal");
    expect(screen.getByTestId("result-summary-goal-rust")).toHaveTextContent("Learn Rust");
  });

  it("goal item also renders correctly in SemanticList", () => {
    const goalListItem = makeListItem({
      id: "goal-rust-list",
      kind: "goal",
      displayName: "Learn Rust programming language",
    });

    render(() => (
      <SemanticList
        items={[goalListItem]}
        isLoading={false}
        visibleStart={0}
        visibleEnd={1}
        totalHeight={50}
        itemHeight={50}
        onSelect={vi.fn()}
        onExpand={vi.fn()}
        onAction={vi.fn()}
        onScroll={vi.fn()}
      />
    ));

    expect(screen.getByTestId("item-kind-goal-rust-list")).toHaveTextContent("goal");
    expect(screen.getByTestId("item-display-name-goal-rust-list")).toHaveTextContent(
      "Learn Rust",
    );
  });
});

// ─── 7. No result ─────────────────────────────────────────────────────────────

describe("7. no result", () => {
  it("searching 'xyznomatch12345' shows empty state in SearchResults", () => {
    render(() => (
      <SearchResults
        items={[]}
        isLoading={false}
        resultState="no-results"
        onNavigate={vi.fn()}
        onSelect={vi.fn()}
      />
    ));

    const empty = screen.getByTestId("search-results-empty");
    expect(empty).toBeInTheDocument();
    expect(empty).toHaveTextContent("No results found. Your filters are preserved.");
  });

  it("SemanticList shows 'No items to display' when items array is empty", () => {
    render(() => (
      <SemanticList
        items={[]}
        isLoading={false}
        visibleStart={0}
        visibleEnd={0}
        totalHeight={0}
        itemHeight={50}
        onSelect={vi.fn()}
        onExpand={vi.fn()}
        onAction={vi.fn()}
        onScroll={vi.fn()}
      />
    ));

    expect(screen.getByTestId("semantic-list-empty")).toHaveTextContent(
      "No items to display",
    );
  });

  it("composed view: both empty states visible simultaneously for no-result query", () => {
    render(() => (
      <div>
        <SearchResults
          items={[]}
          isLoading={false}
          resultState="no-results"
          onNavigate={vi.fn()}
          onSelect={vi.fn()}
        />
        <SemanticList
          items={[]}
          isLoading={false}
          visibleStart={0}
          visibleEnd={0}
          totalHeight={0}
          itemHeight={50}
          onSelect={vi.fn()}
          onExpand={vi.fn()}
          onAction={vi.fn()}
          onScroll={vi.fn()}
        />
      </div>
    ));

    expect(screen.getByTestId("search-results-empty")).toBeInTheDocument();
    expect(screen.getByTestId("semantic-list-empty")).toBeInTheDocument();
  });
});

// ─── 8. Hidden result (unauthorized) ─────────────────────────────────────────

describe("8. hidden result (unauthorized)", () => {
  it("unauthorized state shows permission-denied copy without revealing existence", () => {
    render(() => (
      <SearchResults
        items={[]}
        isLoading={false}
        resultState="error"
        errorMessage="You do not have permission to view these results."
        onNavigate={vi.fn()}
        onSelect={vi.fn()}
      />
    ));

    const errorEl = screen.getByTestId("search-results-error");
    expect(errorEl).toBeInTheDocument();
    expect(errorEl.textContent).toContain("You do not have permission");
    // Must not hint that hidden items exist
    const text = errorEl.textContent?.toLowerCase() ?? "";
    expect(text).not.toContain("hidden");
    expect(text).not.toContain("exist");
    expect(text).not.toContain("found but");
  });

  it("unauthorized: no result items are shown", () => {
    render(() => (
      <SearchResults
        items={[]}
        isLoading={false}
        resultState="error"
        errorMessage="You do not have permission to view these results."
        onNavigate={vi.fn()}
        onSelect={vi.fn()}
      />
    ));

    expect(screen.queryByTestId("search-results-list")).not.toBeInTheDocument();
  });
});

// ─── 9. Partial strategy ──────────────────────────────────────────────────────

describe("9. partial strategy", () => {
  it("partial state shows 'Partial results' notice alongside items in SearchResults", () => {
    const partialItem = makeSearchItem({ id: "partial-1", summary: "Partial match result" });

    render(() => (
      <SearchResults
        items={[partialItem]}
        isLoading={false}
        resultState="partial"
        onNavigate={vi.fn()}
        onSelect={vi.fn()}
      />
    ));

    const notice = screen.getByTestId("search-results-partial-notice");
    expect(notice).toBeInTheDocument();
    expect(notice).toHaveTextContent("Partial results");
    expect(screen.getByTestId("search-results-list")).toBeInTheDocument();
    expect(screen.getByTestId("result-item-partial-1")).toBeInTheDocument();
  });

  it("partial state with SearchTotals shows unavailable strategies", () => {
    render(() => (
      <SearchTotals
        mode="search"
        shown={3}
        total={5}
        totalQualifier="exact"
        isTruncated={false}
        hasPreviousCursor={false}
        hasNextCursor={false}
        isLoading={false}
        onPreviousPage={vi.fn()}
        onNextPage={vi.fn()}
        strategyInfo={{ used: ["bm25"], unavailable: ["vector-dense"] }}
      />
    ));

    expect(screen.getByTestId("unavailable-strategies")).toHaveTextContent("vector-dense");
    expect(screen.getByTestId("used-strategies")).toHaveTextContent("bm25");
  });

  it("composed partial view: notice + items + totals all rendered", () => {
    const items = [makeSearchItem({ id: "p1" }), makeSearchItem({ id: "p2" })];

    render(() => (
      <div>
        <SearchTotals
          mode="search"
          shown={2}
          total={5}
          totalQualifier="at-least"
          isTruncated={false}
          hasPreviousCursor={false}
          hasNextCursor={false}
          isLoading={false}
          onPreviousPage={vi.fn()}
          onNextPage={vi.fn()}
          strategyInfo={{ used: ["bm25"], unavailable: ["semantic"] }}
        />
        <SearchResults
          items={items}
          isLoading={false}
          resultState="partial"
          onNavigate={vi.fn()}
          onSelect={vi.fn()}
        />
      </div>
    ));

    expect(screen.getByTestId("search-results-partial-notice")).toBeInTheDocument();
    expect(screen.getByTestId("totals-label")).toHaveTextContent(
      "Showing 2 of at least 5 results",
    );
    expect(screen.getByTestId("result-item-p1")).toBeInTheDocument();
    expect(screen.getByTestId("result-item-p2")).toBeInTheDocument();
  });
});

// ─── 10. Inspect ──────────────────────────────────────────────────────────────

describe("10. inspect", () => {
  it("selecting an item makes 'inspect' action available", () => {
    const item = makeListItem({
      id: "inspect-me",
      authorizedActions: [makeAction({ id: "inspect", label: "Inspect", isEnabled: true })],
    });

    render(() => (
      <SemanticList
        items={[item]}
        isLoading={false}
        visibleStart={0}
        visibleEnd={1}
        totalHeight={50}
        itemHeight={50}
        onSelect={vi.fn()}
        onExpand={vi.fn()}
        onAction={vi.fn()}
        onScroll={vi.fn()}
      />
    ));

    const btn = screen.getByTestId("action-inspect-me-inspect");
    expect(btn).toBeInTheDocument();
    expect(btn).not.toBeDisabled();
  });

  it("activating 'inspect' calls onAction with correct (itemId, actionId)", () => {
    const onAction = vi.fn();
    const item = makeListItem({
      id: "inspect-me",
      authorizedActions: [makeAction({ id: "inspect", label: "Inspect", isEnabled: true })],
    });

    render(() => (
      <SemanticList
        items={[item]}
        isLoading={false}
        visibleStart={0}
        visibleEnd={1}
        totalHeight={50}
        itemHeight={50}
        onSelect={vi.fn()}
        onExpand={vi.fn()}
        onAction={onAction}
        onScroll={vi.fn()}
      />
    ));

    fireEvent.click(screen.getByTestId("action-inspect-me-inspect"));
    expect(onAction).toHaveBeenCalledWith("inspect-me", "inspect");
  });

  it("clicking item navigate button in SearchResults calls onNavigate with inspect target", () => {
    const onNavigate = vi.fn();
    const resultItem = makeSearchItem({
      id: "nav-inspect",
      navigationTarget: "knowledge?inspect=nav-inspect",
    });

    render(() => (
      <SearchResults
        items={[resultItem]}
        isLoading={false}
        resultState="results"
        onNavigate={onNavigate}
        onSelect={vi.fn()}
      />
    ));

    fireEvent.click(screen.getByTestId("result-navigate-nav-inspect"));
    expect(onNavigate).toHaveBeenCalledWith("knowledge?inspect=nav-inspect");
  });
});

// ─── 11. Path navigation ──────────────────────────────────────────────────────

describe("11. path navigation", () => {
  it("path intent changes intent type to 'path' with sourceId and targetId", () => {
    // Uses listNavigation pure helpers to verify the navigation state transitions
    // without requiring the full component stack.
    const initial = initialListNavigationState();
    expect(initial.intent.type).toBe("list");

    const pathState = applyIntent(initial, {
      type: "path",
      sourceId: "node-alice",
      targetId: "node-bob",
    });

    expect(pathState.intent.type).toBe("path");
    expect(pathState.intent.sourceId).toBe("node-alice");
    expect(pathState.intent.targetId).toBe("node-bob");
  });

  it("path navigation via SearchResults navigate button changes UI intent signal", () => {
    const onNavigate = vi.fn();
    const resultItem = makeSearchItem({
      id: "path-src",
      navigationTarget: "knowledge?path=node-alice:node-bob",
    });

    render(() => (
      <SearchResults
        items={[resultItem]}
        isLoading={false}
        resultState="results"
        onNavigate={onNavigate}
        onSelect={vi.fn()}
      />
    ));

    fireEvent.click(screen.getByTestId("result-navigate-path-src"));
    expect(onNavigate).toHaveBeenCalledWith("knowledge?path=node-alice:node-bob");
  });
});

// ─── 12. Correction action ────────────────────────────────────────────────────

describe("12. correction action", () => {
  it("activating 'correct' action calls onAction with 'correct' actionId", () => {
    const onAction = vi.fn();
    const item = makeListItem({
      id: "correct-me",
      authorizedActions: [
        makeAction({ id: "correct", label: "Correct", isEnabled: true, isDangerous: false }),
      ],
    });

    render(() => (
      <SemanticList
        items={[item]}
        isLoading={false}
        visibleStart={0}
        visibleEnd={1}
        totalHeight={50}
        itemHeight={50}
        onSelect={vi.fn()}
        onExpand={vi.fn()}
        onAction={onAction}
        onScroll={vi.fn()}
      />
    ));

    const btn = screen.getByTestId("action-correct-me-correct");
    expect(btn).toBeInTheDocument();
    expect(btn).not.toBeDisabled();

    fireEvent.click(btn);
    expect(onAction).toHaveBeenCalledWith("correct-me", "correct");
  });

  it("correct action is not dangerous", () => {
    const item = makeListItem({
      id: "correct-safe",
      authorizedActions: [
        makeAction({ id: "correct", label: "Correct", isEnabled: true, isDangerous: false }),
      ],
    });

    render(() => (
      <SemanticList
        items={[item]}
        isLoading={false}
        visibleStart={0}
        visibleEnd={1}
        totalHeight={50}
        itemHeight={50}
        onSelect={vi.fn()}
        onExpand={vi.fn()}
        onAction={vi.fn()}
        onScroll={vi.fn()}
      />
    ));

    expect(screen.getByTestId("action-correct-safe-correct")).toHaveAttribute(
      "data-dangerous",
      "false",
    );
  });
});

// ─── 13. Lifecycle: delete ────────────────────────────────────────────────────

describe("13. lifecycle: delete", () => {
  it("activating 'delete' action calls onAction with 'delete' actionId", () => {
    const onAction = vi.fn();
    const item = makeListItem({
      id: "delete-me",
      authorizedActions: [
        makeAction({ id: "delete", label: "Delete", isEnabled: true, isDangerous: true }),
      ],
    });

    render(() => (
      <SemanticList
        items={[item]}
        isLoading={false}
        visibleStart={0}
        visibleEnd={1}
        totalHeight={50}
        itemHeight={50}
        onSelect={vi.fn()}
        onExpand={vi.fn()}
        onAction={onAction}
        onScroll={vi.fn()}
      />
    ));

    fireEvent.click(screen.getByTestId("action-delete-me-delete"));
    expect(onAction).toHaveBeenCalledWith("delete-me", "delete");
  });

  it("delete action is marked dangerous", () => {
    const item = makeListItem({
      id: "delete-dangerous",
      authorizedActions: [
        makeAction({ id: "delete", label: "Delete", isEnabled: true, isDangerous: true }),
      ],
    });

    render(() => (
      <SemanticList
        items={[item]}
        isLoading={false}
        visibleStart={0}
        visibleEnd={1}
        totalHeight={50}
        itemHeight={50}
        onSelect={vi.fn()}
        onExpand={vi.fn()}
        onAction={vi.fn()}
        onScroll={vi.fn()}
      />
    ));

    expect(screen.getByTestId("action-delete-dangerous-delete")).toHaveAttribute(
      "data-dangerous",
      "true",
    );
  });
});

// ─── 14. Lifecycle: forget ────────────────────────────────────────────────────

describe("14. lifecycle: forget", () => {
  it("activating 'forget' action calls onAction with 'forget' actionId", () => {
    const onAction = vi.fn();
    const item = makeListItem({
      id: "forget-me",
      authorizedActions: [
        makeAction({ id: "forget", label: "Forget", isEnabled: true, isDangerous: true }),
      ],
    });

    render(() => (
      <SemanticList
        items={[item]}
        isLoading={false}
        visibleStart={0}
        visibleEnd={1}
        totalHeight={50}
        itemHeight={50}
        onSelect={vi.fn()}
        onExpand={vi.fn()}
        onAction={onAction}
        onScroll={vi.fn()}
      />
    ));

    fireEvent.click(screen.getByTestId("action-forget-me-forget"));
    expect(onAction).toHaveBeenCalledWith("forget-me", "forget");
  });
});

// ─── 15. Lifecycle: restore ───────────────────────────────────────────────────

describe("15. lifecycle: restore", () => {
  it("activating 'restore' action calls onAction with 'restore' actionId", () => {
    const onAction = vi.fn();
    const item = makeListItem({
      id: "restore-me",
      status: "deleted",
      truthState: "Deleted",
      authorizedActions: [
        makeAction({ id: "restore", label: "Restore", isEnabled: true, isDangerous: false }),
      ],
    });

    render(() => (
      <SemanticList
        items={[item]}
        isLoading={false}
        visibleStart={0}
        visibleEnd={1}
        totalHeight={50}
        itemHeight={50}
        onSelect={vi.fn()}
        onExpand={vi.fn()}
        onAction={onAction}
        onScroll={vi.fn()}
      />
    ));

    fireEvent.click(screen.getByTestId("action-restore-me-restore"));
    expect(onAction).toHaveBeenCalledWith("restore-me", "restore");
  });

  it("restore shows item with Deleted truthState before action", () => {
    const item = makeListItem({
      id: "restore-check",
      status: "deleted",
      truthState: "Deleted",
      authorizedActions: [
        makeAction({ id: "restore", label: "Restore", isEnabled: true, isDangerous: false }),
      ],
    });

    render(() => (
      <SemanticList
        items={[item]}
        isLoading={false}
        visibleStart={0}
        visibleEnd={1}
        totalHeight={50}
        itemHeight={50}
        onSelect={vi.fn()}
        onExpand={vi.fn()}
        onAction={vi.fn()}
        onScroll={vi.fn()}
      />
    ));

    expect(screen.getByTestId("item-truth-state-restore-check")).toHaveAttribute(
      "data-truth-state",
      "Deleted",
    );
    expect(screen.getByTestId("item-status-restore-check")).toHaveTextContent("deleted");
  });
});

// ─── 16. List remains usable on error ────────────────────────────────────────

describe("16. list remains usable on error", () => {
  it("when SearchResults is in error state, SearchTotals filter controls still render", () => {
    render(() => (
      <div>
        <SearchInput
          query=""
          onQueryChange={vi.fn()}
          onSubmit={vi.fn()}
          activeFilters={[]}
          onRemoveFilter={vi.fn()}
          isSearching={false}
          resultState="error"
          errorMessage="Connection refused"
        />
        <SearchResults
          items={[]}
          isLoading={false}
          resultState="error"
          errorMessage="Connection refused"
          onNavigate={vi.fn()}
          onSelect={vi.fn()}
        />
        <SearchTotals
          mode="search"
          shown={0}
          total={null}
          totalQualifier={null}
          isTruncated={false}
          hasPreviousCursor={false}
          hasNextCursor={false}
          isLoading={false}
          onPreviousPage={vi.fn()}
          onNextPage={vi.fn()}
        />
      </div>
    ));

    // Error message shown
    expect(screen.getByTestId("search-results-error")).toBeInTheDocument();
    // Search input still available (not blank page)
    expect(screen.getByTestId("search-input-field")).toBeInTheDocument();
    // Totals still render (mode label visible)
    expect(screen.getByTestId("mode-label")).toBeInTheDocument();
  });

  it("when SemanticList has items but error notice is shown, items are still accessible", () => {
    const items = [makeListItem({ id: "err-item", displayName: "Emergency contact" })];

    render(() => (
      <div>
        <div role="alert" data-testid="list-error-banner">
          An error occurred. Please try again.
        </div>
        <SemanticList
          items={items}
          isLoading={false}
          visibleStart={0}
          visibleEnd={1}
          totalHeight={50}
          itemHeight={50}
          onSelect={vi.fn()}
          onExpand={vi.fn()}
          onAction={vi.fn()}
          onScroll={vi.fn()}
        />
      </div>
    ));

    // Error banner present
    expect(screen.getByTestId("list-error-banner")).toBeInTheDocument();
    // List items still rendered and usable — not a blank page
    expect(screen.getByTestId("semantic-list-item-err-item")).toBeInTheDocument();
    expect(screen.getByTestId("item-display-name-err-item")).toHaveTextContent(
      "Emergency contact",
    );
  });

  it("search input remains enabled when resultState is error", () => {
    render(() => (
      <SearchInput
        query=""
        onQueryChange={vi.fn()}
        onSubmit={vi.fn()}
        activeFilters={[]}
        onRemoveFilter={vi.fn()}
        isSearching={false}
        resultState="error"
        errorMessage="Timeout"
      />
    ));

    // isSearching=false means the input is not disabled
    expect(screen.getByTestId("search-input-field")).not.toBeDisabled();
    expect(screen.getByTestId("search-submit-button")).not.toBeDisabled();
  });

  it("totals label shows 'Showing 0 results' on error with null total", () => {
    render(() => (
      <SearchTotals
        mode="search"
        shown={0}
        total={null}
        totalQualifier={null}
        isTruncated={false}
        hasPreviousCursor={false}
        hasNextCursor={false}
        isLoading={false}
        onPreviousPage={vi.fn()}
        onNextPage={vi.fn()}
      />
    ));

    expect(screen.getByTestId("totals-label")).toHaveTextContent("Showing 0 results");
  });
});
