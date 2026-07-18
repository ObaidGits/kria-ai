/**
 * graphData tests (task 6.4) — the typed knowledge-graph read-model slice.
 * Verifies data→model mapping + cap, focus/expand merge, predicted-link
 * materialize (backend WRITE via the existing command, view reflects), pin/hide
 * view-state, and honest error state. The bridge is mocked so no Tauri runtime
 * is needed.
 */
import { describe, it, expect, beforeEach, vi } from "vitest";

const invokeMock = vi.fn();
vi.mock("../../../../bridge/invoke", () => ({
  bridgeInvoke: (command: string, args?: Record<string, unknown>) => invokeMock(command, args),
}));

import { graphData } from "./graphData";

const ok = <T>(data: T) => ({ ok: true as const, data });
const err = (message: string) => ({ ok: false as const, code: "error" as const, message });

beforeEach(() => {
  invokeMock.mockReset();
  graphData.reset();
});

describe("load — data → capped model", () => {
  it("maps centrality + communities into typed, capped nodes with 'showing N of M'", async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === "memory_graph_centrality") {
        return Promise.resolve(
          ok({
            nodes: [
              { entity: "a", display_name: "Alpha", degree: 10 },
              { entity: "b", display_name: "Beta", degree: 5 },
              { entity: "c", display_name: "Gamma", degree: 1 },
            ],
            count: 3,
          }),
        );
      }
      if (command === "memory_graph_communities") {
        return Promise.resolve(ok({ communities: [["a", "b"]], count: 1 }));
      }
      return Promise.resolve(ok(null));
    });

    await graphData.load(2); // cap to 2

    expect(graphData.loading()).toBe(false);
    expect(graphData.error()).toBeNull();
    const nodes = graphData.nodes();
    expect(nodes).toHaveLength(2); // capped to top-2 by centrality
    expect(nodes.map((n) => n.id)).toEqual(["a", "b"]);
    expect(nodes[0].community).toBe(0); // from community index
    expect(graphData.capped()?.label).toBe("Showing 2 of 3");
  });

  it("sets an honest error and empties the model when centrality fails", async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === "memory_graph_centrality") return Promise.resolve(err("graph offline"));
      return Promise.resolve(ok({ communities: [], count: 0 }));
    });

    await graphData.load();

    expect(graphData.error()).toBe("graph offline");
    expect(graphData.nodes()).toHaveLength(0);
    expect(graphData.loading()).toBe(false);
  });

  it("tolerates a missing communities service (nodes still load, community -1)", async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === "memory_graph_centrality") {
        return Promise.resolve(ok({ nodes: [{ entity: "a", display_name: "Alpha", degree: 1 }], count: 1 }));
      }
      return Promise.resolve(err("communities unavailable"));
    });

    await graphData.load();
    expect(graphData.nodes()).toHaveLength(1);
    expect(graphData.nodes()[0].community).toBe(-1);
    expect(graphData.error()).toBeNull();
  });
});

describe("expand — focus adds relationships + predicted links (read-only)", () => {
  beforeEach(async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === "memory_graph_centrality") {
        return Promise.resolve(ok({ nodes: [{ entity: "a", display_name: "Alpha", degree: 3 }], count: 1 }));
      }
      if (command === "memory_graph_communities") return Promise.resolve(ok({ communities: [], count: 0 }));
      return Promise.resolve(ok(null));
    });
    await graphData.load();
  });

  it("merges relationship edges + prediction edges and records predictions", async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === "memory_graph_relationships") {
        return Promise.resolve(ok([{ source_id: "a", target_id: "b", rel_type: "knows" }]));
      }
      if (command === "memory_graph_predict_links") {
        return Promise.resolve(ok({ predictions: [{ target: "c", display_name: "Gamma", score: 0.8 }], count: 1 }));
      }
      return Promise.resolve(ok(null));
    });

    await graphData.expand("a");

    expect(graphData.focusedId()).toBe("a");
    // New node "b" from the relationship + "c" implied by prediction edge target.
    expect(graphData.nodes().some((n) => n.id === "b")).toBe(true);
    const edges = graphData.edges();
    expect(edges.some((e) => e.source === "a" && e.target === "b" && !e.predicted)).toBe(true);
    expect(edges.some((e) => e.target === "c" && e.predicted)).toBe(true);
    expect(graphData.predicted().map((p) => p.target)).toContain("c");
  });
});

describe("materializePrediction — backend write, view reflects (§5.4)", () => {
  beforeEach(async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === "memory_graph_centrality") {
        return Promise.resolve(ok({ nodes: [{ entity: "a", display_name: "Alpha", degree: 3 }], count: 1 }));
      }
      if (command === "memory_graph_communities") return Promise.resolve(ok({ communities: [], count: 0 }));
      if (command === "memory_graph_relationships") return Promise.resolve(ok([]));
      if (command === "memory_graph_predict_links") {
        return Promise.resolve(ok({ predictions: [{ target: "c", display_name: "Gamma", score: 0.8 }], count: 1 }));
      }
      return Promise.resolve(ok(null));
    });
    await graphData.load();
    await graphData.expand("a");
  });

  it("dispatches the create-relationship command and promotes the predicted edge", async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === "memory_graph_create_relationship") return Promise.resolve(ok("rel-1"));
      return Promise.resolve(ok(null));
    });

    const res = await graphData.materializePrediction("c", "related_to");
    expect(res.ok).toBe(true);
    expect(invokeMock).toHaveBeenCalledWith(
      "memory_graph_create_relationship",
      expect.objectContaining({ sourceId: "a", targetId: "c", relType: "related_to" }),
    );
    const edge = graphData.edges().find((e) => e.target === "c");
    expect(edge?.predicted).toBe(false); // promoted to a real edge
    expect(graphData.predicted().some((p) => p.target === "c")).toBe(false); // prediction cleared
  });

  it("returns an honest failure and leaves the prediction when the write fails", async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === "memory_graph_create_relationship") return Promise.resolve(err("write denied"));
      return Promise.resolve(ok(null));
    });

    const res = await graphData.materializePrediction("c");
    expect(res.ok).toBe(false);
    if (!res.ok) expect(res.message).toBe("write denied");
    expect(graphData.edges().find((e) => e.target === "c")?.predicted).toBe(true);
  });

  it("refuses to materialize with no focused node", async () => {
    graphData.reset();
    const res = await graphData.materializePrediction("c");
    expect(res.ok).toBe(false);
  });
});

describe("view-state interactions (pin / hide)", () => {
  beforeEach(async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === "memory_graph_centrality") {
        return Promise.resolve(
          ok({
            nodes: [
              { entity: "a", display_name: "Alpha", degree: 3 },
              { entity: "b", display_name: "Beta", degree: 2 },
            ],
            count: 2,
          }),
        );
      }
      if (command === "memory_graph_communities") return Promise.resolve(ok({ communities: [], count: 0 }));
      return Promise.resolve(ok(null));
    });
    await graphData.load();
    graphData.seed({
      nodes: graphData.nodes(),
      edges: [{ source: "a", target: "b", predicted: false }],
    });
  });

  it("toggles pin state without mutating the model", () => {
    graphData.togglePin("a");
    expect(graphData.pinned().has("a")).toBe(true);
    graphData.togglePin("a");
    expect(graphData.pinned().has("a")).toBe(false);
  });

  it("hides a node and prunes its dangling edges from the visible set", () => {
    graphData.hide("b");
    expect(graphData.hidden().has("b")).toBe(true);
    expect(graphData.visibleNodes().some((n) => n.id === "b")).toBe(false);
    // The a→b edge is pruned because b is hidden.
    expect(graphData.visibleEdges()).toHaveLength(0);
    graphData.unhideAll();
    expect(graphData.visibleNodes().some((n) => n.id === "b")).toBe(true);
  });
});
