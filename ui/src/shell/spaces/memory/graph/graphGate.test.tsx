/**
 * Generic LensRenderMode gate tests. Marker branches verify gate mechanics;
 * they do not mount `GraphCanvas3D` or prove Memory Graph 3D integration.
 */
import { describe, it, expect } from "vitest";
import { render, screen } from "@solidjs/testing-library";
import { LensRenderMode } from "../../../../platform/LensRenderMode";
import { applyProbeResult, initRenderMode } from "../../../../platform/renderMode";
import type { CapabilitySnapshot, ProbeResult } from "../../../../platform/capabilities";

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

function Lens() {
  return (
    <LensRenderMode
      twoD={() => <div data-testid="fallback-2d">2D fallback</div>}
      threeD={() => <div data-testid="scene-3d">3D scene</div>}
    />
  );
}

describe("generic lens capability gate (not Memory Graph integration)", () => {
  it("yields to the 2D fallback and does NOT mount 3D when WebGL is absent", () => {
    initRenderMode(snapshot({ hasWebGL: false, webglTier: "none" }));
    render(() => <Lens />);
    expect(screen.getByTestId("fallback-2d")).toBeInTheDocument();
    expect(screen.queryByTestId("scene-3d")).toBeNull();
  });

  it("stays 2D before any probe has run (2D-first default)", () => {
    initRenderMode(snapshot({ probe: null }));
    render(() => <Lens />);
    expect(screen.getByTestId("fallback-2d")).toBeInTheDocument();
    expect(screen.queryByTestId("scene-3d")).toBeNull();
  });

  it("mounts the 3D scene ONLY after a passing on-device probe", () => {
    initRenderMode(snapshot({ probe: null }));
    applyProbeResult(passingProbe());
    render(() => <Lens />);
    expect(screen.getByTestId("scene-3d")).toBeInTheDocument();
    expect(screen.queryByTestId("fallback-2d")).toBeNull();
  });

  it("stays 2D under reduced-motion even with a passing probe", () => {
    initRenderMode(snapshot({ prefersReducedMotion: true }));
    applyProbeResult(passingProbe());
    render(() => <Lens />);
    expect(screen.getByTestId("fallback-2d")).toBeInTheDocument();
    expect(screen.queryByTestId("scene-3d")).toBeNull();
  });
});
