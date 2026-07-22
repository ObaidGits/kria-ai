/**
 * Long/expanded translation rendering for Window Mode controls and the kit
 * primitives that carry their labels (task 4.7, design §20.2 "long translations").
 *
 * Component-level assertion only: expanded descriptive copy (window-mode purpose
 * text, disclosure trigger label, dialog/popover labels) must still render the
 * control and keep it reachable. Pixel-level overflow is a later visual/Linux
 * gate; here we prove no control is dropped and rendering does not break.
 */
import { fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { shellStore } from "../stores/shellStore";
import { disposeWindowModeManager } from "../windowing/windowModeManager";
import { WindowModeSwitch } from "./WindowModeSwitch";
import { Dialog, Popover } from "../kit";

const LONG =
  "Immersiver Vollfokus-Arbeitsbereich mit erweiterter lokalisierter Beschreibung, " +
  "die deutlich länger ist als der englische Originaltext und dennoch rendern muss";

describe("WindowModeSwitch — expanded window-mode descriptions render", () => {
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

  it("discloses every mode with its expanded purpose text and keeps all controls reachable", () => {
    render(() => <WindowModeSwitch />);

    // The current mode stays visible on the collapsed disclosure trigger.
    fireEvent.click(screen.getByRole("button", { name: "Window mode: Standard" }));

    // Expanded, descriptive purpose copy renders inside the disclosure for every
    // mode without dropping any option button.
    for (const label of ["Standard", "Mini", "Immersive", "Companion"]) {
      expect(screen.getByRole("button", { name: `${label} window mode` })).toBeInTheDocument();
    }
    expect(screen.getByText(/Full-focus workspace/)).toBeInTheDocument();
    expect(screen.getByText(/quick-interaction window/)).toBeInTheDocument();
  });
});

describe("kit primitives render with long expanded labels (window-mode label carriers)", () => {
  it("Popover trigger renders and opens with a very long localized label", () => {
    render(() => (
      <Popover triggerLabel={`Window mode: ${LONG}`} title={LONG}>
        <p>Inhalt</p>
      </Popover>
    ));
    const trigger = screen.getByRole("button", { name: `Window mode: ${LONG}` });
    expect(trigger).toBeInTheDocument();
    fireEvent.click(trigger);
    // Content still mounts with the expanded title (no render break).
    expect(screen.getByText("Inhalt")).toBeInTheDocument();
  });

  it("Dialog trigger and panel render with a very long localized title", () => {
    render(() => (
      <Dialog triggerLabel={LONG} title={LONG}>
        <p>Körper</p>
      </Dialog>
    ));
    const trigger = screen.getByRole("button", { name: LONG });
    expect(trigger).toBeInTheDocument();
    fireEvent.click(trigger);
    expect(screen.getByRole("dialog", { name: LONG })).toBeInTheDocument();
  });
});
