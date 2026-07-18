import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@solidjs/testing-library";
import { Dialog } from "./Dialog";
import { Confirm } from "./Confirm";

describe("Dialog", () => {
  it("is closed until the trigger is activated", () => {
    render(() => (
      <Dialog triggerLabel="Open" title="Settings">
        <p>Body</p>
      </Dialog>
    ));
    expect(screen.queryByRole("dialog")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Open" }));
    expect(screen.getByRole("dialog")).toBeInTheDocument();
  });

  it("labels the dialog with its title", () => {
    render(() => (
      <Dialog triggerLabel="Open" title="Rename thread">
        <p>Body</p>
      </Dialog>
    ));
    fireEvent.click(screen.getByRole("button", { name: "Open" }));
    expect(screen.getByRole("dialog", { name: "Rename thread" })).toBeInTheDocument();
  });

  it("exposes a labeled close control", () => {
    render(() => (
      <Dialog triggerLabel="Open" title="X">
        <p>Body</p>
      </Dialog>
    ));
    fireEvent.click(screen.getByRole("button", { name: "Open" }));
    expect(screen.getByRole("button", { name: "Close" })).toBeInTheDocument();
  });

  it("installs a focus trap and handles DE-agnostic Escape", async () => {
    render(() => (
      <Dialog triggerLabel="Open accessible dialog" title="Accessible dialog">
        <button type="button">Inner action</button>
      </Dialog>
    ));
    const trigger = screen.getByRole("button", { name: "Open accessible dialog" });
    fireEvent.click(trigger);
    const dialog = await screen.findByRole("dialog", { name: "Accessible dialog" });
    expect(dialog.querySelectorAll("[data-focus-trap]")).toHaveLength(2);

    fireEvent.keyDown(dialog, { key: "Escape", code: "Escape" });
    expect(trigger).toHaveAttribute("aria-expanded", "false");
    expect(dialog).toHaveAttribute("data-closed");
  });
});

describe("Confirm", () => {
  it("shows cancel and confirm actions and a risk note for danger", () => {
    render(() => (
      <Confirm
        triggerLabel="Delete"
        title="Delete?"
        message="Irreversible."
        risk="danger"
      />
    ));
    fireEvent.click(screen.getByRole("button", { name: "Delete" }));
    expect(screen.getByRole("button", { name: "Cancel" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Confirm" })).toBeInTheDocument();
    // Risk is conveyed by text, not color alone (Req 17.3).
    expect(screen.getByRole("note")).toHaveTextContent("irreversible");
  });

  it("invokes onConfirm when confirmed", () => {
    const onConfirm = vi.fn();
    render(() => (
      <Confirm
        triggerLabel="Proceed"
        title="Proceed?"
        message="Do the thing."
        confirmLabel="Do it"
        onConfirm={onConfirm}
      />
    ));
    fireEvent.click(screen.getByRole("button", { name: "Proceed" }));
    fireEvent.click(screen.getByRole("button", { name: "Do it" }));
    expect(onConfirm).toHaveBeenCalledOnce();
  });
});
