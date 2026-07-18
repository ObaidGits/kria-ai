import { describe, it, expect, vi } from "vitest";
import { render, fireEvent } from "@solidjs/testing-library";
import { Input } from "./Input";

describe("Input", () => {
  it("associates the label with the textbox", () => {
    const { getByRole } = render(() => <Input label="Name" />);
    expect(getByRole("textbox", { name: "Name" })).toBeInTheDocument();
  });

  it("emits changed value", () => {
    const onChange = vi.fn();
    const { getByRole } = render(() => <Input label="Name" onChange={onChange} />);
    const input = getByRole("textbox") as HTMLInputElement;
    fireEvent.input(input, { target: { value: "hello" } });
    expect(onChange).toHaveBeenCalledWith("hello");
  });

  it("marks the control invalid and shows the error text", () => {
    const { getByRole, getByText } = render(() => (
      <Input label="Name" errorMessage="Required" />
    ));
    expect(getByRole("textbox")).toHaveAttribute("aria-invalid", "true");
    expect(getByText("Required")).toBeInTheDocument();
  });

  it("disables the control", () => {
    const { getByRole } = render(() => <Input label="Name" disabled />);
    expect(getByRole("textbox")).toBeDisabled();
  });
});
