import fc from "fast-check";
import { describe, expect, it } from "vitest";
import type { GraphNode } from "./graphModel";
import { buildUniverse, categoryForNode } from "./memoryUniverseModel";

const node = (id: string, label: string, community = -1): GraphNode => ({
  id,
  label,
  community,
  centrality: 1,
});

describe("generated memory navigation facets", () => {
  it("places unmatched nodes in an explicit Other facet instead of deriving meaning from community or ID", () => {
    expect(categoryForNode(node("alpha", "Unmatched label", 1)).id).toBe("other");
    expect(categoryForNode(node("beta", "Unmatched label", 99)).id).toBe("other");
  });

  it("retains supplied authority relationships without creating facet edges", () => {
    const nodes = [node("project", "Project Atlas"), node("goal", "Goal launch")];
    const model = buildUniverse(nodes, [{ source: "project", target: "goal", relType: "supports" }]);

    expect(model.relationships).toHaveLength(1);
    expect(model.relationships[0]).toMatchObject({ source: "project", target: "goal", relType: "supports" });
    expect(model.hubs.every((hub) => hub.generated && hub.authorityClass === "navigation")).toBe(true);
  });

  /** **Validates: Requirements 1.4, 2.2** — Design Property 4: Navigation Exclusion. */
  it("never turns generated facet membership into authority topology", () => {
    const graphNode = fc.record({
      id: fc.uuid(),
      label: fc.string({ maxLength: 60 }),
      community: fc.integer({ min: -1, max: 100 }),
      centrality: fc.double({ min: 0, max: 10_000, noNaN: true }),
    });

    fc.assert(
      fc.property(fc.uniqueArray(graphNode, { selector: (value) => value.id, maxLength: 100 }), (nodes) => {
        const model = buildUniverse(nodes, []);
        expect(model.relationships).toEqual([]);
        expect(model.memories.map((memory) => memory.id).sort()).toEqual(nodes.map((value) => value.id).sort());
        expect(model.hubs.every((hub) => hub.generated && hub.authorityClass === "navigation")).toBe(true);
      }),
    );
  });
});
