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

  // The `label` prop rendered a bare `GroupLabel`, which Kobalte refuses outside a
  // `Group` — it threw "useMenuGroupContext must be used within a Menu.Group" and
  // the app's error boundary turned that into a full "startup error" screen. The
  // Converse export menu is the one caller that passes `label`, so the crash showed
  // up as "the download button breaks the app". The existing tests missed it because
  // portal content only renders once the menu is OPEN, so this one opens it.
  it("renders a group label without crashing, and still shows its items", () => {
    render(() => (
      <Menu
        triggerLabel="Export conversation"
        label="Export format"
        items={[
          { id: "text", label: "Plain text (.txt)" },
          { id: "json", label: "JSON (.json)" },
        ]}
      />
    ));
    openMenu("Export conversation");

    expect(screen.getByRole("menu")).toBeInTheDocument();
    expect(screen.getByText("Export format")).toBeInTheDocument();
    expect(screen.getAllByRole("menuitem").length).toBe(2);
    expect(
      screen.getByRole("menuitem", { name: "JSON (.json)" }),
    ).toBeInTheDocument();
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
