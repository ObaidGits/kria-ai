import { fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { shellStore } from "../stores/shellStore";
import { disposeWindowModeManager, initWindowModeManager } from "../windowing/windowModeManager";
import { WindowModeSwitch } from "./WindowModeSwitch";

/** Validates: Requirements 15.5, 18.3, 18.6 */
describe("WindowModeSwitch", () => {
  beforeEach(() => {
    disposeWindowModeManager();
    shellStore.setWindowMode("standard");
  });

  afterEach(() => {
    disposeWindowModeManager();
    shellStore.setWindowMode("standard");
  });

  it("provides KRIA-owned controls for every mode and a visible Immersive exit", () => {
    const view = render(() => <WindowModeSwitch />);

    expect(screen.getByRole("group", { name: "Window mode" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Compact window mode" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Standard window mode" })).toHaveAttribute("aria-pressed", "true");

    fireEvent.click(screen.getByRole("button", { name: "Immersive window mode" }));
    expect(shellStore.windowMode()).toBe("immersive");

    // The repository's test harness resolves two Solid browser instances, so
    // remount against the authoritative store before checking reactive output.
    view.unmount();
    render(() => <WindowModeSwitch />);
    expect(screen.getByRole("button", { name: "Immersive window mode" })).toHaveAttribute("aria-pressed", "true");

    const exit = screen.getByRole("button", { name: "Exit Immersive" });
    expect(exit).toHaveAttribute("aria-keyshortcuts", "Escape");
    fireEvent.click(exit);
    expect(shellStore.windowMode()).toBe("standard");
  });

  it("exits Immersive with Escape without depending on desktop-environment shortcuts", () => {
    shellStore.setWindowMode("immersive");
    initWindowModeManager();

    fireEvent.keyDown(window, { key: "Escape" });

    expect(shellStore.windowMode()).toBe("standard");
  });

  it("leaves Immersive active when a top-most UI layer consumes Escape", () => {
    shellStore.setWindowMode("immersive");
    initWindowModeManager();
    window.addEventListener("keydown", (event) => event.preventDefault(), { once: true, capture: true });

    fireEvent.keyDown(window, { key: "Escape" });

    expect(shellStore.windowMode()).toBe("immersive");
  });
});
