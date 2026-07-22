import { describe, it, expect } from "vitest";
import {
  CAPABILITY_FIELD_MAP,
  ALL_CAPABILITY_FACT_IDS,
  MUST_OMIT_FACTS,
  MUST_OMIT_FACT_IDS,
  NEVER_SURFACE,
  getCapabilityFact,
  evaluateOmission,
  type CapabilityFactId,
  type OmissionOutcome,
} from "./capabilityFieldMap";
import { ALL_SPACES } from "../shell/router";

/**
 * Task 10.2 — data-driven verification of the read-only capability field map.
 * Asserts completeness (F1–F12), omission-rule behavior for present/absent/
 * unknown inputs, the G7 available-not-used classification, EXISTING detail
 * destinations only, and the §2 must-omit encoding.
 *
 * Validates: Requirements 8.1, 8.4, 8.5; UIE-H-002, UIE-M-011, UIE-M-018.
 */

const KNOWN_INSPECTOR_TYPES = new Set([
  "memory",
  "capability",
  "automation-node",
  "device",
  "observatory",
]);

describe("capabilityFieldMap — completeness (F1–F12)", () => {
  it("defines all twelve inventory facts exactly once", () => {
    expect(ALL_CAPABILITY_FACT_IDS).toHaveLength(12);
    expect(new Set(ALL_CAPABILITY_FACT_IDS).size).toBe(12);
    expect(Object.keys(CAPABILITY_FIELD_MAP).sort()).toEqual(
      [...ALL_CAPABILITY_FACT_IDS].sort(),
    );
  });

  it("each descriptor's id matches its map key and carries required fields", () => {
    for (const id of ALL_CAPABILITY_FACT_IDS) {
      const d = CAPABILITY_FIELD_MAP[id];
      expect(d.id).toBe(id);
      expect(d.sourceAccessor.length).toBeGreaterThan(0);
      expect(d.owner.length).toBeGreaterThan(0);
      expect(d.ownerSurface.length).toBeGreaterThan(0);
      expect(d.displayLabel.length).toBeGreaterThan(0);
      expect(typeof d.omissionRule).toBe("function");
    }
  });

  it("getCapabilityFact resolves each id", () => {
    for (const id of ALL_CAPABILITY_FACT_IDS) {
      expect(getCapabilityFact(id).id).toBe(id);
    }
  });
});

describe("capabilityFieldMap — G7: active model is available/configured, not used", () => {
  it("F1 is classified available + configured (never used)", () => {
    const f1 = CAPABILITY_FIELD_MAP.F1;
    expect(f1.kind).toBe("available");
    expect(f1.freshness).toBe("configured");
    expect(f1.kind).not.toBe("used");
  });

  it("used-context facts are classified used", () => {
    for (const id of ["F3", "F5", "F10", "F11"] as CapabilityFactId[]) {
      expect(CAPABILITY_FIELD_MAP[id].kind).toBe("used");
    }
  });
});

// ─── Omission-rule behavior (data-driven) ────────────────────────────────────────

interface OmissionCase {
  readonly id: CapabilityFactId;
  readonly desc: string;
  readonly value: unknown;
  readonly expected: OmissionOutcome;
}

const OMISSION_CASES: readonly OmissionCase[] = [
  // F1 active model — null / no-config → omit (never "Not configured"); configured → show.
  { id: "F1", desc: "null runtime", value: null, expected: "omit" },
  { id: "F1", desc: "no providerId and no activeModel", value: { providerId: "", activeModel: null }, expected: "omit" },
  { id: "F1", desc: "configured provider", value: { providerId: "local", activeModel: "qwen2.5" }, expected: "show" },
  { id: "F1", desc: "model only", value: { activeModel: "gpt" }, expected: "show" },

  // F2 context rail — empty → omit (no auto-open); populated → show.
  { id: "F2", desc: "empty rail", value: [], expected: "omit" },
  { id: "F2", desc: "populated rail", value: [{ id: "a" }], expected: "show" },

  // F3 used memory — undefined/empty → omit; present → show.
  { id: "F3", desc: "undefined ids", value: undefined, expected: "omit" },
  { id: "F3", desc: "empty ids", value: [], expected: "omit" },
  { id: "F3", desc: "one id", value: ["m1"], expected: "show" },

  // F4 memory facts
  { id: "F4", desc: "no facts", value: [], expected: "omit" },
  { id: "F4", desc: "some facts", value: [{}], expected: "show" },

  // F5 tool activity
  { id: "F5", desc: "idle", value: [], expected: "omit" },
  { id: "F5", desc: "active call", value: [{}], expected: "show" },

  // F6 tools registry
  { id: "F6", desc: "not loaded", value: [], expected: "omit" },
  { id: "F6", desc: "loaded caps", value: [{}], expected: "show" },

  // F7 OpenClaw — offline/null runtime → unavailable; active+empty → omit; active+skills → show.
  { id: "F7", desc: "null settings (offline)", value: { settings: null, skillCount: 0 }, expected: "unavailable" },
  { id: "F7", desc: "runtime not active", value: { settings: { runtimeActive: false }, skillCount: 3 }, expected: "unavailable" },
  { id: "F7", desc: "active runtime, no skills", value: { settings: { runtimeActive: true }, skillCount: 0 }, expected: "omit" },
  { id: "F7", desc: "active runtime, skills installed", value: { settings: { runtimeActive: true }, skillCount: 2 }, expected: "show" },

  // F8 automations
  { id: "F8", desc: "no workflows", value: [], expected: "omit" },
  { id: "F8", desc: "workflows present", value: [{}], expected: "show" },

  // F9 workflow sessions
  { id: "F9", desc: "no sessions", value: [], expected: "omit" },
  { id: "F9", desc: "active session", value: [{}], expected: "show" },

  // F10 planning
  { id: "F10", desc: "no plan blocks", value: [], expected: "omit" },
  { id: "F10", desc: "plan block", value: [{}], expected: "show" },

  // F11 gui cognition — null (idle) → omit; live session → show.
  { id: "F11", desc: "idle (null)", value: null, expected: "omit" },
  { id: "F11", desc: "live session", value: { lifecycle: "executing" }, expected: "show" },

  // F12 space/activity — Space always known → always show.
  { id: "F12", desc: "active space", value: null, expected: "show" },
  { id: "F12", desc: "with error", value: { error: "boom" }, expected: "show" },
];

describe("capabilityFieldMap — omission rules (present/absent/unknown)", () => {
  it.each(OMISSION_CASES)(
    "$id ($desc) → $expected",
    ({ id, value, expected }) => {
      const d = CAPABILITY_FIELD_MAP[id];
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      expect(evaluateOmission(d as any, value as any)).toBe(expected);
    },
  );

  it("F1 null yields omit — never the 'Not configured' placeholder", () => {
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    expect(evaluateOmission(CAPABILITY_FIELD_MAP.F1 as any, null)).toBe("omit");
    expect(CAPABILITY_FIELD_MAP.F1.displayLabel).not.toMatch(/not configured/i);
  });

  it("every omission rule only ever returns show | omit | unavailable", () => {
    const allowed = new Set<OmissionOutcome>(["show", "omit", "unavailable"]);
    for (const c of OMISSION_CASES) {
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      const out = evaluateOmission(CAPABILITY_FIELD_MAP[c.id] as any, c.value as any);
      expect(allowed.has(out)).toBe(true);
    }
  });
});

// ─── Detail destinations must be EXISTING surfaces (no invented route) ───────────

describe("capabilityFieldMap — detail destinations are existing surfaces", () => {
  it("every fact maps to an existing Space route", () => {
    for (const id of ALL_CAPABILITY_FACT_IDS) {
      const dest = CAPABILITY_FIELD_MAP[id].detailDestination;
      expect(ALL_SPACES).toContain(dest.space);
    }
  });

  it("any inspectorType is one of the registered Inspector types (no new type)", () => {
    for (const id of ALL_CAPABILITY_FACT_IDS) {
      const { inspectorType } = CAPABILITY_FIELD_MAP[id].detailDestination;
      if (inspectorType) {
        expect(KNOWN_INSPECTOR_TYPES.has(inspectorType)).toBe(true);
      }
    }
  });
});

// ─── §2 must-omit encoding ───────────────────────────────────────────────────────

describe("capabilityFieldMap — must-omit facts (no authoritative field)", () => {
  it("encodes the six §2 must-omit facts", () => {
    expect(MUST_OMIT_FACTS).toHaveLength(6);
    expect(MUST_OMIT_FACT_IDS.size).toBe(6);
    expect(NEVER_SURFACE).toBe("omit");
  });

  it("each must-omit fact records a spec touchpoint and reason", () => {
    for (const f of MUST_OMIT_FACTS) {
      expect(f.specTouchpoint.length).toBeGreaterThan(0);
      expect(f.reason.length).toBeGreaterThan(0);
      expect(MUST_OMIT_FACT_IDS.has(f.id)).toBe(true);
    }
  });

  it("must-omit ids are disjoint from the surfaced F-facts", () => {
    for (const f of MUST_OMIT_FACTS) {
      expect(ALL_CAPABILITY_FACT_IDS).not.toContain(f.id as CapabilityFactId);
    }
  });
});
