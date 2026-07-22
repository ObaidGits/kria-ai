/**
 * AppShell — the global shell (Req 1.1). Composes the shell regions:
 *   PresenceBar · Dock · SpaceRouter · InspectorHost · StatusLine
 * plus the one-at-a-time ModalHost (Req 1.6).
 *
 * Responsibilities:
 *   • Boot the Tauri bridge on mount, dispose on unmount (task 1.3 wiring).
 *   • Keep the typed router and shellStore in sync: the router is authoritative
 *     for which Space renders (deep-link/restore, task 1.1); shellStore mirrors
 *     the active Space so the event bus + other surfaces observe it (task 1.2).
 *   • Reflect theme / window-mode / density onto the document root for tokenized
 *     styling (design.md §4).
 *   • Persist router session so Space/selection/scroll restore on relaunch.
 *
 * The shell is presentation/routing only — NO orchestration logic lives here
 * (architecture invariant). It consumes stores + the bridge; it never calls
 * kria-core directly.
 *
 * Requirements: 1.1, 1.2, 1.6, 20.4
 */
import { Show, createEffect, createSignal, onMount, onCleanup } from "solid-js";
import {
  shellStore,
  converseStore,
  coreStore,
  voiceStore,
  settingsStore,
  memoryStore,
  automationStore,
  capabilityStore,
  provisioningStore,
  eventBus,
  initCoreTray,
  disposeCoreTray,
} from "../stores";
import { tauriBridge } from "../bridge";
import {
  currentRoute,
  initRouterPersistence,
  initHashSync,
  getRestoredThreadId,
  setSessionThreadId,
  type Space,
} from "./router";
import { PresenceBar } from "./PresenceBar";
import { Dock } from "./Dock";
import { SpaceRouter } from "./SpaceRouter";
import { InspectorHost } from "./InspectorHost";
import { StatusLine } from "./StatusLine";
import { ModalHost } from "./ModalHost";
import { CommandPalette, initPaletteDefaults } from "../palette";
import {
  ApprovalCenter,
  captureApprovalPlace,
  restoreApprovalPlace,
  type ApprovalPlaceSnapshot,
} from "./approvals";
import { NotificationCenter, NotificationAnnouncer } from "./notifications";
import { VoiceSurface } from "./voice";
import { approvalStore } from "../stores";
import { capturePlace, restorePlace, type PlaceSnapshot } from "./placePreservation";
import {
  beginConversationPlace,
  endConversationPlace,
} from "./spaces/converse/conversationPlace";
import { initOverlayInertness, registerOverlaySurface } from "./overlayLayers";
import { initSummon } from "../summon";
import { spaceComposition } from "./windowModePolicy";
import { initWindowPresentation, disposeWindowPresentation } from "../windowing/detachableSurfaces";
import { initWindowModeManager, disposeWindowModeManager } from "../windowing/windowModeManager";
import { CompanionFallbackHost } from "../windowing/MiniCompanions";
import { CompanionEmber } from "./spaces/home/CompanionEmber";
import { SetupExperience } from "./setup/SetupExperience";
import { isFeatureEnabled } from "../featureFlags";
import SurfaceHost from "../app/SurfaceHost";
import "./AppShell.css";

export interface AppShellProps {
  /**
   * Open the Approval Center. Defaults to the built-in overlay
   * (`shellStore.setApprovalsOpen(true)`); pass a custom handler only to
   * override (e.g. focus a detached approvals window, Req 11.4 / task 12.3).
   */
  onOpenApprovals?: () => void;
}

export function AppShell(props: AppShellProps) {
  // Command Center homepage (frontend-only, static demo) — full-screen HUD
  // surface that replaces the standard shell when the flag is ON (default).
  // Early return before any shell effects register so the demo surface is
  // fully self-contained. Flip `home.command-center` OFF (localStorage/env) to
  // restore the normal presence shell.
  if (isFeatureEnabled("home.command-center")) {
    return <SurfaceHost />;
  }

  const [provisioningResolved, setProvisioningResolved] = createSignal(false);

  // Restore the last active Converse thread from the persisted session BEFORE
  // wiring the sync effect, so the restored id is not clobbered by the initial
  // (null) store value (Req 1.4). Threads themselves load async from the bridge;
  // restoring the id here means selection resolves once they arrive.
  const restoredThreadId = getRestoredThreadId();
  if (restoredThreadId) {
    converseStore.setActiveThread(restoredThreadId);
  }

  // Keep the session's active-thread mirror in sync with converseStore so it is
  // persisted and restored on relaunch.
  createEffect(() => {
    setSessionThreadId(converseStore.activeThreadId());
  });

  // Persist router session and expose production deep links through the URL
  // hash. Both are shell-lifetime services; hash sync returns explicit cleanup
  // so hot reload/tests never accumulate route effects or DOM listeners.
  initRouterPersistence();
  const disposeHashSync = initHashSync();
  onCleanup(disposeHashSync);

  // Snapshot transient DOM place before mode composition changes and restore it
  // after the curated shell settles. Domain state is never copied/reset: route,
  // active thread, Inspector selection, and per-thread draft remain authoritative
  // in their existing stores (Req 15.3).
  //
  // The virtualized conversation viewport is DELEGATED to its single anchor
  // owner (design §21 IU-10 / UIE-M-005, task 9.3): `capturePlace` excludes it,
  // and the conversation coordinator (`beginConversationPlace`/
  // `endConversationPlace`) captures its message anchor + offset once and
  // restores it exactly once — even if this mode change coincides with a pending
  // approval (P-B), which delegates to the SAME coordinator below.
  let modePlaceSnapshot: PlaceSnapshot | null = null;
  const stopCapturingModePlace = eventBus.on("shell:mode-changing", () => {
    modePlaceSnapshot = capturePlace();
    beginConversationPlace();
  }, "none");
  const stopRestoringModePlace = eventBus.on("shell:mode-changed", () => {
    const snap = modePlaceSnapshot;
    modePlaceSnapshot = null;
    queueMicrotask(() => {
      restorePlace(snap);
      endConversationPlace();
    });
  }, "none");
  onCleanup(() => {
    stopCapturingModePlace();
    stopRestoringModePlace();
  });

  // Mirror the authoritative router Space into shellStore so the event bus and
  // other surfaces (StatusLine, adaptive ranking, etc.) observe Space changes.
  createEffect(() => {
    const space: Space = currentRoute().space;
    shellStore.setActiveSpace(space);
  });

  // Reflect shell chrome state onto the document root for tokenized styling.
  createEffect(() => {
    if (typeof document === "undefined") return;
    const root = document.documentElement;
    root.setAttribute("data-theme", shellStore.theme());
    root.setAttribute("data-window-mode", shellStore.windowMode());
    root.setAttribute("data-density", shellStore.density());
  });

  // Boot the Tauri bridge on mount; dispose on unmount (graceful degradation is
  // handled inside the bridge — Req 20.4). Also wire the Core state machine so
  // domain events drive the Core, and mirror Core state onto the OS tray glyph
  // as an enhancement with in-app fallback (task 2.3, Req 3.4/18.2).
  // Register the palette's default commands + shortcuts (task 2.4). Disposed on
  // unmount so tests / hot-reload don't accumulate duplicates.
  // Wire summon (task 2.5, Req 2.5/18.2): the guaranteed in-app Cmd/Ctrl+K
  // hotkey plus the backend global-hotkey/tray summon event. The global hotkey
  // is a backend enhancement; this in-app listener never depends on OS support.
  let disposePalette: (() => void) | undefined;
  let disposeSummonWiring: (() => void) | undefined;
  let disposeRuntimeStatusStream: (() => void) | undefined;
  // Overlay inertness controller (design §20.3): marks lower surfaces inert
  // while a blocking layer (pending approval, its confirm, or a modal) is up.
  const disposeOverlayInertness = initOverlayInertness();
  onCleanup(disposeOverlayInertness);
  // The shell background/regions are the lowest layer; register so they are
  // inerted behind any blocking overlay (portaled overlays are unaffected).
  let unregisterShell: (() => void) | undefined;
  const bindShellRoot = (el: HTMLDivElement) => {
    unregisterShell?.();
    unregisterShell = registerOverlaySurface(el, "shell");
  };
  onCleanup(() => unregisterShell?.());
  onMount(() => {
    const installBrowserHarness =
      import.meta.env.DEV &&
      typeof window !== "undefined" &&
      new URLSearchParams(window.location.search).get("e2e") === "1";
    void tauriBridge.init().then(async () => {
      // Backend provisioning state is authoritative. Resolve it before normal
      // AppShell chrome is presented so first-run setup cannot be bypassed by
      // stale local storage or legacy presentation state.
      await provisioningStore.loadState();
      setProvisioningResolved(true);
      void capabilityStore.loadLlmRuntimeStatus();
      void converseStore.initialize();
      void settingsStore.initialize();
      const memoryInitialization = memoryStore.initialize();
      void automationStore.initialize();
      if (installBrowserHarness) {
        // Browser flow fixtures become available only after Memory's initial
        // backend read settles, preventing deterministic seeds from being
        // overwritten by an in-flight refresh.
        await memoryInitialization;
        const { installE2EHarness } = await import("../test/e2eHarness");
        installE2EHarness();
      } else {
        void memoryInitialization;
      }
    });
    void initWindowPresentation();
    initWindowModeManager();
    // Live app/LLM lifecycle → footer (starting/initializing/ready/failed).
    disposeRuntimeStatusStream = capabilityStore.initRuntimeStatusStream();
    coreStore.initCoreStateMachine();
    // Reflect the real backend voice pipeline (phase + barge-in/stop-phrase)
    // into voiceStore so the compact surface + Core stay truthful (Req 12.5).
    voiceStore.initVoiceBridge();
    initCoreTray();
    disposePalette = initPaletteDefaults();
    disposeSummonWiring = initSummon();
  });
  onCleanup(() => {
    disposeSummonWiring?.();
    disposePalette?.();
    disposeRuntimeStatusStream?.();
    disposeCoreTray();
    disposeWindowModeManager();
    disposeWindowPresentation();
    voiceStore.disposeVoiceBridge();
    automationStore.dispose();
    memoryStore.disposeRuntime();
    settingsStore.disposeRuntime();
    converseStore.disposeRuntime();
    coreStore.disposeCoreStateMachine();
    tauriBridge.dispose();
  });

  // Approval place snapshot across the blocking interrupt (design §20.3
  // Focus_Return_Owner, §20.4 focus fallback; Req 11.5/13.4). The Approval
  // Center is the sole asynchronous Blocking_Interrupt and the only surface that
  // seizes focus. AppShell owns the place snapshot "until the queue clears":
  // when a decision becomes pending we capture the user's transient place
  // (focused control + owning region, caret, scroll); when the queue fully
  // clears we return focus following the §20.4 ladder (original invoker → owning
  // region heading/container → #space-root → stable shell control), never onto
  // an Approve/destructive control, and without resetting draft/route/selection/
  // work state. Drafts/session already persist content; this covers the
  // in-flight place they don't.
  let placeSnapshot: ApprovalPlaceSnapshot | null = null;
  let wasPending = false;
  createEffect(() => {
    const pending = approvalStore.hasPending();
    if (pending && !wasPending) {
      placeSnapshot = captureApprovalPlace();
      // Delegate the conversation viewport to its single anchor owner (§21
      // IU-10): the coordinator dedupes with a coinciding mode change so the
      // stream is restored exactly once, never in raw px by this path.
      beginConversationPlace();
    } else if (!pending && wasPending) {
      const snap = placeSnapshot;
      placeSnapshot = null;
      // Restore after the overlay has torn down so focus lands on real content.
      queueMicrotask(() => {
        restoreApprovalPlace(snap);
        endConversationPlace();
      });
    }
    wasPending = pending;
  });

  // Default approvals opener: the built-in Approval Center overlay (Req 11.1).
  const openApprovals = () =>
    props.onOpenApprovals ? props.onOpenApprovals() : shellStore.setApprovalsOpen(true);

  // Default notifications opener: the built-in Notification Center (Req 13.3).
  const openNotifications = () => shellStore.setNotificationsOpen(true);

  return (
    <Show
      when={provisioningResolved()}
      fallback={
        // The footer is ALWAYS present (Req: persistent status) — even before
        // provisioning resolves — so the app/LLM lifecycle status is visible
        // from the first paint.
        <div class="kria-boot-shell">
          <main class="kria-shell-boot" role="status" aria-live="polite">
            <span>Loading provisioning state…</span>
          </main>
          <StatusLine />
        </div>
      }
    >
      <Show
        when={provisioningStore.isComplete()}
        fallback={
          <div class="kria-boot-shell">
            <SetupExperience />
            <StatusLine />
          </div>
        }
      >
        <div
      ref={bindShellRoot}
      class="kria-shell"
      data-window-mode={shellStore.windowMode()}
      data-space-composition={spaceComposition(shellStore.activeSpace(), shellStore.windowMode())}
    >
      <a class="kria-skip-link" href="#space-root">Skip to workspace</a>
      <PresenceBar onOpenApprovals={openApprovals} onOpenNotifications={openNotifications} />
      <div class="kria-shell__body">
        {/* Router is the sole authority for the rendered Space (Req 7.10 /
            design §9, §20.1). The Dock navigates via navigate(); the effect
            above mirrors currentRoute().space into shellStore.activeSpace.

            One unified, always-present sidebar across EVERY Space (matching the
            homepage sidebar). The former hover-reveal HiddenDock on the Converse
            home is retired so navigation is identical everywhere — no per-Space
            reveal behaviour. Same canonical 7-Space Dock; routing unchanged. */}
        <Dock />
        <SpaceRouter />
        <InspectorHost />
      </div>
      <StatusLine />
      <ModalHost />
      {/* Overlay layer (global singletons, design.md §2.2/§6.8). Mounted once so
          each opens instantly on a signal flip — no lazy chunk fetch (Req 2.1).
          The Approval Center is the one blocking interrupt (Req 11.5). */}
      <CommandPalette />
      <ApprovalCenter />
      {/* Notification Center is strictly below the Approval Center in the
          interruption ladder (Req 13.2): non-blocking, never auto-opens. The
          announcer is always mounted for the polite live region. */}
      <NotificationCenter />
      <NotificationAnnouncer />
      {/* Voice expressed through the Core + one transcript line — compact, not
          full-screen (Req 12.1). Shown only while voiceStore is active. */}
      <VoiceSurface />
      {/* Tauri multi-window is an enhancement. When unavailable, one bounded
          companion is hosted in-shell with identical dispatch-only controls. */}
      <CompanionFallbackHost />
      {/* Companion Mode ember (task 8.3, Req 15): the floating cross-application
          presence. Self-gates on Companion View Mode + the on-by-default opt-out,
          mirrors the Core state read-only, and degrades to an in-app ember where
          the compositor restricts always-on-top. */}
      <CompanionEmber />
        </div>
      </Show>
    </Show>
  );
}

export default AppShell;
