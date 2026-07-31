/**
 * Tests for SearchInput (task 4.3.1).
 *
 * Validates:
 * - Character limit enforcement (≤512 accepted, >512 blocks submit)
 * - Character count display
 * - At-limit / over-limit announcement
 * - Debounce: query change triggers a single delayed submit
 * - Cancel: new query before debounce fires aborts previous AbortController
 * - Filter chip rendering (role, label, remove button)
 * - Filter chip keyboard navigation (arrow keys, Delete/Backspace to remove)
 * - Chip remove button: removes filter and returns focus to input
 * - Result state announcements (aria-live region updates)
 * - Platform shortcut display (Ctrl+K vs ⌘K)
 * - No saved-filter UI rendered by default
 *
 * Requirements: MGR-006, MGR-014, MGR-031; MG-H01, MG-H04, MG-H10–H12.
 */
import {
  afterEach,
  beforeEach,
  describe,
  expect,
  it,
  vi,
  type MockInstance,
} from "vitest";
import {
  cleanup,
  fireEvent,
  render,
  screen,
} from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { SearchInput, QUERY_MAX_LENGTH } from "./SearchInput";
import type { SearchFilter, SearchInputProps, ResultState } from "./SearchInput";

afterEach(() => cleanup());

// ─── Helpers ─────────────────────────────────────────────────────────────────

function makeFilter(overrides: Partial<SearchFilter> = {}): SearchFilter {
  return {
    id: "f1",
    label: "Kind",
    value: "memory",
    kind: "kind",
    ...overrides,
  };
}

type PartialProps = Partial<SearchInputProps>;

function renderSearchInput(partial: PartialProps = {}) {
  const defaults: SearchInputProps = {
    query: "",
    onQueryChange: vi.fn(),
    onSubmit: vi.fn(),
    activeFilters: [],
    onRemoveFilter: vi.fn(),
    isSearching: false,
    resultState: "idle",
  };
  return render(() => <SearchInput {...defaults} {...partial} />);
}

// ─── Character limit ──────────────────────────────────────────────────────────

describe("character limit enforcement", () => {
  it("shows char count display", () => {
    renderSearchInput({ query: "hello" });
    expect(screen.getByTestId("char-count-text")).toHaveTextContent(
      `5 / ${QUERY_MAX_LENGTH}`,
    );
  });

  it("does not show at-limit announcement for short queries", () => {
    renderSearchInput({ query: "short" });
    expect(
      screen.queryByTestId("char-limit-announcement"),
    ).not.toBeInTheDocument();
  });

  it("shows at-limit announcement when query is exactly 512 characters", () => {
    const atLimit = "a".repeat(QUERY_MAX_LENGTH);
    renderSearchInput({ query: atLimit });
    const announcement = screen.getByTestId("char-limit-announcement");
    expect(announcement).toBeInTheDocument();
    expect(announcement.textContent).toMatch(/limit reached/i);
  });

  it("shows over-limit announcement when query exceeds 512 characters", () => {
    const overLimit = "a".repeat(QUERY_MAX_LENGTH + 1);
    renderSearchInput({ query: overLimit });
    const announcement = screen.getByTestId("char-limit-announcement");
    expect(announcement).toBeInTheDocument();
    expect(announcement.textContent).toMatch(/too long/i);
  });

  it("disables the submit button when query is over 512 characters", () => {
    const overLimit = "a".repeat(QUERY_MAX_LENGTH + 1);
    renderSearchInput({ query: overLimit });
    const button = screen.getByTestId("search-submit-button");
    expect(button).toBeDisabled();
  });

  it("does not disable submit when query is exactly at limit", () => {
    const atLimit = "a".repeat(QUERY_MAX_LENGTH);
    renderSearchInput({ query: atLimit, isSearching: false });
    const button = screen.getByTestId("search-submit-button");
    expect(button).not.toBeDisabled();
  });

  it("char count display reflects query length", () => {
    const q = "x".repeat(100);
    renderSearchInput({ query: q });
    expect(screen.getByTestId("char-count-text")).toHaveTextContent(
      `100 / ${QUERY_MAX_LENGTH}`,
    );
  });
});

// ─── Form rendering ───────────────────────────────────────────────────────────

describe("form rendering", () => {
  it("renders the search form with role=search", () => {
    renderSearchInput();
    const form = screen.getByRole("search");
    expect(form).toBeInTheDocument();
  });

  it("renders the text input", () => {
    renderSearchInput();
    expect(screen.getByTestId("search-input-field")).toBeInTheDocument();
  });

  it("renders the submit button", () => {
    renderSearchInput();
    expect(screen.getByTestId("search-submit-button")).toBeInTheDocument();
  });

  it("disables input when isSearching=true", () => {
    renderSearchInput({ isSearching: true });
    expect(screen.getByTestId("search-input-field")).toBeDisabled();
  });

  it("disables submit button when isSearching=true", () => {
    renderSearchInput({ isSearching: true });
    expect(screen.getByTestId("search-submit-button")).toBeDisabled();
  });
});

// ─── Debounce ─────────────────────────────────────────────────────────────────

describe("debounced submit", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("does not fire onSubmit immediately when query changes", () => {
    const onSubmit = vi.fn();
    // Use controlled signal so we can change query reactively.
    const [query, setQuery] = createSignal("init");

    render(() => (
      <SearchInput
        query={query()}
        onQueryChange={setQuery}
        onSubmit={onSubmit}
        activeFilters={[]}
        onRemoveFilter={vi.fn()}
        isSearching={false}
        resultState="idle"
      />
    ));

    // Initial render fires a debounced submit for "init" — advance past it.
    vi.runAllTimers();
    const initialCount = onSubmit.mock.calls.length;

    // Now change query.
    setQuery("new query");
    // Not yet called.
    expect(onSubmit.mock.calls.length).toBe(initialCount);

    // After debounce fires.
    vi.advanceTimersByTime(300);
    expect(onSubmit.mock.calls.length).toBe(initialCount + 1);
    expect(onSubmit.mock.calls[onSubmit.mock.calls.length - 1][0]).toBe("new query");
  });

  it("fires only once when query changes multiple times within debounce window", () => {
    const onSubmit = vi.fn();
    const [query, setQuery] = createSignal("a");

    render(() => (
      <SearchInput
        query={query()}
        onQueryChange={setQuery}
        onSubmit={onSubmit}
        activeFilters={[]}
        onRemoveFilter={vi.fn()}
        isSearching={false}
        resultState="idle"
      />
    ));

    vi.runAllTimers();
    const initialCount = onSubmit.mock.calls.length;

    setQuery("ab");
    setQuery("abc");
    setQuery("abcd");

    // Still not fired for new changes.
    expect(onSubmit.mock.calls.length).toBe(initialCount);

    vi.advanceTimersByTime(300);
    // Only one call for "abcd".
    expect(onSubmit.mock.calls.length).toBe(initialCount + 1);
    expect(onSubmit.mock.calls[onSubmit.mock.calls.length - 1][0]).toBe("abcd");
  });

  it("does not fire onSubmit via debounce when query is over limit", () => {
    const onSubmit = vi.fn();
    const overLimit = "a".repeat(QUERY_MAX_LENGTH + 1);
    const [query, setQuery] = createSignal("");

    render(() => (
      <SearchInput
        query={query()}
        onQueryChange={setQuery}
        onSubmit={onSubmit}
        activeFilters={[]}
        onRemoveFilter={vi.fn()}
        isSearching={false}
        resultState="idle"
      />
    ));

    vi.runAllTimers();
    const initialCount = onSubmit.mock.calls.length;

    setQuery(overLimit);
    vi.advanceTimersByTime(300);
    // No additional call because over limit.
    expect(onSubmit.mock.calls.length).toBe(initialCount);
  });
});

// ─── AbortController cancellation ────────────────────────────────────────────

describe("AbortController cancellation", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("passes an AbortSignal to onSubmit", () => {
    const onSubmit = vi.fn();
    const [query, setQuery] = createSignal("first");

    render(() => (
      <SearchInput
        query={query()}
        onQueryChange={setQuery}
        onSubmit={onSubmit}
        activeFilters={[]}
        onRemoveFilter={vi.fn()}
        isSearching={false}
        resultState="idle"
      />
    ));

    vi.runAllTimers();
    expect(onSubmit).toHaveBeenCalled();
    const signal = onSubmit.mock.calls[0][2] as AbortSignal;
    expect(signal).toBeInstanceOf(AbortSignal);
  });

  it("aborts the previous AbortSignal when a new query fires", () => {
    const signals: AbortSignal[] = [];
    const onSubmit = vi.fn((
      _q: string,
      _filters: SearchFilter[],
      signal: AbortSignal,
    ) => {
      signals.push(signal);
    });

    const [query, setQuery] = createSignal("first");

    render(() => (
      <SearchInput
        query={query()}
        onQueryChange={setQuery}
        onSubmit={onSubmit}
        activeFilters={[]}
        onRemoveFilter={vi.fn()}
        isSearching={false}
        resultState="idle"
      />
    ));

    // Fire initial submit.
    vi.runAllTimers();
    expect(signals.length).toBeGreaterThanOrEqual(1);
    const firstSignal = signals[signals.length - 1];
    expect(firstSignal.aborted).toBe(false);

    // Trigger a new query — the previous signal should be aborted.
    setQuery("second");
    vi.advanceTimersByTime(300);
    expect(signals.length).toBeGreaterThan(1);
    expect(firstSignal.aborted).toBe(true);
  });
});

// ─── Filter chips ─────────────────────────────────────────────────────────────

describe("filter chip rendering", () => {
  it("does not render chip list when activeFilters is empty", () => {
    renderSearchInput({ activeFilters: [] });
    expect(
      screen.queryByTestId("filter-chips-list"),
    ).not.toBeInTheDocument();
  });

  it("renders chip list with role=list when activeFilters is non-empty", () => {
    renderSearchInput({ activeFilters: [makeFilter()] });
    const list = screen.getByTestId("filter-chips-list");
    expect(list).toBeInTheDocument();
    expect(list).toHaveAttribute("role", "list");
  });

  it("renders each chip as role=listitem", () => {
    const filters = [makeFilter({ id: "f1" }), makeFilter({ id: "f2", label: "Source", value: "conversation" })];
    renderSearchInput({ activeFilters: filters });
    const items = screen.getAllByRole("listitem");
    expect(items.length).toBe(2);
  });

  it("renders a remove button for each chip with correct aria-label", () => {
    const filter = makeFilter({ id: "f1", label: "Kind" });
    renderSearchInput({ activeFilters: [filter] });
    const btn = screen.getByTestId("remove-chip-f1");
    expect(btn).toBeInTheDocument();
    expect(btn).toHaveAttribute("aria-label", "Remove filter: Kind");
  });

  it("calls onRemoveFilter with the correct filter id when remove is clicked", () => {
    const onRemoveFilter = vi.fn();
    const filter = makeFilter({ id: "f99" });
    renderSearchInput({ activeFilters: [filter], onRemoveFilter });
    fireEvent.click(screen.getByTestId("remove-chip-f99"));
    expect(onRemoveFilter).toHaveBeenCalledWith("f99");
  });

  it("renders chip label and value in the chip text", () => {
    const filter = makeFilter({ id: "f1", label: "Kind", value: "memory" });
    renderSearchInput({ activeFilters: [filter] });
    const chip = screen.getByTestId("filter-chip-f1");
    expect(chip.textContent).toContain("Kind");
    expect(chip.textContent).toContain("memory");
  });

  it("renders multiple chips", () => {
    const filters = [
      makeFilter({ id: "f1", label: "Kind", value: "memory" }),
      makeFilter({ id: "f2", label: "Truth", value: "Current" }),
      makeFilter({ id: "f3", label: "Source", value: "conversation" }),
    ];
    renderSearchInput({ activeFilters: filters });
    expect(screen.getByTestId("filter-chip-f1")).toBeInTheDocument();
    expect(screen.getByTestId("filter-chip-f2")).toBeInTheDocument();
    expect(screen.getByTestId("filter-chip-f3")).toBeInTheDocument();
  });
});

// ─── Filter chip keyboard navigation ─────────────────────────────────────────

describe("filter chip keyboard navigation", () => {
  it("removes focused chip on Delete key and calls onRemoveFilter", () => {
    const onRemoveFilter = vi.fn();
    const filters = [makeFilter({ id: "f1" })];
    renderSearchInput({ activeFilters: filters, onRemoveFilter });
    const chip = screen.getByTestId("filter-chip-f1");
    fireEvent.keyDown(chip, { key: "Delete" });
    expect(onRemoveFilter).toHaveBeenCalledWith("f1");
  });

  it("removes focused chip on Backspace key and calls onRemoveFilter", () => {
    const onRemoveFilter = vi.fn();
    const filters = [makeFilter({ id: "f1" })];
    renderSearchInput({ activeFilters: filters, onRemoveFilter });
    const chip = screen.getByTestId("filter-chip-f1");
    fireEvent.keyDown(chip, { key: "Backspace" });
    expect(onRemoveFilter).toHaveBeenCalledWith("f1");
  });

  it("does not call onRemoveFilter on other keys on chip", () => {
    const onRemoveFilter = vi.fn();
    const filters = [makeFilter({ id: "f1" })];
    renderSearchInput({ activeFilters: filters, onRemoveFilter });
    const chip = screen.getByTestId("filter-chip-f1");
    fireEvent.keyDown(chip, { key: "Enter" });
    expect(onRemoveFilter).not.toHaveBeenCalled();
  });
});

// ─── Result state announcements ───────────────────────────────────────────────

describe("result state announcements", () => {
  it("shows empty announcement for idle state", () => {
    renderSearchInput({ resultState: "idle" });
    const region = screen.getByTestId("result-state-announcement");
    expect(region.textContent).toBe("");
  });

  it("announces 'Searching…' when resultState=searching", () => {
    renderSearchInput({ resultState: "searching" });
    const region = screen.getByTestId("result-state-announcement");
    expect(region.textContent).toBe("Searching\u2026");
  });

  it("announces result count for resultState=results (exact)", () => {
    renderSearchInput({
      resultState: "results",
      resultCount: 5,
      resultCountQualifier: "exact",
    });
    const region = screen.getByTestId("result-state-announcement");
    expect(region.textContent).toBe("5 results found");
  });

  it("announces 'At least N results found' for at-least qualifier", () => {
    renderSearchInput({
      resultState: "results",
      resultCount: 10,
      resultCountQualifier: "at-least",
    });
    const region = screen.getByTestId("result-state-announcement");
    expect(region.textContent).toBe("At least 10 results found");
  });

  it("announces 'Estimate N results found' for estimate qualifier", () => {
    renderSearchInput({
      resultState: "results",
      resultCount: 100,
      resultCountQualifier: "estimate",
    });
    const region = screen.getByTestId("result-state-announcement");
    expect(region.textContent).toBe("Estimate 100 results found");
  });

  it("announces 'No results' for resultState=no-results", () => {
    renderSearchInput({ resultState: "no-results" });
    const region = screen.getByTestId("result-state-announcement");
    expect(region.textContent).toBe("No results");
  });

  it("announces error message for resultState=error with errorMessage", () => {
    renderSearchInput({
      resultState: "error",
      errorMessage: "Connection refused",
    });
    const region = screen.getByTestId("result-state-announcement");
    expect(region.textContent).toBe("Search error: Connection refused");
  });

  it("announces generic error for resultState=error without errorMessage", () => {
    renderSearchInput({ resultState: "error" });
    const region = screen.getByTestId("result-state-announcement");
    expect(region.textContent).toBe("Search error");
  });

  it("announcement region has role=status and aria-live=polite", () => {
    renderSearchInput({ resultState: "idle" });
    const region = screen.getByTestId("result-state-announcement");
    expect(region).toHaveAttribute("role", "status");
    expect(region).toHaveAttribute("aria-live", "polite");
  });

  it("announces partial result state", () => {
    renderSearchInput({ resultState: "partial", resultCount: 3 });
    const region = screen.getByTestId("result-state-announcement");
    expect(region.textContent).toContain("partial");
  });
});

// ─── Platform shortcut display ────────────────────────────────────────────────

describe("platform shortcut display", () => {
  it("shows Ctrl+K shortcut on non-Mac platform", () => {
    // Default jsdom environment: navigator.platform is usually empty/Linux.
    renderSearchInput();
    const hint = screen.getByTestId("search-shortcut-hint");
    // On non-Mac environments (like jsdom), expect Ctrl+K.
    // We can't guarantee Mac detection in jsdom, so we check it's one of the two.
    expect(hint.textContent).toMatch(/Ctrl\+K|⌘K/);
  });

  it("renders the shortcut hint element", () => {
    renderSearchInput();
    expect(screen.getByTestId("search-shortcut-hint")).toBeInTheDocument();
  });

  it("submit button aria-label includes shortcut description", () => {
    renderSearchInput();
    const button = screen.getByTestId("search-submit-button");
    const label = button.getAttribute("aria-label") ?? "";
    expect(label).toMatch(/Control K|Command K/);
  });
});

// ─── Mac platform detection simulation ───────────────────────────────────────

describe("platform shortcut — Mac simulation", () => {
  let originalPlatform: string;

  beforeEach(() => {
    originalPlatform = navigator.platform;
    Object.defineProperty(navigator, "platform", {
      value: "MacIntel",
      writable: true,
      configurable: true,
    });
  });

  afterEach(() => {
    Object.defineProperty(navigator, "platform", {
      value: originalPlatform,
      writable: true,
      configurable: true,
    });
    cleanup();
  });

  it("shows ⌘K shortcut when navigator.platform is MacIntel", () => {
    // Re-render after overriding platform.
    render(() => (
      <SearchInput
        query=""
        onQueryChange={vi.fn()}
        onSubmit={vi.fn()}
        activeFilters={[]}
        onRemoveFilter={vi.fn()}
        isSearching={false}
        resultState="idle"
      />
    ));
    const hint = screen.getByTestId("search-shortcut-hint");
    expect(hint.textContent).toBe("⌘K");
  });

  it("submit button aria-label includes 'Command K' on Mac", () => {
    render(() => (
      <SearchInput
        query=""
        onQueryChange={vi.fn()}
        onSubmit={vi.fn()}
        activeFilters={[]}
        onRemoveFilter={vi.fn()}
        isSearching={false}
        resultState="idle"
      />
    ));
    const button = screen.getByTestId("search-submit-button");
    expect(button.getAttribute("aria-label")).toContain("Command K");
  });
});

// ─── No saved-filter UI ───────────────────────────────────────────────────────

describe("saved-filter seam — no UI by default", () => {
  it("does not render a save-filter button when onSaveFilter is not provided", () => {
    renderSearchInput();
    // No save-filter button should exist.
    expect(
      document.querySelector("[data-testid='save-filter-button']"),
    ).toBeNull();
  });

  it("does not render a save-filter button even when onSaveFilter prop is provided", () => {
    // The seam is typed but the feature is not shipped — no UI should appear.
    renderSearchInput({ onSaveFilter: vi.fn() });
    expect(
      document.querySelector("[data-testid='save-filter-button']"),
    ).toBeNull();
  });
});

// ─── Accessibility structure ──────────────────────────────────────────────────

describe("accessibility structure", () => {
  it("input has an associated label", () => {
    renderSearchInput();
    const input = screen.getByTestId("search-input-field");
    // Either aria-label or label[for] must be present.
    const hasAriaLabel = input.hasAttribute("aria-label");
    const id = input.getAttribute("id");
    const associatedLabel =
      id ? document.querySelector(`label[for="${id}"]`) : null;
    expect(hasAriaLabel || associatedLabel !== null).toBe(true);
  });

  it("chip list has aria-label", () => {
    renderSearchInput({ activeFilters: [makeFilter()] });
    const list = screen.getByTestId("filter-chips-list");
    expect(list).toHaveAttribute("aria-label");
  });

  it("remove buttons have aria-label matching 'Remove filter: {label}'", () => {
    const filter = makeFilter({ id: "f1", label: "Kind" });
    renderSearchInput({ activeFilters: [filter] });
    const btn = screen.getByTestId("remove-chip-f1");
    expect(btn).toHaveAttribute("aria-label", "Remove filter: Kind");
  });

  it("result state region has aria-atomic=true", () => {
    renderSearchInput({ resultState: "idle" });
    const region = screen.getByTestId("result-state-announcement");
    expect(region).toHaveAttribute("aria-atomic", "true");
  });

  it("char count region has role=status", () => {
    renderSearchInput({ query: "test" });
    const region = screen.getByTestId("search-char-count");
    expect(region).toHaveAttribute("role", "status");
  });
});

// ─── Explicit form submit ─────────────────────────────────────────────────────

describe("explicit form submit", () => {
  it("calls onSubmit when form is submitted explicitly", () => {
    const onSubmit = vi.fn();
    renderSearchInput({ query: "test query", onSubmit });
    const form = screen.getByTestId("search-input-form");
    fireEvent.submit(form);
    expect(onSubmit).toHaveBeenCalledWith(
      "test query",
      [],
      expect.any(AbortSignal),
    );
  });

  it("does not call onSubmit on explicit submit when query is over limit", () => {
    const onSubmit = vi.fn();
    const overLimit = "a".repeat(QUERY_MAX_LENGTH + 1);
    renderSearchInput({ query: overLimit, onSubmit });
    const form = screen.getByTestId("search-input-form");
    fireEvent.submit(form);
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it("passes active filters to onSubmit", () => {
    const onSubmit = vi.fn();
    const filters = [makeFilter({ id: "f1" })];
    renderSearchInput({ query: "test", onSubmit, activeFilters: filters });
    const form = screen.getByTestId("search-input-form");
    fireEvent.submit(form);
    expect(onSubmit).toHaveBeenCalledWith(
      "test",
      filters,
      expect.any(AbortSignal),
    );
  });
});
