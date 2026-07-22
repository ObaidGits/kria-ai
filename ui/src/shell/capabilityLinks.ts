/**
 * Capability / context FACT LINKS — the single, shared, read-only navigation
 * helper that maps a surfaced Task-10 fact (a `CapabilityFactId`, plus an
 * optional source-owned entity id) to the correct EXISTING owner surface using
 * the `capabilityFieldMap` `detailDestination` table. Task 10.5 (IU-07;
 * UIE-H-011, UIE-H-012, UIE-M-019).
 *
 * WHY THIS EXISTS: every Task-10 surface (Homepage/current-work summary, the
 * ContextRail, grounded starters in 10.6, any summary/rail reference) must
 * deep-link a fact to its ONE authoritative home consistently — without any
 * surface hardcoding a destination, and without inventing a route/dashboard.
 * This module is that dispatch seam.
 *
 * STRICTLY READ-ONLY / DISPATCH-ONLY (design §20.1, 10.11 constraint):
 *   • Resolution (`resolveFactLink`) is PURE — it reads the static
 *     `detailDestination` map and returns a plain descriptor. No side effects.
 *   • Activation (`activateFactLink` / `openFactDetail`) performs ONLY
 *     `navigate(...)` (existing typed router) or `shellStore.openInspector(...)`
 *     (the ONE shared, non-stacking Inspector on a REGISTERED type). It NEVER
 *     runs a tool, launches work, grants an approval, sends a draft, mutates a
 *     runtime/automation/approval store, or issues a backend request.
 *   • It adds NO new route, Space, segment, Inspector type, or dashboard — every
 *     destination is drawn from `detailDestination`, whose `space` is one of the
 *     existing `Space`s and whose `inspectorType` is one of the registered
 *     Inspector types.
 *
 * NO FABRICATION (UIE-H-011 regression): an Inspector link is produced ONLY when
 * an authoritative, source-owned entity id is provided. A surface that has no id
 * (e.g. a memory rail item that carries none) resolves to `null` and MUST omit
 * the control — the helper never invents an id or a broken destination.
 *
 * Requirements: 8.6 (one-fact-one-home), 9.3 (Focus_Return_Owner), 13.3; design
 * §8.6, §20.1, §20.3; UIE-H-011, UIE-H-012, UIE-M-019.
 */
import { navigate, type Space } from "./router";
import { shellStore, type OpenInspectorOptions } from "../stores/shellStore";
import {
  getCapabilityFact,
  type CapabilityFactId,
  type FactDetailDestination,
} from "../stores/capabilityFieldMap";
import { nonEmpty } from "../stores/currentWorkSummary";

/** A resolved link is either pure navigation or an Inspector open. */
export type FactLinkMode = "navigate" | "inspector";

/** Registered shared-Inspector target type (mirrors capabilityFieldMap). */
export type InspectorType = NonNullable<FactDetailDestination["inspectorType"]>;

/**
 * A resolved, activatable fact link. All fields are drawn from the fact's
 * `detailDestination` (existing surfaces only) plus the caller-supplied,
 * source-owned `entityId`. Contains no behaviour — activation is a separate,
 * explicit dispatch step so resolution stays pure and testable.
 */
export interface ResolvedFactLink {
  readonly factId: CapabilityFactId;
  readonly mode: FactLinkMode;
  /** Owning Space (existing). Always present — every fact has a Space home. */
  readonly space: Space;
  /** Existing Space segment, when the destination is segment-scoped. */
  readonly segment?: string;
  /** Registered Inspector type — present only for `mode === "inspector"`. */
  readonly inspectorType?: InspectorType;
  /** Source-owned authoritative entity id — never fabricated. */
  readonly entityId?: string;
  /** Concise, accessible destination phrase, e.g. "Open in Memory". */
  readonly destinationLabel: string;
}

export interface FactLinkRequest {
  /**
   * Source-owned authoritative id (memory id, workflow id, capability id, …).
   * Passed verbatim from the owning store — NEVER invented by the consumer. A
   * blank/absent id means "no entity", and (with `inspectorOnly`) yields no link.
   */
  readonly entityId?: string | null;
  /**
   * When true, resolution returns `null` unless it can produce an Inspector
   * link (i.e. the fact has a registered Inspector type AND an authoritative
   * `entityId` was supplied). Used where navigating to the Space would be
   * meaningless — e.g. a memory rail item while already in Converse: without a
   * memory id there is simply no link, so the control is omitted (no fabrication).
   */
  readonly inspectorOnly?: boolean;
}

/** Human labels for the existing Spaces (accessible destination phrasing). */
const SPACE_LABEL: Readonly<Record<Space, string>> = {
  converse: "Converse",
  memory: "Memory",
  automations: "Automations",
  capabilities: "Capabilities",
  machines: "Machines",
  observatory: "Observatory",
  settings: "Settings",
};

/**
 * Human labels for the registered Inspector types — an Inspector link opens the
 * shared Inspector, so its accessible phrase names the Inspector, not the Space
 * the item happens to live in (a memory rail item lives in Converse but links to
 * the Memory Inspector).
 */
const INSPECTOR_LABEL: Readonly<Record<InspectorType, string>> = {
  memory: "Memory",
  capability: "Capabilities",
  "automation-node": "Automations",
  device: "Machines",
  observatory: "Observatory",
};

/**
 * Resolve a fact (+ optional source-owned entity id) to an activatable link
 * against its EXISTING owner surface. Pure: reads only the static
 * `detailDestination` map. Returns `null` when no link should be offered
 * (see `inspectorOnly` / no-fabrication).
 *
 * Rules:
 *   1. Inspector link — produced when the fact's `detailDestination` has a
 *      registered `inspectorType` AND an authoritative `entityId` is supplied.
 *      (The Inspector needs a concrete target; without an id we do not open it.)
 *   2. Navigate link — otherwise route to the owning `space` (+ existing
 *      `segment`, and the `entityId` when present, per the router grammar).
 *   3. `inspectorOnly` — when set, only rule (1) applies; a fact without a
 *      registered Inspector type or without an id resolves to `null`.
 */
export function resolveFactLink(
  factId: CapabilityFactId,
  request: FactLinkRequest = {},
): ResolvedFactLink | null {
  const dest = getCapabilityFact(factId).detailDestination;
  const rawId = request.entityId ?? undefined;
  const entityId = nonEmpty(rawId) ? rawId : undefined;

  // Rule 1 — Inspector link (registered type + authoritative id, no fabrication).
  if (dest.inspectorType && entityId) {
    return {
      factId,
      mode: "inspector",
      space: dest.space,
      ...(dest.segment ? { segment: dest.segment } : {}),
      inspectorType: dest.inspectorType,
      entityId,
      destinationLabel: `Open in ${INSPECTOR_LABEL[dest.inspectorType]}`,
    };
  }

  // `inspectorOnly` callers get no link when rule 1 could not fire.
  if (request.inspectorOnly) return null;

  // Rule 2 — navigate to the existing owning Space (+ segment / entity).
  return {
    factId,
    mode: "navigate",
    space: dest.space,
    ...(dest.segment ? { segment: dest.segment } : {}),
    ...(entityId ? { entityId } : {}),
    destinationLabel: `Open in ${SPACE_LABEL[dest.space]}`,
  };
}

/**
 * Activate a resolved link. Dispatch-only: opens the ONE shared Inspector on a
 * registered type (with the §20.3 Focus_Return_Owner from `focus`), or routes
 * via the existing typed router. Performs no other effect.
 */
export function activateFactLink(
  link: ResolvedFactLink,
  focus?: OpenInspectorOptions,
): void {
  if (link.mode === "inspector" && link.inspectorType && link.entityId) {
    shellStore.openInspector(link.inspectorType, link.entityId, undefined, focus);
    return;
  }
  navigate(link.space, link.segment, link.entityId);
}

/**
 * Convenience: resolve + activate in one call. Returns `true` when a link was
 * offered and activated, `false` when the fact yielded no link (omit the
 * control). Consumers that need to conditionally render a control should use
 * `resolveFactLink` directly and only render when it is non-null.
 */
export function openFactDetail(
  factId: CapabilityFactId,
  request: FactLinkRequest = {},
  focus?: OpenInspectorOptions,
): boolean {
  const link = resolveFactLink(factId, request);
  if (!link) return false;
  activateFactLink(link, focus);
  return true;
}
