import { describe, it, expect, vi } from "vitest";
import { render, fireEvent } from "@solidjs/testing-library";
import { SegmentBar } from "./SegmentBar";

const options = [
  { value: "a", label: "Alpha" },
  { value: "b", label: "Beta" },
  { value: "c", label: "Gamma" },
];

describe("SegmentBar", () => {
  it("exposes a labeled group of segments", () => {
    const { getByRole } = render(() => <SegmentBar label="Choice" options={options} />);
    expect(getByRole("group", { name: "Choice" })).toBeInTheDocument();
  });

  it("renders each option as a toggle button", () => {
    const { getAllByRole } = render(() => (
      <SegmentBar label="Choice" options={options} />
    ));
    expect(getAllByRole("button").length).toBe(3);
  });

  it("reflects the selected segment via aria-pressed", () => {
    const { getByRole } = render(() => (
      <SegmentBar label="Choice" options={options} defaultValue="b" />
    ));
    expect(getByRole("button", { name: "Beta" })).toHaveAttribute("aria-pressed", "true");
  });

  it("emits the chosen value", () => {
    const onChange = vi.fn();
    const { getByRole } = render(() => (
      <SegmentBar label="Choice" options={options} onChange={onChange} />
    ));
    fireEvent.click(getByRole("button", { name: "Beta" }));
    expect(onChange).toHaveBeenCalledWith("b");
  });
});
