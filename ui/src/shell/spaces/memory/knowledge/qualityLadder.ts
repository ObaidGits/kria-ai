/**
 * memory/knowledge/qualityLadder — Adaptive quality-level selection.
 *
 * Pure TypeScript module — no JSX, no DOM, no side effects.
 *
 * Selects the appropriate rendering quality level for the knowledge graph
 * based on system pressure (memory, CPU, thermal, battery) and scene size.
 *
 * Quality ladder (lowest → highest):
 *   list-first       → fallback when Canvas is unavailable or system is stressed
 *   decoration-only  → Canvas but no labels or analytics
 *   with-labels      → Canvas + labels
 *   with-analytics   → Canvas + labels + analytics
 *   scene-120        → Full scene, up to 120 items
 *   scene-180        → Full scene, up to 180 items (all balanced caps)
 *
 * IDs: MGD-003; task 4.7.8.
 */

// ─── Types ────────────────────────────────────────────────────────────────────

export type QualityLevel =
  | "list-first"       // fallback: no Canvas
  | "decoration-only"  // Canvas but no labels or analytics
  | "with-labels"      // Canvas + labels
  | "with-analytics"   // Canvas + labels + analytics
  | "scene-120"        // Full scene, up to 120 items
  | "scene-180";       // Full scene, up to 180 items (all balanced caps)

export interface SystemPressure {
  memoryPressureBytes: number;
  cpuUtilisationPercent: number;
  thermalState: "nominal" | "throttled" | "critical";
  batteryPercent: number | null;
}

// ─── Public API ───────────────────────────────────────────────────────────────

/**
 * Select the appropriate quality level given current system pressure and scene
 * item count.
 *
 * Decision order (highest-priority first):
 *  1. Canvas not available           → list-first
 *  2. Thermal state = critical       → list-first
 *  3. CPU ≥ 90 %                     → list-first
 *  4. Thermal state = throttled      → decoration-only (at most)
 *  5. sceneItemCount > 180           → decoration-only
 *  6. sceneItemCount > 120           → scene-120
 *  7. CPU ≥ 70 %                     → with-labels
 *  8. Otherwise                      → scene-180
 */
export function selectQualityLevel(
  pressure: SystemPressure,
  sceneItemCount: number,
  canvasAvailable: boolean,
): QualityLevel {
  if (!canvasAvailable) {
    return "list-first";
  }

  if (pressure.thermalState === "critical") {
    return "list-first";
  }

  if (pressure.cpuUtilisationPercent >= 90) {
    return "list-first";
  }

  if (pressure.thermalState === "throttled") {
    return "decoration-only";
  }

  if (sceneItemCount > 180) {
    return "decoration-only";
  }

  if (sceneItemCount > 120) {
    return "scene-120";
  }

  if (pressure.cpuUtilisationPercent >= 70) {
    return "with-labels";
  }

  return "scene-180";
}

/**
 * Returns true when the given quality level is the list-first fallback
 * (i.e. Canvas is not used).
 */
export function isListFirst(level: QualityLevel): boolean {
  return level === "list-first";
}

/**
 * Maximum number of scene items for the given quality level.
 *
 *   scene-180        → 180
 *   scene-120        → 120
 *   with-analytics   → 120
 *   with-labels      → 120
 *   decoration-only  → 240
 *   list-first       → 0   (no Canvas scene)
 */
export function maxItemsForLevel(level: QualityLevel): number {
  switch (level) {
    case "scene-180":
      return 180;
    case "scene-120":
      return 120;
    case "with-analytics":
      return 120;
    case "with-labels":
      return 120;
    case "decoration-only":
      return 240;
    case "list-first":
      return 0;
  }
}
