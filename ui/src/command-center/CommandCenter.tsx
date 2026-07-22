/**
 * CommandCenter — the full HUD homepage (frontend-only demo).
 *
 * Three-column command layout matching the reference: a pinned SideDock (left),
 * the Core presence flow (centre: Globe + Orbit → Presence → Composer → Chips →
 * Context card), and a StatusRail (right: clock, context overview, active status,
 * focus suggestion, recent activity), framed by a top bar and a bottom bar.
 *
 * Pure presentation with STATIC demo data (`./data`). No backend, no stores, no
 * execution. Mounted by the surface router (`app/SurfaceHost`) as the "home"
 * surface. The dock's Command Deck item opens the Command Deck surface.
 *
 * The Context Engine (`./context`) drives the adaptive bits (Orbit, Presence,
 * Composer, Context Overview); ⌥⇧C cycles context (demo signal), ESC dismisses
 * the one contextual surface.
 */
import { onCleanup, onMount } from "solid-js";
import { CcIcon } from "./CcIcon";
import { Globe } from "./Globe";
import { reducedMotion } from "./parts";
import { PresenceLine } from "./PresenceLine";
import { HomeComposer } from "./HomeComposer";
import { ActionChips } from "./ActionChips";
import { ContextSurface } from "./ContextSurface";
import { SideDock } from "./SideDock";
import { StatusRail } from "./StatusRail";
import { Orbit } from "./Orbit";
import { ContextPanel } from "./ContextPanel";
import { BOTTOM, BRAND } from "./data";
import { activeCapability, closeCapability } from "./homeNav";
import { currentContext, cycleContext } from "./context";
import "./command-center.css";

export function CommandCenter() {
  const isStatic = reducedMotion();

  // ⌥⇧C cycles the demo context; ESC dismisses the contextual surface.
  onMount(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.altKey && e.shiftKey && e.key.toLowerCase() === "c") {
        e.preventDefault();
        cycleContext();
        return;
      }
      if (e.key === "Escape" && activeCapability()) {
        e.preventDefault();
        closeCapability();
      }
    };
    window.addEventListener("keydown", onKey);
    onCleanup(() => window.removeEventListener("keydown", onKey));
  });

  return (
    <div class="cc cc--full" data-reduced-motion={isStatic ? "on" : "off"} data-region="command-center">
      {/* Cinematic environment — the compressed landscape photo behind the Orb,
          darkened + core-bloomed so it supports the interface (offline asset). */}
      <div class="cc-env" aria-hidden="true" />

      {/* ── Top status bar ─────────────────────────────────────────── */}
      <header class="cc-topbar">
        <div class="cc-topbar__brand">
          <span class="cc-emblem"><CcIcon name="hexlogo" size={22} /></span>
          <div class="cc-brand-text">
            <strong>{BRAND.name}</strong>
            <span>{BRAND.tagline}</span>
          </div>
        </div>
        <label class="cc-search">
          <CcIcon name="search" size={16} />
          <input type="text" placeholder="Search or ask KRIA…" aria-label="Search or ask KRIA" />
          <kbd class="cc-search__kbd">⌘K</kbd>
        </label>
        <button
          type="button"
          class="cc-ctx"
          onClick={cycleContext}
          aria-label={`Current context: ${currentContext().label}. Activate to change context.`}
        >
          <span class="cc-dot cc-dot--active" /> CONTEXT · <b>{currentContext().label.toUpperCase()}</b>
        </button>
        <div class="cc-topbar__right">
          <button type="button" class="cc-icon-btn cc-icon-btn--notify" aria-label="Notifications"><CcIcon name="bell" /></button>
          <button type="button" class="cc-icon-btn" aria-label="Activity"><CcIcon name="waveform" /></button>
          <button type="button" class="cc-icon-btn" aria-label="Appearance"><CcIcon name="sun" /></button>
          <button type="button" class="cc-profile">
            <span class="cc-profile__avatar"><CcIcon name="hexlogo" size={16} /></span>
            <span class="cc-profile__text"><b>Operator</b><span>Commander</span></span>
            <CcIcon name="chevron" size={13} class="cc-profile__chev" />
          </button>
        </div>
      </header>

      {/* ── Left pinned dock ───────────────────────────────────────── */}
      <SideDock />

      {/* ── Centre: the Core presence flow ─────────────────────────── */}
      <main class="cc-content" aria-label="Home">
        <div class="cc-corezone">
          <section class="cc-hero">
            <Globe />
            <div class="cc-hero__label" aria-hidden="true">
              <span class="cc-hero__title">{BRAND.coreTitle}</span>
              <span class="cc-hero__sub">{BRAND.coreSub}</span>
              <span class="cc-hero__rule" />
            </div>
          </section>
          <Orbit />
        </div>
        <div class="cc-presence">
          <PresenceLine />
          <div class="cc-presence__sub">
            <span>{currentContext().sub}</span>
            <span class="cc-focus-badge"><span class="cc-dot cc-dot--online" /> Optimal Focus</span>
          </div>
          <HomeComposer />
          <ActionChips />
          <ContextSurface />
        </div>
      </main>

      {/* ── Right status rail ──────────────────────────────────────── */}
      <StatusRail />

      {/* One contextual surface emerges from the Core (One-Surface Rule). */}
      <ContextPanel />

      {/* ── Bottom bar ─────────────────────────────────────────────── */}
      <footer class="cc-bottombar">
        <div class="cc-bottombar__stats">
          <span class="cc-stat"><span class="cc-stat__k"><CcIcon name="pin" size={13} /> {BOTTOM.location}</span></span>
          <span class="cc-stat"><span class="cc-stat__k"><CcIcon name="cloud" size={13} /> {BOTTOM.weather}</span></span>
          <span class="cc-stat"><span class="cc-stat__k">SYSTEM STATUS · All Systems Operational</span></span>
        </div>
        <button type="button" class="cc-brief"><CcIcon name="brief" size={15} /> Executive Briefing</button>
      </footer>
    </div>
  );
}

export default CommandCenter;
