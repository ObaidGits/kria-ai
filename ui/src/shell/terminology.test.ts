import { describe, it, expect } from "vitest";
import {
  TERMINOLOGY_MATRIX,
  REQUIRED_TERM_IDS,
  getTerm,
  isSpaceRouteTerm,
  type TermId,
  type TerminologyEntry,
} from "./terminology";
import { ALL_SPACES, isValidSpace } from "./router";

/**
 * Canonical terminology matrix guard (task 7.5; IU-08; UIE-M-016, UIE-M-017).
 *
 * design.md §12: "Threads, Tools, Skills, Integrations, and Lab are concepts or
 * nested surfaces—not top-level Spaces." Req 7.11: when Threads, Tools, Skills,
 * Integrations, or Lab are described, they must be identified as concepts or
 * surfaces rather than top-level Space_Routes.
 *
 * This LOCKS the single source of truth: exactly the nine required terms, each
 * with all four columns populated, and correct route-vs-concept status. Later
 * sub-tasks (7.6/7.7) read from this matrix, so drift here would ripple to
 * every navigation/empty/decision surface — CI must catch it.
 *
 * Validates: Requirements 7.3, 7.4, 7.5, 7.6, 7.11
 */

// The authoritative route-vs-concept classification per design.md §12.
const EXPECTED_SPACE_ROUTES: readonly TermId[] = ["machines", "observatory", "memory"];
const EXPECTED_CONCEPTS: readonly TermId[] = [
  "threads",
  "tools",
  "skills",
  "integrations",
  "temporary-threads",
  "lab-mode",
];

const FOUR_COLUMNS = ["outcome", "persistence", "authority"] as const;

describe("TERMINOLOGY_MATRIX coverage (Req 7.3–7.6)", () => {
  it("covers exactly the nine required terms, once each, in matrix order", () => {
    const ids = TERMINOLOGY_MATRIX.map((t) => t.id);
    // No duplicates.
    expect(new Set(ids).size).toBe(ids.length);
    // Exactly the required set (order-independent).
    expect([...ids].sort()).toEqual([...REQUIRED_TERM_IDS].sort());
    // REQUIRED_TERM_IDS itself lists the nine terms without duplication.
    expect(REQUIRED_TERM_IDS).toHaveLength(9);
    expect(new Set(REQUIRED_TERM_IDS).size).toBe(9);
  });

  it("populates all four distinguishing columns for every term", () => {
    for (const entry of TERMINOLOGY_MATRIX) {
      // Status is one of the two allowed values (the route/concept column).
      expect(["space-route", "concept"]).toContain(entry.status);
      // The three textual columns are present and non-empty.
      for (const col of FOUR_COLUMNS) {
        const value = entry[col as keyof TerminologyEntry] as string;
        expect(typeof value, `${entry.id}.${col} must be a string`).toBe("string");
        expect(value.trim().length, `${entry.id}.${col} must be non-empty`).toBeGreaterThan(0);
      }
      // Label is present and human-readable.
      expect(entry.label.trim().length).toBeGreaterThan(0);
    }
  });

  it("anchors every term to a valid canonical Space", () => {
    for (const entry of TERMINOLOGY_MATRIX) {
      expect(isValidSpace(entry.space), `${entry.id}.space must be a real Space`).toBe(true);
      expect(ALL_SPACES).toContain(entry.space);
    }
  });
});

describe("route-vs-concept status (design.md §12, Req 7.11)", () => {
  it("marks Machines, Observatory, and Memory as top-level Space_Routes", () => {
    for (const id of EXPECTED_SPACE_ROUTES) {
      expect(getTerm(id).status, `${id} must be a space-route`).toBe("space-route");
      expect(isSpaceRouteTerm(id)).toBe(true);
    }
  });

  it("space-route terms map to their own Space id", () => {
    expect(getTerm("machines").space).toBe("machines");
    expect(getTerm("observatory").space).toBe("observatory");
    expect(getTerm("memory").space).toBe("memory");
  });

  it("marks Threads, Tools, Skills, Integrations, Temporary threads, and Lab mode as concepts, NOT top-level routes", () => {
    for (const id of EXPECTED_CONCEPTS) {
      expect(getTerm(id).status, `${id} must be a concept/surface`).toBe("concept");
      expect(isSpaceRouteTerm(id), `${id} must NOT be a top-level Space_Route`).toBe(false);
    }
  });

  it("nests each concept within the correct home Space (Threads/Temporary/Lab → Converse; Tools/Skills/Integrations → Capabilities)", () => {
    expect(getTerm("threads").space).toBe("converse");
    expect(getTerm("temporary-threads").space).toBe("converse");
    expect(getTerm("lab-mode").space).toBe("converse");
    expect(getTerm("tools").space).toBe("capabilities");
    expect(getTerm("skills").space).toBe("capabilities");
    expect(getTerm("integrations").space).toBe("capabilities");
  });

  it("classifies every term as either space-route or concept, with no other status", () => {
    const routes = TERMINOLOGY_MATRIX.filter((t) => t.status === "space-route").map((t) => t.id);
    const concepts = TERMINOLOGY_MATRIX.filter((t) => t.status === "concept").map((t) => t.id);
    expect([...routes].sort()).toEqual([...EXPECTED_SPACE_ROUTES].sort());
    expect([...concepts].sort()).toEqual([...EXPECTED_CONCEPTS].sort());
    expect(routes.length + concepts.length).toBe(TERMINOLOGY_MATRIX.length);
  });
});

describe("getTerm lookup", () => {
  it("returns the matching entry for every required id", () => {
    for (const id of REQUIRED_TERM_IDS) {
      expect(getTerm(id).id).toBe(id);
    }
  });

  it("throws on an unknown id", () => {
    expect(() => getTerm("nonexistent" as TermId)).toThrow(/Unknown terminology id/);
  });
});
