import { describe, it, expect, beforeEach, vi, afterEach } from "vitest";
import { render, screen, fireEvent, cleanup, within, waitFor } from "@solidjs/testing-library";
import ConverseEmptyState, { COLD_EXAMPLE_INTENTS } from "./ConverseEmptyState";
import { groundedStarters, type ExampleIntent } from "./groundedStarters";
import {
  getAdaptiveUsage,
  recordAdaptiveUse,
  resetAdaptiveSuggestions,
} from "../../../adaptive";
import { converseStore, capabilityStore } from "../../../stores";
import type { Thread } from "../../../stores/converseStore";
import type { Capability, OpenClawSettings } from "../../../stores/capabilityStore";
import { capabilityDisclosures } from "./capabilityDisclosure";
import { currentRoute, navigate } from "../../router";
import { shellStore } from "../../../stores/shellStore";

function makeThread(id: string, title: string, updatedAt: number): Thread {
  return {
    id,
    title,
    createdAt: updatedAt,
    updatedAt,
    pinned: false,
    archived: false,
    temporary: false,
  };
}

function activeTool(id: string): Capability {
  return {
    id,
    name: id,
    type: "tool",
    status: "active",
    description: "",
    source: "native",
    riskLevel: "green",
  };
}

/**
 * Reset every signal the empty state derives from so tests are order-independent.
 * `activeThreadId` in particular LEAKS across tests (the default continuation
 * handler calls the real `setActiveThread`), and under the 4-state classifier a
 * lone active empty thread is Cold Start, not Continuation — so it must be reset.
 */
function resetStores(): void {
  if (converseStore.activeThreadId() !== null) converseStore.setActiveThread(null);
  converseStore.setThreads([]);
  converseStore.clearMessages();
  capabilityStore.setCapabilities([]);
  capabilityStore.setSkills([]);
  capabilityStore.setMcpServers([]);
  capabilityStore.setGenerateStatus(null);
  capabilityStore.setOpenClawSettings(null);
  resetAdaptiveSuggestions();
}

/** Minimal OpenClaw runtime settings for F7 grounding (task 10.6). */
function openClawSettings(runtimeActive: boolean): OpenClawSettings {
  return {
    enabled: runtimeActive,
    image: "",
    warmPerClass: 0,
    maxConcurrentInvocations: 0,
    defaultTimeoutSecs: 0,
    maxWarmAgeSecs: 0,
    maxRestartAttempts: 0,
    rewriteDescriptions: false,
    checkUpdates: false,
    registryIndexUrl: "",
    communityAllowsNetwork: false,
    verifiedSkipsHitl: false,
    runtimeActive,
  };
}

/** An installed + enabled skill for F7 grounding (task 10.6). */
function enabledSkill(slug: string) {
  return {
    slug,
    name: slug,
    description: "",
    category: "general",
    trustTier: "local",
    installed: true,
    enabled: true,
  };
}

// NOTE: placed first on purpose. Coach retirement is module-global and
// `resetAdaptiveSuggestions()` does NOT un-retire coaches, and later tests
// retire it by selecting starters. Running the coach assertion first keeps the
// coach visible so we can prove it lives behind (and only behind) the disclosure.
describe("ConverseEmptyState — secondary disclosure defers coach/controls/reset (task 6.5, UIE-H-004, Req 6.4)", () => {
  beforeEach(resetStores);
  afterEach(cleanup);

  it("keeps starters + Composer focal: coach, per-suggestion controls, and reset are NOT in the primary flow before the disclosure is opened", () => {
    render(() => <ConverseEmptyState />);

    // Primary content is the starters (focal, primary tab stops).
    expect(screen.getByRole("button", { name: "Ask a question" })).toBeInTheDocument();

    // Deferred controls are absent from the primary flow until disclosed.
    expect(screen.queryByRole("note", { name: "Getting started hint" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Reset suggestions to defaults" })).toBeNull();
    expect(
      screen.queryByRole("group", { name: /^Suggestion controls for/ }),
    ).toBeNull();
    expect(
      screen.queryByRole("button", { name: "Pin suggestion: Ask a question" }),
    ).toBeNull();
  });

  it("exposes a labelled, keyboard-reachable disclosure trigger that is not a starter tab stop", () => {
    render(() => <ConverseEmptyState />);
    const trigger = screen.getByRole("button", { name: "Customize suggestions" });
    expect(trigger).toBeInTheDocument();
    expect(trigger.tagName).toBe("BUTTON"); // real button → keyboard-focusable/activatable
    // The disclosure content (a dialog) is not mounted until the trigger is used.
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("opening the disclosure reveals coach + per-suggestion pin/dismiss/explain + reset, all still wired", () => {
    render(() => <ConverseEmptyState />);
    fireEvent.click(screen.getByRole("button", { name: "Customize suggestions" }));

    // Focus-managed dialog panel.
    expect(screen.getByRole("dialog")).toBeInTheDocument();

    // Coach text.
    expect(screen.getByRole("note", { name: "Getting started hint" })).toBeInTheDocument();

    // Per-suggestion controls for each visible starter: explain (note), pin, dismiss.
    const controls = screen.getByRole("group", { name: "Suggestion controls for Ask a question" });
    expect(within(controls).getByRole("note")).toBeInTheDocument();
    expect(
      within(controls).getByRole("button", { name: "Pin suggestion: Ask a question" }),
    ).toBeInTheDocument();
    expect(
      within(controls).getByRole("button", { name: "Dismiss suggestion: Ask a question" }),
    ).toBeInTheDocument();

    // Reset control.
    expect(
      screen.getByRole("button", { name: "Reset suggestions to defaults" }),
    ).toBeInTheDocument();
  });
});

describe("ConverseEmptyState — Core-forward 4-state empty state (task 6.4, Req 6.1–6.6)", () => {
  beforeEach(resetStores);

  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it("is Core-forward: always renders the KRIA Core presence", () => {
    render(() => <ConverseEmptyState />);
    // CorePresence renders role="img" with a per-state accessible label.
    expect(screen.getByRole("img", { name: /KRIA/i })).toBeInTheDocument();
  });

  it("Cold Start: no history + no capabilities → concise orientation + ≤3 safe starters", () => {
    render(() => <ConverseEmptyState />);
    const region = screen.getByRole("region", { name: "Start a conversation" });
    expect(region).toHaveAttribute("data-empty-mode", "cold");
    expect(screen.getByRole("heading", { name: "What can I help with?" })).toBeInTheDocument();

    const starters = screen.getByRole("list", { name: "Starter prompts" });
    const items = starters.querySelectorAll("li");
    expect(items.length).toBeGreaterThan(0);
    expect(items.length).toBeLessThanOrEqual(3);
    // With no capability available, fall back to safe generic-but-truthful base
    // starters — never fabricate an unavailable capability.
    expect(screen.getByRole("button", { name: "Ask a question" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Remember something" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Generate an image" })).toBeNull();
  });

  it("Cold Start starters are GROUNDED in enabled capabilities and omit unavailable ones", () => {
    // Only generation is available → the image starter appears; automate/skill do not.
    capabilityStore.setGenerateStatus({ available: true, backend: "comfyui", detail: "" });
    const { unmount } = render(() => <ConverseEmptyState />);
    expect(screen.getByRole("button", { name: "Generate an image" })).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Automate a task on your computer" }),
    ).toBeNull();
    expect(screen.queryByRole("button", { name: "Run one of your skills" })).toBeNull();
    unmount();

    // Only tools are available → the automate starter appears; image does not.
    capabilityStore.setGenerateStatus(null);
    capabilityStore.setCapabilities([activeTool("files.read")]);
    render(() => <ConverseEmptyState />);
    expect(
      screen.getByRole("button", { name: "Automate a task on your computer" }),
    ).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Generate an image" })).toBeNull();
  });

  it("Cold Start caps grounded starters at 3 even when many capabilities are enabled", () => {
    capabilityStore.setGenerateStatus({ available: true, backend: "comfyui", detail: "" });
    capabilityStore.setCapabilities([activeTool("files.read")]);
    capabilityStore.setSkills([
      {
        slug: "s1",
        name: "Skill One",
        description: "",
        category: "general",
        trustTier: "local",
        installed: true,
        enabled: true,
      },
    ]);
    render(() => <ConverseEmptyState />);
    const items = screen.getByRole("list", { name: "Starter prompts" }).querySelectorAll("li");
    expect(items.length).toBe(3);
    // Capability-specific starters win the slots over generic base starters.
    expect(
      screen.getByRole("button", { name: "Automate a task on your computer" }),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Generate an image" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Run one of your skills" })).toBeInTheDocument();
  });

  it("Intentional New Thread: shows the new-task starters regardless of unrelated history", () => {
    // Unrelated history exists, but explicit new-thread intent outranks it.
    converseStore.setThreads([
      makeThread("t1", "Old research", 3),
      makeThread("t2", "Budget", 2),
    ]);
    vi.spyOn(converseStore, "emptyStateClass").mockReturnValue("intentional-new-thread");

    render(() => <ConverseEmptyState />);
    const region = screen.getByRole("region", { name: "Start a conversation" });
    expect(region).toHaveAttribute("data-empty-mode", "new");
    expect(screen.getByRole("heading", { name: "Start a new task" })).toBeInTheDocument();
    // New-task starters render; continuation choices do NOT (history does not leak).
    expect(screen.getByRole("list", { name: "Starter prompts" })).toBeInTheDocument();
    expect(screen.queryByRole("list", { name: "Continue suggestions" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Continue: Old research" })).toBeNull();
  });

  it("Continuation: shows ≤3 relevant resumptions, most-recent first", () => {
    converseStore.setThreads([
      makeThread("t1", "Daily notes", 3),
      makeThread("t2", "Trip plan", 2),
      makeThread("t3", "Budget", 1),
      makeThread("t4", "Old thread", 0),
    ]);
    render(() => <ConverseEmptyState />);
    const region = screen.getByRole("region", { name: "Start a conversation" });
    expect(region).toHaveAttribute("data-empty-mode", "continuation");
    expect(
      screen.getByRole("heading", { name: "Continue where you left off" }),
    ).toBeInTheDocument();

    const list = screen.getByRole("list", { name: "Continue suggestions" });
    const items = list.querySelectorAll("li");
    expect(items.length).toBe(3); // capped at 3 even though 4 threads exist
    expect(items[0].textContent).toContain("Daily notes");
  });

  it("Cold Start: clicking a starter STAGES the composer draft (no send/tool/nav)", () => {
    const updateDraft = vi.spyOn(converseStore, "updateDraft");
    const sendMessage = vi.spyOn(converseStore, "sendMessage");
    const setActiveThread = vi.spyOn(converseStore, "setActiveThread");

    render(() => <ConverseEmptyState />);
    const first = COLD_EXAMPLE_INTENTS[0];
    fireEvent.click(screen.getByRole("button", { name: first.label }));

    // Stages the draft text for review — never auto-sends, never runs a tool.
    expect(updateDraft).toHaveBeenCalledWith({ text: first.draft });
    expect(sendMessage).not.toHaveBeenCalled();
    expect(setActiveThread).not.toHaveBeenCalled();
  });

  it("Cold Start: repeated starter selection is idempotent (stages, never sends)", () => {
    const updateDraft = vi.spyOn(converseStore, "updateDraft");
    const sendMessage = vi.spyOn(converseStore, "sendMessage");

    render(() => <ConverseEmptyState />);
    const button = screen.getByRole("button", { name: "Ask a question" });
    fireEvent.click(button);
    fireEvent.click(button);

    expect(updateDraft).toHaveBeenCalledWith({ text: "What can you help me with?" });
    expect(sendMessage).not.toHaveBeenCalled();
  });

  it("Continuation: clicking a resumption OPENS the thread (no send/tool)", () => {
    converseStore.setThreads([makeThread("t1", "Daily notes", 3)]);
    const setActiveThread = vi.spyOn(converseStore, "setActiveThread");
    const sendMessage = vi.spyOn(converseStore, "sendMessage");

    render(() => <ConverseEmptyState />);
    fireEvent.click(screen.getByRole("button", { name: "Continue: Daily notes" }));

    expect(setActiveThread).toHaveBeenCalledWith("t1");
    expect(sendMessage).not.toHaveBeenCalled();
  });

  it("never renders a blank page: content is always present (Req 4.6)", () => {
    // Cold Start
    const { unmount } = render(() => <ConverseEmptyState />);
    expect(screen.getByRole("heading").textContent).toBeTruthy();
    expect(screen.getByRole("img", { name: /KRIA/i })).toBeInTheDocument();
    unmount();

    // Continuation
    converseStore.setThreads([makeThread("t1", "Daily notes", 1)]);
    render(() => <ConverseEmptyState />);
    expect(screen.getByRole("heading").textContent).toBeTruthy();
    expect(screen.getByRole("img", { name: /KRIA/i })).toBeInTheDocument();
  });

  it("respects injected adaptive lists (task 13.x hooks)", () => {
    const onSelectIntent = vi.fn();
    const onContinue = vi.fn();

    // Explicit suggestions force the continuation branch regardless of state.
    render(() => (
      <ConverseEmptyState
        suggestions={[{ id: "x1", label: "Resume research" }]}
        onContinue={onContinue}
      />
    ));
    fireEvent.click(screen.getByRole("button", { name: "Continue: Resume research" }));
    expect(onContinue).toHaveBeenCalledWith({ id: "x1", label: "Resume research" });
    cleanup();

    // Explicit intents (no threads) stay on the starter branch and use the handler.
    render(() => (
      <ConverseEmptyState
        intents={[{ id: "i1", icon: "zap", label: "Custom intent", draft: "custom" }]}
        onSelectIntent={onSelectIntent}
      />
    ));
    fireEvent.click(screen.getByRole("button", { name: "Custom intent" }));
    expect(onSelectIntent).toHaveBeenCalledWith({
      id: "i1",
      icon: "zap",
      label: "Custom intent",
      draft: "custom",
    });
  });
});


describe("ConverseEmptyState — adaptive presentation (Req 19.1/19.2)", () => {
  beforeEach(resetStores);

  afterEach(cleanup);

  it("records explicit starter selection and promotes it only on the next presentation", () => {
    const { unmount } = render(() => <ConverseEmptyState />);
    fireEvent.click(screen.getByRole("button", { name: "Ask a question" }));
    expect(getAdaptiveUsage("empty-state", "intent:ask")?.count).toBe(1);
    unmount();

    render(() => <ConverseEmptyState />);
    const labels = Array.from(
      screen.getByRole("list", { name: "Starter prompts" }).querySelectorAll("li"),
      (item) => item.textContent,
    );
    expect(labels[0]).toContain("Ask a question");
  });

  it("can promote an older thread into the visible suggestions without deleting peers", () => {
    converseStore.setThreads([
      makeThread("t1", "Newest", 4),
      makeThread("t2", "Second", 3),
      makeThread("t3", "Third", 2),
      makeThread("t4", "Older frequent", 1),
    ]);
    for (let use = 0; use < 5; use += 1) recordAdaptiveUse("empty-state", "thread:t4");

    render(() => <ConverseEmptyState />);
    expect(screen.getByRole("button", { name: "Continue: Older frequent" })).toBeInTheDocument();
    expect(screen.getAllByRole("listitem")).toHaveLength(3);
  });
});


describe("ConverseEmptyState — explainable controls (Req 19.3/19.4)", () => {
  beforeEach(resetStores);

  afterEach(cleanup);

  it("explains, pins, dismisses, and resets an adaptive starter", () => {
    // Re-query the group after each action: re-ranking recreates the row DOM, so
    // a held reference would go stale.
    const controlsFor = (name: string) =>
      screen.getByRole("group", { name: `Suggestion controls for ${name}` });

    render(() => <ConverseEmptyState />);
    // Controls now live behind the labelled secondary disclosure (task 6.5).
    fireEvent.click(screen.getByRole("button", { name: "Customize suggestions" }));
    expect(within(controlsFor("Remember something")).getByRole("note")).toHaveTextContent(
      "Default suggestion",
    );

    fireEvent.click(
      within(controlsFor("Remember something")).getByRole("button", {
        name: "Pin suggestion: Remember something",
      }),
    );
    expect(within(controlsFor("Remember something")).getByRole("note")).toHaveTextContent(
      "Pinned by you",
    );

    fireEvent.click(
      within(controlsFor("Remember something")).getByRole("button", {
        name: "Dismiss suggestion: Remember something",
      }),
    );
    expect(screen.queryByRole("button", { name: "Remember something" })).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Reset suggestions to defaults" }));
    expect(screen.getByRole("button", { name: "Remember something" })).toBeInTheDocument();
  });

  it("keeps the Continuation state after every resumption is dismissed", () => {
    // Two non-archived threads with no active thread → Continuation. Dismissing
    // the visible resumptions must NOT collapse the branch back to Cold Start:
    // the branch is driven by the deterministic classifier, not by which
    // adaptive suggestions remain visible.
    converseStore.setThreads([
      makeThread("t1", "Daily notes", 2),
      makeThread("t2", "Trip plan", 1),
    ]);
    render(() => <ConverseEmptyState />);
    // Resumption controls now live behind the labelled secondary disclosure.
    fireEvent.click(screen.getByRole("button", { name: "Customize suggestions" }));
    fireEvent.click(screen.getByRole("button", { name: "Dismiss suggestion: Daily notes" }));
    fireEvent.click(screen.getByRole("button", { name: "Dismiss suggestion: Trip plan" }));
    expect(screen.getByRole("region", { name: "Start a conversation" }))
      .toHaveAttribute("data-empty-mode", "continuation");
    expect(screen.getByRole("heading", { name: "Continue where you left off" })).toBeInTheDocument();
  });
});


/**
 * Correctness Property 5 — "Safe staging and preserved authority" (design §11.6,
 * §20; UIE-L-002; Req 6.5 / 6.6; task 6.6).
 *
 *   Repeated starter/suggestion selection changes only editable staged
 *   presentation state. No presentation enhancement sends, invokes a tool,
 *   grants approval, or changes policy/runtime authority.
 *
 * Coverage is GENERATED: a seeded enumeration over the full set of grounded
 * starters (under every capability configuration that can surface each starter)
 * × arbitrary repeat counts. `fast-check` is not a dependency of ui/, so — like
 * the other matrix tasks in this spec — we use a deterministic seeded generator
 * (mulberry32) that is fully reproducible.
 *
 * The property asserts, for every starter and every repeat count N ≥ 1:
 *   (a) the staged Composer draft text equals THAT starter's draft (replace, not
 *       append) — i.e. selecting is IDEMPOTENT: N selections leave exactly the
 *       starter's draft, never an accumulation;
 *   (b) NO authority-bearing action fired: sendMessage, submitIntent,
 *       selectPlanOption (tool/plan), or setActiveThread (navigation) were never
 *       called — the ONLY store mutation is updateDraft;
 *   (c) the draft stays editable (updateDraft targets the composerDraft.text the
 *       Composer textarea reflects) and other draft fields are preserved.
 *
 * The continuation companion property asserts a resumption is PURE navigation:
 * repeated selection calls only setActiveThread and never sends/invokes a tool.
 *
 * Validates: Requirements 6.5, 6.6
 */

/** Deterministic, reproducible PRNG so the "generated" repeats never flake. */
function mulberry32(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a |= 0;
    a = (a + 0x6d2b79f5) | 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/** Capability presets, each surfacing a distinct grounded starter set. */
function applyCapabilityPreset(preset: "none" | "generate" | "tools" | "skills" | "all"): void {
  capabilityStore.setGenerateStatus(null);
  capabilityStore.setCapabilities([]);
  capabilityStore.setSkills([]);
  capabilityStore.setMcpServers([]);
  if (preset === "generate" || preset === "all") {
    capabilityStore.setGenerateStatus({ available: true, backend: "comfyui", detail: "" });
  }
  if (preset === "tools" || preset === "all") {
    capabilityStore.setCapabilities([activeTool("files.read")]);
  }
  if (preset === "skills" || preset === "all") {
    capabilityStore.setSkills([
      {
        slug: "s1",
        name: "Skill One",
        description: "",
        category: "general",
        trustTier: "local",
        installed: true,
        enabled: true,
      },
    ]);
  }
}

describe("ConverseEmptyState — Property 5: safe staging + preserved authority (task 6.6, Req 6.5/6.6)", () => {
  beforeEach(resetStores);
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it("covers the FULL grounded-starter set: every distinct starter is reachable across capability presets", () => {
    const seen = new Set<string>();
    for (const preset of ["none", "generate", "tools", "skills", "all"] as const) {
      applyCapabilityPreset(preset);
      for (const s of groundedStarters()) seen.add(s.id);
    }
    // automate + generate-image + run-skill + remember + ask = every candidate.
    expect(seen).toEqual(new Set(["automate", "generate-image", "run-skill", "remember", "ask"]));
  });

  it("PROPERTY: repeated selection of any starter stages ONLY that draft (idempotent, replace) and never sends/invokes/approves/navigates", () => {
    const rand = mulberry32(0x6c6f6);
    const presets = ["none", "generate", "tools", "skills", "all"] as const;

    // Enumerate every distinct grounded starter (id → intent) across presets.
    const universe = new Map<string, ExampleIntent>();
    for (const preset of presets) {
      applyCapabilityPreset(preset);
      for (const s of groundedStarters()) universe.set(s.id, s);
    }

    for (const intent of universe.values()) {
      // 6 arbitrary-but-reproducible repeat counts in [1, 8] per starter.
      for (let iter = 0; iter < 6; iter += 1) {
        const repeat = 1 + Math.floor(rand() * 8);

        resetStores();
        // Pre-seed an unrelated draft field to prove REPLACE preserves others
        // and does not append to prior text.
        converseStore.updateDraft({ text: "PRE-EXISTING", attachments: [] });

        const sendMessage = vi.spyOn(converseStore, "sendMessage");
        const submitIntent = vi.spyOn(converseStore, "submitIntent");
        const selectPlanOption = vi.spyOn(converseStore, "selectPlanOption");
        const setActiveThread = vi.spyOn(converseStore, "setActiveThread");

        // Drive the REAL default handler (no onSelectIntent override) so we
        // observe the store's true staged state.
        const { unmount } = render(() => <ConverseEmptyState intents={[intent]} />);
        const button = screen.getByRole("button", { name: intent.label });
        for (let n = 0; n < repeat; n += 1) fireEvent.click(button);

        // (a)+(c): staged text is exactly this starter's draft — replace, not
        // append, and idempotent regardless of repeat count.
        expect(converseStore.composerDraft().text).toBe(intent.draft);
        // Other draft fields are preserved (attachments untouched by staging).
        expect(converseStore.composerDraft().attachments).toEqual([]);

        // (b): no authority-bearing action ever fired.
        expect(sendMessage).not.toHaveBeenCalled();
        expect(submitIntent).not.toHaveBeenCalled();
        expect(selectPlanOption).not.toHaveBeenCalled();
        expect(setActiveThread).not.toHaveBeenCalled();

        unmount();
        vi.restoreAllMocks();
      }
    }
  });

  it("PROPERTY: repeated continuation selection is PURE navigation (setActiveThread only, never sends/invokes a tool)", () => {
    const rand = mulberry32(0x7ab1e);
    for (let iter = 0; iter < 8; iter += 1) {
      const repeat = 1 + Math.floor(rand() * 8);
      resetStores();

      const sendMessage = vi.spyOn(converseStore, "sendMessage");
      const submitIntent = vi.spyOn(converseStore, "submitIntent");
      const selectPlanOption = vi.spyOn(converseStore, "selectPlanOption");
      const onContinue = vi.fn();

      render(() => (
        <ConverseEmptyState
          suggestions={[{ id: "t1", label: "Resume research" }]}
          onContinue={onContinue}
        />
      ));
      const button = screen.getByRole("button", { name: "Continue: Resume research" });
      for (let n = 0; n < repeat; n += 1) fireEvent.click(button);

      // Navigation handler fired once per click, always with the same target
      // (idempotent target — no accumulation, no send).
      expect(onContinue).toHaveBeenCalledTimes(repeat);
      expect(onContinue).toHaveBeenLastCalledWith({ id: "t1", label: "Resume research" });
      expect(sendMessage).not.toHaveBeenCalled();
      expect(submitIntent).not.toHaveBeenCalled();
      expect(selectPlanOption).not.toHaveBeenCalled();

      cleanup();
      vi.restoreAllMocks();
    }
  });

  it("PROPERTY: default continuation handler performs navigation only (setActiveThread), never a send/tool", () => {
    const rand = mulberry32(0x0dd);
    for (let iter = 0; iter < 6; iter += 1) {
      const repeat = 1 + Math.floor(rand() * 6);
      resetStores();
      converseStore.setThreads([makeThread("t1", "Daily notes", 3)]);

      const setActiveThread = vi.spyOn(converseStore, "setActiveThread");
      const sendMessage = vi.spyOn(converseStore, "sendMessage");
      const submitIntent = vi.spyOn(converseStore, "submitIntent");
      const selectPlanOption = vi.spyOn(converseStore, "selectPlanOption");

      render(() => <ConverseEmptyState />);
      const button = screen.getByRole("button", { name: "Continue: Daily notes" });
      for (let n = 0; n < repeat; n += 1) fireEvent.click(button);

      expect(setActiveThread).toHaveBeenCalledWith("t1");
      expect(sendMessage).not.toHaveBeenCalled();
      expect(submitIntent).not.toHaveBeenCalled();
      expect(selectPlanOption).not.toHaveBeenCalled();

      cleanup();
      vi.restoreAllMocks();
    }
  });
});


/**
 * Task 6.8 — heading/disclosure semantics, keyboard starter/disclosure flow,
 * focus management, and localization expansion for the empty state.
 *
 * These extend (do not duplicate) the task 6.4/6.5 coverage above: they add the
 * explicit heading-LEVEL assertions, the disclosure dialog labelling + keyboard
 * open/close + focus-return assertions, the "starters are the primary tab stops
 * ahead of the disclosure" ordering assertion, and the long-localized-string
 * layout-safety assertion.
 *
 * Validates: Requirements 6.4, 16.4
 */
describe("ConverseEmptyState — heading/disclosure semantics + keyboard/focus (task 6.8, Req 6.4/16.4)", () => {
  beforeEach(resetStores);
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it("every empty-state presentation exposes exactly one real level-2 heading", () => {
    // Cold Start.
    const cold = render(() => <ConverseEmptyState />);
    const coldHeading = screen.getByRole("heading", { level: 2, name: "What can I help with?" });
    expect(coldHeading.tagName).toBe("H2");
    cold.unmount();

    // Intentional New Thread (explicit intent outranks unrelated history).
    converseStore.setThreads([makeThread("t1", "Old research", 2)]);
    vi.spyOn(converseStore, "emptyStateClass").mockReturnValue("intentional-new-thread");
    const nt = render(() => <ConverseEmptyState />);
    expect(screen.getByRole("heading", { level: 2, name: "Start a new task" }).tagName).toBe("H2");
    nt.unmount();
    vi.restoreAllMocks();

    // Continuation.
    converseStore.setThreads([makeThread("c1", "Daily notes", 3)]);
    render(() => <ConverseEmptyState />);
    expect(
      screen.getByRole("heading", { level: 2, name: "Continue where you left off" }).tagName,
    ).toBe("H2");
  });

  it("the disclosure is a labelled button that opens a labelled dialog (button/dialog semantics)", () => {
    render(() => <ConverseEmptyState />);
    const trigger = screen.getByRole("button", { name: "Customize suggestions" });
    // Real <button> → intrinsically keyboard-focusable and Enter/Space activatable.
    expect(trigger.tagName).toBe("BUTTON");
    expect(trigger.tabIndex).toBeGreaterThanOrEqual(0);
    // Dialog is not mounted until the trigger is used.
    expect(screen.queryByRole("dialog")).toBeNull();

    fireEvent.click(trigger);
    // Opened panel has proper dialog semantics AND an accessible name from its title.
    expect(screen.getByRole("dialog", { name: "Suggestion settings" })).toBeInTheDocument();
  });

  it("is keyboard-openable and focus-managed: opening moves focus into the panel and exposes a labelled Close affordance", async () => {
    // NOTE: full dismiss + focus-restore-to-trigger is exercised in the browser
    // E2E (task-6.8-homepage-empty-state) — the Kobalte Popover's presence layer
    // does not tear down under jsdom, so the DOM-teardown half is asserted where
    // a real engine runs. Here we assert the reliably-observable half: the
    // disclosure is a keyboard-activatable button and opening MOVES focus into
    // the dialog panel (focus is managed, not left on the trigger/body).
    render(() => <ConverseEmptyState />);
    const trigger = screen.getByRole("button", { name: "Customize suggestions" });

    trigger.focus();
    expect(trigger).toHaveFocus();
    fireEvent.click(trigger); // the activation a keyboard Enter/Space dispatches on a button

    const dialog = screen.getByRole("dialog", { name: "Suggestion settings" });
    expect(dialog).toBeInTheDocument();
    await waitFor(() => expect(dialog.contains(document.activeElement)).toBe(true));
    // A labelled Close affordance is present so the panel is dismissable by keyboard.
    expect(within(dialog).getByRole("button", { name: "Close" })).toBeInTheDocument();
  });

  it("keeps starters as the primary tab stops that precede the secondary disclosure trigger in DOM/focus order", () => {
    const { container } = render(() => <ConverseEmptyState />);
    const starterList = screen.getByRole("list", { name: "Starter prompts" });
    const firstStarter = within(starterList).getAllByRole("button")[0];
    const trigger = screen.getByRole("button", { name: "Customize suggestions" });

    // Both are focusable buttons (default tab stops, no negative tabindex).
    expect(firstStarter.tabIndex).toBeGreaterThanOrEqual(0);
    expect(trigger.tabIndex).toBeGreaterThanOrEqual(0);

    // The whole starter list is ordered before the disclosure trigger, so
    // starters are reached first when tabbing (primary before secondary).
    const rel = starterList.compareDocumentPosition(trigger);
    expect(rel & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();

    // No focusable control between starters and the disclosure is a negative
    // tab stop that would remove a starter from the tab order.
    const focusables = Array.from(
      container.querySelectorAll<HTMLElement>("button, a[href], [tabindex]"),
    );
    expect(focusables.every((el) => el.tabIndex >= 0)).toBe(true);
  });
});

/**
 * Localization expansion (task 6.8): headings + starter labels + continuation
 * labels must not break layout when a translation renders far longer than the
 * English source. The starter copy is currently English literals in
 * `groundedStarters()`; this asserts that arbitrarily long labels render intact
 * (the CSS wraps rather than overflowing) so a future localization pass can
 * supply long strings safely.
 *
 * FOLLOW-UP: starter/continuation copy should be routed through the i18n
 * catalog (ui/src/locales/*.json) so it localizes with the rest of the shell;
 * the component already renders whatever label it is given without truncation.
 *
 * Validates: Requirements 6.4, 16.4
 */
describe("ConverseEmptyState — localization expansion renders long strings without breaking (task 6.8)", () => {
  beforeEach(resetStores);
  afterEach(cleanup);

  const LONG_LABEL =
    "Stellen Sie eine ausführliche Frage zu einem beliebigen komplexen Thema und erhalten Sie eine gründliche, hilfreiche Antwort von KRIA";
  const LONG_DRAFT = `${LONG_LABEL} — ${LONG_LABEL}`;

  it("renders long localized starter labels intact in the cold/new starter branch", () => {
    render(() => (
      <ConverseEmptyState
        intents={[
          { id: "i1", icon: "zap", label: LONG_LABEL, draft: LONG_DRAFT },
          { id: "i2", icon: "sparkles", label: `${LONG_LABEL} (2)`, draft: LONG_DRAFT },
        ]}
      />
    ));
    // The full (untruncated) label text is present, and the region + heading render.
    expect(screen.getByRole("region", { name: "Start a conversation" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { level: 2 })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: LONG_LABEL })).toHaveTextContent(LONG_LABEL);
    // The disclosure label stays a stable, present control alongside long copy.
    expect(screen.getByRole("button", { name: "Customize suggestions" })).toBeInTheDocument();
  });

  it("renders long localized continuation labels intact in the continuation branch", () => {
    render(() => (
      <ConverseEmptyState
        suggestions={[{ id: "s1", label: LONG_LABEL }]}
        onContinue={() => {}}
      />
    ));
    expect(
      screen.getByRole("heading", { level: 2, name: "Continue where you left off" }),
    ).toBeInTheDocument();
    // Continuation button label is prefixed but still carries the full title.
    expect(screen.getByRole("button", { name: `Continue: ${LONG_LABEL}` })).toHaveTextContent(
      LONG_LABEL,
    );
  });
});


/**
 * Task 10.6 — grounded, READ-ONLY capability disclosure (IU-07; UIE-M-019).
 *
 * The empty state may surface a concise, informational "what KRIA can do" cue
 * for the F6 (tools/MCP) and F7 (OpenClaw skills) capability facts. Those cues
 * MUST be grounded in the AUTHORITATIVE global enabled/available state via the
 * shared `capabilityFieldMap` F6/F7 omission rules:
 *   • not-loaded registry → OMITTED (never a fabricated "ready" cue),
 *   • OpenClaw runtime offline → shown truthfully as "unavailable",
 *   • present state → a read-only deep-link to the capability's existing home.
 * And the disclosure MUST stay strictly read-only: activating a cue performs
 * ONLY navigate/openInspector — never a send, tool invoke, approval, draft send,
 * or staged-review bypass. M5: descriptors read the GLOBAL registries, never a
 * synthesized per-turn availability set.
 *
 * Validates: Requirements 6.6, 8.4, 8.6
 */
describe("ConverseEmptyState — grounded read-only capability disclosure (task 10.6, UIE-M-019)", () => {
  beforeEach(() => {
    resetStores();
    navigate("converse");
    shellStore.setInspectorTarget(null);
  });
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
    navigate("converse");
  });

  it("GROUNDING: with no capabilities loaded, both F6/F7 cues are OMITTED (no fabricated 'ready' state)", () => {
    render(() => <ConverseEmptyState />);
    // No tools registry, no OpenClaw settings → F6 omit, F7 omit → no region.
    expect(screen.queryByRole("list", { name: "Available capabilities" })).toBeNull();
    expect(screen.queryByRole("button", { name: /Tools:/ })).toBeNull();
    expect(screen.queryByText(/Skills unavailable/)).toBeNull();
  });

  it("GROUNDING: an active tools registry surfaces the F6 'Tools' cue as a read-only link", () => {
    capabilityStore.setCapabilities([activeTool("files.read")]);
    render(() => <ConverseEmptyState />);
    const cue = screen.getByRole("button", { name: /^Tools:/ });
    expect(cue).toBeInTheDocument();
    expect(cue.getAttribute("data-fact")).toBe("F6");
    expect(cue.getAttribute("data-outcome")).toBe("show");
    // The accessible name names the read-only destination (Capabilities).
    expect(cue.getAttribute("aria-label")).toContain("Capabilities");
  });

  it("GROUNDING: OpenClaw runtime OFFLINE → F7 'Skills' cue is shown truthfully as UNAVAILABLE, never as ready", () => {
    // An installed+enabled skill exists, but the substrate runtime is inactive.
    capabilityStore.setSkills([enabledSkill("s1")]);
    capabilityStore.setOpenClawSettings(openClawSettings(false));
    render(() => <ConverseEmptyState />);
    // Truthful unavailable: a static, NON-actionable label (not a ready link).
    expect(screen.getByText("Skills unavailable")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /^Skills:/ })).toBeNull();
  });

  it("GROUNDING: OpenClaw runtime ACTIVE + installed skill → F7 'Skills' cue is a read-only link (ready shown truthfully)", () => {
    capabilityStore.setSkills([enabledSkill("s1")]);
    capabilityStore.setOpenClawSettings(openClawSettings(true));
    render(() => <ConverseEmptyState />);
    const cue = screen.getByRole("button", { name: /^Skills:/ });
    expect(cue.getAttribute("data-fact")).toBe("F7");
    expect(cue.getAttribute("data-outcome")).toBe("show");
  });

  it("GROUNDING: runtime ACTIVE but ZERO installed skills → F7 cue OMITTED (empty, not unavailable, not fabricated)", () => {
    capabilityStore.setOpenClawSettings(openClawSettings(true));
    render(() => <ConverseEmptyState />);
    expect(screen.queryByRole("button", { name: /^Skills:/ })).toBeNull();
    expect(screen.queryByText("Skills unavailable")).toBeNull();
  });

  it("M5: capabilityDisclosures reads GLOBAL registries (no per-turn availability set) and applies the field-map rules", () => {
    // Pure grounding read reflects exactly the global capabilityStore state.
    expect(capabilityDisclosures()).toEqual([]);

    capabilityStore.setCapabilities([activeTool("files.read")]);
    capabilityStore.setOpenClawSettings(openClawSettings(false));
    const cues = capabilityDisclosures();
    expect(cues.map((c) => c.factId)).toEqual(["F6", "F7"]);
    expect(cues.find((c) => c.factId === "F6")?.outcome).toBe("show");
    expect(cues.find((c) => c.factId === "F7")?.outcome).toBe("unavailable");
    // Every emitted cue carries a resolved read-only link to an existing Space.
    for (const cue of cues) {
      expect(cue.link).not.toBeNull();
      expect(cue.link!.mode).toBe("navigate");
    }
  });

  it("READ-ONLY: activating an F6 cue only NAVIGATES to Capabilities — no send/tool/approval/draft-send", () => {
    capabilityStore.setCapabilities([activeTool("files.read")]);

    const sendMessage = vi.spyOn(converseStore, "sendMessage");
    const submitIntent = vi.spyOn(converseStore, "submitIntent");
    const selectPlanOption = vi.spyOn(converseStore, "selectPlanOption");
    const updateDraft = vi.spyOn(converseStore, "updateDraft");
    const setApprovalsOpen = vi.spyOn(shellStore, "setApprovalsOpen");

    render(() => <ConverseEmptyState />);
    fireEvent.click(screen.getByRole("button", { name: /^Tools:/ }));

    // ONLY effect: navigation to the capabilities Space (Tools segment owner).
    expect(currentRoute().space).toBe("capabilities");
    // No inspector opened (no fabricated entity id), no approvals seized.
    expect(shellStore.inspectorTarget()).toBeNull();
    expect(setApprovalsOpen).not.toHaveBeenCalled();
    expect(shellStore.approvalsOpen()).toBe(false);
    // No authority-bearing action, and NO composer draft was staged/sent.
    expect(sendMessage).not.toHaveBeenCalled();
    expect(submitIntent).not.toHaveBeenCalled();
    expect(selectPlanOption).not.toHaveBeenCalled();
    expect(updateDraft).not.toHaveBeenCalled();
  });

  it("READ-ONLY: the disclosure never appears in the continuation branch", () => {
    capabilityStore.setCapabilities([activeTool("files.read")]);
    converseStore.setThreads([makeThread("t1", "Daily notes", 3)]);
    render(() => <ConverseEmptyState />);
    // Continuation state → starters + disclosure suppressed; only resumptions.
    expect(screen.getByRole("list", { name: "Continue suggestions" })).toBeInTheDocument();
    expect(screen.queryByRole("list", { name: "Available capabilities" })).toBeNull();
    expect(screen.queryByRole("button", { name: /^Tools:/ })).toBeNull();
  });

  it("INVARIANT PRESERVED: staging a starter draft still only stages (no send), with the disclosure present", () => {
    // Tools available → both an automate starter AND the F6 disclosure render.
    capabilityStore.setCapabilities([activeTool("files.read")]);
    const updateDraft = vi.spyOn(converseStore, "updateDraft");
    const sendMessage = vi.spyOn(converseStore, "sendMessage");
    const setActiveThread = vi.spyOn(converseStore, "setActiveThread");

    render(() => <ConverseEmptyState />);
    // The read-only disclosure coexists with the staging starters.
    expect(screen.getByRole("button", { name: /^Tools:/ })).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Ask a question" }));
    expect(updateDraft).toHaveBeenCalledWith({ text: "What can you help me with?" });
    expect(sendMessage).not.toHaveBeenCalled();
    expect(setActiveThread).not.toHaveBeenCalled();
  });
});

/** A capability that EXISTS but is disabled (not active). */
function disabledTool(id: string): Capability {
  return { ...activeTool(id), status: "disabled" };
}

describe("ConverseEmptyState — capability disclosure bounded/edge cases (task 10.7, UIE-M-019)", () => {
  beforeEach(() => {
    resetStores();
    navigate("converse");
    shellStore.setInspectorTarget(null);
  });
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
    navigate("converse");
  });

  it("DISABLED CAPABILITY: a disabled (non-active) capability is NOT shown as ready — F6 cue omitted, never fabricated", () => {
    // A capability exists in the registry but is disabled → must not count as an
    // available tool (truthful: omitted, not presented as ready).
    capabilityStore.setCapabilities([disabledTool("files.read")]);
    render(() => <ConverseEmptyState />);
    expect(screen.queryByRole("button", { name: /^Tools:/ })).toBeNull();
    expect(screen.queryByRole("list", { name: "Available capabilities" })).toBeNull();

    // Adding an ACTIVE capability alongside flips it to the truthful ready link.
    capabilityStore.setCapabilities([disabledTool("files.read"), activeTool("web.search")]);
    cleanup();
    render(() => <ConverseEmptyState />);
    expect(screen.getByRole("button", { name: /^Tools:/ })).toBeInTheDocument();
  });

  it("BOUNDED: a shown cue carries the shared bounded-text class + a full-value title (long/localized labels never overflow)", () => {
    capabilityStore.setCapabilities([activeTool("files.read")]);
    render(() => <ConverseEmptyState />);
    const cue = screen.getByRole("button", { name: /^Tools:/ });
    expect(cue).toHaveClass("kria-bounded");
    // Full label recoverable on hover (bounded presentation, task 10.7).
    expect(cue).toHaveAttribute("title", "Tools");
  });

  it("BOUNDED: an UNAVAILABLE cue is bounded too and carries its full-value title", () => {
    capabilityStore.setSkills([enabledSkill("s1")]);
    capabilityStore.setOpenClawSettings(openClawSettings(false));
    render(() => <ConverseEmptyState />);
    const label = screen.getByText("Skills unavailable");
    expect(label).toHaveClass("kria-bounded");
    expect(label).toHaveAttribute("title", "Skills unavailable");
  });
});
