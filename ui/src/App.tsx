import { Component, Show, For, createSignal, createMemo, createEffect, onMount, onCleanup } from "solid-js";
import { appStore } from "./stores/app";
import { provisioningStore } from "./stores/provisioning";
import ChatView from "./components/ChatView";
import FleetMatrix from "./components/FleetMatrix";
import AddTargetModal from "./components/AddTargetModal";
import PromptLabView from "./components/PromptLabView";
import SessionSidebar from "./components/SessionSidebar";
import SettingsModal from "./components/SettingsModal";
import HitlModal from "./components/HitlModal";
import VoiceOverlay from "./components/VoiceOverlay";
import SetupWizard from "./components/SetupWizard";
import { FleetTargetView, useFleetHeartbeat } from "./hooks/useFleetHeartbeat";

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
const IRONCLAD_EXPANDED_STORAGE_KEY = "kria_ironclad_expanded";
const FLEET_MATRIX_VISIBLE_STORAGE_KEY = "kria_fleet_matrix_visible";

const App: Component = () => {
  const {
    showSettings,
    showHitl,
    voiceActive,
    setShowSettings,
    currentEnvironment,
    colabDispatchWarning,
  } = appStore;
  const [showShortcuts, setShowShortcuts] = createSignal(false);
  const [showWizard, setShowWizard] = createSignal(false);
  const [wizardLoading, setWizardLoading] = createSignal(true);
  const [showForensics, setShowForensics] = createSignal(false);
  const initialIroncladExpanded =
    typeof window === "undefined"
      ? false
      : window.localStorage.getItem(IRONCLAD_EXPANDED_STORAGE_KEY) === "true";
  const initialFleetMatrixVisible =
    typeof window === "undefined"
      ? true
      : window.localStorage.getItem(FLEET_MATRIX_VISIBLE_STORAGE_KEY) !== "false";
  const [ironcladExpanded, setIroncladExpanded] = createSignal<boolean>(initialIroncladExpanded);
  const [showFleetMatrix, setShowFleetMatrix] = createSignal<boolean>(initialFleetMatrixVisible);
  const [showAddTargetModal, setShowAddTargetModal] = createSignal(false);
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

  const commanderBaseUrl = createMemo<string | null>(() => {
    const status = ironcladStatus() as Record<string, any> | null;
    const settings = appStore.settings() as Record<string, any> | null;
    const candidates: unknown[] = [
      status?.fleet?.pool_packet?.commander_base_url,
      status?.fleet?.pool_packet?.commanderBaseUrl,
      status?.fleet?.commander_base_url,
      status?.fleet?.commanderBaseUrl,
      status?.commander_base_url,
      status?.commanderBaseUrl,
      settings?.ironclad?.commander_url,
      settings?.ironclad?.commanderUrl,
      settings?.ironclad?.commander_base_url,
      settings?.ironclad?.commanderBaseUrl,
      settings?.fleet?.commander_base_url,
      settings?.fleet?.commanderBaseUrl,
      settings?.fleet?.commander_url,
      settings?.fleet?.commanderUrl,
      settings?.server?.commander_base_url,
      settings?.server?.commanderBaseUrl,
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

  const fleetHeartbeat = useFleetHeartbeat({
    commanderBaseUrl,
    leaseId: fleetLeaseId,
    heartbeatIntervalMs: 15_000,
    autoStart: false,
  });

  const fleetTargets = createMemo<FleetTargetView[]>(() => fleetHeartbeat.targets());

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

  const toggleIroncladExpanded = () => {
    setIroncladExpanded((prev) => {
      const next = !prev;
      if (typeof window !== "undefined") {
        window.localStorage.setItem(IRONCLAD_EXPANDED_STORAGE_KEY, String(next));
      }
      return next;
    });
  };

  const toggleFleetMatrix = () => {
    setShowFleetMatrix((prev) => {
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

    const commander = commanderBaseUrl();
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
    addToast("Soldier enrolled successfully", "success");
    void appStore.loadIroncladStatus();
    fleetHeartbeat.reconnectNow();
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
      appStore.createSession();
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

  onMount(() => {
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

  createEffect(() => {
    const shouldStreamFleet = ironcladExpanded() && showFleetMatrix();
    if (shouldStreamFleet) {
      fleetHeartbeat.start();
      return;
    }
    fleetHeartbeat.stop();
  });

  onCleanup(() => {
    document.removeEventListener("keydown", handleGlobalKeydown);
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
        <SessionSidebar />
        <main class="main-content">
          <div class="assistant-header">
            <div>
              <div class="assistant-header-kicker">Adaptive Workspace Assistant</div>
              <h1>KRIA Command Center</h1>
              <p>{assistantStatus().detail}</p>
            </div>
            <div class="assistant-header-chips">
              <div class="status-pill">
                <span class={statusDotClass()} />
                <span>{assistantStatus().label}</span>
              </div>
              <div class="status-pill subtle">Routing {routingSummary()}</div>
              <div class="status-pill subtle">{connectedMcpServers()} MCP online</div>
              <div class="status-pill subtle">{appStore.alerts().length} active alerts</div>
            </div>
          </div>
          <Show when={colabDispatchWarning()}>
            <div class="startup-warning-banner">
              <strong>Colab Routing:</strong> {colabDispatchWarning()}
            </div>
          </Show>
          <Show when={ocrStartupWarning()}>
            <div class="startup-warning-banner">
              <strong>OCR Warning:</strong> {ocrStartupWarning()}
            </div>
          </Show>

          <section class={`ironclad-strip ${ironcladExpanded() ? "" : "collapsed"}`}>
            <div class="ironclad-strip-top">
              <div class="ironclad-strip-title">
                <span>Ironclad Runtime</span>
                <span class="ironclad-strip-subtitle">Non-blocking + trust-first controls</span>
              </div>
              <div class="ironclad-strip-actions">
                <button class="btn-secondary" onClick={() => { void appStore.loadIroncladStatus(); void appStore.loadIroncladForensics(); }}>
                  Refresh
                </button>
                <button class="btn-secondary" onClick={toggleIroncladExpanded}>
                  {ironcladExpanded() ? "Collapse" : "Expand"}
                </button>
                <Show when={ironcladExpanded()}>
                  <button class="btn-secondary" onClick={toggleFleetMatrix}>
                    {showFleetMatrix() ? "Hide Fleet" : "Show Fleet"}
                  </button>
                  <button class="btn-secondary" onClick={() => setShowForensics((v) => !v)}>
                    {showForensics() ? "Hide Forensics" : "View Forensics"}
                  </button>
                </Show>
              </div>
            </div>

            <Show when={!ironcladExpanded()}>
              <div class="ironclad-collapsed-row">
                <span class={qosTrafficLightClass()} />
                <span>Ready {ironcladStatus()?.fleet?.ready_targets ?? 0}</span>
                <span>Leased {ironcladStatus()?.fleet?.leased_targets ?? 0}</span>
                <span>{resetPhaseLabel()}</span>
              </div>
            </Show>

            <Show when={ironcladExpanded()}>
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

              <Show when={showForensics()}>
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
            </Show>
          </section>

          <Show when={ironcladExpanded() && showFleetMatrix()}>
            <FleetMatrix
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
            />
          </Show>

          <Show when={currentEnvironment() === "assistant"}>
            <ChatView />
          </Show>
          <Show when={currentEnvironment() === "prompt_lab"}>
            <PromptLabView />
          </Show>
          <div class="status-bar">
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
              <span>{appStore.theme() === "dark" ? "🌙" : "☀️"}</span>
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

      <Show when={showHitl()}>
        <HitlModal />
      </Show>

      <Show when={voiceActive()}>
        <VoiceOverlay />
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
