/**
 * Tests for Recall destination (task 4.2.3).
 *
 * Validates:
 * - Search form is rendered with an input (maxLength=512) and submit button
 * - Loading indicator shown when isLoading=true
 * - Loading indicator hidden when isLoading=false
 * - Results list shown when results are non-empty
 * - Results list hidden when results are empty
 * - Each result renders kind, matchedField, rationale, relative score, truthState
 * - Score label says "Relative score" — never "confidence" or "probability"
 * - "Why this answer?" button shown only for results with non-null traceId
 * - "Why this answer?" button NOT shown for results with traceId=null
 * - onOpenTrace called with traceId when trace button clicked
 * - Total count rendered with correct semantics (exact / at_least / estimate)
 * - Total count NOT rendered when totalCount=null
 * - Partial strategies warning shown when unavailableStrategies is non-empty
 * - Partial strategies warning hidden when unavailableStrategies is empty
 * - "No results" shown when results are empty and not loading
 * - "No results" NOT shown when loading
 * - "No results" NOT shown when results are non-empty
 * - onSearch called when form is submitted
 * - Input max length is 512
 *
 * Requirements: F4.2 (task 4.2.3) — Recall destination. MGR-006.
 */
import { describe, it, expect, afterEach, vi } from "vitest";
import { render, screen, cleanup, fireEvent } from "@solidjs/testing-library";
import {
  Recall,
  type RecallProps,
  type RecallResult,
  type TotalSemantics,
} from "./Recall";

afterEach(() => cleanup());

// ─── Helpers ─────────────────────────────────────────────────────────────────

function makeResult(overrides: Partial<RecallResult> = {}): RecallResult {
  return {
    id: "r1",
    kind: "memory",
    matchedField: "content",
    rationale: "High BM25 score on query terms",
    relativeScore: "0.87",
    truthState: "Current",
    revision: 42,
    traceId: "trace-001",
    ...overrides,
  };
}

function renderRecall(props: Partial<RecallProps> = {}) {
  const defaults: RecallProps = {
    query: "",
    results: [],
    totalCount: null,
    unavailableStrategies: [],
    isLoading: false,
    onSearch: vi.fn(),
    onOpenTrace: vi.fn(),
  };
  return render(() => <Recall {...defaults} {...props} />);
}

// ─── Search form ──────────────────────────────────────────────────────────────

describe("search form", () => {
  it("renders the search form", () => {
    renderRecall();
    expect(screen.getByTestId("search-form")).toBeInTheDocument();
  });

  it("renders an input inside the search form", () => {
    renderRecall();
    const form = screen.getByTestId("search-form");
    const input = form.querySelector("input");
    expect(input).not.toBeNull();
  });

  it("renders a submit button inside the search form", () => {
    renderRecall();
    const form = screen.getByTestId("search-form");
    const button = form.querySelector("button[type='submit']");
    expect(button).not.toBeNull();
  });

  it("input has maxLength of 512", () => {
    renderRecall();
    const form = screen.getByTestId("search-form");
    const input = form.querySelector("input");
    expect(input).toHaveAttribute("maxlength", "512");
  });

  it("calls onSearch with the entered query on submit", () => {
    const onSearch = vi.fn();
    renderRecall({ onSearch });
    const input = screen.getByRole("searchbox");
    fireEvent.input(input, { target: { value: "test query" } });
    fireEvent.submit(screen.getByTestId("search-form"));
    expect(onSearch).toHaveBeenCalledWith("test query");
  });

  it("calls onSearch with empty string when input is blank", () => {
    const onSearch = vi.fn();
    renderRecall({ onSearch });
    fireEvent.submit(screen.getByTestId("search-form"));
    expect(onSearch).toHaveBeenCalledWith("");
  });
});

// ─── Loading indicator ────────────────────────────────────────────────────────

describe("loading indicator", () => {
  it("shows loading indicator when isLoading=true", () => {
    renderRecall({ isLoading: true });
    expect(screen.getByTestId("loading-indicator")).toBeInTheDocument();
    expect(screen.getByTestId("loading-indicator")).toHaveTextContent("Searching…");
  });

  it("hides loading indicator when isLoading=false", () => {
    renderRecall({ isLoading: false });
    expect(screen.queryByTestId("loading-indicator")).not.toBeInTheDocument();
  });
});

// ─── Results list ─────────────────────────────────────────────────────────────

describe("results list", () => {
  it("shows results list when results are non-empty", () => {
    renderRecall({ results: [makeResult()] });
    expect(screen.getByTestId("results-list")).toBeInTheDocument();
  });

  it("hides results list when results are empty", () => {
    renderRecall({ results: [] });
    expect(screen.queryByTestId("results-list")).not.toBeInTheDocument();
  });

  it("renders each result with kind, matchedField, rationale, relativeScore, and truthState", () => {
    const result = makeResult({
      id: "r1",
      kind: "entity",
      matchedField: "display_name",
      rationale: "Exact alias match",
      relativeScore: "0.95",
      truthState: "Confirmed",
    });
    renderRecall({ results: [result] });
    const list = screen.getByTestId("results-list");
    expect(list).toHaveTextContent("entity");
    expect(list).toHaveTextContent("display_name");
    expect(list).toHaveTextContent("Exact alias match");
    expect(list).toHaveTextContent("0.95");
    expect(list).toHaveTextContent("Confirmed");
  });

  it("renders multiple results", () => {
    const results = [
      makeResult({ id: "r1", kind: "memory" }),
      makeResult({ id: "r2", kind: "summary" }),
    ];
    renderRecall({ results });
    const list = screen.getByTestId("results-list");
    const items = list.querySelectorAll("li");
    expect(items.length).toBe(2);
  });
});

// ─── Score label ──────────────────────────────────────────────────────────────

describe("score label", () => {
  it("labels score as 'Relative score' not 'confidence' or 'probability'", () => {
    renderRecall({ results: [makeResult({ relativeScore: "0.72" })] });
    const list = screen.getByTestId("results-list");
    expect(list).toHaveTextContent("Relative score: 0.72");
    expect(list.textContent).not.toMatch(/confidence/i);
    expect(list.textContent).not.toMatch(/probability/i);
  });

  it("score label text does not contain the word 'confidence'", () => {
    renderRecall({ results: [makeResult()] });
    const scoreEl = document.querySelector("[data-field='relative-score']");
    expect(scoreEl?.textContent).not.toMatch(/confidence/i);
  });

  it("score label text does not contain the word 'probability'", () => {
    renderRecall({ results: [makeResult()] });
    const scoreEl = document.querySelector("[data-field='relative-score']");
    expect(scoreEl?.textContent).not.toMatch(/probability/i);
  });
});

// ─── Trace button ─────────────────────────────────────────────────────────────

describe("Why this answer? trace button", () => {
  it("shows trace button for results with a non-null traceId", () => {
    const result = makeResult({ id: "r1", traceId: "trace-abc" });
    renderRecall({ results: [result] });
    expect(screen.getByTestId("trace-button-r1")).toBeInTheDocument();
    expect(screen.getByTestId("trace-button-r1")).toHaveTextContent(
      "Why this answer?",
    );
  });

  it("does not show trace button for results with traceId=null", () => {
    const result = makeResult({ id: "r2", traceId: null });
    renderRecall({ results: [result] });
    expect(screen.queryByTestId("trace-button-r2")).not.toBeInTheDocument();
  });

  it("calls onOpenTrace with the correct traceId when trace button clicked", () => {
    const onOpenTrace = vi.fn();
    const result = makeResult({ id: "r1", traceId: "trace-xyz" });
    renderRecall({ results: [result], onOpenTrace });
    fireEvent.click(screen.getByTestId("trace-button-r1"));
    expect(onOpenTrace).toHaveBeenCalledWith("trace-xyz");
  });

  it("renders trace buttons only for results that have traceIds", () => {
    const results = [
      makeResult({ id: "r1", traceId: "trace-1" }),
      makeResult({ id: "r2", traceId: null }),
      makeResult({ id: "r3", traceId: "trace-3" }),
    ];
    renderRecall({ results });
    expect(screen.getByTestId("trace-button-r1")).toBeInTheDocument();
    expect(screen.queryByTestId("trace-button-r2")).not.toBeInTheDocument();
    expect(screen.getByTestId("trace-button-r3")).toBeInTheDocument();
  });
});

// ─── Total count ──────────────────────────────────────────────────────────────

describe("total count", () => {
  it("renders 'Showing N results' for exact semantics", () => {
    const total: TotalSemantics = { kind: "exact", value: 17 };
    renderRecall({ totalCount: total });
    expect(screen.getByTestId("total-count")).toHaveTextContent("Showing 17 results");
  });

  it("renders 'Showing at least N results' for at_least semantics", () => {
    const total: TotalSemantics = { kind: "at_least", value: 50 };
    renderRecall({ totalCount: total });
    expect(screen.getByTestId("total-count")).toHaveTextContent(
      "Showing at least 50 results",
    );
  });

  it("renders 'Showing estimate N results' for estimate semantics", () => {
    const total: TotalSemantics = { kind: "estimate", value: 200 };
    renderRecall({ totalCount: total });
    expect(screen.getByTestId("total-count")).toHaveTextContent(
      "Showing estimate 200 results",
    );
  });

  it("does not render total-count when totalCount is null", () => {
    renderRecall({ totalCount: null });
    expect(screen.queryByTestId("total-count")).not.toBeInTheDocument();
  });

  it("renders total-count even when results list is empty (exact 0)", () => {
    const total: TotalSemantics = { kind: "exact", value: 0 };
    renderRecall({ totalCount: total, results: [] });
    expect(screen.getByTestId("total-count")).toHaveTextContent("Showing 0 results");
  });
});

// ─── Partial strategies warning ───────────────────────────────────────────────

describe("partial strategies warning", () => {
  it("shows warning when unavailableStrategies is non-empty", () => {
    renderRecall({ unavailableStrategies: ["vector"] });
    expect(screen.getByTestId("partial-strategies")).toBeInTheDocument();
    expect(screen.getByTestId("partial-strategies")).toHaveTextContent(
      "Partial: vector unavailable",
    );
  });

  it("shows multiple unavailable strategies joined by comma", () => {
    renderRecall({ unavailableStrategies: ["vector", "graph"] });
    expect(screen.getByTestId("partial-strategies")).toHaveTextContent(
      "Partial: vector, graph unavailable",
    );
  });

  it("hides partial-strategies warning when unavailableStrategies is empty", () => {
    renderRecall({ unavailableStrategies: [] });
    expect(screen.queryByTestId("partial-strategies")).not.toBeInTheDocument();
  });
});

// ─── No results state ─────────────────────────────────────────────────────────

describe("no results state", () => {
  it("shows no-results when results are empty and not loading", () => {
    renderRecall({ results: [], isLoading: false });
    expect(screen.getByTestId("no-results")).toBeInTheDocument();
    expect(screen.getByTestId("no-results")).toHaveTextContent("No results");
  });

  it("does not show no-results when loading", () => {
    renderRecall({ results: [], isLoading: true });
    expect(screen.queryByTestId("no-results")).not.toBeInTheDocument();
  });

  it("does not show no-results when results are non-empty", () => {
    renderRecall({ results: [makeResult()], isLoading: false });
    expect(screen.queryByTestId("no-results")).not.toBeInTheDocument();
  });
});

// ─── recall-shell wrapper ─────────────────────────────────────────────────────

describe("recall-shell wrapper", () => {
  it("renders a section with data-testid=recall-shell", () => {
    renderRecall();
    expect(screen.getByTestId("recall-shell")).toBeInTheDocument();
  });
});
