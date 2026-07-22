/**
 * CompanionEmber — the floating, cross-application Companion presence (design
 * §8/§9, Requirements 13.4, 15.1–15.5). Adapts the existing
 * `MiniCompanions`/`detachableSurfaces` companion primitives: it reuses the
 * cheap CSS `CorePresence` glyph as a small always-present ember rather than
 * standing up a new detached-window system.
 *
 * WHAT IT IS (the canonical "Companion" View Mode, reconciled in task 8.1):
 * when the window condenses to Companion mode a small floating **ember** stays
 * present, inheriting the Core emotional state, so KRIA feels like it lives on
 * the desktop while the user works in other apps.
 *
 * ── Inherits Core state, read-only (Req 15.1, guardrails "Never write coreStore")
 * The ember MIRRORS `coreStore.state()` — the sole authority. It only READS the
 * state to render the matching glyph; it never writes `coreStore`. So the ember
 * always reflects the one Core, by construction (correctness property: ember
 * state === coreStore state).
 *
 * ── Cheap 2D (Req 15.2) ──────────────────────────────────────────────────────
 * The glyph is `CorePresence` (CSS/SVG-first, GPU transform/opacity only) — NOT
 * the WebGL 3D scene. Idle cost stays ~0; reduced-motion freezes it to a static
 * frame (CorePresence handles that itself).
 *
 * ── Brightens ONLY for meaningful needs (Req 15.2) ───────────────────────────
 * The ember rests dim. It brightens ONLY when {@link isMeaningfulNeed} holds — a
 * Core attention state (blocked/waiting/error), a pending approval, or a brief
 * pulse when requested work just finished — never for idle chatter. The brighten
 * state is also reflected into `homeStore` (UI state) so the data model stays
 * truthful; `coreStore` is untouched.
 *
 * ── Click-to-talk (Req 15.2) ─────────────────────────────────────────────────
 * The ember is the interactive `CorePresence` (activate → opens voice listening;
 * the same two-interactions contract the homepage Core uses). Keyboard operable.
 *
 * ── Continuous return (Req 15.3) ─────────────────────────────────────────────
 * "Return to KRIA" funnels through `requestWindowMode` (task 8.2) — the ONLY
 * sanctioned mode switch — so the return is continuous with the Core as the
 * continuity anchor and shared state (thread/Core-state/draft/Focus subject)
 * preserved. It restores the mode the user came from (design §9).
 *
 * ── On-by-default opt-out + AT announce (Req 15.4) ───────────────────────────
 * Companion is ON by default with a one-setting opt-out (`companionPreference`,
 * persisted locally); when opted out the ember renders nothing. A polite
 * live region announces mood/need CHANGES to assistive tech. Every control is a
 * real focusable button; nothing is hover/cursor-only.
 *
 * ── Compositor fallback + geometry per mode (Req 13.4 / 15.5) ────────────────
 * On activation it best-effort pins the window always-on-top + shrinks it to the
 * small edge-anchored ember geometry via `companionWindow` (existing Tauri
 * frontend APIs only — no new backend). Where the compositor restricts
 * always-on-top/global positioning it degrades to the guaranteed IN-APP ember
 * (`data-presentation="in-app"`) without breaking. Optional reposition nudges
 * the ember between corners (keyboard operable).
 *
 * Requirements: 13.4, 15.1, 15.2, 15.3, 15.4, 15.5
 */
import { Show, createEffect, createMemo, createSignal, on, onCleanup } from "solid-js";
import { CorePresence, CORE_STATE_LABELS } from "../../../components/CorePresence";
import { IconButton } from "../../../kit";
import { coreStore } from "../../../stores/coreStore";
import { approvalStore } from "../../../stores/approvalStore";
import { homeStore, type EdgeAnchor } from "../../../stores/homeStore";
import { eventBus } from "../../../stores/eventBus";
import { requestWindowMode } from "../../../windowing/modeTransitionCoordinator";
import {
  DEFAULT_EMBER_ANCHOR,
  companionPreference,
  emberAnchorStyle,
  isMeaningfulNeed,
  nextEmberAnchor,
  rememberPriorMode,
  returnViewMode,
} from "./companionEmber";
import {
  activateCompanionWindow,
  repositionCompanionWindow,
  restoreCompanionWindow,
} from "./companionWindow";
import type { CompanionPresentation } from "./companionEmber";
import "./CompanionEmber.css";

/** How long a "work just finished" celebratory brighten lasts (ms). */
export const WORK_FINISHED_BRIGHTEN_MS = 2500;

export interface CompanionEmberProps {
  /**
   * Override whether the ember is active. When omitted it reads
   * `homeStore.companion().active` (Companion View Mode). Injectable for
   * stories/tests without driving the whole mode machine.
   */
  active?: () => boolean;
  /** Override the opt-out preference (defaults to `companionPreference`). */
  enabled?: () => boolean;
  /** Routing hook for the continuous return (defaults to `requestWindowMode`). */
  onReturn?: () => void;
  class?: string;
}

export function CompanionEmber(props: CompanionEmberProps) {
  const active = (): boolean => props.active?.() ?? homeStore.companion().active;
  const enabled = (): boolean => props.enabled?.() ?? companionPreference.enabled();
  const visible = (): boolean => active() && enabled();

  // The ember MIRRORS the authoritative Core state (read-only, Req 15.1).
  const emberState = () => coreStore.state();

  // Anchor corner (design §9 optional reposition). Seeded from homeStore's
  // preserved companion position, defaulting to the bottom-right.
  const anchor = (): EdgeAnchor => homeStore.companion().position ?? DEFAULT_EMBER_ANCHOR;

  // A brief celebratory brighten when requested work just finished (design §9).
  const [workJustFinished, setWorkJustFinished] = createSignal(false);
  let finishTimer: ReturnType<typeof setTimeout> | undefined;
  const pulseWorkFinished = (): void => {
    if (!active()) return; // only the ember celebrates; ignore while in-window
    setWorkJustFinished(true);
    if (finishTimer) clearTimeout(finishTimer);
    finishTimer = setTimeout(() => setWorkJustFinished(false), WORK_FINISHED_BRIGHTEN_MS);
  };

  // Meaningful-need brighten gate (Req 15.2): attention state OR pending
  // approval OR the work-finished pulse — never idle chatter.
  const brightened = createMemo(() =>
    isMeaningfulNeed(emberState(), {
      pendingApproval: approvalStore.hasPending(),
      workJustFinished: workJustFinished(),
    }),
  );

  // Which presentation the compositor actually granted (Req 15.5). Starts
  // in-app (the guaranteed fallback) and upgrades if always-on-top pins.
  const [presentation, setPresentation] = createSignal<CompanionPresentation>("in-app");

  // ── Work-finished signals (design §9 "requested work finished") ────────────
  const offThinking = eventBus.on("converse:thinking-changed", (p) => {
    if (p.thinking === false) pulseWorkFinished();
  });
  const offWorkflow = eventBus.on("automation:workflow-completed", (p) => {
    if (p.success) pulseWorkFinished();
  });
  onCleanup(() => {
    offThinking();
    offWorkflow();
    if (finishTimer) clearTimeout(finishTimer);
  });

  // ── Keep the return-mode memory current: whenever the view mode settles to a
  // non-Companion mode, remember it so a later return restores where we were.
  createEffect(() => rememberPriorMode(homeStore.viewMode()));

  // ── Reflect brighten into the (UI-only) home store so the data model stays
  // truthful (design §13.1 companion.brightened). Never touches coreStore.
  createEffect(() => homeStore.setCompanionBrightened(brightened()));

  // ── Native presentation lifecycle (Req 13.4 / 15.5). On becoming visible,
  // best-effort pin always-on-top + shrink to the ember geometry; on leaving,
  // clear the pin (windowModeManager restores the prior geometry). Guarded and
  // degrading — never breaks when the compositor refuses.
  createEffect(
    on(visible, (isVisible, wasVisible) => {
      if (isVisible && !wasVisible) {
        void activateCompanionWindow(anchor()).then(setPresentation);
      } else if (!isVisible && wasVisible) {
        setPresentation("in-app");
        void restoreCompanionWindow();
      }
    }),
  );
  onCleanup(() => {
    if (visible()) void restoreCompanionWindow();
  });

  // ── AT announcement of mood/need CHANGES (Req 15.4). A polite live region
  // that updates only when the mirrored state or the need changes.
  const announcement = createMemo(() => {
    const label = CORE_STATE_LABELS[emberState()];
    return brightened() ? `${label} — needs your attention` : label;
  });

  const returnToWindow = (): void => {
    if (props.onReturn) return props.onReturn();
    requestWindowMode(returnViewMode());
  };

  const reposition = (): void => {
    const next = nextEmberAnchor(anchor());
    homeStore.setCompanionPosition(next); // preserved anchor (works while companion)
    void repositionCompanionWindow(next);
  };

  return (
    <Show when={visible()}>
      <aside
        class={`kria-companion-ember ${props.class ?? ""}`.trim()}
        data-region="companion-ember"
        data-brightened={brightened() ? "true" : "false"}
        data-presentation={presentation()}
        data-anchor={anchor()}
        aria-label="KRIA Companion"
        style={emberAnchorStyle(anchor())}
      >
        {/* Polite live region: announces mood/need changes to AT (Req 15.4).
            Visually hidden — the ember glyph carries the same meaning visually. */}
        <p class="kria-companion-ember__live" role="status" aria-live="polite">
          {announcement()}
        </p>

        {/* The ember glyph — cheap 2D CorePresence mirroring the Core state,
            interactive for click-to-talk (Req 15.1/15.2). */}
        <div class="kria-companion-ember__glyph">
          <CorePresence state={emberState()} size="md" interactive />
        </div>

        {/* Keyboard-operable controls: continuous return + optional reposition
            (Req 15.3/15.4). Real buttons — never hover/cursor-only. */}
        <div class="kria-companion-ember__controls">
          <IconButton
            icon="corner-up-left"
            label="Return to KRIA"
            size="sm"
            onClick={returnToWindow}
          />
          <IconButton
            icon="move"
            label="Move companion to next corner"
            size="sm"
            onClick={reposition}
          />
        </div>
      </aside>
    </Show>
  );
}

export default CompanionEmber;
