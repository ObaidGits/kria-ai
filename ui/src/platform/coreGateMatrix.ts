/**
 * platform/coreGateMatrix — wire the Core-3D performance gate (task 0.3,
 * `coreRenderMode.ts`) into the Linux device matrix and record a machine-
 * recordable pass/fallback result per device (task 7.3).
 *
 * Design source: design.md §13.3 (Capability gate at boot — probe WebKitGTK/
 * Wayland/GPU; enable the 3D Core only if it passes; else the 2D path),
 * Requirement 20.2 ("pass performance gates on the Linux matrix
 * (GNOME+KDE × Wayland+X11 × NVIDIA/AMD/Intel) … before enabling the 3D Core").
 *
 * ── What this module is (and is not) ─────────────────────────────────────────
 * This module does NOT re-implement the gate or the resolver — those are owned
 * by `coreRenderMode.ts` (task 0.3) and consume the shared capability detection
 * in `capabilities.ts`. This module only:
 *   1. Enumerates the full Linux matrix cells (desktop × session × GPU vendor).
 *   2. Runs the existing gate/resolver for a cell's inputs and captures a
 *      structured, machine-recordable record — a pass/fallback signal PER
 *      device, not prose.
 *   3. Tracks, per cell, whether the record came from a real on-device probe
 *      (`local-verified`) or is an expected decision awaiting the physical
 *      matrix run (`manual-matrix-required`).
 *
 * Honesty note (steering `dev-context.md`): the full PHYSICAL Linux matrix
 * (GNOME+KDE × Wayland+X11 × NVIDIA/AMD/Intel) cannot be executed on a single
 * developer laptop. The gate LOGIC is unit-verified here for every cell; the
 * per-device hardware measurement (the actual on-device probe) remains a manual
 * matrix run. Cells are marked accordingly — measurements are never fabricated.
 */
import type { CapabilitySnapshot, ProbeResult } from "./capabilities";
import {
  coreGatePasses,
  resolveCoreRenderMode,
  type CoreRenderDecision,
} from "./coreRenderMode";

/** Linux desktop environments exercised by the matrix (Req 20.2). */
export type LinuxDesktop = "GNOME" | "KDE";

/** Display-server sessions exercised by the matrix (Req 20.2). */
export type LinuxSession = "Wayland" | "X11";

/** GPU vendors exercised by the matrix (Req 20.2). */
export type LinuxGpuVendor = "NVIDIA" | "AMD" | "Intel";

export const LINUX_DESKTOPS: readonly LinuxDesktop[] = ["GNOME", "KDE"] as const;
export const LINUX_SESSIONS: readonly LinuxSession[] = ["Wayland", "X11"] as const;
export const LINUX_GPU_VENDORS: readonly LinuxGpuVendor[] = ["NVIDIA", "AMD", "Intel"] as const;

/** A single device cell in the Linux matrix. */
export interface CoreGateMatrixCell {
  desktop: LinuxDesktop;
  session: LinuxSession;
  gpu: LinuxGpuVendor;
}

/**
 * The full Core-3D gate matrix: every desktop × session × GPU combination
 * (2 × 2 × 3 = 12 cells). Order-stable so records and the artifact line up.
 */
export const CORE_3D_GATE_MATRIX: readonly CoreGateMatrixCell[] = LINUX_DESKTOPS.flatMap(
  (desktop) =>
    LINUX_SESSIONS.flatMap((session) =>
      LINUX_GPU_VENDORS.map((gpu) => ({ desktop, session, gpu })),
    ),
);

/**
 * Per-cell verification provenance:
 *   - "local-verified"        — a real on-device Core-3D probe was captured on
 *                               this exact cell (this laptop's compositor/GPU).
 *   - "manual-matrix-required" — the gate LOGIC is verified, but the physical
 *                               per-device probe still needs a hardware run.
 */
export type CellVerification = "local-verified" | "manual-matrix-required";

/** The recorded gate outcome for a device: a machine signal, not prose. */
export type CoreGateOutcome = "3d-pass" | "2d-fallback";

/** Inputs needed to evaluate the gate for one cell. */
export interface CoreGateCellInputs {
  /** Device capability snapshot for the cell (WebGL tier, reduced-motion, …). */
  snapshot: CapabilitySnapshot;
  /**
   * The on-device Core-3D probe result, or `null` when no probe has run for
   * this cell yet (keeps the gate failed → 2D fallback, the safe default).
   */
  probe: ProbeResult | null;
  /** Low-power / battery-saver posture reported for the cell. */
  lowPower?: boolean;
  /**
   * True when `probe` was captured by a real on-device run on this exact cell.
   * Drives the `verification` provenance. Defaults to `false` (expected).
   */
  measured?: boolean;
}

/** A structured, machine-recordable gate record for one device cell. */
export interface CoreGateMatrixRecord {
  /** The matrix cell this record describes. */
  cell: CoreGateMatrixCell;
  /** Stable device identifier, e.g. "GNOME/Wayland/NVIDIA". */
  deviceId: string;
  /** The recorded pass/fallback signal. */
  outcome: CoreGateOutcome;
  /** True iff the 3D Core is enabled for this device (outcome === "3d-pass"). */
  enable3D: boolean;
  /** Whether the on-device Core-3D performance gate passed for this cell. */
  gatePassed: boolean;
  /** Full resolver decision (triggers, degraded flag, reason). */
  decision: CoreRenderDecision;
  /** Provenance: local-verified vs manual-matrix-required. */
  verification: CellVerification;
}

/** Build the stable device identifier for a cell. */
export function cellDeviceId(cell: CoreGateMatrixCell): string {
  return `${cell.desktop}/${cell.session}/${cell.gpu}`;
}

/**
 * Evaluate the Core-3D gate + resolver for a single matrix cell and produce a
 * machine-recordable record. The Core-3D gate always runs in `auto` preference
 * (the matrix asks "does this device earn 3D?"), so the outcome reflects the
 * device capability + probe, never a user override.
 */
export function recordCoreGateCell(
  cell: CoreGateMatrixCell,
  inputs: CoreGateCellInputs,
): CoreGateMatrixRecord {
  const gatePassed = coreGatePasses(inputs.probe);
  const decision = resolveCoreRenderMode({
    preference: "auto",
    snapshot: inputs.snapshot,
    gatePassed,
    lowPower: inputs.lowPower ?? false,
    frameDrop: false,
  });
  return {
    cell,
    deviceId: cellDeviceId(cell),
    outcome: decision.enable3D ? "3d-pass" : "2d-fallback",
    enable3D: decision.enable3D,
    gatePassed,
    decision,
    verification: inputs.measured ? "local-verified" : "manual-matrix-required",
  };
}

/**
 * Record the full Linux matrix. `inputsByCell` maps a cell's device id
 * (`cellDeviceId`) to its inputs. Any cell without inputs is recorded with a
 * safe default: no probe (`null`) → gate fails → 2D fallback, marked
 * `manual-matrix-required`. This guarantees a complete, never-fabricated matrix
 * — every cell has an explicit, honest record.
 */
export function recordCoreGateMatrix(
  inputsByCell: Readonly<Record<string, CoreGateCellInputs>> = {},
  matrix: readonly CoreGateMatrixCell[] = CORE_3D_GATE_MATRIX,
): CoreGateMatrixRecord[] {
  return matrix.map((cell) => {
    const inputs = inputsByCell[cellDeviceId(cell)];
    if (inputs) return recordCoreGateCell(cell, inputs);
    // No inputs recorded for this cell yet: safe 2D-fallback default, unverified.
    return recordCoreGateCell(cell, {
      snapshot: {
        webglTier: "none",
        hasWebGL: false,
        prefersReducedMotion: false,
        supportsBackdropFilter: false,
        probe: null,
      },
      probe: null,
      measured: false,
    });
  });
}

/** Aggregate counts over a set of records (for reporting / gate summaries). */
export interface CoreGateMatrixSummary {
  total: number;
  pass3d: number;
  fallback2d: number;
  localVerified: number;
  manualMatrixRequired: number;
}

/** Summarize a matrix run into machine-recordable counts. */
export function summarizeCoreGateMatrix(
  records: readonly CoreGateMatrixRecord[],
): CoreGateMatrixSummary {
  return records.reduce<CoreGateMatrixSummary>(
    (acc, r) => ({
      total: acc.total + 1,
      pass3d: acc.pass3d + (r.outcome === "3d-pass" ? 1 : 0),
      fallback2d: acc.fallback2d + (r.outcome === "2d-fallback" ? 1 : 0),
      localVerified: acc.localVerified + (r.verification === "local-verified" ? 1 : 0),
      manualMatrixRequired:
        acc.manualMatrixRequired + (r.verification === "manual-matrix-required" ? 1 : 0),
    }),
    { total: 0, pass3d: 0, fallback2d: 0, localVerified: 0, manualMatrixRequired: 0 },
  );
}
