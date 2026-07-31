import { describe, it, expect } from "vitest";
import {
  ZOOM_MIN,
  ZOOM_MAX,
  PAN_MARGIN,
  createCamera,
  clampZoom,
  zoomAt,
  panCamera,
  worldToScreen,
  screenToWorld,
  type CameraState,
  type ViewportSize,
} from "./camera";

const VP: ViewportSize = { width: 800, height: 600 };
const WORLD_BOUNDS = { minX: 0, minY: 0, maxX: 1000, maxY: 1000 };

// ─── createCamera ─────────────────────────────────────────────────────────────

describe("createCamera", () => {
  it("centres at (width/2, height/2) with zoom=1", () => {
    const cam = createCamera(VP);
    expect(cam.x).toBe(400);
    expect(cam.y).toBe(300);
    expect(cam.zoom).toBe(1);
  });

  it("works for non-standard viewport sizes", () => {
    const cam = createCamera({ width: 1280, height: 720 });
    expect(cam.x).toBe(640);
    expect(cam.y).toBe(360);
    expect(cam.zoom).toBe(1);
  });
});

// ─── clampZoom ────────────────────────────────────────────────────────────────

describe("clampZoom", () => {
  it("returns ZOOM_MIN for values below minimum", () => {
    expect(clampZoom(0)).toBe(ZOOM_MIN);
    expect(clampZoom(-5)).toBe(ZOOM_MIN);
    expect(clampZoom(0.1)).toBe(ZOOM_MIN);
  });

  it("returns ZOOM_MAX for values above maximum", () => {
    expect(clampZoom(10)).toBe(ZOOM_MAX);
    expect(clampZoom(100)).toBe(ZOOM_MAX);
    expect(clampZoom(4.1)).toBe(ZOOM_MAX);
  });

  it("returns the value unchanged when inside [ZOOM_MIN, ZOOM_MAX]", () => {
    expect(clampZoom(1)).toBe(1);
    expect(clampZoom(ZOOM_MIN)).toBe(ZOOM_MIN);
    expect(clampZoom(ZOOM_MAX)).toBe(ZOOM_MAX);
    expect(clampZoom(2)).toBe(2);
  });
});

// ─── zoomAt ───────────────────────────────────────────────────────────────────

describe("zoomAt", () => {
  it("changes zoom and clamps to [ZOOM_MIN, ZOOM_MAX]", () => {
    const cam = createCamera(VP);

    const zoomedIn = zoomAt(cam, VP, 400, 300, 10); // huge delta → hits ZOOM_MAX
    expect(zoomedIn.zoom).toBe(ZOOM_MAX);

    const zoomedOut = zoomAt(cam, VP, 400, 300, 0.01); // tiny delta → hits ZOOM_MIN
    expect(zoomedOut.zoom).toBe(ZOOM_MIN);

    const moderate = zoomAt(cam, VP, 400, 300, 1.5);
    expect(moderate.zoom).toBeGreaterThan(1);
    expect(moderate.zoom).toBeLessThanOrEqual(ZOOM_MAX);
  });

  it("pivots around the cursor: world point under cursor stays the same", () => {
    const cam = createCamera(VP);
    // Use an off-centre screen point
    const screenPx = 500;
    const screenPy = 200;

    // World point currently under cursor
    const worldBefore = screenToWorld(cam, VP, screenPx, screenPy);

    // Apply a 2× zoom
    const zoomed = zoomAt(cam, VP, screenPx, screenPy, 2);

    // World point under the same screen pixel after zoom
    const worldAfter = screenToWorld(zoomed, VP, screenPx, screenPy);

    expect(worldAfter.x).toBeCloseTo(worldBefore.x, 10);
    expect(worldAfter.y).toBeCloseTo(worldBefore.y, 10);
  });

  it("pivot holds when zooming out", () => {
    const cam: CameraState = { x: 300, y: 400, zoom: 2 };
    const screenPx = 200;
    const screenPy = 150;

    const worldBefore = screenToWorld(cam, VP, screenPx, screenPy);
    const zoomed = zoomAt(cam, VP, screenPx, screenPy, 0.5);
    const worldAfter = screenToWorld(zoomed, VP, screenPx, screenPy);

    expect(worldAfter.x).toBeCloseTo(worldBefore.x, 10);
    expect(worldAfter.y).toBeCloseTo(worldBefore.y, 10);
  });
});

// ─── panCamera ────────────────────────────────────────────────────────────────

describe("panCamera", () => {
  it("applies a pan offset in world space", () => {
    const cam = createCamera(VP); // x=400, y=300, zoom=1
    // Pan right by 100 screen px → camera moves left in world (world centre decreases)
    const panned = panCamera(cam, VP, WORLD_BOUNDS, 100, 0);
    expect(panned.x).toBe(300);
    expect(panned.y).toBe(300);
  });

  it("converts screen-space pan to world-space correctly at zoom=2", () => {
    const cam: CameraState = { x: 500, y: 500, zoom: 2 };
    // 200 screen pixels at zoom=2 → 100 world units
    const panned = panCamera(cam, VP, WORLD_BOUNDS, 200, 100);
    expect(panned.x).toBeCloseTo(400, 10);
    expect(panned.y).toBeCloseTo(450, 10);
  });

  it("clamps to 25% margin bounds when panning beyond world limits", () => {
    const cam = createCamera(VP); // zoom=1
    // Pan far left: dx = -99999 should move camera way right but be clamped
    const panned = panCamera(cam, VP, WORLD_BOUNDS, -99999, 0);
    const marginX = (PAN_MARGIN * VP.width) / cam.zoom; // 200
    expect(panned.x).toBe(WORLD_BOUNDS.maxX + marginX);
  });

  it("clamps when panning to the negative side", () => {
    const cam = createCamera(VP);
    const panned = panCamera(cam, VP, WORLD_BOUNDS, 99999, 0);
    const marginX = (PAN_MARGIN * VP.width) / cam.zoom;
    expect(panned.x).toBe(WORLD_BOUNDS.minX - marginX);
  });

  it("does not change zoom", () => {
    const cam: CameraState = { x: 500, y: 500, zoom: 1.5 };
    const panned = panCamera(cam, VP, WORLD_BOUNDS, 10, 20);
    expect(panned.zoom).toBe(1.5);
  });
});

// ─── worldToScreen / screenToWorld roundtrip ──────────────────────────────────

describe("worldToScreen / screenToWorld", () => {
  const cam: CameraState = { x: 500, y: 400, zoom: 1.5 };

  it("screenToWorld(worldToScreen(p)) ≈ p (roundtrip)", () => {
    const worldPoints = [
      { x: 0, y: 0 },
      { x: 500, y: 400 },
      { x: 123.45, y: -67.89 },
      { x: 999, y: 999 },
    ];

    for (const p of worldPoints) {
      const screen = worldToScreen(cam, VP, p.x, p.y);
      const back = screenToWorld(cam, VP, screen.x, screen.y);
      expect(back.x).toBeCloseTo(p.x, 10);
      expect(back.y).toBeCloseTo(p.y, 10);
    }
  });

  it("worldToScreen(screenToWorld(p)) ≈ p (inverse roundtrip)", () => {
    const screenPoints = [
      { x: 0, y: 0 },
      { x: 400, y: 300 },
      { x: 799, y: 599 },
    ];

    for (const p of screenPoints) {
      const world = screenToWorld(cam, VP, p.x, p.y);
      const back = worldToScreen(cam, VP, world.x, world.y);
      expect(back.x).toBeCloseTo(p.x, 10);
      expect(back.y).toBeCloseTo(p.y, 10);
    }
  });

  it("camera centre maps to viewport centre", () => {
    const screen = worldToScreen(cam, VP, cam.x, cam.y);
    expect(screen.x).toBeCloseTo(VP.width / 2, 10);
    expect(screen.y).toBeCloseTo(VP.height / 2, 10);
  });
});

// ─── Purity check ─────────────────────────────────────────────────────────────

describe("purity", () => {
  it("same input always produces same output (no side effects)", () => {
    const cam: CameraState = { x: 300, y: 200, zoom: 1.2 };

    const r1 = zoomAt(cam, VP, 400, 300, 1.5);
    const r2 = zoomAt(cam, VP, 400, 300, 1.5);
    expect(r1).toEqual(r2);

    const p1 = panCamera(cam, VP, WORLD_BOUNDS, 50, 30);
    const p2 = panCamera(cam, VP, WORLD_BOUNDS, 50, 30);
    expect(p1).toEqual(p2);

    // Input camera is not mutated
    expect(cam).toEqual({ x: 300, y: 200, zoom: 1.2 });
  });
});
