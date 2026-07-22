/**
 * coreNarration — pure state→text mapping contract (task 5.4, UIE-H-013,
 * Req 8.5, 8.6; design §8 "Truth before theater", §15).
 *
 * Verifies:
 *   • each MAPPED state (listening/thinking/planning/acting/waiting/blocked/
 *     error/recovering) yields truthful concise text;
 *   • idle and every UNMAPPED / unknown state yield NO text (omitted) — nothing
 *     is fabricated;
 *   • concrete authoritative objects (work label, block reason, error message)
 *     are named and marked actionable; unknown objects are omitted, not invented.
 */
import { describe, it, expect, beforeEach } from "vitest";
import { narrateCoreState, type CoreNarrationInput } from "./coreNarration";
import { setLocale } from "./i18n";
import type { CoreState } from "./coreStore";

beforeEach(() => setLocale("en"));

function input(overrides: Partial<CoreNarrationInput> & { state: CoreState }): CoreNarrationInput {
  return {
    errorMessage: null,
    blockReason: null,
    pendingApprovals: 0,
    activeWorkLabel: null,
    ...overrides,
  };
}

// ─── Mapped states produce truthful concise text ───────────────────────────────

describe("narrateCoreState — mapped states", () => {
  it("listening → concise non-actionable text", () => {
    const n = narrateCoreState(input({ state: "listening" }));
    expect(n).toEqual({ text: "Listening", key: "core_narration_listening", actionable: false });
  });

  it("thinking → concise non-actionable text", () => {
    const n = narrateCoreState(input({ state: "thinking" }));
    expect(n).toEqual({ text: "Thinking", key: "core_narration_thinking", actionable: false });
  });

  it("planning → concise non-actionable text", () => {
    const n = narrateCoreState(input({ state: "planning" }));
    expect(n?.key).toBe("core_narration_planning");
    expect(n?.actionable).toBe(false);
    expect(n?.text.length).toBeGreaterThan(0);
  });

  it("recovering → concise actionable text", () => {
    const n = narrateCoreState(input({ state: "recovering" }));
    expect(n).toEqual({ text: "Recovering", key: "core_narration_recovering", actionable: true });
  });

  it("waiting → concise objectless text (wait reason not authoritative)", () => {
    const n = narrateCoreState(input({ state: "waiting" }));
    expect(n?.key).toBe("core_narration_waiting");
    expect(n?.actionable).toBe(false);
  });
});

// ─── Object naming: present objects named, unknown objects omitted ─────────────

describe("narrateCoreState — acting names the source-owned work object", () => {
  it("names the active work label and marks it actionable", () => {
    const n = narrateCoreState(input({ state: "acting", activeWorkLabel: "Indexing files" }));
    expect(n).toEqual({
      text: "Working on Indexing files",
      key: "core_narration_acting_object",
      actionable: true,
    });
  });

  it("falls back to the objectless phrase when no work label exists (never invents an object)", () => {
    const n = narrateCoreState(input({ state: "acting", activeWorkLabel: null }));
    expect(n).toEqual({ text: "Working", key: "core_narration_acting", actionable: false });
  });

  it("treats a whitespace-only label as absent", () => {
    const n = narrateCoreState(input({ state: "acting", activeWorkLabel: "   " }));
    expect(n?.key).toBe("core_narration_acting");
  });
});

describe("narrateCoreState — blocked points at its approval owner", () => {
  it("uses the source-owned block reason when present (actionable)", () => {
    const n = narrateCoreState(input({ state: "blocked", blockReason: "Delete 3 files" }));
    expect(n).toEqual({
      text: "Waiting for approval: Delete 3 files",
      key: "core_narration_blocked_reason",
      actionable: true,
    });
  });

  it("falls back to the approval-owner phrase when no reason is available", () => {
    const n = narrateCoreState(input({ state: "blocked", pendingApprovals: 2 }));
    expect(n).toEqual({
      text: "Waiting for your approval",
      key: "core_narration_blocked",
      actionable: true,
    });
  });
});

describe("narrateCoreState — error surfaces recovery", () => {
  it("uses the source-owned error message when present", () => {
    const n = narrateCoreState(input({ state: "error", errorMessage: "Model unreachable" }));
    expect(n).toEqual({
      text: "Error: Model unreachable",
      key: "core_narration_error_message",
      actionable: true,
    });
  });

  it("falls back to a truthful generic when no message is available (never invents one)", () => {
    const n = narrateCoreState(input({ state: "error", errorMessage: "  " }));
    expect(n).toEqual({
      text: "Something went wrong",
      key: "core_narration_error",
      actionable: true,
    });
  });
});

// ─── Omission: idle + unmapped/unknown states fabricate nothing ────────────────

describe("narrateCoreState — omitted states (Truth before theater)", () => {
  it("idle yields no narration (fabricates nothing)", () => {
    expect(narrateCoreState(input({ state: "idle" }))).toBeNull();
  });

  it.each<CoreState>([
    "speaking",
    "running-automation",
    "watching",
    "remembering",
    "reflecting",
    "learning",
  ])("unmapped state %s yields no narration", (state) => {
    expect(narrateCoreState(input({ state }))).toBeNull();
  });

  it("an unknown/future object masquerading as a CoreState yields no narration", () => {
    // Simulate an unrecognized state value slipping through: it must be OMITTED,
    // never given fabricated text.
    expect(narrateCoreState(input({ state: "quantum-foo" as unknown as CoreState }))).toBeNull();
  });
});

// ─── Localization: text follows the active locale ──────────────────────────────

describe("narrateCoreState — localized via the existing i18n path", () => {
  it("returns localized text for the active locale", () => {
    setLocale("es");
    const n = narrateCoreState(input({ state: "listening" }));
    expect(n?.text).toBe("Escuchando");
    setLocale("en");
  });
});
