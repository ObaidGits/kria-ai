/**
 * Tests for SearchResults (task 4.3.2).
 *
 * Validates:
 * - role="list" rendered when items present
 * - Each item renders kind, matchedField, rationale, relativeScore, profileId,
 *   policySummary, truthState, graphRevision, source, validTime,
 *   transactionTime, summary, navigation link
 * - relativeScore rendered as "Rank: X%" — never "confidence"
 * - policySummary shows exact text — no synthetic gap/count labels
 * - truthState shown with data-truth-state attribute for all truth states
 * - Navigation button calls onNavigate with correct target
 * - Enter key on item calls onSelect
 * - Loading state shows loading indicator with role="status"
 * - idle state shows nothing
 * - no-results state shows "No results found. Your filters are preserved." without claiming store empty
 * - error state shows error message
 * - partial state shows partial notice
 * - No invented stats (no "health score", "wellness", "confidence", etc.)
 *
 * Requirements: MGR-006, MGR-014, MGR-023–024, MGR-031;
 *   MGD-026, MGD-030, MGD-046; MG-H01, MG-H04, MG-H10–H12, MG-O05, MG-O25.
 */
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { SearchResults } from "./SearchResults";
import type { SearchResultItem, SearchResultsProps } from "./SearchResults";

afterEach(() => cleanup());

// ─── Fixtures ─────────────────────────────────────────────────────────────────

function makeItem(overrides: Partial<SearchResultItem> = {}): SearchResultItem {
  return {
    id: "item-1",
    kind: "memory",
    matchedField: "title",
    rationale: "Matched on title field with high BM25 score.",
    relativeScore: 0.87,
    profileId: "balanced-v1",
    policySummary: "namespace=default scope=personal sensitivity=0",
    truthState: "Current",
    graphRevision: 42,
    sourceId: "src-001",
    sourceLabel: "Conversation log",
    validTimeStart: "2024-01-01T00:00:00Z",
    validTimeEnd: "2024-12-31T23:59:59Z",
    transactionTime: "2024-06-15T10:30:00Z",
    navigationTarget: "knowledge?inspect=item-1",
    summary: "A note about the project meeting.",
    ...overrides,
  };
}

function renderResults(partial: Partial<SearchResultsProps> = {}) {
  const defaults: SearchResultsProps = {
    items: [],
    isLoading: false,
    resultState: "idle",
    onNavigate: vi.fn(),
    onSelect: vi.fn(),
  };
  return render(() => <SearchResults {...defaults} {...partial} />);
}

// ─── List rendering ───────────────────────────────────────────────────────────

describe("results list", () => {
  it("renders role=list when items are present", () => {
    renderResults({ items: [makeItem()], resultState: "results" });
    const list = screen.getByRole("list");
    expect(list).toBeInTheDocument();
    expect(list).toHaveAttribute("data-testid", "search-results-list");
  });

  it("renders role=listitem for each item", () => {
    renderResults({
      items: [makeItem({ id: "a" }), makeItem({ id: "b" })],
      resultState: "results",
    });
    const items = screen.getAllByRole("listitem");
    expect(items).toHaveLength(2);
  });

  it("does not render the list when items is empty", () => {
    renderResults({ items: [], resultState: "no-results" });
    expect(screen.queryByTestId("search-results-list")).not.toBeInTheDocument();
  });

  it("each item is keyboard-focusable (tabIndex=0)", () => {
    renderResults({ items: [makeItem()], resultState: "results" });
    const item = screen.getByTestId("result-item-item-1");
    expect(item).toHaveAttribute("tabindex", "0");
  });

  it("each item has aria-label combining kind and summary", () => {
    const item = makeItem({ id: "x", kind: "entity", summary: "Alice" });
    renderResults({ items: [item], resultState: "results" });
    const el = screen.getByTestId("result-item-x");
    expect(el.getAttribute("aria-label")).toContain("entity");
    expect(el.getAttribute("aria-label")).toContain("Alice");
  });
});

// ─── Per-item field rendering ─────────────────────────────────────────────────

describe("per-item field rendering", () => {
  it("renders kind badge", () => {
    renderResults({ items: [makeItem({ id: "i1", kind: "entity" })], resultState: "results" });
    expect(screen.getByTestId("result-kind-i1")).toHaveTextContent("entity");
  });

  it("renders summary as primary content", () => {
    renderResults({
      items: [makeItem({ id: "i1", summary: "My test summary" })],
      resultState: "results",
    });
    expect(screen.getByTestId("result-summary-i1")).toHaveTextContent("My test summary");
  });

  it("renders matchedField with 'matched in:' prefix", () => {
    renderResults({
      items: [makeItem({ id: "i1", matchedField: "body" })],
      resultState: "results",
    });
    expect(screen.getByTestId("result-matched-field-i1")).toHaveTextContent("matched in: body");
  });

  it("renders rationale text from backend", () => {
    renderResults({
      items: [makeItem({ id: "i1", rationale: "High BM25 relevance on body field." })],
      resultState: "results",
    });
    expect(screen.getByTestId("result-rationale-i1")).toHaveTextContent(
      "High BM25 relevance on body field.",
    );
  });

  it("renders profileId with 'Profile:' prefix", () => {
    renderResults({
      items: [makeItem({ id: "i1", profileId: "rrf-general-v1" })],
      resultState: "results",
    });
    expect(screen.getByTestId("result-profile-i1")).toHaveTextContent("Profile: rrf-general-v1");
  });

  it("renders graphRevision with 'Rev:' prefix", () => {
    renderResults({
      items: [makeItem({ id: "i1", graphRevision: 99 })],
      resultState: "results",
    });
    expect(screen.getByTestId("result-revision-i1")).toHaveTextContent("Rev: 99");
  });

  it("renders transactionTime with 'Stored:' prefix", () => {
    renderResults({
      items: [makeItem({ id: "i1", transactionTime: "2024-06-15T10:30:00Z" })],
      resultState: "results",
    });
    expect(screen.getByTestId("result-transaction-time-i1")).toHaveTextContent(
      "Stored: 2024-06-15T10:30:00Z",
    );
  });
});

// ─── Relative score — never "confidence" ─────────────────────────────────────

describe("relativeScore rendering", () => {
  it("renders relativeScore as 'Rank: X%'", () => {
    renderResults({
      items: [makeItem({ id: "i1", relativeScore: 0.94 })],
      resultState: "results",
    });
    expect(screen.getByTestId("result-score-i1")).toHaveTextContent("Rank: 94%");
  });

  it("renders 0.0 as 'Rank: 0%'", () => {
    renderResults({
      items: [makeItem({ id: "i1", relativeScore: 0.0 })],
      resultState: "results",
    });
    expect(screen.getByTestId("result-score-i1")).toHaveTextContent("Rank: 0%");
  });

  it("renders 1.0 as 'Rank: 100%'", () => {
    renderResults({
      items: [makeItem({ id: "i1", relativeScore: 1.0 })],
      resultState: "results",
    });
    expect(screen.getByTestId("result-score-i1")).toHaveTextContent("Rank: 100%");
  });

  it("never contains the word 'confidence' in the score field", () => {
    renderResults({
      items: [makeItem({ id: "i1", relativeScore: 0.75 })],
      resultState: "results",
    });
    const el = screen.getByTestId("result-score-i1");
    expect(el.textContent?.toLowerCase()).not.toContain("confidence");
    expect(el.textContent?.toLowerCase()).not.toContain("certainty");
    expect(el.textContent?.toLowerCase()).not.toContain("probability");
  });

  it("rounds to nearest percent", () => {
    renderResults({
      items: [makeItem({ id: "i1", relativeScore: 0.879 })],
      resultState: "results",
    });
    expect(screen.getByTestId("result-score-i1")).toHaveTextContent("Rank: 88%");
  });
});

// ─── policySummary — exact text, no synthesis ─────────────────────────────────

describe("policySummary rendering", () => {
  it("renders exact policySummary text from backend", () => {
    const policyText = "namespace=personal scope=private sensitivity=2";
    renderResults({
      items: [makeItem({ id: "i1", policySummary: policyText })],
      resultState: "results",
    });
    expect(screen.getByTestId("result-policy-i1")).toHaveTextContent(policyText);
  });

  it("does not add synthetic gap/count labels to policySummary", () => {
    const policyText = "open-namespace";
    renderResults({
      items: [makeItem({ id: "i1", policySummary: policyText })],
      resultState: "results",
    });
    const el = screen.getByTestId("result-policy-i1");
    // Must show exact text — no added prefix or suffix from the UI
    expect(el.textContent).toBe(policyText);
  });
});

// ─── truthState — always shown with data-truth-state ─────────────────────────

describe("truthState rendering", () => {
  const allTruthStates = [
    "Current",
    "Stale",
    "Contradicted",
    "Unverified",
    "Superseded",
    "Inferred",
    "Confirmed",
    "Forgotten",
    "Deleted",
    "Unavailable",
  ];

  for (const state of allTruthStates) {
    it(`renders truthState="${state}" with correct data-truth-state attribute`, () => {
      renderResults({
        items: [makeItem({ id: "ts-test", truthState: state })],
        resultState: "results",
      });
      const el = screen.getByTestId("result-truth-state-ts-test");
      expect(el).toHaveTextContent(state);
      expect(el).toHaveAttribute("data-truth-state", state);
      cleanup();
    });
  }

  it("renders truthState text visibly (not just in attribute)", () => {
    renderResults({
      items: [makeItem({ id: "i1", truthState: "Contradicted" })],
      resultState: "results",
    });
    const el = screen.getByTestId("result-truth-state-i1");
    expect(el.textContent).toBe("Contradicted");
  });
});

// ─── Source context ───────────────────────────────────────────────────────────

describe("source context rendering", () => {
  it("renders sourceLabel when present", () => {
    renderResults({
      items: [makeItem({ id: "i1", sourceLabel: "Conversation log", sourceId: "src-001" })],
      resultState: "results",
    });
    expect(screen.getByTestId("result-source-i1")).toHaveTextContent("Conversation log");
  });

  it("falls back to sourceId when sourceLabel is null", () => {
    renderResults({
      items: [makeItem({ id: "i1", sourceLabel: null, sourceId: "src-999" })],
      resultState: "results",
    });
    expect(screen.getByTestId("result-source-i1")).toHaveTextContent("src-999");
  });

  it("does not render source section when both sourceId and sourceLabel are null", () => {
    renderResults({
      items: [makeItem({ id: "i1", sourceId: null, sourceLabel: null })],
      resultState: "results",
    });
    expect(screen.queryByTestId("result-source-i1")).not.toBeInTheDocument();
  });
});

// ─── Valid time ───────────────────────────────────────────────────────────────

describe("valid time rendering", () => {
  it("renders valid time range when both start and end are present", () => {
    renderResults({
      items: [
        makeItem({
          id: "i1",
          validTimeStart: "2024-01-01T00:00:00Z",
          validTimeEnd: "2024-12-31T23:59:59Z",
        }),
      ],
      resultState: "results",
    });
    const el = screen.getByTestId("result-valid-time-i1");
    expect(el.textContent).toContain("2024-01-01T00:00:00Z");
    expect(el.textContent).toContain("2024-12-31T23:59:59Z");
  });

  it("shows 'ongoing' when validTimeEnd is null but start is present", () => {
    renderResults({
      items: [makeItem({ id: "i1", validTimeStart: "2024-01-01T00:00:00Z", validTimeEnd: null })],
      resultState: "results",
    });
    const el = screen.getByTestId("result-valid-time-i1");
    expect(el.textContent).toContain("ongoing");
  });

  it("does not render valid time section when both are null", () => {
    renderResults({
      items: [makeItem({ id: "i1", validTimeStart: null, validTimeEnd: null })],
      resultState: "results",
    });
    expect(screen.queryByTestId("result-valid-time-i1")).not.toBeInTheDocument();
  });
});

// ─── Navigation ───────────────────────────────────────────────────────────────

describe("navigation", () => {
  it("renders a navigation button per item", () => {
    renderResults({ items: [makeItem()], resultState: "results" });
    expect(screen.getByTestId("result-navigate-item-1")).toBeInTheDocument();
  });

  it("navigation button calls onNavigate with correct target on click", () => {
    const onNavigate = vi.fn();
    renderResults({
      items: [makeItem({ id: "i1", navigationTarget: "knowledge?inspect=i1" })],
      resultState: "results",
      onNavigate,
    });
    fireEvent.click(screen.getByTestId("result-navigate-i1"));
    expect(onNavigate).toHaveBeenCalledWith("knowledge?inspect=i1");
  });

  it("navigation button has a descriptive aria-label", () => {
    renderResults({
      items: [makeItem({ id: "i1", kind: "entity", summary: "Alice" })],
      resultState: "results",
    });
    const btn = screen.getByTestId("result-navigate-i1");
    const label = btn.getAttribute("aria-label") ?? "";
    expect(label).toContain("entity");
    expect(label).toContain("Alice");
  });
});

// ─── Enter key activates onSelect ─────────────────────────────────────────────

describe("Enter key activation", () => {
  it("calls onSelect with the item when Enter is pressed on a result", () => {
    const onSelect = vi.fn();
    const item = makeItem({ id: "i1" });
    renderResults({ items: [item], resultState: "results", onSelect });
    const el = screen.getByTestId("result-item-i1");
    fireEvent.keyDown(el, { key: "Enter" });
    expect(onSelect).toHaveBeenCalledWith(item);
  });

  it("does not call onSelect when non-Enter key is pressed", () => {
    const onSelect = vi.fn();
    const item = makeItem({ id: "i1" });
    renderResults({ items: [item], resultState: "results", onSelect });
    const el = screen.getByTestId("result-item-i1");
    fireEvent.keyDown(el, { key: " " });
    fireEvent.keyDown(el, { key: "Tab" });
    expect(onSelect).not.toHaveBeenCalled();
  });
});

// ─── Loading state ────────────────────────────────────────────────────────────

describe("loading state", () => {
  it("renders a loading indicator with role=status when isLoading=true", () => {
    renderResults({ isLoading: true, resultState: "searching" });
    const indicator = screen.getByTestId("search-results-loading");
    expect(indicator).toBeInTheDocument();
    expect(indicator).toHaveAttribute("role", "status");
  });

  it("loading indicator has aria-live=polite", () => {
    renderResults({ isLoading: true, resultState: "searching" });
    expect(screen.getByTestId("search-results-loading")).toHaveAttribute(
      "aria-live",
      "polite",
    );
  });

  it("does not show the results list while loading", () => {
    renderResults({
      items: [makeItem()],
      isLoading: true,
      resultState: "searching",
    });
    expect(screen.queryByTestId("search-results-list")).not.toBeInTheDocument();
  });
});

// ─── Idle state ───────────────────────────────────────────────────────────────

describe("idle state", () => {
  it("renders nothing visible when resultState=idle and not loading", () => {
    renderResults({ items: [], isLoading: false, resultState: "idle" });
    expect(screen.queryByTestId("search-results-list")).not.toBeInTheDocument();
    expect(screen.queryByTestId("search-results-loading")).not.toBeInTheDocument();
    expect(screen.queryByTestId("search-results-empty")).not.toBeInTheDocument();
    expect(screen.queryByTestId("search-results-error")).not.toBeInTheDocument();
    expect(screen.queryByTestId("search-results-partial-notice")).not.toBeInTheDocument();
  });
});

// ─── No-results state ─────────────────────────────────────────────────────────

describe("no-results state", () => {
  it("shows the preserved-filters copy", () => {
    renderResults({ items: [], isLoading: false, resultState: "no-results" });
    const el = screen.getByTestId("search-results-empty");
    expect(el).toHaveTextContent("No results found. Your filters are preserved.");
  });

  it("does not claim the store is empty", () => {
    renderResults({ items: [], isLoading: false, resultState: "no-results" });
    const el = screen.getByTestId("search-results-empty");
    const text = el.textContent?.toLowerCase() ?? "";
    // Forbidden copy patterns that imply the store is empty
    expect(text).not.toContain("nothing here");
    expect(text).not.toContain("store is empty");
    expect(text).not.toContain("no memories");
    expect(text).not.toContain("empty");
  });
});

// ─── Error state ──────────────────────────────────────────────────────────────

describe("error state", () => {
  it("shows the error message when resultState=error", () => {
    renderResults({
      items: [],
      isLoading: false,
      resultState: "error",
      errorMessage: "Connection refused",
    });
    const el = screen.getByTestId("search-results-error");
    expect(el).toHaveTextContent("Connection refused");
  });

  it("shows a generic error when no errorMessage is provided", () => {
    renderResults({ items: [], isLoading: false, resultState: "error" });
    const el = screen.getByTestId("search-results-error");
    expect(el).toBeInTheDocument();
    expect(el.textContent?.length).toBeGreaterThan(0);
  });

  it("does not claim the store is empty in the error state", () => {
    renderResults({
      items: [],
      isLoading: false,
      resultState: "error",
      errorMessage: "Timeout",
    });
    const el = screen.getByTestId("search-results-error");
    const text = el.textContent?.toLowerCase() ?? "";
    expect(text).not.toContain("store is empty");
    expect(text).not.toContain("no memories");
  });
});

// ─── Partial state ────────────────────────────────────────────────────────────

describe("partial state", () => {
  it("shows the partial notice", () => {
    renderResults({
      items: [makeItem()],
      isLoading: false,
      resultState: "partial",
    });
    const notice = screen.getByTestId("search-results-partial-notice");
    expect(notice).toHaveTextContent("Partial results");
    expect(notice).toHaveTextContent("some strategies unavailable");
  });

  it("still renders results alongside the partial notice", () => {
    renderResults({
      items: [makeItem({ id: "p1" })],
      isLoading: false,
      resultState: "partial",
    });
    expect(screen.getByTestId("search-results-list")).toBeInTheDocument();
    expect(screen.getByTestId("search-results-partial-notice")).toBeInTheDocument();
  });
});

// ─── Mixed kinds ──────────────────────────────────────────────────────────────

describe("mixed result kinds", () => {
  it("renders items of different kinds in one list", () => {
    const items: SearchResultItem[] = [
      makeItem({ id: "a", kind: "entity" }),
      makeItem({ id: "b", kind: "memory" }),
      makeItem({ id: "c", kind: "relation" }),
      makeItem({ id: "d", kind: "source" }),
      makeItem({ id: "e", kind: "goal" }),
    ];
    renderResults({ items, resultState: "results" });
    const list = screen.getByTestId("search-results-list");
    expect(list).toBeInTheDocument();
    expect(screen.getByTestId("result-kind-a")).toHaveTextContent("entity");
    expect(screen.getByTestId("result-kind-b")).toHaveTextContent("memory");
    expect(screen.getByTestId("result-kind-c")).toHaveTextContent("relation");
    expect(screen.getByTestId("result-kind-d")).toHaveTextContent("source");
    expect(screen.getByTestId("result-kind-e")).toHaveTextContent("goal");
  });
});

// ─── No invented stats ────────────────────────────────────────────────────────

describe("no invented stats", () => {
  it("does not render any 'health score' or 'wellness' text", () => {
    renderResults({
      items: [makeItem({ id: "inv" })],
      resultState: "results",
    });
    const root = screen.getByTestId("search-results-root");
    const text = root.textContent?.toLowerCase() ?? "";
    expect(text).not.toContain("health score");
    expect(text).not.toContain("wellness");
    expect(text).not.toContain("certainty");
    expect(text).not.toContain("confidence");
  });
});
