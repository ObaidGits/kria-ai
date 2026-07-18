import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, fireEvent, cleanup, waitFor } from "@solidjs/testing-library";
import type { ServiceResult } from "../../../bridge/types";

// Controllable bridge mock (hoisted before the store/component import).
const bridgeInvoke = vi.fn();
vi.mock("../../../bridge/invoke", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../../bridge/invoke")>();
  return { ...actual, bridgeInvoke: (...args: unknown[]) => bridgeInvoke(...args) };
});

import { MemoryInspector } from "./MemoryInspector";
import { memoryStore, notificationStore, shellStore } from "../../../stores";
import type { InspectorTarget } from "../../../stores/shellStore";

const EXPLAIN = {
  id: "m1",
  content: "the sky is blue",
  memory_type: "semantic",
  state: "active",
  confidence: 0.82,
  importance: 0.4,
  source_event_tag: "user",
  derived_from: ["parent-1"],
  contradicts: ["conflict-9"],
  worth_success: 3,
  worth_failure: 1,
  worth_samples: 4,
  access_count: 5,
  staleness_class: "Slow",
  superseded_by: null,
};

function ok<T>(data: T): ServiceResult<T> {
  return { ok: true, data };
}
function fail(message: string): ServiceResult<never> {
  return { ok: false, code: "error", message } as ServiceResult<never>;
}

/** Route bridgeInvoke by command; memory_explain defaults to EXPLAIN. */
function routeInvoke(map: Record<string, () => ServiceResult<unknown>> = {}): void {
  bridgeInvoke.mockImplementation((command: string) => {
    if (map[command]) return Promise.resolve(map[command]());
    if (command === "memory_explain") return Promise.resolve(ok(EXPLAIN));
    return Promise.resolve(ok(undefined));
  });
}

const target: InspectorTarget = { type: "memory", id: "m1" };

beforeEach(() => {
  bridgeInvoke.mockReset();
  notificationStore.clear();
  // Seed the local read-model with the fact so `forget` can buffer an undo.
  memoryStore.setFacts([
    {
      id: "m1",
      content: "the sky is blue",
      confidence: 0.82,
      worth: 0.6,
      staleness: 0.1,
      source: "user",
      createdAt: Date.now(),
      updatedAt: Date.now(),
      tags: [],
    },
  ]);
  memoryStore.clearUndo();
  shellStore.setInspectorTarget(target);
  routeInvoke();
});

afterEach(() => cleanup());

describe("MemoryInspector — detail (Req 5.2)", () => {
  it("fetches via memory_explain and discloses every Req-5.2 field", async () => {
    render(() => <MemoryInspector target={target} />);
    await waitFor(() => expect(screen.getByText("the sky is blue")).toBeInTheDocument());

    // confidence, truth/state, worth, staleness cues (icon+text). Several of
    // these strings appear in more than one disclosure (cue + field + the AI
    // explanation), so assert presence via getAllByText.
    expect(screen.getAllByText(/82% confidence/).length).toBeGreaterThan(0);
    expect(screen.getAllByText("active").length).toBeGreaterThan(0);
    expect(screen.getByText(/worth 75%/)).toBeInTheDocument();
    expect(screen.getByText("staleness: Slow")).toBeInTheDocument();
    // source, conflicts, lineage.
    expect(screen.getByText("conflict-9")).toBeInTheDocument(); // conflicts
    expect(screen.getByText("parent-1")).toBeInTheDocument(); // derived-from lineage
    // Verification/truth-state field is disclosed.
    expect(screen.getByText("Verification / truth state")).toBeInTheDocument();
    // AI explanation section present.
    expect(screen.getByRole("heading", { name: /AI explanation/i })).toBeInTheDocument();
    expect(screen.getByLabelText("AI-authored by KRIA")).toHaveTextContent("Explained by KRIA");
    expect(screen.getByRole("region", { name: "AI explanation" })).toHaveAttribute(
      "data-provenance",
      "kria",
    );
    expect(bridgeInvoke).toHaveBeenCalledWith("memory_explain", { memoryId: "m1" });
  });

  it("shows an honest loading state before detail arrives", () => {
    render(() => <MemoryInspector target={target} />);
    expect(screen.getByText("Loading memory detail…")).toBeInTheDocument();
  });

  it("shows an honest error with retry when memory_explain fails (no silent blank)", async () => {
    routeInvoke({ memory_explain: () => fail("db locked") });
    render(() => <MemoryInspector target={target} />);
    await waitFor(() => expect(screen.getByRole("alert")).toBeInTheDocument());
    expect(screen.getByText("db locked")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Retry/ })).toBeInTheDocument();
  });

  it("sanitizes the AI explanation (no script survives)", async () => {
    routeInvoke({
      memory_explain: () => ok({ ...EXPLAIN, source_event_tag: "<script>alert(1)</script>" }),
    });
    const { container } = render(() => <MemoryInspector target={target} />);
    await waitFor(() => expect(screen.getByText("the sky is blue")).toBeInTheDocument());
    expect(container.querySelector("script")).toBeNull();
  });
});

describe("MemoryInspector — actions (Req 5.3)", () => {
  async function ready() {
    render(() => <MemoryInspector target={target} />);
    await waitFor(() => expect(screen.getByText("the sky is blue")).toBeInTheDocument());
  }

  it("verify → memory_verify", async () => {
    routeInvoke({ memory_verify: () => ok(true) });
    await ready();
    fireEvent.click(screen.getByRole("button", { name: /Verify/ }));
    await waitFor(() => expect(bridgeInvoke).toHaveBeenCalledWith("memory_verify", { memoryId: "m1" }));
  });

  it("reinforce / penalize → thumbs_up / thumbs_down feedback", async () => {
    await ready();
    fireEvent.click(screen.getByRole("button", { name: /Reinforce/ }));
    await waitFor(() =>
      expect(bridgeInvoke).toHaveBeenCalledWith("memory_record_feedback", {
        targetId: "m1",
        targetKind: "memory",
        signal: "thumbs_up",
      }),
    );
    fireEvent.click(screen.getByRole("button", { name: /Penalize/ }));
    await waitFor(() =>
      expect(bridgeInvoke).toHaveBeenCalledWith("memory_record_feedback", {
        targetId: "m1",
        targetKind: "memory",
        signal: "thumbs_down",
      }),
    );
  });

  it("correct → inline edit then memory_record_feedback(correction)", async () => {
    await ready();
    fireEvent.click(screen.getByRole("button", { name: /Correct/ }));
    const textarea = screen.getByLabelText("Corrected content") as HTMLTextAreaElement;
    fireEvent.input(textarea, { target: { value: "the sky is grey" } });
    fireEvent.click(screen.getByRole("button", { name: /Save correction/ }));
    await waitFor(() =>
      expect(bridgeInvoke).toHaveBeenCalledWith("memory_record_feedback", {
        targetId: "m1",
        targetKind: "memory",
        signal: "correction",
        detail: "the sky is grey",
      }),
    );
  });

  it("forget → memory_forget, then shows Undo which re-adds via memory_remember", async () => {
    routeInvoke({ memory_forget: () => ok(1), memory_remember: () => ok({ decision: "stored" }) });
    await ready();
    fireEvent.click(screen.getByRole("button", { name: /Forget/ }));
    await waitFor(() => expect(bridgeInvoke).toHaveBeenCalledWith("memory_forget", { kind: "memory", value: "m1" }));

    // Undo affordance appears (reversible, Req 5.3).
    const undo = await screen.findByRole("button", { name: /Undo/ });
    fireEvent.click(undo);
    await waitFor(() => expect(bridgeInvoke).toHaveBeenCalledWith("memory_remember", { text: "the sky is blue" }));
  });

  it("surfaces a notification on action failure (no silent failure)", async () => {
    routeInvoke({ memory_verify: () => fail("verify unavailable") });
    await ready();
    fireEvent.click(screen.getByRole("button", { name: /Verify/ }));
    await waitFor(() =>
      expect(notificationStore.active().some((n) => n.level === "error" && n.message === "verify unavailable")).toBe(true),
    );
  });

  // NOTE: kept LAST — it opens a modal Confirm. In the app, confirming unmounts
  // the whole Inspector (closeInspector) which tears the dialog down; rendered
  // standalone here the modal lingers, so we isolate it as the final test.
  it("hard-delete requires a deliberate confirm, then calls memory_hard_delete", async () => {
    routeInvoke({ memory_hard_delete: () => ok(1) });
    await ready();

    // Clicking the action does NOT delete yet — it opens the confirm.
    fireEvent.click(screen.getByRole("button", { name: /Hard delete/ }));
    expect(bridgeInvoke).not.toHaveBeenCalledWith("memory_hard_delete", expect.anything());

    // Deliberate confirmation.
    const confirm = await screen.findByRole("button", { name: /Delete permanently/ });
    fireEvent.click(confirm);
    await waitFor(() =>
      expect(bridgeInvoke).toHaveBeenCalledWith("memory_hard_delete", { kind: "memory", value: "m1" }),
    );
  });
});
