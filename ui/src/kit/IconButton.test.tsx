import { describe, it, expect, vi } from "vitest";
import { render, fireEvent } from "@solidjs/testing-library";
import { IconButton } from "./IconButton";

describe("IconButton", () => {
  it("exposes its label as the accessible name", () => {
    const { getByRole } = render(() => <IconButton icon="x" label="Close" />);
    expect(getByRole("button", { name: "Close" })).toBeInTheDocument();
  });

  it("renders the requested sprite icon", () => {
    const { container } = render(() => <IconButton icon="search" label="Search" />);
    expect(container.querySelector("use")?.getAttribute("href")).toBe(
      "/icons/lucide-sprite.svg#search",
    );
  });

  it("fires onClick when activated", () => {
    const onClick = vi.fn();
    const { getByRole } = render(() => (
      <IconButton icon="x" label="Close" onClick={onClick} />
    ));
    fireEvent.click(getByRole("button"));
    expect(onClick).toHaveBeenCalledOnce();
  });

  it("does not fire onClick when disabled", () => {
    const onClick = vi.fn();
    const { getByRole } = render(() => (
      <IconButton icon="x" label="Close" disabled onClick={onClick} />
    ));
    fireEvent.click(getByRole("button"));
    expect(onClick).not.toHaveBeenCalled();
  });
});
