/**
 * Current Work Summary — a READ-ONLY, DERIVED projection (UIE-H-010, Req 8.1–8.4).
 *
 * This module projects a single concise view of KRIA's current/resumable work
 * from EXISTING authoritative signals. It is intentionally NOT a store:
 *
 *   • No signals of its own, no setters, no side effects, no timers.
 *   • It NEVER owns task/approval/work lifecycle — it only reflects owners.
 *   • It NEVER mutates runtime task, route, model, or approval state
 *     (design.md §20.1 read-only-projection invariant).
 *   • It NEVER adds a backend API — it reads what the frontend already holds.
 *
 * Every projected fact carries its SOURCE OWNER and SOURCE-OWNED id/status, so
 * presentation surfaces (task 5.3: PresenceBar, StatusLine, WorkLane, Inspector)
 * can deep-link back to the real owner. Absent / unknown values are OMITTED and
 * never inferred (Req 8.4): a field is present only when its authoritative
 * source provides it.
 *
 * ── Signal inventory (fact → source store/field → owner → status) ──────────────
 *  activity  ← coreStore.state()                     owner: coreStore          status: CoreState (idle omitted)
 *  work      ← converseStore.workBlocks()            owner: converseStore      status: WorkBlockStatus
 *  work      ← activeGuiCognitionSession()           owner: guiCognitionSession status: GuiCognitionLifecycle
 *  approvals ← approvalStore.pendingCount/highRisk   owner: approvalStore      status: "pending"
 *  model     ← capabilityStore.activeLlmRuntime()    owner: capabilityStore    status: active | disabled
 *  context   ← converseStore.contextRail()           owner: converseStore      status: count/types
 *  bg (F8)   ← automationStore.workflows/running/…   owner: automationStore    status: WorkflowStatus (running/paused/failed only)
 *  bg (F9)   ← workflowStore.recentSessions()        owner: workflowStore      status: WorkflowLifecycle (non-terminal only)
 *  error     ← coreStore.errorMessage/blockReason    owner: coreStore          status: error | blocked | recovering
 *  error     ← converseStore.runtimeError()          owner: converseStore      status: error
 *  space     ← shellStore.activeSpace()              owner: shellStore         status: "active"
 *
 * ── Background work (F8/F9, task 10.3, IU-07; UIE-H-002/H-012, UIE-M-018) ──────
 * The summary now ALSO reflects CURRENT/RESUMABLE background work owned by
 * `automationStore` (n8n workflows) and `workflowStore` (workflow sessions),
 * keeping the same read-only discipline: each background item carries its SOURCE
 * OWNER + SOURCE-OWNED id/status; only non-terminal (running/paused/pending/
 * resumable-failed) items appear; terminal/settled runs are OMITTED, never
 * inferred. When the optional n8n service is offline the store simply holds no
 * workflows, so background is empty (omitted) — the summary never fabricates an
 * "n8n" state it cannot prove. Active background work IS work: it makes
 * {@link CurrentWorkSummary.hasActiveBackgroundWork} true and suppresses the
 * idle state (§8.2), but it stays SEPARATE from foreground `work` so the
 * cross-Space PresenceBar indicator keeps pointing at the Converse WorkLane.
 *
 * The pure {@link deriveCurrentWorkSummary} takes a plain snapshot so it can be
 * unit-tested with seeded states; {@link currentWorkSummary} wires the live
 * signals for reactive consumers.
 *
 * Requirements: 8.1, 8.2, 8.3, 8.4; design §11.8, §20.1; UIE-H-010, UIE-M-012.
 */
import type { Space } from "../shell/router";
import type { CoreState } from "./coreStore";
import { coreStore } from "./coreStore";
import type {
  ContextRailItem,
  WorkBlock,
  WorkBlockStatus,
  WorkBlockType,
} from "./converseStore";
import { converseStore } from "./converseStore";
import { approvalStore } from "./approvalStore";
import { capabilityStore, type ActiveLlmRuntime } from "./capabilityStore";
import { shellStore } from "./shellStore";
import {
  automationStore,
  type RunProgress,
  type Workflow,
  type WorkflowStatus,
} from "./automationStore";
import { workflowStore } from "./workflowSession";
import type {
  WorkflowLifecycle,
  WorkflowSession,
} from "../types/workflowRuntime";
import {
  activeGuiCognitionSession,
  guiCognitionRoutingStatus,
} from "./guiCognitionSession";
import type {
  GuiCognitionLifecycle,
  GuiCognitionSessionState,
} from "../types/guiCognition";

// ─── Fact types (each carries its source owner + source-owned status) ───────────

/**
 * WorkBlock statuses that represent CURRENT or RESUMABLE work. `completed` and
 * `stopped` blocks are terminal/settled and are omitted from the summary.
 * `failed` is retained because it is still actionable (resume/retry).
 */
export const ACTIVE_OR_RESUMABLE_WORK: ReadonlySet<WorkBlockStatus> = new Set([
  "pending",
  "running",
  "failed",
]);

/** The single Core-derived activity narration hook (coreStore is the owner). */
export interface WorkSummaryActivityFact {
  readonly source: "coreStore.state";
  readonly status: CoreState;
}

/** One current/resumable work item projected from an existing work owner. */
export interface WorkSummaryWorkItem {
  readonly source: "converseStore.workBlocks" | "guiCognitionSession";
  /** Source-owned id (WorkBlock.id / gui turnId|sessionId). Omitted if unknown. */
  readonly id?: string;
  /** Source-owned status verbatim — never re-derived. */
  readonly status: WorkBlockStatus | GuiCognitionLifecycle;
  readonly kind: WorkBlockType | "gui-cognition-session";
  /** Source-owned concise label (WorkBlock.summary / gui goal). Omitted if empty. */
  readonly label?: string;
}

/**
 * Automation (n8n workflow) statuses that represent CURRENT or RESUMABLE
 * background work (F8). `running`/`paused` are in-flight; `failed` is terminal
 * but still actionable (resume/retry), mirroring {@link ACTIVE_OR_RESUMABLE_WORK}.
 * `idle` (configured, never/not currently running) and `completed` (settled) are
 * OMITTED — they are not current work.
 */
export const ACTIVE_OR_RESUMABLE_AUTOMATION: ReadonlySet<WorkflowStatus> = new Set([
  "running",
  "paused",
  "failed",
]);

/**
 * Workflow-session lifecycles that represent CURRENT (in-flight / paused)
 * background work (F9). `finalized` and `cancelled` are terminal/settled and are
 * OMITTED. `hitl_pending` is a paused-awaiting-approval state and is retained.
 */
export const ACTIVE_WORKFLOW_LIFECYCLES: ReadonlySet<WorkflowLifecycle> = new Set([
  "created",
  "planned",
  "executing",
  "hitl_pending",
  "verifying",
]);

/**
 * One current/resumable BACKGROUND work item (F8 automations / F9 workflow
 * sessions). Read-only, carrying the source owner + source-owned id/status so a
 * surface can deep-link to the real owner (Automations Space / Approval Center).
 */
export interface WorkSummaryBackgroundItem {
  readonly source: "automationStore.workflows" | "workflowStore.sessions";
  /** Source-owned id (Workflow.id / WorkflowSession.workflowId). */
  readonly id: string;
  /** Source-owned status verbatim — never re-derived. */
  readonly status: WorkflowStatus | WorkflowLifecycle;
  readonly kind: "automation" | "workflow-session";
  /** Source-owned concise label. Omitted when the source supplies none. */
  readonly label?: string;
}

/** Pending-approval aggregate (approvalStore is the owner; no fact is invented). */
export interface WorkSummaryApprovalFact {
  readonly source: "approvalStore";
  readonly status: "pending";
  readonly pendingCount: number;
  readonly highRisk: boolean;
}

/** Active model fact (capabilityStore owns the active LLM runtime). */
export interface WorkSummaryModelFact {
  readonly source: "capabilityStore.activeLlmRuntime";
  /** Source-owned provider id. Omitted if the source leaves it empty. */
  readonly id?: string;
  readonly status: "active" | "disabled";
  readonly providerLabel: string;
  /** Active model name. Omitted when the source leaves it empty (never inferred). */
  readonly model?: string;
}

/** Context provenance aggregate (converseStore owns the context rail). */
export interface WorkSummaryContextFact {
  readonly source: "converseStore.contextRail";
  readonly itemCount: number;
  /** Distinct rail item types present, source-owned. */
  readonly types: readonly ContextRailItem["type"][];
}

/** Error / blocked / recovering fact. Owner + message come from the source. */
export interface WorkSummaryErrorFact {
  readonly source: "coreStore" | "converseStore.runtimeError";
  readonly status: "error" | "blocked" | "recovering";
  /** Source-owned message. Omitted when the source provides none (no fabrication). */
  readonly message?: string;
}

/** Active Space fact (shellStore owns the active Space; always known). */
export interface WorkSummarySpaceFact {
  readonly source: "shellStore.activeSpace";
  readonly id: Space;
  readonly status: "active";
}

/** The complete read-only projection. Absent facts are `null` / empty. */
export interface CurrentWorkSummary {
  readonly activity: WorkSummaryActivityFact | null;
  readonly work: readonly WorkSummaryWorkItem[];
  /**
   * Current/resumable BACKGROUND work (F8 automations + F9 workflow sessions),
   * kept separate from foreground `work`. Empty when nothing is running or the
   * optional service is offline (omitted, never inferred).
   */
  readonly background: readonly WorkSummaryBackgroundItem[];
  readonly approvals: WorkSummaryApprovalFact | null;
  readonly model: WorkSummaryModelFact | null;
  readonly context: WorkSummaryContextFact | null;
  readonly error: WorkSummaryErrorFact | null;
  readonly space: WorkSummarySpaceFact;
  /** True when at least one current/resumable foreground work item exists. */
  readonly hasActiveWork: boolean;
  /** True when at least one current/resumable background item exists (F8/F9). */
  readonly hasActiveBackgroundWork: boolean;
  /**
   * True when NO active/resumable foreground OR background work, pending
   * approval, error, or non-idle Core activity exists (Req 8.2 idle state).
   * Ambient model/context/space facts do NOT count as work and never suppress
   * the idle state; active background automations/workflow sessions DO (§8.2).
   */
  readonly isIdle: boolean;
}

/** Plain snapshot of the authoritative source signals (test-seedable). */
export interface WorkSummaryInput {
  readonly coreState: CoreState;
  readonly coreError: string | null;
  readonly coreBlockReason: string | null;
  readonly workBlocks: readonly WorkBlock[];
  readonly guiSession: GuiCognitionSessionState | null;
  readonly guiRoutingStatus: string | null;
  readonly pendingApprovals: number;
  readonly highRiskApprovals: boolean;
  readonly activeModel: ActiveLlmRuntime | null;
  readonly contextRail: readonly ContextRailItem[];
  /** F8 — automation (n8n) workflows, source-owned. */
  readonly automations: readonly Workflow[];
  /** F8 — source-owned set of currently-running workflow ids. */
  readonly runningWorkflowIds: ReadonlySet<string>;
  /** F8 — source-owned live run progress keyed by workflow id. */
  readonly runProgress: Readonly<Record<string, RunProgress>>;
  /** F9 — workflow sessions (most-recent-first), source-owned. */
  readonly workflowSessions: readonly WorkflowSession[];
  readonly runtimeError: string | null;
  readonly activeSpace: Space;
}

// ─── Derivation helpers (pure) ──────────────────────────────────────────────────

/**
 * Shared omission primitive: a string fact is present ONLY when it is a
 * non-blank string; otherwise it is absent and must be OMITTED (never inferred
 * or replaced with a placeholder). Exported so the read-only capability field
 * map (capabilityFieldMap.ts) reuses THIS omission discipline instead of
 * forking a parallel one (Req 8.4; design §20.1).
 */
export function nonEmpty(value: string | undefined | null): string | undefined {
  if (typeof value !== "string") return undefined;
  const trimmed = value.trim();
  return trimmed.length > 0 ? trimmed : undefined;
}

function deriveWork(
  workBlocks: readonly WorkBlock[],
  guiSession: GuiCognitionSessionState | null,
  guiRoutingStatus: string | null,
): WorkSummaryWorkItem[] {
  const items: WorkSummaryWorkItem[] = [];

  for (const block of workBlocks) {
    if (!ACTIVE_OR_RESUMABLE_WORK.has(block.status)) continue;
    const label = nonEmpty(block.summary);
    items.push({
      source: "converseStore.workBlocks",
      id: block.id,
      status: block.status,
      kind: block.type,
      ...(label ? { label } : {}),
    });
  }

  if (guiSession) {
    // activeGuiCognitionSession() already returns null when idle, so any session
    // here is live work. Prefer the source-owned goal summary, then the routing
    // status label; omit entirely if the source supplies neither (no fabrication).
    const id = nonEmpty(guiSession.turnId) ?? nonEmpty(guiSession.sessionId);
    const label = nonEmpty(guiSession.goalSummary) ?? nonEmpty(guiRoutingStatus);
    items.push({
      source: "guiCognitionSession",
      ...(id ? { id } : {}),
      status: guiSession.lifecycle,
      kind: "gui-cognition-session",
      ...(label ? { label } : {}),
    });
  }

  return items;
}

/**
 * Concise, source-owned label for a running workflow session (F9). A session
 * carries no title, so prefer the currently-running step's description; omit
 * entirely when the source supplies none (never fabricated).
 */
function workflowSessionLabel(session: WorkflowSession): string | undefined {
  const running = session.steps.find((s) => s.status === "running");
  return nonEmpty(running?.description);
}

/**
 * Project CURRENT/RESUMABLE background work (F8 automations + F9 workflow
 * sessions). Terminal/settled items are omitted; each retained item keeps its
 * source owner and source-owned id/status verbatim. Pure and side-effect free.
 */
function deriveBackground(
  automations: readonly Workflow[],
  runningWorkflowIds: ReadonlySet<string>,
  runProgress: Readonly<Record<string, RunProgress>>,
  workflowSessions: readonly WorkflowSession[],
): WorkSummaryBackgroundItem[] {
  const items: WorkSummaryBackgroundItem[] = [];

  for (const workflow of automations) {
    // Current if the source's running set holds it; otherwise resumable only for
    // the actionable terminal statuses. `idle`/`completed` → omitted.
    const running = runningWorkflowIds.has(workflow.id);
    if (!running && !ACTIVE_OR_RESUMABLE_AUTOMATION.has(workflow.status)) continue;
    // Prefer the source-owned workflow name; fall back to the live run message.
    const label = nonEmpty(workflow.name) ?? nonEmpty(runProgress[workflow.id]?.message);
    items.push({
      source: "automationStore.workflows",
      id: workflow.id,
      status: workflow.status,
      kind: "automation",
      ...(label ? { label } : {}),
    });
  }

  for (const session of workflowSessions) {
    if (!ACTIVE_WORKFLOW_LIFECYCLES.has(session.lifecycle)) continue;
    const label = workflowSessionLabel(session);
    items.push({
      source: "workflowStore.sessions",
      id: session.workflowId,
      status: session.lifecycle,
      kind: "workflow-session",
      ...(label ? { label } : {}),
    });
  }

  return items;
}

function deriveModel(activeModel: ActiveLlmRuntime | null): WorkSummaryModelFact | null {
  if (!activeModel) return null;
  const id = nonEmpty(activeModel.providerId);
  const model = nonEmpty(activeModel.activeModel);
  // No real provider configured → treat as unavailable and omit (Req 8.4);
  // do not surface the source's "Not configured" placeholder as a fact.
  if (!id && !model) return null;
  return {
    source: "capabilityStore.activeLlmRuntime",
    ...(id ? { id } : {}),
    status: activeModel.enabled ? "active" : "disabled",
    providerLabel: activeModel.displayName,
    ...(model ? { model } : {}),
  };
}

function deriveContext(contextRail: readonly ContextRailItem[]): WorkSummaryContextFact | null {
  if (contextRail.length === 0) return null;
  const types: ContextRailItem["type"][] = [];
  for (const item of contextRail) {
    if (!types.includes(item.type)) types.push(item.type);
  }
  return {
    source: "converseStore.contextRail",
    itemCount: contextRail.length,
    types,
  };
}

function deriveError(
  coreState: CoreState,
  coreError: string | null,
  coreBlockReason: string | null,
  runtimeError: string | null,
): WorkSummaryErrorFact | null {
  const coreErrorMsg = nonEmpty(coreError);
  if (coreErrorMsg) return { source: "coreStore", status: "error", message: coreErrorMsg };

  const runtimeMsg = nonEmpty(runtimeError);
  if (runtimeMsg) return { source: "converseStore.runtimeError", status: "error", message: runtimeMsg };

  const blockMsg = nonEmpty(coreBlockReason);
  if (blockMsg) return { source: "coreStore", status: "blocked", message: blockMsg };

  // Core reports an error/blocked/recovering state but no message is available:
  // surface the status truthfully with the message omitted (never invented).
  if (coreState === "error") return { source: "coreStore", status: "error" };
  if (coreState === "blocked") return { source: "coreStore", status: "blocked" };
  if (coreState === "recovering") return { source: "coreStore", status: "recovering" };

  return null;
}

// ─── Pure derivation ────────────────────────────────────────────────────────────

/**
 * Pure projection: authoritative snapshot → {@link CurrentWorkSummary}.
 *
 * Deterministic and side-effect free. Performs NO mutation of any store, signal,
 * or the input itself. Every produced fact is a fresh read-only object.
 */
export function deriveCurrentWorkSummary(input: WorkSummaryInput): CurrentWorkSummary {
  const work = deriveWork(input.workBlocks, input.guiSession, input.guiRoutingStatus);
  const background = deriveBackground(
    input.automations,
    input.runningWorkflowIds,
    input.runProgress,
    input.workflowSessions,
  );
  const approvals: WorkSummaryApprovalFact | null =
    input.pendingApprovals > 0
      ? { source: "approvalStore", status: "pending", pendingCount: input.pendingApprovals, highRisk: input.highRiskApprovals }
      : null;
  const model = deriveModel(input.activeModel);
  const context = deriveContext(input.contextRail);
  const error = deriveError(input.coreState, input.coreError, input.coreBlockReason, input.runtimeError);
  const activity: WorkSummaryActivityFact | null =
    input.coreState === "idle" ? null : { source: "coreStore.state", status: input.coreState };
  const space: WorkSummarySpaceFact = {
    source: "shellStore.activeSpace",
    id: input.activeSpace,
    status: "active",
  };

  const hasActiveWork = work.length > 0;
  const hasActiveBackgroundWork = background.length > 0;
  const isIdle =
    !hasActiveWork &&
    !hasActiveBackgroundWork &&
    approvals === null &&
    error === null &&
    activity === null;

  return {
    activity,
    work,
    background,
    approvals,
    model,
    context,
    error,
    space,
    hasActiveWork,
    hasActiveBackgroundWork,
    isIdle,
  };
}

// ─── Live reactive accessor ──────────────────────────────────────────────────────

/**
 * Live read-only projection wired to the authoritative signals. Safe to call
 * inside a Solid memo/JSX: it reads the source signals (establishing reactive
 * dependencies) and returns a fresh {@link CurrentWorkSummary}. It performs no
 * writes and owns no lifecycle.
 */
export function currentWorkSummary(): CurrentWorkSummary {
  return deriveCurrentWorkSummary({
    coreState: coreStore.state(),
    coreError: coreStore.errorMessage(),
    coreBlockReason: coreStore.blockReason(),
    workBlocks: converseStore.workBlocks(),
    guiSession: activeGuiCognitionSession(),
    guiRoutingStatus: guiCognitionRoutingStatus(),
    pendingApprovals: approvalStore.pendingCount(),
    highRiskApprovals: approvalStore.highRiskPending(),
    activeModel: capabilityStore.activeLlmRuntime(),
    contextRail: converseStore.contextRail(),
    automations: automationStore.workflows(),
    runningWorkflowIds: automationStore.runningWorkflowIds(),
    runProgress: automationStore.runProgress(),
    workflowSessions: workflowStore.recentSessions(),
    runtimeError: converseStore.runtimeError(),
    activeSpace: shellStore.activeSpace(),
  });
}
