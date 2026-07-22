/**
 * VoiceLine — the Focus headline beneath the Core (design.md §6.1, Requirement 3).
 *
 * One adaptive sentence KRIA "says" beneath the Core. It renders the current
 * `homeFocusStore.voiceLine` subject and nothing else — one line, never a
 * dashboard (L2/L3). Pure presentation over a read-model: it NEVER sends,
 * executes a tool, or touches approval state. An optional deep link ROUTES ONLY
 * (KRIA runtime-authority invariant, Req 3.6 / 29.3).
 *
 * ── Source (Req 3.1) ─────────────────────────────────────────────────────────
 * By default the line comes from the live Focus frame
 * ({@link homeFocusStore.createLiveFocusFrame}) — the engine already applies the
 * fixed precedence, notification suppression, and the anti-flicker DWELL (a
 * subject holds ~6 s before a lower-priority one can replace it), so the Voice
 * Line never fabricates content and never thrashes. Callers/tests may inject an
 * explicit `line` accessor to drive the component deterministically.
 *
 * ── Announce-once, no focus theft (Req 3.5) ─────────────────────────────────
 * The visible line IS a polite live region (`role="status"`, `aria-live=
 * "polite"`, `aria-atomic="true"`). A live region announces only when its text
 * CHANGES, so it never steals focus. The outgoing "ghost" copy used for the
 * crossfade is `aria-hidden`, so AT reads the current subject exactly once.
 *
 * ── No consecutive repeat (Req 3.3) ─────────────────────────────────────────
 * The engine guarantees this via the subject `key`, and the component enforces
 * it again at the presentation layer: identical incoming text is a no-op — the
 * displayed text (and therefore the live-region announcement) does not change,
 * so the same sentence is never announced twice in a row.
 *
 * ── Dwell + crossfade (Req 3.4) ─────────────────────────────────────────────
 * Subject changes never SNAP: the outgoing line fades out (a short-lived
 * `aria-hidden` ghost) while the incoming line fades in. Minimum dwell is owned
 * by the engine's dwell stabilizer; the component owns the crossfade only.
 * Under reduced motion / the global kill-switch the crossfade collapses to an
 * instant, fade-only swap (no ghost, no timers) — reduced-motion safe (Req
 * 17.4 / 21.4). Motion is opacity-only and token-driven.
 *
 * ── Deep link (Req 3.6) ─────────────────────────────────────────────────────
 * When the subject references a navigable owner (`actionable && link`), the
 * line renders a keyboard-operable control that ROUTES ONLY via the typed
 * router — no send/tool/approval side effect. The route target is supplied by
 * the engine (`FocusVoiceLine.link: Route`); activation calls `navigate` only.
 *
 * ── Failure / empty → render nothing (design §14) ───────────────────────────
 * When no subject qualifies (rest) OR reading the frame throws, the component
 * renders NOTHING — never an empty container/box. Silence is a valid premium
 * output (§5.7).
 *
 * Requirements: 3.1, 3.2, 3.3, 3.4, 3.5, 3.6.
 */
import {
  Show,
  createEffect,
  createMemo,
  createSignal,
  onCleanup,
  untrack,
  type JSX,
} from "solid-js";

import { navigate, type Route } from "../../router";
import { homeFocusStore, type FocusVoiceLine } from "../../../stores/homeFocusStore";
import "./VoiceLine.css";

/**
 * Crossfade duration for a subject change (ms). Matches the token
 * `--motion-duration-recede` so the CSS transition and the ghost-cleanup timer
 * stay in lock-step. Only used when motion is allowed.
 */
export const VOICE_LINE_CROSSFADE_MS = 600;

export interface VoiceLineProps {
  /**
   * Optional explicit source of the current Voice Line subject. When omitted
   * the component reads the live Focus frame ({@link homeFocusStore}). Injecting
   * this keeps the component deterministic in tests/stories without coupling to
   * the real domain stores.
   */
  line?: () => FocusVoiceLine | undefined;
  /**
   * Force the static (reduced-motion) rendering. When omitted the component
   * derives it from the global kill-switch + OS `prefers-reduced-motion`
   * (Req 3.4 / 17.4), mirroring `Room`/`CorePresence`.
   */
  reducedMotion?: boolean;
  /**
   * Routing hook for the deep link (Req 3.6). Defaults to the typed router's
   * `navigate` (routing ONLY — no send/tool/approval side effect). Overridable
   * for tests to assert routing-only behavior.
   */
  onNavigate?: (route: Route) => void;
  class?: string;
}

/**
 * Resolve the reduced-motion preference: the global kill-switch
 * (`data-reduced-motion="on"` on the root) wins, then the OS media query.
 * Mirrors `Room`/`CorePresence` so the whole homepage freezes together.
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

export function VoiceLine(props: VoiceLineProps) {
  // Default source: the live, dwell-stabilized Focus frame. Created lazily so
  // an injected `line` accessor never spins up a live frame (and so tests stay
  // decoupled from the domain stores). Disposed with the component scope.
  let live: ReturnType<typeof homeFocusStore.createLiveFocusFrame> | undefined;
  const source = (): FocusVoiceLine | undefined => {
    if (props.line) return props.line();
    if (!live) {
      live = homeFocusStore.createLiveFocusFrame();
      onCleanup(() => live?.dispose());
    }
    return live.frame().voiceLine;
  };

  // Failure isolation (design §14): if reading the frame throws, surface
  // nothing rather than crashing the homepage.
  const safeSource = createMemo<FocusVoiceLine | undefined>(() => {
    try {
      return source();
    } catch {
      return undefined;
    }
  });

  const isStatic = (): boolean => props.reducedMotion ?? detectReducedMotion();

  // The currently-shown line (drives the live region) and the outgoing "ghost"
  // rendered only during a crossfade (aria-hidden).
  const [displayed, setDisplayed] = createSignal<FocusVoiceLine | undefined>(
    untrack(safeSource),
  );
  const [ghost, setGhost] = createSignal<FocusVoiceLine | undefined>(undefined);
  const [transitioning, setTransitioning] = createSignal(false);

  createEffect(() => {
    const next = safeSource();
    const current = untrack(displayed);

    // Rest / empty → show nothing (no ghost, no lingering box).
    if (!next) {
      setDisplayed(undefined);
      setGhost(undefined);
      setTransitioning(false);
      return;
    }

    // No-consecutive-repeat (Req 3.3): identical text is a no-op, so the live
    // region never re-announces the same sentence. (Engine also guarantees this
    // via `key`; the presentation layer enforces it independently.)
    if (current && current.text === next.text) return;

    // First content, or reduced-motion → instant fade-only swap (no ghost).
    if (!current || isStatic()) {
      setDisplayed(next);
      setGhost(undefined);
      setTransitioning(false);
      return;
    }

    // Crossfade: the outgoing line becomes an aria-hidden ghost that fades out
    // while the incoming line fades in. A single timer clears the ghost after
    // the transition; re-registered each run so it can never leak.
    setGhost(current);
    setDisplayed(next);
    setTransitioning(true);
    const timer = setTimeout(() => {
      setGhost(undefined);
      setTransitioning(false);
    }, VOICE_LINE_CROSSFADE_MS);
    onCleanup(() => clearTimeout(timer));
  });

  const hasContent = (): boolean => displayed() !== undefined || ghost() !== undefined;

  /** Render the line body: a routing-only deep link when actionable, else text. */
  const renderBody = (line: FocusVoiceLine): JSX.Element => {
    if (line.actionable && line.link) {
      const route = line.link;
      return (
        <button
          type="button"
          class="kria-voiceline__link"
          data-role="deep-link"
          onClick={() => {
            // ROUTING ONLY (Req 3.6): navigate to the owning surface. No send,
            // no tool call, no approval mutation.
            const go = props.onNavigate ?? ((r: Route) => navigate(r.space, r.segment, r.entityId));
            go(route);
          }}
        >
          {line.text}
        </button>
      );
    }
    return <span class="kria-voiceline__text">{line.text}</span>;
  };

  return (
    <Show when={hasContent()}>
      <div
        class={`kria-voiceline ${props.class ?? ""}`.trim()}
        data-region="voice-line"
        data-motion={isStatic() ? "static" : "animated"}
        data-transitioning={transitioning() ? "true" : "false"}
        data-actionable={displayed()?.actionable ? "true" : "false"}
      >
        {/* Outgoing ghost — decoration only, read by nobody (aria-hidden). */}
        <Show when={ghost()} keyed>
          {(g) => (
            <p class="kria-voiceline__ghost" aria-hidden="true">
              {g.text}
            </p>
          )}
        </Show>

        {/* The live line: a polite, atomic status region. Stable node while the
            component is shown, so a subject change announces exactly once and
            never steals focus (Req 3.5). */}
        <p class="kria-voiceline__line" role="status" aria-live="polite" aria-atomic="true">
          {/* `keyed` remounts the body on each distinct subject so the entrance
              fade re-runs; the surrounding region node stays stable. */}
          <Show when={displayed()} keyed>
            {(line) => renderBody(line)}
          </Show>
        </p>
      </div>
    </Show>
  );
}

export default VoiceLine;
