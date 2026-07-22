/**
 * permissionUx — unit + property tests (design.md §10.4 / §19 "Permission",
 * Requirement 10).
 *
 * Named correctness properties for the Permission UX mapping (task 8.5):
 *   • Property P1 — tier → mode mapping is TOTAL & CORRECT: every RiskLevel maps
 *     to exactly one mode; green→report, yellow→intent, red|black→decision.
 *     **Validates: Requirements 10.1, 10.2, 10.3**
 *   • Property P2 — GREEN always offers undo when reversible, and a report is
 *     always non-blocking.
 *     **Validates: Requirements 10.1**
 *   • Property P3 — RED/BLACK always yields a single-line decision (never a
 *     report/intent) AND never stacks over an open overlay (no modal-on-modal).
 *     **Validates: Requirements 10.3**
 *   • Property P4 — YELLOW always yields an intent view with a bounded halt
 *     window.
 *     **Validates: Requirements 10.2**
 *   • Property P5 — deferral is total: whenever a blocking overlay is open, NO
 *     tier produces an inline surface (always `deferred`).
 *     **Validates: Requirements 10.3**
 */
import { describe, it, expect } from "vitest";
import fc from "fast-check";

import type { ApprovalRequest, RiskLevel } from "../../../stores/approvalStore";
import {
  resolvePermissionMode,
  resolvePermissionView,
  selectPermissionSubject,
  shouldDeferToActiveOverlay,
  toPermissionSubject,
  HALT_WINDOW_MS,
  type OverlayState,
  type PermissionSubject,
} from "./permissionUx";

// ─── Generators ──────────────────────────────────────────────────────────────

const ALL_RISKS: readonly RiskLevel[] = ["green", "yellow", "red", "black"];
const arbRisk = fc.constantFrom<RiskLevel>(...ALL_RISKS);

const arbOverlay: fc.Arbitrary<OverlayState> = fc.record({
  approvalCenterOpen: fc.boolean(),
  modalOpen: fc.boolean(),
});

function subject(over: Partial<PermissionSubject> = {}): PermissionSubject {
  const risk = over.risk ?? "red";
  return {
    requestId: over.requestId ?? "req-1",
    risk,
    mode: over.mode ?? resolvePermissionMode(risk),
    what: over.what ?? "Delete 12 files in ~/work",
    why: over.why ?? "You asked me to clean up the build output",
    reversible: over.reversible ?? true,
    createdAt: over.createdAt ?? 1_000,
  };
}

const arbSubject: fc.Arbitrary<PermissionSubject> = fc
  .record({
    requestId: fc.string({ minLength: 1, maxLength: 8 }),
    risk: arbRisk,
    what: fc.string({ maxLength: 40 }),
    why: fc.string({ maxLength: 40 }),
    reversible: fc.boolean(),
    createdAt: fc.integer({ min: 0, max: 1_000_000 }),
  })
  .map((r) => subject(r));

const CLOSED_OVERLAY: OverlayState = { approvalCenterOpen: false, modalOpen: false };

function request(over: Partial<ApprovalRequest> = {}): ApprovalRequest {
  return {
    id: over.id ?? "a1",
    type: over.type ?? "tool-hitl",
    title: over.title ?? "Do a thing",
    description: over.description ?? "because reasons",
    risk: over.risk ?? "green",
    irreversible: over.irreversible,
    payload: null,
    createdAt: over.createdAt ?? 1,
    status: over.status ?? "pending",
  };
}

// ═══════════════════════════════════════════════════════════════════════════
// resolvePermissionMode — tier → mode (P1)
// ═══════════════════════════════════════════════════════════════════════════

describe("resolvePermissionMode — tier → mode mapping", () => {
  it("maps each concrete tier to its documented mode (Req 10.1/10.2/10.3)", () => {
    expect(resolvePermissionMode("green")).toBe("report");
    expect(resolvePermissionMode("yellow")).toBe("intent");
    expect(resolvePermissionMode("red")).toBe("decision");
    expect(resolvePermissionMode("black")).toBe("decision");
  });

  it("Property P1: is total & correct over every RiskLevel", () => {
    fc.assert(
      fc.property(arbRisk, (risk) => {
        const mode = resolvePermissionMode(risk);
        // Always one of the three modes (total, never throws/undefined).
        expect(["report", "intent", "decision"]).toContain(mode);
        // And the mapping is exactly the documented one.
        const expected =
          risk === "green" ? "report" : risk === "yellow" ? "intent" : "decision";
        expect(mode).toBe(expected);
      }),
    );
  });
});

// ═══════════════════════════════════════════════════════════════════════════
// toPermissionSubject — projection
// ═══════════════════════════════════════════════════════════════════════════

describe("toPermissionSubject — approval projection", () => {
  it("carries what/why and derives the mode from risk", () => {
    const s = toPermissionSubject(request({ risk: "yellow", title: "Send email", description: "draft ready" }));
    expect(s.mode).toBe("intent");
    expect(s.what).toBe("Send email");
    expect(s.why).toBe("draft ready");
  });

  it("is reversible unless the request is explicitly irreversible", () => {
    expect(toPermissionSubject(request({ irreversible: true })).reversible).toBe(false);
    expect(toPermissionSubject(request({ irreversible: false })).reversible).toBe(true);
    expect(toPermissionSubject(request({})).reversible).toBe(true);
  });
});

// ═══════════════════════════════════════════════════════════════════════════
// selectPermissionSubject — deterministic single-subject selection
// ═══════════════════════════════════════════════════════════════════════════

describe("selectPermissionSubject — one subject, deterministic", () => {
  it("returns undefined when nothing is pending", () => {
    expect(selectPermissionSubject([])).toBeUndefined();
    expect(selectPermissionSubject([request({ status: "approved" })])).toBeUndefined();
  });

  it("prefers the highest risk tier (RED over YELLOW over GREEN)", () => {
    const picked = selectPermissionSubject([
      request({ id: "g", risk: "green", createdAt: 100 }),
      request({ id: "y", risk: "yellow", createdAt: 100 }),
      request({ id: "r", risk: "red", createdAt: 100 }),
    ]);
    expect(picked?.requestId).toBe("r");
  });

  it("breaks ties by recency then id (fully deterministic)", () => {
    const picked = selectPermissionSubject([
      request({ id: "r1", risk: "red", createdAt: 5 }),
      request({ id: "r2", risk: "red", createdAt: 9 }),
    ]);
    expect(picked?.requestId).toBe("r2");
  });

  it("ignores non-pending requests", () => {
    const picked = selectPermissionSubject([
      request({ id: "done", risk: "red", createdAt: 100, status: "approved" }),
      request({ id: "live", risk: "yellow", createdAt: 1 }),
    ]);
    expect(picked?.requestId).toBe("live");
  });
});

// ═══════════════════════════════════════════════════════════════════════════
// shouldDeferToActiveOverlay — no-modal-on-modal gate
// ═══════════════════════════════════════════════════════════════════════════

describe("shouldDeferToActiveOverlay — no modal-on-modal", () => {
  it("defers iff a blocking overlay is open", () => {
    expect(shouldDeferToActiveOverlay(CLOSED_OVERLAY)).toBe(false);
    expect(shouldDeferToActiveOverlay({ approvalCenterOpen: true, modalOpen: false })).toBe(true);
    expect(shouldDeferToActiveOverlay({ approvalCenterOpen: false, modalOpen: true })).toBe(true);
    expect(shouldDeferToActiveOverlay({ approvalCenterOpen: true, modalOpen: true })).toBe(true);
  });
});

// ═══════════════════════════════════════════════════════════════════════════
// resolvePermissionView — presence view resolution (P2–P5)
// ═══════════════════════════════════════════════════════════════════════════

describe("resolvePermissionView — presence views", () => {
  it("yields `none` when there is no subject", () => {
    expect(resolvePermissionView(undefined, CLOSED_OVERLAY)).toEqual({ kind: "none" });
  });

  it("GREEN → report with undo when reversible (Req 10.1)", () => {
    const v = resolvePermissionView(subject({ risk: "green", reversible: true }), CLOSED_OVERLAY);
    expect(v.kind).toBe("report");
    if (v.kind === "report") expect(v.undo).toBe(true);
  });

  it("GREEN irreversible → report without undo (Req 10.1 'where applicable')", () => {
    const v = resolvePermissionView(subject({ risk: "green", reversible: false }), CLOSED_OVERLAY);
    expect(v.kind).toBe("report");
    if (v.kind === "report") expect(v.undo).toBe(false);
  });

  it("YELLOW → intent with the halt window (Req 10.2)", () => {
    const v = resolvePermissionView(subject({ risk: "yellow" }), CLOSED_OVERLAY);
    expect(v.kind).toBe("intent");
    if (v.kind === "intent") expect(v.haltWindowMs).toBe(HALT_WINDOW_MS);
  });

  it("RED → single-line decision with what/why (Req 10.3/10.4)", () => {
    const v = resolvePermissionView(
      subject({ risk: "red", what: "Overwrite prod config", why: "you approved the rollout" }),
      CLOSED_OVERLAY,
    );
    expect(v.kind).toBe("decision");
    if (v.kind === "decision") {
      expect(v.what).toBe("Overwrite prod config");
      expect(v.why).toBe("you approved the rollout");
      expect(v.blockedContext).toBe(false);
    }
  });

  it("RED in a blocked context carries the calm posture flag (Req 26.3)", () => {
    const v = resolvePermissionView(subject({ risk: "red" }), CLOSED_OVERLAY, {
      blockedContext: true,
    });
    expect(v.kind === "decision" && v.blockedContext).toBe(true);
  });

  it("defers to an open Approval Center — never stacks (Req 10.3)", () => {
    const v = resolvePermissionView(subject({ risk: "red", requestId: "r9" }), {
      approvalCenterOpen: true,
      modalOpen: false,
    });
    expect(v).toEqual({ kind: "deferred", requestId: "r9" });
  });

  // ── Properties ────────────────────────────────────────────────────────────

  it("Property P2: a report is non-blocking and offers undo iff reversible", () => {
    fc.assert(
      fc.property(arbSubject, (s) => {
        const green = { ...s, risk: "green" as RiskLevel, mode: "report" as const };
        const v = resolvePermissionView(green, CLOSED_OVERLAY);
        expect(v.kind).toBe("report");
        if (v.kind === "report") {
          // A report never blocks (no allow/deny fields exist on it) and its
          // undo flag mirrors reversibility exactly.
          expect(v.undo).toBe(green.reversible);
        }
      }),
    );
  });

  it("Property P3: RED/BLACK is always a single decision and never stacks", () => {
    fc.assert(
      fc.property(arbSubject, arbOverlay, (s, overlay) => {
        const red: PermissionSubject = {
          ...s,
          risk: "black",
          mode: "decision",
        };
        const v = resolvePermissionView(red, overlay);
        if (overlay.approvalCenterOpen || overlay.modalOpen) {
          // No-modal-on-modal: defers, never an inline decision surface.
          expect(v.kind).toBe("deferred");
        } else {
          expect(v.kind).toBe("decision");
          // Never a report/intent for a RED-band tier.
          expect(v.kind).not.toBe("report");
          expect(v.kind).not.toBe("intent");
        }
      }),
    );
  });

  it("Property P4: YELLOW always yields a bounded halt window", () => {
    fc.assert(
      fc.property(arbSubject, (s) => {
        const yellow: PermissionSubject = { ...s, risk: "yellow", mode: "intent" };
        const v = resolvePermissionView(yellow, CLOSED_OVERLAY);
        expect(v.kind).toBe("intent");
        if (v.kind === "intent") {
          expect(v.haltWindowMs).toBeGreaterThan(0);
          expect(v.haltWindowMs).toBe(HALT_WINDOW_MS);
        }
      }),
    );
  });

  it("Property P5: deferral is total — an open overlay suppresses every tier", () => {
    fc.assert(
      fc.property(arbSubject, arbOverlay, (s, overlay) => {
        const v = resolvePermissionView(s, overlay);
        if (overlay.approvalCenterOpen || overlay.modalOpen) {
          expect(v.kind).toBe("deferred");
        } else {
          // With no overlay, the view is exactly the tier's mode (never deferred).
          expect(v.kind).toBe(s.mode);
        }
      }),
    );
  });
});
