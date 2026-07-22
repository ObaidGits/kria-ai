/**
 * Capability / context FACT LINKS tests (Task 10.5 / IU-07; UIE-H-011,
 * UIE-H-012, UIE-M-019).
 *
 * Verifies the shared read-only link helper maps each surfaced fact to an
 * EXISTING destination from `capabilityFieldMap`'s `detailDestination` (an
 * existing Space; a REGISTERED Inspector type) — never an invented route — and
 * that activation performs ONLY navigate/openInspector, with no
 * send/tool/approval mutation, and no fabricated id.
 */
import { describe, it, expect, beforeEach, vi, afterEach } from "vitest";

import {
  resolveFactLink,
  activateFactLink,
  openFactDetail,
} from "./capabilityLinks";
import { ALL_SPACES, currentRoute, navigate } from "./router";
import { shellStore } from "../stores/shellStore";
import {
  ALL_CAPABILITY_FACT_IDS,
  getCapabilityFact,
  type CapabilityFactId,
} from "../stores/capabilityFieldMap";

const REGISTERED_INSPECTOR_TYPES = new Set([
  "memory",
  "capability",
  "automation-node",
  "device",
  "observatory",
]);

beforeEach(() => {
  navigate("converse");
  shellStore.setInspectorTarget(null);
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe("resolveFactLink — maps to EXISTING destinations only (no invented route)", () => {
  it("every fact resolves to an existing Space (navigate mode, no entity)", () => {
    for (const id of ALL_CAPABILITY_FACT_IDS) {
      const link = resolveFactLink(id);
      expect(link).not.toBeNull();
      expect(ALL_SPACES).toContain(link!.space);
      // Without an authoritative id, no Inspector link is produced.
      expect(link!.mode).toBe("navigate");
      expect(link!.entityId).toBeUndefined();
    }
  });

  it("the resolved space/segment matches the fact's detailDestination exactly", () => {
    for (const id of ALL_CAPABILITY_FACT_IDS) {
      const dest = getCapabilityFact(id).detailDestination;
      const link = resolveFactLink(id)!;
      expect(link.space).toBe(dest.space);
      expect(link.segment).toBe(dest.segment);
    }
  });

  it("produces an Inspector link on a REGISTERED type when an authoritative id is supplied", () => {
    // Facts whose detailDestination carries an inspectorType.
    const withInspector = ALL_CAPABILITY_FACT_IDS.filter(
      (id) => getCapabilityFact(id).detailDestination.inspectorType,
    );
    expect(withInspector.length).toBeGreaterThan(0);
    for (const id of withInspector) {
      const dest = getCapabilityFact(id).detailDestination;
      const link = resolveFactLink(id, { entityId: "entity-1" })!;
      expect(link.mode).toBe("inspector");
      expect(link.inspectorType).toBe(dest.inspectorType);
      expect(REGISTERED_INSPECTOR_TYPES.has(link.inspectorType!)).toBe(true);
      expect(link.entityId).toBe("entity-1");
    }
  });
});

describe("no fabrication — a fact without an authoritative id yields no Inspector link", () => {
  it("inspectorOnly + blank/absent id → null (control omitted, never a broken destination)", () => {
    // F2 (context rail) has a memory Inspector destination.
    expect(resolveFactLink("F2", { inspectorOnly: true })).toBeNull();
    expect(resolveFactLink("F2", { entityId: "", inspectorOnly: true })).toBeNull();
    expect(resolveFactLink("F2", { entityId: "   ", inspectorOnly: true })).toBeNull();
    expect(resolveFactLink("F2", { entityId: null, inspectorOnly: true })).toBeNull();
  });

  it("inspectorOnly on a fact with NO inspector type → null even with an id", () => {
    // F5 (tool activity) → WorkLane, no inspectorType.
    expect(getCapabilityFact("F5").detailDestination.inspectorType).toBeUndefined();
    expect(resolveFactLink("F5", { entityId: "x", inspectorOnly: true })).toBeNull();
  });

  it("inspectorOnly with a real id → Inspector link (memory)", () => {
    const link = resolveFactLink("F2", { entityId: "mem-42", inspectorOnly: true })!;
    expect(link.mode).toBe("inspector");
    expect(link.inspectorType).toBe("memory");
    expect(link.entityId).toBe("mem-42");
  });
});

describe("activateFactLink — dispatch-only (navigate / openInspector)", () => {
  it("navigate link routes to the existing Space via the typed router", () => {
    const link = resolveFactLink("F8")!; // Automations
    activateFactLink(link);
    expect(currentRoute().space).toBe("automations");
    expect(shellStore.inspectorTarget()).toBeNull();
  });

  it("inspector link opens the ONE shared Inspector on the registered type, with focus-return owner", () => {
    const spy = vi.spyOn(shellStore, "openInspector");
    const link = resolveFactLink("F2", { entityId: "mem-9", inspectorOnly: true })!;
    activateFactLink(link, { regionSelector: "#space-root" });
    expect(spy).toHaveBeenCalledTimes(1);
    expect(spy).toHaveBeenCalledWith("memory", "mem-9", undefined, {
      regionSelector: "#space-root",
    });
    // The single Inspector target is set (non-stacking).
    expect(shellStore.inspectorTarget()).toEqual({
      type: "memory",
      id: "mem-9",
      data: undefined,
    });
  });

  it("openFactDetail returns false (no dispatch) when no link is offered", () => {
    const navSpy = vi.spyOn(shellStore, "openInspector");
    const ok = openFactDetail("F2", { inspectorOnly: true });
    expect(ok).toBe(false);
    expect(navSpy).not.toHaveBeenCalled();
    // Route unchanged; inspector unchanged.
    expect(currentRoute().space).toBe("converse");
    expect(shellStore.inspectorTarget()).toBeNull();
  });

  it("read-only: activation never mutates send/tool/approval state (only shell/router touched)", () => {
    // Approval overlay + inspector start closed/null; a link activation must not
    // open approvals, seize approval state, or run anything.
    const approvalsSpy = vi.spyOn(shellStore, "setApprovalsOpen");
    activateFactLink(resolveFactLink("F9")!); // workflow sessions → Automations
    expect(approvalsSpy).not.toHaveBeenCalled();
    expect(shellStore.approvalsOpen()).toBe(false);
    expect(currentRoute().space).toBe("automations");
  });
});

describe("per-fact destination table (F2–F11 → existing owner)", () => {
  const cases: Array<{ id: CapabilityFactId; space: string; inspector?: string }> = [
    { id: "F2", space: "converse", inspector: "memory" },
    { id: "F3", space: "memory", inspector: "memory" },
    { id: "F4", space: "memory", inspector: "memory" },
    { id: "F5", space: "converse" },
    { id: "F6", space: "capabilities", inspector: "capability" },
    { id: "F7", space: "capabilities", inspector: "capability" },
    { id: "F8", space: "automations", inspector: "automation-node" },
    { id: "F9", space: "automations" },
    { id: "F10", space: "converse" },
    { id: "F11", space: "converse" },
  ];
  for (const c of cases) {
    it(`${c.id} → Space ${c.space}${c.inspector ? ` / Inspector ${c.inspector}` : ""}`, () => {
      const nav = resolveFactLink(c.id)!;
      expect(nav.space).toBe(c.space);
      if (c.inspector) {
        const insp = resolveFactLink(c.id, { entityId: "id-1" })!;
        expect(insp.mode).toBe("inspector");
        expect(insp.inspectorType).toBe(c.inspector);
      }
    });
  }
});
