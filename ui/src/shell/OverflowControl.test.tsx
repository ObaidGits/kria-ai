import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@solidjs/testing-library";
import { OverflowControl } from "./OverflowControl";

// Kobalte opens via keyboard (Enter). Content portals to document.body → query
// via `screen`. Mirrors ui/src/kit/Menu.test.tsx.
function openMenu(name: string | RegExp) {
  const trigger = screen.getByRole("button", { name });
  trigger.focus();
  fireEvent.keyDown(trigger, { key: "Enter" });
  return trigger;
}

describe("OverflowControl", () => {
  it("renders a labelled trigger (default 'More actions')", () => {
    render(() => <OverflowControl items={[{ id: "a", label: "Export" }]} />);
    expect(screen.getByRole("button", { name: /More actions/ })).toBeInTheDocument();
  });

  it("uses a custom base label when provided", () => {
    render(() => <OverflowControl label="Toolbar actions" items={[{ id: "a", label: "Export" }]} />);
    expect(screen.getByRole("button", { name: /Toolbar actions/ })).toBeInTheDocument();
  });

  it("folds waiting/error counts and state into the trigger accessible name", () => {
    render(() => (
      <OverflowControl
        items={[{ id: "a", label: "Retry" }]}
        waitingCount={3}
        errorCount={1}
        state="attention needed"
      />
    ));
    const trigger = screen.getByRole("button");
    const name = trigger.getAttribute("aria-label") ?? "";
    expect(name).toContain("More actions");
    expect(name).toContain("3 waiting");
    expect(name).toContain("1 error");
    expect(name).toContain("attention needed");
  });

  it("shows a decorative badge reflecting the combined count", () => {
    render(() => <OverflowControl items={[{ id: "a", label: "Retry" }]} waitingCount={2} errorCount={1} />);
    // Badge text = total pending; decorative wrapper is aria-hidden.
    const badge = screen.getByText("3");
    expect(badge).toBeInTheDocument();
    expect(badge.closest("[aria-hidden='true']")).not.toBeNull();
  });

  it("renders no badge when there is nothing waiting/erroring and no state", () => {
    render(() => <OverflowControl items={[{ id: "a", label: "Export" }]} />);
    expect(screen.queryByText("!")).toBeNull();
  });

  it("lists the overflowed items when opened", () => {
    render(() => (
      <OverflowControl
        items={[
          { id: "export", label: "Export" },
          { id: "detach", label: "Detach" },
        ]}
      />
    ));
    openMenu(/More actions/);
    expect(screen.getByRole("menu")).toBeInTheDocument();
    expect(screen.getByRole("menuitem", { name: "Export" })).toBeInTheDocument();
    expect(screen.getByRole("menuitem", { name: "Detach" })).toBeInTheDocument();
  });

  it("invokes an item's onSelect when chosen", () => {
    const onSelect = vi.fn();
    render(() => <OverflowControl items={[{ id: "export", label: "Export", onSelect }]} />);
    openMenu(/More actions/);
    const item = screen.getByRole("menuitem", { name: "Export" });
    fireEvent.keyDown(item, { key: "Enter" });
    fireEvent.keyUp(item, { key: "Enter" });
    expect(onSelect).toHaveBeenCalledOnce();
  });

  it("closing via Escape invokes no action and dismisses the menu", () => {
    const onSelect = vi.fn();
    render(() => <OverflowControl items={[{ id: "export", label: "Export", onSelect }]} />);
    const trigger = openMenu(/More actions/);
    const menu = screen.getByRole("menu");
    expect(trigger).toHaveAttribute("aria-expanded", "true");
    fireEvent.keyDown(menu, { key: "Escape" });
    // Kobalte marks the surface dismissed (data-closed) and collapses the
    // trigger; closing runs no action.
    expect(menu).toHaveAttribute("data-closed");
    expect(trigger).toHaveAttribute("aria-expanded", "false");
    expect(onSelect).not.toHaveBeenCalled();
  });
});
