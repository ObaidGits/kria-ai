/** Dev-only deterministic browser harness for final flow-map E2E. */
import {
  approvalStore,
  automationStore,
  capabilityStore,
  converseStore,
  coreStore,
  eventBus,
  memoryStore,
  notificationStore,
  shellStore,
  voiceStore,
} from "../stores";
import type { Theme } from "../stores/shellStore";
import type { ActiveLlmRuntime, OpenClawSettings } from "../stores/capabilityStore";
import type { Workflow } from "../stores/automationStore";
import { setWindowPresentationActive } from "../windowing/detachableSurfaces";
import { currentRoute } from "../shell/router";
import { coreNarration } from "../stores/coreNarration";
import { openModal, closeModal } from "../shell/modalHost";
import {
  beginConversationPlace,
  endConversationPlace,
  __conversationRestoreCount,
} from "../shell/spaces/converse/conversationPlace";

const CORRECTION_FACT = {
  id: "e2e-memory-correction",
  content: "Project Atlas launches on Monday",
  confidence: 0.72,
  worth: 0.6,
  staleness: 0.1,
  source: "conversation",
  createdAt: 1,
  updatedAt: 1,
  tags: ["project"],
};

const VOICE_FACT = {
  id: "e2e-voice-memory",
  content: "Voice-approved deployment completed with verification",
  confidence: 0.95,
  worth: 0.9,
  staleness: 0,
  source: "verified-action",
  createdAt: 2,
  updatedAt: 2,
  tags: ["voice", "verified"],
};

const MULTI_WINDOW_APPROVAL = {
  id: "e2e-multi-window-approval",
  source: "tool-hitl" as const,
  title: "Approve remote maintenance",
  description: "KRIA needs permission before the verified maintenance step.",
  risk: "yellow" as const,
  effects: ["Run one bounded maintenance action"],
  routing: { requestId: "e2e-maintenance-request" },
};

type BackendCall = { command: string; args?: Record<string, unknown> };
type FixtureBackend = {
  calls: BackendCall[];
  setMemoryEntries(entries: Array<Record<string, unknown>>): void;
};

function backend(): FixtureBackend | undefined {
  return (window as unknown as { __KRIA_E2E_BACKEND__?: FixtureBackend }).__KRIA_E2E_BACKEND__;
}

function syncFixtureMemory(facts: typeof CORRECTION_FACT[]): void {
  backend()?.setMemoryEntries(facts);
}

function addMultiWindowApproval(): void {
  approvalStore.addFromEnvelope(MULTI_WINDOW_APPROVAL);
}

// ── Task 10.9 capability-exposure fixtures (IU-07) ───────────────────────────
// Deterministic factories that build the AUTHORITATIVE store shapes the
// read-only Task-10 exposure surfaces (CurrentWorkSummary, ContextRail,
// capabilityDisclosure) read. Bridge-free: they set store signals only — no
// send, no tool, no approval, no backend/network request.

function exposureRuntime(overrides: Partial<ActiveLlmRuntime> = {}): ActiveLlmRuntime {
  return {
    providerId: "local",
    providerType: "local",
    displayName: "Local llama.cpp",
    activeModel: "qwen2.5-7b",
    endpoint: "http://127.0.0.1:8080",
    enabled: true,
    configured: true,
    isLocal: true,
    isLlamaCppRuntime: true,
    requiresApiKey: false,
    routingMode: "local",
    restartRequiredForLocalModelChange: false,
    routerHealthy: true,
    envWins: false,
    activeEnvVars: [],
    ...overrides,
  } as ActiveLlmRuntime;
}

function exposureWorkflow(overrides: Partial<Workflow> = {}): Workflow {
  return {
    id: overrides.id ?? "e2e-exposure-wf",
    name: overrides.name ?? "Nightly sync",
    description: overrides.description ?? "",
    status: overrides.status ?? "running",
    lastRunAt: overrides.lastRunAt ?? null,
    createdAt: overrides.createdAt ?? 0,
    ...overrides,
  } as Workflow;
}

function exposureOpenClaw(runtimeActive: boolean): OpenClawSettings {
  return {
    enabled: runtimeActive,
    image: "",
    warmPerClass: 0,
    maxConcurrentInvocations: 0,
    defaultTimeoutSecs: 0,
    maxWarmAgeSecs: 0,
    maxRestartAttempts: 0,
    rewriteDescriptions: false,
    checkUpdates: false,
    registryIndexUrl: "",
    communityAllowsNetwork: false,
    verifiedSkipsHitl: false,
    runtimeActive,
  } as OpenClawSettings;
}

/** Reset every authoritative source the Task-10 exposure surfaces read. */
function resetCapabilityExposure(): void {
  coreStore.reset();
  converseStore.clearWorkBlocks();
  converseStore.setContextRailItems([]);
  automationStore.setWorkflows([]);
  capabilityStore.setActiveLlmRuntime(null);
  capabilityStore.setCapabilities([]);
  capabilityStore.setSkills([]);
  capabilityStore.setOpenClawSettings(null);
}

export function installE2EHarness(): void {
  const target = window as unknown as { __KRIA_E2E__?: Record<string, unknown> };
  if (target.__KRIA_E2E__) return;

  const workCancelRequests: Array<{ blockId: string; blockType: string }> = [];
  eventBus.on("converse:work-cancel-requested", (request) => workCancelRequests.push(request));

  window.addEventListener("storage", (event) => {
    if (event.key === "kria-e2e-pending-approval" && event.newValue) {
      addMultiWindowApproval();
    }
    if (event.key === "kria-e2e-approval-resolution" && event.newValue) {
      const value = JSON.parse(event.newValue) as { id?: string };
      if (value.id) approvalStore.dismiss(value.id);
    }
  });

  target.__KRIA_E2E__ = {
    seedVoiceApproval() {
      voiceStore.activate();
      voiceStore.setState("listening");
      voiceStore.setTranscript("Deploy the verified preview and remember the result", false);
      approvalStore.addFromEnvelope({
        id: "e2e-voice-approval",
        source: "tool-hitl",
        title: "Deploy verified preview",
        description: "Voice intent resolved to a bounded deployment requiring approval.",
        risk: "yellow",
        effects: ["Deploy preview", "Verify health", "Record verified outcome"],
        evidence: "Voice transcript matched the preview deployment intent.",
        routing: { requestId: "e2e-voice-request" },
      });
    },
    completeVoiceExecution() {
      voiceStore.setState("thinking");
      const facts = [...memoryStore.facts().filter((f) => f.id !== VOICE_FACT.id), VOICE_FACT];
      syncFixtureMemory(facts);
      memoryStore.setFacts(facts);
      eventBus.emit("memory:updated", { factId: VOICE_FACT.id });
      voiceStore.setTranscript("Deployment completed, verified, and remembered", false);
      voiceStore.setState("speaking");
    },
    seedMemoryCorrection() {
      syncFixtureMemory([CORRECTION_FACT]);
      memoryStore.setFacts([CORRECTION_FACT]);
    },
    seedMultiWindowApproval() {
      addMultiWindowApproval();
      localStorage.setItem("kria-e2e-pending-approval", String(Date.now()));
    },
    setWindowActive(value: boolean) {
      setWindowPresentationActive(value);
    },
    setConverseWorkVisible(visible: boolean) {
      coreStore.reset();
      converseStore.clearWorkBlocks();
      if (visible) {
        converseStore.addWorkBlock({
          id: "e2e-converse-geometry-work",
          type: "tool-call",
          status: "running",
          summary: "Inspect semantic lane geometry",
          startedAt: Date.now(),
          details: "Integrated semantic lane regression fixture.",
          evidence: [{
            id: "e2e-converse-geometry-evidence",
            label: "Semantic geometry trace",
            detail: "Verified semantic lane occupancy",
          }],
        });
      }
    },
    setConverseContextAvailable(available: boolean) {
      converseStore.setContextRailItems(available ? [{
        id: "e2e-converse-context",
        type: "memory",
        label: "Geometry context",
        data: { source: "e2e" },
      }] : []);
    },
    /**
     * Task 10.9 — drive the READ-ONLY capability/context exposure surfaces
     * (CurrentWorkSummary in the PresenceBar, the ContextRail, and the empty-
     * state capabilityDisclosure) into one canonical state for visual capture.
     * Every state mutates only authoritative store signals — it sends nothing,
     * invokes no tool, grants no approval, and issues NO backend/network request
     * (the exact "no extra request" invariant the spec asserts via backendCalls).
     *
     *   • "empty"        — idle: no model, no work, no background, empty rail
     *                      (Cold Start Homepage). Truthful "Idle" cue only.
     *   • "partial"      — one fact present: a configured model (F1) + live
     *                      foreground work (F5), no background, empty rail.
     *   • "full"         — model + foreground work + running background
     *                      automation (F8) + a populated, enriched ContextRail.
     *   • "long-name"    — a very long model / workflow / context source name so
     *                      bounded (clamped) presentation is exercised.
     *   • "active-background-work" — a running automation only (foreground idle):
     *                      the background indicator surfaces on its own.
     *   • "optional-service-unavailable" — OpenClaw settings present but the
     *                      runtime is offline (+ an installed/enabled skill) so
     *                      the F7 disclosure reads "unavailable" — truthfully,
     *                      never fabricated as ready.
     */
    setCapabilityExposureState(
      state:
        | "empty"
        | "partial"
        | "full"
        | "long-name"
        | "active-background-work"
        | "optional-service-unavailable",
    ) {
      resetCapabilityExposure();
      // Cold-start Homepage as the neutral backdrop (empty conversation) so the
      // empty-state disclosure surface is mounted where relevant.
      converseStore.clearMessages();
      converseStore.setThreads([]);
      converseStore.setActiveThread(null);

      switch (state) {
        case "empty":
          break;
        case "partial":
          capabilityStore.setActiveLlmRuntime(exposureRuntime());
          converseStore.addWorkBlock({
            id: "e2e-exposure-fg",
            type: "tool-call",
            status: "running",
            summary: "Indexing files",
            startedAt: Date.now(),
          });
          break;
        case "full":
          capabilityStore.setActiveLlmRuntime(exposureRuntime());
          converseStore.addWorkBlock({
            id: "e2e-exposure-fg",
            type: "tool-call",
            status: "running",
            summary: "Indexing files",
            startedAt: Date.now(),
          });
          automationStore.setWorkflows([
            exposureWorkflow({ id: "e2e-exposure-wf", name: "Nightly sync", status: "running" }),
          ]);
          converseStore.setContextRailItems([
            {
              id: "e2e-exposure-ctx",
              type: "document",
              label: "Q3 report",
              data: null,
              source: "quarterly-report.pdf",
              use: "used",
              detail: "Pages 3-5 summarized",
            },
          ]);
          break;
        case "long-name":
          capabilityStore.setActiveLlmRuntime(
            exposureRuntime({
              activeModel:
                "qwen2.5-72b-instruct-vision-preview-extended-context-quantized-q8-experimental",
            }),
          );
          automationStore.setWorkflows([
            exposureWorkflow({
              id: "e2e-exposure-wf-long",
              name: "Automatisierungs-Workflow zur nächtlichen Synchronisierung sämtlicher Dokumentenspeicher und Wissensdatenbanken",
              status: "running",
            }),
          ]);
          converseStore.setContextRailItems([
            {
              id: "e2e-exposure-ctx-long",
              type: "document",
              label:
                "An extraordinarily long context label that would otherwise expand the lane and force horizontal overflow across the layout",
              data: null,
              source:
                "extremely-long-source-provenance-identifier-that-must-not-break-the-layout.document.v3.final",
              use: "used",
            },
          ]);
          break;
        case "active-background-work":
          automationStore.setWorkflows([
            exposureWorkflow({ id: "e2e-exposure-wf", name: "Nightly sync", status: "running" }),
          ]);
          break;
        case "optional-service-unavailable":
          // Settings present but runtime OFFLINE → F7 disclosure "unavailable".
          capabilityStore.setOpenClawSettings(exposureOpenClaw(false));
          capabilityStore.setSkills([
            {
              slug: "calendar-connector",
              name: "Calendar Connector",
              description: "",
              category: "productivity",
              trustTier: "community",
              installed: true,
              enabled: true,
            },
          ]);
          break;
      }
    },
    /**
     * Cold Start / first run (task 6.8, Req 6.3/6.1): no active thread, no
     * usable history, no messages → `emptyStateClass()` is "cold-start", so the
     * Homepage shows the orientation heading + ≤3 grounded starters and the
     * ThreadSidebar defaults closed. Bridge-free: mutates only authoritative
     * store signals, sends nothing, invokes no tool.
     */
    seedConverseColdStart() {
      coreStore.reset();
      converseStore.clearMessages();
      converseStore.setThreads([]);
      converseStore.setActiveThread(null);
    },
    /**
     * Intentional New Thread with unrelated history (task 6.8, Req 6.1,
     * UIE-H-005): seed unrelated history threads plus a fresh empty active
     * thread carrying explicit new-thread intent. The classifier MUST present
     * the new-task state (starters, not continuation) even though unrelated
     * history exists, and that history stays reachable in the sidebar. Uses the
     * bridge-free `markIntentionalNewThread` seam — no backend call, no send.
     */
    seedConverseIntentionalNewThread() {
      coreStore.reset();
      converseStore.clearMessages();
      const now = Date.now();
      converseStore.setThreads([
        { id: "e2e-new-thread", title: "New task", createdAt: now, updatedAt: now, pinned: false, archived: false, temporary: false },
        { id: "e2e-history-research", title: "Unrelated research notes", createdAt: now - 3000, updatedAt: now - 3000, pinned: false, archived: false, temporary: false },
        { id: "e2e-history-budget", title: "Q3 budget planning", createdAt: now - 4000, updatedAt: now - 4000, pinned: false, archived: false, temporary: false },
      ]);
      converseStore.markIntentionalNewThread("e2e-new-thread");
    },
    /**
     * Continuation / returning user (task 6.9, Req 6.1/6.4): an empty active
     * thread plus usable non-archived history and NO explicit new-thread intent
     * → `emptyStateClass()` is "continuation", so the Homepage shows the
     * "Continue where you left off" heading + ≤3 relevant resumptions and the
     * ThreadSidebar defaults OPEN (returning users retain their history).
     * Bridge-free: mutates only authoritative store signals, sends nothing,
     * invokes no tool.
     */
    seedConverseContinuation() {
      coreStore.reset();
      converseStore.clearMessages();
      const now = Date.now();
      converseStore.setThreads([
        { id: "e2e-cont-active", title: "Current empty thread", createdAt: now, updatedAt: now, pinned: false, archived: false, temporary: false },
        { id: "e2e-cont-deploy", title: "Deploy verified preview", createdAt: now - 1000, updatedAt: now - 1000, pinned: false, archived: false, temporary: false },
        { id: "e2e-cont-research", title: "Research on embeddings", createdAt: now - 2000, updatedAt: now - 2000, pinned: false, archived: false, temporary: false },
        { id: "e2e-cont-budget", title: "Q3 budget planning", createdAt: now - 3000, updatedAt: now - 3000, pinned: false, archived: false, temporary: false },
      ]);
      converseStore.setActiveThread("e2e-cont-active");
    },
    converseEmptyStateClass() {
      return converseStore.emptyStateClass();
    },
    seedConverseMessages(count = 300) {
      converseStore.clearMessages();
      for (let index = 0; index < count; index += 1) {
        converseStore.addMessage({
          id: `e2e-layout-message-${index}`,
          threadId: "e2e-layout-thread",
          role: index % 2 === 0 ? "user" : "assistant",
          content: `E2E layout message ${index}`,
          timestamp: index,
        });
      }
    },
    openConverseInspector() {
      shellStore.openInspector("memory", "e2e-converse-inspector");
    },
    closeConverseInspector() {
      shellStore.closeInspector();
    },
    seedConverseResponsivePropertyState() {
      shellStore.setActiveSpace("converse");
      shellStore.setWindowMode("standard");
      shellStore.closeInspector();
      converseStore.setActiveThread("e2e-responsive-property-thread");
      converseStore.updateDraft({
        text: "Property-preserved responsive draft",
        mode: "assistant",
        attachments: [],
      });
    },
    setConverseWindowMode(mode: "standard" | "mini" | "immersive") {
      shellStore.setWindowMode(mode);
    },
    // ── Task 8.10 Overlay/VoiceSurface z-order + Wayland scaling hooks ────────
    // Bridge-free deterministic drivers for the authored z-order/visual specs.
    // Each mutates only authoritative store/host signals — no send, no tool.
    setVoiceActive(active: boolean) {
      if (active) {
        voiceStore.activate();
        voiceStore.setState("listening");
        voiceStore.setTranscript("Listening for your request", false);
      } else {
        voiceStore.deactivate();
      }
    },
    /** Approval-only pending interrupt (no voice) for isolated z-order capture. */
    seedPendingApprovalOnly() {
      approvalStore.setQueue([]);
      approvalStore.addFromEnvelope({
        id: "e2e-zorder-approval",
        source: "tool-hitl",
        title: "Approve the drafted email",
        description: "KRIA needs permission before sending the drafted email.",
        risk: "yellow",
        effects: ["Sends 1 email"],
        routing: { requestId: "e2e-zorder-request" },
      });
    },
    /** Open the nested approval confirmation ABOVE the Approval Center (§20.3). */
    openApprovalConfirm() {
      openModal({
        id: "e2e-approval-confirm",
        title: "Confirm high-risk action",
        layer: "approval-confirm",
        render: () => null,
      });
    },
    closeApprovalConfirm() {
      closeModal("e2e-approval-confirm");
    },
    // ── Task 14.9 §24.5 Overlay-matrix drivers (isolation + concurrency) ──────
    // Bridge-free openers for the remaining §24.5 rows the earlier harness did
    // not expose. Each mutates only authoritative shell/notification signals —
    // no send, no tool, no approval, no backend/network request.
    /** Open the Command Palette (§20.3 palette row). */
    openPalette() {
      shellStore.setPaletteOpen(true);
    },
    /** Seed one notice and open the (non-blocking) Notification Center. */
    openNotificationCenter() {
      notificationStore.push({
        id: "e2e-matrix-notice",
        level: "info",
        message: "Background sync completed",
      });
      shellStore.setNotificationsOpen(true);
    },
    /** Open a plain (non-approval) ModalHost dialog (§20.3 ModalHost row). */
    openPlainModal() {
      openModal({
        id: "e2e-plain-modal",
        title: "Dialog",
        render: () => null,
      });
    },
    closePlainModal() {
      closeModal("e2e-plain-modal");
    },
    /** Set the shell theme (drives the root data-theme attribute). */
    setTheme(theme: Theme) {
      shellStore.setTheme(theme);
    },
    clearOverlays() {
      closeModal();
      approvalStore.setQueue([]);
      voiceStore.deactivate();
      shellStore.setPaletteOpen(false);
      shellStore.setNotificationsOpen(false);
      shellStore.setApprovalsOpen(false);
      shellStore.closeInspector();
      notificationStore.clear();
    },
    converseResponsivePropertyState() {
      const draft = converseStore.composerDraft();
      return {
        route: { ...currentRoute() },
        activeSpace: shellStore.activeSpace(),
        activeThreadId: converseStore.activeThreadId(),
        draft: {
          text: draft.text,
          mode: draft.mode,
          toolLock: draft.toolLock,
          attachmentIds: draft.attachments.map((attachment) => attachment.id),
        },
      };
    },
    stressTelemetry(count = 2_000) {
      for (let index = 0; index < count; index += 1) {
        eventBus.emit("observatory:telemetry", { metric: "cpu", value: index % 100, ts: index });
      }
    },
    backendCalls() {
      return backend()?.calls ?? [];
    },
    clearWorkCancelRequests() {
      workCancelRequests.length = 0;
    },
    workCancelRequests() {
      return [...workCancelRequests];
    },
    pendingApprovalCount() {
      return approvalStore.pendingCount();
    },
    // ── Task 9.8 long-thread perf hooks (IU-10 / UIE-M-005) ──────────────────
    // Read-only bridge to the single-owner restoration coordinator so the perf
    // spec can prove "restore exactly once per settled transition, no duplicate
    // restoration" against the SAME module the shell uses (no fabricated state).
    conversationRestoreCount() {
      return __conversationRestoreCount();
    },
    /** Drive one coordinated conversation-place transition (begin then end). */
    driveConversationTransition() {
      beginConversationPlace();
      endConversationPlace();
    },
    /** Begin a coordinated conversation-place transition (for overlap tests). */
    beginConversationPlace() {
      beginConversationPlace();
    },
    /** End a coordinated conversation-place transition (for overlap tests). */
    endConversationPlace() {
      endConversationPlace();
    },
    /**
     * Drive the shell into one canonical status-presence state so the
     * StatusLine (Core narration + idle minimization) and the cross-Space
     * CurrentWorkSummary indicator can be captured/announced deterministically
     * (task 5.10). Read-only surfaces are exercised through their real
     * authoritative signals only — no fabricated UI state.
     */
    setStatusPresenceState(
      state: "active" | "idle" | "blocked" | "error" | "recovered",
    ) {
      // Reset every authoritative source first so each state is clean.
      coreStore.reset();
      converseStore.clearWorkBlocks();
      approvalStore.setQueue([]);

      switch (state) {
        case "active": {
          converseStore.addWorkBlock({
            id: "e2e-status-active-work",
            type: "tool-call",
            status: "running",
            summary: "Index the project workspace",
            startedAt: Date.now(),
            details: "Cross-Space active-work capture fixture.",
          });
          coreStore.setState("acting");
          break;
        }
        case "idle": {
          // reset() already leaves Core idle with no work/approvals.
          break;
        }
        case "blocked": {
          approvalStore.addFromEnvelope({
            id: "e2e-status-blocked-approval",
            source: "tool-hitl",
            title: "Approve maintenance step",
            description: "KRIA needs permission before the bounded maintenance step.",
            risk: "yellow",
            effects: ["Run one bounded maintenance action"],
            routing: { requestId: "e2e-status-blocked-request" },
          });
          coreStore.setBlocked("waiting for approval");
          break;
        }
        case "error": {
          coreStore.setError("model runtime disconnected");
          break;
        }
        case "recovered": {
          // error → recovering is the authoritative recovery transition.
          coreStore.setError("model runtime disconnected");
          coreStore.setState("recovering");
          break;
        }
      }
    },
    /**
     * Snapshot the CURRENT StatusLine live-region content (Core label + concise
     * narration) exactly as assistive tech would read it. Used to build the
     * announcement transcript and prove unchanged text is not re-announced.
     */
    statusNarrationSnapshot() {
      const n = coreNarration();
      return {
        coreState: coreStore.state(),
        narrationKey: n?.key ?? null,
        narrationText: n?.text ?? null,
        actionable: n?.actionable ?? false,
        minimized: !n && !coreStore.isActive() && !coreStore.needsAttention(),
      };
    },
  };
}
