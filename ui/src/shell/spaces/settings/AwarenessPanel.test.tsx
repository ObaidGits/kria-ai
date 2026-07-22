/**
 * Tests for the "what KRIA can sense" Settings panel (task 3.8, Req 25.5).
 *   • Renders every registered source with its purpose, tier and availability.
 *   • Per-source opt-in/opt-out flows through the registry.
 *   • The opt-into-memory toggle appears only when a source is enabled and drives
 *     the registry's memory state (ephemeral by default — Req 25.4).
 */
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { AwarenessPanel } from "./AwarenessPanel";
import {
  createDefaultDesktopAwarenessRegistry,
  type DesktopAwarenessRegistry,
} from "../../../stores/desktopAwarenessBridge";

function makeRegistry(): DesktopAwarenessRegistry {
  return createDefaultDesktopAwarenessRegistry({
    platform: "wayland",
    now: () => 100,
    tauriAvailable: () => false,
    setBridge: vi.fn(),
    clearBridge: vi.fn(),
  });
}

afterEach(() => cleanup());

describe("AwarenessPanel", () => {
  it("lists every registered source with purpose and availability", () => {
    const registry = makeRegistry();
    render(() => <AwarenessPanel registry={registry} />);

    expect(screen.getByRole("heading", { name: "What KRIA can sense" })).toBeInTheDocument();
    // Battery source from the §25.1 catalog.
    expect(screen.getByText("Battery & power")).toBeInTheDocument();
    expect(
      screen.getByText("Let KRIA notice low battery so it can offer to pause heavy work."),
    ).toBeInTheDocument();
    // Wayland + X11 availability badges are shown per source.
    expect(screen.getAllByText(/Wayland:/).length).toBeGreaterThan(0);
    expect(screen.getAllByText(/X11:/).length).toBeGreaterThan(0);
    // A switch exists for each source.
    expect(screen.getAllByRole("switch").length).toBe(registry.list().length);
  });

  it("shows all sources OFF and no memory toggles by default", () => {
    const registry = makeRegistry();
    render(() => <AwarenessPanel registry={registry} />);

    expect(screen.getByText(/not sensing anything/i)).toBeInTheDocument();
    // No memory toggle until a source is enabled.
    expect(screen.queryByRole("switch", { name: /Remember .* in memory/ })).toBeNull();
  });

  it("opting a source in flows through the registry and reveals its memory toggle", async () => {
    const registry = makeRegistry();
    render(() => <AwarenessPanel registry={registry} />);

    const batterySwitch = screen.getByRole("switch", { name: "Sense Battery & power: Off" });
    expect(batterySwitch).not.toBeChecked();

    fireEvent.click(batterySwitch);
    expect(registry.isEnabled("battery")).toBe(true);
    // The label re-renders to the On state.
    expect(await screen.findByRole("switch", { name: "Sense Battery & power: On" })).toBeInTheDocument();
    // The opt-into-memory toggle now exists for this source.
    const memorySwitch = await screen.findByRole("switch", {
      name: "Remember Battery & power in memory: Off",
    });
    expect(memorySwitch).not.toBeChecked();
  });

  it("opting a source into memory flows through the registry (ephemeral → remembered)", async () => {
    const registry = makeRegistry();
    render(() => <AwarenessPanel registry={registry} />);

    fireEvent.click(screen.getByRole("switch", { name: "Sense Battery & power: Off" }));
    const memorySwitch = await screen.findByRole("switch", {
      name: "Remember Battery & power in memory: Off",
    });
    expect(registry.isRemembered("battery")).toBe(false);

    fireEvent.click(memorySwitch);
    expect(registry.isRemembered("battery")).toBe(true);
    expect(
      await screen.findByRole("switch", { name: "Remember Battery & power in memory: On" }),
    ).toBeInTheDocument();
  });

  it("opting a source back out removes its memory toggle and disables it", async () => {
    const registry = makeRegistry();
    render(() => <AwarenessPanel registry={registry} />);

    const on = screen.getByRole("switch", { name: "Sense Battery & power: Off" });
    fireEvent.click(on);
    fireEvent.click(await screen.findByRole("switch", { name: "Sense Battery & power: On" }));

    expect(registry.isEnabled("battery")).toBe(false);
    await vi.waitFor(() =>
      expect(screen.queryByRole("switch", { name: /Remember Battery & power in memory/ })).toBeNull(),
    );
  });
});
