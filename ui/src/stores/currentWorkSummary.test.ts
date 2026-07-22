import { describe, it, expect, beforeEach, afterEach } from "vitest";
import {
  deriveCurrentWorkSummary,
  currentWorkSummary,
  ACTIVE_OR_RESUMABLE_WORK,
  type WorkSummaryInput,
  type CurrentWorkSummary,
} from "./currentWorkSummary";
import type { WorkBlock, WorkBlockStatus } from "./converseStore";
import type { CoreState } from "./coreStore";
import type { ActiveLlmRuntime } from "./capabilityStore";
import type { Workflow } from "./automationStore";
import type { WorkflowSession } from "../types/workflowRuntime";
import type { GuiCognitionSessionState, GuiCognitionLifecycle } from "../types/guiCognition";
import type { Space } from "../shell/router";
import { coreStore } from "./coreStore";
import { converseStore } from "./converseStore";
import { approvalStore } from "./approvalStore";
import { capabilityStore } from "./capabilityStore";
import { shellStore } from "./shellStore";
import { eventBus } from "./eventBus";
import {
  handleGuiCognitionEvent,
  clearGuiCognitionSession,
  activeGuiCognitionSession,
} from "./guiCognitionSession";

let guiTurnCounter = 0;

/** Seed an active GUI-cognition session via the real event handler (leaves idle). */
function seedLiveGuiSession(): void {
  guiTurnCounter += 1;
  handleGuiCognitionEvent({
    version: 1,
    session_id: `cws-session-${guiTurnCounter}`,
    turn_id: `cws-turn-${guiTurnCounter}`,
    workflow_id: `cws-workflow-${guiTurnCounter}`,
    sequence: 1,
    timestamp_ms: Date.now(),
    event: { type: "TurnStarted" },
  } as never);
}

// ─── Seed helpers ────────────────────────────────────────────────────────────

function block(overrides: Partial<WorkBlock> = {}): WorkBlock {
  return {
    id: overrides.id ?? "wb-1",
    type: overrides.type ?? "tool-call",
    status: overrides.status ?? "running",
    summary: overrides.summary ?? "Running search",
    startedAt: overrides.startedAt ?? 1000,
    ...overrides,
  };
}

function runtime(overrides: Partial<ActiveLlmRuntime> = {}): ActiveLlmRuntime {
  return {
    providerId: "local",
    providerType: "llama-cpp",
    displayName: "Local llama",
    activeModel: "qwen2.5",
    endpoint: "http://localhost:8080",
    enabled: true,
    configured: true,
    isLocal: true,
    isLlamaCppRuntime: true,
    requiresApiKey: false,
    routingMode: "local",
    restartRequiredForLocalModelChange: false,
    routerHealthy: true,
    envWins: false,
    activeEnvVars: [],
    ...overrides,
  };
}

function guiSession(overrides: Partial<GuiCognitionSessionState> = {}): GuiCognitionSessionState {
  // The projection only reads lifecycle/turnId/sessionId/goalSummary; a minimal
  // cast is sufficient to seed the source snapshot for the pure derivation.
  return {
    lifecycle: "executing",
    lastSequence: 1,
    ...overrides,
  } as unknown as GuiCognitionSessionState;
}

function baseInput(overrides: Partial<WorkSummaryInput> = {}): WorkSummaryInput {
  return {
    coreState: "idle",
    coreError: null,
    coreBlockReason: null,
    workBlocks: [],
    guiSession: null,
    guiRoutingStatus: null,
    pendingApprovals: 0,
    highRiskApprovals: false,
    activeModel: null,
    contextRail: [],
    automations: [],
    runningWorkflowIds: new Set<string>(),
    runProgress: {},
    workflowSessions: [],
    runtimeError: null,
    activeSpace: "converse",
    ...overrides,
  };
}

// ─── Background (F8/F9) seed helpers ──────────────────────────────────────────

function workflow(overrides: Partial<Workflow> = {}): Workflow {
  return {
    id: overrides.id ?? "wf-1",
    name: overrides.name ?? "Nightly sync",
    description: overrides.description ?? "",
    status: overrides.status ?? "running",
    lastRunAt: overrides.lastRunAt ?? null,
    createdAt: overrides.createdAt ?? 0,
    ...overrides,
  };
}

function session(overrides: Partial<WorkflowSession> = {}): WorkflowSession {
  return {
    workflowId: overrides.workflowId ?? "sess-wf-1",
    lifecycle: overrides.lifecycle ?? "executing",
    executionMode: overrides.executionMode ?? { type: "structural" },
    steps: overrides.steps ?? [],
    telemetry: overrides.telemetry ?? [],
    continuationActions: overrides.continuationActions ?? [],
    startedAt: overrides.startedAt ?? 0,
    updatedAt: overrides.updatedAt ?? 0,
    source: overrides.source ?? "substrate_router",
    ...overrides,
  };
}

// ─── Idle / empty ────────────────────────────────────────────────────────────

describe("deriveCurrentWorkSummary — idle", () => {
  it("reports idle with all facts omitted when nothing is happening", () => {
    const s = deriveCurrentWorkSummary(baseInput());
    expect(s.isIdle).toBe(true);
    expect(s.hasActiveWork).toBe(false);
    expect(s.activity).toBeNull();
    expect(s.work).toEqual([]);
    expect(s.approvals).toBeNull();
    expect(s.model).toBeNull();
    expect(s.context).toBeNull();
    expect(s.error).toBeNull();
  });

  it("always exposes the active Space (always known, never inferred)", () => {
    const s = deriveCurrentWorkSummary(baseInput({ activeSpace: "memory" }));
    expect(s.space).toEqual({ source: "shellStore.activeSpace", id: "memory", status: "active" });
  });
});

// ─── Work derivation ─────────────────────────────────────────────────────────

describe("deriveCurrentWorkSummary — work", () => {
  it("includes only active/resumable WorkBlocks (pending, running, failed)", () => {
    const s = deriveCurrentWorkSummary(
      baseInput({
        workBlocks: [
          block({ id: "a", status: "pending" }),
          block({ id: "b", status: "running" }),
          block({ id: "c", status: "failed" }),
          block({ id: "d", status: "completed" }),
          block({ id: "e", status: "stopped" }),
        ],
      }),
    );
    expect(s.work.map((w) => w.id)).toEqual(["a", "b", "c"]);
    expect(s.hasActiveWork).toBe(true);
    expect(s.isIdle).toBe(false);
  });

  it("preserves source-owned id, status, and label verbatim", () => {
    const s = deriveCurrentWorkSummary(
      baseInput({ workBlocks: [block({ id: "wb-x", status: "running", type: "reasoning", summary: "Thinking hard" })] }),
    );
    expect(s.work[0]).toEqual({
      source: "converseStore.workBlocks",
      id: "wb-x",
      status: "running",
      kind: "reasoning",
      label: "Thinking hard",
    });
  });

  it("omits an empty/whitespace label rather than inferring one", () => {
    const s = deriveCurrentWorkSummary(baseInput({ workBlocks: [block({ summary: "   " })] }));
    expect(s.work[0]).not.toHaveProperty("label");
  });

  it("projects an active GUI cognition session with turnId + goal", () => {
    const s = deriveCurrentWorkSummary(
      baseInput({ guiSession: guiSession({ turnId: "turn-9", sessionId: "sess-1", goalSummary: "Open settings" }) }),
    );
    expect(s.work).toContainEqual({
      source: "guiCognitionSession",
      id: "turn-9",
      status: "executing",
      kind: "gui-cognition-session",
      label: "Open settings",
    });
  });

  it("falls back id→sessionId and label→routing status when goal/turn absent", () => {
    const s = deriveCurrentWorkSummary(
      baseInput({
        guiSession: guiSession({ sessionId: "sess-1", lifecycle: "blocked" }),
        guiRoutingStatus: "Blocked",
      }),
    );
    expect(s.work[0]).toEqual({
      source: "guiCognitionSession",
      id: "sess-1",
      status: "blocked",
      kind: "gui-cognition-session",
      label: "Blocked",
    });
  });

  it("omits gui id and label entirely when the source provides neither", () => {
    const s = deriveCurrentWorkSummary(baseInput({ guiSession: guiSession({ lifecycle: "observing" }) }));
    expect(s.work[0]).toEqual({
      source: "guiCognitionSession",
      status: "observing",
      kind: "gui-cognition-session",
    });
  });
});

// ─── Approvals ───────────────────────────────────────────────────────────────

describe("deriveCurrentWorkSummary — approvals", () => {
  it("projects the pending aggregate with high-risk flag", () => {
    const s = deriveCurrentWorkSummary(baseInput({ pendingApprovals: 3, highRiskApprovals: true }));
    expect(s.approvals).toEqual({ source: "approvalStore", status: "pending", pendingCount: 3, highRisk: true });
    expect(s.isIdle).toBe(false);
  });

  it("omits approvals when none are pending", () => {
    expect(deriveCurrentWorkSummary(baseInput({ pendingApprovals: 0 })).approvals).toBeNull();
  });
});

// ─── Model ───────────────────────────────────────────────────────────────────

describe("deriveCurrentWorkSummary — model", () => {
  it("projects an active model with source-owned provider id and model name", () => {
    const s = deriveCurrentWorkSummary(baseInput({ activeModel: runtime() }));
    expect(s.model).toEqual({
      source: "capabilityStore.activeLlmRuntime",
      id: "local",
      status: "active",
      providerLabel: "Local llama",
      model: "qwen2.5",
    });
  });

  it("marks a disabled runtime as disabled", () => {
    const s = deriveCurrentWorkSummary(baseInput({ activeModel: runtime({ enabled: false }) }));
    expect(s.model?.status).toBe("disabled");
  });

  it("omits the model fact when no provider is configured (never surfaces a placeholder)", () => {
    const s = deriveCurrentWorkSummary(
      baseInput({ activeModel: runtime({ providerId: "", activeModel: "", displayName: "Not configured" }) }),
    );
    expect(s.model).toBeNull();
  });

  it("omits the model name when the source leaves it empty", () => {
    const s = deriveCurrentWorkSummary(baseInput({ activeModel: runtime({ activeModel: "" }) }));
    expect(s.model).not.toBeNull();
    expect(s.model).not.toHaveProperty("model");
  });

  it("omits the model fact when the source signal is unavailable (null)", () => {
    expect(deriveCurrentWorkSummary(baseInput({ activeModel: null })).model).toBeNull();
  });
});

// ─── Context ─────────────────────────────────────────────────────────────────

describe("deriveCurrentWorkSummary — context", () => {
  it("projects item count and distinct types", () => {
    const s = deriveCurrentWorkSummary(
      baseInput({
        contextRail: [
          { id: "1", type: "memory", label: "m", data: null },
          { id: "2", type: "memory", label: "m2", data: null },
          { id: "3", type: "document", label: "d", data: null },
        ],
      }),
    );
    expect(s.context).toEqual({ source: "converseStore.contextRail", itemCount: 3, types: ["memory", "document"] });
  });

  it("omits context when the rail is empty", () => {
    expect(deriveCurrentWorkSummary(baseInput({ contextRail: [] })).context).toBeNull();
  });
});

// ─── Background work (F8 automations + F9 workflow sessions, task 10.3) ────────

describe("deriveCurrentWorkSummary — background work (F8/F9)", () => {
  // **Validates: Requirements 8.1, 8.2, 8.4**
  it("omits background when nothing is running (empty stores)", () => {
    const s = deriveCurrentWorkSummary(baseInput());
    expect(s.background).toEqual([]);
    expect(s.hasActiveBackgroundWork).toBe(false);
    expect(s.isIdle).toBe(true);
  });

  it("projects a running automation with source owner + source-owned status", () => {
    const s = deriveCurrentWorkSummary(
      baseInput({
        automations: [workflow({ id: "wf-run", status: "running", name: "Nightly sync" })],
        runningWorkflowIds: new Set(["wf-run"]),
      }),
    );
    expect(s.background).toEqual([
      {
        source: "automationStore.workflows",
        id: "wf-run",
        status: "running",
        kind: "automation",
        label: "Nightly sync",
      },
    ]);
    expect(s.hasActiveBackgroundWork).toBe(true);
  });

  it("retains paused + failed (resumable) automations but omits idle + completed (terminal)", () => {
    const s = deriveCurrentWorkSummary(
      baseInput({
        automations: [
          workflow({ id: "run", status: "running" }),
          workflow({ id: "pause", status: "paused" }),
          workflow({ id: "fail", status: "failed" }),
          workflow({ id: "idle", status: "idle" }),
          workflow({ id: "done", status: "completed" }),
        ],
        runningWorkflowIds: new Set(["run", "pause"]),
      }),
    );
    expect(s.background.map((b) => b.id)).toEqual(["run", "pause", "fail"]);
  });

  it("falls back to the live run message when the workflow name is blank", () => {
    const s = deriveCurrentWorkSummary(
      baseInput({
        automations: [workflow({ id: "wf", status: "running", name: "  " })],
        runningWorkflowIds: new Set(["wf"]),
        runProgress: {
          wf: {
            workflowId: "wf",
            phase: "running",
            completedSteps: 1,
            totalSteps: 3,
            message: "Step 2 of 3",
            updatedAt: 0,
          },
        },
      }),
    );
    expect(s.background[0].label).toBe("Step 2 of 3");
  });

  it("omits the automation label entirely when neither name nor run message exist", () => {
    const s = deriveCurrentWorkSummary(
      baseInput({
        automations: [workflow({ id: "wf", status: "failed", name: "   " })],
      }),
    );
    expect(s.background[0]).toEqual({
      source: "automationStore.workflows",
      id: "wf",
      status: "failed",
      kind: "automation",
    });
    expect(s.background[0]).not.toHaveProperty("label");
  });

  it("projects a non-terminal workflow session with the running step description", () => {
    const s = deriveCurrentWorkSummary(
      baseInput({
        workflowSessions: [
          session({
            workflowId: "sess-1",
            lifecycle: "executing",
            steps: [
              {
                index: 0,
                description: "Open the browser",
                stepType: "app_launch",
                executionMode: "visible",
                status: "completed",
                artifacts: [],
              },
              {
                index: 1,
                description: "Navigate to page",
                stepType: "browser_navigation",
                executionMode: "visible",
                status: "running",
                artifacts: [],
              },
            ],
          }),
        ],
      }),
    );
    expect(s.background).toEqual([
      {
        source: "workflowStore.sessions",
        id: "sess-1",
        status: "executing",
        kind: "workflow-session",
        label: "Navigate to page",
      },
    ]);
    expect(s.hasActiveBackgroundWork).toBe(true);
  });

  it("retains hitl_pending sessions but omits finalized + cancelled (terminal)", () => {
    const s = deriveCurrentWorkSummary(
      baseInput({
        workflowSessions: [
          session({ workflowId: "exec", lifecycle: "executing" }),
          session({ workflowId: "hitl", lifecycle: "hitl_pending" }),
          session({ workflowId: "done", lifecycle: "finalized" }),
          session({ workflowId: "gone", lifecycle: "cancelled" }),
        ],
      }),
    );
    expect(s.background.map((b) => b.id)).toEqual(["exec", "hitl"]);
  });

  it("omits the session label when no step is running (never fabricated)", () => {
    const s = deriveCurrentWorkSummary(
      baseInput({ workflowSessions: [session({ workflowId: "s", lifecycle: "planned", steps: [] })] }),
    );
    expect(s.background[0]).not.toHaveProperty("label");
  });

  it("makes an active background workflow suppress idle without touching foreground work", () => {
    const s = deriveCurrentWorkSummary(
      baseInput({
        automations: [workflow({ id: "wf", status: "running" })],
        runningWorkflowIds: new Set(["wf"]),
      }),
    );
    expect(s.isIdle).toBe(false);
    expect(s.hasActiveWork).toBe(false); // foreground work is untouched
    expect(s.work).toEqual([]);
  });

  it("keeps ambient model/context/space idle even though background is separate", () => {
    const s = deriveCurrentWorkSummary(
      baseInput({ activeModel: runtime(), contextRail: [{ id: "1", type: "memory", label: "m", data: null }] }),
    );
    expect(s.isIdle).toBe(true);
    expect(s.hasActiveBackgroundWork).toBe(false);
  });
});

// ─── Error precedence ──────────────────────────────────────────────────────────

describe("deriveCurrentWorkSummary — error", () => {
  it("prefers core error message over all others", () => {
    const s = deriveCurrentWorkSummary(
      baseInput({ coreError: "core boom", runtimeError: "runtime boom", coreBlockReason: "blocked reason" }),
    );
    expect(s.error).toEqual({ source: "coreStore", status: "error", message: "core boom" });
  });

  it("falls back to runtime error when no core error", () => {
    const s = deriveCurrentWorkSummary(baseInput({ runtimeError: "runtime boom" }));
    expect(s.error).toEqual({ source: "converseStore.runtimeError", status: "error", message: "runtime boom" });
  });

  it("reports blocked with the block reason", () => {
    const s = deriveCurrentWorkSummary(baseInput({ coreBlockReason: "waiting on approval" }));
    expect(s.error).toEqual({ source: "coreStore", status: "blocked", message: "waiting on approval" });
  });

  it("reports a messageless error/blocked/recovering Core state without inventing text", () => {
    expect(deriveCurrentWorkSummary(baseInput({ coreState: "error" })).error).toEqual({
      source: "coreStore",
      status: "error",
    });
    expect(deriveCurrentWorkSummary(baseInput({ coreState: "blocked" })).error).toEqual({
      source: "coreStore",
      status: "blocked",
    });
    expect(deriveCurrentWorkSummary(baseInput({ coreState: "recovering" })).error).toEqual({
      source: "coreStore",
      status: "recovering",
    });
  });

  it("omits error when nothing is wrong", () => {
    expect(deriveCurrentWorkSummary(baseInput()).error).toBeNull();
  });
});

// ─── Activity + idle interplay ───────────────────────────────────────────────

describe("deriveCurrentWorkSummary — activity + idle", () => {
  it("omits activity when Core is idle", () => {
    expect(deriveCurrentWorkSummary(baseInput({ coreState: "idle" })).activity).toBeNull();
  });

  it("projects the Core state verbatim when non-idle", () => {
    const s = deriveCurrentWorkSummary(baseInput({ coreState: "thinking" }));
    expect(s.activity).toEqual({ source: "coreStore.state", status: "thinking" });
    expect(s.isIdle).toBe(false);
  });

  it("is not idle when only an approval or error exists even with no work", () => {
    expect(deriveCurrentWorkSummary(baseInput({ pendingApprovals: 1 })).isIdle).toBe(false);
    expect(deriveCurrentWorkSummary(baseInput({ runtimeError: "boom" })).isIdle).toBe(false);
  });

  it("stays idle when only ambient model/context/space facts exist", () => {
    const s = deriveCurrentWorkSummary(
      baseInput({
        activeModel: runtime(),
        contextRail: [{ id: "1", type: "memory", label: "m", data: null }],
        activeSpace: "capabilities",
      }),
    );
    expect(s.isIdle).toBe(true);
    expect(s.hasActiveWork).toBe(false);
  });
});

// ─── No mutation / purity ──────────────────────────────────────────────────────

describe("deriveCurrentWorkSummary — purity", () => {
  it("does not mutate the input snapshot or its collections", () => {
    const input = baseInput({
      workBlocks: [block({ id: "a", status: "running" }), block({ id: "b", status: "completed" })],
      contextRail: [{ id: "1", type: "memory", label: "m", data: null }],
    });
    const workRef = input.workBlocks;
    const workLen = input.workBlocks.length;
    const railLen = input.contextRail.length;

    deriveCurrentWorkSummary(input);

    expect(input.workBlocks).toBe(workRef);
    expect(input.workBlocks.length).toBe(workLen);
    expect(input.contextRail.length).toBe(railLen);
  });

  it("produces a fresh work array (never aliases the source array)", () => {
    const input = baseInput({ workBlocks: [block({ status: "running" })] });
    const s = deriveCurrentWorkSummary(input);
    expect(s.work).not.toBe(input.workBlocks);
  });
});

// ─── Generated matrix (breadth over source combinations) ───────────────────────

describe("deriveCurrentWorkSummary — generated matrix", () => {
  // Representative subset (not full cross-product): work statuses shrink to
  // absent/active/resumable/terminal (none/running/failed/completed) — the
  // terminal branch is exercised by "completed"; gui lifecycles cover
  // absent + active + terminal. Boundary cases per dimension are preserved.
  // Was 5×6×4×2=240 → 5×4×3×2=120.
  const coreStates: CoreState[] = ["idle", "thinking", "blocked", "error", "recovering"];
  const workStatuses: (WorkBlockStatus | "none")[] = ["none", "running", "failed", "completed"];
  const guiLifecycles: (GuiCognitionLifecycle | "none")[] = ["none", "executing", "completed"];
  const approvalCounts = [0, 2];

  it("holds core invariants across every combination", () => {
    for (const coreState of coreStates) {
      for (const ws of workStatuses) {
        for (const gl of guiLifecycles) {
          for (const pending of approvalCounts) {
            const s = deriveCurrentWorkSummary(
              baseInput({
                coreState,
                workBlocks: ws === "none" ? [] : [block({ status: ws })],
                guiSession: gl === "none" ? null : guiSession({ lifecycle: gl, turnId: "t" }),
                pendingApprovals: pending,
                highRiskApprovals: pending > 0,
              }),
            );

            // hasActiveWork is exactly the presence of projected work items.
            expect(s.hasActiveWork).toBe(s.work.length > 0);

            // Every projected work item carries a source-owned status.
            for (const item of s.work) {
              expect(item.status).toBeTruthy();
              expect(["converseStore.workBlocks", "guiCognitionSession"]).toContain(item.source);
            }

            // Terminal-only WorkBlocks are never projected as active work.
            const wbItems = s.work.filter((w) => w.source === "converseStore.workBlocks");
            if (ws === "none" || ws === "completed" || ws === "stopped") {
              expect(wbItems).toHaveLength(0);
            } else {
              expect(ACTIVE_OR_RESUMABLE_WORK.has(ws as WorkBlockStatus)).toBe(true);
              expect(wbItems).toHaveLength(1);
            }

            // idle ⇒ no work, no approvals, no error, no activity.
            if (s.isIdle) {
              expect(s.hasActiveWork).toBe(false);
              expect(s.approvals).toBeNull();
              expect(s.error).toBeNull();
              expect(s.activity).toBeNull();
            }

            // Space is always present and marked active.
            expect(s.space.status).toBe("active");
          }
        }
      }
    }
  });
});

// ─── Generated matrix: active Space × approval × work × error (task 5.8) ───────
//
// Property 3 (Truthful derived state), design §11.9 / §20: the active Space is a
// Space-independent fact. Work, approval, and error facts are cross-Space and must
// derive identically regardless of which Space is active; switching Space must
// never fabricate or drop any of them, and error precedence must hold everywhere.

describe("deriveCurrentWorkSummary — Space × approval × work × error matrix (task 5.8)", () => {
  // **Validates: Requirements 8.1, 8.4, 9.1, 9.4**
  const allSpaces: Space[] = [
    "converse",
    "memory",
    "automations",
    "capabilities",
    "machines",
    "observatory",
    "settings",
  ];

  type ErrorScenario = {
    readonly name: string;
    readonly patch: Partial<WorkSummaryInput>;
    readonly expected: CurrentWorkSummary["error"];
  };
  const errorScenarios: ErrorScenario[] = [
    { name: "no-error", patch: {}, expected: null },
    {
      name: "core-error-message",
      patch: { coreState: "error", coreError: "core boom", runtimeError: "rt boom", coreBlockReason: "blk" },
      expected: { source: "coreStore", status: "error", message: "core boom" },
    },
    {
      name: "runtime-error",
      patch: { runtimeError: "rt boom", coreBlockReason: "blk" },
      expected: { source: "converseStore.runtimeError", status: "error", message: "rt boom" },
    },
    {
      name: "blocked-reason",
      patch: { coreState: "blocked", coreBlockReason: "waiting on approval" },
      expected: { source: "coreStore", status: "blocked", message: "waiting on approval" },
    },
    {
      name: "recovering-state",
      patch: { coreState: "recovering" },
      expected: { source: "coreStore", status: "recovering" },
    },
  ];

  type WorkScenario = { readonly name: string; readonly patch: Partial<WorkSummaryInput>; readonly hasWork: boolean };
  const workScenarios: WorkScenario[] = [
    { name: "no-work", patch: {}, hasWork: false },
    { name: "running-block", patch: { workBlocks: [block({ id: "wb", status: "running" })] }, hasWork: true },
    { name: "terminal-block", patch: { workBlocks: [block({ id: "wb", status: "completed" })] }, hasWork: false },
    {
      name: "gui-session",
      patch: { guiSession: guiSession({ turnId: "t", lifecycle: "executing" }) },
      hasWork: true,
    },
  ];
  const approvalScenarios = [
    { name: "no-approval", pending: 0, high: false },
    { name: "low-risk", pending: 2, high: false },
    { name: "high-risk", pending: 1, high: true },
  ];

  it("derives Space-independent work/approval/error facts across the full cross-product", () => {
    for (const err of errorScenarios) {
      for (const work of workScenarios) {
        for (const appr of approvalScenarios) {
          const commonPatch: Partial<WorkSummaryInput> = {
            ...err.patch,
            ...work.patch,
            pendingApprovals: appr.pending,
            highRiskApprovals: appr.high,
          };
          const label = `${err.name}|${work.name}|${appr.name}`;

          // Derive the SAME situation under two different active Spaces.
          const a = deriveCurrentWorkSummary(baseInput({ ...commonPatch, activeSpace: "converse" }));
          const b = deriveCurrentWorkSummary(baseInput({ ...commonPatch, activeSpace: "machines" }));

          // Only the space fact changes; every cross-Space fact is identical.
          expect(a.space, `${label}: space A`).toEqual({
            source: "shellStore.activeSpace",
            id: "converse",
            status: "active",
          });
          expect(b.space.id, `${label}: space B`).toBe("machines");
          expect(a.work, `${label}: work space-independent`).toEqual(b.work);
          expect(a.approvals, `${label}: approvals space-independent`).toEqual(b.approvals);
          expect(a.error, `${label}: error space-independent`).toEqual(b.error);
          expect(a.hasActiveWork, `${label}: hasActiveWork space-independent`).toBe(b.hasActiveWork);
          expect(a.isIdle, `${label}: isIdle space-independent`).toBe(b.isIdle);

          // Error precedence holds identically regardless of Space.
          expect(a.error, `${label}: error precedence`).toEqual(err.expected);

          // Approval projection is exact.
          if (appr.pending > 0) {
            expect(a.approvals, `${label}: approval fact`).toEqual({
              source: "approvalStore",
              status: "pending",
              pendingCount: appr.pending,
              highRisk: appr.high,
            });
          } else {
            expect(a.approvals, `${label}: no approval`).toBeNull();
          }

          // Work presence is exactly what the source provides (never inferred).
          expect(a.hasActiveWork, `${label}: hasActiveWork`).toBe(work.hasWork);

          // idle ⇔ no work, no approval, no error, no non-idle activity.
          const activityPresent = a.activity !== null;
          const expectedIdle = !work.hasWork && appr.pending === 0 && err.expected === null && !activityPresent;
          expect(a.isIdle, `${label}: idle`).toBe(expectedIdle);
        }
      }
    }
  });

  it("marks the active Space active for every one of the seven Spaces", () => {
    for (const space of allSpaces) {
      const s = deriveCurrentWorkSummary(baseInput({ activeSpace: space }));
      expect(s.space).toEqual({ source: "shellStore.activeSpace", id: space, status: "active" });
    }
  });
});

// ─── Generated matrix: Core activity × WorkBlocks × GUI session (task 5.8) ──────
//
// Task 5.1 rule: work visibility is EXACTLY (workBlocks > 0 || guiSession). Core
// activity ALONE never creates work. The projection's hasActiveWork must equal the
// presence of projected work items and match that predicate for every combination.

describe("deriveCurrentWorkSummary — Core × WorkBlocks × GUI matrix (task 5.8)", () => {
  // **Validates: Requirements 8.1, 9.1**
  // Representative subset (not full cross-product): the invariant is source-driven
  // and Core-independent, so a boundary-covering slice proves it. Core states cover
  // idle + one active (thinking) + blocked + error + recovering; work statuses cover
  // absent/active/resumable/terminal (none/running/failed/completed); gui covers
  // absent + one active + one terminal (none/executing/completed). Was 9×6×7=378.
  const coreStates: CoreState[] = ["idle", "thinking", "blocked", "error", "recovering"];
  const workStatuses: (WorkBlockStatus | "none")[] = ["none", "running", "failed", "completed"];
  const guiStates: (GuiCognitionLifecycle | "none")[] = ["none", "executing", "completed"];

  it("holds work visibility === (active workBlocks > 0 || guiSession) for every combination", () => {
    for (const coreState of coreStates) {
      for (const ws of workStatuses) {
        for (const gl of guiStates) {
          const hasActiveBlock = ws !== "none" && ACTIVE_OR_RESUMABLE_WORK.has(ws as WorkBlockStatus);
          const hasGui = gl !== "none";
          const s = deriveCurrentWorkSummary(
            baseInput({
              coreState,
              workBlocks: ws === "none" ? [] : [block({ id: "wb", status: ws })],
              guiSession: gl === "none" ? null : guiSession({ turnId: "t", lifecycle: gl }),
            }),
          );
          const label = `${coreState}|${ws}|${gl}`;

          // The Task 5.1 predicate, computed independently of Core activity.
          expect(s.hasActiveWork, `${label}: predicate`).toBe(hasActiveBlock || hasGui);
          expect(s.hasActiveWork, `${label}: matches items`).toBe(s.work.length > 0);

          // Core activity alone (no block, no gui) NEVER creates work.
          if (!hasActiveBlock && !hasGui) {
            expect(s.work, `${label}: core-alone no work`).toEqual([]);
            expect(s.hasActiveWork, `${label}: core-alone hasActiveWork`).toBe(false);
          }

          // A gui session always contributes exactly one gui work item.
          const guiItems = s.work.filter((w) => w.source === "guiCognitionSession");
          expect(guiItems.length, `${label}: gui item count`).toBe(hasGui ? 1 : 0);
        }
      }
    }
  });
});

// ─── Live accessor: reads real signals, mutates nothing ────────────────────────

describe("currentWorkSummary — live accessor", () => {
  beforeEach(() => {
    coreStore.reset();
    converseStore.clearWorkBlocks();
    converseStore.setContextRailItems([]);
    approvalStore.setQueue([]);
    shellStore.setActiveSpace("converse");
    capabilityStore.setActiveLlmRuntime(null);
  });

  afterEach(() => {
    coreStore.reset();
    converseStore.clearWorkBlocks();
    converseStore.setContextRailItems([]);
    approvalStore.setQueue([]);
    eventBus.clear();
  });

  it("reflects idle stores as an idle summary", () => {
    const s = currentWorkSummary();
    expect(s.isIdle).toBe(true);
    expect(s.space.id).toBe("converse");
  });

  it("reflects seeded work + model without altering the source signals", () => {
    converseStore.addWorkBlock(block({ id: "live-1", status: "running", summary: "Live work" }));
    capabilityStore.setActiveLlmRuntime(runtime());
    const before = converseStore.workBlocks();

    const s = currentWorkSummary();

    expect(s.hasActiveWork).toBe(true);
    expect(s.work[0].id).toBe("live-1");
    expect(s.model?.model).toBe("qwen2.5");
    // Reading the projection must not mutate the underlying store signals.
    expect(converseStore.workBlocks()).toBe(before);
    expect(converseStore.workBlocks()).toHaveLength(1);
  });

  it("returns a fresh object each call (no cached lifecycle state)", () => {
    expect(currentWorkSummary()).not.toBe(currentWorkSummary());
  });
});

// ─── Live matrix: completion cleanup, stale IDs, Space switches (task 5.8) ─────
//
// These exercise the DERIVED-EACH-READ contract against real store mutations:
// terminal work drops out (no stale running status), removed/ended sources leave
// no lingering id, and Space switches move only the space fact.

describe("currentWorkSummary — completion cleanup, stale IDs, Space switches (task 5.8)", () => {
  // **Validates: Requirements 8.1, 8.4, 9.1, 9.4**
  beforeEach(() => {
    coreStore.reset();
    converseStore.clearWorkBlocks();
    converseStore.setContextRailItems([]);
    approvalStore.setQueue([]);
    shellStore.setActiveSpace("converse");
    capabilityStore.setActiveLlmRuntime(null);
    clearGuiCognitionSession();
  });

  afterEach(() => {
    coreStore.reset();
    converseStore.clearWorkBlocks();
    converseStore.setContextRailItems([]);
    approvalStore.setQueue([]);
    clearGuiCognitionSession();
    eventBus.clear();
  });

  it("drops a WorkBlock from the projection on every terminal transition (no stale running status)", () => {
    for (const terminal of ["completed", "stopped"] as const) {
      converseStore.clearWorkBlocks();
      converseStore.addWorkBlock(block({ id: "wb-term", status: "running", summary: "Working" }));
      expect(currentWorkSummary().hasActiveWork, `${terminal}: running present`).toBe(true);

      // Source owner transitions the block to a terminal status.
      converseStore.updateWorkBlock("wb-term", { status: terminal, completedAt: Date.now() });

      const s = currentWorkSummary();
      expect(s.work.some((w) => w.id === "wb-term"), `${terminal}: dropped`).toBe(false);
      expect(s.hasActiveWork, `${terminal}: no active work`).toBe(false);
      // No lingering "running" status for the settled block anywhere in work.
      expect(s.work.every((w) => w.status !== "running"), `${terminal}: no stale running`).toBe(true);
    }
  });

  it("retains a failed block (resumable) but never a completed one alongside it", () => {
    converseStore.addWorkBlock(block({ id: "keep", status: "failed", summary: "Retryable" }));
    converseStore.addWorkBlock(block({ id: "gone", status: "completed", summary: "Done" }));
    const s = currentWorkSummary();
    expect(s.work.map((w) => w.id)).toEqual(["keep"]);
  });

  it("clears the GUI work item when the session ends (completion cleanup)", () => {
    seedLiveGuiSession();
    expect(activeGuiCognitionSession()).not.toBeNull();
    const withGui = currentWorkSummary();
    expect(withGui.work.some((w) => w.source === "guiCognitionSession")).toBe(true);
    expect(withGui.hasActiveWork).toBe(true);

    // Session ends → the projection has no gui work item on the next read.
    clearGuiCognitionSession();
    const afterEnd = currentWorkSummary();
    expect(afterEnd.work.some((w) => w.source === "guiCognitionSession")).toBe(false);
    expect(afterEnd.hasActiveWork).toBe(false);
  });

  it("does not linger a stale/removed WorkBlock id (projection is derived each read)", () => {
    converseStore.addWorkBlock(block({ id: "stale-1", status: "running", summary: "First" }));
    expect(currentWorkSummary().work.map((w) => w.id)).toEqual(["stale-1"]);

    // Remove all blocks then add a different id: the old id must not survive.
    converseStore.clearWorkBlocks();
    converseStore.addWorkBlock(block({ id: "fresh-2", status: "running", summary: "Second" }));

    const ids = currentWorkSummary().work.map((w) => w.id);
    expect(ids).not.toContain("stale-1");
    expect(ids).toEqual(["fresh-2"]);
  });

  it("switches the Space fact only, never fabricating or dropping cross-Space work/approval/error", () => {
    // Seed Space-independent facts: active work + pending approval + runtime error.
    converseStore.addWorkBlock(block({ id: "xspace", status: "running", summary: "Cross-space work" }));
    approvalStore.setQueue([
      {
        id: "appr-1",
        kind: "tool-execution",
        title: "Approve",
        description: "Please approve",
        risk: "red",
        status: "pending",
      } as never,
    ]);
    coreStore.setState("error");

    const spaces: Space[] = ["converse", "memory", "automations", "capabilities", "machines", "observatory", "settings"];
    let previous = currentWorkSummary();
    expect(previous.space.id).toBe("converse");

    for (const space of spaces) {
      shellStore.setActiveSpace(space);
      const s = currentWorkSummary();

      // Space fact tracks the switch.
      expect(s.space, `${space}: space fact`).toEqual({ source: "shellStore.activeSpace", id: space, status: "active" });

      // Cross-Space facts are neither fabricated nor dropped by the switch.
      expect(s.work, `${space}: work preserved`).toEqual(previous.work);
      expect(s.work.map((w) => w.id), `${space}: work id`).toEqual(["xspace"]);
      expect(s.approvals, `${space}: approval preserved`).toEqual(previous.approvals);
      expect(s.approvals?.pendingCount, `${space}: approval count`).toBe(1);
      expect(s.error, `${space}: error preserved`).toEqual(previous.error);
      expect(s.hasActiveWork, `${space}: hasActiveWork`).toBe(true);

      previous = s;
    }
  });
});
