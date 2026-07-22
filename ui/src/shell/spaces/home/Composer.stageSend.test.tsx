/**
 * Composer (homepage) — stage-not-send, per-thread draft persistence, Send⇄Stop,
 * and keyboard operability (task 5.2, Req 4.4 / 4.5 / 4.6).
 *
 * These tests pin the runtime-authority invariant and the Composer contract that
 * task 5.2 owns, complementing Composer.test.tsx (which covers 4.1/4.2/4.3):
 *
 *   • Stage-not-send (Req 4.4): the entry points that place content — Contextual
 *     Chips today, and the same staging contract that starters/Orbit reuse —
 *     STAGE a reviewable draft (converseStore.updateDraft) or ROUTE only; they
 *     NEVER call a send/execute path. A guardrail source-scan asserts the chip
 *     entry point references no send/execute API.
 *   • Per-thread draft persistence (Req 4.5): switching threads restores that
 *     thread's draft; the home / no-thread draft persists across a switch too.
 *   • Send⇄Stop (Req 4.4): the single primary action is Send while idle and
 *     becomes a prominent Stop while a turn is active (coreStore.isActive());
 *     Stop halts the active turn via the existing cancellation.
 *   • Keyboard-operable + visible focus + labelled (Req 4.6): input, mic,
 *     send/stop, and the ⌘K hint are real focusable, labelled controls.
 */
import { describe, it, expect, afterEach, beforeEach, vi } from "vitest";
import { render, cleanup, fireEvent } from "@solidjs/testing-library";

import HomeComposer from "./Composer";
import ConverseComposer from "../converse/Composer";
import ContextualChips from "./ContextualChips";
import chipsSource from "./ContextualChips.tsx?raw";
import { converseStore, HOME_DRAFT_KEY } from "../../../stores/converseStore";
import { coreStore } from "../../../stores/coreStore";
import type { Chip } from "../../../stores/homeFocusStore";

function resetDraftState(): void {
  // Land on the home surface with a clean draft before each test.
  converseStore.setActiveThread(null);
  converseStore.updateDraft({ text: "", attachments: [], mode: "assistant" });
  coreStore.reset();
  if (typeof window !== "undefined") window.localStorage.clear();
}

beforeEach(resetDraftState);
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  coreStore.reset();
});

// ── Stage-not-send (Req 4.4) ─────────────────────────────────────────────────

describe("stage-not-send — chips/starters/orbit stage or route, never send (Req 4.4)", () => {
  const stageChip: Chip = {
    id: "stage-1",
    label: "Resume draft",
    icon: "message-square",
    kind: "stage",
    payload: "Draft: weekly report for the team.",
  };
  const routeChip: Chip = {
    id: "route-1",
    label: "Review",
    icon: "shield-alert",
    kind: "route",
    payload: { space: "converse" } as Chip["payload"],
  };

  it("a stage chip stages a reviewable draft into the shared draft — no send (Req 4.4)", () => {
    const send = vi.spyOn(converseStore, "sendMessage");
    const submit = vi.spyOn(converseStore, "submitIntent");

    const { getByText } = render(() => <ContextualChips chips={() => [stageChip]} />);
    fireEvent.click(getByText("Resume draft"));

    // Staged into the SAME per-thread draft the Composer reads (reviewable).
    expect(converseStore.composerDraft().text).toBe("Draft: weekly report for the team.");
    // The runtime-authority invariant: staging never sends/executes.
    expect(send).not.toHaveBeenCalled();
    expect(submit).not.toHaveBeenCalled();
    expect(converseStore.thinking()).toBe(false);
  });

  it("the staged draft appears in the homepage Composer input for review (Req 4.4)", () => {
    render(() => <ContextualChips chips={() => [stageChip]} />);
    const { container } = render(() => <HomeComposer />);
    const textarea = container.querySelector<HTMLTextAreaElement>("textarea")!;

    fireEvent.click(document.querySelector('[data-chip-kind="stage"]') as HTMLElement);
    expect(textarea.value).toBe("Draft: weekly report for the team.");
  });

  it("a route chip routes only — no draft mutation, no send (Req 4.4)", () => {
    const send = vi.spyOn(converseStore, "sendMessage");
    const onNavigate = vi.fn();

    const { getByText } = render(() => (
      <ContextualChips chips={() => [routeChip]} onNavigate={onNavigate} />
    ));
    fireEvent.click(getByText("Review"));

    expect(onNavigate).toHaveBeenCalledTimes(1);
    expect(onNavigate).toHaveBeenCalledWith(routeChip.payload);
    expect(send).not.toHaveBeenCalled();
    expect(converseStore.composerDraft().text).toBe("");
  });

  it("guardrail: the chip staging entry point references no send/execute API", () => {
    // Static assertion — the staging entry point (reused by starters/Orbit) must
    // stage (updateDraft) or route (navigate) only, never reach a send path.
    expect(chipsSource).toContain("updateDraft");
    for (const forbidden of [
      "sendMessage",
      "submitIntent",
      "send_message",
      "send_lab_message",
      "stopTurn",
    ]) {
      expect(chipsSource).not.toContain(forbidden);
    }
  });
});

// ── Per-thread draft persistence (Req 4.5) ───────────────────────────────────

describe("per-thread draft persistence — switch restores each draft (Req 4.5)", () => {
  it("restores each thread's draft across switches", () => {
    converseStore.setActiveThread("thread-A");
    converseStore.updateDraft({ text: "A draft", mode: "lab" });

    converseStore.setActiveThread("thread-B");
    expect(converseStore.composerDraft().text).toBe(""); // new thread is clean
    converseStore.updateDraft({ text: "B draft" });

    converseStore.setActiveThread("thread-A");
    expect(converseStore.composerDraft().text).toBe("A draft");
    expect(converseStore.composerDraft().mode).toBe("lab");

    converseStore.setActiveThread("thread-B");
    expect(converseStore.composerDraft().text).toBe("B draft");
  });

  it("the home / no-thread draft persists across a thread switch and back (Req 4.5)", () => {
    // On the home surface (no active thread) stage a draft.
    converseStore.setActiveThread(null);
    converseStore.updateDraft({ text: "home draft" });
    expect(converseStore.composerDraft().text).toBe("home draft");

    // Switch into a thread — the thread starts clean, home draft is preserved.
    converseStore.setActiveThread("thread-Z");
    expect(converseStore.composerDraft().text).toBe("");

    // Return home — the home draft is restored, not lost.
    converseStore.setActiveThread(null);
    expect(converseStore.composerDraft().text).toBe("home draft");
  });

  it("keys the home draft under HOME_DRAFT_KEY and persists to localStorage (Req 4.5)", async () => {
    converseStore.setActiveThread(null);
    converseStore.updateDraft({ text: "persisted home draft" });

    // Wait out the debounced write.
    await new Promise((r) => setTimeout(r, 260));
    const raw = window.localStorage.getItem("kria.converse.drafts");
    expect(raw).not.toBeNull();
    const parsed = JSON.parse(raw!) as Record<string, { text: string }>;
    expect(parsed[HOME_DRAFT_KEY]?.text).toBe("persisted home draft");
  });
});

// ── Send ⇄ Stop (Req 4.4) ────────────────────────────────────────────────────

describe("Send ⇄ Stop — one primary action toggles on turn-active (Req 4.4)", () => {
  it("shows Send while idle and a prominent Stop while a turn is active", () => {
    const { queryByLabelText } = render(() => <HomeComposer />);

    // Idle: Send present, Stop absent.
    expect(queryByLabelText("Send message")).not.toBeNull();
    expect(queryByLabelText("Stop response")).toBeNull();

    // Turn active (Core doing something) → Stop replaces Send.
    coreStore.setState("responding");
    expect(queryByLabelText("Send message")).toBeNull();
    const stop = queryByLabelText("Stop response");
    expect(stop).not.toBeNull();
    expect(stop?.classList.contains("kria-composer__stop")).toBe(true);
  });

  it("Stop halts the active turn via the existing cancellation (Req 4.4)", () => {
    const onStop = vi.fn();
    const { getByLabelText } = render(() => <ConverseComposer onStop={onStop} />);

    coreStore.setState("responding");
    fireEvent.click(getByLabelText("Stop response"));
    expect(onStop).toHaveBeenCalledTimes(1);
  });

  it("Stop wires to converseStore.stopTurn by default", () => {
    const stopTurn = vi.spyOn(converseStore, "stopTurn").mockResolvedValue();
    const { getByLabelText } = render(() => <ConverseComposer />);

    coreStore.setState("responding");
    fireEvent.click(getByLabelText("Stop response"));
    expect(stopTurn).toHaveBeenCalledTimes(1);
  });
});

// ── Keyboard-operable + visible focus + labelled (Req 4.6) ───────────────────

describe("keyboard-operable + labelled controls (Req 4.6)", () => {
  it("input, mic, send, and the ⌘K hint are real focusable, labelled controls", () => {
    // A non-empty draft so Send is enabled (a disabled control is not focusable).
    converseStore.updateDraft({ text: "hello" });
    const { container, getByLabelText } = render(() => <HomeComposer />);

    const textarea = getByLabelText("Message KRIA") as HTMLTextAreaElement;
    expect(textarea.tagName).toBe("TEXTAREA");

    const mic = getByLabelText("Start voice input");
    expect(mic.tagName).toBe("BUTTON");

    const send = getByLabelText("Send message");
    expect(send.tagName).toBe("BUTTON");

    const hint = container.querySelector<HTMLButtonElement>('[data-role="palette-hint"]')!;
    expect(hint.tagName).toBe("BUTTON");
    expect(hint.getAttribute("aria-keyshortcuts")).toBe("Meta+K Control+K");

    // Each control is genuinely keyboard-focusable (no tabindex=-1 trap).
    for (const el of [textarea, mic, send, hint]) {
      expect(el.getAttribute("tabindex")).not.toBe("-1");
      el.focus();
      expect(document.activeElement).toBe(el);
    }
  });

  it("the Stop control is keyboard-focusable and labelled while active (Req 4.6)", () => {
    const { getByLabelText } = render(() => <HomeComposer />);
    coreStore.setState("responding");

    const stop = getByLabelText("Stop response");
    expect(stop.tagName).toBe("BUTTON");
    expect(stop.getAttribute("tabindex")).not.toBe("-1");
    stop.focus();
    expect(document.activeElement).toBe(stop);
  });
});
