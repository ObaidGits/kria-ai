/**
 * ContextualChips — ≤3 live next-action affordances beneath the Composer
 * (design.md §6.3, Requirement 5).
 *
 * The chips are KRIA offering "what you might do next" — never a menu, never a
 * launcher. They render `homeFocusStore.chips` (already ≤3 and adaptively
 * ranked by the Focus engine) and nothing else. Pure presentation over a
 * read-model: a chip NEVER sends, NEVER executes a tool, and NEVER mutates
 * approval/domain state (KRIA runtime-authority invariant, Req 5.3 / 29.3).
 *
 * ── Source + the ≤3 cap (Req 5.1) ────────────────────────────────────────────
 * By default the chips come from the live Focus frame
 * ({@link homeFocusStore.createLiveFocusFrame}) — the engine already applies the
 * fixed precedence, notification suppression, and the cap (`chips.length ≤ 3`).
 * The component TRUSTS that contract but also ENFORCES it defensively by slicing
 * to {@link MAX_CHIPS}, so a bug upstream can never paint a fourth chip past the
 * cognitive-load budget (guardrails.md, `MAX_CHIPS`). Callers/tests may inject
 * an explicit `chips` accessor to drive the component deterministically.
 *
 * ── Omit entirely when there is no real action (Req 5.2) ─────────────────────
 * The Focus engine derives chips only from real signals (pending approval,
 * resumable thread/session, imminent event, active capability) and omits them
 * when no real action exists — never generic filler. When the (capped) chip
 * list is empty OR reading the frame throws, this component renders NOTHING (no
 * container, no placeholder row). Silence is a valid premium output (§5.7); the
 * resting-calm guardrail (`checkRestingCalm`) stays clean by construction.
 *
 * ── Stage-a-draft OR route only, never execute/send (Req 5.3) ────────────────
 * Activation depends on the chip `kind`:
 *   • `stage` → the chip's `payload` (draft text) is STAGED into the Composer
 *     draft via {@link converseStore.updateDraft} — the SAME per-thread draft
 *     store the Composer/Send path reads. The user reviews before sending; the
 *     chip never auto-sends (Req 4.4 / 5.3). Overridable via `onStage` (e.g.
 *     before the homepage Composer is wired, or in tests).
 *   • `route` → the typed router navigates to the owning surface (`payload` is a
 *     {@link Route}); routing only, no side effect. Overridable via `onNavigate`.
 * There is deliberately NO code path that calls a send/execute/approve API.
 *
 * ── Labelled icon + text, focus-visible, keyboard-operable (Req 5.4) ─────────
 * Each chip is a real `<button>` (Enter/Space activate natively, visible
 * focus ring) carrying BOTH an icon AND a text label — meaning is never
 * conveyed by icon or color alone (Req 5.4 / 21.2). The icon is decorative
 * (`aria-hidden`) because the adjacent text is the accessible name.
 *
 * ── Token-only, reduced-motion safe (Req 16.2 / 17.4) ────────────────────────
 * Styling is token-only (zero raw color). The only motion is a token-driven
 * hover lift (design §11.2 "chip lift on hover"); under reduced motion / the
 * global kill-switch it is frozen (CSS + `data-motion="static"`).
 *
 * Requirements: 5.1, 5.2, 5.3, 5.4.
 */
import { For, Show, createMemo, onCleanup } from "solid-js";

import { Icon } from "../../../components/Icon";
import { navigate, type Route } from "../../router";
import { converseStore } from "../../../stores/converseStore";
import { homeFocusStore, type Chip, MAX_CHIPS } from "../../../stores/homeFocusStore";
import "./ContextualChips.css";

export interface ContextualChipsProps {
  /**
   * Optional explicit source of the current chips. When omitted the component
   * reads the live Focus frame ({@link homeFocusStore}). Injecting this keeps
   * the component deterministic in tests/stories without coupling to the real
   * domain stores.
   */
  chips?: () => readonly Chip[];
  /**
   * Staging hook for a `stage` chip (Req 5.3). Defaults to staging the payload
   * text into the Composer draft via {@link converseStore.updateDraft} — a
   * REVIEWABLE draft, never an auto-send. Overridable for tests, or to target a
   * different draft target while the homepage Composer is still being built.
   */
  onStage?: (text: string) => void;
  /**
   * Routing hook for a `route` chip (Req 5.3). Defaults to the typed router's
   * `navigate` (routing ONLY — no send/tool/approval side effect). Overridable
   * for tests to assert routing-only behavior.
   */
  onNavigate?: (route: Route) => void;
  /**
   * Force the static (reduced-motion) rendering. When omitted the component
   * derives it from the global kill-switch + OS `prefers-reduced-motion`
   * (Req 17.4 / 21.4), mirroring `VoiceLine`/`Room`/`CorePresence`.
   */
  reducedMotion?: boolean;
  class?: string;
}

/**
 * Resolve the reduced-motion preference: the global kill-switch
 * (`data-reduced-motion="on"` on the root) wins, then the OS media query.
 * Mirrors `VoiceLine`/`AdaptiveContextSurface`/`Room` so the whole homepage
 * freezes together.
 */
function detectReducedMotion(): boolean {
  if (typeof document !== "undefined") {
    const root = document.documentElement;
    if (root && root.getAttribute("data-reduced-motion") === "on") return true;
  }
  if (typeof window !== "undefined" && typeof window.matchMedia === "function") {
    try {
      return window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    } catch {
      return false;
    }
  }
  return false;
}

export function ContextualChips(props: ContextualChipsProps) {
  // Default source: the live, dwell-stabilized Focus frame — the SAME frame the
  // Voice Line + ACS read. Created lazily so an injected `chips` accessor never
  // spins up a live frame (and tests stay decoupled from the domain stores).
  // Disposed with the component scope.
  let live: ReturnType<typeof homeFocusStore.createLiveFocusFrame> | undefined;
  const source = (): readonly Chip[] => {
    if (props.chips) return props.chips();
    if (!live) {
      live = homeFocusStore.createLiveFocusFrame();
      onCleanup(() => live?.dispose());
    }
    return live.frame().chips;
  };

  // Failure isolation (design §14): if reading the frame throws, render none
  // rather than crash the homepage. Also ENFORCE the ≤3 cap defensively so an
  // upstream bug can never exceed the cognitive-load budget (Req 5.1).
  const chips = createMemo<readonly Chip[]>(() => {
    try {
      const list = source() ?? [];
      return list.slice(0, MAX_CHIPS);
    } catch {
      return [];
    }
  });

  const isStatic = (): boolean => props.reducedMotion ?? detectReducedMotion();

  const activate = (chip: Chip): void => {
    if (chip.kind === "stage") {
      // STAGE a reviewable draft (Req 5.3) — never auto-send. `payload` is the
      // draft text; default staging mirrors it into the Composer draft store.
      const stage =
        props.onStage ?? ((text: string) => converseStore.updateDraft({ text }));
      stage(String(chip.payload));
      return;
    }
    // ROUTE ONLY (Req 5.3): navigate to the owning surface. `payload` is a Route.
    const route = chip.payload as Route;
    const go = props.onNavigate ?? ((r: Route) => navigate(r.space, r.segment, r.entityId));
    go(route);
  };

  return (
    // Omit entirely when there is no real action (Req 5.2): no chips → no DOM.
    <Show when={chips().length > 0}>
      <div
        class={`kria-chips ${props.class ?? ""}`.trim()}
        data-region="contextual-chips"
        data-motion={isStatic() ? "static" : "animated"}
        role="list"
        aria-label="Suggested actions"
      >
        <For each={chips()}>
          {(chip) => (
            <button
              type="button"
              class="kria-chip"
              role="listitem"
              data-role="chip"
              data-chip-kind={chip.kind}
              onClick={() => activate(chip)}
            >
              {/* Icon is decorative — the adjacent text is the accessible name,
                  so meaning is never conveyed by icon/color alone (Req 5.4). */}
              <Icon class="kria-chip__icon" name={chip.icon} size="body" />
              <span class="kria-chip__label">{chip.label}</span>
            </button>
          )}
        </For>
      </div>
    </Show>
  );
}

export default ContextualChips;
