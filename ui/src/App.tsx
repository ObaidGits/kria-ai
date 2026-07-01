import { Component, Show, For, createSignal, createMemo, createEffect, onMount, onCleanup, lazy, Suspense, ErrorBoundary } from "solid-js";
import { listen } from "@tauri-apps/api/event";
import { appStore } from "./stores/app";
import { provisioningStore } from "./stores/provisioning";
import ChatView from "./components/ChatView";
import TasksView from "./components/TasksView";
import AddTargetModal from "./components/AddTargetModal";
import EditTargetModal from "./components/EditTargetModal";
import PromptLabView from "./components/PromptLabView";
import SessionSidebar from "./components/SessionSidebar";
import SettingsModal from "./components/SettingsModal";
import HitlModal from "./components/HitlModal";
import DecisionActionCenter from "./components/DecisionActionCenter";
import VoiceOverlay from "./components/VoiceOverlay";
import VoiceOnboarding from "./components/VoiceOnboarding";
import SetupWizard from "./components/SetupWizard";
import { DeviceTargetView, useDeviceStatus } from "./hooks/useDeviceStatus";
const DeviceMatrix = lazy(() => import("./components/DeviceMatrix"));
const TestRunnerDashboard = lazy(() => import("./components/TestRunnerDashboard"));
const AnalyticsDashboard = lazy(() => import("./components/AnalyticsDashboard"));
const N8nDashboard = lazy(() => import("./components/N8nDashboard"));

interface Toast {
  id: number;
  message: string;
  type: "success" | "error" | "info";
}

let toastId = 0;

export function addToast(message: string, type: Toast["type"] = "info") {
  const id = ++toastId;
  setToasts((prev) => [...prev, { id, message, type }]);
  setTimeout(() => {
    setToasts((prev) => prev.filter((t) => t.id !== id));
  }, 4000);
}

const [toasts, setToasts] = createSignal<Toast[]>([]);
const CONTROL_PANEL_EXPANDED_STORAGE_KEY = "kria_control_panel_expanded";
const FLEET_MATRIX_VISIBLE_STORAGE_KEY = "kria_fleet_matrix_visible";
type AppRoute = "home" | "dashboard" | "vm-management" | "settings" | "tasks";

function routeFromHash(hash: string): AppRoute {
  const clean = hash.replace(/^#/, "").trim();
  if (clean === "/dashboard") return "dashboard";
  if (clean === "/vm-management") return "vm-management";
  if (clean === "/settings") return "settings";
  if (clean === "/tasks") return "tasks";
  return "home";
}

function hashForRoute(route: AppRoute): string {
  if (route === "dashboard") return "#/dashboard";
  if (route === "vm-management") return "#/vm-management";
  if (route === "settings") return "#/settings";
  if (route === "tasks") return "#/tasks";
  return "#/";
}

const App: Component = () => {
  const {
    showSettings,
    showHitl,
    voiceActive,
    showVoiceOnboarding,
    setShowSettings,
    currentEnvironment,
    colabDispatchWarning,
    developerMode,
  } = appStore;
  const [showShortcuts, setShowShortcuts] = createSignal(false);
  const [showWizard, setShowWizard] = createSignal(false);
  const [wizardLoading, setWizardLoading] = createSignal(true);
  const [dashboardView, setDashboardView] = createSignal<"overview" | "operations" | "forensics" | "n8n">("overview");
  const [route, setRoute] = createSignal<AppRoute>(routeFromHash(typeof window !== "undefined" ? window.location.hash : "#/"));
  const initialControlPanelExpanded =
    typeof window === "undefined"
      ? false
      : window.localStorage.getItem(CONTROL_PANEL_EXPANDED_STORAGE_KEY) === "true";
  const initialDeviceMatrixVisible =
    typeof window === "undefined"
      ? true
      : window.localStorage.getItem(FLEET_MATRIX_VISIBLE_STORAGE_KEY) !== "false";
  const [controlPanelExpanded, setControlPanelExpanded] = createSignal<boolean>(initialControlPanelExpanded);
  const [showDeviceMatrix, setShowDeviceMatrix] = createSignal<boolean>(initialDeviceMatrixVisible);
  const [showAddTargetModal, setShowAddTargetModal] = createSignal(false);
  const [showEditTargetModal, setShowEditTargetModal] = createSignal(false);
  const [editingTarget, setEditingTarget] = createSignal<any>(null);
  const [deletingTargetIds, setDeletingTargetIds] = createSignal<Set<string>>(new Set());
  const [showTestDashboard, setShowTestDashboard] = createSignal(false);
  const [showAnalytics, setShowAnalytics] = createSignal(false);
  const [resetReason, setResetReason] = createSignal("");
  const [hardResetConfirmation, setHardResetConfirmation] = createSignal("");
  const [lastResetToastEventId, setLastResetToastEventId] = createSignal<string | null>(null);

  const assistantStatus = createMemo(() => appStore.assistantStatus());
  const ironcladStatus = createMemo(() => appStore.ironcladStatus());
  const resetSnapshot = createMemo(() => ironcladStatus()?.reset ?? null);
  const resetBusy = createMemo(() => Boolean(resetSnapshot()?.in_flight));
  const forensicEntries = createMemo(() => appStore.ironcladForensics().slice(0, 10));
  const qosTrafficLightClass = createMemo(() => {
    const light = ironcladStatus()?.qos?.traffic_light ?? "gray";
    if (light === "green") return "ironclad-traffic-dot green";
    if (light === "yellow") return "ironclad-traffic-dot yellow";
    if (light === "red") return "ironclad-traffic-dot red";
    return "ironclad-traffic-dot gray";
  });
  const resetPhaseLabel = createMemo(() => {
    const phase = resetSnapshot()?.phase ?? "idle";
    if (phase === "in_progress") return "Reset in progress";
    if (phase === "healthy") return "Last reset healthy";
    if (phase === "failed") return "Last reset failed";
    if (phase === "requested") return "Reset queued";
    return "Reset idle";
  });
  const routingMode = createMemo(() => {
    const raw = String(appStore.settings()?.llm?.routing_mode ?? "local").toLowerCase();
    if (raw === "cloud") return "gemini";
    if (raw === "hybrid") return "local";
    if (["local", "colab", "gemini", "external"].includes(raw)) return raw;
    return "local";
  });
  const routingSummary = createMemo(() => {
    const requested = routingMode();
    if (colabDispatchWarning()) {
      return `${requested} -> local fallback`;
    }
    return requested;
  });
  const connectedMcpServers = createMemo(
    () => appStore
      .mcpServers()
      .filter((server) => {
        const runtimeState = String(server.runtime_state ?? (server.enabled ? "running" : "stopped")).toLowerCase();
        return runtimeState === "running";
      })
      .length
  );
  const statusDotClass = createMemo(() => {
    const state = assistantStatus().state;
    return state === "ready"
      ? "status-dot"
      : state === "warming"
      ? "status-dot warming"
      : state === "degraded"
      ? "status-dot degraded"
      : "status-dot disconnected";
  });

  const controllerBaseUrl = createMemo<string | null>(() => {
    const status = ironcladStatus() as Record<string, any> | null;
    const settings = appStore.settings() as Record<string, any> | null;
    const candidates: unknown[] = [
      status?.fleet?.pool_packet?.controller_base_url,
      status?.fleet?.pool_packet?.controllerBaseUrl,
      status?.fleet?.controller_base_url,
      status?.fleet?.controllerBaseUrl,
      status?.controller_base_url,
      status?.controllerBaseUrl,
      settings?.ironclad?.controller_url,
      settings?.ironclad?.controllerUrl,
      settings?.ironclad?.controller_base_url,
      settings?.ironclad?.controllerBaseUrl,
      settings?.fleet?.controller_base_url,
      settings?.fleet?.controllerBaseUrl,
      settings?.fleet?.controller_url,
      settings?.fleet?.controllerUrl,
      settings?.server?.controller_base_url,
      settings?.server?.controllerBaseUrl,
      settings?.server?.base_url,
      settings?.server?.baseUrl,
    ];

    for (const candidate of candidates) {
      if (typeof candidate === "string" && candidate.trim().length > 0) {
        return candidate.trim().replace(/\/+$/, "").replace(/\/v1$/i, "");
      }
    }

    const hostCandidate =
      settings?.server?.host ?? settings?.server?.local_host ?? settings?.local_host;
    const portCandidate =
      settings?.server?.port ?? settings?.server?.local_port ?? settings?.local_port;
    const parsedPort =
      typeof portCandidate === "number"
        ? portCandidate
        : typeof portCandidate === "string"
        ? Number.parseInt(portCandidate, 10)
        : Number.NaN;

    if (typeof hostCandidate === "string" && hostCandidate.trim().length > 0 && Number.isFinite(parsedPort)) {
      const normalizedHost = hostCandidate.trim() === "0.0.0.0" ? "127.0.0.1" : hostCandidate.trim();
      return `http://${normalizedHost}:${Math.trunc(parsedPort)}`;
    }

    return null;
  });

  const fleetLeaseId = createMemo(() => {
    const status = ironcladStatus() as Record<string, any> | null;
    const settings = appStore.settings() as Record<string, any> | null;
    const candidates: unknown[] = [
      status?.fleet?.pool_packet?.active_lease_id,
      status?.fleet?.pool_packet?.activeLeaseId,
      status?.fleet?.pool_packet?.lease_id,
      status?.fleet?.pool_packet?.leaseId,
      status?.fleet?.active_lease_id,
      status?.fleet?.activeLeaseId,
      status?.fleet?.lease_id,
      status?.fleet?.leaseId,
      status?.active_lease_id,
      status?.activeLeaseId,
      status?.lease_id,
      status?.leaseId,
      settings?.ironclad?.active_lease_id,
      settings?.ironclad?.activeLeaseId,
      settings?.ironclad?.lease_id,
      settings?.ironclad?.leaseId,
      settings?.fleet?.active_lease_id,
      settings?.fleet?.activeLeaseId,
      settings?.fleet?.lease_id,
      settings?.fleet?.leaseId,
    ];

    for (const candidate of candidates) {
      if (typeof candidate === "string" && candidate.trim().length > 0) {
        return candidate.trim();
      }
    }

    return null;
  });

  const initialRegistryTargets = createMemo<DeviceTargetView[]>(() => {
    const status = ironcladStatus() as Record<string, any> | null;
    const fleet = (status?.fleet ?? {}) as Record<string, any>;
    const rawTargets: unknown[] = [];

    if (Array.isArray(fleet.enrolled_targets)) {
      rawTargets.push(...fleet.enrolled_targets);
    }
    if (Array.isArray(fleet.connection_control_targets)) {
      rawTargets.push(...fleet.connection_control_targets);
    }

    const map = new Map<string, DeviceTargetView>();
    for (const entry of rawTargets) {
      if (!entry || typeof entry !== "object") {
        continue;
      }
      const row = entry as Record<string, unknown>;
      const targetIdRaw = row.target_id ?? row.targetId ?? row.id;
      const targetId = typeof targetIdRaw === "string" ? targetIdRaw.trim() : "";
      if (!targetId || map.has(targetId)) {
        continue;
      }

      const displayNameRaw = row.display_name ?? row.displayName;
      const modeRaw = row.mode;
      // Use live backend data from connection_control_targets when available,
      // fall back to sensible defaults only for plain enrolled_targets.
      const isLiveData = typeof row.state === "string" && row.state !== "unknown";
      const validStates = ["ready", "leased", "quarantine", "tainted", "disabled", "degraded", "unreachable", "unknown"] as const;
      const validDockerHealth = ["unknown", "running", "pass", "fail"] as const;
      const rawState = isLiveData ? String(row.state) : "unknown";
      const state = validStates.includes(rawState as any) ? rawState as DeviceTargetView["state"] : "unknown";
      const rawDocker = typeof row.docker_health === "string" ? row.docker_health : "unknown";
      const dockerHealth = validDockerHealth.includes(rawDocker as any) ? rawDocker as DeviceTargetView["dockerHealth"] : "unknown";
      map.set(targetId, {
        targetId,
        displayName: typeof displayNameRaw === "string" && displayNameRaw.trim().length > 0
          ? displayNameRaw.trim()
          : targetId,
        mode: typeof modeRaw === "string" && modeRaw.trim().length > 0 ? modeRaw.trim() : "ssh_bootstrap",
        state,
        tainted: Boolean(row.tainted ?? row.taint_reason),
        taintReason: typeof row.taint_reason === "string" ? row.taint_reason : (typeof row.reason === "string" ? row.reason : null),
        healthScore: typeof row.health_score === "number" && row.health_score > 0 ? row.health_score : (typeof row.healthScore === "number" && row.healthScore > 0 ? row.healthScore : 1),
        latencyEwmaMs: typeof row.latency_ewma_ms === "number" && row.latency_ewma_ms > 0 ? row.latency_ewma_ms : (typeof row.latencyEwmaMs === "number" && row.latencyEwmaMs > 0 ? row.latencyEwmaMs : 50),
        recentFailureRate: typeof row.recent_failure_rate === "number" ? row.recent_failure_rate : (typeof row.recentFailureRate === "number" ? row.recentFailureRate : 0),
        dockerHealth,
        dockerPassCount: typeof row.docker_pass_count === "number" ? row.docker_pass_count : 0,
        dockerFailCount: typeof row.docker_fail_count === "number" ? row.docker_fail_count : 0,
        dockerLastRunAtUnixMs: typeof row.docker_last_run_at_unix_ms === "number" ? row.docker_last_run_at_unix_ms : null,
        updatedAtUnixMs: typeof row.updated_at_unix_ms === "number" ? row.updated_at_unix_ms : Date.now(),
      });
    }

    return Array.from(map.values());
  });

  const fleetHeartbeat = useDeviceStatus({
    commanderBaseUrl: controllerBaseUrl,
    initialTargets: initialRegistryTargets,
    leaseId: fleetLeaseId,
    heartbeatIntervalMs: 15_000,
    autoStart: false,
  });

  const fleetTargets = createMemo<DeviceTargetView[]>(() => fleetHeartbeat.targets());

  // Remove target from live fleet view when backend confirms deletion
  onMount(() => {
    const unlistenDeleted = listen<{ target_id: string }>("fleet:target_deleted", (event) => {
      fleetHeartbeat.removeTarget(event.payload.target_id);
    });
    const unlistenUpdated = listen<{ target_id: string }>("fleet:target_updated", () => {
      void appStore.loadIroncladStatus();
    });
    onCleanup(() => {
      void unlistenDeleted.then((fn) => fn());
      void unlistenUpdated.then((fn) => fn());
    });
  });

  const ocrStartupWarning = createMemo(() => {
    const info = appStore.healthInfo();
    const services = Array.isArray(info?.services) ? info!.services : [];
    const ocrSvc = services.find((svc: any) => svc?.name === "ocr_dependency");
    if (!ocrSvc) return null;

    const status = String(ocrSvc.status ?? "").toLowerCase();
    if (status === "degraded" || status === "unhealthy" || status === "stopped") {
        return String(
          ocrSvc.message ||
          "OCR dependency is unavailable. Vision analysis still works, but text extraction quality may be reduced."
        );
    }

    return null;
  });

  const shortcuts: { key: string; desc: string }[] = [
    { key: "Ctrl+,", desc: "Open settings" },
    { key: "Ctrl+N", desc: "New session" },
    { key: "Ctrl+Shift+V", desc: "Toggle voice" },
    { key: "Ctrl+K", desc: "Show shortcuts" },
    { key: "Enter", desc: "Send message" },
    { key: "Shift+Enter", desc: "New line" },
    { key: "/command", desc: "Slash commands" },
  ];

  const formatUnixMs = (value?: number | null) => {
    if (!value || Number.isNaN(value)) return "-";
    return new Date(value).toLocaleString();
  };

  const toggleControlPanelExpanded = () => {
    setControlPanelExpanded((prev) => {
      const next = !prev;
      if (typeof window !== "undefined") {
        window.localStorage.setItem(CONTROL_PANEL_EXPANDED_STORAGE_KEY, String(next));
      }
      return next;
    });
  };

  const toggleDeviceMatrix = () => {
    setShowDeviceMatrix((prev) => {
      const next = !prev;
      if (typeof window !== "undefined") {
        window.localStorage.setItem(FLEET_MATRIX_VISIBLE_STORAGE_KEY, String(next));
      }
      return next;
    });
  };

  const runFleetDockerEvals = async (targetId: string) => {
    const leaseId = fleetLeaseId();
    if (!leaseId) {
      addToast("Docker eval requires an active lease id", "error");
      return;
    }

    const commander = controllerBaseUrl();
    if (!commander) {
      addToast("Fleet commander endpoint unavailable", "error");
      return;
    }

    const endpoint = `${commander}/api/fleet/docker-evals`;
    try {
      const response = await fetch(endpoint, {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
        },
        body: JSON.stringify({
          lease_id: leaseId,
          target_id: targetId,
        }),
      });

      if (!response.ok) {
        throw new Error(`status ${response.status}`);
      }

      addToast(`Docker eval triggered for ${targetId}`, "info");
    } catch (e) {
      addToast(`Docker eval failed: ${String(e)}`, "error");
    }
  };

  const handleTargetRegistered = () => {
    addToast("Device enrolled successfully", "success");
    void appStore.loadIroncladStatus();
    fleetHeartbeat.reconnectNow();
  };

  const navigate = (next: AppRoute) => {
    setRoute(next);
    if (typeof window === "undefined") return;

    const targetHash = hashForRoute(next);
    if (window.location.hash !== targetHash) {
      window.history.pushState(null, "", targetHash);
    }

    queueMicrotask(() => {
      if (window.location.hash === targetHash && route() !== next) {
        setRoute(next);
      }
    });
  };

  const triggerSoftReset = async () => {
    try {
      await appStore.requestIroncladSoftReset(resetReason().trim() || undefined);
      addToast("Soft reset queued", "info");
    } catch (e) {
      addToast(`Soft reset failed: ${String(e)}`, "error");
    }
  };

  const triggerHardReset = async () => {
    if (hardResetConfirmation().trim() !== "HARD RESET") {
      addToast("Hard reset blocked: type HARD RESET exactly", "error");
      return;
    }

    try {
      await appStore.requestIroncladHardReset("HARD RESET", resetReason().trim() || undefined);
      addToast("Hard reset queued", "info");
      setHardResetConfirmation("");
    } catch (e) {
      addToast(`Hard reset failed: ${String(e)}`, "error");
    }
  };

  const handleGlobalKeydown = (e: KeyboardEvent) => {
    // Ctrl+, → settings
    if (e.ctrlKey && e.key === ",") {
      e.preventDefault();
      setShowSettings(true);
    }
    // Ctrl+N → new session
    if (e.ctrlKey && e.key === "n") {
      e.preventDefault();
      navigate("home");
      void appStore.createSession();
    }
    // Ctrl+Shift+V → toggle voice
    if (e.ctrlKey && e.shiftKey && e.key === "V") {
      e.preventDefault();
      appStore.toggleVoice();
    }
    // Ctrl+K → show shortcuts
    if (e.ctrlKey && e.key === "k") {
      e.preventDefault();
      setShowShortcuts((v) => !v);
    }
    // Escape → close overlays
    if (e.key === "Escape") {
      setShowShortcuts(false);
    }
  };

  const handleHashRouteChange = () => {
    if (typeof window === "undefined") return;
    setRoute(routeFromHash(window.location.hash));
  };

  onMount(() => {
    window.addEventListener("hashchange", handleHashRouteChange);
    window.addEventListener("popstate", handleHashRouteChange);
    document.addEventListener("keydown", handleGlobalKeydown);

    // Retry session hydration once the app is mounted and Tauri runtime is ready.
    void appStore.rehydrateSessionsAfterReady();

    // Check provisioning state before loading main app
    void (async () => {
      const wizardAlreadyCompleted =
        typeof window !== "undefined" &&
        window.localStorage.getItem("kria_wizard_complete") === "true";

      if (wizardAlreadyCompleted) {
        setShowWizard(false);
        setWizardLoading(false);
        return;
      }

      const state = await provisioningStore.loadState();
      if (state && state.current_step === "complete") {
        window.localStorage.setItem("kria_wizard_complete", "true");
        setShowWizard(false);
      } else if (state && state.current_step !== "complete") {
        setShowWizard(true);
      }
      setWizardLoading(false);
    })();

    appStore.loadHealth();
    appStore.loadMcpServers();
    appStore.loadAlerts();
    void appStore.loadIroncladStatus();
    void appStore.loadIroncladForensics();
  });

  createEffect(() => {
    const reset = resetSnapshot();
    if (!reset?.event_id) return;
    if (lastResetToastEventId() === reset.event_id) return;

    if (reset.phase === "in_progress") {
      addToast("Recovery reset in progress", "info");
      setLastResetToastEventId(reset.event_id);
      return;
    }

    if (reset.phase === "healthy") {
      addToast("Recovery reset completed", "success");
      setLastResetToastEventId(reset.event_id);
      return;
    }

    if (reset.phase === "failed") {
      addToast(`Recovery reset failed: ${reset.detail}`, "error");
      setLastResetToastEventId(reset.event_id);
    }
  });

  // Start fleet heartbeat as soon as a commander URL is available,
  // not just when the device matrix is visible.  This ensures targets
  // get live state (health, latency, docker) immediately on app load
  // instead of showing hardcoded "unknown" / 0% until the user navigates.
  createEffect(() => {
    if (controllerBaseUrl()) {
      fleetHeartbeat.start();
    } else {
      fleetHeartbeat.stop();
    }
  });

  onCleanup(() => {
    document.removeEventListener("keydown", handleGlobalKeydown);
    if (typeof window !== "undefined") {
      window.removeEventListener("hashchange", handleHashRouteChange);
      window.removeEventListener("popstate", handleHashRouteChange);
    }
  });

  return (
    <div class="app">
      <Show when={wizardLoading()}>
        <div class="setup-wizard">
          <div class="wizard-content">
            <div class="wizard-spinner-row">
              <div class="wizard-spinner" />
              <span>Loading…</span>
            </div>
          </div>
        </div>
      </Show>

      <Show when={!wizardLoading() && showWizard()}>
        <SetupWizard onComplete={() => setShowWizard(false)} />
      </Show>

      <Show when={!wizardLoading() && !showWizard()}>
      <div class="app-layout">
        <SessionSidebar onSessionActivated={() => navigate("home")} />
        <main class="main-content modern-main-shell">
          <div class="modern-topbar">
            <div class="modern-topbar-left">
              <div class="modern-title">KRIA</div>
              <div class="modern-subtitle">{assistantStatus().detail}</div>
            </div>
            <div class="modern-topbar-right">
              <span class={statusDotClass()} />
              <span class="modern-chip">{assistantStatus().label}</span>
              <span class="modern-chip">Routing {routingSummary()}</span>
              <span class="modern-chip">{connectedMcpServers()} MCP online</span>
              <span class="modern-chip">{appStore.alerts().length} alerts</span>
            </div>
          </div>

          <div class="modern-nav">
            <button type="button" class={`modern-nav-btn ${route() === "home" ? "active" : ""}`} onClick={() => navigate("home")}>Home</button>
            <button type="button" class={`modern-nav-btn ${route() === "dashboard" ? "active" : ""}`} onClick={() => navigate("dashboard")}>Dashboard</button>
            <button type="button" class={`modern-nav-btn ${route() === "vm-management" ? "active" : ""}`} onClick={() => navigate("vm-management")}>VM Management</button>
            <button type="button" class={`modern-nav-btn ${route() === "tasks" ? "active" : ""}`} onClick={() => navigate("tasks")}>Tasks</button>
            <button type="button" class={`modern-nav-btn ${route() === "settings" ? "active" : ""}`} onClick={() => { navigate("settings"); setShowSettings(true); }}>Settings</button>
          </div>

          <Show when={developerMode() && colabDispatchWarning()}>
            <div class="startup-warning-banner">
              <strong>Colab Routing:</strong> {colabDispatchWarning()}
            </div>
          </Show>
          <Show when={developerMode() && ocrStartupWarning()}>
            <div class="startup-warning-banner">
              <strong>OCR Warning:</strong> {ocrStartupWarning()}
            </div>
          </Show>

          {/* Per-route error isolation: a render crash in any routed view (VM
              Management / Dashboard / Settings / Home) is caught here so it can
              NEVER wedge navigation. "Back to Home" resets the boundary and
              re-navigates, guaranteeing the user can always return. */}
          <ErrorBoundary
            fallback={(err, reset) => (
              <section class="ironclad-strip route-error-boundary">
                <div class="ironclad-strip-top">
                  <div class="ironclad-strip-title">
                    <span>This view hit an error</span>
                    <span class="ironclad-strip-subtitle">{String((err && (err as Error).message) || err)}</span>
                  </div>
                  <div class="ironclad-strip-actions">
                    <button class="btn-secondary" onClick={() => reset()}>Reload view</button>
                    <button class="btn-secondary" onClick={() => { reset(); navigate("home"); }}>Back to Home</button>
                  </div>
                </div>
              </section>
            )}
          >
          <Show when={route() === "tasks"}>
            <TasksView />
          </Show>

          <Show when={route() === "dashboard"}>
          <section class={`ironclad-strip modern-dashboard ${controlPanelExpanded() ? "" : "collapsed"}`}>
            <div class="ironclad-strip-top">
              <div class="ironclad-strip-title">
                <span>Runtime Status</span>
                <span class="ironclad-strip-subtitle">Non-blocking + trust-first controls</span>
              </div>
              <div class="ironclad-strip-actions">
                <button type="button" class="btn-secondary" onClick={() => { void appStore.loadIroncladStatus(); void appStore.loadIroncladForensics(); }}>
                  Refresh
                </button>
                <button type="button" class="btn-secondary" onClick={toggleControlPanelExpanded}>
                  {controlPanelExpanded() ? "Collapse" : "Expand"}
                </button>
                <button type="button" class="btn-secondary" onClick={() => setShowTestDashboard((v) => !v)}>
                  {showTestDashboard() ? "Hide Tests" : "Tests"}
                </button>
                <Show when={dashboardView() === "overview"}>
                  <button type="button" class="btn-secondary" onClick={() => setShowAnalytics((v) => !v)}>
                    Analytics {showAnalytics() ? "▾" : "▸"}
                  </button>
                </Show>
                <Show when={controlPanelExpanded()}>
                  <button type="button" class="btn-secondary" onClick={() => setDashboardView("forensics")}>
                    Forensics
                  </button>
                </Show>
              </div>
            </div>

            <div class="modern-dashboard-tabs">
              <button type="button" class={`modern-nav-btn ${dashboardView() === "overview" ? "active" : ""}`} onClick={() => setDashboardView("overview")}>Overview</button>
              <button type="button" class={`modern-nav-btn ${dashboardView() === "operations" ? "active" : ""}`} onClick={() => setDashboardView("operations")}>Operations</button>
              <button type="button" class={`modern-nav-btn ${dashboardView() === "n8n" ? "active" : ""}`} onClick={() => setDashboardView("n8n")}>n8n</button>
              <button type="button" class={`modern-nav-btn ${dashboardView() === "forensics" ? "active" : ""}`} onClick={() => setDashboardView("forensics")}>Forensics</button>
            </div>

            <Show when={!controlPanelExpanded() && dashboardView() === "overview"}>
              <div class="ironclad-collapsed-row">
                <span class={qosTrafficLightClass()} />
                <span>Ready {ironcladStatus()?.fleet?.ready_targets ?? 0}</span>
                <span>Leased {ironcladStatus()?.fleet?.leased_targets ?? 0}</span>
                <span>{resetPhaseLabel()}</span>
              </div>
            </Show>

            <Show when={controlPanelExpanded() && dashboardView() === "overview"}>
              <div class="ironclad-metric-row">
                <div class="ironclad-card">
                  <div class="ironclad-card-label">Fleet Health</div>
                  <div class="ironclad-chip-row">
                    <span class="ironclad-chip">Ready {ironcladStatus()?.fleet?.ready_targets ?? 0}</span>
                    <span class="ironclad-chip">Leased {ironcladStatus()?.fleet?.leased_targets ?? 0}</span>
                    <span class="ironclad-chip warn">Tainted {ironcladStatus()?.fleet?.tainted_targets ?? 0}</span>
                    <span class="ironclad-chip warn">Quarantine {ironcladStatus()?.fleet?.quarantined_targets ?? 0}</span>
                    <span class="ironclad-chip">Total {ironcladStatus()?.fleet?.total_targets ?? 0}</span>
                  </div>
                </div>

                <div class="ironclad-card">
                  <div class="ironclad-card-label">Adaptive QoS</div>
                  <div class="ironclad-qos-row">
                    <span class={qosTrafficLightClass()} />
                    <span>
                      p95 {ironcladStatus()?.qos?.high_recovery_wait_p95_ms ?? 0}ms / SLO {ironcladStatus()?.qos?.high_recovery_slo_ms ?? 0}ms
                    </span>
                  </div>
                  <div class="ironclad-muted">
                    {ironcladStatus()?.qos?.reason || "No active adaptation reason"}
                  </div>
                </div>

                <div class="ironclad-card">
                  <div class="ironclad-card-label">Recovery FSM</div>
                  <div class="ironclad-reset-state">{resetPhaseLabel()}</div>
                  <div class="ironclad-muted">{resetSnapshot()?.detail || "No recent reset events"}</div>
                </div>
              </div>
            </Show>

            <Show when={controlPanelExpanded() && dashboardView() === "operations"}>
              <div class="ironclad-reset-controls">
                <div class="ironclad-control-group">
                  <label>Reset reason</label>
                  <input
                    type="text"
                    value={resetReason()}
                    onInput={(event) => setResetReason(event.currentTarget.value)}
                    placeholder="manual_recovery_check"
                    disabled={resetBusy()}
                  />
                </div>
                <div class="ironclad-control-group">
                  <label>Hard reset confirmation</label>
                  <input
                    type="text"
                    value={hardResetConfirmation()}
                    onInput={(event) => setHardResetConfirmation(event.currentTarget.value)}
                    placeholder="Type HARD RESET"
                    disabled={resetBusy()}
                  />
                </div>
                <button class="btn-secondary" disabled={resetBusy()} onClick={triggerSoftReset}>
                  Soft Reset
                </button>
                <button class="btn-danger" disabled={resetBusy()} onClick={triggerHardReset}>
                  Hard Reset
                </button>
              </div>
            </Show>

            <Show when={dashboardView() === "forensics" && controlPanelExpanded()}>
                <div class="ironclad-forensics-panel">
                  <div class="ironclad-forensics-head">
                    <strong>Forensic Audit</strong>
                    <span>{appStore.ironcladForensicsTotal()} records</span>
                  </div>

                  <Show when={forensicEntries().length > 0} fallback={<div class="ironclad-muted">No forensic records yet.</div>}>
                    <For each={forensicEntries()}>
                      {(record) => (
                        <div class="ironclad-forensic-entry">
                          <div class="ironclad-forensic-summary">
                            <span class={`ironclad-severity ${record.severity}`}>{record.severity}</span>
                            <span>{record.summary}</span>
                          </div>
                          <div class="ironclad-forensic-meta">
                            <span>{formatUnixMs(record.timestamp_unix_ms)}</span>
                            <span>{record.category}</span>
                            <span>{record.source}</span>
                            <Show when={record.last_gasp_detected}>
                              <span class="ironclad-last-gasp">last gasp</span>
                            </Show>
                          </div>
                          <details>
                            <summary>Evidence</summary>
                            <pre>{record.evidence}</pre>
                          </details>
                        </div>
                      )}
                    </For>
                  </Show>

                  <div class="ironclad-muted">
                    Last reset start: {formatUnixMs(resetSnapshot()?.started_unix_ms)} | Last completion: {formatUnixMs(resetSnapshot()?.completed_unix_ms)}
                  </div>
                </div>
            </Show>

            <Show when={dashboardView() === "n8n" && controlPanelExpanded()}>
              <Suspense fallback={<div class="status-pill subtle">Loading n8n…</div>}>
                <N8nDashboard />
              </Suspense>
            </Show>
          </section>
          </Show>

          <Show when={route() === "vm-management"}>
            <section class="ironclad-strip">
              <div class="ironclad-strip-top">
                <div class="ironclad-strip-title">
                  <span>VM Management</span>
                  <span class="ironclad-strip-subtitle">Device orchestration and operations</span>
                </div>
                <div class="ironclad-strip-actions">
                  <button class="btn-secondary" onClick={() => setShowAddTargetModal(true)}>Add Target</button>
                  <button class="btn-secondary" onClick={fleetHeartbeat.reconnectNow}>Reconnect</button>
                  <button class="btn-secondary" onClick={toggleDeviceMatrix}>
                    {showDeviceMatrix() ? "Hide Matrix" : "Show Matrix"}
                  </button>
                </div>
              </div>
            </section>
            <Show when={showDeviceMatrix()}>
              <Suspense fallback={<div class="status-pill subtle">Loading VM matrix…</div>}>
                <DeviceMatrix
                  title="Live Orchestration Matrix"
                  fleet={fleetTargets()}
                  focusedTerminalTargetId={fleetHeartbeat.focusedTargetId()}
                  terminalLines={fleetHeartbeat.focusedTerminalLines()}
                  alerts={fleetHeartbeat.alerts()}
                  streamState={fleetHeartbeat.streamState()}
                  lastHeartbeatAtUnixMs={fleetHeartbeat.lastHeartbeatAtUnixMs()}
                  leaseHealthy={fleetHeartbeat.leaseHealthy()}
                  lastError={fleetHeartbeat.lastError()}
                  onAddTarget={() => setShowAddTargetModal(true)}
                  onReconnectStreams={fleetHeartbeat.reconnectNow}
                  onFocusTerminal={fleetHeartbeat.focusTarget}
                  onRunDockerEvals={runFleetDockerEvals}
                  dockerActionDisabled={!fleetLeaseId()}
                  lastTestResultByTarget={fleetHeartbeat.lastTestResultByTarget}
                  deletingTargetIds={deletingTargetIds()}
                  onDeleteTarget={async (targetId) => {
                    if (deletingTargetIds().has(targetId)) return;
                    setDeletingTargetIds((prev) => new Set(prev).add(targetId));
                    try {
                      await appStore.deleteTarget(targetId);
                      fleetHeartbeat.removeTarget(targetId);
                      addToast("Target deleted successfully", "success");
                    } catch (e: any) {
                      addToast(`Failed to delete target: ${e?.message ?? e}`, "error");
                    } finally {
                      setDeletingTargetIds((prev) => {
                        const next = new Set(prev);
                        next.delete(targetId);
                        return next;
                      });
                    }
                  }}
                  onEditTarget={(target) => {
                    setEditingTarget(target);
                    setShowEditTargetModal(true);
                  }}
                />
              </Suspense>
            </Show>
          </Show>

          <Show when={route() === "dashboard" && controlPanelExpanded() && showTestDashboard()}>
            <Suspense fallback={<div class="status-pill subtle">Loading tests…</div>}>
              <TestRunnerDashboard />
            </Suspense>
          </Show>

          <Show when={route() === "dashboard" && dashboardView() === "overview" && showAnalytics()}>
            <Suspense fallback={<div class="status-pill subtle">Loading analytics…</div>}>
              <AnalyticsDashboard />
            </Suspense>
          </Show>

          <Show when={route() === "home" && currentEnvironment() === "assistant"}>
            <ChatView />
          </Show>
          <Show when={route() === "home" && currentEnvironment() === "prompt_lab"}>
            <PromptLabView />
          </Show>
          <Show when={route() === "settings"}>
            <section class="ironclad-strip">
              <div class="ironclad-strip-top">
                <div class="ironclad-strip-title">
                  <span>Settings</span>
                  <span class="ironclad-strip-subtitle">Use the settings panel for full configuration</span>
                </div>
                <div class="ironclad-strip-actions">
                  <button class="btn-secondary" onClick={() => setShowSettings(true)}>Open Settings Panel</button>
                  <button class="btn-secondary" onClick={() => navigate("home")}>Back to Home</button>
                </div>
              </div>
            </section>
          </Show>
          </ErrorBoundary>
          <div class="status-bar modern-statusbar">
            <div class="status-item">
              <span class={statusDotClass()} />
              <span>{assistantStatus().label}</span>
            </div>
            <div class="status-item">
              <span>Core: {assistantStatus().detail}</span>
            </div>
            <div class="status-item">
              <span>MCP: {connectedMcpServers()} online</span>
            </div>
            <div class="status-item">
              <span>Routing: {routingSummary()}</span>
            </div>
            <div class="status-item">
              <span>Theme: {appStore.theme()}</span>
            </div>
          </div>
        </main>
      </div>

      <Show when={showSettings()}>
        <SettingsModal />
      </Show>

      <Show when={showAddTargetModal()}>
        <AddTargetModal
          onClose={() => setShowAddTargetModal(false)}
          onRegistered={() => {
            setShowAddTargetModal(false);
            handleTargetRegistered();
          }}
        />
      </Show>

      <Show when={showEditTargetModal() && editingTarget()}>
        <EditTargetModal
          target={editingTarget()!}
          enrolledTarget={(ironcladStatus() as any)?.fleet?.enrolled_targets?.find(
            (t: any) => t.target_id === editingTarget()?.targetId
          )}
          onClose={() => {
            setShowEditTargetModal(false);
            setEditingTarget(null);
          }}
          onUpdated={async (request) => {
            try {
              await appStore.updateTarget(request);
              addToast("Target updated successfully", "success");
              setShowEditTargetModal(false);
              setEditingTarget(null);
            } catch (e: any) {
              addToast(`Failed to update target: ${e?.message ?? e}`, "error");
            }
          }}
        />
      </Show>

      <Show when={showHitl()}>
        <HitlModal />
      </Show>

      <DecisionActionCenter />

      <Show when={voiceActive()}>
        <VoiceOverlay />
      </Show>

      <Show when={showVoiceOnboarding()}>
        <VoiceOnboarding />
      </Show>

      {/* Keyboard shortcuts overlay */}
      <Show when={showShortcuts()}>
        <div class="shortcuts-overlay" onClick={() => setShowShortcuts(false)}>
          <div class="shortcuts-panel" onClick={(e) => e.stopPropagation()}>
            <h2>Keyboard Shortcuts</h2>
            {shortcuts.map((s) => (
              <div class="shortcut-row">
                <span>{s.desc}</span>
                <span class="shortcut-key">{s.key}</span>
              </div>
            ))}
          </div>
        </div>
      </Show>
      </Show>

      {/* Toast notifications */}
      <div class="toast-container">
        <For each={toasts()}>
          {(toast) => (
            <div class={`toast toast-${toast.type}`}>
              {toast.message}
            </div>
          )}
        </For>
      </div>
    </div>
  );
};

export default App;
