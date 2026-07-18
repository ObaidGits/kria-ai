import { describe, it, expect } from "vitest";
import {
  healthPct,
  healthPresentation,
  statePresentation,
  dockerPresentation,
  testPresentation,
  formatAgo,
  formatAbsolute,
} from "./fleetPresentation";
import type { DeviceTargetView, DeviceTestResultView } from "../../../hooks/useDeviceStatus";

function device(over: Partial<DeviceTargetView> = {}): DeviceTargetView {
  return {
    targetId: "vm-1",
    displayName: "VM",
    mode: "ssh_bootstrap",
    state: "ready",
    tainted: false,
    taintReason: null,
    healthScore: 1,
    latencyEwmaMs: 40,
    recentFailureRate: 0,
    dockerHealth: "pass",
    dockerPassCount: 1,
    dockerFailCount: 0,
    dockerLastRunAtUnixMs: Date.now(),
    updatedAtUnixMs: Date.now(),
    ...over,
  };
}

describe("fleetPresentation — healthPct (task 9.1)", () => {
  it("clamps to 0–100 and penalises by recent failure rate", () => {
    expect(healthPct(device({ healthScore: 1, recentFailureRate: 0 }))).toBe(100);
    expect(healthPct(device({ healthScore: 1, recentFailureRate: 1 }))).toBe(50);
    expect(healthPct(device({ healthScore: 0.5, recentFailureRate: 0 }))).toBe(50);
  });

  it("defaults a non-positive/NaN score to full health", () => {
    expect(healthPct(device({ healthScore: 0 }))).toBe(100);
    expect(healthPct(device({ healthScore: Number.NaN }))).toBe(100);
  });
});

describe("fleetPresentation — signal mappers give icon + text, never color alone (Req 17.3)", () => {
  it("maps health bands to tone + icon + text", () => {
    expect(healthPresentation(device({ healthScore: 0.95 }))).toMatchObject({ tone: "success" });
    expect(healthPresentation(device({ healthScore: 0.6 }))).toMatchObject({ tone: "warning" });
    expect(healthPresentation(device({ healthScore: 0.2 }))).toMatchObject({ tone: "danger" });
    // Every mapping carries a non-empty icon + label.
    const p = healthPresentation(device());
    expect(p.icon.length).toBeGreaterThan(0);
    expect(p.label.length).toBeGreaterThan(0);
  });

  it("maps every state to a distinct labelled presentation", () => {
    const states: DeviceTargetView["state"][] = [
      "ready",
      "leased",
      "quarantine",
      "tainted",
      "disabled",
      "degraded",
      "unreachable",
      "unknown",
    ];
    for (const s of states) {
      const p = statePresentation(s);
      expect(p.label.length).toBeGreaterThan(0);
      expect(p.icon.length).toBeGreaterThan(0);
    }
    expect(statePresentation("ready").label).toBe("Ready");
    expect(statePresentation("tainted").tone).toBe("danger");
  });

  it("maps docker health to tone + icon + text", () => {
    expect(dockerPresentation("pass")).toMatchObject({ tone: "success", label: "Pass" });
    expect(dockerPresentation("fail")).toMatchObject({ tone: "danger", label: "Fail" });
    expect(dockerPresentation("running")).toMatchObject({ tone: "info" });
    expect(dockerPresentation("unknown")).toMatchObject({ tone: "neutral" });
  });

  it("maps test results, with an honest 'No runs' when absent", () => {
    expect(testPresentation(null)).toMatchObject({ label: "No runs" });
    const pass: DeviceTestResultView = {
      targetId: "vm-1",
      suiteName: "smoke",
      zone: "prod",
      status: "pass",
      timestampUnixMs: Date.now(),
      reportPath: "",
    };
    expect(testPresentation(pass)).toMatchObject({ tone: "success", label: "Pass" });
    expect(testPresentation({ ...pass, status: "fail" })).toMatchObject({ tone: "danger" });
    expect(testPresentation({ ...pass, status: "skip" })).toMatchObject({ label: "Skip" });
  });
});

describe("fleetPresentation — timestamp formatting", () => {
  it("formatAgo handles null / recent / minutes / hours", () => {
    expect(formatAgo(null)).toBe("never");
    expect(formatAgo(Date.now())).toBe("just now");
    expect(formatAgo(Date.now() - 90_000)).toBe("1m ago");
    expect(formatAgo(Date.now() - 7_200_000)).toBe("2h ago");
  });

  it("formatAbsolute returns 'never' for null and a string otherwise", () => {
    expect(formatAbsolute(null)).toBe("never");
    expect(typeof formatAbsolute(Date.now())).toBe("string");
    expect(formatAbsolute(Date.now()).length).toBeGreaterThan(0);
  });
});
