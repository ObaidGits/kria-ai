import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const invokeMock = vi.fn();
vi.mock("../../bridge/invoke", () => ({
  bridgeInvoke: (command: string, args?: unknown, options?: unknown) =>
    invokeMock(command, args, options),
  bridgeInvokeOptional: vi.fn(async () => null),
}));

import MemoryOnboarding from "../../components/memory/MemoryOnboarding";
import { memoryStore } from "../../stores/memoryStore";
import { navigate } from "../router";
import MemorySpace from "./MemorySpace";

const ok = <T,>(data: T) => ({ ok: true as const, data });

describe("MemorySpace reasoning interactions", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    memoryStore.setLoading(false);
    navigate("memory");
  });
  afterEach(() => cleanup());

  it("submits a reasoning-history query and renders authoritative traces", async () => {
    invokeMock.mockResolvedValue(ok({
      traces: [{ task: "battery incident", approach: "causal review", outcome: "charger failed", confidence: 0.91 }],
    }));
    render(() => <MemorySpace />);
    fireEvent.click(screen.getByRole("tab", { name: "Reasoning & Causal" }));
    fireEvent.input(screen.getByLabelText("Task"), { target: { value: "battery incident" } });
    fireEvent.click(screen.getByRole("button", { name: "Query memory" }));

    await waitFor(() => expect(screen.getByText("causal review")).toBeInTheDocument());
    expect(invokeMock).toHaveBeenCalledWith(
      "memory_reasoning_history",
      { task: "battery incident", limit: 100 },
      undefined,
    );
    expect(screen.getByText("charger failed")).toBeInTheDocument();
  });

  it("renders causal links returned by canonical causal query", async () => {
    invokeMock.mockResolvedValue(ok({
      links: [{ cause: "low battery", effect: "shutdown", strength: 0.9 }],
    }));
    await memoryStore.queryReasoning("effects", "low battery");
    navigate("memory", "reasoning");
    render(() => <MemorySpace />);

    expect(screen.getByText("low battery")).toBeInTheDocument();
    expect(screen.getByText("shutdown")).toBeInTheDocument();
    expect(screen.getByText("90% strength")).toBeInTheDocument();
  });
});

describe("MemoryOnboarding canonical flow", () => {
  afterEach(() => {
    vi.restoreAllMocks();
    cleanup();
  });

  it("previews consented data, imports selection, then completes", async () => {
    vi.spyOn(memoryStore, "coldStartStatus").mockReturnValue({
      onboardingComplete: false,
      granted: ["filesystem"],
    });
    const preview = vi.spyOn(memoryStore, "previewColdStart").mockResolvedValue(ok([
      { path: "/home/user/notes.md", detail: "Markdown document" },
      { path: "/home/user/todo.txt", detail: "Text document" },
    ]));
    const importCandidates = vi.spyOn(memoryStore, "importColdStart").mockResolvedValue(ok(1));
    const complete = vi.spyOn(memoryStore, "completeColdStart").mockResolvedValue(ok(undefined));
    const onDone = vi.fn();

    render(() => <MemoryOnboarding onDone={onDone} />);
    fireEvent.click(screen.getByRole("button", { name: "Get started" }));
    fireEvent.click(screen.getByRole("button", { name: "Preview" }));

    await waitFor(() => expect(screen.getByText("/home/user/notes.md")).toBeInTheDocument());
    expect(preview).toHaveBeenCalledWith("filesystem", "", 200);
    fireEvent.click(screen.getAllByRole("checkbox")[1]);
    fireEvent.click(screen.getByRole("button", { name: "Import 1 selected" }));

    await waitFor(() => expect(screen.getByText("Import complete")).toBeInTheDocument());
    expect(importCandidates).toHaveBeenCalledWith("filesystem", [
      { path: "/home/user/notes.md", detail: "Markdown document" },
    ]);
    fireEvent.click(screen.getByRole("button", { name: "Finish" }));
    await waitFor(() => expect(onDone).toHaveBeenCalledOnce());
    expect(complete).toHaveBeenCalledOnce();
  });
});