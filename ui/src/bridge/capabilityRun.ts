/**
 * Capability run → permission-gate → Approval Center bridge
 * (kria-ui-redesign task 8.2, Req 7.3).
 *
 * A capability run is NEVER executed straight from the UI. It flows through the
 * runtime's own permission gate:
 *
 *   Intent → Capability → Policy(permission-gate) → Substrate → Tool → Verify
 *
 * `runCapability` dispatches the run to the runtime via the EXISTING gated
 * `cpp_execute` command. The runtime authorizes first:
 *   • `ok` / `declined` / `denied` → the runtime already decided; we surface the
 *     honest result. No UI-side execution ever happens.
 *   • `needs_approval` → the runtime is asking a human. We build a unified
 *     `capability-run` {@link ApprovalEnvelope} carrying the five scope options
 *     (once / session / workspace / always — plus DENY, which the Approval
 *     Center renders as its always-one-action escape) and route it into the ONE
 *     Approval Center (`approvalStore`, Req 7.3 / 11.1). The decision is routed
 *     back through `cpp_approve` (+ re-`cpp_execute`) by the approval resolver
 *     (bridge/approval.ts) — this module NEVER approves, grants, or executes.
 *
 * ── KRIA runtime-authority invariant ────────────────────────────────────────
 * This module only TRANSLATES a gate result into an approval request and
 * enqueues it. It creates no prompt→tool shortcut, no substrate self-authority,
 * and never bypasses the permission gate. Every backend call uses
 * `bridgeInvoke` so an unavailable command degrades gracefully (Req 20.4).
 *
 * Pure builder (`buildCapabilityRunEnvelope`) is exported for testing.
 */

import { eventBus } from "../stores/eventBus";
import {
  approvalStore,
  type ApprovalEnvelope,
  type ApprovalScope,
  type RiskLevel,
} from "../stores/approvalStore";
import { bridgeInvoke } from "./invoke";
import { mirrorApprovalPresentation } from "./approval";

/** The full scope ladder offered on a capability-run approval (Req 7.3). */
export const CAPABILITY_RUN_SCOPES: readonly ApprovalScope[] = [
  "once",
  "session",
  "workspace",
  "always",
] as const;

/** Stable approval id for a capability run (dedupes a re-emitted gate result). */
export function capabilityRunApprovalId(providerId: string, capabilityId: string): string {
  return `cap-run:${providerId}:${capabilityId}`;
}

/** The permission decision surfaced by the gate when a prompt is required. */
export interface CapabilityAuthDecision {
  kind: string;
  tier?: string;
  effects?: string[];
  risk?: string | null;
  reason?: string | null;
  grantId?: string | null;
}

/** Raw `CppExecuteResult` shape from the backend (snake/camel tolerant). */
interface RawExecuteResult {
  status?: string;
  decision?: {
    kind?: string;
    tier?: string;
    effects?: string[];
    risk?: string | null;
    reason?: string | null;
    grant_id?: string | null;
    grantId?: string | null;
  } | null;
  value?: unknown;
  reason?: string | null;
}

/**
 * Map the runtime's coarse risk label (`low`/`medium`/`high`) — or an elevated
 * flag — onto the unified risk ramp (Req 11.2). Elevated capabilities are at
 * least `yellow`; a high/critical label escalates to `red`.
 */
export function riskFromDecision(risk?: string | null, elevated?: boolean): RiskLevel {
  const lvl = (risk ?? "").toLowerCase();
  if (lvl.includes("black") || lvl.includes("critical")) return "black";
  if (lvl.includes("high") || lvl.includes("red")) return "red";
  if (lvl.includes("medium") || lvl.includes("moderate") || lvl.includes("yellow")) return "yellow";
  if (lvl === "low" || lvl.includes("green")) return elevated ? "yellow" : "green";
  return elevated ? "yellow" : "green";
}

/** Inputs to build a capability-run approval envelope. */
export interface CapabilityRunEnvelopeInput {
  providerId: string;
  capabilityId: string;
  /** Human name for the card headline (falls back to the capability id). */
  name?: string;
  /** Plain-language description / why (Req 11.2). */
  description?: string;
  /** Effect classes the runtime reported for this run (Req 11.2). */
  effects?: string[];
  /** Coarse risk label from the gate decision. */
  risk?: string | null;
  /** Whether the capability runs elevated / with broader effects. */
  elevated?: boolean;
  /** Opaque args to re-invoke `cpp_execute` with on approve. */
  args?: unknown;
  /** Optional CPP session/workspace scope keys. */
  sessionId?: string;
  workspaceId?: string;
  createdAtMs?: number;
}

/**
 * Build a unified `capability-run` approval envelope. Pure — no side effects —
 * so it is independently testable. Always offers the full scope ladder
 * (once/session/workspace/always, Req 7.3); deny is the Approval Center's
 * always-one-action escape and needs no scope.
 */
export function buildCapabilityRunEnvelope(input: CapabilityRunEnvelopeInput): ApprovalEnvelope {
  const risk = riskFromDecision(input.risk, input.elevated);
  const name = input.name?.trim() || input.capabilityId;
  return {
    id: capabilityRunApprovalId(input.providerId, input.capabilityId),
    source: "capability-run",
    title: `Run ${name}`,
    description:
      input.description?.trim() ||
      `KRIA wants to run ${name} from ${input.providerId}. Choose how long to allow it.`,
    risk,
    effects: input.effects && input.effects.length > 0 ? input.effects : undefined,
    // Elevated/high-risk runs are not casually reversible; high risk already
    // forces an explicit confirm before approval is staged (Req 11.3).
    irreversible: risk === "red" || risk === "black",
    scopeOptions: [...CAPABILITY_RUN_SCOPES],
    routing: {
      providerId: input.providerId,
      capabilityId: input.capabilityId,
      sessionId: input.sessionId,
      workspaceId: input.workspaceId,
      capabilityArgs: input.args ?? {},
    },
    payload: {
      providerId: input.providerId,
      capabilityId: input.capabilityId,
    },
    createdAtMs: input.createdAtMs ?? Date.now(),
  };
}

/** Outcome of a `runCapability` dispatch (before any approval is resolved). */
export type CapabilityRunOutcome =
  | { status: "ok"; value: unknown }
  | { status: "denied" | "declined"; reason?: string }
  | { status: "needs_approval" }
  | { status: "error"; message: string };

function normalizeDecision(d: RawExecuteResult["decision"]): CapabilityAuthDecision | null {
  if (!d) return null;
  return {
    kind: String(d.kind ?? "prompt"),
    tier: d.tier != null ? String(d.tier) : undefined,
    effects: Array.isArray(d.effects) ? d.effects.map(String) : [],
    risk: d.risk ?? null,
    reason: d.reason ?? null,
    grantId: d.grantId ?? d.grant_id ?? null,
  };
}

export interface RunCapabilityInput {
  providerId: string;
  capabilityId: string;
  args?: unknown;
  name?: string;
  description?: string;
  elevated?: boolean;
  sessionId?: string;
  workspaceId?: string;
}

/**
 * Dispatch a capability run through the runtime's permission gate. On
 * `needs_approval` a `capability-run` request is enqueued into the unified
 * Approval Center (Req 7.3); otherwise the honest gate result is returned.
 *
 * NOTE: this never executes the capability itself. The runtime's `cpp_execute`
 * owns authorization + execution; on approval the resolver re-invokes it.
 */
export async function runCapability(input: RunCapabilityInput): Promise<CapabilityRunOutcome> {
  eventBus.emit("capability:run-requested", {
    providerId: input.providerId,
    capabilityId: input.capabilityId,
  });

  const res = await bridgeInvoke<RawExecuteResult>("cpp_execute", {
    providerId: input.providerId,
    capabilityId: input.capabilityId,
    args: input.args ?? {},
    sessionId: input.sessionId ?? null,
    workspaceId: input.workspaceId ?? null,
  });

  if (!res.ok) {
    const message = res.message?.trim() ? res.message : "cpp_execute failed";
    emitResult(input, "error", message);
    return { status: "error", message };
  }

  const data = res.data ?? {};
  const status = String(data.status ?? "error");

  if (status === "needs_approval") {
    const decision = normalizeDecision(data.decision);
    const envelope = buildCapabilityRunEnvelope({
      providerId: input.providerId,
      capabilityId: input.capabilityId,
      name: input.name,
      description: input.description,
      effects: decision?.effects,
      risk: decision?.risk,
      elevated: input.elevated,
      args: input.args,
      sessionId: input.sessionId,
      workspaceId: input.workspaceId,
    });
    approvalStore.addFromEnvelope(envelope);
    mirrorApprovalPresentation(envelope);
    emitResult(input, "needs_approval");
    return { status: "needs_approval" };
  }

  if (status === "ok") {
    emitResult(input, "ok");
    return { status: "ok", value: data.value ?? null };
  }

  if (status === "denied" || status === "declined") {
    const reason = data.reason ?? undefined;
    emitResult(input, status, reason ?? undefined);
    return { status, reason: reason ?? undefined };
  }

  emitResult(input, "error", `Unexpected run status: ${status}`);
  return { status: "error", message: `Unexpected run status: ${status}` };
}

function emitResult(
  input: RunCapabilityInput,
  status: string,
  reason?: string,
): void {
  eventBus.emit("capability:run-result", {
    providerId: input.providerId,
    capabilityId: input.capabilityId,
    status,
    reason,
  });
}
