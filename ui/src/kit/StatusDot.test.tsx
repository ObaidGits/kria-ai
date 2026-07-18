import { describe, it, expect } from "vitest";
import { render } from "@solidjs/testing-library";
import { StatusDot } from "./StatusDot";

describe("StatusDot", () => {
  it("exposes the label as a status region (not color-only)", () => {
    const { getByRole } = render(() => <StatusDot tone="online" label="Online" />);
    expect(getByRole("status")).toHaveTextContent("Online");
  });

  it("keeps the label for assistive tech even when visually hidden", () => {
    const { getByText } = render(() => (
      <StatusDot tone="error" label="Disconnected" hideLabel />
    ));
    expect(getByText("Disconnected").className).toContain("kit-visually-hidden");
  });

  it("applies the tone class", () => {
    const { getByRole } = render(() => <StatusDot tone="busy" label="Busy" />);
    expect(getByRole("status").className).toContain("kit-status--busy");
  });
});
