/**
 * Router-authority guard tests (task 7.3; Req 7.10 / design §9, §20.1).
 *
 * These tests PIN the single-authority route model:
 *   • The typed router (`currentRoute`) is the SOLE authority for the rendered
 *     Space — SpaceRouter reads it, `navigate()` updates it.
 *   • `shellStore.activeSpace` is a DERIVED MIRROR only: it converges to the
 *     router (driven by AppShell's route→effect), and it can NEVER independently
 *     select the rendered content. Writing `setActiveSpace` without `navigate`
 *     does NOT change what SpaceRouter renders.
 *   • A Dock click converges router + mirror to the SAME Space (no divergence),
 *     proving the removed duplicate `onSelect→setActiveSpace` write was not
 *     required architecture.
 *
 * Requirements: 1.1, 7.10
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, within, cleanup } from "@solidjs/testing-library";
import { AppShell } from "./AppShell";
import { SpaceRouter } from "./SpaceRouter";
import { shellStore, converseStore, coreStore, provisioningStore } from "../stores";
import { navigate, currentRoute } from "./router";

function mockProvisioningComplete() {
  vi.spyOn(provisioningStore, "loadState").mockResolvedValue({
    current_step: "complete",
    steps: {},
    hardware_profile: null,
    backend_choice: null,
    models_dir: null,
    errors: [],
  });
  vi.spyOn(provisioningStore, "isComplete").mockReturnValue(true);
}

describe("Router authority — rendered Space (task 7.3, Req 7.10)", () => {
  beforeEach(() => {
    navigate("converse");
    shellStore.setActiveSpace("converse");
    shellStore.setWindowMode("standard");
    shellStore.setInspectorTarget(null);
    converseStore.setActiveThread(null);
    coreStore.reset();
    mockProvisioningComplete();
  });

  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it("SpaceRouter renders per currentRoute, NOT per shellStore.activeSpace", () => {
    // Route says converse; force the mirror to disagree (memory).
    navigate("converse");
    shellStore.setActiveSpace("memory");

    render(() => <SpaceRouter />);

    // Rendered content follows the router (Converse), not the diverged mirror.
    expect(screen.getByRole("region", { name: "Converse" })).toBeInTheDocument();
    expect(screen.queryByRole("region", { name: "Memory" })).toBeNull();
  });

  it("shellStore.activeSpace CANNOT independently change the rendered Space", () => {
    navigate("converse");
    render(() => <SpaceRouter />);
    expect(screen.getByRole("region", { name: "Converse" })).toBeInTheDocument();

    // Write the mirror directly WITHOUT navigate — the router is untouched.
    shellStore.setActiveSpace("machines");

    // SpaceRouter still renders the router's Space; the mirror write is inert.
    expect(currentRoute().space).toBe("converse");
    expect(screen.getByRole("region", { name: "Converse" })).toBeInTheDocument();
    expect(screen.queryByRole("region", { name: "Machines" })).toBeNull();
  });

  it("navigate(space) updates currentRoute AND shellStore.activeSpace converges to it", async () => {
    render(() => <AppShell />);
    // Wait for the shell (and its route→mirror effect) to mount.
    await screen.findByRole("navigation", { name: "Spaces" });

    navigate("automations");
    await Promise.resolve();

    // Router is authoritative; the mirror has converged to the same Space.
    expect(currentRoute().space).toBe("automations");
    expect(shellStore.activeSpace()).toBe("automations");
  });

  it("Dock click converges router + mirror to the same Space (no divergence)", async () => {
    render(() => <AppShell />);
    const nav = await screen.findByRole("navigation", { name: "Spaces" });

    fireEvent.click(within(nav).getByRole("button", { name: "Memory" }));
    await Promise.resolve();

    // Single authority: both the router and its derived mirror agree, and the
    // rendered Space follows suit.
    expect(currentRoute().space).toBe("memory");
    expect(shellStore.activeSpace()).toBe("memory");
    expect(
      await screen.findByRole("region", { name: "Memory" }, { timeout: 5_000 })
    ).toBeInTheDocument();
  });
});
