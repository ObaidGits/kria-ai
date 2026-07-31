/**
 * Tests for Timeline destination (task 4.5.3).
 *
 * Validates all rendering requirements:
 * - Unavailable capability: shows timeline-unavailable, no controls
 * - Available capability: shows timeline-destination and capability badge
 * - Capability badge has correct data-capability attribute
 * - Range start/end shown when non-null; hidden when null
 * - Timezone shown when non-null; hidden when null
 * - Revision shown when non-null; hidden when null
 * - Filter toggles: each kind button shown
 * - Filter toggle data-active correct when included/excluded
 * - Toggle calls onChangeKindToggle with correct kind
 * - Loading state
 * - Error state
 * - Changes list shown when non-empty
 * - Empty state shown when empty and not loading
 * - Each change: kind, label, description, transactionTime, revision
 * - validTimeStart/End conditional
 * - Load more shown when hasMore=true; hidden when false; calls onLoadMore
 *
 * Requirements: MGR-010 (temporal graph correctness), F4.2 task 4.5.3.
 */
import { describe, it, expect, vi, afterEach } from "vitest";
import { render, screen, cleanup, fireEvent } from "@solidjs/testing-library";
import {
  Timeline,
  type TimelineProps,
  type TimelineState,
  type TimelineChange,
  type TimelineCapability,
  type TimelineChangeKind,
} from "./Timeline";

afterEach(() => cleanup());

// ─── Helpers ──────────────────────────────────────────────────────────────────

function makeChange(overrides: Partial<TimelineChange> = {}): TimelineChange {
  return {
    id: "chg-1",
    kind: "addition",
    label: "Test change",
    validTimeStart: null,
    validTimeEnd: null,
    transactionRevision: 1,
    transactionTime: "2024-01-15T10:00:00Z",
    description: "A test description",
    ...overrides,
  };
}

function makeState(overrides: Partial<TimelineState> = {}): TimelineState {
  return {
    capability: "full",
    isLoading: false,
    errorMessage: null,
    query: {
      rangeStart: null,
      rangeEnd: null,
      timezone: null,
      graphRevision: null,
      includeChangeKinds: ["addition", "expiry", "contradiction", "supersession", "correction"],
    },
    changes: [],
    hasMore: false,
    cursorToken: null,
    ...overrides,
  };
}

function renderTimeline(
  stateOverrides: Partial<TimelineState> = {},
  propsOverrides: Partial<Pick<TimelineProps, "onLoadMore" | "onChangeKindToggle">> = {},
) {
  const defaults: TimelineProps = {
    state: makeState(stateOverrides),
    onLoadMore: vi.fn(),
    onChangeKindToggle: vi.fn(),
    ...propsOverrides,
  };
  return render(() => <Timeline {...defaults} />);
}

// ─── Unavailable capability ───────────────────────────────────────────────────

describe("unavailable capability", () => {
  it("shows timeline-unavailable when capability is unavailable", () => {
    renderTimeline({ capability: "unavailable" });
    expect(screen.getByTestId("timeline-unavailable")).toBeInTheDocument();
    expect(screen.getByTestId("timeline-unavailable")).toHaveTextContent(
      "Timeline is not available for this context.",
    );
  });

  it("does not render timeline-destination when capability is unavailable", () => {
    renderTimeline({ capability: "unavailable" });
    expect(screen.queryByTestId("timeline-destination")).not.toBeInTheDocument();
  });

  it("does not render capability badge when unavailable", () => {
    renderTimeline({ capability: "unavailable" });
    expect(screen.queryByTestId("timeline-capability")).not.toBeInTheDocument();
  });

  it("does not render any filter toggles when unavailable", () => {
    renderTimeline({ capability: "unavailable" });
    expect(screen.queryByTestId("timeline-filter-addition")).not.toBeInTheDocument();
    expect(screen.queryByTestId("timeline-filter-expiry")).not.toBeInTheDocument();
    expect(screen.queryByTestId("timeline-filter-contradiction")).not.toBeInTheDocument();
    expect(screen.queryByTestId("timeline-filter-supersession")).not.toBeInTheDocument();
    expect(screen.queryByTestId("timeline-filter-correction")).not.toBeInTheDocument();
  });

  it("does not render loading, error, list, empty, or load-more when unavailable", () => {
    renderTimeline({
      capability: "unavailable",
      isLoading: true,
      errorMessage: "some error",
      changes: [makeChange()],
      hasMore: true,
    });
    expect(screen.queryByTestId("timeline-loading")).not.toBeInTheDocument();
    expect(screen.queryByTestId("timeline-error")).not.toBeInTheDocument();
    expect(screen.queryByTestId("timeline-changes-list")).not.toBeInTheDocument();
    expect(screen.queryByTestId("timeline-load-more")).not.toBeInTheDocument();
  });
});

// ─── Available capability — root ──────────────────────────────────────────────

describe("available capability root", () => {
  it("renders timeline-destination for full capability", () => {
    renderTimeline({ capability: "full" });
    expect(screen.getByTestId("timeline-destination")).toBeInTheDocument();
  });

  it("renders timeline-destination for valid-time-only capability", () => {
    renderTimeline({ capability: "valid-time-only" });
    expect(screen.getByTestId("timeline-destination")).toBeInTheDocument();
  });

  it("renders timeline-destination for transaction-time-only capability", () => {
    renderTimeline({ capability: "transaction-time-only" });
    expect(screen.getByTestId("timeline-destination")).toBeInTheDocument();
  });

  it("does not render timeline-unavailable for full capability", () => {
    renderTimeline({ capability: "full" });
    expect(screen.queryByTestId("timeline-unavailable")).not.toBeInTheDocument();
  });
});

// ─── Capability badge ─────────────────────────────────────────────────────────

describe("capability badge", () => {
  const cases: TimelineCapability[] = ["full", "valid-time-only", "transaction-time-only"];

  for (const cap of cases) {
    it(`shows timeline-capability with data-capability="${cap}"`, () => {
      renderTimeline({ capability: cap });
      const badge = screen.getByTestId("timeline-capability");
      expect(badge).toBeInTheDocument();
      expect(badge).toHaveAttribute("data-capability", cap);
    });
  }
});

// ─── Query metadata — range ───────────────────────────────────────────────────

describe("range metadata", () => {
  it("shows timeline-range-start when rangeStart is non-null", () => {
    renderTimeline({ query: { rangeStart: "2024-01-01T00:00:00Z", rangeEnd: null, timezone: null, graphRevision: null, includeChangeKinds: [] } });
    expect(screen.getByTestId("timeline-range-start")).toHaveTextContent("2024-01-01T00:00:00Z");
  });

  it("hides timeline-range-start when rangeStart is null", () => {
    renderTimeline({ query: { rangeStart: null, rangeEnd: null, timezone: null, graphRevision: null, includeChangeKinds: [] } });
    expect(screen.queryByTestId("timeline-range-start")).not.toBeInTheDocument();
  });

  it("shows timeline-range-end when rangeEnd is non-null", () => {
    renderTimeline({ query: { rangeStart: null, rangeEnd: "2024-01-31T23:59:59Z", timezone: null, graphRevision: null, includeChangeKinds: [] } });
    expect(screen.getByTestId("timeline-range-end")).toHaveTextContent("2024-01-31T23:59:59Z");
  });

  it("hides timeline-range-end when rangeEnd is null", () => {
    renderTimeline({ query: { rangeStart: null, rangeEnd: null, timezone: null, graphRevision: null, includeChangeKinds: [] } });
    expect(screen.queryByTestId("timeline-range-end")).not.toBeInTheDocument();
  });
});

// ─── Query metadata — timezone ────────────────────────────────────────────────

describe("timezone metadata", () => {
  it("shows timeline-timezone when timezone is non-null", () => {
    renderTimeline({ query: { rangeStart: null, rangeEnd: null, timezone: "America/New_York", graphRevision: null, includeChangeKinds: [] } });
    expect(screen.getByTestId("timeline-timezone")).toHaveTextContent("America/New_York");
  });

  it("hides timeline-timezone when timezone is null", () => {
    renderTimeline({ query: { rangeStart: null, rangeEnd: null, timezone: null, graphRevision: null, includeChangeKinds: [] } });
    expect(screen.queryByTestId("timeline-timezone")).not.toBeInTheDocument();
  });

  it("shows UTC timezone when set", () => {
    renderTimeline({ query: { rangeStart: null, rangeEnd: null, timezone: "UTC", graphRevision: null, includeChangeKinds: [] } });
    expect(screen.getByTestId("timeline-timezone")).toHaveTextContent("UTC");
  });
});

// ─── Query metadata — revision ────────────────────────────────────────────────

describe("revision metadata", () => {
  it("shows timeline-revision when graphRevision is non-null", () => {
    renderTimeline({ query: { rangeStart: null, rangeEnd: null, timezone: null, graphRevision: 42, includeChangeKinds: [] } });
    expect(screen.getByTestId("timeline-revision")).toHaveTextContent("42");
  });

  it("hides timeline-revision when graphRevision is null", () => {
    renderTimeline({ query: { rangeStart: null, rangeEnd: null, timezone: null, graphRevision: null, includeChangeKinds: [] } });
    expect(screen.queryByTestId("timeline-revision")).not.toBeInTheDocument();
  });

  it("shows timeline-revision=0 when graphRevision is 0", () => {
    renderTimeline({ query: { rangeStart: null, rangeEnd: null, timezone: null, graphRevision: 0, includeChangeKinds: [] } });
    expect(screen.getByTestId("timeline-revision")).toHaveTextContent("0");
  });
});

// ─── Filter toggles — presence ────────────────────────────────────────────────

describe("filter toggle presence", () => {
  const allKinds: TimelineChangeKind[] = [
    "addition",
    "expiry",
    "contradiction",
    "supersession",
    "correction",
  ];

  it("renders all five filter toggle buttons when capability is available", () => {
    renderTimeline({ capability: "full" });
    for (const kind of allKinds) {
      expect(screen.getByTestId(`timeline-filter-${kind}`)).toBeInTheDocument();
    }
  });

  it("renders filter toggles for valid-time-only", () => {
    renderTimeline({ capability: "valid-time-only" });
    for (const kind of allKinds) {
      expect(screen.getByTestId(`timeline-filter-${kind}`)).toBeInTheDocument();
    }
  });

  it("renders filter toggles for transaction-time-only", () => {
    renderTimeline({ capability: "transaction-time-only" });
    for (const kind of allKinds) {
      expect(screen.getByTestId(`timeline-filter-${kind}`)).toBeInTheDocument();
    }
  });
});

// ─── Filter toggles — data-active ─────────────────────────────────────────────

describe("filter toggle data-active attribute", () => {
  it("data-active=true when kind is included in includeChangeKinds", () => {
    renderTimeline({
      query: {
        rangeStart: null,
        rangeEnd: null,
        timezone: null,
        graphRevision: null,
        includeChangeKinds: ["addition", "correction"],
      },
    });
    expect(screen.getByTestId("timeline-filter-addition")).toHaveAttribute("data-active", "true");
    expect(screen.getByTestId("timeline-filter-correction")).toHaveAttribute("data-active", "true");
  });

  it("data-active=false when kind is excluded from includeChangeKinds", () => {
    renderTimeline({
      query: {
        rangeStart: null,
        rangeEnd: null,
        timezone: null,
        graphRevision: null,
        includeChangeKinds: ["addition"],
      },
    });
    expect(screen.getByTestId("timeline-filter-expiry")).toHaveAttribute("data-active", "false");
    expect(screen.getByTestId("timeline-filter-contradiction")).toHaveAttribute("data-active", "false");
    expect(screen.getByTestId("timeline-filter-supersession")).toHaveAttribute("data-active", "false");
    expect(screen.getByTestId("timeline-filter-correction")).toHaveAttribute("data-active", "false");
  });

  it("data-active=false for all when includeChangeKinds is empty", () => {
    renderTimeline({
      query: {
        rangeStart: null,
        rangeEnd: null,
        timezone: null,
        graphRevision: null,
        includeChangeKinds: [],
      },
    });
    for (const kind of ["addition", "expiry", "contradiction", "supersession", "correction"] as TimelineChangeKind[]) {
      expect(screen.getByTestId(`timeline-filter-${kind}`)).toHaveAttribute("data-active", "false");
    }
  });

  it("data-active=true for all when all kinds included", () => {
    renderTimeline({
      query: {
        rangeStart: null,
        rangeEnd: null,
        timezone: null,
        graphRevision: null,
        includeChangeKinds: ["addition", "expiry", "contradiction", "supersession", "correction"],
      },
    });
    for (const kind of ["addition", "expiry", "contradiction", "supersession", "correction"] as TimelineChangeKind[]) {
      expect(screen.getByTestId(`timeline-filter-${kind}`)).toHaveAttribute("data-active", "true");
    }
  });
});

// ─── Filter toggle callbacks ──────────────────────────────────────────────────

describe("filter toggle callbacks", () => {
  const allKinds: TimelineChangeKind[] = [
    "addition",
    "expiry",
    "contradiction",
    "supersession",
    "correction",
  ];

  for (const kind of allKinds) {
    it(`clicking timeline-filter-${kind} calls onChangeKindToggle with "${kind}"`, () => {
      const onChangeKindToggle = vi.fn();
      renderTimeline({}, { onChangeKindToggle });
      fireEvent.click(screen.getByTestId(`timeline-filter-${kind}`));
      expect(onChangeKindToggle).toHaveBeenCalledOnce();
      expect(onChangeKindToggle).toHaveBeenCalledWith(kind);
    });
  }
});

// ─── Loading state ────────────────────────────────────────────────────────────

describe("loading state", () => {
  it("shows timeline-loading with role=status when isLoading=true", () => {
    renderTimeline({ isLoading: true });
    const el = screen.getByTestId("timeline-loading");
    expect(el).toBeInTheDocument();
    expect(el).toHaveAttribute("role", "status");
  });

  it("hides timeline-loading when isLoading=false", () => {
    renderTimeline({ isLoading: false });
    expect(screen.queryByTestId("timeline-loading")).not.toBeInTheDocument();
  });
});

// ─── Error state ──────────────────────────────────────────────────────────────

describe("error state", () => {
  it("shows timeline-error with role=alert when errorMessage is non-null", () => {
    renderTimeline({ errorMessage: "Temporal query failed" });
    const el = screen.getByTestId("timeline-error");
    expect(el).toBeInTheDocument();
    expect(el).toHaveAttribute("role", "alert");
    expect(el).toHaveTextContent("Temporal query failed");
  });

  it("hides timeline-error when errorMessage is null", () => {
    renderTimeline({ errorMessage: null });
    expect(screen.queryByTestId("timeline-error")).not.toBeInTheDocument();
  });
});

// ─── Changes list ─────────────────────────────────────────────────────────────

describe("changes list", () => {
  it("shows timeline-changes-list with role=list when changes non-empty", () => {
    renderTimeline({ changes: [makeChange()] });
    const list = screen.getByTestId("timeline-changes-list");
    expect(list).toBeInTheDocument();
    expect(list).toHaveAttribute("role", "list");
  });

  it("hides timeline-changes-list when changes array is empty", () => {
    renderTimeline({ changes: [] });
    expect(screen.queryByTestId("timeline-changes-list")).not.toBeInTheDocument();
  });
});

// ─── Empty state ──────────────────────────────────────────────────────────────

describe("empty state", () => {
  it("shows timeline-empty when changes empty and not loading", () => {
    renderTimeline({ changes: [], isLoading: false });
    expect(screen.getByTestId("timeline-empty")).toBeInTheDocument();
  });

  it("hides timeline-empty when changes non-empty", () => {
    renderTimeline({ changes: [makeChange()], isLoading: false });
    expect(screen.queryByTestId("timeline-empty")).not.toBeInTheDocument();
  });

  it("hides timeline-empty while loading", () => {
    renderTimeline({ changes: [], isLoading: true });
    expect(screen.queryByTestId("timeline-empty")).not.toBeInTheDocument();
  });
});

// ─── Individual change fields ─────────────────────────────────────────────────

describe("change item fields", () => {
  it("renders timeline-change-{id} with data-change-kind", () => {
    renderTimeline({ changes: [makeChange({ id: "c1", kind: "addition" })] });
    const item = screen.getByTestId("timeline-change-c1");
    expect(item).toBeInTheDocument();
    expect(item).toHaveAttribute("data-change-kind", "addition");
  });

  it("renders change-kind-{id} with kind label", () => {
    renderTimeline({ changes: [makeChange({ id: "c1", kind: "expiry" })] });
    expect(screen.getByTestId("change-kind-c1")).toHaveTextContent("Expiry");
  });

  it("renders change-label-{id} with backend label", () => {
    renderTimeline({ changes: [makeChange({ id: "c1", label: "Entity fact expired" })] });
    expect(screen.getByTestId("change-label-c1")).toHaveTextContent("Entity fact expired");
  });

  it("renders change-description-{id} with backend description", () => {
    renderTimeline({ changes: [makeChange({ id: "c1", description: "Valid time ended 2024-01-01" })] });
    expect(screen.getByTestId("change-description-c1")).toHaveTextContent("Valid time ended 2024-01-01");
  });

  it("renders change-transaction-time-{id}", () => {
    renderTimeline({ changes: [makeChange({ id: "c1", transactionTime: "2024-06-01T08:00:00Z" })] });
    expect(screen.getByTestId("change-transaction-time-c1")).toHaveTextContent("2024-06-01T08:00:00Z");
  });

  it("renders change-revision-{id}", () => {
    renderTimeline({ changes: [makeChange({ id: "c1", transactionRevision: 77 })] });
    expect(screen.getByTestId("change-revision-c1")).toHaveTextContent("77");
  });
});

// ─── validTimeStart / validTimeEnd conditional ────────────────────────────────

describe("change valid time fields", () => {
  it("renders change-valid-start-{id} when validTimeStart is non-null", () => {
    renderTimeline({
      changes: [makeChange({ id: "c1", validTimeStart: "2023-01-01T00:00:00Z" })],
    });
    expect(screen.getByTestId("change-valid-start-c1")).toHaveTextContent("2023-01-01T00:00:00Z");
  });

  it("hides change-valid-start-{id} when validTimeStart is null", () => {
    renderTimeline({ changes: [makeChange({ id: "c1", validTimeStart: null })] });
    expect(screen.queryByTestId("change-valid-start-c1")).not.toBeInTheDocument();
  });

  it("renders change-valid-end-{id} when validTimeEnd is non-null", () => {
    renderTimeline({
      changes: [makeChange({ id: "c1", validTimeEnd: "2024-12-31T23:59:59Z" })],
    });
    expect(screen.getByTestId("change-valid-end-c1")).toHaveTextContent("2024-12-31T23:59:59Z");
  });

  it("hides change-valid-end-{id} when validTimeEnd is null", () => {
    renderTimeline({ changes: [makeChange({ id: "c1", validTimeEnd: null })] });
    expect(screen.queryByTestId("change-valid-end-c1")).not.toBeInTheDocument();
  });
});

// ─── All five change kind variants ────────────────────────────────────────────

describe("all five change kind variants", () => {
  const cases: Array<[TimelineChangeKind, string]> = [
    ["addition", "Addition"],
    ["expiry", "Expiry"],
    ["contradiction", "Contradiction"],
    ["supersession", "Supersession"],
    ["correction", "Correction"],
  ];

  for (const [kind, expectedLabel] of cases) {
    it(`renders kind label "${expectedLabel}" for kind "${kind}"`, () => {
      renderTimeline({
        changes: [makeChange({ id: `c-${kind}`, kind })],
      });
      const item = screen.getByTestId(`timeline-change-c-${kind}`);
      expect(item).toHaveAttribute("data-change-kind", kind);
      expect(screen.getByTestId(`change-kind-c-${kind}`)).toHaveTextContent(expectedLabel);
    });
  }

  it("renders all five change kinds together with correct attributes", () => {
    const changes: TimelineChange[] = [
      makeChange({ id: "c1", kind: "addition" }),
      makeChange({ id: "c2", kind: "expiry" }),
      makeChange({ id: "c3", kind: "contradiction" }),
      makeChange({ id: "c4", kind: "supersession" }),
      makeChange({ id: "c5", kind: "correction" }),
    ];
    renderTimeline({ changes });
    const list = screen.getByTestId("timeline-changes-list");
    const items = list.querySelectorAll("[role='listitem']");
    expect(items).toHaveLength(5);

    expect(screen.getByTestId("timeline-change-c1")).toHaveAttribute("data-change-kind", "addition");
    expect(screen.getByTestId("timeline-change-c2")).toHaveAttribute("data-change-kind", "expiry");
    expect(screen.getByTestId("timeline-change-c3")).toHaveAttribute("data-change-kind", "contradiction");
    expect(screen.getByTestId("timeline-change-c4")).toHaveAttribute("data-change-kind", "supersession");
    expect(screen.getByTestId("timeline-change-c5")).toHaveAttribute("data-change-kind", "correction");
  });
});

// ─── Load more ────────────────────────────────────────────────────────────────

describe("load more", () => {
  it("shows timeline-load-more when hasMore=true", () => {
    renderTimeline({ hasMore: true });
    expect(screen.getByTestId("timeline-load-more")).toBeInTheDocument();
  });

  it("hides timeline-load-more when hasMore=false", () => {
    renderTimeline({ hasMore: false });
    expect(screen.queryByTestId("timeline-load-more")).not.toBeInTheDocument();
  });

  it("calls onLoadMore when timeline-load-more is clicked", () => {
    const onLoadMore = vi.fn();
    renderTimeline({ hasMore: true }, { onLoadMore });
    fireEvent.click(screen.getByTestId("timeline-load-more"));
    expect(onLoadMore).toHaveBeenCalledOnce();
  });

  it("does not show timeline-load-more when unavailable even if hasMore=true", () => {
    renderTimeline({ capability: "unavailable", hasMore: true });
    expect(screen.queryByTestId("timeline-load-more")).not.toBeInTheDocument();
  });
});
