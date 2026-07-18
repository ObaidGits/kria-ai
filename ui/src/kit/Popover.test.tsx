import { describe, it, expect } from "vitest";
import { render, screen, fireEvent, within } from "@solidjs/testing-library";
import { Popover } from "./Popover";

describe("Popover", () => {
  it("renders a labeled trigger and no content until opened", () => {
    render(() => (
      <Popover triggerLabel="Open" title="Panel">
        <p>Body text</p>
      </Popover>
    ));
    expect(screen.getByRole("button", { name: "Open" })).toBeInTheDocument();
    expect(screen.queryByText("Body text")).toBeNull();
  });

  it("opens on trigger activation and exposes a dialog with its content", () => {
    render(() => (
      <Popover triggerLabel="Open" title="Panel">
        <p>Body text</p>
      </Popover>
    ));
    fireEvent.click(screen.getByRole("button", { name: "Open" }));
    expect(screen.getByRole("dialog")).toBeInTheDocument();
    expect(screen.getByText("Body text")).toBeInTheDocument();
  });

  it("provides a labeled close control once open", () => {
    render(() => (
      <Popover triggerLabel="Open">
        <p>Body</p>
      </Popover>
    ));
    fireEvent.click(screen.getByRole("button", { name: "Open" }));
    const dialogs = screen.getAllByRole("dialog");
    const dialog = dialogs[dialogs.length - 1];
    expect(within(dialog).getByRole("button", { name: "Close" })).toBeInTheDocument();
  });
});
