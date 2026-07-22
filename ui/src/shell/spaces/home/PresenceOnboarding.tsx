/**
 * PresenceOnboarding — the first-run presence teaching (design.md §17
 * "Onboarding", Requirement 19.1/19.2/19.3).
 *
 * Renders, one calm whisper at a time, the presence model's three one-time
 * teaching moments (Core whisper → Dock hint → Orbit reveal-once-engaged). It is
 * NOT a tour: no progress bar, no forced next/prev, no blocking overlay — just
 * the next in-context one-time cue, each independently dismissible. Once every
 * hint is retired it renders nothing and NEVER returns (design §17).
 *
 * Permanence reuses the EXISTING one-time coach-hint ledger
 * (`adaptive/presentationRanking`: `shouldShowCoachHint` / `retireCoachHint`,
 * persisted in localStorage) — no new persistence, no new backend contract.
 * `shouldShowCoachHint` reads the adaptive revision signal, so retiring a hint
 * reactively removes it here.
 *
 * Pure presentation over `homeStore` (read-only, for the Orbit-engaged context)
 * + the coach ledger. It NEVER sends, executes a tool, mutates approval state,
 * or writes `coreStore` (KRIA runtime-authority invariant / guardrails.md).
 * Token-only styling (zero raw color), reduced-motion safe (CSS freeze), and
 * fully keyboard- and screen-reader-operable (a real button; a polite
 * announce-once live region — meaning is available as text, never motion/color
 * alone).
 *
 * Requirements: 19.1, 19.2, 19.3, 21.1, 21.2, 21.3
 */
import { Show, createMemo } from "solid-js";
import { homeStore } from "../../../stores/homeStore";
import { retireCoachHint, shouldShowCoachHint } from "../../../adaptive/presentationRanking";
import { visibleOnboardingHints, type OnboardingContext } from "./presenceOnboarding";
import "./PresenceOnboarding.css";

export interface PresenceOnboardingProps {
  /**
   * Retired-hint predicate. Defaults to the durable coach-hint ledger
   * (`!shouldShowCoachHint`). Overridable so tests assert visibility logic
   * without touching localStorage.
   */
  isRetired?: (coachId: string) => boolean;
  /**
   * Retire a hint permanently. Defaults to the coach-hint ledger's
   * `retireCoachHint`. Routing-free — retiring only records the one-time flag.
   */
  onRetire?: (coachId: string) => void;
  /**
   * Orbit-engaged context. Defaults to the live `homeStore.orbitEngaged` so the
   * Orbit reveal only appears once the ring lights (never at rest). Overridable
   * for tests / stories.
   */
  orbitEngaged?: () => boolean;
  class?: string;
}

/**
 * The first-run presence onboarding. Shows the single next qualifying hint (or
 * nothing when onboarding is complete). Additive and transient — never a
 * resting placeholder/widget.
 */
export function PresenceOnboarding(props: PresenceOnboardingProps) {
  const isRetired = (coachId: string): boolean =>
    (props.isRetired ?? ((id: string) => !shouldShowCoachHint(id)))(coachId);
  const retire = (coachId: string): void =>
    (props.onRetire ?? retireCoachHint)(coachId);
  const orbitEngaged = (): boolean =>
    (props.orbitEngaged ?? homeStore.orbitEngaged)();

  // The next qualifying hint in canonical order (Core → Orbit → Dock), gated by
  // the one-time ledger + live context. Reactive: retiring a hint bumps the
  // adaptive revision signal, so the current hint is removed and the next (if
  // any) takes its place. Undefined once onboarding is complete → renders none.
  const current = createMemo(() => {
    const ctx: OnboardingContext = { orbitEngaged: orbitEngaged() };
    return visibleOnboardingHints(isRetired, ctx)[0];
  });

  return (
    <Show when={current()}>
      {(hint) => (
        <aside
          class={`kria-onboarding ${props.class ?? ""}`.trim()}
          data-onboarding-hint={hint().id}
          // Polite live region: announces the teaching line once when it
          // appears, without stealing focus (Req 21.3).
          role="status"
          aria-live="polite"
        >
          <p class="kria-onboarding__message">{hint().message}</p>
          <Show when={hint().detail}>
            {(detail) => <p class="kria-onboarding__detail">{detail()}</p>}
          </Show>
          {/* A real, keyboard-operable dismissal. Retiring records the one-time
              flag so this hint never returns (design §17). Routing/teaching
              only — no send, no execute, no coreStore write. */}
          <button
            type="button"
            class="kria-onboarding__dismiss kit-focusable"
            onClick={() => retire(hint().coachId)}
          >
            Got it
          </button>
        </aside>
      )}
    </Show>
  );
}

export default PresenceOnboarding;
