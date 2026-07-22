/**
 * SpaceRouter — operation-specific loading copy + section-scoped fallback
 * (sub-task 12.6; UIE-M-013; Req 13.1, 13.3, 17.2).
 *
 * Pins that the Space loading fallback:
 *   • NAMES the Space being opened instead of a generic "Loading…" (Req 13.1),
 *     routed through the shared operation vocabulary + copy layer.
 *   • Lives INSIDE the `<main id="space-root">` landmark, so a lazy chunk load
 *     replaces only the Space content region and never the surrounding shell
 *     controls (Req 13.3 — a section-scoped, not shell-wide, loading state).
 *   • Stays a polite live region (Req 17.2) and carries the operation state as a
 *     data hook.
 *
 * The registry is mocked with a component that suspends forever so the fallback
 * is shown deterministically (the real Converse space is eager and would never
 * suspend). SPACE_META (the canonical labels) is kept real.
 */
import { describe, it, expect, vi, afterEach } from "vitest";
import { render, screen, cleanup } from "@solidjs/testing-library";

vi.mock("./spaces", async (importOriginal) => {
  const actual = await importOriginal<typeof import("./spaces")>();
  const { lazy } = await import("solid-js");
  // A lazily-loaded chunk whose import never resolves → SpaceRouter stays in
  // its Suspense fallback deterministically (Solid Suspense is driven by lazy/
  // resource reads, so we use a real never-resolving lazy import here).
  const Suspending = lazy(() => new Promise<{ default: () => null }>(() => {}));
  return {
    ...actual,
    SPACE_COMPONENTS: new Proxy(
      {},
      { get: () => Suspending },
    ) as typeof actual.SPACE_COMPONENTS,
  };
});

import SpaceRouter from "./SpaceRouter";
import { navigate } from "./router";

afterEach(cleanup);

describe("SpaceRouter — operation-specific loading fallback (Req 13.1)", () => {
  it("names the Space being opened instead of a generic 'Loading…'", async () => {
    navigate("converse");
    render(() => <SpaceRouter />);

    const status = await screen.findByRole("status");
    expect(status.textContent).toBe("Loading Converse…");
    // The old generic copy must be gone.
    expect(status.textContent).not.toBe("Loading…");
  });

  it("names each Space from the canonical label", async () => {
    navigate("memory");
    render(() => <SpaceRouter />);
    const status = await screen.findByRole("status");
    expect(status.textContent).toBe("Loading Memory…");
  });
});

describe("SpaceRouter — section-scoped loading (Req 13.3 / 17.2)", () => {
  it("renders the fallback inside the main#space-root landmark", async () => {
    navigate("settings");
    const { container } = render(() => <SpaceRouter />);

    const main = container.querySelector("main#space-root");
    expect(main).not.toBeNull();
    // The active-space marker persists on the landmark while loading.
    expect(main!.getAttribute("data-active-space")).toBe("settings");

    const status = await screen.findByRole("status");
    // The loading fallback is a DESCENDANT of the space content region, so it
    // replaces only the Space — never the shell controls mounted as siblings
    // of <main> in AppShell (nav/presence/status/composer stay available).
    expect(main!.contains(status)).toBe(true);
    expect(status.getAttribute("data-operation-state")).toBe("loading");
    expect(status.getAttribute("aria-live")).toBe("polite");
  });
});
