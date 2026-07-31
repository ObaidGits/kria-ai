/**
 * Tests for UseSectionDetail component.
 *
 * Verifies:
 *   - Root element render
 *   - whyStored: shows text or fallback
 *   - whyRecalled: shows text or fallback
 *   - howUsed: shows text or fallback
 *   - Trace injections list visibility and content
 *   - Trace navigation button calls onNavigate with correct target
 *   - Filtered reasons visibility and content (count + type only, no content)
 *   - usedInTraceCount visibility
 *
 * Requirements: F4.4 (task 4.4.3)
 */
import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@solidjs/testing-library";
import { UseSectionDetail } from "./UseSectionDetail";
import type { UseSectionDetailData } from "./UseSectionDetail";

// ─── Helpers ──────────────────────────────────────────────────────────────────

function makeData(overrides: Partial<UseSectionDetailData> = {}): UseSectionDetailData {
  return {
    whyStored: null,
    whyRecalled: null,
    howUsed: null,
    traceInjections: [],
    filteredReasons: [],
    usedInTraceCount: null,
    ...overrides,
  };
}

// ─── Tests ────────────────────────────────────────────────────────────────────

describe("UseSectionDetail", () => {

  // Root element
  it("renders root element with correct testid", () => {
    render(() => (
      <UseSectionDetail data={makeData()} onNavigate={() => {}} />
    ));
    expect(screen.getByTestId("use-section-detail")).toBeTruthy();
  });

  // Why stored
  it("shows whyStored text when provided", () => {
    render(() => (
      <UseSectionDetail
        data={makeData({ whyStored: "Stored because user confirmed this fact" })}
        onNavigate={() => {}}
      />
    ));
    const el = screen.getByTestId("why-stored");
    expect(el.textContent).toBe("Stored because user confirmed this fact");
  });

  it('shows "No storage rationale available" when whyStored is null', () => {
    render(() => (
      <UseSectionDetail data={makeData({ whyStored: null })} onNavigate={() => {}} />
    ));
    const el = screen.getByTestId("why-stored");
    expect(el.textContent).toBe("No storage rationale available");
  });

  // Why recalled
  it("shows whyRecalled text when provided", () => {
    render(() => (
      <UseSectionDetail
        data={makeData({ whyRecalled: "Recalled because it matched query context" })}
        onNavigate={() => {}}
      />
    ));
    const el = screen.getByTestId("why-recalled");
    expect(el.textContent).toBe("Recalled because it matched query context");
  });

  it('shows "Not recalled in current context" when whyRecalled is null', () => {
    render(() => (
      <UseSectionDetail data={makeData({ whyRecalled: null })} onNavigate={() => {}} />
    ));
    const el = screen.getByTestId("why-recalled");
    expect(el.textContent).toBe("Not recalled in current context");
  });

  // How used
  it("shows howUsed text when provided", () => {
    render(() => (
      <UseSectionDetail
        data={makeData({ howUsed: "Injected as supporting context for the answer" })}
        onNavigate={() => {}}
      />
    ));
    const el = screen.getByTestId("how-used");
    expect(el.textContent).toBe("Injected as supporting context for the answer");
  });

  it('shows "Not used in current context" when howUsed is null', () => {
    render(() => (
      <UseSectionDetail data={makeData({ howUsed: null })} onNavigate={() => {}} />
    ));
    const el = screen.getByTestId("how-used");
    expect(el.textContent).toBe("Not used in current context");
  });

  // Trace injections
  it("shows trace injections list when traceInjections is non-empty", () => {
    render(() => (
      <UseSectionDetail
        data={makeData({
          traceInjections: [
            {
              traceId: "trace-abc",
              traceLabel: "Trace Alpha",
              navigationTarget: "recall?trace=trace-abc",
              wasInjected: true,
            },
          ],
        })}
        onNavigate={() => {}}
      />
    ));
    expect(screen.getByTestId("trace-injections-list")).toBeTruthy();
  });

  it("hides trace injections list when traceInjections is empty", () => {
    render(() => (
      <UseSectionDetail data={makeData({ traceInjections: [] })} onNavigate={() => {}} />
    ));
    expect(screen.queryByTestId("trace-injections-list")).toBeNull();
  });

  it("shows traceLabel for each injection when traceLabel is provided", () => {
    render(() => (
      <UseSectionDetail
        data={makeData({
          traceInjections: [
            {
              traceId: "trace-abc",
              traceLabel: "Trace Alpha",
              navigationTarget: "recall?trace=trace-abc",
              wasInjected: true,
            },
          ],
        })}
        onNavigate={() => {}}
      />
    ));
    const item = screen.getByTestId("trace-injection-trace-abc");
    expect(item.textContent).toContain("Trace Alpha");
  });

  it("shows traceId for each injection when traceLabel is null", () => {
    render(() => (
      <UseSectionDetail
        data={makeData({
          traceInjections: [
            {
              traceId: "trace-xyz",
              traceLabel: null,
              navigationTarget: "recall?trace=trace-xyz",
              wasInjected: true,
            },
          ],
        })}
        onNavigate={() => {}}
      />
    ));
    const item = screen.getByTestId("trace-injection-trace-xyz");
    expect(item.textContent).toContain("trace-xyz");
  });

  it("calls onNavigate with correct target when trace nav button is clicked", () => {
    const onNavigate = vi.fn();
    render(() => (
      <UseSectionDetail
        data={makeData({
          traceInjections: [
            {
              traceId: "trace-abc",
              traceLabel: "Trace Alpha",
              navigationTarget: "recall?trace=trace-abc",
              wasInjected: true,
            },
          ],
        })}
        onNavigate={onNavigate}
      />
    ));
    const btn = screen.getByTestId("trace-navigate-trace-abc");
    btn.click();
    expect(onNavigate).toHaveBeenCalledOnce();
    expect(onNavigate).toHaveBeenCalledWith("recall?trace=trace-abc");
  });

  // Filtered reasons
  it("shows filtered reasons section when filteredReasons is non-empty", () => {
    render(() => (
      <UseSectionDetail
        data={makeData({
          filteredReasons: [
            { filterType: "non-injected", count: 3 },
          ],
        })}
        onNavigate={() => {}}
      />
    ));
    expect(screen.getByTestId("filtered-reasons")).toBeTruthy();
  });

  it("hides filtered reasons section when filteredReasons is empty", () => {
    render(() => (
      <UseSectionDetail data={makeData({ filteredReasons: [] })} onNavigate={() => {}} />
    ));
    expect(screen.queryByTestId("filtered-reasons")).toBeNull();
  });

  it("shows count and filterType for each filtered reason", () => {
    render(() => (
      <UseSectionDetail
        data={makeData({
          filteredReasons: [
            { filterType: "non-injected", count: 3 },
            { filterType: "policy-filtered", count: 1 },
          ],
        })}
        onNavigate={() => {}}
      />
    ));
    const r1 = screen.getByTestId("filtered-reason-non-injected");
    expect(r1.textContent).toContain("3");
    expect(r1.textContent).toContain("non-injected");

    const r2 = screen.getByTestId("filtered-reason-policy-filtered");
    expect(r2.textContent).toContain("1");
    expect(r2.textContent).toContain("policy-filtered");
  });

  it("does not expose content of filtered reasons — only count and type", () => {
    // This test ensures the component only renders count + filterType, not any
    // payload or content from the filtered items (which we never receive in the data type).
    render(() => (
      <UseSectionDetail
        data={makeData({
          filteredReasons: [
            { filterType: "below-threshold", count: 7 },
          ],
        })}
        onNavigate={() => {}}
      />
    ));
    const el = screen.getByTestId("filtered-reason-below-threshold");
    // Should contain the count and type
    expect(el.textContent).toContain("7");
    expect(el.textContent).toContain("below-threshold");
    // The data type carries no content field, so there's nothing else to leak.
    // Verify no stray text beyond count + space + filterType is present.
    const text = el.textContent?.replace(/\s+/g, " ").trim();
    expect(text).toBe("7 below-threshold");
  });

  // Used in trace count
  it("shows used-in-trace-count when usedInTraceCount is non-null", () => {
    render(() => (
      <UseSectionDetail
        data={makeData({ usedInTraceCount: 5 })}
        onNavigate={() => {}}
      />
    ));
    const el = screen.getByTestId("used-in-trace-count");
    expect(el.textContent).toBe("5");
  });

  it("hides used-in-trace-count when usedInTraceCount is null", () => {
    render(() => (
      <UseSectionDetail data={makeData({ usedInTraceCount: null })} onNavigate={() => {}} />
    ));
    expect(screen.queryByTestId("used-in-trace-count")).toBeNull();
  });

});
