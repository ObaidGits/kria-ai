/**
 * Reading Mode controller (design.md §11, Requirement 11.1–11.4).
 *
 * Reading Mode is the homepage macro state entered when a conversation begins.
 * Its defining move (Req 11.1) is a **depth-recession**, NOT a page-swap or a
 * corner-dock: the Room + Core stay in the same space and recede in depth while
 * the conversation rises forward onto a near-solid, legible reading backing. It
 * **reverses on empty** (Req 11.3): when the thread empties (or the user returns
 * home) the Core floats forward, the Room re-lights, and the transition unwinds.
 *
 * This module owns the PURE decision logic + the thin Solid wiring that keeps
 * the homepage macro state (`homeStore`) in sync with the conversation's
 * message count (`converseStore`). It performs NO orchestration, no sends, and
 * no `coreStore` writes (authority invariant, Req 30.3, guardrail-lint) — it
 * only drives `homeStore` macro-state transitions, which are themselves pure UI
 * state. The visual depth-recession + hard-dim + near-solid backing + settle
 * motion live in `ReadingBackdrop`/CSS; the message stream is REUSED from
 * Converse (never rebuilt), preserving conversation-dominance (Req 11.4).
 *
 * ── Why a pure `resolveReadingSync` ──────────────────────────────────────────
 * The first-send→recede and empty→reverse rules are the correctness core of
 * this task and are property-tested (see `readingMode.test.ts`). Keeping them in
 * a side-effect-free function lets the tests assert the invariants — "first send
 * always recedes, never navigates" and "empty always reverses" — across
 * randomized transition sequences without a DOM.
 *
 * Requirements: 11.1, 11.2, 11.3, 11.4, 30.1
 */
import { createEffect, onCleanup } from "solid-js";
import { homeStore, type HomeState } from "../../../stores/homeStore";
import { converseStore } from "../../../stores/converseStore";

/**
 * The macro-state action the reading sync should apply for a given
 * (hasMessages, homeState) observation. Deterministic and side-effect free.
 *
 *   • `enter` — first send: recede into Reading Mode. `via` records whether the
 *     resting homepage must `engage()` first (from `rest`, since `reading` is
 *     only reachable via `engaged`, Req 11.1 / homeStore transition table) or
 *     can enter directly (already `engaged`).
 *   • `exit`  — reverse on empty: float the Core forward + re-light the Room by
 *     returning to the resting homepage (Req 11.3).
 *   • `none`  — already in sync, or a transient overlay / companion owns the
 *     surface and must not be hijacked.
 */
export type ReadingSyncAction =
  | { kind: "enter"; via: "engage-first" | "direct" }
  | { kind: "exit" }
  | { kind: "none" };

/**
 * Pure decision: given whether the active thread has any messages and the
 * current homepage macro state, what reading-mode transition (if any) keeps the
 * homepage in sync?
 *
 * Invariants (property-tested):
 *   • First send (hasMessages, not yet reading) from a RESTING homepage state
 *     (`rest`/`engaged`) always resolves to `enter` — a depth-recession — and
 *     NEVER any other kind (it never navigates away).
 *   • An empty thread while in `reading` always resolves to `exit` (reverse).
 *   • Transient overlays (`blocked`, `mode-transition`) and `companion` are
 *     never auto-entered/exited here — reading is a stable-state concern only.
 *   • The function is idempotent: once in `reading` with messages, or resting
 *     with none, it returns `none` (no thrash).
 */
export function resolveReadingSync(input: {
  hasMessages: boolean;
  homeState: HomeState;
}): ReadingSyncAction {
  const { hasMessages, homeState } = input;
  const reading = homeState === "reading";

  // Reverse on empty (Req 11.3): the thread emptied while reading → unwind.
  if (reading && !hasMessages) return { kind: "exit" };

  // First send → depth-recession (Req 11.1). Only from a resting homepage
  // state; never hijack a transient overlay (blocked / mode-transition) or the
  // companion ember.
  if (!reading && hasMessages) {
    if (homeState === "engaged") return { kind: "enter", via: "direct" };
    if (homeState === "rest") return { kind: "enter", via: "engage-first" };
  }

  return { kind: "none" };
}

/**
 * Apply a resolved reading-sync action to `homeStore`. Returns `true` when a
 * transition was applied. `enter` from `rest` engages first (so the `reading`
 * transition is legal per the homeStore table), then recedes into `reading`;
 * `exit` returns to the resting homepage (Core forward, Room re-lit — Req 11.3).
 *
 * Side-effecting but tiny + deterministic; separated from {@link resolveReadingSync}
 * so the decision logic stays unit/property-testable in isolation.
 */
export function applyReadingSync(action: ReadingSyncAction): boolean {
  switch (action.kind) {
    case "enter":
      if (action.via === "engage-first") homeStore.engage();
      return homeStore.enterReading();
    case "exit":
      // Reverse the recession all the way back to the resting homepage so the
      // Room comes forward and blooms (Req 11.3), rather than only stepping back
      // to `engaged`.
      return homeStore.rest();
    case "none":
      return false;
  }
}

/**
 * Wire Reading Mode to the live conversation: whenever the active thread gains
 * its first message the homepage recedes into `reading`; whenever it empties the
 * recession reverses. Reactive + idempotent (guarded by {@link resolveReadingSync}),
 * so re-running on unrelated signal churn is a no-op.
 *
 * Mount this ONCE from the surface that owns the presence homepage (ConverseSpace,
 * behind the `home.presence.v2` flag). It reads `converseStore.messages()` and
 * `homeStore.state()` only; it never writes a domain store or `coreStore`.
 *
 * @returns a disposer (also auto-cleaned via `onCleanup` when called inside a
 *          reactive owner).
 */
export function createReadingModeController(): () => void {
  let disposed = false;

  const stop = createEffect(() => {
    if (disposed) return;
    const hasMessages = converseStore.messages().length > 0;
    const homeState = homeStore.state();
    applyReadingSync(resolveReadingSync({ hasMessages, homeState }));
  });

  const dispose = () => {
    disposed = true;
    // `createEffect` has no explicit stop handle; the `disposed` guard makes the
    // effect inert. When mounted inside a component owner it is torn down with
    // the owner, so this guard only matters for imperative/manual disposal.
    void stop;
  };

  onCleanup(dispose);
  return dispose;
}
