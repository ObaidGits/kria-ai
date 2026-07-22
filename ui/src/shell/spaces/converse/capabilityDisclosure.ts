/**
 * capabilityDisclosure — READ-ONLY capability disclosure for the Core-forward
 * empty state, GROUNDED in existing global enabled/available capability state
 * (task 10.6; IU-07, UIE-M-019, Req 6.6 / 8.4).
 *
 * The Cold Start / Intentional New Thread empty state may surface a concise,
 * PURELY INFORMATIONAL "what KRIA can do" cue for the F6 (tools/MCP registry)
 * and F7 (OpenClaw skills) capability facts. This module resolves those cues by:
 *
 *   1. reading the AUTHORITATIVE global `capabilityStore` state (F6:
 *      `capabilities()` + `mcpServers()`; F7: `skills()` + `openClawSettings()`),
 *   2. applying the SAME pure `capabilityFieldMap` F6/F7 `omissionRule` every
 *      other Task-10 surface uses (`show` / `omit` / `unavailable`), so a
 *      not-loaded registry is OMITTED and an offline OpenClaw runtime is shown
 *      truthfully as `unavailable` — never fabricated as ready, and
 *   3. attaching the EXISTING read-only deep-link (`resolveFactLink`, task 10.5)
 *      to the fact's one authoritative home (Capabilities → Tools / Skills, or
 *      the `capability` Inspector when an id is supplied).
 *
 * ── M5: global enabled/installed ONLY, NO per-turn availability set ───────────
 * There is no "tools available to this turn" concept (must-omit M5). Grounding
 * reads the GLOBAL `capabilities()` / `skills()` registries — never a synthesized
 * turn-scoped set. Reading these already-materialised signals dispatches no load,
 * tool, or side effect (Req 8.4 "no inference").
 *
 * ── STRICTLY READ-ONLY DISCLOSURE (10.11 constraint) ─────────────────────────
 * A disclosure is informational + may deep-link via the shared read-only
 * `capabilityLinks` helper. Activating it performs ONLY `navigate` /
 * `openInspector` (dispatch-only). It NEVER invokes a tool, launches work,
 * grants/stages an approval, sends the composer draft, mutates a runtime/
 * automation/approval store, or bypasses staged review. It adds no new route,
 * Space, or Inspector type — every destination comes from `detailDestination`.
 *
 * Requirements: 6.6, 8.4, 8.6; design §8.6, §20.1; UIE-M-019.
 */
import { capabilityStore } from "../../../stores/capabilityStore";
import {
  evaluateOmission,
  getCapabilityFact,
  type CapabilityFactId,
  type OmissionOutcome,
} from "../../../stores/capabilityFieldMap";
import { openFactDetail, resolveFactLink, type ResolvedFactLink } from "../../capabilityLinks";

/**
 * The capability facts this disclosure grounds. F6 (tools/MCP registry) and F7
 * (OpenClaw skills) are the two `available`-kind facts the empty state cues.
 * Both are read from GLOBAL enabled/installed state (M5: no per-turn set).
 */
export const DISCLOSURE_FACT_IDS = ["F6", "F7"] as const satisfies readonly CapabilityFactId[];
export type DisclosureFactId = (typeof DISCLOSURE_FACT_IDS)[number];

/** A resolved, read-only capability cue. `omit` is never emitted (filtered). */
export interface CapabilityDisclosure {
  readonly factId: DisclosureFactId;
  /** Bounded label from the field map (e.g. "Tools", "Skills"). */
  readonly label: string;
  /** `show` (present) or `unavailable` (optional service offline). Never `omit`. */
  readonly outcome: Exclude<OmissionOutcome, "omit">;
  /**
   * The EXISTING read-only deep-link to this fact's owner surface (task 10.5).
   * Present for both `show` and `unavailable` so the user can inspect the
   * capability's home; activation is navigate/openInspector only.
   */
  readonly link: ResolvedFactLink | null;
}

/**
 * F6 authoritative value: the GLOBAL tools registry — native active tools plus
 * MCP servers. Matches the F6 `sourceAccessor`
 * (`capabilityStore.capabilities / mcpServers`). Read-only.
 */
function toolsRegistryValue(): readonly unknown[] {
  return [
    ...capabilityStore.capabilities().filter((c) => c.status === "active"),
    ...capabilityStore.mcpServers(),
  ];
}

/**
 * F7 authoritative value: the OpenClaw runtime settings + the count of
 * installed AND enabled skills. Matches the F7 `sourceAccessor`
 * (`capabilityStore.skills / openClawSettings`). Read-only.
 */
function openClawValue(): { settings: { runtimeActive?: boolean } | null; skillCount: number } {
  return {
    settings: capabilityStore.openClawSettings(),
    skillCount: capabilityStore.skills().filter((s) => s.installed && s.enabled).length,
  };
}

/** Read-only value accessor per fact — global registries only (M5). */
const FACT_VALUE: Readonly<Record<DisclosureFactId, () => unknown>> = {
  F6: toolsRegistryValue,
  F7: openClawValue,
};

/**
 * Resolve the read-only capability cues for the empty state, grounded in global
 * enabled/available state via the field-map F6/F7 omission rules. `omit`
 * outcomes are dropped (never surfaced); `unavailable` is kept and rendered
 * truthfully by the caller (never as ready). Pure read — no side effects.
 */
export function capabilityDisclosures(): CapabilityDisclosure[] {
  const out: CapabilityDisclosure[] = [];
  for (const factId of DISCLOSURE_FACT_IDS) {
    // NOT-LOADED vs OFFLINE (task 10.6): the field-map F7 rule maps `settings ===
    // null` to "unavailable", but a null OpenClaw settings signal means the
    // optional service has NOT LOADED yet — not that it is offline. Treat
    // not-loaded as OMIT (nothing to disclose) and let the field-map rule report
    // "unavailable" only once settings are present and the runtime is inactive.
    // F6's empty registry already maps to omit, so no guard is needed there.
    if (factId === "F7" && capabilityStore.openClawSettings() === null) continue;

    const descriptor = getCapabilityFact(factId);
    let outcome: OmissionOutcome;
    try {
      outcome = evaluateOmission(descriptor, FACT_VALUE[factId]());
    } catch {
      // A momentarily-unreadable signal counts as absent → omit (never fabricate).
      outcome = "omit";
    }
    if (outcome === "omit") continue;
    out.push({
      factId,
      label: descriptor.displayLabel,
      outcome,
      link: resolveFactLink(factId),
    });
  }
  return out;
}

/**
 * Activate a capability disclosure's read-only deep-link. Dispatch-only: reuses
 * the shared `openFactDetail` (navigate / openInspector). Performs NO tool
 * invocation, work launch, approval, draft send, or staged-review bypass.
 * Returns `true` when a link was offered and activated.
 */
export function openCapabilityDisclosure(factId: DisclosureFactId): boolean {
  return openFactDetail(factId);
}
