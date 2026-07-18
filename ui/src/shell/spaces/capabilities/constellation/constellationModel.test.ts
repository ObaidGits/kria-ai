/**
 * constellationModel tests (task 8.3) — the pure data→graph mapping backing the
 * Capabilities Constellation lens + its 2D catalog fallback (Req 7.5).
 *
 * Verifies the mapping produces the shared graph grammar (nodes/edges), the
 * documented relationship edges (provider→capability, provider→model, skill→
 * trust group, integration→capability dependency), degree-based centrality,
 * per-kind community coloring, and stable/deterministic output. No GL / DOM.
 */
import { describe, it, expect } from "vitest";
import {
  buildConstellation,
  iconForKind,
  labelForKind,
  providerNodeId,
  toolNodeId,
  modelNodeId,
  skillNodeId,
  integrationNodeId,
  trustGroupNodeId,
  type ConstellationInputs,
} from "./constellationModel";
import type {
  Capability,
  IntegrationView,
  ModelView,
  Provider,
  SkillView,
} from "../../../../stores";

function capability(overrides: Partial<Capability> = {}): Capability {
  return {
    id: "prov-a:tool-x",
    name: "Tool X",
    type: "tool",
    status: "active",
    description: "Does X",
    source: "prov-a",
    riskLevel: "green",
    providerId: "prov-a",
    capabilityId: "tool-x",
    tags: [],
    elevated: false,
    ...overrides,
  };
}

function inputs(overrides: Partial<ConstellationInputs> = {}): ConstellationInputs {
  return {
    capabilities: [],
    models: [],
    providers: [],
    skills: [],
    integrations: [],
    ...overrides,
  };
}

describe("buildConstellation — nodes", () => {
  it("maps a capability to a tool node under its provider (provider→capability edge)", () => {
    const model = buildConstellation(
      inputs({
        capabilities: [capability()],
        providers: [{ id: "prov-a", name: "Provider A", type: "local", active: true } as Provider],
      }),
    );

    const toolId = toolNodeId("prov-a", "tool-x");
    const provId = providerNodeId("prov-a");
    expect(model.nodes.find((n) => n.id === toolId)).toBeDefined();
    expect(model.nodes.find((n) => n.id === provId)).toBeDefined();
    // Provider node takes the provider's display name when known.
    expect(model.meta.get(provId)?.name).toBe("Provider A");
    // The "provides" edge exists provider → tool.
    expect(model.edges).toContainEqual(
      expect.objectContaining({ source: provId, target: toolId, relType: "provides" }),
    );
  });

  it("tags a tool node with a descriptor target when provider+capability ids exist", () => {
    const model = buildConstellation(inputs({ capabilities: [capability()] }));
    const m = model.meta.get(toolNodeId("prov-a", "tool-x"));
    expect(m?.kind).toBe("tool");
    expect(m?.hasDescriptor).toBe(true);
    expect(m?.providerId).toBe("prov-a");
    expect(m?.capabilityId).toBe("tool-x");
  });

  it("maps a model under its provider (provider→model 'serves' edge)", () => {
    const model = buildConstellation(
      inputs({
        models: [{ id: "m1", name: "Llama", provider: "prov-b" } as ModelView],
      }),
    );
    expect(model.edges).toContainEqual(
      expect.objectContaining({
        source: providerNodeId("prov-b"),
        target: modelNodeId("m1"),
        relType: "serves",
      }),
    );
    expect(model.meta.get(modelNodeId("m1"))?.kind).toBe("model");
  });

  it("groups skills by trust tier (skill→trust-group 'trusted-as' edge)", () => {
    const model = buildConstellation(
      inputs({
        skills: [
          { slug: "s1", name: "Skill One", description: "", category: "gen", trustTier: "community", installed: true, enabled: true } as SkillView,
          { slug: "s2", name: "Skill Two", description: "", category: "gen", trustTier: "community", installed: true, enabled: true } as SkillView,
        ],
      }),
    );
    const group = trustGroupNodeId("community");
    expect(model.nodes.find((n) => n.id === group)).toBeDefined();
    expect(model.edges).toContainEqual(
      expect.objectContaining({ source: skillNodeId("s1"), target: group, relType: "trusted-as" }),
    );
    expect(model.edges).toContainEqual(
      expect.objectContaining({ source: skillNodeId("s2"), target: group, relType: "trusted-as" }),
    );
  });

  it("links an MCP integration to the tools it backs (dependency 'exposes' edge)", () => {
    const model = buildConstellation(
      inputs({
        capabilities: [capability({ id: "srv1:t", providerId: "srv1", capabilityId: "t", source: "srv1" })],
        integrations: [
          { id: "mcp:srv1", name: "Server 1", kind: "mcp", status: "connected", detail: "2 tools" } as IntegrationView,
        ],
      }),
    );
    expect(model.edges).toContainEqual(
      expect.objectContaining({
        source: integrationNodeId("mcp:srv1"),
        target: toolNodeId("srv1", "t"),
        relType: "exposes",
      }),
    );
  });

  it("keeps a non-MCP integration as an isolated node (honest: no surfaced tools)", () => {
    const model = buildConstellation(
      inputs({
        integrations: [
          { id: "google", name: "Google Workspace", kind: "google", status: "connected", detail: "you@x" } as IntegrationView,
        ],
      }),
    );
    const id = integrationNodeId("google");
    expect(model.nodes.find((n) => n.id === id)).toBeDefined();
    expect(model.edges.filter((e) => e.source === id || e.target === id)).toHaveLength(0);
  });
});

describe("buildConstellation — centrality + community", () => {
  it("computes centrality as node degree (busy provider is the biggest hub)", () => {
    const model = buildConstellation(
      inputs({
        capabilities: [
          capability({ id: "p:a", providerId: "p", capabilityId: "a", source: "p" }),
          capability({ id: "p:b", providerId: "p", capabilityId: "b", source: "p" }),
        ],
        models: [{ id: "m", name: "M", provider: "p" } as ModelView],
      }),
    );
    const prov = model.nodes.find((n) => n.id === providerNodeId("p"));
    // provider connects to 2 tools + 1 model = degree 3.
    expect(prov?.centrality).toBe(3);
    // each tool has degree 1.
    expect(model.nodes.find((n) => n.id === toolNodeId("p", "a"))?.centrality).toBe(1);
  });

  it("colors nodes by kind community index (distinct per kind, deterministic)", () => {
    const model = buildConstellation(
      inputs({
        capabilities: [capability()],
        models: [{ id: "m", name: "M", provider: "prov-a" } as ModelView],
      }),
    );
    const prov = model.meta.get(providerNodeId("prov-a"));
    const tool = model.meta.get(toolNodeId("prov-a", "tool-x"));
    const modelMeta = model.meta.get(modelNodeId("m"));
    expect(prov?.kind).toBe("provider");
    expect(tool?.kind).toBe("tool");
    expect(modelMeta?.kind).toBe("model");
    const provNode = model.nodes.find((n) => n.id === providerNodeId("prov-a"))!;
    const toolNode = model.nodes.find((n) => n.id === toolNodeId("prov-a", "tool-x"))!;
    expect(provNode.community).not.toBe(toolNode.community);
  });

  it("produces stable, id-sorted node order", () => {
    const model = buildConstellation(
      inputs({
        capabilities: [
          capability({ id: "z:z", providerId: "z", capabilityId: "z", source: "z", name: "Z" }),
          capability({ id: "a:a", providerId: "a", capabilityId: "a", source: "a", name: "A" }),
        ],
      }),
    );
    const ids = model.nodes.map((n) => n.id);
    expect(ids).toEqual([...ids].sort((x, y) => x.localeCompare(y)));
  });
});

describe("kind helpers", () => {
  it("gives every kind an icon + label", () => {
    for (const kind of ["provider", "tool", "model", "skill", "integration", "trustgroup"] as const) {
      expect(iconForKind(kind)).toBeTruthy();
      expect(labelForKind(kind)).toBeTruthy();
    }
  });
});
