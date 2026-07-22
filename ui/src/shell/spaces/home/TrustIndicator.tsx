/**
 * TrustIndicator — the quiet on-device / local-first trust affordance
 * (design.md §9 / §19 "Trust", Requirement 9).
 *
 * A small, MUTED confirmation near the Composer that KRIA runs on-device. It is
 * behavior-first (design §9.1), so it earns trust by how it behaves rather than
 * by marketing copy:
 *
 *   • **Stays lit offline** (Req 9.1) — the confirmation remains fully present/
 *     lit whether connectivity is online OR offline. Local-first means offline
 *     is the *normal, healthy* state; it is NEVER rendered as an error or unlit.
 *     Connectivity is consumed from the existing browser/webview signal
 *     (`navigator.onLine` + `online`/`offline` events) — no backend capability.
 *
 *   • **Muted, non-emerald** (Req 9.2) — the cue is a neutral status dot + plain
 *     "On-device" label. It never spends the emerald accent (that is reserved
 *     for the Core; guardrails.md L4) and uses no marketing language.
 *
 *   • **Visible Core→edge reach on desktop action** (Req 9.1) — when the Core is
 *     acting on the computer ({@link isDesktopAction}: `acting` /
 *     `running-automation`), a directed reach cue lights from the indicator
 *     toward the screen edge, reusing the shared-light `--core-*` variables the
 *     Core publishes. It is bounded to the duration of the action (driven by the
 *     authoritative `coreStore` state, not a free-running loop) and degrades to
 *     an instant/static cue under reduced motion.
 *
 *   • **Routes to Settings** (Req 9.3) — activating the indicator routes full
 *     privacy/on-device detail to the "Memory & Privacy" Settings group via the
 *     existing typed router ({@link TRUST_SETTINGS_ROUTE}). Routing ONLY — no
 *     send, no tool call, no setting mutation, no Tauri contract renamed.
 *
 * ── Read-only w.r.t. coreStore (Req 30.3) ────────────────────────────────────
 * Reads `coreStore.state()` and connectivity only; it NEVER writes `coreStore`
 * (guardrail-lint enforces this for the home dir). Pure presentation over the
 * pure projection in {@link ./trustIndicator}.
 *
 * ── Accessibility (Req 21) ───────────────────────────────────────────────────
 * The indicator is a single keyboard-operable, labelled button with visible
 * focus. Meaning is available as TEXT (the "On-device" label + an offline word +
 * a polite live region announcing connectivity/reach changes once) — never by
 * color/motion alone (Req 21.2). No hover/cursor-only affordance. Motion is
 * opacity/transform-only and token-driven; under reduced motion / the global
 * kill-switch the reach collapses to a static cue.
 *
 * Requirements: 9.1, 9.2, 9.3, 21.1, 21.2.
 */
import { Show, createEffect, createMemo, createSignal, onCleanup } from "solid-js";

import { coreStore } from "../../../stores";
import type { CoreState } from "../../../stores/coreStore";
import { navigate, type Route } from "../../router";
import {
  resolveTrustView,
  TRUST_SETTINGS_ROUTE,
  type TrustView,
} from "./trustIndicator";
import "./TrustIndicator.css";

export interface TrustIndicatorProps {
  /**
   * Explicit connectivity source. When omitted the component derives it from
   * the browser/webview `navigator.onLine` + `online`/`offline` events. Injecting
   * this keeps the component deterministic in tests/stories.
   */
  online?: () => boolean;
  /**
   * Explicit Core-state source. Defaults to the live `coreStore.state` reader.
   * Injectable so tests/stories can drive the reach cue without the store.
   */
  coreState?: () => CoreState;
  /** Force static (reduced-motion) rendering; otherwise self-detected. */
  reducedMotion?: boolean;
  /**
   * Routing hook (Req 9.3). Defaults to the typed router's `navigate` to the
   * Memory & Privacy Settings group. Routing ONLY — overridable for tests to
   * assert no side effect beyond navigation.
   */
  onNavigate?: (route: Route) => void;
  class?: string;
}

/** Reduced-motion: the global kill-switch wins, then the OS media query. */
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

/**
 * A reactive connectivity signal over the existing browser/webview online state.
 * Reads `navigator.onLine` and tracks `online`/`offline` events; listeners are
 * torn down with the calling scope. No polling, no backend call — offline is a
 * first-class healthy state (the indicator stays lit regardless).
 */
export function createConnectivitySignal(): () => boolean {
  const initial =
    typeof navigator !== "undefined" && typeof navigator.onLine === "boolean"
      ? navigator.onLine
      : true;
  const [online, setOnline] = createSignal(initial);

  if (typeof window !== "undefined" && typeof window.addEventListener === "function") {
    const goOnline = (): void => {
      setOnline(true);
    };
    const goOffline = (): void => {
      setOnline(false);
    };
    window.addEventListener("online", goOnline);
    window.addEventListener("offline", goOffline);
    onCleanup(() => {
      window.removeEventListener("online", goOnline);
      window.removeEventListener("offline", goOffline);
    });
  }

  return online;
}

export function TrustIndicator(props: TrustIndicatorProps) {
  // Default connectivity source: the live browser/webview online state. Created
  // lazily so an injected `online` accessor never wires window listeners.
  let liveOnline: (() => boolean) | undefined;
  const online = (): boolean => {
    if (props.online) return props.online();
    if (!liveOnline) liveOnline = createConnectivitySignal();
    return liveOnline();
  };

  const coreState = (): CoreState => (props.coreState ?? coreStore.state)();
  const isStatic = (): boolean => props.reducedMotion ?? detectReducedMotion();

  // The resolved view. Failure-isolated (design §14): a throw must never crash
  // the homepage — fall back to the lit, muted, no-reach resting confirmation.
  const view = createMemo<TrustView>(() => {
    try {
      return resolveTrustView({ online: online(), coreState: coreState() });
    } catch {
      return {
        lit: true,
        tone: "muted",
        reach: false,
        connectivity: "online",
        label: "On-device",
        detail: "Running locally, on your device. Open Settings for privacy detail.",
      };
    }
  });

  // Polite, atomic live region text so connectivity/reach CHANGES are announced
  // once as text (Req 21.2) without stealing focus. Only updates on change.
  const [announcement, setAnnouncement] = createSignal("");
  createEffect(() => {
    const v = view();
    setAnnouncement(v.detail);
  });

  const goToSettings = (): void => {
    const route = TRUST_SETTINGS_ROUTE;
    (props.onNavigate ?? ((r: Route) => navigate(r.space, r.segment, r.entityId)))(route);
  };

  return (
    <div
      class={`kria-trust ${props.class ?? ""}`.trim()}
      data-region="trust-indicator"
      data-tone={view().tone}
      data-lit={view().lit ? "true" : "false"}
      data-reach={view().reach ? "true" : "false"}
      data-connectivity={view().connectivity}
      data-motion={isStatic() ? "static" : "animated"}
    >
      <button
        type="button"
        class="kria-trust__button"
        data-role="trust"
        aria-label={`${view().label}. ${view().detail}`}
        onClick={goToSettings}
      >
        {/* The muted, always-lit status dot. Decorative — meaning is carried by
            the visible label + live region, never by this dot's color alone. */}
        <span class="kria-trust__dot" aria-hidden="true" />

        {/* Directed Core→edge reach cue — visible only while acting on the
            device (Req 9.1). Decorative (announced via the live region); reuses
            the published `--core-*` shared-light so it points from the Core. */}
        <span class="kria-trust__reach" aria-hidden="true" />

        {/* Glanceable label — plain, non-marketing (Req 9.2). */}
        <span class="kria-trust__label">{view().label}</span>

        {/* Offline is healthy, not an error: a calm secondary word, still lit. */}
        <Show when={view().connectivity === "offline"}>
          <span class="kria-trust__offline">Offline</span>
        </Show>
      </button>

      {/* Meaning-as-text: announce connectivity/reach changes once, no focus
          theft (Req 21.2). Visually hidden; the button label carries the glance. */}
      <p class="kria-trust__sr" role="status" aria-live="polite" aria-atomic="true">
        {announcement()}
      </p>
    </div>
  );
}

export default TrustIndicator;
