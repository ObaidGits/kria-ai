import { describe, it, expect } from "vitest";
import { render } from "@solidjs/testing-library";
import { Badge } from "./Badge";

describe("Badge", () => {
  it("renders its text content", () => {
    const { getByText } = render(() => <Badge>New</Badge>);
    expect(getByText("New")).toBeInTheDocument();
  });

  it("applies the tone class", () => {
    const { getByText } = render(() => <Badge tone="danger">Error</Badge>);
    expect(getByText("Error").className).toContain("kit-badge--danger");
  });

  it("defaults to the neutral tone", () => {
    const { getByText } = render(() => <Badge>Info</Badge>);
    expect(getByText("Info").className).toContain("kit-badge--neutral");
  });
});
