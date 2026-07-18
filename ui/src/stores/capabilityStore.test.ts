import { describe, it, expect, beforeEach } from "vitest";
import { capabilityStore, CAPABILITY_SEGMENTS } from "./capabilityStore";
import type { Capability, SkillView } from "./capabilityStore";

function cap(over: Partial<Capability> = {}): Capability {
  return {
    id: "prov:cap",
    name: "Web search",
    type: "tool",
    status: "active",
    description: "Search the web",
    source: "prov",
    riskLevel: "green",
    providerId: "prov",
    capabilityId: "cap",
    tags: ["web"],
    elevated: false,
    ...over,
  };
}

describe("capabilityStore (task 8.1, Req 7.1/7.2)", () => {
  beforeEach(() => {
    capabilityStore.setCapabilities([]);
    capabilityStore.setSkills([]);
    capabilityStore.setProviders([]);
    capabilityStore.setModels([]);
    capabilityStore.setIntegrations([]);
    capabilityStore.setGrants([]);
    capabilityStore.setProposals([]);
    capabilityStore.clearDescriptor();
    capabilityStore.setActiveSegment("tools");
  });

  it("exposes exactly the six segments in order (Req 7.1)", () => {
    expect(CAPABILITY_SEGMENTS).toEqual([
      "tools",
      "skills",
      "models",
      "integrations",
      "governance",
      "generate",
    ]);
  });

  it("defaults the active segment to tools", () => {
    expect(capabilityStore.activeSegment()).toBe("tools");
  });

  it("setActiveSegment switches the active segment", () => {
    capabilityStore.setActiveSegment("skills");
    expect(capabilityStore.activeSegment()).toBe("skills");
  });

  it("holds tool capabilities for the palette + Tools segment", () => {
    capabilityStore.setCapabilities([cap()]);
    expect(capabilityStore.capabilities()).toHaveLength(1);
    expect(capabilityStore.capabilities()[0].name).toBe("Web search");
  });

  it("holds skills for the Skills segment", () => {
    const skill: SkillView = {
      slug: "s1",
      name: "PDF reader",
      description: "read pdfs",
      category: "productivity",
      trustTier: "verified",
      installed: true,
      enabled: true,
    };
    capabilityStore.setSkills([skill]);
    expect(capabilityStore.skills()[0].trustTier).toBe("verified");
  });

  it("fetchDescriptor degrades gracefully when the backend is unavailable (Req 20.4)", async () => {
    // No Tauri runtime in the test env → bridgeInvoke returns unavailable.
    const res = await capabilityStore.fetchDescriptor("prov", "cap");
    expect(res.ok).toBe(false);
    // Honest error state is recorded; no stale descriptor is left behind.
    expect(capabilityStore.descriptor()).toBeNull();
    expect(capabilityStore.descriptorError()).toBeTruthy();
    expect(capabilityStore.descriptorLoading()).toBe(false);
  });

  it("clearDescriptor resets descriptor + error state", () => {
    capabilityStore.clearDescriptor();
    expect(capabilityStore.descriptor()).toBeNull();
    expect(capabilityStore.descriptorError()).toBeNull();
  });

  it("loadSegment settles the honest loading flag even when services are absent", async () => {
    await capabilityStore.loadSegment("tools");
    expect(capabilityStore.loading()).toBe(false);
  });
});
