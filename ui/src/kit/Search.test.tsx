import { describe, it, expect, vi } from "vitest";
import { render, fireEvent } from "@solidjs/testing-library";
import { Search } from "./Search";

describe("Search", () => {
  it("renders a labeled searchbox", () => {
    const { getByRole } = render(() => <Search label="Find" />);
    expect(getByRole("searchbox", { name: "Find" })).toBeInTheDocument();
  });

  it("defaults the accessible name to 'Search' when no label is given", () => {
    const { getByRole } = render(() => <Search />);
    expect(getByRole("searchbox", { name: "Search" })).toBeInTheDocument();
  });

  it("emits changed value", () => {
    const onChange = vi.fn();
    const { getByRole } = render(() => <Search onChange={onChange} />);
    fireEvent.input(getByRole("searchbox"), { target: { value: "memory" } });
    expect(onChange).toHaveBeenCalledWith("memory");
  });

  it("renders the leading search icon", () => {
    const { container } = render(() => <Search />);
    expect(container.querySelector("use")?.getAttribute("href")).toBe(
      "/icons/lucide-sprite.svg#search",
    );
  });
});
