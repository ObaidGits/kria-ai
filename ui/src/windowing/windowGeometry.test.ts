import { describe, expect, it } from "vitest";
import { isWindowGeometry, normalizeGeometry, type GeometryMonitor } from "./windowGeometry";

const MONITORS: GeometryMonitor[] = [
  { workArea: { position: { x: 0, y: 0 }, size: { width: 1920, height: 1040 } }, scaleFactor: 1 },
  { workArea: { position: { x: 1920, y: -200 }, size: { width: 2560, height: 1400 } }, scaleFactor: 2 },
];

describe("window geometry memory", () => {
  it("rejects malformed persisted geometry", () => {
    expect(isWindowGeometry({ x: 0, y: 0, width: -1, height: 10, scaleFactor: 1 })).toBe(false);
    expect(isWindowGeometry({ x: 0, y: 0, width: 800, height: 600, scaleFactor: 1 })).toBe(true);
  });

  it("moves geometry from a disconnected display onto a visible work area", () => {
    const restored = normalizeGeometry(
      { x: 9000, y: 9000, width: 900, height: 700, scaleFactor: 1 },
      [MONITORS[0]],
    );
    expect(restored).toEqual({ x: 1020, y: 340, width: 900, height: 700, scaleFactor: 1 });
  });

  it("rescales physical size when monitor scale factor changes", () => {
    const restored = normalizeGeometry(
      { x: 2100, y: 0, width: 600, height: 500, scaleFactor: 1 },
      MONITORS,
    );
    expect(restored).toMatchObject({ width: 1200, height: 1000, scaleFactor: 2 });
  });

  it("clamps an oversized geometry down to the target monitor work area", () => {
    const restored = normalizeGeometry(
      { x: -100, y: -100, width: 5000, height: 5000, scaleFactor: 1 },
      [MONITORS[0]],
    );
    expect(restored).not.toBeNull();
    // Size cannot exceed the work area, and origin is pulled back on-screen.
    expect(restored!.width).toBeLessThanOrEqual(MONITORS[0].workArea.size.width);
    expect(restored!.height).toBeLessThanOrEqual(MONITORS[0].workArea.size.height);
    expect(restored!.x).toBeGreaterThanOrEqual(MONITORS[0].workArea.position.x);
    expect(restored!.y).toBeGreaterThanOrEqual(MONITORS[0].workArea.position.y);
    // Fully contained within the work area (no overflow past the right/bottom edges).
    expect(restored!.x + restored!.width).toBeLessThanOrEqual(
      MONITORS[0].workArea.position.x + MONITORS[0].workArea.size.width,
    );
    expect(restored!.y + restored!.height).toBeLessThanOrEqual(
      MONITORS[0].workArea.position.y + MONITORS[0].workArea.size.height,
    );
  });

  it("returns null when there is no monitor or the geometry is invalid", () => {
    expect(normalizeGeometry({ x: 0, y: 0, width: 800, height: 600, scaleFactor: 1 }, [])).toBeNull();
    expect(
      normalizeGeometry({ x: 0, y: 0, width: -1, height: 600, scaleFactor: 1 }, [MONITORS[0]]),
    ).toBeNull();
  });

  it("keeps generated valid geometries fully visible across mixed-DPI work areas", () => {
    for (let seed = 0; seed < 200; seed += 1) {
      const geometry = {
        x: seed * 137 - 5000,
        y: seed * 83 - 3000,
        width: (seed % 17 + 1) * 173,
        height: (seed % 13 + 1) * 149,
        scaleFactor: seed % 2 === 0 ? 1 : 2,
      };
      const restored = normalizeGeometry(geometry, MONITORS);
      expect(restored).not.toBeNull();
      const containing = MONITORS.find((monitor) => {
        const work = monitor.workArea;
        return restored!.x >= work.position.x && restored!.y >= work.position.y
          && restored!.x + restored!.width <= work.position.x + work.size.width
          && restored!.y + restored!.height <= work.position.y + work.size.height;
      });
      expect(containing).toBeDefined();
    }
  });
});
