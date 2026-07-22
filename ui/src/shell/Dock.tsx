/**
 * Dock — the 7-Space navigation rail (Req 1.2). Selecting an item switches
 * Space in exactly ONE interaction (Req 1.3). Every item is a real <button>,
 * keyboard-operable, focus-visible (kit-focusable), labelled, and marks the
 * active Space with aria-current (Req 17.1/17.2).
 *
 * Beginner-oriented presentation (Req 7.2 / design §12; UIE-H-003) visually
 * emphasizes the primary destination (Converse) and groups the rest —
 * supporting → system → utility — using PRESENTATION ONLY (emphasis class +
 * decorative aria-hidden separators). This grouping MUST NOT reorder the DOM /
 * focus / tab order, add a route level, remove a route, or turn the Dock into
 * onboarding. DOM order stays the canonical `ALL_SPACES` sequence and every
 * item remains a one-click Space switch with aria-current on the active one.
 *
 * Requirements: 1.2, 1.3, 7.2, 7.7, 17.1, 17.2
 */
import { For, Show } from "solid-js";
import { Icon } from "../components/Icon";
import { navigate, currentRoute, ALL_SPACES, type Space } from "./router";
import { SPACE_META } from "./spaces";
import { getTerm, type TermId } from "./terminology";
import "./AppShell.css";

/**
 * Presentation-only hierarchy roles from design.md §12. These classify the
 * canonical Spaces for VISUAL grouping/emphasis and nothing else — they never
 * influence DOM/focus order, route identity, the route grammar, or the
 * one-click switch contract. Converse is primary; memory/automations/
 * capabilities are supporting; machines/observatory are system; settings is
 * utility.
 */
export type DockGroup = "primary" | "supporting" | "system" | "utility";

export const SPACE_GROUP: Record<Space, DockGroup> = {
  converse: "primary",
  memory: "supporting",
  automations: "supporting",
  capabilities: "supporting",
  machines: "system",
  observatory: "system",
  settings: "utility",
};

/**
 * Space → terminology matrix id, for the Spaces that ARE top-level
 * Space_Routes in the matrix (Req 7.3–7.4; UIE-M-017). Concise outcome copy is
 * READ from the task-7.5 terminology matrix (the single source of truth) and
 * surfaced as each Dock button's description — never re-authored here. Only
 * Machines, Observatory, and Memory have "space-route" matrix entries; the
 * remaining Spaces (converse/automations/capabilities/settings) have no
 * space-route entry, so we surface no fabricated outcome for them (Req 7.7).
 */
const SPACE_TERM: Partial<Record<Space, TermId>> = {
  machines: "machines",
  observatory: "observatory",
  memory: "memory",
};

/** Concise outcome for a Space, read from the matrix, or undefined if none. */
export function spaceOutcome(space: Space): string | undefined {
  const id = SPACE_TERM[space];
  return id ? getTerm(id).outcome : undefined;
}

export interface DockProps {
  /**
   * Optional post-navigation hook, fired AFTER `navigate(space)` has already
   * updated the authoritative typed router. Use only for genuinely separate
   * concerns (e.g. analytics/telemetry). It MUST NOT be used to mirror the
   * Space into `shellStore.activeSpace`: the router is the sole route authority
   * (Req 7.10 / design §9, §20.1) and AppShell already mirrors the route into
   * `shellStore.activeSpace` via a derived effect. Writing activeSpace here
   * would establish a second, redundant authority.
   */
  onSelect?: (space: Space) => void;
}

export function Dock(props: DockProps) {
  const activeSpace = () => currentRoute().space;

  const select = (space: Space) => {
    navigate(space);
    props.onSelect?.(space);
  };

  return (
    <nav class="kria-dock" aria-label="Spaces">
      <ul class="kria-dock__list">
        <For each={ALL_SPACES}>
          {(space, index) => {
            const meta = SPACE_META[space];
            const group = SPACE_GROUP[space];
            const isActive = () => activeSpace() === space;
            // Concise outcome distinction read from the terminology matrix
            // (single source of truth). Present as an accessible description
            // (aria-describedby → visually-hidden text) plus a hover/focus
            // tooltip, so the outcome is conveyed even when the label is
            // visually hidden in Mini — WITHOUT changing the accessible
            // NAME (aria-label stays the full Space label) or adding a focus
            // stop. Spaces without a matrix space-route entry get no fabricated
            // copy (Req 7.7).
            const outcome = spaceOutcome(space);
            const descId = outcome ? `kria-dock-desc-${space}` : undefined;
            // A group boundary (never before the first Space) gets a purely
            // decorative divider. It is aria-hidden and non-interactive, so it
            // conveys visual grouping WITHOUT adding a focus stop, a route, or
            // any change to the canonical DOM/focus order of the seven buttons.
            const startsNewGroup =
              index() > 0 && SPACE_GROUP[ALL_SPACES[index() - 1]] !== group;
            return (
              <>
                <Show when={startsNewGroup}>
                  <li class="kria-dock__separator" role="presentation" aria-hidden="true" />
                </Show>
                <li class="kria-dock__item" data-dock-group={group}>
                  <button
                    type="button"
                    class="kria-dock__button kit-focusable kit-transition"
                    classList={{
                      "is-active": isActive(),
                      "kria-dock__button--primary": group === "primary",
                    }}
                    aria-current={isActive() ? "page" : undefined}
                    aria-label={meta.label}
                    aria-describedby={descId}
                    title={outcome ? `${meta.label}: ${outcome}` : meta.label}
                    onClick={() => select(space)}
                  >
                    <Icon name={meta.icon} size={20} />
                    <span class="kria-dock__label">{meta.label}</span>
                    <Show when={outcome}>
                      <span id={descId} class="kit-visually-hidden">
                        {outcome}
                      </span>
                    </Show>
                  </button>
                </li>
              </>
            );
          }}
        </For>
      </ul>
    </nav>
  );
}

export default Dock;
