/**
 * CommandCenter — the embedded HUD homepage.
 *
 * Two-column command layout: the Core presence flow (centre: Globe + Orbit →
 * Composer → Cards) and StatusRail (right), framed by a contextual strip. The
 * shared shell owns global navigation, modes, approvals, and notifications.
 *
 * Pure presentation with STATIC demo data (`./data`). No backend, no stores, no
 * execution. Mounted by the surface router (`app/SurfaceHost`) as the "home"
 * surface. The dock's Command Deck item opens the Command Deck surface.
 *
 * The Context Engine (`./context`) drives the adaptive bits (Orbit, Presence,
 * Composer, Context Overview); ⌥⇧C cycles context (demo signal), ESC dismisses
 * the one contextual surface.
 */
import { Show, createSignal, onCleanup, onMount } from "solid-js";
import { CcIcon } from "./CcIcon";
import { Globe } from "./Globe";
import { reducedMotion, useClock } from "./parts";
import { HomeComposer } from "./HomeComposer";
import { ActionChips } from "./ActionChips";
import { ActiveContextCard, NowCard, SystemReadinessCard, WorkstreamCard } from "./CenterCards";
import { StatusRail } from "./StatusRail";
import { Orbit } from "./Orbit";
import { ContextPanel } from "./ContextPanel";
import { BRAND } from "./data";
import { activeCapability, closeCapability } from "./homeNav";
import { shellStore } from "../stores";
import {
  coreState,
  currentCognition,
  currentContext,
  cycleContext,
  setActiveIntent,
  setCoreState,
} from "./context";
import "./command-center.css";

export function CommandCenter() {
  const isStatic = reducedMotion();
  const clock = useClock();
  const [coreFocused, setCoreFocused] = createSignal(false);
  const cognitionTimers: number[] = [];

  const clearCognitionTimers = () => {
    while (cognitionTimers.length) window.clearTimeout(cognitionTimers.pop());
  };
  const focusCommand = () => document.getElementById("cc-command-input")?.focus();
  const focusCoreToggle = () => document.querySelector<HTMLButtonElement>(".cc-core-focus-trigger")?.focus();
  const greeting = () => {
    clock.time();
    const hour = new Date().getHours();
    if (hour < 12) return "Good morning";
    if (hour < 18) return "Good afternoon";
    return "Good evening";
  };
  const toggleCoreFocus = () => {
    const next = !coreFocused();
    setCoreFocused(next);
    queueMicrotask(next ? focusCommand : focusCoreToggle);
  };
  const restoreCoreCards = () => {
    setCoreFocused(false);
    queueMicrotask(focusCoreToggle);
  };

  const handleIntent = (value: string) => {
    clearCognitionTimers();
    setActiveIntent(value);
    setCoreState("thinking");
    cognitionTimers.push(window.setTimeout(() => setCoreState("retrieving"), 650));
    cognitionTimers.push(window.setTimeout(() => setCoreState("executing"), 1350));
    cognitionTimers.push(window.setTimeout(() => setCoreState("idle"), 2600));
  };

  const handleListening = (active: boolean) => {
    clearCognitionTimers();
    setCoreState(active ? "listening" : "idle");
  };

  onMount(() => {
    const onKey = (event: KeyboardEvent) => {
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "k") {
        event.preventDefault();
        focusCommand();
        return;
      }
      if (event.altKey && event.shiftKey && event.key.toLowerCase() === "c") {
        event.preventDefault();
        cycleContext();
        return;
      }
      if (event.key === "Escape") {
        if (activeCapability()) {
          event.preventDefault();
          closeCapability();
          return;
        }
        if (coreState() === "listening") {
          event.preventDefault();
          setCoreState("idle");
          return;
        }
        if (coreFocused()) {
          event.preventDefault();
          restoreCoreCards();
        }
      }
    };
    // Capture ensures this overlay layer consumes Escape before the shell-level
    // Immersive exit listener, independent of component mount order.
    window.addEventListener("keydown", onKey, { capture: true });
    onCleanup(() => {
      window.removeEventListener("keydown", onKey, { capture: true });
      clearCognitionTimers();
    });
  });

  return (
    <div
      class="cc cc--full"
      data-core-state={coreState()}
      data-core-focus={coreFocused() ? "on" : "off"}
      data-view-mode={shellStore.windowMode()}
      data-reduced-motion={isStatic ? "on" : "off"}
      data-region="command-center"
    >
      <div class="cc-env" aria-hidden="true" />

      <header class="cc-topbar" aria-label="KRIA command strip">
        <div class="cc-os-status">
          <span class="cc-emblem"><CcIcon name="hexlogo" size={19} /></span>
          <span class="cc-os-status__copy"><b>{BRAND.name}</b><small><i />{currentCognition().stateLabel}</small></span>
        </div>

        <button type="button" class="cc-os-context" onClick={cycleContext} aria-label={`Current context: ${currentContext().label}. Activate to change context.`}>
          <span class="cc-os-greeting">{greeting()}</span>
          <span class="cc-os-path"><b>KRIA Homepage</b><CcIcon name="chevron" size={10} /><span>{currentContext().label}</span></span>
        </button>

        <div class="cc-topbar__right">
          <time class="cc-os-date" datetime={new Date().toISOString()}>{clock.date()}</time>
        </div>
      </header>

      <main id="space-root" class="cc-content cc-home-main" tabindex={-1} aria-label="Home">
        <div class="cc-corezone" data-state={coreState()}>
          <div class="cc-corefield" aria-hidden="true">
            <i class="cc-corefield__ring cc-corefield__ring--one" />
            <i class="cc-corefield__ring cc-corefield__ring--two" />
            <i class="cc-corefield__wave" />
            <span /><span /><span /><span /><span /><span /><span /><span />
          </div>
          <section class="cc-hero" aria-label={`KRIA Core: ${currentCognition().stateLabel}`}>
            <Globe />
            <div class="cc-hero__label" aria-hidden="true">
              <span class="cc-hero__title">{BRAND.coreTitle}</span>
              <span class="cc-hero__state"><i />{currentCognition().stateLabel}</span>
              <span class="cc-hero__insight">{coreState() === "idle" ? currentContext().objective : currentCognition().activity}</span>
            </div>
          </section>
          <Show when={shellStore.windowMode() !== "mini"}>
            <button
              type="button"
              class="cc-core-focus-trigger"
              aria-label={coreFocused() ? "Show Core cards" : "Hide Core cards and focus the Orb"}
              aria-pressed={coreFocused()}
              title={coreFocused() ? "Show cards · Esc" : "Focus Core"}
              onClick={toggleCoreFocus}
            >
              <span><CcIcon name="focus" size={13} />{coreFocused() ? "Show cards" : "Focus Core"}</span>
            </button>
          </Show>
          <Orbit />
        </div>

        <div class="cc-cognition-thread" data-state={coreState()} aria-hidden="true"><span /><i /><span /></div>
        <NowCard onIntent={handleIntent} />

        <section class="cc-command-zone" aria-label="Command KRIA">
          <HomeComposer state={coreState()} onIntent={handleIntent} onListeningChange={handleListening} />
          <ActionChips onSelect={handleIntent} />
        </section>

        <ActiveContextCard onIntent={handleIntent} />

        <div class="cc-home-lower">
          <WorkstreamCard onIntent={handleIntent} />
          <SystemReadinessCard />
        </div>
      </main>

      <StatusRail onIntent={handleIntent} />
      <ContextPanel />
    </div>
  );
}

export default CommandCenter;
