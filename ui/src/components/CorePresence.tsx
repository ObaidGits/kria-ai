/**
 * CorePresence — the KRIA Core (Req 3). One living presence that expresses what
 * KRIA is doing via **breath** (subtle scale/opacity pulse), **density** (how
 * solid/bright the orb reads), **temperature** (a warmer/cooler shift kept
 * inside the accent family, with semantic hues reserved for attention states),
 * and **light** (an ambient glow/aura) — never a generic spinner (Req 3.2).
 *
 * Rendering is CSS/SVG-first (design.md §1.7): three compositor-cheap layers
 * (aura, attention ring, body) whose motion is driven entirely by CSS keyframes
 * parameterized per `data-core-state`. No JS animation loop runs, so idle cost
 * stays near zero (Req 16.1). A shader layer was evaluated and is NOT needed —
 * CSS transform/opacity + a blurred radial gradient achieve the living aura
 * within budget.
 *
 * Reduced-motion (Req 3.5 / 16.3 / 17.4): the Core renders a STATIC settled
 * frame. This honors BOTH the OS `prefers-reduced-motion` media query AND the
 * global kill-switch (`data-reduced-motion="on"` on the document root, set by
 * platform/boot + task 14.1). The static state is reflected in `data-motion`
 * for tooling/tests; CSS also freezes the animation independently as defense in
 * depth.
 *
 * Accessibility: `role="img"` + a human-readable `aria-label` per state
 * (Req 17.2/17.3 — meaning never by color/motion alone). Decorative layers are
 * `aria-hidden`.
 *
 * Pure presentation by default: reads `coreStore` only. No orchestration, no
 * side effects, no tool calls (KRIA runtime-authority invariant).
 *
 * ── Optional interactivity (task 2.2, Req 2.3 / 2.4) ─────────────────────────
 * When (and ONLY when) `interactive` is set, the Core becomes operable with
 * EXACTLY TWO interactions, both about *talking* (Req 2.3 / design §4.2):
 *
 *   1. **Activate** — a quick click / Enter / Space *tap*: opens voice listening
 *      (via `voiceStore` + the existing optional `start_voice` command) and
 *      focus-readies the Composer (fires `onRequestComposerFocus`, which the
 *      host — HomeSpace — wires to `presenceIntent.setComposerFocused(true)` so
 *      the Core leans toward the Composer). The actual Composer is owned by a
 *      later task, so the Core only *requests* focus via the callback.
 *   2. **Press-and-hold** — a sustained pointer/key press past a short hold
 *      threshold: push-to-talk. Listening begins when the hold engages and the
 *      turn is SENT on release (via the existing optional `voice_ptt_release`
 *      command + `onPushToTalkSend`). A cancelled hold (pointer leaves / cancel)
 *      stands down WITHOUT sending.
 *
 * These are the ONLY two handlers. The Core has NO navigation, launcher, menu,
 * settings, drag, or widget behavior and NO `href`/`role=menu`/`aria-haspopup`
 * (Req 2.4, L1/L2). There is NO cursor tracking (Req 2.5) — nothing listens to
 * pointer *move*; reactions fire on discrete press/tap intent only.
 *
 * Accessibility pattern (documented, task 2.2): a non-interactive Core is
 * `role="img"` (a labelled presence indicator). An INTERACTIVE Core is a custom
 * `role="button"` with `tabindex=0` and keyboard operation (Enter/Space), since
 * its purpose becomes "activate to talk". It KEEPS the same per-state
 * descriptive `aria-label` (Req 2.7 / 21.2 — the emotional state stays a text
 * equivalent, never conveyed by motion/color alone); `role="button"` conveys
 * that it is actionable. A synthetic AT `click` (some screen readers activate a
 * custom button via click rather than key events) is handled as an Activate.
 *
 * Requirements: 2.3, 2.4, 3.2, 3.5, 16.3
 */
import { createSignal, onCleanup, onMount, splitProps } from "solid-js";
import { coreStore, voiceStore } from "../stores";
import type { CoreState } from "../stores/coreStore";
import { bridgeInvokeOptional } from "../bridge/invoke";
import "./CorePresence.css";

/** Named sizes for the common placements; a raw px number is also accepted. */
export type CoreSize = "sm" | "md" | "lg";

const SIZE_PX: Readonly<Record<CoreSize, number>> = { sm: 24, md: 32, lg: 48 };

/**
 * How long a press must be held (ms) before it counts as push-to-talk rather
 * than a tap-activate. Short enough to feel intentional, long enough that a
 * normal click/tap never accidentally engages PTT. Overridable for tests.
 */
export const CORE_HOLD_THRESHOLD_MS = 250;

/**
 * Human-readable state descriptions for the accessible name. Meaning is carried
 * by text, not by the visual treatment (Req 17.3).
 */
export const CORE_STATE_LABELS: Readonly<Record<CoreState, string>> = {
  idle: "KRIA is idle",
  listening: "KRIA is listening",
  thinking: "KRIA is thinking",
  planning: "KRIA is planning",
  speaking: "KRIA is speaking",
  responding: "KRIA is responding",
  acting: "KRIA is acting",
  "running-automation": "KRIA is running an automation",
  watching: "KRIA is watching",
  remembering: "KRIA is remembering",
  reflecting: "KRIA is reflecting",
  learning: "KRIA is learning",
  waiting: "KRIA is waiting",
  blocked: "KRIA is blocked and needs your approval",
  error: "KRIA encountered an error",
  recovering: "KRIA is recovering",
};

export interface CorePresenceProps {
  /**
   * State to render. Defaults to the live `coreStore.state()`. An explicit value
   * lets stories/tests/detached surfaces render a specific state without
   * mutating the global store.
   */
  state?: CoreState;
  /** Size: a named tier (sm/md/lg) or an explicit px number. Defaults to "md". */
  size?: CoreSize | number;
  /** Override the accessible label (rarely needed). */
  label?: string;
  /**
   * Force the static (reduced-motion) rendering. When omitted the component
   * derives it from the global kill-switch + OS preference.
   */
  reducedMotion?: boolean;
  /**
   * Enable the two talking interactions (activate + press-hold). Defaults to
   * `false` so stories, the PresenceBar glyph, and other decorative placements
   * stay a non-interactive `role="img"` presence indicator (Req 2.3 scopes the
   * interactions to the homepage Core). When true the Core becomes a keyboard-
   * operable `role="button"`.
   */
  interactive?: boolean;
  /**
   * Activate (tap) callback — fired AFTER voice listening is opened. The host
   * uses this to focus-ready the Composer and drive `presenceIntent` lean
   * (Req 2.3, design §4.2). Kept as a callback because the Composer is owned by
   * a later task; the Core never reaches into it directly.
   */
  onRequestComposerFocus?: () => void;
  /** Optional alias fired alongside {@link onRequestComposerFocus} on activate. */
  onActivate?: () => void;
  /** Fired when a press-and-hold engages (push-to-talk begins). */
  onPushToTalkStart?: () => void;
  /** Fired when a press-and-hold is released (push-to-talk sends the turn). */
  onPushToTalkSend?: () => void;
  /** Hold threshold in ms (defaults to {@link CORE_HOLD_THRESHOLD_MS}). */
  holdThresholdMs?: number;
  class?: string;
}

/**
 * Resolve the reduced-motion preference: the global kill-switch
 * (`data-reduced-motion="on"` on the root) wins, then the OS media query.
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

export function CorePresence(props: CorePresenceProps) {
  const [local] = splitProps(props, [
    "state",
    "size",
    "label",
    "reducedMotion",
    "interactive",
    "onRequestComposerFocus",
    "onActivate",
    "onPushToTalkStart",
    "onPushToTalkSend",
    "holdThresholdMs",
    "class",
  ]);

  const state = (): CoreState => local.state ?? coreStore.state();

  const [detected, setDetected] = createSignal(detectReducedMotion());

  // Track live changes to the OS preference and the global kill-switch so the
  // Core freezes/unfreezes immediately (only when the caller hasn't forced it).
  onMount(() => {
    if (local.reducedMotion !== undefined) return;
    setDetected(detectReducedMotion());

    let mql: MediaQueryList | undefined;
    const onChange = () => setDetected(detectReducedMotion());
    if (typeof window !== "undefined" && typeof window.matchMedia === "function") {
      try {
        mql = window.matchMedia("(prefers-reduced-motion: reduce)");
        mql.addEventListener("change", onChange);
      } catch {
        mql = undefined;
      }
    }

    let observer: MutationObserver | undefined;
    if (typeof MutationObserver !== "undefined" && typeof document !== "undefined") {
      observer = new MutationObserver(onChange);
      observer.observe(document.documentElement, {
        attributes: true,
        attributeFilter: ["data-reduced-motion"],
      });
    }

    onCleanup(() => {
      mql?.removeEventListener("change", onChange);
      observer?.disconnect();
    });
  });

  const isStatic = (): boolean => local.reducedMotion ?? detected();

  const sizePx = (): number => {
    const s = local.size ?? "md";
    return typeof s === "number" ? s : SIZE_PX[s];
  };

  const label = (): string => local.label ?? CORE_STATE_LABELS[state()];

  const interactive = (): boolean => local.interactive ?? false;
  const holdMs = (): number => local.holdThresholdMs ?? CORE_HOLD_THRESHOLD_MS;

  // ── The two talking interactions (Req 2.3) ────────────────────────────────
  // A single press is either a TAP (→ activate) or a HOLD (→ push-to-talk),
  // discriminated by a short threshold timer. NO other behavior exists here
  // (Req 2.4). Nothing tracks pointer movement (Req 2.5).
  let holdTimer: ReturnType<typeof setTimeout> | undefined;
  let pressing = false; // a press is in progress
  let holding = false; // the hold threshold was crossed → PTT engaged
  let suppressClick = false; // a pointer press handled activation; ignore the trailing click

  const clearHoldTimer = (): void => {
    if (holdTimer !== undefined) {
      clearTimeout(holdTimer);
      holdTimer = undefined;
    }
  };

  /** Open voice listening — the honest path (store + existing optional cmd). */
  const openVoiceListening = (): void => {
    voiceStore.activate();
    void bridgeInvokeOptional("start_voice");
  };

  /** Activate (tap): open voice listening + focus-ready the Composer. */
  const activate = (): void => {
    openVoiceListening();
    local.onActivate?.();
    local.onRequestComposerFocus?.();
  };

  const beginPress = (): void => {
    if (!interactive() || pressing) return;
    pressing = true;
    holding = false;
    clearHoldTimer();
    holdTimer = setTimeout(() => {
      // Threshold crossed → this press is a push-to-talk HOLD. Begin listening.
      holdTimer = undefined;
      holding = true;
      voiceStore.setPttActive(true);
      openVoiceListening();
      local.onPushToTalkStart?.();
    }, holdMs());
  };

  const endPress = (): void => {
    if (!interactive() || !pressing) return;
    pressing = false;
    clearHoldTimer();
    if (holding) {
      // Push-to-talk release → send the turn on release (Req 2.3).
      holding = false;
      voiceStore.setPttActive(false);
      void bridgeInvokeOptional("voice_ptt_release");
      local.onPushToTalkSend?.();
    } else {
      // Quick tap → activate.
      activate();
    }
  };

  /** Pointer left / press cancelled → stand down WITHOUT sending. */
  const cancelPress = (): void => {
    if (!pressing) return;
    pressing = false;
    clearHoldTimer();
    if (holding) {
      holding = false;
      voiceStore.setPttActive(false);
      void bridgeInvokeOptional("stop_voice");
    }
  };

  const onPointerDown = (event: PointerEvent): void => {
    if (!interactive()) return;
    // Primary pointer only; ignore secondary/aux (context-menu) buttons. A null
    // button (synthetic/environment without full PointerEvent) counts as primary.
    if (event.button != null && event.button !== 0) return;
    suppressClick = true; // this pointer press owns activation; ignore trailing click
    beginPress();
  };
  const onPointerUp = (): void => {
    if (interactive()) endPress();
  };
  const onPointerLeave = (): void => {
    if (interactive()) cancelPress();
  };
  const onPointerCancel = (): void => {
    if (interactive()) cancelPress();
  };

  const isActivationKey = (key: string): boolean =>
    key === "Enter" || key === " " || key === "Spacebar";

  const onKeyDown = (event: KeyboardEvent): void => {
    if (!interactive() || !isActivationKey(event.key)) return;
    event.preventDefault(); // no Space page-scroll / default action
    if (event.repeat) return; // ignore auto-repeat; the hold timer tracks duration
    beginPress();
  };
  const onKeyUp = (event: KeyboardEvent): void => {
    if (!interactive() || !isActivationKey(event.key)) return;
    event.preventDefault();
    endPress();
  };

  /**
   * Click handler exists ONLY so screen readers that activate a custom button
   * via a synthetic `click` (rather than key events) can Activate. A physical
   * pointer press already handled activation in {@link onPointerDown}/
   * {@link onPointerUp}, so its trailing click is suppressed here.
   */
  const onClick = (): void => {
    if (!interactive()) return;
    if (suppressClick) {
      suppressClick = false;
      return;
    }
    activate();
  };

  onCleanup(() => clearHoldTimer());

  return (
    <span
      class={`kria-core ${local.class ?? ""}`.trim()}
      role={interactive() ? "button" : "img"}
      aria-label={label()}
      tabindex={interactive() ? 0 : undefined}
      data-core-state={state()}
      data-motion={isStatic() ? "static" : "animated"}
      data-interactive={interactive() ? "true" : undefined}
      style={{ "--core-size": `${sizePx()}px` }}
      onPointerDown={interactive() ? onPointerDown : undefined}
      onPointerUp={interactive() ? onPointerUp : undefined}
      onPointerLeave={interactive() ? onPointerLeave : undefined}
      onPointerCancel={interactive() ? onPointerCancel : undefined}
      onKeyDown={interactive() ? onKeyDown : undefined}
      onKeyUp={interactive() ? onKeyUp : undefined}
      onClick={interactive() ? onClick : undefined}
    >
      <span class="kria-core__aura" aria-hidden="true" />
      <span class="kria-core__ring" aria-hidden="true" />
      <span class="kria-core__body" aria-hidden="true" />
    </span>
  );
}

export default CorePresence;
