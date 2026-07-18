import { createEffect, onCleanup, onMount, Show } from "solid-js";
import { CorePresence } from "../components/CorePresence";
import { EmptyState } from "../kit";
import { tauriBridge } from "../bridge";
import {
  approvalStore,
  converseStore,
  coreStore,
  initCoreTray,
  disposeCoreTray,
  observatoryStore,
  settingsStore,
  shellStore,
} from "../stores";
import { ApprovalCenter } from "../shell/approvals";
import { ModalHost } from "../shell/ModalHost";
import MessageStream from "../shell/spaces/converse/MessageStream";
import Composer from "../shell/spaces/converse/Composer";
import KnowledgeGraphLens from "../shell/spaces/memory/graph/KnowledgeGraphLens";
import ConstellationLens from "../shell/spaces/capabilities/constellation/ConstellationLens";
import RemoteDesktopCanvas from "../shell/spaces/machines/RemoteDesktopCanvas";
import { NowRegion } from "../shell/spaces/ObservatorySpace";
import {
  isCompanionSurface,
  disposeWindowPresentation,
  initWindowPresentation,
  windowPresentation,
} from "./detachableSurfaces";
import { MiniCompanionSurface } from "./MiniCompanions";
import "../shell/spaces/ConverseSpace.css";
import "../shell/spaces/machines/machines.css";
import "../shell/spaces/ObservatorySpace.css";
import "./MiniCompanions.css";
import "./DetachedSurfaceRoot.css";

const TITLES = {
  thread: "Thread",
  "approval-center": "Approval Center",
  lens: "Lens",
  "remote-desktop": "Remote Desktop",
  "observatory-now": "Observatory Now",
  "kria-mini": "Mini",
  "now-mini": "Now mini",
} as const;

export function DetachedSurfaceRoot() {
  onMount(() => {
    if (import.meta.env.DEV && new URLSearchParams(window.location.search).get("e2e") === "1") {
      void import("../test/e2eHarness").then(({ installE2EHarness }) => installE2EHarness());
    }
    void tauriBridge.init();
    void settingsStore.load();
    void initWindowPresentation();
    coreStore.initCoreStateMachine();
    initCoreTray();
  });
  onCleanup(() => {
    disposeCoreTray();
    coreStore.disposeCoreStateMachine();
    disposeWindowPresentation();
    tauriBridge.dispose();
  });

  createEffect(() => {
    document.documentElement.setAttribute("data-theme", shellStore.theme());
    document.documentElement.setAttribute("data-density", shellStore.density());
    document.documentElement.setAttribute("data-window-mode", "standard");
  });

  createEffect(() => {
    const surface = windowPresentation.surface();
    if (surface === "thread") converseStore.setActiveThread(windowPresentation.context());
    if (surface === "approval-center") shellStore.setApprovalsOpen(true);
  });

  const companion = () => {
    const current = windowPresentation.surface();
    return isCompanionSurface(current) ? current : null;
  };

  return (
    <Show when={companion()} keyed fallback={
      <div class="kria-detached" data-detached-surface={windowPresentation.surface() ?? "unknown"}>
        <header class="kria-detached__header">
          <CorePresence size="md" />
          <div>
            <strong>KRIA</strong>
            <span>{windowPresentation.surface() ? TITLES[windowPresentation.surface()!] : "Surface unavailable"}</span>
          </div>
          <Show when={approvalStore.pendingCount() > 0}>
            <span class="kria-detached__approval-count" role="status">
              {approvalStore.pendingCount()} approval{approvalStore.pendingCount() === 1 ? "" : "s"}
            </span>
          </Show>
        </header>
        <main class="kria-detached__main"><SurfaceBody /></main>
        <ModalHost />
        <ApprovalCenter />
      </div>
    }>
      {(surface) => (
        <div class="kria-companion-window" data-companion-surface={surface}>
          <MiniCompanionSurface surface={surface} />
          <ModalHost />
          <ApprovalCenter />
        </div>
      )}
    </Show>
  );
}

function SurfaceBody() {
  const surface = windowPresentation.surface;
  return (
    <Show when={surface()} fallback={
      <EmptyState icon="monitor-off" title="Surface unavailable" description="This window type is not supported." />
    }>
      <Show when={surface() === "thread"}><DetachedThread /></Show>
      <Show when={surface() === "approval-center"}>
        <p class="kria-detached__waiting">Approval Center follows this window while it is active.</p>
      </Show>
      <Show when={surface() === "lens"}><DetachedLens /></Show>
      <Show when={surface() === "remote-desktop"}><RemoteDesktopCanvas /></Show>
      <Show when={surface() === "observatory-now"}><DetachedObservatoryNow /></Show>
    </Show>
  );
}

function DetachedThread() {
  return (
    <section class="kria-detached__thread" aria-label="Detached thread">
      <div class="kria-detached__messages"><MessageStream /></div>
      <div class="kria-detached__composer"><Composer /></div>
    </section>
  );
}

function DetachedLens() {
  return (
    <section class="kria-detached__lens" aria-label="Detached lens">
      <Show
        when={windowPresentation.context() === "capabilities"}
        fallback={<KnowledgeGraphLens />}
      >
        <ConstellationLens />
      </Show>
    </section>
  );
}

function DetachedObservatoryNow() {
  onMount(() => void observatoryStore.loadExecutiveSnapshot());
  onCleanup(observatoryStore.connectTelemetry());
  onCleanup(observatoryStore.connectJobs());
  return <section class="kria-observatory kria-detached__now" aria-label="Observatory Now"><NowRegion /></section>;
}

export default DetachedSurfaceRoot;
