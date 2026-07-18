import { describe, it, expect, vi } from "vitest";
import { render, fireEvent } from "@solidjs/testing-library";
import { Card } from "./Card";

describe("Card", () => {
  it("renders a non-interactive container by default", () => {
    const { queryByRole, getByText } = render(() => <Card>Body</Card>);
    expect(getByText("Body")).toBeInTheDocument();
    expect(queryByRole("button")).toBeNull();
  });

  it("becomes a keyboard-operable button when interactive", () => {
    const onClick = vi.fn();
    const { getByRole } = render(() => (
      <Card onClick={onClick} aria-label="Open">
        Body
      </Card>
    ));
    const btn = getByRole("button", { name: "Open" });
    fireEvent.click(btn);
    expect(onClick).toHaveBeenCalledOnce();
    expect(btn.className).toContain("kit-focusable");
  });
});
