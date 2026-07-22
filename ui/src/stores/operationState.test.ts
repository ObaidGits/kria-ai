import { describe, it, expect } from "vitest";
import {
  OPERATION_STATES,
  OPERATION_STATE_PRIORITY,
  ATTENTION_OPERATION_STATES,
  TERMINAL_OPERATION_STATES,
  isAttentionOperation,
  normalizeMeasuredProgress,
  coreStateToOperationState,
  workBlockStatusToOperationState,
  automationStatusToOperationState,
  workflowLifecycleToOperationState,
  jobStatusToOperationState,
  resolveOperationState,
  deriveOperationSnapshot,
  type AsyncSignalInput,
  type OperationState,
} from "./operationState";
import type { CoreState } from "./coreStore";
import type { WorkBlockStatus } from "./converseStore";
import type { WorkflowStatus } from "./automationStore";
import type { JobStatus } from "./observatoryStore";
import type { WorkflowLifecycle } from "../types/workflowRuntime";

// **Validates: Requirements 13.1, 13.2, 13.5, 13.6**

const base = (over: Partial<AsyncSignalInput> = {}): AsyncSignalInput => ({
  source: "testStore",
  ...over,
});

// ─── Vocabulary shape ────────────────────────────────────────────────────────────

describe("operation vocabulary — shape", () => {
  it("covers exactly the ten Req 13.6 states", () => {
    expect([...OPERATION_STATES].sort()).toEqual(
      [
        "active",
        "blocked",
        "completed",
        "empty",
        "failed",
        "loading",
        "optional-service-unavailable",
        "recovered",
        "retrying",
        "waiting",
      ].sort(),
    );
  });

  it("assigns a precedence to every state", () => {
    for (const s of OPERATION_STATES) {
      expect(typeof OPERATION_STATE_PRIORITY[s]).toBe("number");
    }
  });

  it("ranks failure/decision/offline as attention states above idle floor", () => {
    expect([...ATTENTION_OPERATION_STATES].sort()).toEqual(
      ["blocked", "failed", "optional-service-unavailable", "waiting"].sort(),
    );
    for (const s of ATTENTION_OPERATION_STATES) {
      expect(isAttentionOperation(s)).toBe(true);
      expect(OPERATION_STATE_PRIORITY[s]).toBeGreaterThan(OPERATION_STATE_PRIORITY.empty);
    }
    expect(isAttentionOperation("active")).toBe(false);
  });

  it("treats completed/recovered/empty as terminal/settled", () => {
    expect([...TERMINAL_OPERATION_STATES].sort()).toEqual(
      ["completed", "empty", "recovered"].sort(),
    );
  });
});

// ─── Progress omission (no fabricated percentage — UIE-M-013) ────────────────────

describe("normalizeMeasuredProgress — never fabricates", () => {
  it("keeps a measured value inside [0, 1]", () => {
    expect(normalizeMeasuredProgress(0)).toBe(0);
    expect(normalizeMeasuredProgress(0.42)).toBe(0.42);
    expect(normalizeMeasuredProgress(1)).toBe(1);
  });

  it("omits missing / non-finite / out-of-range values (indeterminate, never invented)", () => {
    expect(normalizeMeasuredProgress(undefined)).toBeUndefined();
    expect(normalizeMeasuredProgress(null)).toBeUndefined();
    expect(normalizeMeasuredProgress(Number.NaN)).toBeUndefined();
    expect(normalizeMeasuredProgress(Number.POSITIVE_INFINITY)).toBeUndefined();
    expect(normalizeMeasuredProgress(-0.1)).toBeUndefined();
    expect(normalizeMeasuredProgress(1.5)).toBeUndefined();
  });
});

// ─── coreStore adapter ───────────────────────────────────────────────────────────

describe("coreStateToOperationState", () => {
  it("maps idle to empty and the attention states to their peers", () => {
    expect(coreStateToOperationState("idle")).toBe("empty");
    expect(coreStateToOperationState("waiting")).toBe("waiting");
    expect(coreStateToOperationState("blocked")).toBe("blocked");
    expect(coreStateToOperationState("error")).toBe("failed");
    expect(coreStateToOperationState("recovering")).toBe("retrying");
  });

  it("collapses every sustained activity state to active", () => {
    const activities: CoreState[] = [
      "listening",
      "thinking",
      "planning",
      "speaking",
      "acting",
      "running-automation",
      "watching",
      "remembering",
      "reflecting",
      "learning",
    ];
    for (const s of activities) expect(coreStateToOperationState(s)).toBe("active");
  });
});

// ─── converseStore WorkBlock adapter ─────────────────────────────────────────────

describe("workBlockStatusToOperationState", () => {
  const cases: Array<[WorkBlockStatus, OperationState]> = [
    ["pending", "loading"],
    ["running", "active"],
    ["completed", "completed"],
    ["failed", "failed"],
    ["stopped", "empty"],
  ];
  it.each(cases)("maps %s -> %s", (status, expected) => {
    expect(workBlockStatusToOperationState(status)).toBe(expected);
  });
});

// ─── automationStore adapter ─────────────────────────────────────────────────────

describe("automationStatusToOperationState", () => {
  const cases: Array<[WorkflowStatus, OperationState]> = [
    ["idle", "empty"],
    ["running", "active"],
    ["completed", "completed"],
    ["failed", "failed"],
    ["paused", "waiting"],
  ];
  it.each(cases)("maps %s -> %s", (status, expected) => {
    expect(automationStatusToOperationState(status)).toBe(expected);
  });
});

// ─── workflow-session lifecycle adapter ──────────────────────────────────────────

describe("workflowLifecycleToOperationState", () => {
  const cases: Array<[WorkflowLifecycle, OperationState]> = [
    ["created", "loading"],
    ["planned", "loading"],
    ["executing", "active"],
    ["verifying", "active"],
    ["hitl_pending", "blocked"],
    ["finalized", "completed"],
    ["cancelled", "empty"],
  ];
  it.each(cases)("maps %s -> %s", (lifecycle, expected) => {
    expect(workflowLifecycleToOperationState(lifecycle)).toBe(expected);
  });
});

// ─── observatory job adapter ─────────────────────────────────────────────────────

describe("jobStatusToOperationState", () => {
  const cases: Array<[JobStatus, OperationState]> = [
    ["queued", "loading"],
    ["running", "active"],
    ["paused", "waiting"],
    ["completed", "completed"],
    ["failed", "failed"],
    ["timed_out", "failed"],
    ["rolled_back", "recovered"],
    ["recovered", "recovered"],
    ["cancelled", "empty"],
    ["unknown", "empty"],
  ];
  it.each(cases)("maps %s -> %s", (status, expected) => {
    expect(jobStatusToOperationState(status)).toBe(expected);
  });
});

// ─── Generic derivation: precedence ──────────────────────────────────────────────

describe("resolveOperationState — precedence & truthful defaults", () => {
  it("defaults to empty when nothing is active", () => {
    expect(resolveOperationState(base())).toBe("empty");
    expect(resolveOperationState(base({ hasData: false }))).toBe("empty");
  });

  it("surfaces an offline OPTIONAL service above all else", () => {
    expect(
      resolveOperationState(
        base({ serviceOptional: true, serviceAvailable: false, loading: true, error: "boom" }),
      ),
    ).toBe("optional-service-unavailable");
  });

  it("does not treat a present, available optional service as unavailable", () => {
    expect(
      resolveOperationState(base({ serviceOptional: true, serviceAvailable: true, active: true })),
    ).toBe("active");
    // serviceAvailable unknown (undefined) must not fabricate unavailability
    expect(resolveOperationState(base({ serviceOptional: true, loading: true }))).toBe("loading");
  });

  it("treats a failure (flag or message) above blocked/retry/active", () => {
    expect(resolveOperationState(base({ failed: true, blocked: true, active: true }))).toBe("failed");
    expect(resolveOperationState(base({ error: "disk full", loading: true }))).toBe("failed");
  });

  it("treats blocked (flag or reason) above retry/wait/loading/active", () => {
    expect(resolveOperationState(base({ blocked: true, retrying: true }))).toBe("blocked");
    expect(resolveOperationState(base({ blockReason: "needs approval", loading: true }))).toBe("blocked");
  });

  it("orders retrying > waiting > loading > active > recovered > completed", () => {
    expect(resolveOperationState(base({ retrying: true, waiting: true }))).toBe("retrying");
    expect(resolveOperationState(base({ waiting: true, loading: true }))).toBe("waiting");
    expect(resolveOperationState(base({ loading: true, active: true }))).toBe("loading");
    expect(resolveOperationState(base({ active: true, recovered: true }))).toBe("active");
    expect(resolveOperationState(base({ recovered: true, completed: true }))).toBe("recovered");
    expect(resolveOperationState(base({ completed: true }))).toBe("completed");
  });
});

// ─── Generic derivation: snapshot omission rules ─────────────────────────────────

describe("deriveOperationSnapshot — omission & provenance", () => {
  it("always carries the source owner", () => {
    expect(deriveOperationSnapshot(base()).source).toBe("testStore");
  });

  it("omits operationId / message / progress when the source provides none", () => {
    const snap = deriveOperationSnapshot(base({ loading: true }));
    expect(snap.state).toBe("loading");
    expect(snap).not.toHaveProperty("operationId");
    expect(snap).not.toHaveProperty("message");
    expect(snap).not.toHaveProperty("progress");
  });

  it("keeps a source-owned operation id verbatim, omits blank ones", () => {
    expect(deriveOperationSnapshot(base({ operationId: "job-7", active: true })).operationId).toBe("job-7");
    expect(deriveOperationSnapshot(base({ operationId: "   ", active: true }))).not.toHaveProperty(
      "operationId",
    );
  });

  it("prefers the failure cause for failed and the block reason for blocked", () => {
    expect(deriveOperationSnapshot(base({ error: "disk full" })).message).toBe("disk full");
    expect(deriveOperationSnapshot(base({ blockReason: "needs approval" })).message).toBe(
      "needs approval",
    );
  });

  it("falls back to the generic source message and omits a blank one", () => {
    expect(deriveOperationSnapshot(base({ active: true, message: "Syncing" })).message).toBe("Syncing");
    expect(deriveOperationSnapshot(base({ active: true, message: "  " }))).not.toHaveProperty(
      "message",
    );
  });

  it("attaches measured progress only on a progress-bearing state", () => {
    expect(deriveOperationSnapshot(base({ loading: true, progress: 0.3 })).progress).toBe(0.3);
    expect(deriveOperationSnapshot(base({ active: true, progress: 0 })).progress).toBe(0);
    expect(deriveOperationSnapshot(base({ retrying: true, progress: 0.9 })).progress).toBe(0.9);
  });

  it("never surfaces progress on a non-progress-bearing state (no fabricated bar)", () => {
    expect(deriveOperationSnapshot(base({ completed: true, progress: 1 }))).not.toHaveProperty(
      "progress",
    );
    expect(deriveOperationSnapshot(base({ waiting: true, progress: 0.5 }))).not.toHaveProperty(
      "progress",
    );
    expect(deriveOperationSnapshot(base({ error: "x", progress: 0.5 }))).not.toHaveProperty(
      "progress",
    );
  });

  it("omits progress on an in-flight state when the source did not measure it (indeterminate)", () => {
    expect(deriveOperationSnapshot(base({ loading: true }))).not.toHaveProperty("progress");
    expect(deriveOperationSnapshot(base({ loading: true, progress: null }))).not.toHaveProperty(
      "progress",
    );
    expect(deriveOperationSnapshot(base({ loading: true, progress: 1.4 }))).not.toHaveProperty(
      "progress",
    );
  });
});
