/**
 * LensModeToggle + manual-2D-preference tests (task 6.5, Req 5.5 / 17.5).
 *
 * The manual toggle must let a user force the 2D representation even on a
 * 3D-capable device, and return to the automatic decision — reusing the shared
 * render-mode gate. Also covers the auto-degrade ladder resolving to 2D under
 * no-WebGL / reduced-motion (LensRenderMode renders the 2D branch).
 */
import { describe, it, expect, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@solidjs/testing-library";
import { LensModeToggle } from "./LensModeToggle";
import { LensRenderMode } from "./LensRenderMode";
import { applyProbeResult, initRenderMode, lensRenderMode, preferTwoD, setPreferTwoD } from "./renderMode";
import type { CapabilitySnapshot, ProbeResult } from "./capabilities";

function snapshot(overrides: Partial<CapabilitySnapshot> = {}): CapabilitySnapshot {
  return {
    webglTier: "webgl2",
    hasWebGL: true,
    prefersReducedMotion: false,
    supportsBackdropFilter: true,
    probe: null,
    ...overrides,
  };
}

const passingProbe = (): ProbeResult => ({
  interactionFrameMs: 12,
  interactionFps: 60,
  idleQuiet: true,
  nodeCount: 1500,
});

beforeEach(() => initRenderMode(snapshot()));

describe("setPreferTwoD — manual 2D override", () => {
  it("forces 2D even when a passing probe would enable 3D", () => {
    applyProbeResult(passingProbe());
    expect(lensRenderMode().enable3D).toBe(true);

    const s = setPreferTwoD(true);
    expect(s.mode).toBe("2d");
    expect(s.enable3D).toBe(false);
    expect(preferTwoD()).toBe(true);
    expect(s.reason).toMatch(/preference/i);
  });

  it("returns to the automatic 3D decision when cleared", () => {
    applyProbeResult(passingProbe());
    setPreferTwoD(true);
    expect(lensRenderMode().enable3D).toBe(false);

    const s = setPreferTwoD(false);
    expect(s.enable3D).toBe(true);
    expect(s.mode).toBe("3d");
  });

  it("is reset to auto by initRenderMode", () => {
    setPreferTwoD(true);
    initRenderMode(snapshot());
    expect(preferTwoD()).toBe(false);
  });
});

describe("LensModeToggle component", () => {
  it("renders a 2D/3D segmented control and forces 2D on selection", () => {
    applyProbeResult(passingProbe());
    render(() => <LensModeToggle />);

    const twoD = screen.getByRole("button", { name: "2D" });
    fireEvent.click(twoD);
    expect(preferTwoD()).toBe(true);
    expect(lensRenderMode().enable3D).toBe(false);
  });

  it("disables the 3D option and explains why when the device can't do 3D", () => {
    initRenderMode(snapshot({ hasWebGL: false, webglTier: "none" }));
    render(() => <LensModeToggle />);
    expect(screen.getByRole("button", { name: "3D" })).toBeDisabled();
    expect(screen.getByText(/3D unavailable on this device/)).toBeInTheDocument();
  });
});

describe("auto-degrade ladder → LensRenderMode renders 2D", () => {
  function Lens() {
    return (
      <LensRenderMode
        twoD={() => <div data-testid="twoD">2D</div>}
        threeD={() => <div data-testid="threeD">3D</div>}
      />
    );
  }

  it("renders the 2D branch when WebGL is absent", () => {
    initRenderMode(snapshot({ hasWebGL: false, webglTier: "none" }));
    render(() => <Lens />);
    expect(screen.getByTestId("twoD")).toBeInTheDocument();
    expect(screen.queryByTestId("threeD")).toBeNull();
  });

  it("renders the 2D branch under reduced-motion even with a passing probe", () => {
    initRenderMode(snapshot({ prefersReducedMotion: true }));
    applyProbeResult(passingProbe());
    render(() => <Lens />);
    expect(screen.getByTestId("twoD")).toBeInTheDocument();
    expect(screen.queryByTestId("threeD")).toBeNull();
  });

  it("renders the 2D branch after a manual downgrade even with a passing probe", () => {
    applyProbeResult(passingProbe());
    setPreferTwoD(true);
    render(() => <Lens />);
    expect(screen.getByTestId("twoD")).toBeInTheDocument();
    expect(screen.queryByTestId("threeD")).toBeNull();
  });
});
