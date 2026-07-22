/**
 * ContextualOrbit — capability *awareness* as partial, temporary light-points
 * around the Core (design.md §6.4, Requirement 6).
 *
 * The Orbit is KRIA's BODY LANGUAGE — a gesture that says "here is what I can
 * help with right now" — NOT navigation, NOT a launcher, NOT a menu, and NOT a
 * permanent ring (Req 6.3). It renders `homeFocusStore.orbit` (the set of LIT
 * points the Focus engine chose for the current context) and nothing else. Pure
 * presentation over a read-model: an Orbit point NEVER sends, NEVER executes a
 * tool, and NEVER mutates domain/approval state; an actionable point ROUTES
 * ONLY (KRIA runtime-authority invariant, Req 6.4 / 29.3).
 *
 * ── Partial, and appears on engagement / fades on disengage (Req 6.1/6.2) ────
 * The Orbit is absent at rest. It appears only while the homepage is *engaged*
 * (`homeStore.orbitEngaged` — set on composer focus / task start / relevant
 * capability activity, task 0.5) AND there is at least one lit point. When
 * engagement ends it FADES OUT and then unmounts (temporary), so at rest the
 * homepage carries no Orbit DOM at all — resting calm (Req 1.5) holds by
 * construction. Only the LIT points render (the frame already contains only lit
 * points; the component re-filters defensively), so a COMPLETE ring is almost
 * never painted (Req 6.2) — the Orbit is partial by design.
 *
 * ── Routing-only actionable points (Req 6.4) ────────────────────────────────
 * A point with a `route` is exposed as a real, labelled, focusable `<button>`
 * that deep-links to the owning Space via the typed router — routing ONLY, no
 * send/tool/approval side effect. Overridable via `onNavigate` for tests. A
 * point WITHOUT a route is a non-interactive awareness light (a labelled
 * `role="img"`), never a dead button.
 *
 * ── Labelled, meaning never by color/motion alone (Req 6.4/6.6/21.2) ─────────
 * Every point carries its text `label` (its accessible name) plus a decorative,
 * capability-mapped icon (`aria-hidden`). So meaning is never conveyed by color
 * or motion alone, and the SAME labels survive the reduced-motion static-dot
 * fallback (Req 6.6). The group is exposed as a labelled `role="group"` — NOT a
 * `menu`/`menubar`/`navigation` role, because the Orbit is body language, not a
 * menu or a navigation region (Req 6.3).
 *
 * ── Reduced-motion / low-capability → static labelled dots (Req 6.6) ─────────
 * Under the global kill-switch / OS `prefers-reduced-motion` the Orbit degrades
 * to static labelled dots (`data-motion="static"`): no orbit drift, no fade —
 * the SAME points, labels, and routing, painted as plain dots that appear/hide
 * instantly. Styling is token-only (Req 16.2 — zero raw color).
 *
 * ── Exactly ONE capability-awareness system (Req 6.5) ────────────────────────
 * The Orbit SUBSUMES the former "capability sparks" concept entirely: it is the
 * single capability-awareness surface. It marks itself
 * `data-capability-awareness="orbit"` so the guardrail
 * ({@link checkSingleCapabilityAwareness}) can assert there is exactly one such
 * system and that no duplicate/legacy sparks UI exists (Req 6.5).
 *
 * ── Failure / empty → render nothing (design §14) ───────────────────────────
 * When there are no lit points, the homepage is not engaged, or reading the
 * frame throws, the component renders NOTHING — never an empty ring/box.
 *
 * Requirements: 6.1, 6.2, 6.3, 6.4, 6.5, 6.6.
 */
import { For, Show, createEffect, createMemo, createSignal, onCleanup } from "solid-js";

import { Icon } from "../../../components/Icon";
import { navigate, type Route } from "../../router";
import {
  homeFocusStore,
  type OrbitPoint,
  type OrbitCapability,
} from "../../../stores/homeFocusStore";
import { homeStore } from "../../../stores/homeStore";
import { CAPABILITY_AWARENESS_ATTR } from "./guardrails";
import "./ContextualOrbit.css";

/**
 * Fade-out duration when engagement ends (ms). Matches the token
 * `--motion-duration-recede` so the CSS transition and the unmount timer stay
 * in lock-step. Only used when motion is allowed; under reduced motion the
 * Orbit hides instantly (no timer).
 */
export const ORBIT_FADE_MS = 600;

/**
 * Capability → decorative icon. The icon is `aria-hidden` (the text label is
 * the accessible name), so this map only affects the glyph, never meaning
 * (Req 6.4/21.2). Unknown capabilities fall back to a neutral dot.
 */
const CAPABILITY_ICONS: Record<string, string> = {
  memory: "brain",
  automation: "workflow",
  desktop: "monitor",
  local: "cpu",
  approval: "shield-check",
  conversation: "message-circle",
};

function iconFor(capability: OrbitCapability): string {
  return CAPABILITY_ICONS[capability] ?? "circle";
}

export interface ContextualOrbitProps {
  /**
   * Optional explicit source of the current Orbit points. When omitted the
   * component reads the live Focus frame ({@link homeFocusStore}). Injecting
   * this keeps the component deterministic in tests/stories without coupling to
   * the real domain stores.
   */
  orbit?: () => readonly OrbitPoint[];
  /**
   * Whether the homepage is currently engaged (Req 6.1). When omitted the
   * component reads {@link homeStore.orbitEngaged}. The Orbit appears only while
   * engaged and there is ≥1 lit point.
   */
  engaged?: () => boolean;
  /**
   * Routing hook for an actionable point (Req 6.4). Defaults to the typed
   * router's `navigate` (routing ONLY — no send/tool/approval side effect).
   * Overridable for tests to assert routing-only behavior.
   */
  onNavigate?: (route: Route) => void;
  /**
   * Force the static (reduced-motion) rendering. When omitted the component
   * derives it from the global kill-switch + OS `prefers-reduced-motion`
   * (Req 6.6 / 17.4), mirroring `VoiceLine`/`ContextualChips`/`Room`.
   */
  reducedMotion?: boolean;
  class?: string;
}

/**
 * Resolve the reduced-motion preference: the global kill-switch
 * (`data-reduced-motion="on"` on the root) wins, then the OS media query.
 * Mirrors `VoiceLine`/`ContextualChips`/`Room` so the whole homepage freezes
 * together.
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

export function ContextualOrbit(props: ContextualOrbitProps) {
  // Default source: the live, dwell-stabilized Focus frame — the SAME frame the
  // Voice Line / ACS / chips read. Created lazily so an injected `orbit`
  // accessor never spins up a live frame. Disposed with the component scope.
  let live: ReturnType<typeof homeFocusStore.createLiveFocusFrame> | undefined;
  const source = (): readonly OrbitPoint[] => {
    if (props.orbit) return props.orbit();
    if (!live) {
      live = homeFocusStore.createLiveFocusFrame();
      onCleanup(() => live?.dispose());
    }
    return live.frame().orbit;
  };

  // Failure isolation (design §14): if reading the frame throws, render none
  // rather than crash the homepage. Also enforce "only lit points" defensively
  // so an upstream bug can never paint an unlit (or complete) ring (Req 6.2).
  const points = createMemo<readonly OrbitPoint[]>(() => {
    try {
      return (source() ?? []).filter((p) => p.lit);
    } catch {
      return [];
    }
  });

  const isStatic = (): boolean => props.reducedMotion ?? detectReducedMotion();
  const engaged = (): boolean => props.engaged?.() ?? homeStore.orbitEngaged();

  // Appear on engagement, fade on disengage (Req 6.1). `visible` gates whether
  // the Orbit is in the DOM at all; `leaving` drives the fade-out before an
  // engaged→disengaged unmount. At rest (never engaged) nothing mounts, so the
  // resting homepage carries no Orbit DOM.
  const [visible, setVisible] = createSignal(false);
  const [leaving, setLeaving] = createSignal(false);

  createEffect(() => {
    const shouldShow = engaged() && points().length > 0;
    if (shouldShow) {
      setLeaving(false);
      setVisible(true);
      return;
    }
    // Disengaged / no points. Under reduced motion (or if not currently shown)
    // hide instantly — no fade timer. Otherwise play the fade-out, then unmount.
    if (!visible() || isStatic()) {
      setLeaving(false);
      setVisible(false);
      return;
    }
    setLeaving(true);
    const timer = setTimeout(() => {
      setVisible(false);
      setLeaving(false);
    }, ORBIT_FADE_MS);
    onCleanup(() => clearTimeout(timer));
  });

  const activate = (point: OrbitPoint): void => {
    if (!point.route) return; // non-actionable awareness light — never routes.
    const route = point.route;
    const go = props.onNavigate ?? ((r: Route) => navigate(r.space, r.segment, r.entityId));
    go(route);
  };

  return (
    <Show when={visible()}>
      <div
        class={`kria-orbit ${props.class ?? ""}`.trim()}
        data-region="contextual-orbit"
        // Exactly ONE capability-awareness system (Req 6.5): the guardrail
        // asserts there is a single `[data-capability-awareness]` and no
        // legacy sparks UI.
        {...{ [CAPABILITY_AWARENESS_ATTR]: "orbit" }}
        data-motion={isStatic() ? "static" : "animated"}
        data-engaged={leaving() ? "false" : "true"}
        // Body language, NOT a menu / navigation region (Req 6.3): a plain
        // labelled group, never role="menu"/"menubar"/"navigation".
        role="group"
        aria-label="What KRIA can help with right now"
        style={{ "--orbit-count": String(points().length) }}
      >
        <For each={points()}>
          {(point, i) => {
            const actionable = (): boolean => point.route !== undefined;
            // Distribute points around the Core on a shared circle. A single
            // `--orbit-angle` var per point drives the CSS placement; partial
            // sets stay visually balanced without a full ring.
            const angle = (): string => {
              const n = points().length;
              const step = n > 1 ? 300 / (n - 1) : 0; // spread across a 300° arc
              return `${-150 + i() * step}deg`;
            };
            return (
              <Show
                when={actionable()}
                fallback={
                  // Non-actionable awareness light: labelled, non-interactive.
                  <span
                    class="kria-orbit__point"
                    data-role="orbit-point"
                    data-orbit-capability={point.capability}
                    data-actionable="false"
                    role="img"
                    aria-label={point.label}
                    style={{ "--orbit-angle": angle() }}
                  >
                    <Icon class="kria-orbit__icon" name={iconFor(point.capability)} size="body" />
                    <span class="kria-orbit__label" aria-hidden="true">
                      {point.label}
                    </span>
                  </span>
                }
              >
                {/* Actionable point: a real, labelled, focusable control that
                    ROUTES ONLY to the owning Space (Req 6.4). */}
                <button
                  type="button"
                  class="kria-orbit__point"
                  data-role="orbit-point"
                  data-orbit-capability={point.capability}
                  data-actionable="true"
                  aria-label={point.label}
                  title={point.label}
                  style={{ "--orbit-angle": angle() }}
                  onClick={() => activate(point)}
                >
                  <Icon class="kria-orbit__icon" name={iconFor(point.capability)} size="body" />
                  <span class="kria-orbit__label">{point.label}</span>
                </button>
              </Show>
            );
          }}
        </For>
      </div>
    </Show>
  );
}

export default ContextualOrbit;
