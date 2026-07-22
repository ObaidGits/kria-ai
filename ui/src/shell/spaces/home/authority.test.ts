/**
 * Guardrail test — AI vs Rules vs User Decision Authority Framework (design §31).
 *
 * Machine-checks the five authority principles that keep the homepage honest:
 *   P1. Explicit user actions ALWAYS win a conflict (User > Rules > AI).
 *   P2. Rules are DETERMINISTIC (same input → same output; no AI fuzzing).
 *   P3. AI outputs are STAGED, never auto-executed.
 *   P4. Only GREEN auto-acts, and a GREEN auto-act REPORTS.
 *   P5. AI never overrides navigation (nav source is never "ai").
 *
 * Unit tests pin the exact examples; property tests (fast-check) prove the
 * universal statements over the whole input space. Static scans reuse the
 * `scripts/guardrail-lint.mjs` detectors to prove the real Focus-engine
 * read-model modules never call `navigate`/`send`/`execute`.
 *
 * **Validates: Requirements 29.1, 29.2, 29.3**
 */
import { describe, it, expect } from "vitest";
import fc from "fast-check";

import {
  AUTHORITY_PRECEDENCE,
  DECISION_OWNER,
  domainsOwnedBy,
  resolveConflict,
  isNavAuthoritative,
  assertNavAuthoritative,
  NavigationAuthorityError,
  USER_OWNED_NAV_SOURCES,
  FORBIDDEN_NAV_SOURCES,
  aiOutputCommitMode,
  aiOutputIsAutoExecuted,
  isAutoActable,
  autoActReports,
  AUTO_ACTABLE_RISK,
  riskPresentationRule,
  type AuthorityActor,
  type AiOutputKind,
  type NavSource,
} from "./authority";
import type { RiskLevel } from "../../../stores/approvalStore";
import { resolvePermissionMode } from "./permissionUx";

// Standalone linter runs in Node; Vitest imports its pure detectors directly.
// @ts-expect-error Standalone ESM script has no generated declaration file.
import { findNavOverrides, findAiSendExecute } from "../../../../scripts/guardrail-lint.mjs";

// Raw source of the AI read-model (Focus engine) modules — scanned to prove they
// never override navigation nor auto-send/execute (design §31 enforcement point).
import homeFocusStoreSource from "../../../stores/homeFocusStore.ts?raw";
import homeGreetingStoreSource from "../../../stores/homeGreetingStore.ts?raw";
import relationshipEvolutionSource from "../../../stores/relationshipEvolution.ts?raw";

// ─── Arbitraries ──────────────────────────────────────────────────────────────

const RISKS: readonly RiskLevel[] = ["green", "yellow", "red", "black"];
const ACTORS: readonly AuthorityActor[] = ["user", "rules", "ai"];
const AI_OUTPUT_KINDS: readonly AiOutputKind[] = [
  "focus-subject",
  "chip-stage",
  "chip-route",
  "starter",
  "greeting",
  "learned-fact",
];
const NAV_SOURCES: readonly NavSource[] = [
  "user",
  "palette",
  "dock",
  "message-action",
  "chip-route",
  "ai",
  "focus-engine",
];

const riskArb = fc.constantFrom(...RISKS);
const actorArb = fc.constantFrom(...ACTORS);
const aiKindArb = fc.constantFrom(...AI_OUTPUT_KINDS);
const navSourceArb = fc.constantFrom(...NAV_SOURCES);
/** A non-empty conflict: a set of actors, at least one present. */
const claimsArb = fc
  .record({ user: fc.boolean(), rules: fc.boolean(), ai: fc.boolean() })
  .filter((c) => c.user || c.rules || c.ai);

// ─── P1: explicit user actions ALWAYS win (Req 29.1) ─────────────────────────

describe("Authority P1 — explicit user actions always win (Req 29.1)", () => {
  it("resolves User > Rules > AI in the canonical examples", () => {
    expect(resolveConflict({ user: true, rules: true, ai: true })).toBe("user");
    expect(resolveConflict({ rules: true, ai: true })).toBe("rules");
    expect(resolveConflict({ ai: true })).toBe("ai");
    expect(resolveConflict({})).toBeNull();
  });

  it("precedence table is exactly [user, rules, ai]", () => {
    expect([...AUTHORITY_PRECEDENCE]).toEqual(["user", "rules", "ai"]);
  });

  it("PROPERTY: any conflict that includes the user resolves to the user", () => {
    fc.assert(
      fc.property(claimsArb, (claims) => {
        if (claims.user) expect(resolveConflict(claims)).toBe("user");
      }),
    );
  });

  it("PROPERTY: the winner is always the highest-precedence present actor", () => {
    fc.assert(
      fc.property(claimsArb, (claims) => {
        const expected = AUTHORITY_PRECEDENCE.find((a) => claims[a]) ?? null;
        expect(resolveConflict(claims)).toBe(expected);
      }),
    );
  });

  it("PROPERTY: an AI claim never wins over a user or rules claim", () => {
    fc.assert(
      fc.property(claimsArb, (claims) => {
        if ((claims.user || claims.rules) && claims.ai) {
          expect(resolveConflict(claims)).not.toBe("ai");
        }
      }),
    );
  });
});

// ─── P2: rules are deterministic (Req 29.2) ──────────────────────────────────

describe("Authority P2 — rules are deterministic & auditable (Req 29.2)", () => {
  it("rule-owned domains map to the `rules` actor (auditable table)", () => {
    for (const domain of [
      "ranking-precedence",
      "interruptibility",
      "risk-classification",
      "layout",
      "dwell",
      "tier-degradation",
    ] as const) {
      expect(DECISION_OWNER[domain]).toBe("rules");
    }
    // The rule domains partition cleanly from user/ai domains.
    expect(domainsOwnedBy("rules").length).toBeGreaterThan(0);
    expect(domainsOwnedBy("user")).toContain("navigation");
    expect(domainsOwnedBy("ai")).toContain("chip");
  });

  it("PROPERTY: the risk-presentation rule is a pure function (same input → same output)", () => {
    fc.assert(
      fc.property(riskArb, (risk) => {
        // Deterministic across repeated calls and equal to the reused mapping.
        expect(riskPresentationRule(risk)).toBe(riskPresentationRule(risk));
        expect(riskPresentationRule(risk)).toBe(resolvePermissionMode(risk));
      }),
    );
  });

  it("PROPERTY: conflict resolution & commit-mode are deterministic", () => {
    fc.assert(
      fc.property(claimsArb, aiKindArb, (claims, kind) => {
        expect(resolveConflict(claims)).toBe(resolveConflict(claims));
        expect(aiOutputCommitMode(kind)).toBe(aiOutputCommitMode(kind));
      }),
    );
  });
});

// ─── P3: AI outputs are STAGED, never auto-executed (Req 29.3) ───────────────

describe("Authority P3 — AI outputs are staged, never auto-executed (Req 29.3)", () => {
  it("classifies each AI output kind by how it commits (none auto-execute)", () => {
    expect(aiOutputCommitMode("chip-stage")).toBe("staged");
    expect(aiOutputCommitMode("chip-route")).toBe("route");
    expect(aiOutputCommitMode("focus-subject")).toBe("informational");
    expect(aiOutputCommitMode("greeting")).toBe("informational");
    expect(aiOutputCommitMode("learned-fact")).toBe("informational");
    expect(aiOutputCommitMode("starter")).toBe("informational");
  });

  it("PROPERTY: no AI output kind is ever auto-executed", () => {
    fc.assert(
      fc.property(aiKindArb, (kind) => {
        expect(aiOutputIsAutoExecuted(kind)).toBe(false);
        expect(aiOutputCommitMode(kind)).not.toBe("auto-execute");
      }),
    );
  });

  it("AI-owned decision domains are suggestions only (never send)", () => {
    expect(domainsOwnedBy("ai").sort()).toEqual(
      ["chip", "focus-subject", "greeting", "learned-fact", "starter"].sort(),
    );
    // "send" is a USER domain — the AI never owns sending.
    expect(DECISION_OWNER.send).toBe("user");
  });
});

// ─── P4: GREEN auto-acts then reports (Req 29.3) ─────────────────────────────

describe("Authority P4 — only GREEN auto-acts, and it reports (Req 29.3)", () => {
  it("GREEN is the sole auto-actable tier and it presents as a report", () => {
    expect(AUTO_ACTABLE_RISK).toBe("green");
    expect(isAutoActable("green")).toBe(true);
    expect(autoActReports("green")).toBe(true);
    expect(resolvePermissionMode("green")).toBe("report");
  });

  it("PROPERTY: no non-GREEN tier is auto-actable, and only GREEN reports as an auto-act", () => {
    fc.assert(
      fc.property(riskArb, (risk) => {
        if (risk === "green") {
          expect(isAutoActable(risk)).toBe(true);
          expect(autoActReports(risk)).toBe(true);
        } else {
          expect(isAutoActable(risk)).toBe(false);
          expect(autoActReports(risk)).toBe(false);
        }
      }),
    );
  });
});

// ─── P5: AI never overrides navigation (Req 29.1) ────────────────────────────

describe("Authority P5 — AI never overrides navigation (Req 29.1)", () => {
  it("user-owned nav sources are authoritative; AI sources are forbidden", () => {
    for (const source of USER_OWNED_NAV_SOURCES) {
      expect(isNavAuthoritative(source)).toBe(true);
      expect(() => assertNavAuthoritative(source)).not.toThrow();
    }
    for (const source of FORBIDDEN_NAV_SOURCES) {
      expect(isNavAuthoritative(source)).toBe(false);
      expect(() => assertNavAuthoritative(source)).toThrow(NavigationAuthorityError);
    }
  });

  it("navigation is a USER decision domain", () => {
    expect(DECISION_OWNER.navigation).toBe("user");
  });

  it("PROPERTY: assertNavAuthoritative throws iff the source is an AI source", () => {
    fc.assert(
      fc.property(navSourceArb, (source) => {
        const forbidden = (FORBIDDEN_NAV_SOURCES as readonly string[]).includes(source);
        if (forbidden) {
          expect(isNavAuthoritative(source)).toBe(false);
          expect(() => assertNavAuthoritative(source)).toThrow(NavigationAuthorityError);
        } else {
          expect(isNavAuthoritative(source)).toBe(true);
          expect(() => assertNavAuthoritative(source)).not.toThrow();
        }
      }),
    );
  });
});

// ─── Static enforcement — the real Focus-engine read-model (design §31) ──────

describe("Authority static lint — the AI read-model never overrides nav / auto-sends", () => {
  it("detectors flag synthetic violations", () => {
    expect(findNavOverrides('navigate("memory");')).toHaveLength(1);
    expect(findAiSendExecute("converseStore.send(draft)")).toHaveLength(1);
    expect(findAiSendExecute("runTool({ id })")).toHaveLength(1);
  });

  it("detectors do not flag legitimate read-model code", () => {
    // Type-only Route import + a reactive read are fine.
    expect(findNavOverrides('import type { Route } from "../shell/router";')).toEqual([]);
    expect(findAiSendExecute("const s = converseStore.messages();")).toEqual([]);
  });

  it("homeFocusStore.ts calls no navigate and no send/execute (Req 29.1/29.3)", () => {
    expect(findNavOverrides(homeFocusStoreSource)).toEqual([]);
    expect(findAiSendExecute(homeFocusStoreSource)).toEqual([]);
  });

  it("homeGreetingStore.ts calls no navigate and no send/execute", () => {
    expect(findNavOverrides(homeGreetingStoreSource)).toEqual([]);
    expect(findAiSendExecute(homeGreetingStoreSource)).toEqual([]);
  });

  it("relationshipEvolution.ts calls no navigate and no send/execute", () => {
    expect(findNavOverrides(relationshipEvolutionSource)).toEqual([]);
    expect(findAiSendExecute(relationshipEvolutionSource)).toEqual([]);
  });
});
