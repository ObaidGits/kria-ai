import { describe, it, expect, vi } from "vitest";
import { render, fireEvent } from "@solidjs/testing-library";
import { Button } from "./Button";

describe("Button", () => {
  it("renders a semantic button with its label", () => {
    const { getByRole } = render(() => <Button>Save</Button>);
    expect(getByRole("button", { name: "Save" })).toBeInTheDocument();
  });

  it("fires onClick when activated", () => {
    const onClick = vi.fn();
    const { getByRole } = render(() => <Button onClick={onClick}>Go</Button>);
    fireEvent.click(getByRole("button"));
    expect(onClick).toHaveBeenCalledOnce();
  });

  it("does not fire onClick when disabled", () => {
    const onClick = vi.fn();
    const { getByRole } = render(() => (
      <Button disabled onClick={onClick}>
        Go
      </Button>
    ));
    fireEvent.click(getByRole("button"));
    expect(onClick).not.toHaveBeenCalled();
  });

  it("applies the requested variant and size classes (focusable ring included)", () => {
    const { getByRole } = render(() => (
      <Button variant="danger" size="lg">
        Delete
      </Button>
    ));
    const btn = getByRole("button");
    expect(btn.className).toContain("kit-button--danger");
    expect(btn.className).toContain("kit-button--lg");
    expect(btn.className).toContain("kit-focusable");
  });

  it("is keyboard focusable", () => {
    const { getByRole } = render(() => <Button>Focus me</Button>);
    const btn = getByRole("button");
    btn.focus();
    expect(document.activeElement).toBe(btn);
  });
});
