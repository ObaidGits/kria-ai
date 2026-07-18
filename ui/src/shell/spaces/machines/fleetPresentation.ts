/**
 * Fleet presentation helpers — pure display mappers shared by DeviceRow and the
 * device Inspector (task 9.1). Every risk/health/state signal is mapped to an
 * icon + text label (never color alone — Req 17.3). Kept pure + framework-free
 * for unit testing.
 */
import type { BadgeTone } from "../../../kit";
import type {
  DeviceTargetView,
  DeviceTestResultView,
} from "../../../hooks/useDeviceStatus";

/** A tone + icon + text triple so consumers never rely on color alone (Req 17.3). */
export interface SignalPresentation {
  tone: BadgeTone;
  icon: string;
  label: string;
}

/** Health as a 0–100 integer, penalised by recent failure rate. */
export function healthPct(target: DeviceTargetView): number {
  const rawScore = target.healthScore;
  const score = Math.max(
    0,
    Math.min(1, typeof rawScore === "number" && Number.isFinite(rawScore) && rawScore > 0 ? rawScore : 1),
  );
  const penalty = Math.max(0, Math.min(1, target.recentFailureRate ?? 0));
  const adjusted = Math.max(0, score * (1 - penalty * 0.5));
  return Math.round(adjusted * 100);
}

/** Health band → icon + text (icon + text, never color alone — Req 17.3). */
export function healthPresentation(target: DeviceTargetView): SignalPresentation {
  const pct = healthPct(target);
  if (pct >= 80) return { tone: "success", icon: "activity", label: `${pct}% healthy` };
  if (pct >= 50) return { tone: "warning", icon: "alert-circle", label: `${pct}% degraded` };
  return { tone: "danger", icon: "alert-triangle", label: `${pct}% critical` };
}

/** Target state → tone + icon + text. */
export function statePresentation(state: DeviceTargetView["state"]): SignalPresentation {
  switch (state) {
    case "ready":
      return { tone: "success", icon: "check-circle", label: "Ready" };
    case "leased":
      return { tone: "info", icon: "lock", label: "Leased" };
    case "quarantine":
      return { tone: "warning", icon: "shield-alert", label: "Quarantine" };
    case "tainted":
      return { tone: "danger", icon: "alert-triangle", label: "Tainted" };
    case "disabled":
      return { tone: "neutral", icon: "square", label: "Disabled" };
    case "degraded":
      return { tone: "warning", icon: "activity", label: "Degraded" };
    case "unreachable":
      return { tone: "danger", icon: "alert-triangle", label: "Unreachable" };
    default:
      return { tone: "neutral", icon: "circle-help", label: "Unknown" };
  }
}

/** Docker eval health → tone + icon + text. */
export function dockerPresentation(health: DeviceTargetView["dockerHealth"]): SignalPresentation {
  switch (health) {
    case "pass":
      return { tone: "success", icon: "check-circle", label: "Pass" };
    case "fail":
      return { tone: "danger", icon: "alert-circle", label: "Fail" };
    case "running":
      return { tone: "info", icon: "loader", label: "Running" };
    default:
      return { tone: "neutral", icon: "circle-help", label: "Unknown" };
  }
}

/** Latest test result → tone + icon + text (or an honest "no runs" state). */
export function testPresentation(result: DeviceTestResultView | null): SignalPresentation {
  if (!result) return { tone: "neutral", icon: "circle", label: "No runs" };
  switch (result.status) {
    case "pass":
      return { tone: "success", icon: "check-circle", label: "Pass" };
    case "fail":
      return { tone: "danger", icon: "alert-circle", label: "Fail" };
    default:
      return { tone: "neutral", icon: "circle", label: "Skip" };
  }
}

/** Relative "…ago" string for a unix-ms timestamp. */
export function formatAgo(unixMs: number | null | undefined): string {
  if (!unixMs || Number.isNaN(unixMs)) return "never";
  const diffMs = Math.max(0, Date.now() - unixMs);
  if (diffMs < 1_000) return "just now";
  if (diffMs < 60_000) return `${Math.floor(diffMs / 1_000)}s ago`;
  if (diffMs < 3_600_000) return `${Math.floor(diffMs / 60_000)}m ago`;
  return `${Math.floor(diffMs / 3_600_000)}h ago`;
}

/** Absolute local timestamp for a unix-ms value (or "never"). */
export function formatAbsolute(unixMs: number | null | undefined): string {
  if (!unixMs || Number.isNaN(unixMs)) return "never";
  return new Date(unixMs).toLocaleString();
}
