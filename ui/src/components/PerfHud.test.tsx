import { afterEach, describe, expect, it } from "vitest";
import { render, cleanup } from "@solidjs/testing-library";
import PerfHud, { PERF_HUD_ENABLED } from "./PerfHud";
import { clearMeasures, measureSince } from "../utils/perf";

afterEach(() => {
  cleanup();
  clearMeasures();
});

describe("PerfHud", () => {
  it("is enabled in dev/test builds", () => {
    expect(PERF_HUD_ENABLED).toBe(true);
  });

  it("renders the HUD with an accessible label and empty state", () => {
    const { getByLabelText, getByText } = render(() => <PerfHud />);
    expect(getByLabelText("Performance HUD")).toBeInTheDocument();
    expect(getByText("No measures yet.")).toBeInTheDocument();
  });

  it("shows new measures as they are recorded", async () => {
    const { findByText } = render(() => <PerfHud />);
    measureSince("palette-open", performance.now());
    expect(await findByText(/palette-open/)).toBeInTheDocument();
  });

  it("flags an over-budget measure", async () => {
    const { findByLabelText } = render(() => <PerfHud />);
    // 500ms ago exceeds the 150ms space-switch budget.
    measureSince("space-switch", performance.now() - 500);
    expect(await findByLabelText(/over budget/)).toBeInTheDocument();
  });
});
