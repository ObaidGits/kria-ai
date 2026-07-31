import { describe, it, expect, beforeEach, afterEach } from "vitest";
import { render, screen, fireEvent, cleanup, waitFor } from "@solidjs/testing-library";
import MemorySpace from "./MemorySpace";
import { memoryStore, shellStore } from "../../stores";
import type { MemoryFact } from "../../stores";
import { navigate, currentRoute } from "../router";

function makeFact(id: string, content: string, over: Partial<MemoryFact> = {}): MemoryFact {
  const now = Date.now();
  return {
    id,
    content,
    confidence: 0.8,
    worth: 0.5,
    staleness: 0,
    source: "conversation",
    createdAt: now,
    updatedAt: now,
    tags: [],
    ...over,
  };
}

describe("MemorySpace — landing + segments (task 6.1, Req 5.1)", () => {
  beforeEach(() => {
    // Clean, empty store + reset the router to the Memory landing each test.
    memoryStore.setFacts([]);
    memoryStore.setDocuments([]);
    memoryStore.setSearchQuery("");
    memoryStore.setLoading(false);
    navigate("memory");
  });

  afterEach(() => cleanup());

  it("renders a tablist with the landing + all eight lens segments (Req 5.1)", () => {
    render(() => <MemorySpace />);
    expect(screen.getByRole("tablist")).toBeInTheDocument();
    for (const name of [
      "Overview",
      "Explorer",
      "Timeline",
      "Goals & Plans",
      "Reasoning & Causal",
      "Library",
      "Knowledge Graph",
      "Cognition",
      "Cold Start",
    ]) {
      expect(screen.getByRole("tab", { name })).toBeInTheDocument();
    }
  });

  it("shows the landing (Overview) as the default view when no segment is routed", () => {
    render(() => <MemorySpace />);
    expect(screen.getByRole("tab", { name: "Overview" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(screen.getByRole("heading", { name: "Overview" })).toBeInTheDocument();
  });

  it("routes the segment via the typed router and swaps the shown region on switch (Req 1.5/5.1)", () => {
    render(() => <MemorySpace />);
    // Landing is shown; Explorer region is not.
    expect(screen.getByRole("heading", { name: "Overview" })).toBeInTheDocument();
    expect(screen.queryByRole("heading", { name: "Explorer" })).toBeNull();

    fireEvent.click(screen.getByRole("tab", { name: "Explorer" }));

    // Router now carries the segment, and the Explorer region is shown.
    expect(currentRoute().space).toBe("memory");
    expect(currentRoute().segment).toBe("explorer");
    expect(memoryStore.activeSegment()).toBe("explorer");
    expect(screen.getByRole("heading", { name: "Explorer" })).toBeInTheDocument();
    expect(screen.queryByRole("heading", { name: "Overview" })).toBeNull();
  });

  it("renders each lens region with a placeholder when its data lands later (Req 5.1)", () => {
    render(() => <MemorySpace />);
    fireEvent.click(screen.getByRole("tab", { name: "Goals & Plans" }));
    expect(currentRoute().segment).toBe("goals");
    // The Goals & Plans region is present as an honest placeholder.
    expect(screen.getByRole("heading", { name: "Goals & Plans" })).toBeInTheDocument();
  });

  it("mounts the v2 Knowledge destination (list-first, no synthetic universe) (Req 5.4/5.5)", async () => {
    render(() => <MemorySpace />);
    fireEvent.click(screen.getByRole("tab", { name: "Knowledge Graph" }));
    expect(currentRoute().segment).toBe("knowledgegraph");
    // No legacy SVG universe — that renderer was deleted in F4.9.6.
    expect(document.querySelector(".memory-universe")).toBeNull();
    // v2 Knowledge destination renders its accessible list shell immediately.
    expect(document.querySelector('[data-testid="knowledge-shell"]')).not.toBeNull();

    // Items are now loaded from the backend (task: live-wired Knowledge).
    // Under jsdom the Tauri `invoke` bridge is unavailable, so the load settles
    // with zero items and the honest empty state appears. Wait for the
    // in-flight load to resolve before asserting the settled state.
    await waitFor(() => {
      expect(document.querySelector('[data-testid="semantic-list-loading"]')).toBeNull();
      expect(document.querySelector('[data-testid="empty-state"]')).not.toBeNull();
    });

    // The prototype workspace remains mounted so loading, empty, and recovery
    // states retain the same spatial frame instead of swapping presentations.
    expect(document.querySelector("canvas")).not.toBeNull();
    expect(document.querySelector('[data-testid="empty-state"]')).not.toBeNull();
  });

  it("filters memories in Explorer by the header search (Req 5.1)", () => {
    memoryStore.setFacts([
      makeFact("f1", "alpha fact about rust"),
      makeFact("f2", "beta fact about python"),
    ]);
    render(() => <MemorySpace />);
    fireEvent.click(screen.getByRole("tab", { name: "Explorer" }));

    // Both facts shown, honest count.
    expect(screen.getByText("Showing 2 of 2")).toBeInTheDocument();
    expect(screen.getByText("alpha fact about rust")).toBeInTheDocument();
    expect(screen.getByText("beta fact about python")).toBeInTheDocument();

    // Search narrows the list (client filter over the store facts).
    memoryStore.setSearchQuery("alpha");
    expect(screen.getByText("Showing 1 of 2")).toBeInTheDocument();
    expect(screen.getByText("alpha fact about rust")).toBeInTheDocument();
    expect(screen.queryByText("beta fact about python")).toBeNull();
  });

  it("wires the header search box to the store", () => {
    render(() => <MemorySpace />);
    const box = screen.getByRole("searchbox", { name: "Search memory" });
    fireEvent.input(box, { target: { value: "hello" } });
    expect(memoryStore.searchQuery()).toBe("hello");
  });

  it("shows an honest empty state on the landing when there are no memories (Req 5.1)", () => {
    render(() => <MemorySpace />);
    expect(screen.getByRole("heading", { name: "No memories yet" })).toBeInTheDocument();
  });

  it("shows an honest empty state in Explorer when there are no facts", () => {
    render(() => <MemorySpace />);
    fireEvent.click(screen.getByRole("tab", { name: "Explorer" }));
    expect(
      screen.getByRole("heading", { name: "No memories to explore" }),
    ).toBeInTheDocument();
  });

  it("shows an honest loading state instead of an empty state while loading", () => {
    memoryStore.setLoading(true);
    render(() => <MemorySpace />);
    expect(screen.getByRole("status")).toHaveTextContent("Loading memory…");
    // Not the empty state while loading.
    expect(screen.queryByRole("heading", { name: "No memories yet" })).toBeNull();
  });

  it("lists indexed documents in the Library segment where data exists", () => {
    memoryStore.setDocuments([
      { id: "d1", title: "Design doc", type: "markdown", indexedAt: Date.now(), size: 10 },
    ]);
    render(() => <MemorySpace />);
    fireEvent.click(screen.getByRole("tab", { name: "Library" }));
    expect(screen.getByText("Design doc")).toBeInTheDocument();
  });
});

describe("MemorySpace — deep-link entityId opens the Inspector (Req 5.7)", () => {
  beforeEach(() => {
    memoryStore.setFacts([]);
    memoryStore.setDocuments([]);
    memoryStore.setSearchQuery("");
    memoryStore.setLoading(false);
    shellStore.closeInspector();
    navigate("memory");
  });

  afterEach(() => {
    cleanup();
    shellStore.closeInspector();
  });

  it("opens the shared Inspector on the routed memory when deep-linked", () => {
    // Deep-link as Converse's "why did KRIA answer this" does.
    navigate("memory", "explorer", "mem-42");
    render(() => <MemorySpace />);
    const target = shellStore.inspectorTarget();
    expect(target?.type).toBe("memory");
    expect(target?.id).toBe("mem-42");
  });

  it("passes the fact payload to the Inspector when the memory is loaded", () => {
    memoryStore.setFacts([makeFact("mem-7", "cached fact")]);
    navigate("memory", "explorer", "mem-7");
    render(() => <MemorySpace />);
    const target = shellStore.inspectorTarget();
    expect(target?.id).toBe("mem-7");
    expect((target?.data as MemoryFact | undefined)?.content).toBe("cached fact");
  });

  it("does not open the Inspector when no entityId is routed", () => {
    navigate("memory", "explorer");
    render(() => <MemorySpace />);
    expect(shellStore.inspectorTarget()).toBeNull();
  });
});


describe("MemorySpace — long-list virtualization (Req 16.2)", () => {
  beforeEach(() => {
    memoryStore.setFacts(Array.from({ length: 500 }, (_, i) =>
      makeFact(`virtual-${i}`, `Virtualized memory ${i}`, { createdAt: i, updatedAt: i })));
    memoryStore.setDocuments([]);
    memoryStore.setSearchQuery("");
    memoryStore.setLoading(false);
    navigate("memory");
  });

  afterEach(() => cleanup());

  it("mounts only visible Explorer cards", () => {
    render(() => <MemorySpace />);
    fireEvent.click(screen.getByRole("tab", { name: "Explorer" }));
    const mounted = document.querySelectorAll('[data-virtual-list="memory-explorer"] [data-fact-id]').length;
    expect(mounted).toBeGreaterThan(0);
    expect(mounted).toBeLessThan(500);
  });

  it("mounts only visible Timeline rows", () => {
    render(() => <MemorySpace />);
    fireEvent.click(screen.getByRole("tab", { name: "Timeline" }));
    const mounted = document.querySelectorAll('[data-virtual-list="memory-timeline"] [data-fact-id]').length;
    expect(mounted).toBeGreaterThan(0);
    expect(mounted).toBeLessThan(500);
  });
});
