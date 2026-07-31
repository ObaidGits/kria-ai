/**
 * Tests for SearchTotals (task 4.3.3).
 *
 * Validates:
 * - Total label with exact qualifier: "Showing N of N results"
 * - Total label with at-least qualifier: "Showing N of at least N results"
 * - Total label with estimate qualifier: "Showing N of estimated N results"
 * - Total label with null total: "Showing N results"
 * - mode="search" shows "Full-corpus search"
 * - mode="filter" shows "Filter this view"
 * - data-mode attribute correct for both modes
 * - Truncation notice shown when isTruncated=true, hidden when false
 * - Truncation notice text includes "truncated" and "cursor"
 * - Prev/next buttons present
 * - Prev button disabled when hasPreviousCursor=false
 * - Next button disabled when hasNextCursor=false
 * - Both buttons disabled when isLoading=true
 * - Prev/next button callbacks fire when enabled
 * - strategyInfo section hidden when not provided
 * - strategyInfo shows used and unavailable strategies when provided
 * - Accessibility: buttons have aria-labels
 *
 * Requirements: MGR-006, MGR-014, MGR-023–024, MGR-031;
 *   MGD-026, MGD-030, MGD-046; MG-H01, MG-H04, MG-O05, MG-O25.
 */
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { SearchTotals } from "./SearchTotals";
import type { SearchTotalsProps, StrategyInfo } from "./SearchTotals";

afterEach(() => cleanup());

// ─── Helpers ──────────────────────────────────────────────────────────────────

function renderTotals(partial: Partial<SearchTotalsProps> = {}) {
  const defaults: SearchTotalsProps = {
    mode: "search",
    shown: 10,
    total: 10,
    totalQualifier: "exact",
    isTruncated: false,
    hasPreviousCursor: false,
    hasNextCursor: false,
    isLoading: false,
    onPreviousPage: vi.fn(),
    onNextPage: vi.fn(),
  };
  return render(() => <SearchTotals {...defaults} {...partial} />);
}

// ─── Total label ──────────────────────────────────────────────────────────────

describe("total label", () => {
  it("renders 'Showing N of N results' with exact qualifier", () => {
    renderTotals({ shown: 5, total: 20, totalQualifier: "exact" });
    expect(screen.getByTestId("totals-label")).toHaveTextContent(
      "Showing 5 of 20 results",
    );
  });

  it("renders 'Showing N of at least N results' with at-least qualifier", () => {
    renderTotals({ shown: 5, total: 100, totalQualifier: "at-least" });
    expect(screen.getByTestId("totals-label")).toHaveTextContent(
      "Showing 5 of at least 100 results",
    );
  });

  it("renders 'Showing N of estimated N results' with estimate qualifier", () => {
    renderTotals({ shown: 5, total: 500, totalQualifier: "estimate" });
    expect(screen.getByTestId("totals-label")).toHaveTextContent(
      "Showing 5 of estimated 500 results",
    );
  });

  it("renders 'Showing N results' when total is null", () => {
    renderTotals({ shown: 7, total: null, totalQualifier: null });
    expect(screen.getByTestId("totals-label")).toHaveTextContent(
      "Showing 7 results",
    );
  });

  it("renders 'Showing N results' when total is null regardless of qualifier", () => {
    // qualifier is meaningless when total is null — just show shown count
    renderTotals({ shown: 3, total: null, totalQualifier: "exact" });
    expect(screen.getByTestId("totals-label")).toHaveTextContent(
      "Showing 3 results",
    );
  });

  it("shows shown=0 correctly", () => {
    renderTotals({ shown: 0, total: 0, totalQualifier: "exact" });
    expect(screen.getByTestId("totals-label")).toHaveTextContent(
      "Showing 0 of 0 results",
    );
  });
});

// ─── Mode label ───────────────────────────────────────────────────────────────

describe("mode label", () => {
  it("shows 'Full-corpus search' when mode=search", () => {
    renderTotals({ mode: "search" });
    expect(screen.getByTestId("mode-label")).toHaveTextContent(
      "Full-corpus search",
    );
  });

  it("shows 'Filter this view' when mode=filter", () => {
    renderTotals({ mode: "filter" });
    expect(screen.getByTestId("mode-label")).toHaveTextContent(
      "Filter this view",
    );
  });

  it("mode-label has data-mode=search when mode=search", () => {
    renderTotals({ mode: "search" });
    expect(screen.getByTestId("mode-label")).toHaveAttribute(
      "data-mode",
      "search",
    );
  });

  it("mode-label has data-mode=filter when mode=filter", () => {
    renderTotals({ mode: "filter" });
    expect(screen.getByTestId("mode-label")).toHaveAttribute(
      "data-mode",
      "filter",
    );
  });

  it("mode label is always present (mode=search)", () => {
    renderTotals({ mode: "search" });
    expect(screen.getByTestId("mode-label")).toBeInTheDocument();
  });

  it("mode label is always present (mode=filter)", () => {
    renderTotals({ mode: "filter" });
    expect(screen.getByTestId("mode-label")).toBeInTheDocument();
  });
});

// ─── Truncation notice ────────────────────────────────────────────────────────

describe("truncation notice", () => {
  it("shows truncation-notice when isTruncated=true", () => {
    renderTotals({ isTruncated: true });
    expect(screen.getByTestId("truncation-notice")).toBeInTheDocument();
  });

  it("does not show truncation-notice when isTruncated=false", () => {
    renderTotals({ isTruncated: false });
    expect(
      screen.queryByTestId("truncation-notice"),
    ).not.toBeInTheDocument();
  });

  it("truncation notice text includes 'truncated'", () => {
    renderTotals({ isTruncated: true });
    const notice = screen.getByTestId("truncation-notice");
    expect(notice.textContent?.toLowerCase()).toContain("truncated");
  });

  it("truncation notice text includes 'cursor'", () => {
    renderTotals({ isTruncated: true });
    const notice = screen.getByTestId("truncation-notice");
    expect(notice.textContent?.toLowerCase()).toContain("cursor");
  });

  it("truncation notice mentions the shown count", () => {
    renderTotals({ isTruncated: true, shown: 50 });
    const notice = screen.getByTestId("truncation-notice");
    expect(notice.textContent).toContain("50");
  });
});

// ─── Pagination buttons ───────────────────────────────────────────────────────

describe("pagination buttons presence", () => {
  it("renders prev-page-button", () => {
    renderTotals();
    expect(screen.getByTestId("prev-page-button")).toBeInTheDocument();
  });

  it("renders next-page-button", () => {
    renderTotals();
    expect(screen.getByTestId("next-page-button")).toBeInTheDocument();
  });
});

describe("prev button disabled states", () => {
  it("prev button is disabled when hasPreviousCursor=false", () => {
    renderTotals({ hasPreviousCursor: false });
    expect(screen.getByTestId("prev-page-button")).toBeDisabled();
  });

  it("prev button is enabled when hasPreviousCursor=true and not loading", () => {
    renderTotals({ hasPreviousCursor: true, isLoading: false });
    expect(screen.getByTestId("prev-page-button")).not.toBeDisabled();
  });

  it("prev button is disabled when isLoading=true even if hasPreviousCursor=true", () => {
    renderTotals({ hasPreviousCursor: true, isLoading: true });
    expect(screen.getByTestId("prev-page-button")).toBeDisabled();
  });
});

describe("next button disabled states", () => {
  it("next button is disabled when hasNextCursor=false", () => {
    renderTotals({ hasNextCursor: false });
    expect(screen.getByTestId("next-page-button")).toBeDisabled();
  });

  it("next button is enabled when hasNextCursor=true and not loading", () => {
    renderTotals({ hasNextCursor: true, isLoading: false });
    expect(screen.getByTestId("next-page-button")).not.toBeDisabled();
  });

  it("next button is disabled when isLoading=true even if hasNextCursor=true", () => {
    renderTotals({ hasNextCursor: true, isLoading: true });
    expect(screen.getByTestId("next-page-button")).toBeDisabled();
  });
});

describe("both buttons disabled when loading", () => {
  it("both buttons are disabled when isLoading=true", () => {
    renderTotals({
      hasPreviousCursor: true,
      hasNextCursor: true,
      isLoading: true,
    });
    expect(screen.getByTestId("prev-page-button")).toBeDisabled();
    expect(screen.getByTestId("next-page-button")).toBeDisabled();
  });
});

describe("pagination callbacks", () => {
  it("onPreviousPage is called when prev button is clicked while enabled", () => {
    const onPreviousPage = vi.fn();
    renderTotals({ hasPreviousCursor: true, isLoading: false, onPreviousPage });
    fireEvent.click(screen.getByTestId("prev-page-button"));
    expect(onPreviousPage).toHaveBeenCalledTimes(1);
  });

  it("onNextPage is called when next button is clicked while enabled", () => {
    const onNextPage = vi.fn();
    renderTotals({ hasNextCursor: true, isLoading: false, onNextPage });
    fireEvent.click(screen.getByTestId("next-page-button"));
    expect(onNextPage).toHaveBeenCalledTimes(1);
  });

  it("onPreviousPage is not called when prev button is disabled", () => {
    const onPreviousPage = vi.fn();
    renderTotals({ hasPreviousCursor: false, onPreviousPage });
    fireEvent.click(screen.getByTestId("prev-page-button"));
    expect(onPreviousPage).not.toHaveBeenCalled();
  });

  it("onNextPage is not called when next button is disabled", () => {
    const onNextPage = vi.fn();
    renderTotals({ hasNextCursor: false, onNextPage });
    fireEvent.click(screen.getByTestId("next-page-button"));
    expect(onNextPage).not.toHaveBeenCalled();
  });
});

// ─── Strategy info ────────────────────────────────────────────────────────────

describe("strategy info", () => {
  it("does not render strategy-info when strategyInfo is not provided", () => {
    renderTotals({ strategyInfo: undefined });
    expect(screen.queryByTestId("strategy-info")).not.toBeInTheDocument();
  });

  it("renders strategy-info when strategyInfo is provided", () => {
    const info: StrategyInfo = { used: ["vector", "fts"], unavailable: [] };
    renderTotals({ strategyInfo: info });
    expect(screen.getByTestId("strategy-info")).toBeInTheDocument();
  });

  it("renders used-strategies listing the used strategy names", () => {
    const info: StrategyInfo = {
      used: ["vector", "fts"],
      unavailable: ["graph", "temporal"],
    };
    renderTotals({ strategyInfo: info });
    const used = screen.getByTestId("used-strategies");
    expect(used.textContent).toContain("vector");
    expect(used.textContent).toContain("fts");
  });

  it("renders unavailable-strategies listing the unavailable strategy names", () => {
    const info: StrategyInfo = {
      used: ["fts"],
      unavailable: ["graph", "temporal"],
    };
    renderTotals({ strategyInfo: info });
    const unavail = screen.getByTestId("unavailable-strategies");
    expect(unavail.textContent).toContain("graph");
    expect(unavail.textContent).toContain("temporal");
  });

  it("renders used-strategies with 'Used:' prefix", () => {
    const info: StrategyInfo = { used: ["fts"], unavailable: [] };
    renderTotals({ strategyInfo: info });
    const used = screen.getByTestId("used-strategies");
    expect(used.textContent).toContain("Used:");
  });

  it("renders unavailable-strategies with 'Unavailable:' prefix", () => {
    const info: StrategyInfo = { used: [], unavailable: ["graph"] };
    renderTotals({ strategyInfo: info });
    const unavail = screen.getByTestId("unavailable-strategies");
    expect(unavail.textContent).toContain("Unavailable:");
  });

  it("does not render used-strategies when used array is empty", () => {
    const info: StrategyInfo = { used: [], unavailable: ["graph"] };
    renderTotals({ strategyInfo: info });
    expect(screen.queryByTestId("used-strategies")).not.toBeInTheDocument();
  });

  it("does not render unavailable-strategies when unavailable array is empty", () => {
    const info: StrategyInfo = { used: ["fts"], unavailable: [] };
    renderTotals({ strategyInfo: info });
    expect(
      screen.queryByTestId("unavailable-strategies"),
    ).not.toBeInTheDocument();
  });
});

// ─── Accessibility ────────────────────────────────────────────────────────────

describe("accessibility", () => {
  it("prev button has aria-label='Load previous results'", () => {
    renderTotals();
    expect(screen.getByTestId("prev-page-button")).toHaveAttribute(
      "aria-label",
      "Load previous results",
    );
  });

  it("next button has aria-label='Load next results'", () => {
    renderTotals();
    expect(screen.getByTestId("next-page-button")).toHaveAttribute(
      "aria-label",
      "Load next results",
    );
  });
});

// ─── Root element ─────────────────────────────────────────────────────────────

describe("root element", () => {
  it("renders data-testid=search-totals-root on the root element", () => {
    renderTotals();
    expect(screen.getByTestId("search-totals-root")).toBeInTheDocument();
  });
});
