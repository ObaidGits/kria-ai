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
    expect(screen.getByRole("button", { name: "Mini window mode" })).toBeInTheDocument();
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

  it("presents mode controls inline (not disclosed) when width does not require collapse", () => {
    render(() => <WindowModeSwitch />);

    // No matchMedia in jsdom → the control degrades to the fully-visible group,
    // and does NOT render a disclosure trigger.
    expect(screen.getByRole("group", { name: "Window mode" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Mini window mode" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Window mode: / })).not.toBeInTheDocument();
  });
});

/**
 * At constrained widths the mode controls must collapse into a concise
 * secondary disclosure while keeping the current mode visible and the Immersive
 * exit explicit (UIE-H-009, UIE-L-004, Req 10.6 / 10.2 / 10.8).
 *
 * Validates: Requirements 10.2, 10.6, 10.8
 */
describe("WindowModeSwitch — constrained width disclosure", () => {
  let descriptor: PropertyDescriptor | undefined;

  function forceCollapsed(collapsed: boolean) {
    Object.defineProperty(window, "matchMedia", {
      configurable: true,
      writable: true,
      value: (query: string) => ({
        matches: collapsed,
        media: query,
        onchange: null,
        addListener: () => {},
        removeListener: () => {},
        addEventListener: () => {},
        removeEventListener: () => {},
        dispatchEvent: () => false,
      }),
    });
  }

  beforeEach(() => {
    descriptor = Object.getOwnPropertyDescriptor(window, "matchMedia");
    disposeWindowModeManager();
    shellStore.setWindowMode("standard");
    forceCollapsed(true);
  });

  afterEach(() => {
    disposeWindowModeManager();
    shellStore.setWindowMode("standard");
    if (descriptor) Object.defineProperty(window, "matchMedia", descriptor);
    else delete (window as { matchMedia?: unknown }).matchMedia;
  });

  it("collapses to a disclosure whose trigger shows the current mode", () => {
    render(() => <WindowModeSwitch />);

    // Current mode is visible directly on the disclosure trigger…
    expect(screen.getByRole("button", { name: "Window mode: Standard" })).toBeInTheDocument();
    // …and the equally-weighted inline mode buttons are NOT shown until disclosed.
    expect(screen.queryByRole("button", { name: "Mini window mode" })).not.toBeInTheDocument();
  });

  it("discloses all modes, each reachable and selectable via the keyboard", () => {
    render(() => <WindowModeSwitch />);

    fireEvent.click(screen.getByRole("button", { name: "Window mode: Standard" }));

    for (const label of ["Standard", "Mini", "Immersive", "Companion"]) {
      expect(screen.getByRole("button", { name: `${label} window mode` })).toBeInTheDocument();
    }

    // Real buttons → keyboard-activatable. Selecting one drives the store.
    fireEvent.click(screen.getByRole("button", { name: "Immersive window mode" }));
    expect(shellStore.windowMode()).toBe("immersive");
  });

  it("keeps an explicit, always-reachable Immersive exit even when collapsed", () => {
    shellStore.setWindowMode("immersive");
    render(() => <WindowModeSwitch />);

    // Exit is a direct, visible control — not buried behind the disclosure.
    const exit = screen.getByRole("button", { name: "Exit Immersive" });
    expect(exit).toHaveAttribute("aria-keyshortcuts", "Escape");
    fireEvent.click(exit);
    expect(shellStore.windowMode()).toBe("standard");
  });
});
