/**
 * AdaptiveContextSurface (ACS) — the body of the current Focus subject
 * (design.md §6.2, Requirement 8).
 *
 * The ACS is the *second density* of the one Focus subject: the Voice Line is
 * the headline, the ACS is its optional expansion. It renders
 * `homeFocusStore.acs` and nothing else — ONE surface, at ONE fixed location,
 * showing exactly ONE subject. It is pure presentation over a read-model: it
 * NEVER sends, executes a tool, or mutates approval/domain state. Its action
 * and its "more detail" affordance ROUTE (or stage a reviewable draft) ONLY
 * (KRIA runtime-authority invariant, Req 8.2 / 29.3).
 *
 * ── One surface, single subject bound to the Voice Line (Req 8.1 / 8.4) ──────
 * The engine derives the Voice Line and the ACS from the SAME highest-ranked
 * candidate, so `acs.subjectId === voiceLine.subjectId` holds by construction
 * (homeFocusStore Property 2). This component renders a SINGLE `FocusAcs`; it
 * has no code path that shows two subjects at once — the regression alarm
 * "two subjects at once is a defect" (design §6.2) cannot fire here.
 *
 * ── Structure: one title, one line, ≤1 action + route-to-owner (Req 8.2) ─────
 * At most one action verb (`acs.action`) is rendered; deeper detail routes to
 * `acs.ownerRoute`. Both are keyboard-operable controls that route/stage only —
 * the action's `run` is supplied by the engine as a routing/staging callback,
 * and "more detail" calls the typed router's `navigate` (routing only). Never
 * stats, charts, or multiple items (Req 8.3).
 *
 * ── Recede / dissolve when empty — never an empty box (Req 8.3) ──────────────
 * When no subject qualifies (rest) OR reading the frame throws, the ACS
 * DISSOLVES: it is REMOVED from the DOM, never rendered as an empty container.
 * The live surface carries {@link DISSOLVES_WHEN_EMPTY_ATTR} so the resting-calm
 * guardrail (`findEmptyStandingSurfaces`) can prove it never renders empty —
 * and because the surface only mounts when it has a subject (title + line), the
 * guardrail stays clean by construction. Failure → dissolve (design §14).
 *
 * ── Living-glass, fade, min dwell (Req 8.5) ──────────────────────────────────
 * The surface is a single-layer living-glass panel (glass tokens, task 0.1).
 * A subject change cross-dissolves: the outgoing subject becomes a short-lived
 * `aria-hidden` ghost that fades out while the incoming subject fades in; an
 * empty transition fades the surface away (dissolve). Minimum dwell is owned by
 * the engine's dwell stabilizer (`createLiveFocusFrame`, task 3.3) — the same
 * single source of truth the Voice Line uses — so the two densities never
 * disagree; this component owns the fade only. Under reduced motion / the
 * global kill-switch the crossfade collapses to an instant swap (no ghost, no
 * timers) — reduced-motion safe (Req 17.4 / 21.4). Motion is opacity-only.
 *
 * ── Labelled region, once-announce, no focus theft (Req 8.5) ─────────────────
 * The surface is a labelled AT region (`role="region"` + `aria-label`). Its body
 * is a polite, atomic live region (`role="status"`, `aria-live="polite"`,
 * `aria-atomic="true"`): a live region announces only when its content CHANGES
 * and never takes focus, so a subject change is announced exactly once and never
 * steals focus. The outgoing ghost is `aria-hidden`, so AT reads the current
 * subject once. Identical consecutive content is a no-op (no re-announce).
 *
 * Requirements: 8.1, 8.2, 8.3, 8.4, 8.5.
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
import { homeFocusStore, type FocusAcs } from "../../../stores/homeFocusStore";
import { DISSOLVES_WHEN_EMPTY_ATTR } from "./guardrails";
import "./AdaptiveContextSurface.css";

/**
 * Crossfade / dissolve duration for a subject change (ms). Matches the token
 * `--motion-duration-recede` so the CSS transition and the ghost-cleanup timer
 * stay in lock-step. Only used when motion is allowed.
 */
export const ACS_CROSSFADE_MS = 600;

export interface AdaptiveContextSurfaceProps {
  /**
   * Optional explicit source of the current ACS subject. When omitted the
   * component reads the live Focus frame ({@link homeFocusStore}). Injecting
   * this keeps the component deterministic in tests/stories without coupling to
   * the real domain stores.
   */
  acs?: () => FocusAcs | undefined;
  /**
   * Force the static (reduced-motion) rendering. When omitted the component
   * derives it from the global kill-switch + OS `prefers-reduced-motion`
   * (Req 8.5 / 17.4), mirroring `VoiceLine`/`Room`/`CorePresence`.
   */
  reducedMotion?: boolean;
  /**
   * Routing hook for the "more detail" affordance (Req 8.2). Defaults to the
   * typed router's `navigate` (routing ONLY — no send/tool/approval side
   * effect). Overridable for tests to assert routing-only behavior.
   */
  onNavigate?: (route: Route) => void;
  /** Accessible label for the region (Req 8.5). Defaults to "Context". */
  label?: string;
  class?: string;
}

/**
 * Resolve the reduced-motion preference: the global kill-switch
 * (`data-reduced-motion="on"` on the root) wins, then the OS media query.
 * Mirrors `VoiceLine`/`Room` so the whole homepage freezes together.
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

/** Stable identity of a subject for no-consecutive-repeat (subject + content). */
function acsKey(acs: FocusAcs): string {
  return `${acs.subjectId}\u0000${acs.title}\u0000${acs.line}\u0000${acs.action?.label ?? ""}`;
}

export function AdaptiveContextSurface(props: AdaptiveContextSurfaceProps) {
  // Default source: the live, dwell-stabilized Focus frame — the SAME frame the
  // Voice Line reads, so both densities describe one subject (Req 8.4). Created
  // lazily so an injected `acs` accessor never spins up a live frame (and tests
  // stay decoupled from the domain stores). Disposed with the component scope.
  let live: ReturnType<typeof homeFocusStore.createLiveFocusFrame> | undefined;
  const source = (): FocusAcs | undefined => {
    if (props.acs) return props.acs();
    if (!live) {
      live = homeFocusStore.createLiveFocusFrame();
      onCleanup(() => live?.dispose());
    }
    return live.frame().acs;
  };

  // Failure isolation (design §14): if reading the frame throws, dissolve rather
  // than crash the homepage.
  const safeSource = createMemo<FocusAcs | undefined>(() => {
    try {
      return source();
    } catch {
      return undefined;
    }
  });

  const isStatic = (): boolean => props.reducedMotion ?? detectReducedMotion();

  // The currently-shown subject (drives the live region) and the outgoing
  // "ghost" rendered only during a crossfade/dissolve (aria-hidden).
  const [displayed, setDisplayed] = createSignal<FocusAcs | undefined>(untrack(safeSource));
  const [ghost, setGhost] = createSignal<FocusAcs | undefined>(undefined);
  const [transitioning, setTransitioning] = createSignal(false);

  createEffect(() => {
    const next = safeSource();
    const current = untrack(displayed);

    // Rest / empty / failure → dissolve. Animated: fade the outgoing subject out
    // as an aria-hidden ghost, then remove. Static: remove immediately. Either
    // way the live surface (which carries the dissolves-when-empty marker) is
    // gone, so it is NEVER an empty box (Req 8.3).
    if (!next) {
      if (current && !isStatic()) {
        setGhost(current);
        setDisplayed(undefined);
        setTransitioning(true);
        const timer = setTimeout(() => {
          setGhost(undefined);
          setTransitioning(false);
        }, ACS_CROSSFADE_MS);
        onCleanup(() => clearTimeout(timer));
      } else {
        setDisplayed(undefined);
        setGhost(undefined);
        setTransitioning(false);
      }
      return;
    }

    // No-consecutive-repeat / once-announce: identical subject content is a
    // no-op, so the live region never re-announces the same subject.
    if (current && acsKey(current) === acsKey(next)) return;

    // First content, or reduced-motion → instant swap (no ghost).
    if (!current || isStatic()) {
      setDisplayed(next);
      setGhost(undefined);
      setTransitioning(false);
      return;
    }

    // Crossfade between subjects: the outgoing subject becomes an aria-hidden
    // ghost that fades out while the incoming subject fades in. A single timer
    // clears the ghost after the transition; re-registered each run so it can
    // never leak.
    setGhost(current);
    setDisplayed(next);
    setTransitioning(true);
    const timer = setTimeout(() => {
      setGhost(undefined);
      setTransitioning(false);
    }, ACS_CROSSFADE_MS);
    onCleanup(() => clearTimeout(timer));
  });

  const hasContent = (): boolean => displayed() !== undefined || ghost() !== undefined;

  const go = (route: Route): void => {
    // ROUTING ONLY (Req 8.2): navigate to the owning surface. No send, no tool
    // call, no approval mutation.
    const nav = props.onNavigate ?? ((r: Route) => navigate(r.space, r.segment, r.entityId));
    nav(route);
  };

  /** Render the immutable body of a subject (title + line only) for the ghost. */
  const renderGhostBody = (acs: FocusAcs): JSX.Element => (
    <>
      <h3 class="kria-acs__title">{acs.title}</h3>
      <p class="kria-acs__line">{acs.line}</p>
    </>
  );

  return (
    <Show when={hasContent()}>
      <div class={`kria-acs-slot ${props.class ?? ""}`.trim()} data-slot="adaptive-context-surface">
        {/* Outgoing ghost — decoration only, read by nobody (aria-hidden). It
            carries the old subject's content (never empty) and does NOT carry
            the dissolves-when-empty marker, so the guardrail only ever inspects
            the live surface below. */}
        <Show when={ghost()} keyed>
          {(g) => (
            <div class="kria-acs kria-acs--ghost" aria-hidden="true" data-motion={isStatic() ? "static" : "animated"}>
              <div class="kria-acs__body">{renderGhostBody(g)}</div>
            </div>
          )}
        </Show>

        {/* The live surface: a labelled AT region whose body is a polite, atomic
            live region. Mounts ONLY when a subject exists, so it is never an
            empty box; the {DISSOLVES_WHEN_EMPTY_ATTR} marker lets the guardrail
            prove it. `keyed` remounts the body per distinct subject so the
            entrance fade re-runs. */}
        <Show when={displayed()} keyed>
          {(acs) => (
            <section
              class="kria-acs"
              role="region"
              aria-label={props.label ?? "Context"}
              data-region="adaptive-context-surface"
              data-subject-id={acs.subjectId}
              data-motion={isStatic() ? "static" : "animated"}
              data-transitioning={transitioning() ? "true" : "false"}
              {...{ [DISSOLVES_WHEN_EMPTY_ATTR]: "" }}
            >
              <div class="kria-acs__body" role="status" aria-live="polite" aria-atomic="true">
                <h3 class="kria-acs__title">{acs.title}</h3>
                <p class="kria-acs__line">{acs.line}</p>

                <div class="kria-acs__actions">
                  {/* At most ONE action verb (Req 8.2). Its `run` is an engine-
                      supplied routing/staging callback — never a send/execute. */}
                  <Show when={acs.action} keyed>
                    {(action) => (
                      <button
                        type="button"
                        class="kria-acs__action"
                        data-role="acs-action"
                        onClick={() => action.run()}
                      >
                        {action.label}
                      </button>
                    )}
                  </Show>

                  {/* Deeper detail routes to the owning Space (routing only). */}
                  <button
                    type="button"
                    class="kria-acs__detail"
                    data-role="acs-detail"
                    onClick={() => go(acs.ownerRoute)}
                  >
                    More detail
                  </button>
                </div>
              </div>
            </section>
          )}
        </Show>
      </div>
    </Show>
  );
}

export default AdaptiveContextSurface;
