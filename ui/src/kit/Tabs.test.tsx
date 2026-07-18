import { describe, it, expect, vi } from "vitest";
import { render, fireEvent } from "@solidjs/testing-library";
import { Tabs } from "./Tabs";

const items = [
  { value: "one", label: "One", content: () => <div>Panel one</div> },
  { value: "two", label: "Two", content: () => <div>Panel two</div> },
];

describe("Tabs", () => {
  it("renders a tablist with tabs", () => {
    const { getByRole, getAllByRole } = render(() => <Tabs items={items} />);
    expect(getByRole("tablist")).toBeInTheDocument();
    expect(getAllByRole("tab").length).toBe(2);
  });

  it("shows the selected tab's panel", () => {
    const { getByText } = render(() => <Tabs items={items} defaultValue="one" />);
    expect(getByText("Panel one")).toBeVisible();
  });

  it("marks the default selected tab with aria-selected", () => {
    const { getByRole } = render(() => <Tabs items={items} defaultValue="one" />);
    expect(getByRole("tab", { name: "One" })).toHaveAttribute("aria-selected", "true");
    expect(getByRole("tab", { name: "Two" })).toHaveAttribute("aria-selected", "false");
  });

  it("switches selection and emits the new value on click", () => {
    const onChange = vi.fn();
    const { getByRole, getByText } = render(() => (
      <Tabs items={items} onChange={onChange} />
    ));
    fireEvent.click(getByRole("tab", { name: "Two" }));
    expect(onChange).toHaveBeenCalledWith("two");
    expect(getByRole("tab", { name: "Two" })).toHaveAttribute("aria-selected", "true");
    expect(getByText("Panel two")).toBeVisible();
  });
});
