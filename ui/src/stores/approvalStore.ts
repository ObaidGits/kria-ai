/**
 * Approval Store — unified queue for all HITL/decision/gui/workflow approvals.
 *
 * Single source for anything requiring user confirmation: tool HITL, interaction
 * decisions, gui-cognition approval, workflow resume.
 *
 * Requirements: 11.1 (unified Approval Center), 11.2 (risk ramp), 11.5 (deliberate confirm)
 */
import { createSignal } from "solid-js";
import { eventBus } from "./eventBus";

// ─── Types ─────────────────────────────────────────────────────────────────────

export type ApprovalType =
  | "tool-hitl"
  | "interaction-decision"
  | "gui-cognition"
  | "workflow-resume"
  /**
   * A capability run gated by the permission engine (task 8.2, Req 7.3). The
   * frontend builds this envelope after `cpp_execute` returns `needs_approval`;
   * the resolver routes an approve decision back through `cpp_approve` (at the
   * chosen scope) + `cpp_execute`. Deny persists a deny grant. KRIA stays the
   * orchestration authority — the UI never runs the capability itself.
   */
  | "capability-run";
export type RiskLevel = "green" | "yellow" | "red" | "black";
export type ApprovalStatus = "pending" | "approved" | "denied" | "kept-paused" | "expired";

/** Grant scope for an approved capability run (Req 7.3). */
export type ApprovalScope = "once" | "session" | "workspace" | "always";

/**
 * Per-source routing keys the resolver uses to send the human decision back to
 * the correct backend command (Req 11.6). Mirrors the backend `ApprovalRouting`
 * struct. Only the fields relevant to a request's source are populated.
 */
export interface ApprovalRouting {
  /** HITL request id — `approve_action` / `deny_action` (tool-hitl, gui-cognition). */
  requestId?: string;
  /** Interaction-decision id — `resolve_interaction_decision`. */
  decisionId?: string;
  /** Workflow id — `workflow_hitl_respond` / `workflow_cancel`. */
  workflowId?: string;
  /** Option id to submit on APPROVE (interaction/workflow HITL choices). */
  approveOptionId?: string;
  /** Option id to submit on DENY. */
  denyOptionId?: string;
  /** Capability provider id (capability-run) — `cpp_approve` / `cpp_execute`. */
  providerId?: string;
  /** Capability id (capability-run) — `cpp_approve` / `cpp_execute`. */
  capabilityId?: string;
  /** Optional CPP session scope key (capability-run). */
  sessionId?: string;
  /** Optional CPP workspace scope key (capability-run). */
  workspaceId?: string;
  /**
   * The args to re-invoke `cpp_execute` with once a capability run is approved
   * (capability-run). Opaque JSON — passed straight back to the runtime, never
   * inspected or executed by the UI.
   */
  capabilityArgs?: unknown;
}

/**
 * The unified approval-event shape emitted by the backend on `approval://request`
 * (design.md §3.3 contract change a). Mirrors the Rust `ApprovalEnvelope`.
 */
export interface ApprovalEnvelope {
  id: string;
  source: ApprovalType;
  title: string;
  description: string;
  risk: RiskLevel;
  effects?: string[];
  evidence?: unknown;
  irreversible?: boolean;
  scopeOptions?: string[];
  routing?: ApprovalRouting;
  payload?: unknown;
  createdAtMs?: number;
}

export interface ApprovalRequest {
  id: string;
  type: ApprovalType;
  /** What will happen — the headline (Req 11.2). */
  title: string;
  /** Why it is being requested — plain-language rationale (Req 11.2). */
  description: string;
  risk: RiskLevel;
  /** Concrete effects the action will have (Req 11.2). */
  effects?: string[];
  /**
   * Evidence KRIA used or produced (Req 11.2). A string is treated as untrusted
   * markdown/HTML and sanitized before display; a structured value is shown as
   * escaped text. NEVER rendered un-sanitized.
   */
  evidence?: unknown;
  /**
   * Whether the action is irreversible. Irreversible OR high-risk (red/black)
   * requires an explicit confirm before approval can be staged (Req 11.3).
   */
  irreversible?: boolean;
  /** Scope options offered on approve (Req 7.3); defaults to a single "once". */
  scopeOptions?: ApprovalScope[];
  /**
   * Per-source routing keys used to send the decision back to the correct
   * backend command (Req 11.6). Absent for staged/test requests.
   */
  routing?: ApprovalRouting;
  payload: unknown;
  createdAt: number;
  expiresAt?: number;
  status: ApprovalStatus;
}

/** True when a request must show an explicit confirm before approval (Req 11.3). */
export function requiresExplicitConfirm(request: Pick<ApprovalRequest, "risk" | "irreversible">): boolean {
  return request.irreversible === true || request.risk === "red" || request.risk === "black";
}

// ─── Signals ───────────────────────────────────────────────────────────────────

const [queue, setQueue] = createSignal<ApprovalRequest[]>([]);

// ─── Derived ───────────────────────────────────────────────────────────────────

const pendingCount = () => queue().filter((r) => r.status === "pending").length;
const hasPending = () => pendingCount() > 0;
const highRiskPending = () =>
  queue().some(
    (r) => r.status === "pending" && (r.risk === "red" || r.risk === "black"),
  );

// ─── Actions ───────────────────────────────────────────────────────────────────

function addRequest(request: ApprovalRequest): void {
  // Dedupe by id: a re-emitted approval (e.g. bridge reconnect) must not stack
  // duplicate cards. If an id already exists, replace it in place.
  setQueue((prev) => {
    const idx = prev.findIndex((r) => r.id === request.id);
    if (idx === -1) return [...prev, request];
    const next = prev.slice();
    next[idx] = request;
    return next;
  });
  eventBus.emit("approval:request", { id: request.id, type: request.type, payload: request.payload });
}

const VALID_SCOPES: ReadonlySet<string> = new Set(["once", "session", "workspace", "always"]);

/**
 * Ingest a unified approval envelope from the backend (`approval://request`,
 * design.md §3.3 contract change a) and enqueue it as an {@link ApprovalRequest}.
 * This is the single entry point routing ALL HITL sources — tool HITL,
 * interaction decisions, gui-cognition, workflow resume — into one queue (Req
 * 11.1). It only records the decision; resolution is routed back through the
 * runtime by the approval resolver (Req 11.6) — the store never executes.
 */
function addFromEnvelope(envelope: ApprovalEnvelope): void {
  const scopeOptions = (envelope.scopeOptions ?? [])
    .filter((s): s is ApprovalScope => VALID_SCOPES.has(s));

  addRequest({
    id: envelope.id,
    type: envelope.source,
    title: envelope.title,
    description: envelope.description,
    risk: envelope.risk,
    effects: envelope.effects,
    evidence: envelope.evidence,
    irreversible: envelope.irreversible === true,
    scopeOptions: scopeOptions.length > 0 ? scopeOptions : ["once"],
    routing: envelope.routing,
    payload: envelope.payload ?? null,
    createdAt: envelope.createdAtMs ?? Date.now(),
    status: "pending",
  });
}

/** Look up a request by id (used by the resolver to route the decision). */
function get(id: string): ApprovalRequest | undefined {
  return queue().find((r) => r.id === id);
}

/**
 * Stage an APPROVE decision (Req 11.1/11.3). Marks the request approved and
 * emits the staged decision on the bus. This does NOT execute the action — the
 * Tauri bridge routes the decision back through the runtime's approval command
 * (task 4.2), preserving KRIA's confirmation/safety authority.
 */
function approve(id: string, scope?: ApprovalScope): void {
  setQueue((prev) => prev.map((r) => (r.id === id ? { ...r, status: "approved" as const } : r)));
  eventBus.emit("approval:resolved", { id, action: "approve", scope });
}

/** Stage a DENY decision (Req 11.3 — always one action). */
function deny(id: string, reason?: string): void {
  setQueue((prev) => prev.map((r) => (r.id === id ? { ...r, status: "denied" as const } : r)));
  eventBus.emit("approval:resolved", { id, action: "deny", reason });
}

/**
 * Stage a KEEP-PAUSED decision (Req 11.3 — always one action). Leaves the agent
 * paused for a later decision; removes the item from the blocking pending queue
 * without approving or denying it.
 */
function keepPaused(id: string): void {
  setQueue((prev) => prev.map((r) => (r.id === id ? { ...r, status: "kept-paused" as const } : r)));
  eventBus.emit("approval:resolved", { id, action: "keep-paused" });
}

function expireOld(): void {
  const now = Date.now();
  setQueue((prev) =>
    prev.map((r) =>
      r.status === "pending" && r.expiresAt && r.expiresAt < now
        ? { ...r, status: "expired" as const }
        : r
    )
  );
}

function clearResolved(): void {
  setQueue((prev) => prev.filter((r) => r.status === "pending"));
}

/**
 * Remove a request outright (no decision recorded). Used when the underlying
 * work ends on its own — e.g. a workflow finalizes or is cancelled elsewhere —
 * so a now-moot approval card does not linger in the Approval Center. Unlike
 * deny/keep-paused this stages NO decision and routes NOTHING to the runtime.
 */
function dismiss(id: string): void {
  setQueue((prev) => prev.filter((r) => r.id !== id));
}

// ─── Export ────────────────────────────────────────────────────────────────────

export const approvalStore = {
  queue,
  pendingCount,
  hasPending,
  highRiskPending,

  setQueue,
  addRequest,
  addFromEnvelope,
  get,
  approve,
  deny,
  keepPaused,
  expireOld,
  clearResolved,
  dismiss,
} as const;
