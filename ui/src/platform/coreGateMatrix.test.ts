/**
 * coreGateMatrix tests (task 7.3) — wiring the Core-3D performance gate into
 * the Linux device matrix (GNOME+KDE × Wayland+X11 × NVIDIA/AMD/Intel) and
 * recording a machine-recordable pass/fallback signal per device.
 *
 * These verify the matrix WIRING + RECORD SCHEMA (the local-verifiable part of
 * task 7.3). The per-device hardware measurement remains a manual matrix run;
 * these tests prove the gate produces a structured, honest record for every
 * cell rather than fabricated hardware results.
 *
 * Validates: Requirements 20.2
 */
import { describe, it, expect } from "vitest";
import type { CapabilitySnapshot, ProbeResult } from "./capabilities";
import {
  CORE_3D_GATE_MATRIX,
  LINUX_DESKTOPS,
  LINUX_GPU_VENDORS,
  LINUX_SESSIONS,
  cellDeviceId,
  recordCoreGateCell,
  recordCoreGateMatrix,
  summarizeCoreGateMatrix,
  type CoreGateMatrixCell,
} from "./coreGateMatrix";

function caps(overrides: Partial<CapabilitySnapshot> = {}): CapabilitySnapshot {
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
  interactionFrameMs: 16,
  interactionFps: 60,
  idleQuiet: true,
  nodeCount: 400,
});

const cell = (over: Partial<CoreGateMatrixCell> = {}): CoreGateMatrixCell => ({
  desktop: "GNOME",
  session: "Wayland",
  gpu: "NVIDIA",
  ...over,
});

describe("CORE_3D_GATE_MATRIX — full Linux matrix enumeration (Req 20.2)", () => {
  it("enumerates every desktop × session × GPU cell (2 × 2 × 3 = 12)", () => {
    expect(CORE_3D_GATE_MATRIX).toHaveLength(12);
    expect(LINUX_DESKTOPS).toHaveLength(2);
    expect(LINUX_SESSIONS).toHaveLength(2);
    expect(LINUX_GPU_VENDORS).toHaveLength(3);
  });

  it("covers each declared desktop, session, and GPU vendor", () => {
    for (const desktop of LINUX_DESKTOPS) {
      for (const session of LINUX_SESSIONS) {
        for (const gpu of LINUX_GPU_VENDORS) {
          expect(
            CORE_3D_GATE_MATRIX.some(
              (c) => c.desktop === desktop && c.session === session && c.gpu === gpu,
            ),
          ).toBe(true);
        }
      }
    }
  });

  it("produces unique, stable device ids", () => {
    const ids = CORE_3D_GATE_MATRIX.map(cellDeviceId);
    expect(new Set(ids).size).toBe(ids.length);
    expect(cellDeviceId(cell())).toBe("GNOME/Wayland/NVIDIA");
  });
});

describe("recordCoreGateCell — machine-recordable pass/fallback (Req 20.2)", () => {
  it("records a 3D pass when a passing on-device probe is measured", () => {
    const rec = recordCoreGateCell(cell(), {
      snapshot: caps(),
      probe: passingProbe(),
      measured: true,
    });
    expect(rec.outcome).toBe("3d-pass");
    expect(rec.enable3D).toBe(true);
    expect(rec.gatePassed).toBe(true);
    expect(rec.decision.mode).toBe("3d");
    expect(rec.verification).toBe("local-verified");
  });

  it("records a 2D fallback when no probe has run (safe default)", () => {
    const rec = recordCoreGateCell(cell(), { snapshot: caps(), probe: null });
    expect(rec.outcome).toBe("2d-fallback");
    expect(rec.enable3D).toBe(false);
    expect(rec.gatePassed).toBe(false);
    expect(rec.decision.triggers).toContain("failed-gate");
    expect(rec.verification).toBe("manual-matrix-required");
  });

  it("records a 2D fallback on a device without WebGL (WebKitGTK software raster)", () => {
    const rec = recordCoreGateCell(cell({ gpu: "Intel" }), {
      snapshot: caps({ hasWebGL: false, webglTier: "none" }),
      probe: passingProbe(),
      measured: true,
    });
    expect(rec.outcome).toBe("2d-fallback");
    expect(rec.decision.triggers).toContain("no-webgl");
    // Honesty: a real on-device measurement is still local-verified even when it
    // resolves to the 2D fallback.
    expect(rec.verification).toBe("local-verified");
  });

  it("records a 2D fallback under reduced-motion regardless of GPU", () => {
    const rec = recordCoreGateCell(cell({ gpu: "AMD" }), {
      snapshot: caps({ prefersReducedMotion: true }),
      probe: passingProbe(),
    });
    expect(rec.outcome).toBe("2d-fallback");
    expect(rec.decision.triggers).toContain("reduced-motion");
  });

  it("records a 2D fallback under low-power even with a passing probe", () => {
    const rec = recordCoreGateCell(cell(), {
      snapshot: caps(),
      probe: passingProbe(),
      lowPower: true,
    });
    expect(rec.outcome).toBe("2d-fallback");
    expect(rec.decision.triggers).toContain("low-power");
  });

  it("records a 2D fallback when the probe is below the sustained-fps floor", () => {
    const rec = recordCoreGateCell(cell(), {
      snapshot: caps(),
      probe: { ...passingProbe(), interactionFps: 20 },
      measured: true,
    });
    expect(rec.outcome).toBe("2d-fallback");
    expect(rec.gatePassed).toBe(false);
    expect(rec.decision.triggers).toContain("failed-gate");
  });
});

describe("recordCoreGateMatrix — complete honest matrix (Req 20.2)", () => {
  it("records every cell, defaulting missing cells to an unverified 2D fallback", () => {
    const records = recordCoreGateMatrix();
    expect(records).toHaveLength(12);
    for (const rec of records) {
      expect(rec.outcome).toBe("2d-fallback");
      expect(rec.verification).toBe("manual-matrix-required");
    }
  });

  it("applies per-cell inputs and leaves the rest as safe defaults", () => {
    const localCellId = "GNOME/Wayland/Intel";
    const records = recordCoreGateMatrix({
      [localCellId]: { snapshot: caps(), probe: passingProbe(), measured: true },
    });
    const local = records.find((r) => r.deviceId === localCellId);
    expect(local?.outcome).toBe("3d-pass");
    expect(local?.verification).toBe("local-verified");

    // Every other cell stays an honest, unverified 2D fallback.
    for (const rec of records.filter((r) => r.deviceId !== localCellId)) {
      expect(rec.outcome).toBe("2d-fallback");
      expect(rec.verification).toBe("manual-matrix-required");
    }
  });

  it("summarizes counts across the matrix", () => {
    const records = recordCoreGateMatrix({
      "GNOME/Wayland/NVIDIA": { snapshot: caps(), probe: passingProbe(), measured: true },
      "KDE/X11/AMD": { snapshot: caps(), probe: passingProbe(), measured: true },
    });
    const summary = summarizeCoreGateMatrix(records);
    expect(summary.total).toBe(12);
    expect(summary.pass3d).toBe(2);
    expect(summary.fallback2d).toBe(10);
    expect(summary.localVerified).toBe(2);
    expect(summary.manualMatrixRequired).toBe(10);
  });
});
