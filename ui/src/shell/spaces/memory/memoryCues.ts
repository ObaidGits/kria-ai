/**
 * Memory display cues (task 6.2, Req 5.2 / 17.3).
 *
 * Pure helpers that turn a memory's numeric/string signals into an accessible
 * cue: a short text `label`, a kit `Badge` `tone`, and a Lucide `icon` id. Every
 * cue pairs an icon WITH text — meaning is NEVER carried by color alone
 * (Req 17.3). Shared by the compact MemoryCard and the full Inspector body so
 * the two surfaces speak one visual language.
 *
 * No side effects, no I/O — trivially unit-testable.
 */
import type { BadgeTone } from "../../../kit";

export interface MemoryCue {
  /** Human-readable text (the accessible signal — never color-only). */
  label: string;
  /** kit Badge tone. */
  tone: BadgeTone;
  /** Lucide icon id present in the sprite manifest. */
  icon: string;
}

function pct(value: number): number {
  return Math.round(Math.max(0, Math.min(1, value)) * 100);
}

/** Confidence cue: high → success, mid → info, low → warning. */
export function confidenceCue(confidence: number): MemoryCue {
  const p = pct(confidence);
  const tone: BadgeTone = p >= 75 ? "success" : p >= 40 ? "info" : "warning";
  return { label: `${p}% confidence`, tone, icon: "gauge" };
}

/**
 * Worth cue from a normalized 0..1 worth score (as carried on `MemoryFact`).
 * Distinct from the sampled worth in the Inspector.
 */
export function worthCue(worth: number): MemoryCue {
  const p = pct(worth);
  const tone: BadgeTone = p >= 66 ? "success" : p >= 33 ? "info" : "neutral";
  return { label: `worth ${p}%`, tone, icon: "star" };
}

/**
 * Worth cue from raw success/failure sample counts (Inspector detail). Reports
 * an honest "untested" state when there are no samples yet.
 */
export function sampledWorthCue(success: number, failure: number, samples: number): MemoryCue {
  if (samples <= 0 || success + failure <= 0) {
    return { label: "worth: untested", tone: "neutral", icon: "star" };
  }
  const ratio = success / (success + failure);
  const p = Math.round(ratio * 100);
  const tone: BadgeTone = p >= 66 ? "success" : p >= 33 ? "info" : "warning";
  return { label: `worth ${p}% (${success}/${success + failure})`, tone, icon: "star" };
}

/**
 * Staleness cue from a normalized 0..1 staleness score (0 = fresh, 1 = stale),
 * as carried on `MemoryFact`.
 */
export function stalenessCue(staleness: number): MemoryCue {
  const s = Math.max(0, Math.min(1, staleness));
  if (s < 0.34) return { label: "fresh", tone: "success", icon: "clock" };
  if (s < 0.67) return { label: "aging", tone: "info", icon: "clock" };
  return { label: "stale", tone: "warning", icon: "clock" };
}

/**
 * Staleness cue from the class string the truth engine assigns (Inspector
 * detail), e.g. "Fast" / "Slow" / "Static" / "Permanent".
 */
export function stalenessClassCue(stalenessClass: string): MemoryCue {
  const c = (stalenessClass || "unknown").toLowerCase();
  const tone: BadgeTone = c === "fast" ? "warning" : c === "permanent" || c === "static" ? "success" : "info";
  return { label: `staleness: ${stalenessClass || "unknown"}`, tone, icon: "clock" };
}

/**
 * Verification / truth-state cue from the memory's lifecycle state string
 * (Inspector detail). Active/verified reads as success; forgotten/superseded as
 * warning/info; deleted as danger.
 */
export function stateCue(state: string): MemoryCue {
  const s = (state || "unknown").toLowerCase();
  switch (s) {
    case "active":
      return { label: "active", tone: "success", icon: "check-circle" };
    case "forgotten":
      return { label: "forgotten", tone: "warning", icon: "eye-off" };
    case "superseded":
      return { label: "superseded", tone: "info", icon: "layers" };
    case "deleted":
      return { label: "deleted", tone: "danger", icon: "trash-2" };
    default:
      return { label: state || "unknown", tone: "neutral", icon: "info" };
  }
}
