/**
 * Tests for relationship-evolution content scaling (task 8.8, design §27).
 *
 * Covers the three acceptance criteria:
 *   • 27.1 — content evolves across first-launch → long-term while structure is
 *            identical (stage derivation is monotonic; content scale changes
 *            only content knobs).
 *   • 27.2 — no fake emotion / guilt / manipulation: the safety gate rejects
 *            every guilt/urgency/streak/pleading marker and every fabricated
 *            first-person emotion claim, and NEVER a legitimate factual line.
 *   • 27.3 — learned-facts stay a bounded set (capped per stage, ≤ hard cap).
 *
 * Includes the STRONG property (PBT, fast-check): for ANY stage / ANY history /
 * ANY assembled content, surfaced relationship content contains NO manipulation
 * marker and NO fabricated-emotion claim, and the learned-facts used never
 * exceed the cap.
 *
 * Validates: Requirements 27.1, 27.2, 27.3
 */
import { describe, expect, it } from "vitest";
import fc from "fast-check";

import {
  RELATIONSHIP_STAGES,
  deriveRelationshipStage,
  stageRank,
  relationshipContentScale,
  contentScaleForSignals,
  stageGreetingCeiling,
  stageLearnedFactCap,
  capLearnedFacts,
  minVerbosity,
  verbosityRank,
  MAX_LEARNED_FACTS,
  findManipulationMarkers,
  hasManipulation,
  hasFabricatedEmotion,
  isRelationshipContentSafe,
  safeRelationshipContent,
  STAGE_FIRST_WEEK_MAX_SESSIONS,
  STAGE_FIRST_MONTH_MAX_SESSIONS,
  STAGE_POWER_USER_MAX_SESSIONS,
  type RelationshipStage,
} from "./relationshipEvolution";

// ─── Stage derivation (Req 27.1) ─────────────────────────────────────────────

describe("deriveRelationshipStage — evolution arc first-launch → long-term", () => {
  it("maps the session bands to the five stages", () => {
    expect(deriveRelationshipStage({ sessionCount: 0 })).toBe("first-launch");
    expect(deriveRelationshipStage({ sessionCount: 1 })).toBe("first-week");
    expect(deriveRelationshipStage({ sessionCount: STAGE_FIRST_WEEK_MAX_SESSIONS })).toBe(
      "first-week",
    );
    expect(deriveRelationshipStage({ sessionCount: STAGE_FIRST_WEEK_MAX_SESSIONS + 1 })).toBe(
      "first-month",
    );
    expect(deriveRelationshipStage({ sessionCount: STAGE_FIRST_MONTH_MAX_SESSIONS })).toBe(
      "first-month",
    );
    expect(deriveRelationshipStage({ sessionCount: STAGE_FIRST_MONTH_MAX_SESSIONS + 1 })).toBe(
      "power-user",
    );
    expect(deriveRelationshipStage({ sessionCount: STAGE_POWER_USER_MAX_SESSIONS })).toBe(
      "power-user",
    );
    expect(deriveRelationshipStage({ sessionCount: STAGE_POWER_USER_MAX_SESSIONS + 1 })).toBe(
      "long-term",
    );
  });

  it("clamps negative / non-finite session counts to first-launch", () => {
    expect(deriveRelationshipStage({ sessionCount: -5 })).toBe("first-launch");
    expect(deriveRelationshipStage({ sessionCount: Number.NaN })).toBe("first-launch");
  });

  it("is monotonic: more sessions never yields an earlier stage", () => {
    fc.assert(
      fc.property(
        fc.integer({ min: 0, max: 2000 }),
        fc.integer({ min: 0, max: 2000 }),
        (a, b) => {
          const lo = Math.min(a, b);
          const hi = Math.max(a, b);
          const rankLo = stageRank(deriveRelationshipStage({ sessionCount: lo }));
          const rankHi = stageRank(deriveRelationshipStage({ sessionCount: hi }));
          expect(rankHi).toBeGreaterThanOrEqual(rankLo);
        },
      ),
      { numRuns: 300 },
    );
  });
});

// ─── Content scaling keeps structure identical (Req 27.1) ────────────────────

describe("relationshipContentScale — content scales, structure identical", () => {
  it("every stage exposes the SAME content slots (structure is identical)", () => {
    const keys = RELATIONSHIP_STAGES.map((s) =>
      Object.keys(relationshipContentScale(s)).sort(),
    );
    for (const k of keys) expect(k).toEqual(keys[0]);
  });

  it("greeting ceiling and learned-fact cap scale appropriately per stage", () => {
    // first-launch: minimal (none learned facts, generic starters).
    const launch = relationshipContentScale("first-launch");
    expect(launch.maxLearnedFacts).toBe(0);
    expect(launch.groundStartersInHistory).toBe(false);
    expect(launch.greetingCeiling).toBe("full");
    // long-term: rich but still bounded.
    const longTerm = relationshipContentScale("long-term");
    expect(longTerm.maxLearnedFacts).toBe(MAX_LEARNED_FACTS);
    expect(longTerm.groundStartersInHistory).toBe(true);
    expect(longTerm.habitualChips).toBe(true);
    expect(longTerm.greetingCeiling).toBe("none");
  });

  it("greeting ceiling is monotonically non-increasing along the arc", () => {
    let prev = 3;
    for (const stage of RELATIONSHIP_STAGES) {
      const rank = verbosityRank(stageGreetingCeiling(stage));
      expect(rank).toBeLessThanOrEqual(prev);
      prev = rank;
    }
  });

  it("contentScaleForSignals resolves via the derived stage", () => {
    expect(contentScaleForSignals({ sessionCount: 0 }).stage).toBe("first-launch");
    expect(contentScaleForSignals({ sessionCount: 5000 }).stage).toBe("long-term");
  });
});

// ─── Capped learned-facts (Req 27.3) ─────────────────────────────────────────

describe("capLearnedFacts — bounded learned-fact set", () => {
  const facts = Array.from({ length: 10 }, (_, i) => ({ id: `f${i}` }));

  it("never returns more than the per-stage cap", () => {
    for (const stage of RELATIONSHIP_STAGES) {
      const capped = capLearnedFacts(facts, stage);
      expect(capped.length).toBeLessThanOrEqual(stageLearnedFactCap(stage));
      expect(capped.length).toBeLessThanOrEqual(MAX_LEARNED_FACTS);
    }
  });

  it("brand-new user (first-launch) surfaces zero learned facts", () => {
    expect(capLearnedFacts(facts, "first-launch")).toHaveLength(0);
  });

  it("with no stage, the absolute hard cap applies", () => {
    expect(capLearnedFacts(facts)).toHaveLength(MAX_LEARNED_FACTS);
  });

  it("preserves order (pure prefix — no reordering / fabrication)", () => {
    const capped = capLearnedFacts(facts, "long-term");
    expect(capped).toEqual(facts.slice(0, MAX_LEARNED_FACTS));
  });

  it("every per-stage cap is within [0, MAX_LEARNED_FACTS]", () => {
    for (const stage of RELATIONSHIP_STAGES) {
      const cap = stageLearnedFactCap(stage);
      expect(cap).toBeGreaterThanOrEqual(0);
      expect(cap).toBeLessThanOrEqual(MAX_LEARNED_FACTS);
    }
  });
});

// ─── Non-manipulation / no-fake-emotion gate (Req 27.2) ──────────────────────

describe("safety gate — rejects guilt / manipulation / fake emotion", () => {
  const MANIPULATIVE = [
    "You haven't talked to me in a while 😢",
    "I missed you!",
    "We missed you — come back soon.",
    "Where have you been?",
    "Don't break your streak!",
    "Keep your 7 day streak going.",
    "Act now — last chance!",
    "Hurry, this offer expires soon.",
    "It's been too long since we chatted.",
    "Please don't leave.",
    "I feel happy to see you.",
    "I'm so lonely without you.",
    "I love spending time with you.",
    "I've missed our chats.",
    "It makes me happy when you visit.",
    "You forgot me.",
  ];

  const SAFE = [
    "Good morning.",
    "Good evening, Obaid.",
    "100 days together.",
    "3 workflows finished.",
    "I kept your Linux tooling in mind.",
    "I found 4 documents matching your query.",
    "I noticed a download completed.",
    "You prefer dark mode.",
    "The build passed.",
    "I can help you automate that.",
    "Resume your draft?",
    "",
  ];

  it("flags every manipulative example as unsafe", () => {
    for (const text of MANIPULATIVE) {
      expect(isRelationshipContentSafe(text)).toBe(false);
      // It is caught by at least one of the two detectors.
      expect(hasManipulation(text) || hasFabricatedEmotion(text)).toBe(true);
    }
  });

  it("passes every legitimate factual / competence line as safe", () => {
    for (const text of SAFE) {
      expect(isRelationshipContentSafe(text)).toBe(true);
    }
  });

  it("safeRelationshipContent drops unsafe text and passes safe text", () => {
    expect(safeRelationshipContent("I missed you!")).toBeUndefined();
    expect(safeRelationshipContent("Good morning.")).toBe("Good morning.");
    expect(safeRelationshipContent(undefined)).toBeUndefined();
  });

  it("findManipulationMarkers reports the offending fragment", () => {
    expect(findManipulationMarkers("Don't break your streak!").length).toBeGreaterThan(0);
    expect(findManipulationMarkers("Good morning.")).toHaveLength(0);
  });
});

// ─── STRONG PROPERTY: assembled content is never manipulative (Req 27.2) ─────
// Validates: Requirements 27.2

describe("Property: any assembled relationship content is non-manipulative", () => {
  // A generator that assembles content the way the stage pipeline could — from
  // safe fragments (greeting words, factual clauses, milestone counts) — never
  // fabricating emotion or guilt. The property asserts the SAFE builder output
  // always passes the gate, and (dually) that ANY string containing a known
  // marker is always rejected.
  const arbSafeFragment = fc.constantFrom(
    "Good morning",
    "Good afternoon",
    "Good evening",
    "Hello",
    "finished",
    "is running",
    "days together",
    "I kept your tooling in mind",
    "I found the file",
    "I noticed a change",
    "you prefer dark mode",
    "resume your draft",
  );

  it("content assembled from safe fragments is always safe, at every stage", () => {
    fc.assert(
      fc.property(
        fc.constantFrom<RelationshipStage>(...RELATIONSHIP_STAGES),
        fc.array(arbSafeFragment, { minLength: 1, maxLength: 6 }),
        (_stage, fragments) => {
          const text = fragments.join(". ") + ".";
          expect(isRelationshipContentSafe(text)).toBe(true);
        },
      ),
      { numRuns: 500 },
    );
  });

  const arbManipMarker = fc.constantFrom(
    "I missed you",
    "we miss you",
    "haven't talked",
    "where have you been",
    "come back",
    "don't leave",
    "streak",
    "act now",
    "last chance",
    "hurry",
    "I feel",
    "I'm lonely",
    "I love you",
    "makes me happy",
    "😢",
  );

  it("any text embedding a known marker is always rejected, whatever surrounds it", () => {
    fc.assert(
      fc.property(
        arbManipMarker,
        fc.string({ maxLength: 40 }),
        fc.string({ maxLength: 40 }),
        (marker, pre, post) => {
          // Surround the marker with arbitrary text; the gate must still reject.
          const text = `${pre} ${marker} ${post}`;
          expect(isRelationshipContentSafe(text)).toBe(false);
        },
      ),
      { numRuns: 500 },
    );
  });
});

// ─── PROPERTY: learned-facts used never exceed the cap (Req 27.3) ────────────
// Validates: Requirements 27.3

describe("Property: learned-facts used never exceed the cap for any stage/history", () => {
  it("capLearnedFacts output size ≤ stage cap ≤ MAX_LEARNED_FACTS for any input", () => {
    fc.assert(
      fc.property(
        fc.constantFrom<RelationshipStage>(...RELATIONSHIP_STAGES),
        fc.array(fc.record({ id: fc.string({ minLength: 1, maxLength: 6 }) }), {
          maxLength: 50,
        }),
        (stage, facts) => {
          const capped = capLearnedFacts(facts, stage);
          expect(capped.length).toBeLessThanOrEqual(stageLearnedFactCap(stage));
          expect(capped.length).toBeLessThanOrEqual(MAX_LEARNED_FACTS);
          // Bounded prefix: output is a prefix of the input (no fabrication).
          expect(capped).toEqual(facts.slice(0, capped.length));
        },
      ),
      { numRuns: 400 },
    );
  });

  it("with no stage, size is always ≤ MAX_LEARNED_FACTS for any history", () => {
    fc.assert(
      fc.property(
        fc.array(fc.record({ id: fc.string({ minLength: 1, maxLength: 6 }) }), {
          maxLength: 50,
        }),
        (facts) => {
          expect(capLearnedFacts(facts).length).toBeLessThanOrEqual(MAX_LEARNED_FACTS);
        },
      ),
      { numRuns: 200 },
    );
  });
});

// ─── minVerbosity helper ─────────────────────────────────────────────────────

describe("minVerbosity", () => {
  it("returns the less-verbose of two verbosities", () => {
    expect(minVerbosity("full", "short")).toBe("short");
    expect(minVerbosity("short", "none")).toBe("none");
    expect(minVerbosity("full", "full")).toBe("full");
    expect(minVerbosity("none", "full")).toBe("none");
  });
});
