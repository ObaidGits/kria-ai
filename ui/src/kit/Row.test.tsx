import { describe, it, expect, vi } from "vitest";
import { render, fireEvent } from "@solidjs/testing-library";
import { Row } from "./Row";

describe("Row", () => {
  it("renders a static row without a button role", () => {
    const { queryByRole, getByText } = render(() => <Row title="Item" />);
    expect(getByText("Item")).toBeInTheDocument();
    expect(queryByRole("button")).toBeNull();
  });

  it("becomes a selectable button exposing aria-selected", () => {
    const onSelect = vi.fn();
    const { getByRole } = render(() => (
      <Row title="Pick me" selected onSelect={onSelect} />
    ));
    const btn = getByRole("button", { name: "Pick me" });
    expect(btn).toHaveAttribute("aria-selected", "true");
    fireEvent.click(btn);
    expect(onSelect).toHaveBeenCalledOnce();
  });

  it("does not fire when disabled", () => {
    const onSelect = vi.fn();
    const { getByRole } = render(() => (
      <Row title="Nope" onSelect={onSelect} disabled />
    ));
    fireEvent.click(getByRole("button"));
    expect(onSelect).not.toHaveBeenCalled();
  });
});
