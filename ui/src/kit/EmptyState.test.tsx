import { describe, it, expect } from "vitest";
import { render } from "@solidjs/testing-library";
import { EmptyState } from "./EmptyState";

describe("EmptyState", () => {
  it("renders the title as a heading", () => {
    const { getByRole } = render(() => <EmptyState title="Nothing here" />);
    expect(getByRole("heading", { name: "Nothing here" })).toBeInTheDocument();
  });

  it("renders description and action when provided", () => {
    const { getByText, getByRole } = render(() => (
      <EmptyState
        title="Empty"
        description="Add something"
        action={<button>Add</button>}
      />
    ));
    expect(getByText("Add something")).toBeInTheDocument();
    expect(getByRole("button", { name: "Add" })).toBeInTheDocument();
  });

  it("renders the decorative icon via the sprite", () => {
    const { container } = render(() => <EmptyState icon="brain" title="X" />);
    expect(container.querySelector("use")?.getAttribute("href")).toBe(
      "/icons/lucide-sprite.svg#brain",
    );
  });
});
