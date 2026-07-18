import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, describe, expect, it } from "vitest";
import { CoachHint } from "./CoachHint";
import { retireCoachHint } from "./presentationRanking";

afterEach(cleanup);

describe("CoachHint (Req 19.4)", () => {
  it("dismisses once and does not return on remount", () => {
    const featureId = `coach-dismiss-${Date.now()}-${Math.random()}`;
    const first = render(() => <CoachHint featureId={featureId}>Try this feature.</CoachHint>);
    expect(screen.getByRole("note", { name: "Getting started hint" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Dismiss hint" }));
    expect(screen.queryByRole("note", { name: "Getting started hint" })).toBeNull();
    first.unmount();
    render(() => <CoachHint featureId={featureId}>Try this feature.</CoachHint>);
    expect(screen.queryByRole("note", { name: "Getting started hint" })).toBeNull();
  });

  it("retires when the owning feature reports real use", () => {
    const featureId = `coach-use-${Date.now()}-${Math.random()}`;
    retireCoachHint(featureId);
    render(() => <CoachHint featureId={featureId}>Try this feature.</CoachHint>);
    expect(screen.queryByRole("note", { name: "Getting started hint" })).toBeNull();
  });
});
