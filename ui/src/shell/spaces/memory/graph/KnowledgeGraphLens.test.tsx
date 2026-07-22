import { cleanup, fireEvent, render, screen, within } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const invokeMock = vi.fn();
vi.mock("../../../../bridge/invoke", () => ({
  bridgeInvoke: (command: string, args?: unknown) => invokeMock(command, args),
  bridgeInvokeOptional: vi.fn(async () => null),
}));
vi.mock("../../../../prototypes/gateProbes", () => ({
  runG2Probe: vi.fn(async () => null),
}));

import { eventBus } from "../../../../stores/eventBus";
import KnowledgeGraphLens from "./KnowledgeGraphLens";
import { graphData } from "./graphData";

const ok = <T,>(data: T) => ({ ok: true as const, data });
const callsFor = (command: string) => invokeMock.mock.calls.filter(([name]) => name === command).length;
const flush = async () => {
  await Promise.resolve();
  await Promise.resolve();
};

describe("KnowledgeGraphLens memory refresh", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    invokeMock.mockImplementation((command: string) =>
      Promise.resolve(command === "memory_graph_centrality"
        ? ok({ nodes: [], count: 0 })
        : ok({ communities: [], count: 0 })),
    );
    graphData.reset();
  });

  afterEach(() => {
    cleanup();
    vi.useRealTimers();
  });

  it("coalesces graph-relevant memory events and ignores unrelated kinds", async () => {
    render(() => <KnowledgeGraphLens />);
    await flush();
    expect(callsFor("memory_graph_centrality")).toBe(1);

    eventBus.emit("memory:updated", { factId: "m1", kind: "feedback" });
    vi.advanceTimersByTime(300);
    await flush();
    expect(callsFor("memory_graph_centrality")).toBe(1);

    eventBus.emit("memory:updated", { factId: "m1", kind: "relationship" });
    eventBus.emit("memory:updated", { factId: "m2", kind: "entity" });
    vi.advanceTimersByTime(250);
    await flush();
    expect(callsFor("memory_graph_centrality")).toBe(2);
  });

  it("ratifies the shipped 2D renderer and generated-facet contract", async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === "memory_graph_centrality") {
        return Promise.resolve(ok({
          nodes: [{ entity: "project-1", display_name: "Project Atlas", degree: 4 }],
          count: 1,
        }));
      }
      return Promise.resolve(ok({ communities: [["project-1"]], count: 1 }));
    });

    const { container } = render(() => <KnowledgeGraphLens />);
    await flush();

    expect(screen.getByRole("textbox", { name: "Filter visible memories" })).toBeInTheDocument();

    const emphasis = screen.getByRole("group", { name: "Graph display emphasis" });
    const generatedFacets = screen.getByRole("button", { name: /Generated facets/ });
    const relationships = screen.getByRole("button", { name: /Relationships/ });
    const predictions = screen.getByRole("button", { name: /Predicted links/ });
    expect(generatedFacets).toHaveAttribute("aria-pressed", "true");
    expect(relationships).toHaveAttribute("aria-pressed", "false");
    expect(predictions).toHaveAttribute("aria-pressed", "false");
    fireEvent.click(relationships);
    expect(generatedFacets).toHaveAttribute("aria-pressed", "false");
    expect(relationships).toHaveAttribute("aria-pressed", "true");
    expect(within(emphasis).getByRole("button", { name: "Open table view" })).toBeInTheDocument();

    const camera = screen.getByRole("group", { name: "Camera controls" });
    expect(within(camera).getAllByRole("button")).toHaveLength(3);
    expect(within(camera).getByRole("button", { name: "Zoom in" })).toBeInTheDocument();
    expect(within(camera).getByRole("button", { name: "Zoom out" })).toBeInTheDocument();
    expect(within(camera).getByRole("button", { name: "Reset view" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Timeline|Auto arrange|Center graph|Pin memory/ })).not.toBeInTheDocument();
    expect(container.querySelector(".memory-universe__search kbd")).toBeNull();

    expect(container.querySelector('svg[aria-label="Radial 2D memory view centered on current focus"]')).not.toBeNull();
    expect(container.querySelector("canvas")).toBeNull();
    expect(screen.queryByRole("button", { name: "3D" })).not.toBeInTheDocument();
    expect(container).toHaveTextContent("1 memory shown");
    expect(container).not.toHaveTextContent(/semantic search|active memories|communities/i);

    const facet = screen.getByRole("button", { name: "Generated navigation facet Projects, 1 memories" });
    expect(facet).toHaveAttribute("data-authority-class", "navigation");
    expect(facet).toHaveAttribute("data-generated", "true");
    expect(container.querySelectorAll(".memory-universe__navigation-facets path")).toHaveLength(0);
    fireEvent.click(facet);

    expect(screen.getByRole("complementary", { name: "Details for Projects" })).toBeInTheDocument();
    expect(container).toHaveTextContent("GRAPH DETAILS");
    expect(container).toHaveTextContent("Facet membership is not a stored relationship");
    expect(container).not.toHaveTextContent(/AI reasoning|importance|confidence|synchronized|live context/i);
  });
});