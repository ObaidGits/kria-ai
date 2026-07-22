/**
 * emptyStateClassifier — deterministic four-state empty-state classification
 * (task 6.2, IU-05, UIE-H-005).
 *
 * Verifies design §11.6 / §20 rules:
 *   • Active-thread content and explicit Intentional New Thread intent OUTRANK
 *     unrelated global history.
 *   • Continuation only when usable prior (non-archived, non-active) history
 *     exists and no explicit intent.
 *   • Cold Start otherwise.
 *
 * Requirements: 6.1, 6.2
 */
import { describe, it, expect } from "vitest";
import {
  classifyEmptyState,
  type ClassifierThread,
  type EmptyStateClass,
} from "./emptyStateClassifier";

const thread = (id: string, archived = false): ClassifierThread => ({ id, archived });

describe("classifyEmptyState — precedence and four states (Req 6.1)", () => {
  it("Active: active thread has messages, outranking unrelated history", () => {
    expect(
      classifyEmptyState({
        activeThreadId: "t-active",
        hasMessages: true,
        newThreadIntentId: null,
        threads: [thread("t-active"), thread("old-1"), thread("old-2")],
      }),
    ).toBe("active");
  });

  it("Active outranks a stale intent when messages exist", () => {
    // hasMessages wins even if an intent id somehow still matches.
    expect(
      classifyEmptyState({
        activeThreadId: "t-active",
        hasMessages: true,
        newThreadIntentId: "t-active",
        threads: [thread("t-active")],
      }),
    ).toBe("active");
  });

  it("Intentional New Thread outranks unrelated history (UIE-H-005)", () => {
    expect(
      classifyEmptyState({
        activeThreadId: "t-new",
        hasMessages: false,
        newThreadIntentId: "t-new",
        threads: [thread("t-new"), thread("old-1"), thread("old-2")],
      }),
    ).toBe("intentional-new-thread");
  });

  it("Intentional New Thread with no other history", () => {
    expect(
      classifyEmptyState({
        activeThreadId: "t-new",
        hasMessages: false,
        newThreadIntentId: "t-new",
        threads: [thread("t-new")],
      }),
    ).toBe("intentional-new-thread");
  });

  it("Continuation: usable non-archived prior history, no intent, empty active", () => {
    expect(
      classifyEmptyState({
        activeThreadId: "t-empty",
        hasMessages: false,
        newThreadIntentId: null,
        threads: [thread("t-empty"), thread("old-1")],
      }),
    ).toBe("continuation");
  });

  it("Cold Start: no usable history and no intent", () => {
    expect(
      classifyEmptyState({
        activeThreadId: "t-only",
        hasMessages: false,
        newThreadIntentId: null,
        threads: [thread("t-only")],
      }),
    ).toBe("cold-start");
  });

  it("Cold Start: no threads at all", () => {
    expect(
      classifyEmptyState({
        activeThreadId: null,
        hasMessages: false,
        newThreadIntentId: null,
        threads: [],
      }),
    ).toBe("cold-start");
  });

  it("Cold Start: only archived history is not usable continuation", () => {
    expect(
      classifyEmptyState({
        activeThreadId: "t-empty",
        hasMessages: false,
        newThreadIntentId: null,
        threads: [thread("t-empty"), thread("archived-1", true)],
      }),
    ).toBe("cold-start");
  });

  it("does not leak a stale intent that belongs to a non-active thread", () => {
    // Intent id points at a thread we've switched away from → must not classify
    // as intentional-new-thread; falls through to history-based classification.
    expect(
      classifyEmptyState({
        activeThreadId: "t-current",
        hasMessages: false,
        newThreadIntentId: "t-other",
        threads: [thread("t-current"), thread("t-other")],
      }),
    ).toBe("continuation");
  });

  it("stale intent with no other usable history falls back to Cold Start", () => {
    expect(
      classifyEmptyState({
        activeThreadId: "t-current",
        hasMessages: false,
        newThreadIntentId: "t-other",
        threads: [thread("t-current")],
      }),
    ).toBe("cold-start");
  });
});

describe("classifyEmptyState — exhaustive combination truth table (design P7)", () => {
  // Table-driven coverage over every combination of the authoritative inputs:
  // hasMessages × intent-matches-active × usable-history-present.
  const activeId = "active";

  interface Row {
    hasMessages: boolean;
    intentMatchesActive: boolean;
    usableHistory: boolean;
    expected: EmptyStateClass;
  }

  const rows: Row[] = [];
  for (const hasMessages of [false, true]) {
    for (const intentMatchesActive of [false, true]) {
      for (const usableHistory of [false, true]) {
        // Independent, deterministic expectation derived from the documented
        // precedence: Active > Intentional New Thread > Continuation > Cold.
        const expected: EmptyStateClass = hasMessages
          ? "active"
          : intentMatchesActive
            ? "intentional-new-thread"
            : usableHistory
              ? "continuation"
              : "cold-start";
        rows.push({ hasMessages, intentMatchesActive, usableHistory, expected });
      }
    }
  }

  it.each(rows)(
    "hasMessages=$hasMessages intentMatchesActive=$intentMatchesActive usableHistory=$usableHistory -> $expected",
    ({ hasMessages, intentMatchesActive, usableHistory, expected }) => {
      const threads: ClassifierThread[] = [thread(activeId)];
      if (usableHistory) threads.push(thread("other-usable"));
      // Add archived noise that must never count as usable continuation.
      threads.push(thread("archived-noise", true));

      expect(
        classifyEmptyState({
          activeThreadId: activeId,
          hasMessages,
          newThreadIntentId: intentMatchesActive ? activeId : null,
          threads,
        }),
      ).toBe(expected);
    },
  );
});
