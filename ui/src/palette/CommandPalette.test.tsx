import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { render, screen, fireEvent, waitFor } from "@solidjs/testing-library";
import { CommandPalette } from "./CommandPalette";
import { shellStore, converseStore, eventBus } from "../stores";
import { navigate, currentRoute } from "../shell/router";
import { registerCommand, clearCommands } from "./commands";
import { registerShortcut, clearShortcuts } from "./shortcuts";
import { clearRecents, getRecents, recordUse } from "./recents";
import { clearCustomSources } from "./sources";

function openPalette() {
  shellStore.setPaletteOpen(true);
}

describe("CommandPalette", () => {
  beforeEach(() => {
    clearCommands();
    clearShortcuts();
    clearRecents();
    clearCustomSources();
    navigate("converse");
    shellStore.setPaletteOpen(false);
  });

  afterEach(() => {
    shellStore.setPaletteOpen(false);
  });

  it("is not shown until shellStore.paletteOpen is true", () => {
    render(() => <CommandPalette />);
    expect(screen.queryByRole("combobox")).toBeNull();
    openPalette();
    expect(screen.getByRole("combobox")).toBeInTheDocument();
  });

  it("exposes the four modes as a tablist (Go/Do/Ask/Change)", () => {
    render(() => <CommandPalette />);
    openPalette();
    expect(screen.getByRole("tab", { name: /Go/ })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: /Do/ })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: /Ask/ })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: /Change/ })).toBeInTheDocument();
  });

  it("fuzzy-searches Spaces in Go mode and groups results", () => {
    render(() => <CommandPalette />);
    openPalette();
    fireEvent.input(screen.getByRole("combobox"), { target: { value: "sett" } });
    // Settings space option is present and grouped.
    expect(screen.getByRole("option", { name: /Settings/ })).toBeInTheDocument();
    expect(screen.getByRole("group", { name: "Spaces" })).toBeInTheDocument();
    // A non-matching Space is filtered out.
    expect(screen.queryByRole("option", { name: /^Machines$/ })).toBeNull();
  });

  it("navigates on Enter (Go mode) and closes", () => {
    render(() => <CommandPalette />);
    openPalette();
    const input = screen.getByRole("combobox");
    fireEvent.input(input, { target: { value: "memory" } });
    fireEvent.keyDown(input, { key: "Enter" });
    expect(currentRoute().space).toBe("memory");
    expect(shellStore.paletteOpen()).toBe(false);
  });

  it("moves the active option with arrow keys", () => {
    render(() => <CommandPalette />);
    openPalette();
    const input = screen.getByRole("combobox");
    // No query → all Spaces listed; first is active.
    const optionsBefore = screen.getAllByRole("option");
    expect(optionsBefore[0].getAttribute("aria-selected")).toBe("true");
    fireEvent.keyDown(input, { key: "ArrowDown" });
    const optionsAfter = screen.getAllByRole("option");
    expect(optionsAfter[0].getAttribute("aria-selected")).toBe("false");
    expect(optionsAfter[1].getAttribute("aria-selected")).toBe("true");
  });

  it("switches to Do mode and lists registered commands", () => {
    const run = vi.fn();
    registerCommand({ id: "cmd.test", title: "Run the test thing", run });
    render(() => <CommandPalette />);
    openPalette();
    fireEvent.click(screen.getByRole("tab", { name: /Do/ }));
    expect(screen.getByRole("option", { name: /Run the test thing/ })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("option", { name: /Run the test thing/ }));
    expect(run).toHaveBeenCalledOnce();
  });

  it("opens directly in Do mode when requested (Ctrl+Shift+P path, §20.2)", () => {
    const run = vi.fn();
    registerCommand({ id: "cmd.domode", title: "Do mode command", run });
    render(() => <CommandPalette />);
    // Simulate the proven Ctrl+Shift+P chord → shellStore.setPaletteOpen(true, "do").
    shellStore.setPaletteOpen(true, "do");
    // Do tab is selected and Do-mode commands are listed without any prefix/click.
    expect(screen.getByRole("tab", { name: /Do/ }).getAttribute("aria-selected")).toBe("true");
    expect(screen.getByRole("option", { name: /Do mode command/ })).toBeInTheDocument();
  });

  it("defaults to Go mode on a plain open", () => {
    render(() => <CommandPalette />);
    openPalette();
    expect(screen.getByRole("tab", { name: /Go/ }).getAttribute("aria-selected")).toBe("true");
  });

  it("switches modes via a leading prefix token", () => {
    registerCommand({ id: "cmd.pref", title: "Prefixed command", run: () => {} });
    render(() => <CommandPalette />);
    openPalette();
    // ">" selects Do mode without clicking a chip.
    fireEvent.input(screen.getByRole("combobox"), { target: { value: ">pref" } });
    expect(screen.getByRole("option", { name: /Prefixed command/ })).toBeInTheDocument();
  });

  it("surfaces keyboard shortcuts discoverably (Req 2.4)", () => {
    registerShortcut({ id: "sc.test", label: "Do the shortcut", keys: ["Ctrl", "J"] });
    render(() => <CommandPalette />);
    openPalette();
    fireEvent.click(screen.getByRole("tab", { name: /Do/ }));
    fireEvent.input(screen.getByRole("combobox"), { target: { value: "shortcut" } });
    expect(screen.getByRole("option", { name: /Do the shortcut/ })).toBeInTheDocument();
  });

  it("explains and resets adaptive palette ranking (Req 19.3)", () => {
    recordUse("space:memory");
    expect(getRecents()).toContain("space:memory");
    render(() => <CommandPalette />);
    openPalette();
    expect(screen.getByText("Match order adapts from items you use.")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Reset ranking" }));
    expect(getRecents()).toHaveLength(0);
  });

  it("routes Ask through the Converse send path (not a tool call)", () => {
    const handler = vi.fn();
    const unsub = eventBus.on("palette:ask-submitted", handler);
    render(() => <CommandPalette />);
    openPalette();
    fireEvent.click(screen.getByRole("tab", { name: /Ask/ }));
    const input = screen.getByRole("combobox");
    fireEvent.input(input, { target: { value: "what is my schedule" } });
    fireEvent.keyDown(input, { key: "Enter" });

    // Staged into the Converse composer + routed to Converse, and emitted an
    // ask-submitted intent — never a direct capability/tool invocation.
    expect(handler).toHaveBeenCalledWith({ text: "what is my schedule" });
    expect(converseStore.composerDraft().text).toBe("what is my schedule");
    expect(currentRoute().space).toBe("converse");
    unsub();
  });

  it("closes on Escape and marks the event handled (one-layer peel, §20.3)", () => {
    render(() => <CommandPalette />);
    openPalette();
    const dialog = screen.getByRole("dialog", { name: "Command palette" });
    // The palette owns the Escape while open: it closes AND preventDefaults so a
    // lower layer (e.g. Immersive window mode) cannot also peel on the same event.
    const event = new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true });
    dialog.dispatchEvent(event);
    expect(shellStore.paletteOpen()).toBe(false);
    expect(event.defaultPrevented).toBe(true);
  });

  it("does not submit an Ask message on Enter while composing (IME composition guard)", () => {
    const handler = vi.fn();
    const unsub = eventBus.on("palette:ask-submitted", handler);
    render(() => <CommandPalette />);
    openPalette();
    fireEvent.click(screen.getByRole("tab", { name: /Ask/ }));
    const input = screen.getByRole("combobox");
    fireEvent.input(input, { target: { value: "still typing" } });
    // Enter during composition confirms the IME candidate — it must NOT send.
    fireEvent.keyDown(input, { key: "Enter", isComposing: true });
    expect(handler).not.toHaveBeenCalled();
    expect(shellStore.paletteOpen()).toBe(true);
    unsub();
  });

  it("does not select a Go result on Enter while composing (IME composition guard)", () => {
    render(() => <CommandPalette />);
    openPalette();
    const input = screen.getByRole("combobox");
    fireEvent.input(input, { target: { value: "memory" } });
    fireEvent.keyDown(input, { key: "Enter", isComposing: true });
    // No navigation and the palette stays open mid-composition.
    expect(currentRoute().space).toBe("converse");
    expect(shellStore.paletteOpen()).toBe(true);
  });

  it("routes Change to the Settings NL path", () => {
    const handler = vi.fn();
    const unsub = eventBus.on("palette:change-submitted", handler);
    render(() => <CommandPalette />);
    openPalette();
    fireEvent.click(screen.getByRole("tab", { name: /Change/ }));
    const input = screen.getByRole("combobox");
    fireEvent.input(input, { target: { value: "set voice speed to fast" } });
    fireEvent.keyDown(input, { key: "Enter" });
    expect(handler).toHaveBeenCalledWith({ text: "set voice speed to fast" });
    expect(currentRoute().space).toBe("settings");
    unsub();
  });
});
