/**
 * MemoryGraphFallback tests (task 6.5) — the mandatory 2D/keyboard
 * representation of the Knowledge Graph (Req 5.5 / 16.3 / 17.5).
 *
 * Verifies the fallback renders a REAL accessible table of nodes with
 * sort/filter/search, keyboard row navigation, select→focus+expand, pin/hide,
 * predicted-link materialize (routes through the existing command — bridge
 * mocked), the "showing N of M" cap notice, and table a11y (header scope /
 * aria-sort). No WebGL / Tauri needed: graphData is seeded directly and the
 * bridge is mocked.
 */
import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, screen, fireEvent, waitFor, within } from "@solidjs/testing-library";

const invokeMock = vi.fn();
vi.mock("../../../../bridge/invoke", () => ({
  bridgeInvoke: (command: string, args?: Record<string, unknown>) => invokeMock(command, args),
}));

import { MemoryGraphFallback } from "./MemoryGraphFallback";
import { graphData } from "./graphData";
import { applyNodeCap, type GraphEdge, type GraphNode } from "./graphModel";

const ok = <T,>(data: T) => ({ ok: true as const, data });

function nodes(): GraphNode[] {
  return [
    { id: "kria", label: "KRIA", community: 0, centrality: 42 },
    { id: "memory", label: "Memory system", community: 0, centrality: 31 },
    { id: "voice", label: "Voice pipeline", community: 1, centrality: 24 },
    { id: "graph", label: "Knowledge graph", community: 2, centrality: 12 },
    { id: "laptop", label: "Local laptop", community: -1, centrality: 4 },
  ];
}

function edges(): GraphEdge[] {
  return [
    { source: "kria", target: "memory", relType: "uses", predicted: false },
    { source: "kria", target: "voice", relType: "uses", predicted: false },
    { source: "memory", target: "graph", relType: "contains", predicted: false },
    { source: "kria", target: "graph", relType: "predicted", predicted: true },
  ];
}

function seed() {
  const n = nodes();
  graphData.seed({ nodes: n, edges: edges() }, applyNodeCap(n, 300));
}

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockResolvedValue(ok(null));
  graphData.reset();
});

describe("2D fallback table — structure + cap", () => {
  it("renders a real table of nodes with column headers (scope=col)", () => {
    seed();
    render(() => <MemoryGraphFallback reason="2D default (test)" />);

    expect(screen.getByRole("table")).toBeInTheDocument();
    for (const header of ["Entity", "Community", "Centrality", "Connections", "Actions"]) {
      expect(screen.getByRole("columnheader", { name: new RegExp(header) })).toBeInTheDocument();
    }
    // Row header uses scope="row" for the entity (real table semantics).
    expect(screen.getByRole("rowheader", { name: /KRIA/ })).toBeInTheDocument();
  });

  it("shows the honest 'showing N of M' cap notice", () => {
    seed();
    render(() => <MemoryGraphFallback />);
    expect(screen.getByText(/Showing all 5/)).toBeInTheDocument();
  });

  it("shows an honest empty state when there are no nodes", () => {
    render(() => <MemoryGraphFallback />);
    expect(screen.getByText(/No graph yet/)).toBeInTheDocument();
  });
});

describe("2D fallback table — sort", () => {
  it("defaults to centrality descending and toggles direction on header click", () => {
    seed();
    render(() => <MemoryGraphFallback />);

    const centralityHeader = screen.getByRole("columnheader", { name: /Centrality/ });
    expect(centralityHeader).toHaveAttribute("aria-sort", "descending");

    // First visible entity row should be the most central (KRIA).
    const firstRowSelect = screen.getAllByRole("button", { name: /KRIA|Memory system|Voice|Knowledge graph|Local laptop/ })[0];
    expect(firstRowSelect).toHaveTextContent("KRIA");

    fireEvent.click(within(centralityHeader).getByRole("button"));
    expect(centralityHeader).toHaveAttribute("aria-sort", "ascending");
  });

  it("sorts by entity name when the Entity header is activated", () => {
    seed();
    render(() => <MemoryGraphFallback />);
    const entityHeader = screen.getByRole("columnheader", { name: /Entity/ });
    fireEvent.click(within(entityHeader).getByRole("button"));
    expect(entityHeader).toHaveAttribute("aria-sort", "ascending");
  });
});

describe("2D fallback table — search / filter", () => {
  it("filters rows by entity label", async () => {
    seed();
    render(() => <MemoryGraphFallback />);

    const search = screen.getByRole("searchbox", { name: /Filter entities/ });
    fireEvent.input(search, { target: { value: "voice" } });

    await waitFor(() => {
      expect(screen.getByRole("rowheader", { name: /Voice pipeline/ })).toBeInTheDocument();
      expect(screen.queryByRole("rowheader", { name: /^KRIA$/ })).toBeNull();
    });
    expect(screen.getByText(/1 match/)).toBeInTheDocument();
  });
});

describe("2D fallback table — keyboard navigation", () => {
  it("moves focus across rows with arrow keys (roving tabindex)", () => {
    seed();
    render(() => <MemoryGraphFallback />);
    const table = screen.getByRole("table");

    fireEvent.keyDown(table, { key: "ArrowDown" });
    const rowButtons = table.querySelectorAll<HTMLButtonElement>("[data-graph-row]");
    expect(document.activeElement).toBe(rowButtons[0]);

    fireEvent.keyDown(table, { key: "ArrowDown" });
    expect(document.activeElement).toBe(rowButtons[1]);

    fireEvent.keyDown(table, { key: "ArrowUp" });
    expect(document.activeElement).toBe(rowButtons[0]);

    fireEvent.keyDown(table, { key: "End" });
    expect(document.activeElement).toBe(rowButtons[rowButtons.length - 1]);
  });
});

describe("2D fallback table — select → focus + expand", () => {
  it("focuses a node and reveals its relationships as rows", async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === "memory_graph_relationships") {
        return Promise.resolve(ok([{ source_id: "kria", target_id: "memory", rel_type: "uses" }]));
      }
      if (command === "memory_graph_predict_links") {
        return Promise.resolve(ok({ predictions: [], count: 0 }));
      }
      return Promise.resolve(ok(null));
    });
    seed();
    render(() => <MemoryGraphFallback />);

    fireEvent.click(screen.getByRole("button", { name: /^KRIA$/ }));

    await waitFor(() => {
      expect(screen.getByRole("region", { name: /Relationships for KRIA/ })).toBeInTheDocument();
    });
    // The relationship rows include KRIA's neighbours.
    const region = screen.getByRole("region", { name: /Relationships for KRIA/ });
    expect(within(region).getByRole("rowheader", { name: /Memory system/ })).toBeInTheDocument();
  });
});

describe("2D fallback table — pin / hide", () => {
  it("toggles pin state", () => {
    seed();
    render(() => <MemoryGraphFallback />);
    const pinButton = screen.getAllByRole("button", { name: /^Pin$/ })[0];
    fireEvent.click(pinButton);
    expect(graphData.pinned().size).toBe(1);
  });

  it("hides a node so it leaves the table and can be restored", async () => {
    seed();
    render(() => <MemoryGraphFallback />);

    const laptopRow = screen.getByRole("rowheader", { name: /Local laptop/ });
    expect(laptopRow).toBeInTheDocument();

    // Hide the least-central node (last row) via its Hide action.
    const hideButtons = screen.getAllByRole("button", { name: /Hide/ });
    fireEvent.click(hideButtons[hideButtons.length - 1]);

    await waitFor(() => {
      expect(screen.queryByRole("rowheader", { name: /Local laptop/ })).toBeNull();
    });

    fireEvent.click(screen.getByRole("button", { name: /Show hidden/ }));
    await waitFor(() => {
      expect(screen.getByRole("rowheader", { name: /Local laptop/ })).toBeInTheDocument();
    });
  });
});

describe("2D fallback table — materialize predicted link (existing command)", () => {
  it("dispatches memory_graph_create_relationship for a predicted edge", async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === "memory_graph_relationships") return Promise.resolve(ok([]));
      if (command === "memory_graph_predict_links") return Promise.resolve(ok({ predictions: [], count: 0 }));
      if (command === "memory_graph_create_relationship") return Promise.resolve(ok("rel-1"));
      return Promise.resolve(ok(null));
    });
    seed();
    render(() => <MemoryGraphFallback />);

    // Focus KRIA (which has a predicted edge to "graph").
    fireEvent.click(screen.getByRole("button", { name: /^KRIA$/ }));
    await waitFor(() => screen.getByRole("region", { name: /Relationships for KRIA/ }));

    const materialize = await screen.findByRole("button", { name: /Materialize/ });
    fireEvent.click(materialize);

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith(
        "memory_graph_create_relationship",
        expect.objectContaining({ sourceId: "kria", targetId: "graph" }),
      );
    });
  });
});
