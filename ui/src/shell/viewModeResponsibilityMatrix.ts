/**
 * View Mode Responsibility Matrix — the single, typed, TOTAL source of truth for
 * exactly what the homepage shows / hides / persists in each canonical View Mode
 * (design.md §29). It is the formal companion to:
 *
 *   • `windowModePolicy.ts` — the Space×mode *content composition* contract
 *     (which layout a Space renders in each window mode); this file is the
 *     homepage *element responsibility* contract (which presence elements are
 *     shown/hidden/conditional in each mode, and what state persists).
 *   • `homeStore.sharedContext` + `modeTransitionCoordinator` — the shared-state
 *     preservation mechanism (task 8.2). This file declares WHAT must persist
 *     (design §29 "Persists across all modes"); the coordinator + store are the
 *     mechanism that makes it hold by construction. The property tests bind the
 *     two together: for every mode-pair transition the declared persistent state
 *     survives unchanged.
 *
 * The canonical View Modes (task 8.1; Req 13.1) are Immersive / Standard / Mini /
 * Companion. Every homepage element has a defined presence in every mode — there
 * is no undefined cell (the matrix is total, enforced by property test).
 *
 * ── design.md §29 (verbatim mapping) ─────────────────────────────────────────
 *
 *   | Element        | Immersive          | Standard           | Mini              | Companion            |
 *   | Core           | full, central      | full               | small, present    | ember                |
 *   | Room           | full               | full               | minimal           | none (ember only)    |
 *   | Voice Line     | yes                | yes                | Voice Line only   | on-brighten only     |
 *   | Composer       | yes (focal)        | yes                | yes (compact)     | mini on click        |
 *   | Chips          | yes                | yes                | hidden (palette)  | no                   |
 *   | ACS            | yes                | yes                | hidden            | no                   |
 *   | Orbit          | yes                | yes                | hidden            | no                   |
 *   | Navigation Rail| icon rail, expandable| compact, expandable | icon-only         | no                   |
 *   | Notifications  | suppressed→Focus   | suppressed→Focus   | critical only     | ember brighten       |
 *   | Conversation   | full               | full               | scrollable compact| opens Mini/Standard  |
 *
 *   Persists across all modes: active thread, Core emotional state, Composer
 *   draft, current Focus subject.
 *
 * ── Authority invariant (Req 29 / 30.3, guardrails.md "Never") ───────────────
 * This is a pure, static declaration. It performs no orchestration, writes no
 * store, and never touches `coreStore`. Consumers READ it to decide rendering.
 *
 * Requirements: 13.1, 13.2, 13.3, 13.5
 */
import type { HomeViewMode } from "../stores/homeStore";

/** The canonical View Mode axis (design §8 / Req 13.1). Ordered as in §29. */
export const VIEW_MODES: readonly HomeViewMode[] = [
  "immersive",
  "standard",
  "mini",
  "companion",
] as const;

/**
 * The homepage elements whose per-mode responsibility §29 defines. Ordered as in
 * the §29 table so the matrix reads 1:1 against the design.
 */
export type HomeElement =
  | "core"
  | "room"
  | "voiceLine"
  | "composer"
  | "chips"
  | "acs"
  | "orbit"
  | "navigationRail"
  | "notifications"
  | "conversation";

export const HOME_ELEMENTS: readonly HomeElement[] = [
  "core",
  "room",
  "voiceLine",
  "composer",
  "chips",
  "acs",
  "orbit",
  "navigationRail",
  "notifications",
  "conversation",
] as const;

/**
 * How an element is present in a mode:
 *   • `shown`       — part of the mode's resting composition (renders when its
 *                     own content qualifies; e.g. the Voice Line still rests to
 *                     silence, but it IS part of Standard's composition).
 *   • `hidden`      — not part of the composition at all in this mode.
 *   • `conditional` — never present at rest; appears ONLY on the named trigger
 *                     (e.g. Companion Composer "on click").
 */
export type ElementPresence = "shown" | "hidden" | "conditional";

/** The full responsibility of one element in one mode. */
export interface ElementResponsibility {
  /** Whether/when the element is present in this mode. */
  readonly presence: ElementPresence;
  /**
   * The trigger that surfaces a `conditional` element. Present iff
   * `presence === "conditional"` (enforced by property test). Omitted otherwise.
   */
  readonly trigger?: string;
  /** The §29 phrase for this cell — the human-readable responsibility. */
  readonly detail: string;
}

// Small builders keep the table below terse while guaranteeing the
// trigger/presence coupling the property test asserts.
const shown = (detail: string): ElementResponsibility => ({ presence: "shown", detail });
const hidden = (detail: string): ElementResponsibility => ({ presence: "hidden", detail });
const cond = (trigger: string, detail: string): ElementResponsibility => ({
  presence: "conditional",
  trigger,
  detail,
});

/**
 * The §29 matrix, transcribed cell-for-cell. `Record<mode, Record<element, …>>`
 * so `MATRIX[mode][element]` is always defined for the canonical axes (the type
 * makes a missing cell a compile error; the property test makes it a test error
 * for the runtime shape too).
 */
export const VIEW_MODE_RESPONSIBILITY_MATRIX: Readonly<
  Record<HomeViewMode, Readonly<Record<HomeElement, ElementResponsibility>>>
> = {
  immersive: {
    core: shown("full, central"),
    room: shown("full"),
    voiceLine: shown("yes"),
    composer: shown("yes (focal)"),
    chips: shown("yes"),
    acs: shown("yes"),
    orbit: shown("yes"),
    navigationRail: shown("icon rail, expandable"),
    notifications: shown("suppressed→Focus"),
    conversation: shown("full"),
  },
  standard: {
    core: shown("full"),
    room: shown("full"),
    voiceLine: shown("yes"),
    composer: shown("yes"),
    chips: shown("yes"),
    acs: shown("yes"),
    orbit: shown("yes"),
    navigationRail: shown("compact, expandable"),
    notifications: shown("suppressed→Focus"),
    conversation: shown("full"),
  },
  mini: {
    core: shown("small, present"),
    room: shown("minimal"),
    voiceLine: shown("Voice Line only"),
    composer: shown("yes (compact)"),
    chips: hidden("hidden (palette)"),
    acs: hidden("hidden"),
    orbit: hidden("hidden"),
    navigationRail: shown("icon-only"),
    notifications: cond("critical", "critical only"),
    conversation: shown("scrollable compact"),
  },
  companion: {
    core: shown("ember"),
    room: hidden("none (ember only)"),
    voiceLine: cond("brighten", "on-brighten only"),
    composer: cond("click", "mini on click"),
    chips: hidden("no"),
    acs: hidden("no"),
    orbit: hidden("no"),
    navigationRail: hidden("no"),
    notifications: cond("brighten", "ember brighten"),
    conversation: cond("open", "opens Mini/Standard"),
  },
} as const;

/**
 * Shared state that PERSISTS across every mode (design §29 "Persists across all
 * modes"). These keys correspond 1:1 to `HomeSharedContext` in `homeStore`; the
 * property tests assert each survives every mode-pair transition unchanged.
 */
export const PERSISTENT_SHARED_STATE = [
  "thread", // active conversation thread  → HomeSharedContext.threadId
  "coreState", // Core emotional state        → HomeSharedContext.coreState
  "draft", // Composer draft              → HomeSharedContext.draft
  "focusSubject", // current Focus subject       → HomeSharedContext.focusSubjectId
] as const;

export type PersistentSharedState = (typeof PERSISTENT_SHARED_STATE)[number];

/** The full responsibility of `element` in `mode` (always defined — total). */
export function elementResponsibility(
  element: HomeElement,
  mode: HomeViewMode,
): ElementResponsibility {
  return VIEW_MODE_RESPONSIBILITY_MATRIX[mode][element];
}

/**
 * Whether `element` is part of `mode`'s resting composition — i.e. it may render
 * when its own content qualifies. `conditional` elements are NOT visible at rest
 * (they need their trigger), and `hidden` elements are never present. This is the
 * predicate homepage surfaces use to gate per-mode rendering against §29.
 */
export function isElementVisibleAtRest(element: HomeElement, mode: HomeViewMode): boolean {
  return elementResponsibility(element, mode).presence === "shown";
}
