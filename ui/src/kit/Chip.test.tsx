import { describe, it, expect, vi } from "vitest";
import { render, fireEvent } from "@solidjs/testing-library";
import { Chip } from "./Chip";

describe("Chip", () => {
  it("renders static content without a button role", () => {
    const { queryByRole, getByText } = render(() => <Chip>Tag</Chip>);
    expect(getByText("Tag")).toBeInTheDocument();
    expect(queryByRole("button")).toBeNull();
  });

  it("acts as a toggle exposing aria-pressed", () => {
    const onToggle = vi.fn();
    const { getByRole } = render(() => (
      <Chip selected onToggle={onToggle}>
        Filter
      </Chip>
    ));
    const btn = getByRole("button", { name: "Filter" });
    expect(btn).toHaveAttribute("aria-pressed", "true");
    fireEvent.click(btn);
    expect(onToggle).toHaveBeenCalledOnce();
  });

  it("exposes a labeled remove control", () => {
    const onRemove = vi.fn();
    const { getByRole } = render(() => (
      <Chip onRemove={onRemove} removeLabel="Remove tag">
        Tag
      </Chip>
    ));
    fireEvent.click(getByRole("button", { name: "Remove tag" }));
    expect(onRemove).toHaveBeenCalledOnce();
  });
});
