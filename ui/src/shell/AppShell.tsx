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
import { ApprovalCenter } from "./approvals";
import { NotificationCenter, NotificationAnnouncer } from "./notifications";
import { VoiceSurface } from "./voice";
import { approvalStore } from "../stores";
import { capturePlace, restorePlace, type PlaceSnapshot } from "./placePreservation";
import { initSummon } from "../summon";
import { spaceComposition } from "./windowModePolicy";
import { initWindowPresentation, disposeWindowPresentation } from "../windowing/detachableSurfaces";
import { initWindowModeManager, disposeWindowModeManager } from "../windowing/windowModeManager";
import { CompanionFallbackHost } from "../windowing/MiniCompanions";
import { SetupExperience } from "./setup/SetupExperience";
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
  let modePlaceSnapshot: PlaceSnapshot | null = null;
  const stopCapturingModePlace = eventBus.on("shell:mode-changing", () => {
    modePlaceSnapshot = capturePlace();
  }, "none");
  const stopRestoringModePlace = eventBus.on("shell:mode-changed", () => {
    const snap = modePlaceSnapshot;
    modePlaceSnapshot = null;
    queueMicrotask(() => restorePlace(snap));
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

  // Place preservation across the blocking interrupt (Req 13.4). The Approval
  // Center is the only surface that seizes focus; when a decision becomes
  // pending we snapshot the user's transient place (focused control, caret,
  // scroll), and when the queue clears we restore it — so approving/denying
  // returns focus exactly where it was. Drafts/session already persist content;
  // this covers the in-flight place they don't.
  let placeSnapshot: PlaceSnapshot | null = null;
  let wasPending = false;
  createEffect(() => {
    const pending = approvalStore.hasPending();
    if (pending && !wasPending) {
      placeSnapshot = capturePlace();
    } else if (!pending && wasPending) {
      const snap = placeSnapshot;
      placeSnapshot = null;
      // Restore after the overlay has torn down so focus lands on real content.
      queueMicrotask(() => restorePlace(snap));
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
        <main class="kria-shell-boot" role="status" aria-live="polite">
          <span>Loading provisioning state…</span>
        </main>
      }
    >
      <Show when={provisioningStore.isComplete()} fallback={<SetupExperience />}>
        <div
      class="kria-shell"
      data-window-mode={shellStore.windowMode()}
      data-space-composition={spaceComposition(shellStore.activeSpace(), shellStore.windowMode())}
    >
      <a class="kria-skip-link" href="#space-root">Skip to workspace</a>
      <PresenceBar onOpenApprovals={openApprovals} onOpenNotifications={openNotifications} />
      <div class="kria-shell__body">
        <Dock onSelect={(space) => shellStore.setActiveSpace(space)} />
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
        </div>
      </Show>
    </Show>
  );
}

export default AppShell;
