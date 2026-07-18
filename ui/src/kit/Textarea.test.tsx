import { describe, it, expect, vi } from "vitest";
import { render, fireEvent } from "@solidjs/testing-library";
import { Textarea } from "./Textarea";

describe("Textarea", () => {
  it("associates the label with the textbox", () => {
    const { getByRole } = render(() => <Textarea label="Notes" />);
    expect(getByRole("textbox", { name: "Notes" })).toBeInTheDocument();
  });

  it("emits changed value", () => {
    const onChange = vi.fn();
    const { getByRole } = render(() => <Textarea label="Notes" onChange={onChange} />);
    fireEvent.input(getByRole("textbox"), { target: { value: "multi\nline" } });
    expect(onChange).toHaveBeenCalledWith("multi\nline");
  });

  it("marks invalid and shows error text", () => {
    const { getByRole, getByText } = render(() => (
      <Textarea label="Notes" errorMessage="Required" />
    ));
    expect(getByRole("textbox")).toHaveAttribute("aria-invalid", "true");
    expect(getByText("Required")).toBeInTheDocument();
  });

  it("disables the control", () => {
    const { getByRole } = render(() => <Textarea label="Notes" disabled />);
    expect(getByRole("textbox")).toBeDisabled();
  });
});
