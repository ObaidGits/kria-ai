import { cleanup, render } from "@solidjs/testing-library";
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
});