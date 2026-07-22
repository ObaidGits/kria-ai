import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@solidjs/testing-library";
import { Menu } from "./Menu";

// Kobalte menus open via keyboard (Enter/Space/ArrowDown) — the a11y path we
// assert here. Content is portalled to document.body, so query via `screen`.
function openMenu(name: string) {
  const trigger = screen.getByRole("button", { name });
  trigger.focus();
  fireEvent.keyDown(trigger, { key: "Enter" });
}

describe("Menu", () => {
  it("renders a labeled trigger button and no menu until opened", () => {
    render(() => (
      <Menu triggerLabel="Actions" items={[{ id: "a", label: "Alpha" }]} />
    ));
    expect(screen.getByRole("button", { name: "Actions" })).toHaveAttribute(
      "aria-haspopup",
      "true",
    );
    expect(screen.queryByRole("menu")).toBeNull();
  });

  it("opens a menu with menuitems via the keyboard", () => {
    render(() => (
      <Menu
        triggerLabel="Actions"
        items={[
          { id: "a", label: "Alpha" },
          { id: "b", label: "Beta" },
        ]}
      />
    ));
    openMenu("Actions");
    expect(screen.getByRole("menu")).toBeInTheDocument();
    expect(screen.getAllByRole("menuitem").length).toBe(2);
  });

  it("invokes onSelect when an item is chosen", () => {
    const onSelect = vi.fn();
    render(() => (
      <Menu triggerLabel="Actions" items={[{ id: "a", label: "Alpha", onSelect }]} />
    ));
    openMenu("Actions");
    const item = screen.getByRole("menuitem", { name: "Alpha" });
    fireEvent.keyDown(item, { key: "Enter" });
    fireEvent.keyUp(item, { key: "Enter" });
    expect(onSelect).toHaveBeenCalledOnce();
  });

  it("does not describe the trigger when no triggerDescription is given", () => {
    render(() => (
      <Menu triggerLabel="Actions" items={[{ id: "a", label: "Alpha" }]} />
    ));
    expect(screen.getByRole("button", { name: "Actions" })).not.toHaveAttribute(
      "aria-describedby",
    );
  });

  it("wires triggerDescription to the trigger via aria-describedby (not hover-only)", () => {
    render(() => (
      <Menu
        triggerLabel="Export conversation"
        triggerIcon="download"
        triggerDescription="No messages to export yet. Send a message to enable export."
        items={[{ id: "a", label: "Plain text" }]}
      />
    ));
    const trigger = screen.getByRole("button", { name: "Export conversation" });
    const describedBy = trigger.getAttribute("aria-describedby");
    expect(describedBy).toBeTruthy();
    const description = document.getElementById(describedBy!);
    expect(description).not.toBeNull();
    // Reason text is present in the DOM (visible helper, not a hover tooltip).
    expect(description).toHaveTextContent(
      "No messages to export yet. Send a message to enable export.",
    );
  });

  it("exposes a disabled item's reason via ItemDescription / aria-describedby", () => {
    render(() => (
      <Menu
        triggerLabel="Actions"
        items={[
          {
            id: "a",
            label: "Plain text",
            disabled: true,
            description: "Export running. Export is available again when the current export finishes.",
          },
        ]}
      />
    ));
    openMenu("Actions");
    const item = screen.getByRole("menuitem", { name: /Plain text/ });
    const describedBy = item.getAttribute("aria-describedby");
    expect(describedBy).toBeTruthy();
    const description = document.getElementById(describedBy!);
    expect(description).toHaveTextContent(
      "Export running. Export is available again when the current export finishes.",
    );
  });
});
