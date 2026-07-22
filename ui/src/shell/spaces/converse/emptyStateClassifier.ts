/**
 * emptyStateClassifier — the single deterministic empty-state classifier (IU-05,
 * UIE-H-005).
 *
 * Produces exactly the four design empty-state classes (design.md §11.6 /
 * Data Models):
 *
 *   • "active"                 — the current thread has messages. Active-thread
 *                                content OUTRANKS unrelated global history.
 *   • "intentional-new-thread" — an explicit new-thread intent is set for the
 *                                active empty thread. This OUTRANKS unrelated
 *                                global history (design §20: "explicit new-thread
 *                                intent outranks unrelated history").
 *   • "continuation"           — no explicit new-thread intent, active thread is
 *                                empty, and usable (non-archived) prior history
 *                                exists to resume.
 *   • "cold-start"             — no active task, no explicit intent, and no
 *                                usable continuation history.
 *
 * The function is PURE and side-effect free so it is trivially testable and can
 * be consumed by presentation (ConverseEmptyState, task 6.4) without owning any
 * state. It reads only authoritative signals passed in by the store; it never
 * infers absent values (design Property 3 "Truthful derived state").
 *
 * Requirements: 6.1, 6.2
 */

/** The four canonical empty-state classes (design.md §11.6). */
export type EmptyStateClass =
  | "cold-start"
  | "intentional-new-thread"
  | "continuation"
  | "active";

/** The minimal thread shape the classifier needs (archival + identity). */
export interface ClassifierThread {
  id: string;
  archived: boolean;
}

/** Authoritative signals the classifier derives from — nothing else. */
export interface EmptyStateInputs {
  /** The currently active thread id (or null when none is active). */
  activeThreadId: string | null;
  /** Whether the active thread currently has any messages. */
  hasMessages: boolean;
  /**
   * The thread id explicitly marked as an Intentional New Thread, or null. Owned
   * and reset by converseStore at documented lifecycle transitions.
   */
  newThreadIntentId: string | null;
  /** All known threads (used only to detect usable continuation history). */
  threads: readonly ClassifierThread[];
}

/**
 * Classify the current empty state deterministically.
 *
 * Rule precedence (highest first) — this ordering is what makes explicit intent
 * and active content outrank unrelated history:
 *   1. Active conversation — the active thread has messages.
 *   2. Intentional New Thread — an explicit intent is set for the active thread.
 *   3. Continuation — usable non-archived prior history exists (other threads).
 *   4. Cold Start — otherwise.
 */
export function classifyEmptyState(input: EmptyStateInputs): EmptyStateClass {
  // 1. Active-thread content outranks everything, including global history.
  if (input.hasMessages) return "active";

  // 2. Explicit Intentional New Thread outranks unrelated history. The intent
  //    must belong to the active thread — a stale intent for a thread we've
  //    switched away from must never leak into the current classification.
  if (
    input.newThreadIntentId !== null &&
    input.newThreadIntentId === input.activeThreadId
  ) {
    return "intentional-new-thread";
  }

  // 3. Continuation: usable prior work exists to resume. The active (empty)
  //    thread itself is not "prior work" — only OTHER non-archived threads
  //    represent resumable continuation material.
  const hasUsableHistory = input.threads.some(
    (thread) => !thread.archived && thread.id !== input.activeThreadId,
  );
  if (hasUsableHistory) return "continuation";

  // 4. No active task, no explicit intent, no usable history.
  return "cold-start";
}
