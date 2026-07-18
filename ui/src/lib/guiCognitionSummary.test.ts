import { describe, it, expect } from "vitest";
import { deriveGuiCognitionSummary, sanitizeLaymanText } from "./guiCognitionSummary";
import type { GuiCognitionSessionState } from "../types/guiCognition";

/** Patterns that must never appear in the layman layer (Req 16.5). */
const HEX_DIGEST = /\b[0-9a-f]{12,}\b/i;
const UUID = /\b[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\b/i;
const ID_TOKEN =
  /\b(session|turn|workflow|context|ctx|control|proposal|execution|resolution|plan|goal|checkpoint|request|verification|target|prompt|query)[-_](?:hash[-_])?[a-z0-9]/i;
const COORD = /\b\d{2,4}\s*,\s*\d{2,4}\b|\b\d{2,5}x\d{2,5}\b/;
const SECRET = /\b(token|password|secret|api[_-]?key|bearer)\b\s*[:=]\s*\S/i;

function assertLaymanIsClean(summary: ReturnType<typeof deriveGuiCognitionSummary>): void {
  const blob = [
    summary.headline,
    summary.nextStep ?? "",
    ...summary.facts.map((f) => `${f.label} ${f.value}`),
    ...summary.warnings,
  ].join(" \u0001 ");
  expect(blob).not.toMatch(HEX_DIGEST);
  expect(blob).not.toMatch(UUID);
  expect(blob).not.toMatch(ID_TOKEN);
  expect(blob).not.toMatch(COORD);
  expect(blob).not.toMatch(SECRET);
}

function session(overrides: Record<string, unknown>): GuiCognitionSessionState {
  const base = {
    lifecycle: "completed",
    observation: {
      activeWindow: "K.R.I.A.",
      visibleControlCount: 7,
      disabledControlCount: 7,
      screenshotAvailable: true,
      ocrAvailable: true,
      accessibilityAvailable: true,
    },
    planSteps: [],
    typedPlanSteps: [],
    planStepValidationResults: [],
    recoveryOptions: [],
  };
  return { ...base, ...overrides } as unknown as GuiCognitionSessionState;
}

describe("deriveGuiCognitionSummary", () => {
  it("summarizes an observe-only completed turn in plain language", () => {
    const s = deriveGuiCognitionSummary(session({ lifecycle: "completed" }));
    expect(s.statusLabel).toBe("Completed");
    expect(s.statusTone).toBe("success");
    expect(s.headline).toContain("Observed your screen");
    expect(s.headline).toContain("No GUI action");
    // Plain language: no hashes/IDs.
    expect(s.headline).not.toMatch(/[0-9a-f]{16}/);
    const factLabels = s.facts.map((f) => f.label);
    expect(factLabels).toContain("Active window");
    expect(factLabels).toContain("Controls");
  });

  it("flags an all-disabled degraded screen as a warning", () => {
    const s = deriveGuiCognitionSummary(
      session({
        observation: {
          activeWindow: "K.R.I.A.",
          visibleControlCount: 7,
          disabledControlCount: 7,
          screenshotAvailable: true,
          ocrAvailable: true,
          accessibilityAvailable: true,
          accessibilityOverallStatus: "degraded",
          accessibilityTimeoutCount: 1,
          observationTotalMs: 2800,
        },
      }),
    );
    expect(s.warnings.length).toBeGreaterThan(0);
    expect(s.warnings.join(" ")).toMatch(/disabled|degraded|slow/i);
  });

  it("summarizes a verified executed action", () => {
    const s = deriveGuiCognitionSummary(
      session({
        lifecycle: "completed",
        currentAction: { actionKind: "ClickControl", target: "Search", status: "completed" },
        verification: { status: "verified" },
      }),
    );
    expect(s.headline).toContain("clicked the control");
    expect(s.headline).toContain("verified");
  });

  it("summarizes a needs-approval pause with a next step", () => {
    const s = deriveGuiCognitionSummary(
      session({
        lifecycle: "awaiting_approval",
        currentAction: { actionKind: "Send", target: "Send" },
      }),
    );
    expect(s.statusLabel).toBe("Needs approval");
    expect(s.statusTone).toBe("warning");
    expect(s.headline).toContain("approval");
    expect(s.nextStep).toBeTruthy();
  });

  it("summarizes a safe stop with the blocker reason", () => {
    const s = deriveGuiCognitionSummary(
      session({
        lifecycle: "blocked",
        blocker: { type: "execution", reason: "the resolved target is no longer present", options: [] },
      }),
    );
    expect(s.statusLabel).toBe("Blocked");
    expect(s.statusTone).toBe("danger");
    expect(s.headline).toContain("Stopped safely");
    expect(s.headline).toContain("no longer present");
  });

  it("summarizes a multi-step workflow completion", () => {
    const s = deriveGuiCognitionSummary(
      session({
        lifecycle: "completed",
        workflow: { status: "completed", stepCount: 2, completedStepCount: 2, steps: [] },
      }),
    );
    expect(s.headline).toContain("Completed 2 steps");
  });
});

describe("sanitizeLaymanText (Req 16.5 privacy scrub)", () => {
  it("redacts hex digests / screen hashes", () => {
    expect(sanitizeLaymanText("Screen abcdef0123456789 observed")).toBe(
      "Screen [redacted] observed",
    );
  });

  it("redacts UUIDs", () => {
    expect(
      sanitizeLaymanText("turn 123e4567-e89b-12d3-a456-426614174000 done"),
    ).toBe("turn [redacted] done");
  });

  it("redacts internal id tokens (session/turn/workflow/control/prompt hash)", () => {
    expect(sanitizeLaymanText("blocked on control-search")).toBe("blocked on [redacted]");
    expect(sanitizeLaymanText("prompt-hash-123 mismatch")).toBe("[redacted] mismatch");
    expect(sanitizeLaymanText("resolution-1 failed")).toBe("[redacted] failed");
  });

  it("redacts coordinates and pixel sizes", () => {
    expect(sanitizeLaymanText("at 12,24 size 180x32")).toBe("at [redacted] size [redacted]");
  });

  it("redacts secret markers but keeps the key name", () => {
    expect(sanitizeLaymanText("token=abc.def.ghi")).toBe("token=[redacted]");
    expect(sanitizeLaymanText("password: hunter2")).toBe("password=[redacted]");
  });

  it("leaves plain language untouched", () => {
    expect(sanitizeLaymanText("Completed 2 steps, each verified one at a time.")).toBe(
      "Completed 2 steps, each verified one at a time.",
    );
    expect(sanitizeLaymanText("Observed your screen (K.R.I.A.). No GUI action was taken.")).toBe(
      "Observed your screen (K.R.I.A.). No GUI action was taken.",
    );
  });

  it("is idempotent", () => {
    const once = sanitizeLaymanText("control-search at 12,24");
    expect(sanitizeLaymanText(once)).toBe(once);
  });
});

describe("deriveGuiCognitionSummary layman layer never leaks hashes/IDs/coords/secrets", () => {
  it("scrubs a leaky blocker reason that contains an id and a hash", () => {
    const s = deriveGuiCognitionSummary(
      session({
        lifecycle: "blocked",
        blocker: {
          type: "execution",
          reason: "control-search at 12,24 failed (hash abcdef0123456789)",
          options: [],
        },
      }),
    );
    expect(s.headline).toContain("Stopped safely");
    assertLaymanIsClean(s);
  });

  it("scrubs a leaky cancelled reason carrying a turn id", () => {
    const s = deriveGuiCognitionSummary(
      session({
        lifecycle: "cancelled",
        blocker: {
          type: "turn",
          reason: "turn-1 (workflow-9) cancelled by you",
          options: [],
        },
      }),
    );
    assertLaymanIsClean(s);
  });

  it("scrubs a leaky verified-action target that is actually a control id", () => {
    const s = deriveGuiCognitionSummary(
      session({
        lifecycle: "completed",
        currentAction: {
          actionKind: "ClickControl",
          target: "control-abcdef0123456789",
          status: "completed",
        },
        verification: { status: "verified" },
      }),
    );
    expect(s.headline).toContain("clicked the control");
    assertLaymanIsClean(s);
  });

  it("keeps the standard observe-only summary clean", () => {
    assertLaymanIsClean(deriveGuiCognitionSummary(session({ lifecycle: "completed" })));
  });
});
