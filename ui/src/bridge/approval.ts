/**
 * Approval Bridge — unified approval intake + decision routing.
 *
 * Two responsibilities (kria-ui-redesign task 4.2, Req 11.1 / 11.6 / 3.3):
 *
 *  1. INTAKE — validate the backend's unified `approval://request` envelope
 *     (design.md §3.3 contract change a) before it enters `approvalStore`.
 *
 *  2. RESOLUTION — subscribe to the staged `approval:resolved` bus decision and
 *     route it back through the runtime's OWN resolution command, chosen by the
 *     request's source type. The UI NEVER executes the approved action itself —
 *     it hands the human decision to the runtime, which enforces policy and
 *     executes. KRIA stays the orchestration authority (architecture invariant).
 *
 * Every backend call uses `bridgeInvoke` so an unavailable/unregistered command
 * degrades gracefully (Req 20.4) instead of throwing.
 */

import { eventBus, type Unsubscribe } from "../stores/eventBus";
import {
  approvalStore,
  type ApprovalEnvelope,
  type ApprovalType,
} from "../stores/approvalStore";
import { bridgeInvoke } from "./invoke";

/** Canonical Tauri channel carrying the unified approval envelope. */
export const APPROVAL_REQUEST_CHANNEL = "approval://request";

const VALID_SOURCES: ReadonlySet<ApprovalType> = new Set([
  "tool-hitl",
  "interaction-decision",
  "gui-cognition",
  "workflow-resume",
  "capability-run",
]);

const VALID_RISKS = new Set(["green", "yellow", "red", "black"]);

// ─── Intake / validation ─────────────────────────────────────────────────────

/**
 * Validate and normalize a raw Tauri payload into an {@link ApprovalEnvelope}.
 * Returns null for anything malformed (missing id/source, unknown source) so
 * the bridge can drop it without crashing. Untrusted-source discipline: fields
 * are copied explicitly, never spread blindly.
 */
export function coerceApprovalEnvelope(raw: unknown): ApprovalEnvelope | null {
  if (!raw || typeof raw !== "object") return null;
  const p = raw as Record<string, unknown>;

  const id = typeof p.id === "string" ? p.id : null;
  const source = p.source as ApprovalType;
  if (!id || !VALID_SOURCES.has(source)) return null;

  const risk = VALID_RISKS.has(p.risk as string) ? (p.risk as ApprovalEnvelope["risk"]) : "yellow";

  const routingRaw = (p.routing && typeof p.routing === "object" ? p.routing : {}) as Record<
    string,
    unknown
  >;
  const str = (v: unknown): string | undefined => (typeof v === "string" ? v : undefined);

  return {
    id,
    source,
    title: typeof p.title === "string" ? p.title : "Approval required",
    description: typeof p.description === "string" ? p.description : "",
    risk,
    effects: Array.isArray(p.effects) ? p.effects.filter((e): e is string => typeof e === "string") : undefined,
    evidence: p.evidence,
    irreversible: p.irreversible === true,
    scopeOptions: Array.isArray(p.scopeOptions)
      ? p.scopeOptions.filter((s): s is string => typeof s === "string")
      : undefined,
    routing: {
      requestId: str(routingRaw.requestId),
      decisionId: str(routingRaw.decisionId),
      workflowId: str(routingRaw.workflowId),
      approveOptionId: str(routingRaw.approveOptionId),
      denyOptionId: str(routingRaw.denyOptionId),
      providerId: str(routingRaw.providerId),
      capabilityId: str(routingRaw.capabilityId),
      sessionId: str(routingRaw.sessionId),
      workspaceId: str(routingRaw.workspaceId),
      capabilityArgs: routingRaw.capabilityArgs,
    },
    payload: p.payload,
    createdAtMs: typeof p.createdAtMs === "number" ? p.createdAtMs : undefined,
  };
}

export function mirrorApprovalPresentation(envelope: ApprovalEnvelope): void {
  void bridgeInvoke<void>("mirror_approval_presentation", { envelope });
}

// ─── Resolution routing ───────────────────────────────────────────────────────

export type ResolveAction = "approve" | "deny" | "keep-paused";

/** Outcome of routing a decision — surfaced for tests + debug logging. */
export interface ResolveOutcome {
  /** The backend command invoked, or null when no backend action is needed. */
  command: string | null;
  /** Whether the invoke succeeded (true when command is null / keep-paused). */
  ok: boolean;
  /** Human-readable note (unavailable service, missing routing, etc.). */
  note?: string;
}

/**
 * Route a resolved approval decision to the correct backend command for its
 * source type. Pure of UI concerns and independently testable.
 *
 * Mapping (Req 11.6):
 *  - tool-hitl / gui-cognition → `approve_action` / `deny_action` (by requestId)
 *  - interaction-decision      → `resolve_interaction_decision` (decisionId + optionId)
 *  - workflow-resume           → `workflow_hitl_respond` (workflowId + optionId) or `workflow_cancel`
 *  - keep-paused (any source)  → no backend call (agent stays paused for later)
 */
export async function routeApprovalDecision(
  source: ApprovalType,
  action: ResolveAction,
  routing: ApprovalEnvelope["routing"] | undefined,
  opts?: { scope?: string; reason?: string },
): Promise<ResolveOutcome> {
  // keep-paused never calls the backend — the runtime remains paused until a
  // later approve/deny (Req 11.3).
  if (action === "keep-paused") {
    return { command: null, ok: true, note: "kept paused — no backend action" };
  }

  const r = routing ?? {};

  switch (source) {
    case "tool-hitl":
    case "gui-cognition": {
      if (!r.requestId) return { command: null, ok: false, note: "missing requestId" };
      if (action === "approve") {
        return invokeResolution("approve_action", { requestId: r.requestId });
      }
      return invokeResolution("deny_action", { requestId: r.requestId, reason: opts?.reason });
    }

    case "interaction-decision": {
      if (!r.decisionId) return { command: null, ok: false, note: "missing decisionId" };
      const optionId = action === "approve" ? r.approveOptionId : r.denyOptionId;
      if (!optionId) return { command: null, ok: false, note: "missing optionId for decision" };
      return invokeResolution("resolve_interaction_decision", {
        decisionId: r.decisionId,
        optionId,
      });
    }

    case "workflow-resume": {
      if (!r.workflowId) return { command: null, ok: false, note: "missing workflowId" };
      if (action === "deny") {
        // Deny on a workflow HITL cancels the run — cancellation propagates to
        // the runtime (preserves the cancellation-propagation invariant).
        return invokeResolution("workflow_cancel", { workflowId: r.workflowId });
      }
      const optionId = r.approveOptionId;
      if (!optionId) return { command: null, ok: false, note: "missing workflow option" };
      return invokeResolution("workflow_hitl_respond", {
        workflowId: r.workflowId,
        optionId,
        actionType: "approve",
        value: null,
      });
    }

    case "capability-run":
      return routeCapabilityRun(action, r, opts);
  }
}

/**
 * Route a capability-run decision back through the runtime's permission engine
 * (Req 7.3 / 11.6). The UI NEVER executes the capability — it persists the human
 * decision via `cpp_approve` and, on approve, re-invokes the gated `cpp_execute`
 * (which re-authorizes against the fresh grant before running). Substrate
 * self-authority is impossible: the runtime owns both the grant and the run.
 *
 *  • DENY    → `cpp_approve(allow=false)` persists a deny at "once" scope so the
 *              next attempt re-prompts. No execution.
 *  • APPROVE → `cpp_approve(allow=true, scope)` at the chosen scope
 *              (once/session/workspace/always, Req 7.3), then re-`cpp_execute`.
 */
async function routeCapabilityRun(
  action: ResolveAction,
  routing: NonNullable<ApprovalEnvelope["routing"]>,
  opts?: { scope?: string; reason?: string },
): Promise<ResolveOutcome> {
  const providerId = routing.providerId;
  const capabilityId = routing.capabilityId;
  if (!providerId || !capabilityId) {
    return { command: null, ok: false, note: "missing provider/capability id" };
  }

  const scopeArgs = {
    providerId,
    capabilityId,
    sessionId: routing.sessionId ?? null,
    workspaceId: routing.workspaceId ?? null,
  };

  if (action === "deny") {
    const outcome = await invokeResolution("cpp_approve", {
      ...scopeArgs,
      scope: "once",
      allow: false,
    });
    emitCapabilityRunResult(providerId, capabilityId, "denied", opts?.reason);
    return outcome;
  }

  // Approve: persist the grant at the chosen scope, then re-run through the gate.
  const scope = VALID_SCOPE_NAMES.has(opts?.scope ?? "") ? opts!.scope! : "once";
  const grant = await bridgeInvoke("cpp_approve", { ...scopeArgs, scope, allow: true });
  if (!grant.ok) {
    return { command: "cpp_approve", ok: false, note: `cpp_approve failed: ${grant.message}` };
  }

  const exec = await bridgeInvoke<{ status?: string; reason?: string }>("cpp_execute", {
    providerId,
    capabilityId,
    args: routing.capabilityArgs ?? {},
    sessionId: routing.sessionId ?? null,
    workspaceId: routing.workspaceId ?? null,
  });
  if (!exec.ok) {
    emitCapabilityRunResult(providerId, capabilityId, "error", exec.message);
    return { command: "cpp_execute", ok: false, note: `cpp_execute failed: ${exec.message}` };
  }

  const status = String(exec.data?.status ?? "ok");
  emitCapabilityRunResult(providerId, capabilityId, status, exec.data?.reason ?? undefined);
  return { command: "cpp_execute", ok: true, note: `run status: ${status}` };
}

const VALID_SCOPE_NAMES: ReadonlySet<string> = new Set([
  "once",
  "session",
  "workspace",
  "always",
]);

function emitCapabilityRunResult(
  providerId: string,
  capabilityId: string,
  status: string,
  reason?: string,
): void {
  eventBus.emit("capability:run-result", { providerId, capabilityId, status, reason });
}

async function invokeResolution(
  command: string,
  args: Record<string, unknown>,
): Promise<ResolveOutcome> {
  const result = await bridgeInvoke(command, args);
  if (result.ok) return { command, ok: true };

  // Graceful degradation (Req 20.4): an unavailable/unregistered command is
  // reported, not thrown. The decision stays staged in the store so the user
  // can retry once the service is back.
  const note =
    result.code === "unavailable"
      ? `${command} unavailable`
      : result.code === "timeout"
        ? `${command} timed out`
        : `${command} failed: ${result.message}`;
  return { command, ok: false, note };
}

// ─── Wiring ────────────────────────────────────────────────────────────────────

let unsubscribe: Unsubscribe | null = null;

/**
 * Subscribe the resolver to the staged `approval:resolved` bus decision. Called
 * once at bridge init. Idempotent. The resolver looks the request up in the
 * store by id, reads its source + routing, and routes to the runtime.
 */
export function initApprovalResolver(): Unsubscribe {
  if (unsubscribe) return disposeApprovalResolver;

  unsubscribe = eventBus.on("approval:resolved", (payload) => {
    const request = approvalStore.get(payload.id);
    if (!request) return;

    const resolvedStatus = payload.action === "approve"
      ? "approved"
      : payload.action === "deny"
        ? "denied"
        : "kept-paused";
    const syncPresentation = () => bridgeInvoke<void>("sync_approval_presentation", {
      id: payload.id,
      status: resolvedStatus,
    });

    // Staged/test requests have no runtime routing. They still synchronize
    // presentation state, but never gain an execution shortcut.
    if (!request.routing) {
      void syncPresentation();
      return;
    }

    void routeApprovalDecision(request.type, payload.action, request.routing, {
      scope: payload.scope,
      reason: payload.reason,
    }).then((outcome) => {
      if (outcome.ok) {
        // Only mirror dismissal after the existing runtime authority accepts
        // the decision. This command has presentation state only.
        void syncPresentation();
      } else if (outcome.note && import.meta.env.DEV) {
        console.warn(`[approval] resolution not completed for ${payload.id}: ${outcome.note}`);
      }
    });
  });

  return disposeApprovalResolver;
}

/** Detach the resolver subscription. */
export function disposeApprovalResolver(): void {
  if (unsubscribe) {
    unsubscribe();
    unsubscribe = null;
  }
}
