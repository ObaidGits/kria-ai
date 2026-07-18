import { describe, it, expect } from "vitest";
import { render } from "@solidjs/testing-library";
import { Select, type SelectOption } from "./Select";

const options: SelectOption[] = [
  { value: "a", label: "Alpha" },
  { value: "b", label: "Beta" },
];

describe("Select", () => {
  it("renders a labeled trigger button", () => {
    const { getByRole } = render(() => <Select label="Choice" options={options} />);
    const trigger = getByRole("button");
    expect(trigger).toHaveAttribute("aria-haspopup", "listbox");
  });

  it("shows the selected value", () => {
    const { getByRole } = render(() => (
      <Select label="Choice" options={options} defaultValue="b" />
    ));
    expect(getByRole("button")).toHaveTextContent("Beta");
  });

  it("shows the placeholder when nothing is selected", () => {
    const { getByRole } = render(() => (
      <Select label="Choice" options={options} placeholder="Pick one" />
    ));
    expect(getByRole("button")).toHaveTextContent("Pick one");
  });

  it("reflects the disabled state", () => {
    const { getByRole } = render(() => (
      <Select label="Choice" options={options} disabled />
    ));
    expect(getByRole("button")).toHaveAttribute("data-disabled");
  });
});
