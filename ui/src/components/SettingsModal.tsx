import { Component, Show, For, createSignal, createEffect, createMemo, onMount, onCleanup } from "solid-js";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { appStore } from "../stores/app";
import { SUPPORTED_LANGUAGES, setLocale } from "../stores/i18n";
import SkillMarketplace from "./SkillMarketplace";
import SubstrateStatus from "./SubstrateStatus";
import ProviderSettings from "./ProviderSettings";
import N8nSettings from "./N8nSettings";

type Tab = "llm" | "voice" | "safety" | "ui" | "assistant" | "labs" | "search" | "services" | "telegram" | "n8n" | "automation" | "gui_automation" | "hardware" | "knowledge" | "google" | "colab" | "ironclad" | "marketplace" | "developer";
type SettingsLayer = "basic" | "workflow" | "integrations" | "advanced" | "developer";

interface SettingsTabDefinition {
  id: Tab;
  label: string;
  icon: string;
  description: string;
  layer: SettingsLayer;
}

const SETTINGS_LAYERS: { id: SettingsLayer; label: string; description: string }[] = [
  { id: "basic", label: "Basic", description: "Everyday preferences and core assistant setup." },
  { id: "workflow", label: "Workflow", description: "Automation, GUI control, and active task behavior." },
  { id: "integrations", label: "Integrations", description: "Connected apps, providers, and skill surfaces." },
  { id: "advanced", label: "Advanced", description: "Hardware, knowledge, and preview capabilities." },
  { id: "developer", label: "Developer", description: "Diagnostics, fleet controls, and high-risk operations." },
];

interface AssistantFrontendPrefs {
  persona: "operator" | "coach" | "researcher" | "chief_of_staff";
  verbosity: "compact" | "balanced" | "deep";
  proactiveSuggestions: boolean;
  missionBriefings: boolean;
  followupQuestions: boolean;
  smartSessionTitles: boolean;
}

interface LabsFrontendPrefs {
  missionBoard: boolean;
  workflowCanvas: boolean;
  mcpMarketplace: boolean;
  autoPilotQueue: boolean;
  contextMap: boolean;
}

interface McpCatalogItem {
  id: string;
  name: string;
  description: string;
  trust: "GREEN" | "YELLOW" | "RED";
  enabled: boolean;
}

const FONT_SCALE_OPTIONS = [
  { value: "0.8", label: "Small (80%)" },
  { value: "0.9", label: "Compact (90%)" },
  { value: "1.0", label: "Normal (100%)" },
  { value: "1.2", label: "Large (120%)" },
  { value: "1.5", label: "Extra Large (150%)" },
  { value: "2.0", label: "Huge (200%)" },
] as const;

function normalizeFontScaleValue(value: unknown): string {
  const parsed = Number.parseFloat(String(value ?? "1"));
  if (Number.isNaN(parsed)) return "1.0";

  const matched = FONT_SCALE_OPTIONS.find(
    (option) => Math.abs(Number.parseFloat(option.value) - parsed) < 0.001
  );

  return matched?.value ?? "1.0";
}

const SettingsModal: Component = () => {
  const { setShowSettings, settings, loadSettings, saveSettings, models, loadModels, audioDevices, loadAudioDevices, theme, applyTheme, mcpServers, loadMcpServers, addMcpServer, removeMcpServer, toggleMcpServer, healthInfo, loadHealth, scheduledTasks, loadScheduledTasks, addScheduledTask, removeScheduledTask, macros, loadMacros, deleteMacro, workflows, loadWorkflows, deleteWorkflow, hardwareInfo, loadHardwareInfo, knowledgeBase, loadKnowledgeBase, sessions, clearAllChatSessions, telegramConfig, telegramBotInfo, loadTelegramConfig, saveTelegramConfig, testTelegramConnection, startTelegramMcp, stopTelegramMcp, googleStatus, loadGoogleStatus, setGoogleAccount, connectGoogle, disconnectGoogle, colabStatus, loadColabStatus, connectColab, disconnectColab, setColabNotebook, reconcileMcpRuntime, restartMcpServerRuntime, ironcladStatus, loadIroncladStatus, getIroncladConfig, updateIroncladConfig, requestIroncladSoftReset, requestIroncladHardReset, loadIroncladForensics, ironcladForensicsTotal, developerMode, setDeveloperMode } = appStore;

  const [activeTab, setActiveTab] = createSignal<Tab>("llm");
  const [activeLayer, setActiveLayer] = createSignal<SettingsLayer>("basic");
  const [draft, setDraft] = createSignal<Record<string, any>>({});
  const [saving, setSaving] = createSignal(false);
  const [error, setError] = createSignal("");
  const [success, setSuccess] = createSignal("");
  const [clearChatsBusy, setClearChatsBusy] = createSignal(false);
  const [clearChatsConfirm, setClearChatsConfirm] = createSignal(false);
  let dialogEl: HTMLDivElement | undefined;

  // MCP add server form
  const [newServerName, setNewServerName] = createSignal("");
  const [newServerCommand, setNewServerCommand] = createSignal("");
  const [newServerArgs, setNewServerArgs] = createSignal("");
  const [newServerTrust, setNewServerTrust] = createSignal("YELLOW");
  const [mcpFilter, setMcpFilter] = createSignal("");
  const [mcpGroupBy, setMcpGroupBy] = createSignal<"state" | "trust" | "tag">("state");
  const [mcpPage, setMcpPage] = createSignal(1);
  const MCP_PAGE_SIZE = 12;

  // Automation form state
  const [newTaskName, setNewTaskName] = createSignal("");
  const [newTaskInterval, setNewTaskInterval] = createSignal("3600");
  const [newTaskPrompt, setNewTaskPrompt] = createSignal("");

  // Telegram form state
  const [tgBotToken, setTgBotToken] = createSignal("");
  const [tgChatIds, setTgChatIds] = createSignal("");
  const [tgAutoStart, setTgAutoStart] = createSignal(true);
  const [tgTesting, setTgTesting] = createSignal(false);
  const [tgTestResult, setTgTestResult] = createSignal<string | null>(null);
  const [tgSaving, setTgSaving] = createSignal(false);

  // Google Workspace state
  const [gwAccount, setGwAccount] = createSignal("personal");
  const [gwConnecting, setGwConnecting] = createSignal(false);
  const [gwPollTimer, setGwPollTimer] = createSignal<ReturnType<typeof setInterval> | null>(null);
  const [gwMessage, setGwMessage] = createSignal("");

  // Colab tier state
  const [colabServerName, setColabServerName] = createSignal("colab-mcp");
  const [colabNotebookId, setColabNotebookId] = createSignal("");
  const [colabBusy, setColabBusy] = createSignal(false);
  const [colabMessage, setColabMessage] = createSignal("");

  // RFC 008: GUI Automation state
  type GuiAutomationStatus = {
    vision_sidecar: string;
    uinput_daemon: string;
    automation_enabled: boolean;
    global_halt_engaged: boolean;
    halt_kind: string;
    halt_reason: string | null;
    release_conditions: string[];
    vision_pid: number | null;
    uinput_pid: number | null;
    orchestrator_available: boolean;
    session_type: string;
    selected_backend: string;
    backend_selection_reason: string;
    backend_probe_status: string;
    backend_probe_errors: string[];
    xdotool_available: boolean;
    xdotool_usable_for_actions: boolean;
    ydotool_available: boolean;
    ydotool_usable_for_actions: boolean;
    uinput_socket_path: string | null;
    uinput_socket_accessible: boolean;
    can_execute_actions: boolean;
  };
  const [guiAutomationStatus, setGuiAutomationStatus] = createSignal<GuiAutomationStatus | null>(null);
  const [guiAutomationBusy, setGuiAutomationBusy] = createSignal(false);
  const [guiAutomationError, setGuiAutomationError] = createSignal("");
  // Developer/test toggle: bypass the GUI Cognition readiness safety gate so
  // live actions run on the first prompt (no `safety_only` downgrade).
  const [guiReadinessBypass, setGuiReadinessBypass] = createSignal(false);
  const [guiReadinessBusy, setGuiReadinessBusy] = createSignal(false);

  async function loadGuiAutomationStatus() {
    try {
      const status = await invoke<GuiAutomationStatus>("get_gui_automation_status");
      setGuiAutomationStatus(status);
      setGuiAutomationError("");
    } catch (e: any) {
      setGuiAutomationError(String(e?.message ?? e));
    }
    try {
      const bypass = await invoke<boolean>("get_gui_cognition_readiness_bypass");
      setGuiReadinessBypass(Boolean(bypass));
    } catch {
      /* non-fatal: leave previous value */
    }
  }

  async function toggleGuiAutomation(enabled: boolean) {
    setGuiAutomationBusy(true);
    setGuiAutomationError("");
    try {
      const status = await invoke<GuiAutomationStatus>("set_gui_automation_enabled", { enabled });
      setGuiAutomationStatus(status);
    } catch (e: any) {
      setGuiAutomationError(String(e?.message ?? e));
    } finally {
      setGuiAutomationBusy(false);
    }
  }

  async function toggleGuiReadinessBypass(enabled: boolean) {
    setGuiReadinessBusy(true);
    try {
      const next = await invoke<boolean>("set_gui_cognition_readiness_bypass", { enabled });
      setGuiReadinessBypass(Boolean(next));
    } catch (e: any) {
      setGuiAutomationError(String(e?.message ?? e));
    } finally {
      setGuiReadinessBusy(false);
    }
  }

  // Poll GUI automation status when its tab is active
  createEffect(() => {
    if (activeTab() !== "gui_automation") return;
    let cancelled = false;
    void loadGuiAutomationStatus();
    const timer = setInterval(() => {
      if (!cancelled) void loadGuiAutomationStatus();
    }, 2000);
    onCleanup(() => {
      cancelled = true;
      clearInterval(timer);
    });
  });

  // Ironclad state
  const [ironcladHighRecoverySlo, setIroncladHighRecoverySlo] = createSignal("500");
  const [ironcladLeaseTtl, setIroncladLeaseTtl] = createSignal("120000");
  const [ironcladHeartbeatGrace, setIroncladHeartbeatGrace] = createSignal("5000");
  const [ironcladQuarantineCooldown, setIroncladQuarantineCooldown] = createSignal("60000");
  const [ironcladHashDistance, setIroncladHashDistance] = createSignal("0.2");
  const [ironcladResetReason, setIroncladResetReason] = createSignal("");
  const [ironcladHardResetPhrase, setIroncladHardResetPhrase] = createSignal("");
  const [ironcladBusy, setIroncladBusy] = createSignal(false);
  const [ironcladMessage, setIroncladMessage] = createSignal("");

  // Frontend-only assistant/labs preferences
  const [assistantPrefs, setAssistantPrefs] = createSignal<AssistantFrontendPrefs>({
    persona: "chief_of_staff",
    verbosity: "balanced",
    proactiveSuggestions: true,
    missionBriefings: true,
    followupQuestions: true,
    smartSessionTitles: true,
  });
  const [labsPrefs, setLabsPrefs] = createSignal<LabsFrontendPrefs>({
    missionBoard: true,
    workflowCanvas: false,
    mcpMarketplace: true,
    autoPilotQueue: false,
    contextMap: false,
  });
  const [mcpCatalog, setMcpCatalog] = createSignal<McpCatalogItem[]>([
    {
      id: "gmail-ops",
      name: "Gmail Operations",
      description: "Summaries, triage suggestions, and thread drafting tools.",
      trust: "YELLOW",
      enabled: true,
    },
    {
      id: "calendar-orchestrator",
      name: "Calendar Orchestrator",
      description: "Planning blocks, conflict resolution, and follow-up scheduling.",
      trust: "GREEN",
      enabled: false,
    },
    {
      id: "docs-briefing-kit",
      name: "Docs Briefing Kit",
      description: "Extract action items, owners, and deadlines from docs.",
      trust: "YELLOW",
      enabled: false,
    },
    {
      id: "ops-sentinel",
      name: "Ops Sentinel",
      description: "Watchdog connector for system/service event streams.",
      trust: "RED",
      enabled: false,
    },
  ]);

  const closeSettings = () => setShowSettings(false);

  onMount(() => {
    let disposed = false;
    let unlistenConnected: (() => void) | null = null;
    let unlistenError: (() => void) | null = null;
    let unlistenNotice: (() => void) | null = null;
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        closeSettings();
      }
    };

    window.addEventListener("keydown", handleKeyDown);
    queueMicrotask(() => {
      dialogEl?.focus();
    });

    const initialize = async () => {
      await loadSettings();
      await loadModels();
      await loadAudioDevices();
      await loadMcpServers();
      await loadHealth();
      await loadScheduledTasks();
      await loadMacros();
      await loadWorkflows();
      await loadHardwareInfo();
      await loadKnowledgeBase();
      await loadTelegramConfig();
      const initialGoogleStatus = await loadGoogleStatus();
      const initialColabStatus = await loadColabStatus();
      await loadIroncladStatus();
      await hydrateIroncladConfig();
      await loadIroncladForensics(64);
      if (disposed) return;
      if (initialGoogleStatus?.account) {
        setGwAccount(initialGoogleStatus.account);
      }
      if (initialColabStatus?.mcp_server_name) {
        setColabServerName(initialColabStatus.mcp_server_name);
      }
      if (initialColabStatus?.selected_notebook) {
        setColabNotebookId(initialColabStatus.selected_notebook);
      }

      // Restore frontend-only preferences from local storage.
      try {
        const assistantRaw = localStorage.getItem("kria_assistant_frontend_prefs");
        if (assistantRaw) {
          setAssistantPrefs({ ...assistantPrefs(), ...JSON.parse(assistantRaw) });
        }
        const labsRaw = localStorage.getItem("kria_labs_frontend_prefs");
        if (labsRaw) {
          setLabsPrefs({ ...labsPrefs(), ...JSON.parse(labsRaw) });
        }
        const catalogRaw = localStorage.getItem("kria_mcp_catalog");
        if (catalogRaw) {
          const parsed = JSON.parse(catalogRaw);
          if (Array.isArray(parsed)) {
            setMcpCatalog(parsed as McpCatalogItem[]);
          }
        }
      } catch (e) {
        console.warn("Failed to restore frontend preferences:", e);
      }

      if (disposed) return;

      // Listen for OAuth completion events from Tauri backend.
      unlistenConnected = await listen("gw:connected", async (_event: any) => {
        setGwConnecting(false);
        setGwMessage("");
        const pol = gwPollTimer();
        if (pol) {
          clearInterval(pol);
          setGwPollTimer(null);
        }
        try {
          await reconcileMcpRuntime();
        } catch (e) {
          console.warn("Failed to reconcile MCP runtime after Google connect:", e);
        }
        await loadMcpServers();
        await loadGoogleStatus(gwAccount());
      });

      if (disposed) {
        unlistenConnected?.();
        return;
      }

      unlistenError = await listen("gw:error", (event: any) => {
        setGwConnecting(false);
        const pol = gwPollTimer();
        if (pol) {
          clearInterval(pol);
          setGwPollTimer(null);
        }
        setGwMessage(`Authorization failed: ${event.payload?.message ?? "unknown error"}`);
      });

      if (disposed) {
        unlistenError?.();
        return;
      }

      unlistenNotice = await listen("gw:notice", (event: any) => {
        const message = event.payload?.message ?? "";
        if (message) {
          setGwMessage(message);
        }
      });

      if (disposed) {
        unlistenNotice?.();
      }
    };

    void initialize();

    onCleanup(() => {
      disposed = true;
      window.removeEventListener("keydown", handleKeyDown);
      unlistenConnected?.();
      unlistenError?.();
      unlistenNotice?.();
      const pol = gwPollTimer();
      if (pol) clearInterval(pol);
    });
  });

  // Sync draft from loaded settings
  createEffect(() => {
    const s = settings();
    if (s) setDraft(JSON.parse(JSON.stringify(s)));
  });

  // Sync telegram form from loaded config
  createEffect(() => {
    const tg = telegramConfig();
    if (tg) {
      setTgBotToken(tg.bot_token);
      setTgChatIds(tg.allowed_chat_ids);
      setTgAutoStart(tg.auto_start);
    }
  });

  createEffect(() => {
    const status = googleStatus();
    if (status?.account && !gwConnecting()) {
      setGwAccount(status.account);
    }
  });

  createEffect(() => {
    const status = colabStatus();
    if (!status || colabBusy()) return;
    if (status.mcp_server_name) {
      setColabServerName(status.mcp_server_name);
    }
    if (status.selected_notebook) {
      setColabNotebookId(status.selected_notebook);
    }
  });

  createEffect(() => {
    localStorage.setItem("kria_assistant_frontend_prefs", JSON.stringify(assistantPrefs()));
  });

  createEffect(() => {
    localStorage.setItem("kria_labs_frontend_prefs", JSON.stringify(labsPrefs()));
  });

  createEffect(() => {
    localStorage.setItem("kria_mcp_catalog", JSON.stringify(mcpCatalog()));
  });

  const updateField = (section: string, field: string, value: any) => {
    setDraft((prev) => ({
      ...prev,
      [section]: { ...prev[section], [field]: value },
    }));
  };

  const handleSave = async () => {
    setSaving(true);
    setError("");
    setSuccess("");
    try {
      await saveSettings(draft());
      setSuccess("Settings saved");
      setTimeout(() => setSuccess(""), 2000);
    } catch (e) {
      setError(`Failed to save: ${e}`);
    } finally {
      setSaving(false);
    }
  };

  const formatUptime = (secs: number): string => {
    const h = Math.floor(secs / 3600);
    const m = Math.floor((secs % 3600) / 60);
    if (h > 0) return `${h}h ${m}m`;
    if (m > 0) return `${m}m`;
    return `${secs}s`;
  };

  const formatInterval = (secs: number): string => {
    if (secs >= 86400) return `${Math.round(secs / 86400)}d`;
    if (secs >= 3600) return `${Math.round(secs / 3600)}h`;
    if (secs >= 60) return `${Math.round(secs / 60)}m`;
    return `${secs}s`;
  };
  const formatUnixMs = (value?: number | null): string => {
    if (!value || Number.isNaN(value)) return "-";
    return new Date(value).toLocaleString();
  };

  const runtimeDotClass = (state?: string): "running" | "stopped" | "" => {
    const normalized = String(state ?? "").toLowerCase();
    if (normalized === "running") return "running";
    if (normalized === "starting") return "";
    return "stopped";
  };

  const runtimeStateLabel = (state?: string): string => {
    const normalized = String(state ?? "stopped").toLowerCase();
    if (normalized === "starting") return "starting";
    if (normalized === "running") return "running";
    if (normalized === "error") return "error";
    return "stopped";
  };

  const googleStatusMessage = (): string => {
    const status = googleStatus();
    if (!status) {
      return "Checking Google integration status...";
    }
    if (status.connected) {
      return `Connected as ${status.account}`;
    }
    if (status.auth_ready && !status.runtime_ready) {
      return `OAuth is ready, but runtime is unavailable (state=${status.mcp?.state ?? "unknown"}).`;
    }
    if (!status.credentials_configured) {
      return "OAuth credentials are missing.";
    }
    if (status.requires_reauth) {
      return "Account registry exists without a token. Reconnect to re-auth.";
    }
    if (!status.token_present) {
      return `OAuth token is missing for account '${status.account}'.`;
    }
    return "Google integration is not ready.";
  };

  const googleCapabilityEntries = () => {
    const capabilities = googleStatus()?.capabilities;
    if (!capabilities) return [];
    return [
      ["Gmail", capabilities.gmail],
      ["Calendar", capabilities.calendar],
      ["Drive", capabilities.drive],
      ["Docs", capabilities.docs],
      ["Sheets", capabilities.sheets],
      ["Slides", capabilities.slides],
      ["Forms", capabilities.forms],
      ["Meet (direct)", capabilities.meet],
      ["Meet via Calendar", capabilities.meet_via_calendar],
    ] as const;
  };

  const colabStatusMessage = (): string => {
    const status = colabStatus();
    if (!status) {
      return "Checking Colab tier status...";
    }
    if (!status.enabled) {
      return "Colab tier is disabled.";
    }
    if (status.ready_for_cloud_task) {
      return `Ready on ${status.mcp_server_name}${status.selected_notebook ? ` (${status.selected_notebook})` : ""}`;
    }
    if (status.notebook_selection_required) {
      return "Connected. Select an active notebook to run prompts.";
    }
    if (status.runtime_state === "awaiting_browser_connection") {
      return "Waiting for Colab browser session/tool discovery.";
    }
    return `Colab is not ready (state=${status.runtime_state}).`;
  };

  const colabCapabilityEntries = () => {
    const features = colabStatus()?.capabilities?.features;
    if (!features) return [];
    return [
      ["Notebook discovery", features.notebook_discovery],
      ["Notebook selection", features.notebook_selection],
      ["Cell execution", features.cell_execution],
      ["Artifacts IO", features.artifact_io],
      ["Runtime lifecycle", features.runtime_lifecycle],
      ["Package management", features.package_management],
      ["Checkpointing", features.checkpointing],
    ] as const;
  };

  const colabDiscoveredTools = () => colabStatus()?.capabilities?.discovered_tools ?? [];
  const selectedFontScale = () => normalizeFontScaleValue(draft()?.ui?.font_scale);
  const filteredMcpServers = createMemo(() => {
    const query = mcpFilter().trim().toLowerCase();
    const servers = [...mcpServers()].sort((a, b) => a.name.localeCompare(b.name));
    if (!query) return servers;
    return servers.filter((server) => {
      const haystack = [
        server.name,
        server.command,
        server.args.join(" "),
        server.runtime_state ?? "",
        server.runtime_error ?? "",
      ]
        .join(" ")
        .toLowerCase();
      return haystack.includes(query);
    });
  });
  const groupedMcpServers = createMemo(() => {
    const groups = new Map<string, typeof filteredMcpServers extends () => infer T ? T : never>();
    for (const server of filteredMcpServers()) {
      const groupBy = mcpGroupBy();
      const keys =
        groupBy === "state"
          ? [runtimeStateLabel(server.runtime_state)]
          : groupBy === "trust"
          ? [String(server.trust_level || "UNKNOWN").toUpperCase()]
          : (server.tags?.length ? server.tags : ["untagged"]);
      for (const key of keys) {
        const bucket = groups.get(key) ?? [];
        bucket.push(server);
        groups.set(key, bucket);
      }
    }
    return Array.from(groups.entries())
      .sort((a, b) => a[0].localeCompare(b[0]))
      .map(([name, servers]) => ({ name, servers }));
  });
  const mcpTotalPages = createMemo(() => {
    const maxGroupSize = groupedMcpServers().reduce((acc, group) => Math.max(acc, group.servers.length), 0);
    return Math.max(1, Math.ceil(maxGroupSize / MCP_PAGE_SIZE));
  });
  const pagedGroupedMcpServers = createMemo(() => {
    const page = mcpPage();
    const start = (page - 1) * MCP_PAGE_SIZE;
    const end = start + MCP_PAGE_SIZE;
    return groupedMcpServers().map((group) => ({
      name: group.name,
      total: group.servers.length,
      servers: group.servers.slice(start, end),
    }));
  });
  const mcpSummary = createMemo(() => {
    const servers = mcpServers();
    const total = servers.length;
    const running = servers.filter((s) => String(s.runtime_state).toLowerCase() === "running").length;
    const errored = servers.filter((s) => String(s.runtime_state).toLowerCase() === "error").length;
    return { total, running, errored };
  });

  createEffect(() => {
    if (activeTab() !== "services") return;
    let cancelled = false;
    const timer = setInterval(() => {
      if (cancelled) return;
      void loadMcpServers();
    }, 4000);
    onCleanup(() => {
      cancelled = true;
      clearInterval(timer);
    });
  });
  createEffect(() => {
    mcpFilter();
    mcpGroupBy();
    setMcpPage(1);
  });

  const hydrateIroncladConfig = async () => {
    try {
      const result = await getIroncladConfig();
      setIroncladHighRecoverySlo(String(result.config.high_recovery_slo_ms));
      setIroncladLeaseTtl(String(result.config.lease_ttl_ms));
      setIroncladHeartbeatGrace(String(result.config.heartbeat_grace_ms));
      setIroncladQuarantineCooldown(String(result.config.quarantine_cooldown_ms));
      setIroncladHashDistance(String(result.config.max_normalized_hash_distance));
    } catch (e) {
      setIroncladMessage(`Failed to load Ironclad config: ${e}`);
    }
  };

  const saveIroncladConfig = async () => {
    setIroncladBusy(true);
    setIroncladMessage("");
    try {
      const payload = {
        high_recovery_slo_ms: Number.parseInt(ironcladHighRecoverySlo(), 10),
        lease_ttl_ms: Number.parseInt(ironcladLeaseTtl(), 10),
        heartbeat_grace_ms: Number.parseInt(ironcladHeartbeatGrace(), 10),
        quarantine_cooldown_ms: Number.parseInt(ironcladQuarantineCooldown(), 10),
        max_normalized_hash_distance: Number.parseFloat(ironcladHashDistance()),
      };

      if (
        Number.isNaN(payload.high_recovery_slo_ms) ||
        Number.isNaN(payload.lease_ttl_ms) ||
        Number.isNaN(payload.heartbeat_grace_ms) ||
        Number.isNaN(payload.quarantine_cooldown_ms) ||
        Number.isNaN(payload.max_normalized_hash_distance)
      ) {
        setIroncladMessage("All Ironclad fields must be valid numbers.");
        return;
      }

      const result = await updateIroncladConfig(payload);
      if (result.updated) {
        await Promise.all([hydrateIroncladConfig(), loadIroncladStatus(), loadIroncladForensics(64)]);
        setIroncladMessage("Ironclad configuration applied.");
      } else {
        setIroncladMessage(result.reason || "No changes were applied.");
      }
    } catch (e) {
      setIroncladMessage(`Failed to apply Ironclad config: ${e}`);
    } finally {
      setIroncladBusy(false);
    }
  };

  const triggerIroncladSoftReset = async () => {
    setIroncladBusy(true);
    setIroncladMessage("");
    try {
      await requestIroncladSoftReset(ironcladResetReason().trim() || undefined);
      setIroncladMessage("Soft reset queued.");
      await loadIroncladStatus();
    } catch (e) {
      setIroncladMessage(`Soft reset failed: ${e}`);
    } finally {
      setIroncladBusy(false);
    }
  };

  const triggerIroncladHardReset = async () => {
    if (ironcladHardResetPhrase().trim() !== "HARD RESET") {
      setIroncladMessage("Hard reset blocked: type HARD RESET exactly.");
      return;
    }

    setIroncladBusy(true);
    setIroncladMessage("");
    try {
      await requestIroncladHardReset("HARD RESET", ironcladResetReason().trim() || undefined);
      setIroncladMessage("Hard reset queued.");
      setIroncladHardResetPhrase("");
      await loadIroncladStatus();
    } catch (e) {
      setIroncladMessage(`Hard reset failed: ${e}`);
    } finally {
      setIroncladBusy(false);
    }
  };

  const setAssistantPref = <K extends keyof AssistantFrontendPrefs>(key: K, value: AssistantFrontendPrefs[K]) => {
    setAssistantPrefs((prev) => ({ ...prev, [key]: value }));
  };

  const setLabsPref = <K extends keyof LabsFrontendPrefs>(key: K, value: LabsFrontendPrefs[K]) => {
    setLabsPrefs((prev) => ({ ...prev, [key]: value }));
  };

  const toggleCatalogItem = (id: string) => {
    setMcpCatalog((prev) =>
      prev.map((item) => (item.id === id ? { ...item, enabled: !item.enabled } : item))
    );
  };

  const tabGroups: {
    title: string;
    tabs: SettingsTabDefinition[];
  }[] = [
    {
      title: "General",
      tabs: [
        {
          id: "llm",
          label: "Models",
          icon: "M",
          description: "Choose the active local/cloud runtime, manage providers, and tune generation defaults.",
          layer: "basic",
        },
        {
          id: "voice",
          label: "Voice",
          icon: "V",
          description: "Configure microphone selection, VAD sensitivity, language, and TTS behavior.",
          layer: "basic",
        },
        {
          id: "safety",
          label: "Safety",
          icon: "S",
          description: "Tune approval thresholds, rollback windows, and tool execution safety limits.",
          layer: "basic",
        },
        {
          id: "search",
          label: "Search",
          icon: "Q",
          description: "Set the default search provider and endpoint for web retrieval.",
          layer: "basic",
        },
      ],
    },
    {
      title: "Personalization",
      tabs: [
        {
          id: "ui",
          label: "Appearance",
          icon: "A",
          description: "Customize visual theme, language, contrast, motion, and text scale.",
          layer: "basic",
        },
        {
          id: "assistant",
          label: "Assistant",
          icon: "H",
          description: "Select persona, response depth, and helper behavior preferences.",
          layer: "basic",
        },
        {
          id: "labs",
          label: "Labs",
          icon: "L",
          description: "Toggle preview interfaces and prototype modules for advanced workflows.",
          layer: "advanced",
        },
      ],
    },
    {
      title: "Connected Apps",
      tabs: [
        {
          id: "services",
          label: "MCP Services",
          icon: "P",
          description: "Manage MCP servers, runtime status, trust levels, and command registration.",
          layer: "integrations",
        },
        {
          id: "telegram",
          label: "Telegram",
          icon: "T",
          description: "Connect a Telegram bot for mobile chat and remote assistant access.",
          layer: "integrations",
        },
        {
          id: "n8n",
          label: "n8n",
          icon: "N",
          description: "Manage n8n runtime mode, connection settings, secrets, and dashboard access.",
          layer: "integrations",
        },
        {
          id: "google",
          label: "Google",
          icon: "G",
          description: "Manage Google auth, runtime health, capabilities, and synchronization warnings.",
          layer: "integrations",
        },
        {
          id: "colab",
          label: "Colab",
          icon: "C",
          description: "Manage Google Colab cloud-tier runtime, notebook selection, and tool capability readiness.",
          layer: "integrations",
        },
      ],
    },
    {
      title: "System & Data",
      tabs: [
        {
          id: "automation",
          label: "Automation",
          icon: "U",
          description: "Inspect health, schedule jobs, and manage stored macros and workflow assets.",
          layer: "workflow",
        },
        {
          id: "gui_automation",
          label: "GUI Automation",
          icon: "G",
          description: "Master switch for GUI automation, with live status of vision sidecar and uinput daemon.",
          layer: "workflow",
        },
        {
          id: "hardware",
          label: "Hardware",
          icon: "R",
          description: "Review detected hardware and recommended runtime tiers and performance values.",
          layer: "advanced",
        },
        {
          id: "ironclad",
          label: "Ironclad",
          icon: "I",
          description: "Fleet health telemetry, reset controls, forensic audit feed, and advanced runtime config.",
          layer: "developer",
        },
        {
          id: "developer",
          label: "Developer",
          icon: "D",
          description: "Developer mode: show diagnostic details, debug banners, and technical internals across the app.",
          layer: "developer",
        },
        {
          id: "knowledge",
          label: "Knowledge",
          icon: "K",
          description: "Review indexed documents and retrieval corpus status for knowledge grounding.",
          layer: "advanced",
        },
      ],
    },
    {
      title: "Marketplace",
      tabs: [
        {
          id: "marketplace",
          label: "Skill Marketplace",
          icon: "M",
          description: "Browse and manage skills.",
          layer: "integrations",
        },
      ],
    },
  ];

  const activeTabInfo = createMemo(() => {
    for (const group of tabGroups) {
      const tab = group.tabs.find((item) => item.id === activeTab());
      if (tab) {
        const layer = SETTINGS_LAYERS.find((item) => item.id === tab.layer) ?? SETTINGS_LAYERS[0];
        return { group: group.title, layer, tab };
      }
    }
    return { group: tabGroups[0].title, layer: SETTINGS_LAYERS[0], tab: tabGroups[0].tabs[0] };
  });

  const visibleTabGroups = createMemo(() =>
    tabGroups
      .map((group) => ({
        ...group,
        tabs: group.tabs.filter((tab) => tab.layer === activeLayer()),
      }))
      .filter((group) => group.tabs.length > 0)
  );

  const layerOptions = createMemo(() =>
    SETTINGS_LAYERS.map((layer) => ({
      ...layer,
      count: tabGroups.reduce(
        (count, group) => count + group.tabs.filter((tab) => tab.layer === layer.id).length,
        0
      ),
    }))
  );

  const firstTabForLayer = (layer: SettingsLayer): Tab =>
    tabGroups.flatMap((group) => group.tabs).find((tab) => tab.layer === layer)?.id ?? "llm";

  const selectLayer = (layer: SettingsLayer) => {
    setActiveLayer(layer);
    if (activeTabInfo().tab.layer !== layer) {
      setActiveTab(firstTabForLayer(layer));
    }
  };

  const selectTab = (tab: SettingsTabDefinition) => {
    setActiveLayer(tab.layer);
    setActiveTab(tab.id);
  };

  const handleClearAllChats = async () => {
    if (!clearChatsConfirm()) {
      setError("");
      setSuccess("Click Clear all chat sessions again to permanently delete all chat history.");
      setClearChatsConfirm(true);
      return;
    }

    setClearChatsBusy(true);
    setError("");
    setSuccess("");
    try {
      const result = await clearAllChatSessions();
      setSuccess(
        `Cleared ${result.deletedSessionCount} chat session${result.deletedSessionCount === 1 ? "" : "s"} and ${result.deletedTurnCount} stored turn${result.deletedTurnCount === 1 ? "" : "s"}.`
      );
      setClearChatsConfirm(false);
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed to clear chat sessions.");
    } finally {
      setClearChatsBusy(false);
    }
  };

  return (
    <div class="modal-overlay" onClick={closeSettings}>
      <div
        ref={dialogEl}
        class="modal settings-modal"
        role="dialog"
        aria-modal="true"
        aria-labelledby="settings-dialog-title"
        tabIndex={-1}
        onClick={(e) => e.stopPropagation()}
      >
        <div class="modal-header">
          <h2 id="settings-dialog-title">Settings</h2>
          <button class="close-btn" aria-label="Close settings" onClick={closeSettings}>×</button>
        </div>

        <div class="modal-body settings-shell">
          <aside class="settings-sidebar-nav">
            <div class="settings-sidebar-head">
              <h3>Settings Layers</h3>
              <p>Choose a layer, then a focused section.</p>
            </div>

            <div class="settings-layer-nav" aria-label="Settings navigation layers">
              <For each={layerOptions()}>
                {(layer) => (
                  <button
                    type="button"
                    class={`settings-layer-btn ${activeLayer() === layer.id ? "active" : ""}`}
                    aria-pressed={activeLayer() === layer.id}
                    onClick={() => selectLayer(layer.id)}
                  >
                    <span class="settings-layer-main">
                      <span class="settings-layer-label">{layer.label}</span>
                      <span class="settings-layer-count">{layer.count}</span>
                    </span>
                    <span class="settings-layer-summary">{layer.description}</span>
                  </button>
                )}
              </For>
            </div>

            <For each={visibleTabGroups()}>
              {(group) => (
                <div class="settings-nav-group">
                  <div class="settings-nav-group-title">{group.title}</div>
                  <For each={group.tabs}>
                    {(tab) => (
                      <button
                        class={`settings-nav-item ${activeTab() === tab.id ? "active" : ""}`}
                        type="button"
                        aria-current={activeTab() === tab.id ? "page" : undefined}
                        onClick={() => selectTab(tab)}
                        title={tab.description}
                      >
                        <span class="settings-nav-icon" aria-hidden="true">{tab.icon}</span>
                        <span class="settings-nav-label">{tab.label}</span>
                      </button>
                    )}
                  </For>
                </div>
              )}
            </For>
          </aside>

          <section class="settings-content">
            <div class="settings-content-header">
              <span class="settings-content-group">{activeTabInfo().layer.label} / {activeTabInfo().group}</span>
              <h3>{activeTabInfo().tab.label}</h3>
              <p>{activeTabInfo().tab.description}</p>
            </div>

            <div class="settings-content-scroll">
              <Show when={error()}>
                <div class="settings-error">{error()}</div>
              </Show>
              <Show when={success()}>
                <div class="settings-success">{success()}</div>
              </Show>

          {/* LLM Tab */}
          <Show when={activeTab() === "llm"}>
            <>
              <ProviderSettings />

              <section class="settings-section settings-advanced-details">
                <div class="settings-section-heading">
                  <div>
                    <h3>Generation Defaults</h3>
                    <p class="settings-hint">Runtime/provider changes above apply immediately. These defaults use the modal Save button.</p>
                  </div>
                </div>
              <div class="settings-row">
                <div class="settings-field">
                  <label>Temperature</label>
                  <input
                    type="range"
                    min="0"
                    max="2"
                    step="0.1"
                    value={draft()?.llm?.temperature ?? 0.6}
                    onInput={(e) => updateField("llm", "temperature", parseFloat(e.currentTarget.value))}
                  />
                  <span class="field-value">{(draft()?.llm?.temperature ?? 0.6).toFixed(1)}</span>
                </div>
                <div class="settings-field">
                  <label>Max Tokens</label>
                  <input
                    type="number"
                    value={draft()?.llm?.max_tokens ?? 2048}
                    onInput={(e) => updateField("llm", "max_tokens", parseInt(e.currentTarget.value) || 2048)}
                  />
                </div>
                <div class="settings-field">
                  <label>Context Window</label>
                  <input
                    type="number"
                    value={draft()?.llm?.context_window ?? 4096}
                    onInput={(e) => updateField("llm", "context_window", parseInt(e.currentTarget.value) || 4096)}
                  />
                </div>
              </div>
              </section>
            </>
          </Show>

          {/* Voice Tab */}
          <Show when={activeTab() === "voice"}>
            <section class="settings-section">
              <div class="settings-field">
                <label>
                  <input
                    type="checkbox"
                    checked={draft()?.voice?.enabled ?? false}
                    onChange={(e) => updateField("voice", "enabled", e.currentTarget.checked)}
                  />
                  Enable Voice
                </label>
              </div>
              <div class="settings-field">
                <label>Mode</label>
                <select
                  value={draft()?.voice?.mode ?? "push_to_talk"}
                  onChange={(e) => updateField("voice", "mode", e.currentTarget.value)}
                >
                  <option value="push_to_talk">Push to Talk</option>
                  <option value="continuous">Continuous</option>
                  <option value="wake_word">Wake Word</option>
                </select>
              </div>
              <div class="settings-field">
                <label>Microphone</label>
                <select
                  value={draft()?.voice?.mic_device ?? "auto"}
                  onChange={(e) => {
                    const selected = e.currentTarget.value;
                    const followDefault = selected === "auto";
                    updateField("voice", "mic_device", selected);
                    updateField("voice", "follow_system_default_mic", followDefault);
                  }}
                >
                  <option value="auto">
                    {audioDevices()?.default_input
                      ? `System Default (${audioDevices()?.default_input})`
                      : "System Default"}
                  </option>
                  <For each={audioDevices()?.inputs ?? []}>
                    {(device) => (
                      <Show when={device !== "auto"}>
                        <option value={device}>{device}</option>
                      </Show>
                    )}
                  </For>
                  <Show
                    when={
                      (draft()?.voice?.mic_device ?? "auto") !== "auto" &&
                      !(audioDevices()?.inputs ?? []).includes(draft()?.voice?.mic_device ?? "")
                    }
                  >
                    <option value={draft()?.voice?.mic_device}>
                      {(draft()?.voice?.mic_device ?? "Unknown device") + " (unavailable)"}
                    </option>
                  </Show>
                </select>
                <span class="field-hint">If not selected, KRIA uses the system default microphone.</span>
              </div>
              <div class="settings-field">
                <label>TTS Voice</label>
                <select
                  value={draft()?.voice?.tts_voice ?? "en_US-lessac-high"}
                  onChange={(e) => updateField("voice", "tts_voice", e.currentTarget.value)}
                >
                  <option value="en_US-lessac-high">Lessac (High)</option>
                  <option value="en_US-ryan-high">Ryan (High)</option>
                </select>
              </div>
              <div class="settings-field">
                <label>STT Language</label>
                <select
                  value={draft()?.voice?.language ?? "auto"}
                  onChange={(e) => updateField("voice", "language", e.currentTarget.value)}
                >
                  <option value="auto">Auto Detect</option>
                  <option value="en">English</option>
                  <option value="hi">Hindi</option>
                </select>
              </div>
              <div class="settings-field">
                <label>Noise Suppression</label>
                <select
                  value={draft()?.voice?.noise_suppression_mode ?? "off"}
                  onChange={(e) => updateField("voice", "noise_suppression_mode", e.currentTarget.value)}
                >
                  <option value="off">Off</option>
                  <option value="light">Light</option>
                  <option value="aggressive">Aggressive</option>
                </select>
              </div>
              <div class="settings-field">
                <label>VAD Silence (ms)</label>
                <input
                  type="number"
                  value={draft()?.voice?.vad_silence_ms ?? 1000}
                  onInput={(e) => updateField("voice", "vad_silence_ms", parseInt(e.currentTarget.value) || 1000)}
                />
              </div>
              <div class="settings-field">
                <label>Energy Threshold (normalized)</label>
                <input
                  type="number"
                  min="0.001"
                  max="1"
                  step="0.005"
                  value={draft()?.voice?.energy_threshold ?? 0.02}
                  onInput={(e) => updateField("voice", "energy_threshold", parseFloat(e.currentTarget.value) || 0.02)}
                />
                <span class="field-hint">Typical range is 0.01 to 0.08. Lower values make voice activation more sensitive.</span>
              </div>
              <div class="settings-field">
                <label>Partial Transcript Interval (ms)</label>
                <input
                  type="number"
                  min="200"
                  value={draft()?.voice?.partial_update_ms ?? 2000}
                  onInput={(e) => updateField("voice", "partial_update_ms", parseInt(e.currentTarget.value) || 2000)}
                />
              </div>
              <div class="settings-field">
                <label>Transcript Confidence Threshold</label>
                <input
                  type="range"
                  min="0"
                  max="1"
                  step="0.05"
                  value={draft()?.voice?.confidence_threshold ?? 0.3}
                  onInput={(e) => updateField("voice", "confidence_threshold", parseFloat(e.currentTarget.value))}
                />
                <span class="field-value">{(draft()?.voice?.confidence_threshold ?? 0.3).toFixed(2)}</span>
                <span class="field-hint">Final transcripts below this threshold are ignored.</span>
              </div>
            </section>
          </Show>

          {/* Safety Tab */}
          <Show when={activeTab() === "safety"}>
            <section class="settings-section">
              <div class="settings-field">
                <label>HITL Timeout (seconds)</label>
                <input
                  type="number"
                  value={draft()?.safety?.hitl_timeout_secs ?? 30}
                  onInput={(e) => updateField("safety", "hitl_timeout_secs", parseInt(e.currentTarget.value) || 30)}
                />
              </div>
              <div class="settings-field">
                <label>Rollback Retention (hours)</label>
                <input
                  type="number"
                  value={draft()?.safety?.rollback_retention_hours ?? 72}
                  onInput={(e) => updateField("safety", "rollback_retention_hours", parseInt(e.currentTarget.value) || 72)}
                />
              </div>
              <div class="settings-field">
                <label>Tool Timeout (seconds)</label>
                <input
                  type="number"
                  value={draft()?.safety?.tool_timeout_secs ?? 30}
                  onInput={(e) => updateField("safety", "tool_timeout_secs", parseInt(e.currentTarget.value) || 30)}
                />
              </div>
              <div class="settings-field">
                <label>Max Concurrent Tools</label>
                <input
                  type="number"
                  value={draft()?.safety?.max_concurrent_tools ?? 3}
                  onInput={(e) => updateField("safety", "max_concurrent_tools", parseInt(e.currentTarget.value) || 3)}
                />
              </div>
              <div class="settings-field">
                <label>
                  <input
                    type="checkbox"
                    checked={draft()?.safety?.emergency_mode ?? false}
                    onChange={(e) => updateField("safety", "emergency_mode", e.currentTarget.checked)}
                  />
                  Emergency Mode (disable all tools)
                </label>
              </div>
            </section>
          </Show>

          {/* Appearance Tab */}
          <Show when={activeTab() === "ui"}>
            <section class="settings-section">
              <div class="settings-field">
                <label>Theme</label>
                <div class="theme-toggle">
                  <button
                    class={`theme-btn ${theme() === "dark" ? "active" : ""}`}
                    onClick={() => {
                      applyTheme("dark");
                      updateField("ui", "theme", "dark");
                    }}
                  >
                    🌙 Dark
                  </button>
                  <button
                    class={`theme-btn ${theme() === "light" ? "active" : ""}`}
                    onClick={() => {
                      applyTheme("light");
                      updateField("ui", "theme", "light");
                    }}
                  >
                    ☀️ Light
                  </button>
                </div>
              </div>

              <div class="settings-field">
                <label>Language</label>
                <select
                  value={draft()?.ui?.language ?? "en"}
                  onChange={(e) => {
                    const lang = e.currentTarget.value;
                    updateField("ui", "language", lang);
                    setLocale(lang);
                  }}
                >
                  <For each={SUPPORTED_LANGUAGES}>
                    {(lang) => <option value={lang.code}>{lang.label}</option>}
                  </For>
                </select>
              </div>

              <div class="settings-field">
                <label>
                  <input
                    type="checkbox"
                    checked={draft()?.ui?.high_contrast ?? false}
                    onChange={(e) => {
                      const val = e.currentTarget.checked;
                      updateField("ui", "high_contrast", val);
                      document.documentElement.setAttribute("data-high-contrast", String(val));
                    }}
                  />
                  {" "}High Contrast
                </label>
                <span class="field-hint">Increase contrast for better visibility.</span>
              </div>

              <div class="settings-field">
                <label>
                  <input
                    type="checkbox"
                    checked={draft()?.ui?.reduce_motion ?? false}
                    onChange={(e) => {
                      const val = e.currentTarget.checked;
                      updateField("ui", "reduce_motion", val);
                      document.documentElement.setAttribute("data-reduce-motion", String(val));
                    }}
                  />
                  {" "}Reduce Motion
                </label>
                <span class="field-hint">Minimize animations for motion sensitivity.</span>
              </div>

              <div class="settings-field">
                <label>Font Scale</label>
                <select
                  value={selectedFontScale()}
                  onChange={(e) => {
                    const scale = e.currentTarget.value;
                    updateField("ui", "font_scale", parseFloat(scale));
                    document.documentElement.setAttribute("data-font-scale", scale);
                  }}
                >
                  <For each={FONT_SCALE_OPTIONS}>
                    {(option) => <option value={option.value}>{option.label}</option>}
                  </For>
                </select>
                <span class="field-hint">Current scale: {Math.round(parseFloat(selectedFontScale()) * 100)}%</span>
              </div>
            </section>
          </Show>

          {/* Search Tab */}
          <Show when={activeTab() === "assistant"}>
            <section class="settings-section">
              <h3>Assistant Persona</h3>
              <div class="settings-field">
                <label>Default Persona</label>
                <select
                  value={assistantPrefs().persona}
                  onChange={(e) => setAssistantPref("persona", e.currentTarget.value as AssistantFrontendPrefs["persona"])}
                >
                  <option value="chief_of_staff">Chief of Staff</option>
                  <option value="operator">Operator</option>
                  <option value="coach">Coach</option>
                  <option value="researcher">Researcher</option>
                </select>
                <span class="field-hint">Frontend-only preference. Affects assistant framing and UX labels.</span>
              </div>

              <div class="settings-field">
                <label>Response Detail</label>
                <select
                  value={assistantPrefs().verbosity}
                  onChange={(e) => setAssistantPref("verbosity", e.currentTarget.value as AssistantFrontendPrefs["verbosity"])}
                >
                  <option value="compact">Compact</option>
                  <option value="balanced">Balanced</option>
                  <option value="deep">Deep-dive</option>
                </select>
              </div>

              <h3>Interaction Style</h3>
              <div class="settings-field">
                <label>
                  <input
                    type="checkbox"
                    checked={assistantPrefs().proactiveSuggestions}
                    onChange={(e) => setAssistantPref("proactiveSuggestions", e.currentTarget.checked)}
                  />
                  {" "}Proactive suggestions panel
                </label>
              </div>
              <div class="settings-field">
                <label>
                  <input
                    type="checkbox"
                    checked={assistantPrefs().missionBriefings}
                    onChange={(e) => setAssistantPref("missionBriefings", e.currentTarget.checked)}
                  />
                  {" "}Mission briefings in new chats
                </label>
              </div>
              <div class="settings-field">
                <label>
                  <input
                    type="checkbox"
                    checked={assistantPrefs().followupQuestions}
                    onChange={(e) => setAssistantPref("followupQuestions", e.currentTarget.checked)}
                  />
                  {" "}Auto follow-up prompts
                </label>
              </div>
              <div class="settings-field">
                <label>
                  <input
                    type="checkbox"
                    checked={assistantPrefs().smartSessionTitles}
                    onChange={(e) => setAssistantPref("smartSessionTitles", e.currentTarget.checked)}
                  />
                  {" "}Smart session title suggestions
                </label>
              </div>

              <div class="tg-howto">
                <p>
                  Persona preview: <strong>{assistantPrefs().persona.replaceAll("_", " ")}</strong> ·
                  Detail level: <strong>{assistantPrefs().verbosity}</strong>
                </p>
              </div>
            </section>
          </Show>

          <Show when={activeTab() === "labs"}>
            <section class="settings-section">
              <h3>Scalable Frontend Modules</h3>
              <p class="field-hint">These controls are UI-only and designed for future backend wiring.</p>

              <div class="settings-field">
                <label>
                  <input
                    type="checkbox"
                    checked={labsPrefs().missionBoard}
                    onChange={(e) => setLabsPref("missionBoard", e.currentTarget.checked)}
                  />
                  {" "}Mission board workspace
                </label>
              </div>
              <div class="settings-field">
                <label>
                  <input
                    type="checkbox"
                    checked={labsPrefs().workflowCanvas}
                    onChange={(e) => setLabsPref("workflowCanvas", e.currentTarget.checked)}
                  />
                  {" "}Workflow canvas
                </label>
              </div>
              <div class="settings-field">
                <label>
                  <input
                    type="checkbox"
                    checked={labsPrefs().mcpMarketplace}
                    onChange={(e) => setLabsPref("mcpMarketplace", e.currentTarget.checked)}
                  />
                  {" "}MCP marketplace drawer
                </label>
              </div>
              <div class="settings-field">
                <label>
                  <input
                    type="checkbox"
                    checked={labsPrefs().autoPilotQueue}
                    onChange={(e) => setLabsPref("autoPilotQueue", e.currentTarget.checked)}
                  />
                  {" "}Autopilot queue monitor
                </label>
              </div>
              <div class="settings-field">
                <label>
                  <input
                    type="checkbox"
                    checked={labsPrefs().contextMap}
                    onChange={(e) => setLabsPref("contextMap", e.currentTarget.checked)}
                  />
                  {" "}Context map overlay
                </label>
              </div>

              <h3>MCP Skill Catalog (UI Prototype)</h3>
              <div class="mcp-server-list">
                <For each={mcpCatalog()}>
                  {(item) => (
                    <div class="mcp-server-card">
                      <div class="mcp-server-info">
                        <div class="mcp-server-name">
                          <span class={`mcp-status-dot ${item.enabled ? "running" : "stopped"}`}></span>
                          {item.name}
                        </div>
                        <div class="mcp-server-cmd">{item.description}</div>
                        <div class="mcp-server-trust">Trust profile: {item.trust}</div>
                      </div>
                      <div class="mcp-server-actions">
                        <button
                          class={`btn-small ${item.enabled ? "btn-warning" : "btn-success"}`}
                          onClick={() => toggleCatalogItem(item.id)}
                        >
                          {item.enabled ? "Disable" : "Enable"}
                        </button>
                      </div>
                    </div>
                  )}
                </For>
              </div>
            </section>
          </Show>

          {/* Search Tab */}
          <Show when={activeTab() === "search"}>
            <section class="settings-section">
              <div class="settings-field">
                <label>Search Engine</label>
                <select
                  value={draft()?.search?.engine ?? "duckduckgo"}
                  onChange={(e) => updateField("search", "engine", e.currentTarget.value)}
                >
                  <option value="duckduckgo">DuckDuckGo</option>
                  <option value="searxng">SearXNG</option>
                </select>
              </div>
              <Show when={draft()?.search?.engine === "searxng"}>
                <div class="settings-field">
                  <label>SearXNG URL</label>
                  <input
                    type="text"
                    value={draft()?.search?.searxng_url ?? ""}
                    onInput={(e) => updateField("search", "searxng_url", e.currentTarget.value)}
                    placeholder="http://localhost:8888"
                  />
                </div>
              </Show>
            </section>
          </Show>

          {/* Services (MCP) Tab */}
          <Show when={activeTab() === "services"}>
            <section class="settings-section">
              <h3>MCP Servers</h3>
              <p class="field-hint">
                Model Context Protocol servers provide external tools to the AI agent.
              </p>
              <div class="field-hint">
                {mcpSummary().running}/{mcpSummary().total} running
                {mcpSummary().errored > 0 ? ` • ${mcpSummary().errored} in error` : ""}
              </div>
              <div class="mcp-server-actions" style="margin-top:0.6rem">
                <button
                  class="btn-small"
                  onClick={async () => {
                    try {
                      await reconcileMcpRuntime();
                      await loadMcpServers();
                      setSuccess("MCP runtime reconciled");
                      setTimeout(() => setSuccess(""), 2000);
                    } catch (e) {
                      setError(`Failed to reconcile runtime: ${e}`);
                    }
                  }}
                >
                  Reconcile runtime
                </button>
              </div>
              <div class="settings-field">
                <label>Search MCP Servers</label>
                <input
                  type="text"
                  value={mcpFilter()}
                  onInput={(e) => setMcpFilter(e.currentTarget.value)}
                  placeholder="Filter by name, command, state, or error..."
                />
              </div>
              <div class="settings-field">
                <label>Group By</label>
                <select
                  value={mcpGroupBy()}
                  onChange={(e) => setMcpGroupBy(e.currentTarget.value as "state" | "trust" | "tag")}
                >
                  <option value="state">Runtime State</option>
                  <option value="trust">Trust Level</option>
                  <option value="tag">Tag</option>
                </select>
              </div>

              <div class="mcp-server-list">
                <For each={pagedGroupedMcpServers()} fallback={
                  <div class="mcp-empty">No MCP servers configured.</div>
                }>
                  {(group) => (
                    <>
                      <div class="mcp-server-trust" style="margin-top:8px">
                        Group: {group.name} ({group.total})
                      </div>
                      <For each={group.servers}>
                        {(server) => (
                          <div class="mcp-server-card">
                            <div class="mcp-server-info">
                              <div class="mcp-server-name">
                                <span class={`mcp-status-dot ${runtimeDotClass(server.runtime_state)}`}></span>
                                {server.name}
                              </div>
                              <div class="mcp-server-cmd">{server.command} {server.args.join(" ")}</div>
                              <div class="mcp-server-trust">Trust: {server.trust_level}</div>
                              <div class="mcp-server-trust">
                                Runtime: {runtimeStateLabel(server.runtime_state)}
                                {typeof server.runtime_tool_count === "number" ? ` (${server.runtime_tool_count} tools)` : ""}
                              </div>
                              <Show when={server.health}>
                                <div class="mcp-server-trust">Health: {server.health}</div>
                              </Show>
                              <Show when={server.tags && server.tags.length > 0}>
                                <div class="mcp-server-trust">Tags: {(server.tags ?? []).join(", ")}</div>
                              </Show>
                              <Show when={server.runtime_error}>
                                <div class="mcp-server-trust" style="color:#ef4444">Error: {server.runtime_error}</div>
                              </Show>
                              <Show when={server.remediation}>
                                <div class="mcp-server-trust" style="color:#f59e0b">Remediation: {server.remediation}</div>
                              </Show>
                              <Show when={server.last_failure}>
                                <div class="mcp-server-trust">
                                  Last failure: {formatUnixMs(server.last_failure?.timestamp_unix_ms)} • {server.last_failure?.state} • {server.last_failure?.reason}
                                </div>
                              </Show>
                              <Show when={(server.failure_history ?? []).length > 0}>
                                <div class="mcp-server-trust">
                                  Last failures: {(server.failure_history ?? [])
                                    .slice(-3)
                                    .reverse()
                                    .map((entry) => `${formatUnixMs(entry.timestamp_unix_ms)} • ${entry.state} • ${entry.reason}`)
                                    .join(" | ")}
                                </div>
                              </Show>
                            </div>
                            <div class="mcp-server-actions">
                              <button
                                class={`btn-small ${server.enabled ? "btn-warning" : "btn-success"}`}
                                onClick={async () => {
                                  try {
                                    await toggleMcpServer(server.name, !server.enabled);
                                  } catch (e) {
                                    setError(`${e}`);
                                  }
                                }}
                              >
                                {server.enabled ? "Disable" : "Enable"}
                              </button>
                              <button
                                class="btn-small"
                                onClick={async () => {
                                  try {
                                    await restartMcpServerRuntime(server.name);
                                    await loadMcpServers();
                                  } catch (e) {
                                    setError(`${e}`);
                                  }
                                }}
                              >
                                Restart
                              </button>
                              <button
                                class="btn-small btn-danger"
                                onClick={async () => {
                                  try {
                                    await removeMcpServer(server.name);
                                  } catch (e) {
                                    setError(`${e}`);
                                  }
                                }}
                              >
                                Remove
                              </button>
                            </div>
                          </div>
                        )}
                      </For>
                    </>
                  )}
                </For>
              </div>
              <div class="mcp-server-actions" style="margin-top:10px">
                <button
                  class="btn-small"
                  disabled={mcpPage() <= 1}
                  onClick={() => setMcpPage((p) => Math.max(1, p - 1))}
                >
                  Prev
                </button>
                <div class="field-hint">Page {mcpPage()} / {mcpTotalPages()}</div>
                <button
                  class="btn-small"
                  disabled={mcpPage() >= mcpTotalPages()}
                  onClick={() => setMcpPage((p) => Math.min(mcpTotalPages(), p + 1))}
                >
                  Next
                </button>
              </div>

              <h3>Add Server</h3>
              <div class="settings-field">
                <label>Name</label>
                <input
                  type="text"
                  value={newServerName()}
                  onInput={(e) => setNewServerName(e.currentTarget.value)}
                  placeholder="my-server"
                />
              </div>
              <div class="settings-field">
                <label>Command</label>
                <input
                  type="text"
                  value={newServerCommand()}
                  onInput={(e) => setNewServerCommand(e.currentTarget.value)}
                  placeholder="npx -y @modelcontextprotocol/server-filesystem"
                />
              </div>
              <div class="settings-field">
                <label>Arguments (space-separated)</label>
                <input
                  type="text"
                  value={newServerArgs()}
                  onInput={(e) => setNewServerArgs(e.currentTarget.value)}
                  placeholder="/home /tmp"
                />
              </div>
              <div class="settings-field">
                <label>Trust Level</label>
                <select
                  value={newServerTrust()}
                  onChange={(e) => setNewServerTrust(e.currentTarget.value)}
                >
                  <option value="GREEN">GREEN (auto-approve)</option>
                  <option value="YELLOW">YELLOW (ask first)</option>
                  <option value="RED">RED (strict approval)</option>
                </select>
              </div>
              <button
                class="btn-primary"
                disabled={!newServerName().trim() || !newServerCommand().trim()}
                onClick={async () => {
                  try {
                    const args = newServerArgs().trim() ? newServerArgs().trim().split(/\s+/) : [];
                    await addMcpServer(newServerName().trim(), newServerCommand().trim(), args, newServerTrust());
                    setNewServerName("");
                    setNewServerCommand("");
                    setNewServerArgs("");
                    setNewServerTrust("YELLOW");
                    setSuccess("MCP server added");
                    setTimeout(() => setSuccess(""), 2000);
                  } catch (e) {
                    setError(`Failed to add server: ${e}`);
                  }
                }}
              >
                Add Server
              </button>
            </section>
          </Show>

          {/* Telegram Tab */}
          <Show when={activeTab() === "telegram"}>
            <section class="settings-section">
              <h3>Telegram Bot Integration</h3>
              <p class="field-hint">
                Connect a Telegram bot to chat with your AI assistant from your phone.
                Create a bot via <a href="https://t.me/BotFather" target="_blank" rel="noopener">@BotFather</a> on Telegram to get a token.
              </p>

              <Show when={telegramConfig()?.enabled}>
                <div class="tg-status-banner tg-connected">
                  <span class="mcp-status-dot running"></span>
                  <span>Telegram integration is <strong>enabled</strong></span>
                  <Show when={telegramBotInfo()}>
                    <span> — @{telegramBotInfo()!.bot_username}</span>
                  </Show>
                </div>
              </Show>

              <div class="settings-field">
                <label>Bot Token</label>
                <input
                  type="password"
                  value={tgBotToken()}
                  onInput={(e) => setTgBotToken(e.currentTarget.value)}
                  placeholder="123456:ABC-DEF1234ghIkl-zyx57W2v1u123ew11"
                />
                <span class="field-hint">Get this from @BotFather on Telegram</span>
              </div>

              <div class="settings-field">
                <label>Allowed Chat IDs</label>
                <input
                  type="text"
                  value={tgChatIds()}
                  onInput={(e) => setTgChatIds(e.currentTarget.value)}
                  placeholder="123456789, 987654321"
                />
                <span class="field-hint">Comma-separated. Empty = allow all (less secure). Send /start to your bot and check logs for your chat ID.</span>
              </div>

              <div class="settings-field">
                <label>
                  <input
                    type="checkbox"
                    checked={tgAutoStart()}
                    onChange={(e) => setTgAutoStart(e.currentTarget.checked)}
                  />
                  {" "}Auto-start on launch
                </label>
                <span class="field-hint">Automatically register and connect the Telegram MCP server when KRIA starts.</span>
              </div>

              <div class="tg-actions">
                <button
                  class="btn-secondary"
                  disabled={!tgBotToken().trim() || tgTesting()}
                  onClick={async () => {
                    setTgTesting(true);
                    setTgTestResult(null);
                    try {
                      const info = await testTelegramConnection(tgBotToken().trim());
                      setTgTestResult(`Connected to @${info.bot_username} (${info.bot_name})`);
                    } catch (e) {
                      setTgTestResult(`Failed: ${e}`);
                    } finally {
                      setTgTesting(false);
                    }
                  }}
                >
                  {tgTesting() ? "Testing..." : "Test Connection"}
                </button>

                <button
                  class="btn-primary"
                  disabled={!tgBotToken().trim() || tgSaving()}
                  onClick={async () => {
                    setTgSaving(true);
                    setError("");
                    try {
                      await saveTelegramConfig({
                        enabled: true,
                        bot_token: tgBotToken().trim(),
                        allowed_chat_ids: tgChatIds().trim(),
                        auto_start: tgAutoStart(),
                      });
                      await startTelegramMcp();
                      setSuccess("Telegram connected! MCP server registered.");
                      setTimeout(() => setSuccess(""), 3000);
                    } catch (e) {
                      setError(`Failed: ${e}`);
                    } finally {
                      setTgSaving(false);
                    }
                  }}
                >
                  {tgSaving() ? "Saving..." : (telegramConfig()?.enabled ? "Update & Reconnect" : "Enable Telegram")}
                </button>

                <Show when={telegramConfig()?.enabled}>
                  <button
                    class="btn-danger"
                    onClick={async () => {
                      try {
                        await stopTelegramMcp();
                        setTgTestResult(null);
                        setSuccess("Telegram disconnected.");
                        setTimeout(() => setSuccess(""), 2000);
                      } catch (e) {
                        setError(`Failed: ${e}`);
                      }
                    }}
                  >
                    Disconnect
                  </button>
                </Show>
              </div>

              <Show when={tgTestResult()}>
                <div class={`tg-test-result ${tgTestResult()!.startsWith("Failed") ? "tg-error" : "tg-success"}`}>
                  {tgTestResult()}
                </div>
              </Show>

              <h3>How it works</h3>
              <div class="tg-howto">
                <ol>
                  <li>Open Telegram and search for <strong>@BotFather</strong></li>
                  <li>Send <code>/newbot</code> and follow the prompts to create a bot</li>
                  <li>Copy the bot token and paste it above</li>
                  <li>Click "Enable Telegram" — this registers a Telegram MCP server</li>
                  <li>Send a message to your bot from your phone — KRIA will respond!</li>
                </ol>
              </div>
            </section>
          </Show>

          {/* n8n Tab */}
          <Show when={activeTab() === "n8n"}>
            <N8nSettings />
          </Show>

          {/* Automation Tab */}
          <Show when={activeTab() === "automation"}>
            <section class="settings-section">
              {/* Health Status */}
              <h3>System Health</h3>
              <Show when={healthInfo()} fallback={<p class="field-hint">Loading health info...</p>}>
                <div class="health-summary">
                  <span class={`health-badge ${healthInfo()!.status}`}>
                    {healthInfo()!.status}
                  </span>
                  <span class="field-hint">Uptime: {formatUptime(healthInfo()!.uptime_secs)} · {healthInfo()!.tool_count} tools</span>
                </div>
                <div class="health-services">
                  <For each={healthInfo()!.services ?? []}>
                    {(svc: any) => (
                      <div class="health-service-row">
                        <span class={`mcp-status-dot ${svc.status === "healthy" ? "running" : "stopped"}`}></span>
                        <span class="health-svc-name">{svc.name}</span>
                        <span class="health-svc-status">{svc.status}</span>
                        <Show when={svc.message}>
                          <span class="field-hint">({svc.message})</span>
                        </Show>
                      </div>
                    )}
                  </For>
                </div>
              </Show>

              {/* Scheduled Tasks */}
              <h3>Scheduled Tasks</h3>
              <div class="mcp-server-list">
                <For each={scheduledTasks()} fallback={
                  <div class="mcp-empty">No scheduled tasks.</div>
                }>
                  {(task) => (
                    <div class="mcp-server-card">
                      <div class="mcp-server-info">
                        <div class="mcp-server-name">{task.name}</div>
                        <div class="mcp-server-cmd">{task.prompt}</div>
                        <div class="mcp-server-trust">Every {formatInterval(task.interval_secs)}</div>
                      </div>
                      <div class="mcp-server-actions">
                        <button
                          class="btn-small btn-danger"
                          onClick={async () => {
                            try { await removeScheduledTask(task.id); }
                            catch (e) { setError(`${e}`); }
                          }}
                        >
                          Remove
                        </button>
                      </div>
                    </div>
                  )}
                </For>
              </div>

              <h3>Add Task</h3>
              <div class="settings-field">
                <label>Name</label>
                <input
                  type="text"
                  value={newTaskName()}
                  onInput={(e) => setNewTaskName(e.currentTarget.value)}
                  placeholder="Daily summary"
                />
              </div>
              <div class="settings-field">
                <label>Interval (seconds)</label>
                <input
                  type="number"
                  value={newTaskInterval()}
                  onInput={(e) => setNewTaskInterval(e.currentTarget.value)}
                  min="60"
                />
                <span class="field-hint">{formatInterval(parseInt(newTaskInterval()) || 0)}</span>
              </div>
              <div class="settings-field">
                <label>Agent Prompt</label>
                <input
                  type="text"
                  value={newTaskPrompt()}
                  onInput={(e) => setNewTaskPrompt(e.currentTarget.value)}
                  placeholder="Check my email and summarize"
                />
              </div>
              <button
                class="btn-primary"
                disabled={!newTaskName().trim() || !newTaskPrompt().trim()}
                onClick={async () => {
                  try {
                    await addScheduledTask(
                      newTaskName().trim(),
                      parseInt(newTaskInterval()) || 3600,
                      newTaskPrompt().trim()
                    );
                    setNewTaskName("");
                    setNewTaskInterval("3600");
                    setNewTaskPrompt("");
                    setSuccess("Task added");
                    setTimeout(() => setSuccess(""), 2000);
                  } catch (e) {
                    setError(`Failed to add task: ${e}`);
                  }
                }}
              >
                Add Task
              </button>

              {/* Recorded Macros */}
              <h3>Recorded Macros</h3>
              <div class="mcp-server-list">
                <For each={macros()} fallback={
                  <div class="mcp-empty">No recorded macros. Use the agent to record actions.</div>
                }>
                  {(macro_) => (
                    <div class="mcp-server-card">
                      <div class="mcp-server-info">
                        <div class="mcp-server-name">{macro_.name}</div>
                        <div class="mcp-server-cmd">{macro_.description}</div>
                        <div class="mcp-server-trust">{macro_.step_count} steps</div>
                      </div>
                      <div class="mcp-server-actions">
                        <button
                          class="btn-small btn-danger"
                          onClick={async () => {
                            try { await deleteMacro(macro_.name); }
                            catch (e) { setError(`${e}`); }
                          }}
                        >
                          Delete
                        </button>
                      </div>
                    </div>
                  )}
                </For>
              </div>

              {/* Workflows */}
              <h3>Workflows</h3>
              <div class="mcp-server-list">
                <For each={workflows()} fallback={
                  <div class="mcp-empty">No workflows configured.</div>
                }>
                  {(wf) => (
                    <div class="mcp-server-card">
                      <div class="mcp-server-info">
                        <div class="mcp-server-name">{wf.name}</div>
                        <div class="mcp-server-cmd">{wf.description}</div>
                        <div class="mcp-server-trust">{wf.step_count} steps</div>
                      </div>
                      <div class="mcp-server-actions">
                        <button
                          class="btn-small btn-danger"
                          onClick={async () => {
                            try { await deleteWorkflow(wf.id); }
                            catch (e) { setError(`${e}`); }
                          }}
                        >
                          Delete
                        </button>
                      </div>
                    </div>
                  )}
                </For>
              </div>
            </section>
          </Show>

          {/* RFC 008: GUI Automation Tab — Master Toggle + Service Liveness */}
          <Show when={activeTab() === "gui_automation"}>
            <section class="settings-section">
              <h3>GUI Automation Master Switch</h3>
              <p class="field-hint">
                When enabled, KRIA can control your mouse, keyboard, and read your screen for
                GUI automation tasks. When disabled, both the vision sidecar and the uinput
                input daemon are killed and an internal safety halt is engaged.
              </p>

              <Show when={!guiAutomationStatus()?.orchestrator_available}>
                <div class="settings-field" style={{ "background": "#3a1f1f", "padding": "12px", "border-radius": "6px", "border": "1px solid #6b3030" }}>
                  <strong style={{ "color": "#ff8080" }}>⚠ Orchestrator Unavailable</strong>
                  <p class="field-hint" style={{ "margin-top": "8px" }}>
                    The service orchestrator failed to start. Common causes:
                  </p>
                  <ul style={{ "margin": "8px 0 0 16px", "color": "#cccccc", "font-size": "13px" }}>
                    <li>The <code>kria-uinput-daemon</code> binary is missing — run <code>cargo build --release -p kria-uinput-daemon</code></li>
                    <li>Passwordless sudo is not configured for <code>kria-uinput-daemon</code></li>
                    <li>The vision sidecar's <code>main.py</code> could not be located</li>
                  </ul>
                </div>
              </Show>

              <div class="settings-field" style={{ "display": "flex", "align-items": "center", "gap": "16px" }}>
                <label class="toggle-switch" style={{ "display": "inline-flex", "align-items": "center", "gap": "12px", "cursor": guiAutomationBusy() ? "wait" : "pointer" }}>
                  <input
                    type="checkbox"
                    checked={guiAutomationStatus()?.automation_enabled ?? false}
                    disabled={guiAutomationBusy() || !guiAutomationStatus()?.orchestrator_available}
                    onChange={(e) => {
                      void toggleGuiAutomation((e.currentTarget as HTMLInputElement).checked);
                    }}
                  />
                  <span style={{ "font-size": "16px", "font-weight": "600" }}>
                    {guiAutomationStatus()?.automation_enabled ? "Enabled" : "Disabled"}
                  </span>
                </label>
                <Show when={guiAutomationStatus()?.global_halt_engaged}>
                  <span style={{
                    "background": guiAutomationStatus()?.halt_kind === "startup_warming" ? "#4b3b16" : "#5a2020",
                    "color": guiAutomationStatus()?.halt_kind === "startup_warming" ? "#ffd86b" : "#ff9090",
                    "padding": "4px 10px",
                    "border-radius": "12px",
                    "font-size": "12px",
                    "font-weight": "600",
                  }}>
                    {guiAutomationStatus()?.halt_kind === "startup_warming" ? "STARTING GUI AUTOMATION" : "🛑 SAFETY HALT ENGAGED"}
                  </span>
                </Show>
              </div>

              <Show when={guiAutomationStatus()?.global_halt_engaged && guiAutomationStatus()?.halt_reason}>
                <div class="settings-field" style={{
                  "padding": "10px 14px",
                  "background": "#3a1f1f",
                  "border": "1px solid #6b3030",
                  "border-radius": "6px",
                  "color": "#ffd0d0",
                  "font-family": "monospace",
                  "font-size": "13px",
                }}>
                  <strong style={{ "color": "#ff8080" }}>Halt reason:</strong>{" "}
                  {guiAutomationStatus()?.halt_reason}
                  <Show when={(guiAutomationStatus()?.release_conditions.length ?? 0) > 0}>
                    <div style={{ "margin-top": "6px" }}>
                      {guiAutomationStatus()?.release_conditions.join(" ")}
                    </div>
                  </Show>
                </div>
              </Show>

              {/* Developer/test: bypass the readiness safety gate so GUI Cognition
                  runs live actions on the first prompt (no safety_only downgrade). */}
              <div class="settings-field" style={{ "margin-top": "14px", "padding": "12px", "background": "#241a2e", "border": "1px solid #4a356b", "border-radius": "6px" }}>
                <label class="toggle-switch" style={{ "display": "inline-flex", "align-items": "center", "gap": "12px", "cursor": guiReadinessBusy() ? "wait" : "pointer" }}>
                  <input
                    type="checkbox"
                    checked={guiReadinessBypass()}
                    disabled={guiReadinessBusy()}
                    onChange={(e) => {
                      void toggleGuiReadinessBypass((e.currentTarget as HTMLInputElement).checked);
                    }}
                  />
                  <span style={{ "font-size": "15px", "font-weight": "600" }}>
                    Force live execution (skip readiness gate)
                  </span>
                  <Show when={guiReadinessBypass()}>
                    <span style={{ "background": "#5a3a20", "color": "#ffd86b", "padding": "3px 9px", "border-radius": "12px", "font-size": "11px", "font-weight": "700" }}>
                      TEST MODE
                    </span>
                  </Show>
                </label>
                <p class="field-hint" style={{ "margin-top": "8px" }}>
                  When ON, GUI Cognition runs the action on the <strong>first</strong> prompt without
                  waiting for the readiness preconditions — this removes the
                  "Workflow paused safely: execution_mode is safety_only" downgrade. It also relaxes
                  the per-turn runaway guards (cancel/watchdog/abort), so use it for testing the
                  feature, not as a permanent setting. Takes effect on the next prompt; resets to OFF
                  on app restart.
                </p>
              </div>

              <Show when={guiAutomationError()}>
                <div class="settings-field" style={{ "color": "#ff8080", "padding": "8px 12px", "background": "#2a1010", "border-radius": "4px" }}>
                  Error: {guiAutomationError()}
                </div>
              </Show>

              <div class="settings-field" style={{
                "padding": "10px 14px",
                "background": "#101820",
                "border": "1px solid #2a4055",
                "border-radius": "6px",
                "font-size": "13px",
              }}>
                <div>
                  <strong>Action backend:</strong>{" "}
                  {guiAutomationStatus()?.can_execute_actions ? "ready" : "blocked"} ·{" "}
                  {guiAutomationStatus()?.selected_backend ?? "unknown"}
                </div>
                <div style={{ "margin-top": "4px", "color": "#b8c7d6" }}>
                  Session {guiAutomationStatus()?.session_type ?? "unknown"} · probe{" "}
                  {guiAutomationStatus()?.backend_probe_status ?? "unknown"}
                </div>
                <div style={{ "margin-top": "4px", "color": "#b8c7d6" }}>
                  uinput socket {guiAutomationStatus()?.uinput_socket_accessible ? "accessible" : "unavailable"} ·
                  ydotool actions {guiAutomationStatus()?.ydotool_usable_for_actions ? "usable" : "unusable"} ·
                  xdotool actions {guiAutomationStatus()?.xdotool_usable_for_actions ? "usable" : "unusable"}
                </div>
                <Show when={guiAutomationStatus()?.session_type === "wayland" && guiAutomationStatus()?.xdotool_available && !guiAutomationStatus()?.xdotool_usable_for_actions}>
                  <div style={{ "margin-top": "6px", "color": "#ffd86b" }}>
                    xdotool is detected but not usable for Wayland GUI actions.
                  </div>
                </Show>
                <Show when={guiAutomationStatus()?.backend_selection_reason}>
                  <div style={{ "margin-top": "6px", "color": "#b8c7d6" }}>
                    {guiAutomationStatus()?.backend_selection_reason}
                  </div>
                </Show>
              </div>

              <h3 style={{ "margin-top": "24px" }}>Service Status</h3>
              <div class="settings-field">
                <For each={[
                  { key: "vision_sidecar" as const, label: "Vision Sidecar (Python OmniParser)", pid: "vision_pid" as const },
                  { key: "uinput_daemon" as const, label: "UInput Daemon (Input Injection)", pid: "uinput_pid" as const },
                ]}>
                  {(svc) => {
                    const status = () => guiAutomationStatus()?.[svc.key] ?? "stopped";
                    const pid = () => guiAutomationStatus()?.[svc.pid];
                    const colorFor = (s: string) => {
                      switch (s) {
                        case "running": return "#4ade80";
                        case "starting": return "#fbbf24";
                        case "failed": return "#f87171";
                        case "stopped":
                        default: return "#94a3b8";
                      }
                    };
                    return (
                      <div style={{
                        "display": "flex",
                        "align-items": "center",
                        "justify-content": "space-between",
                        "padding": "10px 14px",
                        "margin-bottom": "8px",
                        "background": "#1f2937",
                        "border-radius": "6px",
                        "border": "1px solid #374151",
                      }}>
                        <div style={{ "display": "flex", "align-items": "center", "gap": "12px" }}>
                          <span style={{
                            "width": "10px",
                            "height": "10px",
                            "border-radius": "50%",
                            "background": colorFor(status()),
                            "display": "inline-block",
                            "box-shadow": `0 0 8px ${colorFor(status())}`,
                          }} />
                          <span style={{ "font-weight": "500" }}>{svc.label}</span>
                        </div>
                        <div style={{ "display": "flex", "align-items": "center", "gap": "12px", "color": "#9ca3af", "font-size": "13px" }}>
                          <span style={{ "color": colorFor(status()), "font-weight": "600", "text-transform": "uppercase" }}>
                            {status()}
                          </span>
                          <Show when={pid()}>
                            <span style={{ "font-family": "monospace" }}>PID {pid()}</span>
                          </Show>
                        </div>
                      </div>
                    );
                  }}
                </For>
              </div>

              <h3 style={{ "margin-top": "24px" }}>Safety Anchors (RFC 008)</h3>
              <ul style={{ "color": "#cccccc", "font-size": "13px", "line-height": "1.8", "margin": "0 0 0 16px" }}>
                <li><strong>Logical Anchor:</strong> Hard 100-action budget cap enforced in <code>execute_workflow</code></li>
                <li><strong>Physical Anchor:</strong> Target window lock — immediate halt on PID/class mismatch for type/click</li>
                <li><strong>Hardware Anchor:</strong> Dead-man's switch — daemon flushes buffered keys on disconnect</li>
                <li><strong>Intelligence Anchor:</strong> CompletionFlag prevents re-typing on self-induced perceptual diffs</li>
                <li><strong>Master Kill:</strong> Global safety halt blocks all tool calls when this toggle is off</li>
              </ul>
            </section>
          </Show>

          {/* Hardware Tab */}
          <Show when={activeTab() === "hardware"}>
            <section class="settings-section">
              <h3>Detected Hardware</h3>
              <Show when={hardwareInfo()} fallback={<p>Loading hardware information...</p>}>
                {(hw) => (
                  <>
                    <div class="hw-tier-banner" data-tier={hw().tier}>
                      <span class="hw-tier-label">{hw().tier.toUpperCase()}</span>
                      <span class="hw-tier-host">{hw().hostname} — {hw().os}</span>
                    </div>

                    <div class="hw-grid">
                      <div class="hw-stat">
                        <div class="hw-stat-label">CPU Cores</div>
                        <div class="hw-stat-value">{hw().cpu_cores}</div>
                      </div>
                      <div class="hw-stat">
                        <div class="hw-stat-label">Total RAM</div>
                        <div class="hw-stat-value">{(hw().total_ram_mb / 1024).toFixed(1)} GB</div>
                      </div>
                      <div class="hw-stat">
                        <div class="hw-stat-label">GPU</div>
                        <div class="hw-stat-value">{hw().gpu_name || "None detected"}</div>
                      </div>
                      <div class="hw-stat">
                        <div class="hw-stat-label">VRAM</div>
                        <div class="hw-stat-value">{hw().vram_mb ? `${(hw().vram_mb! / 1024).toFixed(1)} GB` : "N/A"}</div>
                      </div>
                      <div class="hw-stat">
                        <div class="hw-stat-label">Vision</div>
                        <div class="hw-stat-value">{hw().vision_capable ? "Enabled" : "Disabled"}</div>
                      </div>
                      <div class="hw-stat">
                        <div class="hw-stat-label">Context Window</div>
                        <div class="hw-stat-value">{hw().context_window} tokens</div>
                      </div>
                    </div>

                    <h3>Tier Recommendations</h3>
                    <div class="hw-grid">
                      <div class="hw-stat">
                        <div class="hw-stat-label">Recommended LLM</div>
                        <div class="hw-stat-value">{hw().recommended_model}</div>
                      </div>
                      <div class="hw-stat">
                        <div class="hw-stat-label">Recommended STT</div>
                        <div class="hw-stat-value">{hw().recommended_stt}</div>
                      </div>
                      <div class="hw-stat">
                        <div class="hw-stat-label">GPU Layers</div>
                        <div class="hw-stat-value">{hw().gpu_layers === 0 ? "CPU only" : `${hw().gpu_layers} (all)`}</div>
                      </div>
                      <div class="hw-stat">
                        <div class="hw-stat-label">Inference Threads</div>
                        <div class="hw-stat-value">{hw().threads}</div>
                      </div>
                    </div>

                    <h3>Override Tier</h3>
                    <div class="settings-field">
                      <label>Manual Tier (empty = auto-detect)</label>
                      <select
                        value={draft()?.hardware?.tier || ""}
                        onChange={(e) => updateField("hardware", "tier", e.currentTarget.value)}
                      >
                        <option value="">Auto-detect</option>
                        <option value="lite">Lite</option>
                        <option value="standard">Standard</option>
                        <option value="performance">Performance</option>
                        <option value="high">High</option>
                      </select>
                    </div>
                  </>
                )}
              </Show>
            </section>
          </Show>

          {/* Google Workspace Tab */}
          <Show when={activeTab() === "google"}>
            <section class="settings-section">
              <h3>Google Workspace</h3>
              <p class="field-hint">
                Connect your Google account to let KRIA read Gmail, Calendar, Drive, Docs, Sheets, and Slides.
                Uses OAuth 2.0 — KRIA never sees your password.
              </p>

              {/* Connection status banner and details */}
              <Show when={googleStatus()}>
                {(status) => (
                  <>
                    <div
                      class={`tg-status-banner ${status().connected ? "tg-connected" : ""}`}
                      style={status().connected
                        ? ""
                        : "background:var(--surface-2,#2a2a2a);border-left:3px solid var(--text-muted,#888)"}
                    >
                      <span
                        class={`mcp-status-dot ${status().connected ? "running" : runtimeDotClass(status().mcp?.state)}`}
                      ></span>
                      <span>{googleStatusMessage()}</span>
                    </div>

                    <div class="settings-field" style="margin-top:0.5rem">
                      <label>Runtime signals</label>
                      <div style="display:flex;flex-wrap:wrap;gap:0.45rem;margin-top:0.35rem">
                        <span class="mcp-server-trust">Auth: {status().auth_ready ? "ready" : "not ready"}</span>
                        <span class="mcp-server-trust">Runtime: {status().runtime_ready ? "ready" : "not ready"}</span>
                        <span class="mcp-server-trust">MCP state: {runtimeStateLabel(status().mcp?.state)}</span>
                        <span class="mcp-server-trust">Tools: {status().mcp?.tool_count ?? 0}</span>
                        <span class="mcp-server-trust">Bridge: {status().gw_client_wired ? "wired" : "not wired"}</span>
                      </div>
                    </div>

                    <div class="settings-field" style="margin-top:0.75rem">
                      <label>Capabilities</label>
                      <div style="display:flex;flex-wrap:wrap;gap:0.45rem;margin-top:0.35rem">
                        <For each={googleCapabilityEntries()}>
                          {(entry) => (
                            <span class="mcp-server-trust" style={entry[1] ? "" : "opacity:0.7"}>
                              {entry[0]}: {entry[1] ? "yes" : "no"}
                            </span>
                          )}
                        </For>
                      </div>
                      <span class="field-hint">
                        Meet support mode: <code>{status().meet_support_mode}</code> (calendar conference-link fallback)
                      </span>
                    </div>

                    <Show when={status().mcp?.error}>
                      <div class="settings-error" style="margin-top:0.6rem">
                        <strong>MCP runtime error:</strong> {status().mcp?.error}
                      </div>
                    </Show>

                    <Show when={(status().warnings?.length ?? 0) > 0}>
                      <div class="settings-error" style="margin-top:0.6rem">
                        <strong>Warnings</strong>
                        <ul style="margin:0.4rem 0 0 1.2rem;padding:0">
                          <For each={status().warnings}>
                            {(warning) => <li>{warning}</li>}
                          </For>
                        </ul>
                      </div>
                    </Show>
                  </>
                )}
              </Show>

              {/* Missing credentials warning */}
              <Show when={googleStatus() && !googleStatus()!.credentials_configured}>
                <div class="settings-error" style="margin-top:0.75rem">
                  <strong>credentials.json missing.</strong> Create{" "}
                  <code>~/.google-mcp/credentials.json</code> with your Google Cloud OAuth
                  client credentials (installed app type) before connecting.
                </div>
              </Show>

              {/* Account name */}
              <div class="settings-field" style="margin-top:1rem">
                <label>Account name</label>
                <input
                  type="text"
                  value={gwAccount()}
                  onInput={(e) => setGwAccount(e.currentTarget.value)}
                  onBlur={async () => {
                    const normalized = gwAccount().trim();
                    if (!normalized) return;
                    try {
                      await setGoogleAccount(normalized);
                      await loadGoogleStatus(normalized);
                    } catch (e) {
                      setGwMessage(`Failed to persist account: ${e}`);
                    }
                  }}
                  placeholder="personal"
                  disabled={gwConnecting()}
                  style="max-width:220px"
                />
                <span class="field-hint">
                  Name you'll use to identify this Google account (e.g. "personal", "work").
                  This is now persisted as KRIA's single active Google account.
                </span>
              </div>

              {/* Connecting spinner + message */}
              <Show when={gwConnecting()}>
                <div class="tg-status-banner" style="background:var(--surface-2,#2a2a2a);border-left:3px solid #4a9eff;margin-top:0.5rem">
                  <span class="mcp-status-dot" style="background:#4a9eff;animation:pulse 1s infinite"></span>
                  <span>Waiting for authorization in browser...</span>
                </div>
                <p class="field-hint" style="margin-top:0.4rem">
                  A browser tab has opened. Sign in with Google and click <strong>Allow</strong>.
                  This window will update automatically when done.
                </p>
              </Show>

              {/* Feedback message */}
              <Show when={gwMessage()}>
                <div class={`tg-test-result ${gwMessage().startsWith("Authorization failed") ? "tg-error" : "tg-success"}`} style="margin-top:0.5rem">
                  {gwMessage()}
                </div>
              </Show>

              {/* Action buttons */}
              <div class="tg-actions" style="margin-top:1rem">
                <Show when={!googleStatus()?.auth_ready}>
                  <button
                    class="btn-primary"
                    disabled={gwConnecting() || !googleStatus()?.credentials_configured}
                    onClick={async () => {
                      const normalized = gwAccount().trim() || "personal";
                      const existing = gwPollTimer();
                      if (existing) {
                        clearInterval(existing);
                        setGwPollTimer(null);
                      }

                      setGwConnecting(true);
                      setGwMessage("");
                      try {
                        await setGoogleAccount(normalized);
                        const result = await connectGoogle(normalized);
                        if (result?.message) {
                          setGwMessage(result.message);
                        }
                        let attempts = 0;
                        const maxAttempts = 20;

                        // Poll every 3s while OAuth browser flow is in progress.
                        const timer = setInterval(async () => {
                          attempts += 1;
                          const status = await loadGoogleStatus(normalized);

                          if (status?.connected) {
                            clearInterval(timer);
                            setGwPollTimer(null);
                            setGwConnecting(false);
                            setGwMessage("Connected! Google Workspace tools are now active.");
                            await loadMcpServers();
                            setTimeout(() => setGwMessage(""), 4000);
                            return;
                          }

                          if (status?.auth_ready && !status.runtime_ready && status.mcp?.state !== "starting") {
                            clearInterval(timer);
                            setGwPollTimer(null);
                            setGwConnecting(false);
                            setGwMessage(
                              status.warnings?.[0] || "Authorization succeeded, but runtime is not ready yet."
                            );
                            await loadMcpServers();
                            return;
                          }

                          if (attempts >= maxAttempts) {
                            clearInterval(timer);
                            setGwPollTimer(null);
                            setGwConnecting(false);
                            setGwMessage(
                              status?.warnings?.[0] || "Authorization still pending. Please finish OAuth in the browser."
                            );
                          }
                        }, 3000);

                        setGwPollTimer(timer);
                      } catch (e) {
                        setGwConnecting(false);
                        setGwMessage(`Failed to start OAuth: ${e}`);
                      }
                    }}
                  >
                    {gwConnecting() ? "Waiting for browser…" : "Connect with Google"}
                  </button>
                </Show>

                <Show when={googleStatus()}>
                  <button
                    class="btn-secondary"
                    onClick={async () => {
                      const normalized = gwAccount().trim() || "personal";
                      await setGoogleAccount(normalized);
                      const status = await loadGoogleStatus(normalized);
                      await loadMcpServers();

                      if (!status) {
                        setGwMessage("Unable to fetch Google status.");
                      } else if (status.connected) {
                        setGwMessage("Google auth and runtime are healthy.");
                      } else if (status.auth_ready && !status.runtime_ready) {
                        setGwMessage(`OAuth ready; runtime not ready (state=${status.mcp?.state ?? "unknown"}).`);
                      } else if (!status.auth_ready) {
                        setGwMessage("Google OAuth is not ready.");
                      } else {
                        setGwMessage("Google integration is not ready.");
                      }

                      setTimeout(() => setGwMessage(""), 2500);
                    }}
                  >
                    Refresh status
                  </button>
                </Show>

                <Show when={googleStatus()}>
                  <button
                    class="btn-secondary"
                    onClick={async () => {
                      try {
                        await reconcileMcpRuntime();
                        await loadMcpServers();
                        await loadGoogleStatus(gwAccount());
                        setGwMessage("MCP runtime reconciled.");
                      } catch (e) {
                        setGwMessage(`Failed to reconcile runtime: ${e}`);
                      }
                    }}
                  >
                    Reconcile runtime
                  </button>
                  <button
                    class="btn-secondary"
                    onClick={async () => {
                      try {
                        await restartMcpServerRuntime("gworkspace");
                        await loadMcpServers();
                        await loadGoogleStatus(gwAccount());
                        setGwMessage("gworkspace runtime restarted.");
                      } catch (e) {
                        setGwMessage(`Failed to restart runtime: ${e}`);
                      }
                    }}
                  >
                    Restart runtime
                  </button>
                </Show>

                <Show when={googleStatus()?.token_present}>
                  <button
                    class="btn-danger"
                    onClick={async () => {
                      try {
                        const normalized = gwAccount().trim() || "personal";
                        await setGoogleAccount(normalized);
                        await disconnectGoogle(normalized);
                        await loadMcpServers();
                        setGwMessage("Disconnected. OAuth token removed.");
                      } catch (e) {
                        setGwMessage(`Failed to disconnect: ${e}`);
                      }
                    }}
                  >
                    Disconnect
                  </button>
                </Show>
              </div>

              {/* How it works */}
              <h3 style="margin-top:1.5rem">How it works</h3>
              <div class="tg-howto">
                <ol>
                  <li>Go to <a href="https://console.cloud.google.com/" target="_blank" rel="noopener">Google Cloud Console</a> → APIs &amp; Services → Credentials</li>
                  <li>Create an <strong>OAuth 2.0 Client ID</strong> (Application type: <em>Desktop app</em>)</li>
                  <li>Download the JSON and save it as <code>~/.google-mcp/credentials.json</code></li>
                  <li>Enable these APIs: Gmail, Calendar, Drive, Docs, Sheets, Slides, Forms</li>
                  <li>Come back here and click <strong>Connect with Google</strong></li>
                  <li>Sign in and click <strong>Allow</strong> - KRIA can now access your Workspace</li>
                  <li>Meet requests use calendar conference-link mode (<code>calendar_conference_link</code>)</li>
                </ol>
              </div>
            </section>
          </Show>

          {/* Colab Tab */}
          <Show when={activeTab() === "colab"}>
            <section class="settings-section">
              <h3>Google Colab Cloud Tier</h3>
              <p class="field-hint">
                Use Colab MCP tools for prompt-to-code workflows like creating notebooks,
                writing code, executing cells, and collecting outputs.
              </p>

              <Show when={colabStatus()}>
                {(status) => (
                  <>
                    <div
                      class={`tg-status-banner ${status().ready_for_cloud_task ? "tg-connected" : ""}`}
                      style={status().ready_for_cloud_task
                        ? ""
                        : "background:var(--surface-2,#2a2a2a);border-left:3px solid var(--text-muted,#888)"}
                    >
                      <span
                        class={`mcp-status-dot ${status().ready_for_cloud_task ? "running" : runtimeDotClass(status().mcp?.state)}`}
                      ></span>
                      <span>{colabStatusMessage()}</span>
                    </div>

                    <div class="settings-field" style="margin-top:0.5rem">
                      <label>Runtime signals</label>
                      <div style="display:flex;flex-wrap:wrap;gap:0.45rem;margin-top:0.35rem">
                        <span class="mcp-server-trust">Enabled: {status().enabled ? "yes" : "no"}</span>
                        <span class="mcp-server-trust">Runtime: {runtimeStateLabel(status().mcp?.state)}</span>
                        <span class="mcp-server-trust">Ready: {status().ready_for_cloud_task ? "yes" : "no"}</span>
                        <span class="mcp-server-trust">Tools: {status().mcp?.tool_count ?? 0}</span>
                        <span class="mcp-server-trust">Selected notebook: {status().selected_notebook || "none"}</span>
                      </div>
                    </div>

                    <div class="settings-field" style="margin-top:0.75rem">
                      <label>Capabilities</label>
                      <div style="display:flex;flex-wrap:wrap;gap:0.45rem;margin-top:0.35rem">
                        <For each={colabCapabilityEntries()}>
                          {(entry) => (
                            <span class="mcp-server-trust" style={entry[1] ? "" : "opacity:0.7"}>
                              {entry[0]}: {entry[1] ? "yes" : "no"}
                            </span>
                          )}
                        </For>
                      </div>
                      <span class="field-hint">
                        Ready requirements: {status().capabilities?.ready_requirements?.satisfied ? "satisfied" : "missing requirements"}
                      </span>
                    </div>

                    <Show when={status().mcp?.error}>
                      <div class="settings-error" style="margin-top:0.6rem">
                        <strong>MCP runtime error:</strong> {status().mcp?.error}
                      </div>
                    </Show>

                    <Show when={(status().warnings?.length ?? 0) > 0}>
                      <div class="settings-error" style="margin-top:0.6rem">
                        <strong>Warnings</strong>
                        <ul style="margin:0.4rem 0 0 1.2rem;padding:0">
                          <For each={status().warnings}>
                            {(warning) => <li>{warning}</li>}
                          </For>
                        </ul>
                      </div>
                    </Show>
                  </>
                )}
              </Show>

              <div class="settings-field" style="margin-top:1rem">
                <label>Colab MCP server name</label>
                <input
                  type="text"
                  value={colabServerName()}
                  onInput={(e) => setColabServerName(e.currentTarget.value)}
                  placeholder="colab-mcp"
                  disabled={colabBusy()}
                  style="max-width:260px"
                />
                <span class="field-hint">
                  This should match an MCP server entry in config/mcp_servers.json.
                </span>
              </div>

              <div class="settings-field">
                <label>Active notebook identifier</label>
                <input
                  type="text"
                  value={colabNotebookId()}
                  onInput={(e) => setColabNotebookId(e.currentTarget.value)}
                  placeholder="mcp_test.ipynb"
                  disabled={colabBusy()}
                />
                <span class="field-hint">
                  Set this after connection so Colab execution prompts target the selected notebook.
                </span>
              </div>

              <Show when={colabMessage()}>
                <div class={`tg-test-result ${colabMessage().toLowerCase().startsWith("failed") ? "tg-error" : "tg-success"}`} style="margin-top:0.5rem">
                  {colabMessage()}
                </div>
              </Show>

              <div class="tg-actions" style="margin-top:1rem">
                <button
                  class="btn-primary"
                  disabled={colabBusy()}
                  onClick={async () => {
                    setColabBusy(true);
                    setColabMessage("");
                    try {
                      const status = await connectColab(colabServerName().trim() || undefined);
                      await loadMcpServers();
                      setColabMessage(status?.ready_for_cloud_task
                        ? "Colab connected and ready."
                        : "Colab connected. Select notebook if required.");
                    } catch (e) {
                      setColabMessage(`Failed to connect Colab: ${e}`);
                    } finally {
                      setColabBusy(false);
                    }
                  }}
                >
                  {colabBusy() ? "Connecting..." : "Connect Colab"}
                </button>

                <button
                  class="btn-secondary"
                  disabled={colabBusy()}
                  onClick={async () => {
                    await loadColabStatus();
                    await loadMcpServers();
                    setColabMessage("Colab status refreshed.");
                    setTimeout(() => setColabMessage(""), 2500);
                  }}
                >
                  Refresh status
                </button>

                <button
                  class="btn-secondary"
                  disabled={colabBusy()}
                  onClick={async () => {
                    try {
                      await reconcileMcpRuntime();
                      await loadMcpServers();
                      await loadColabStatus();
                      setColabMessage("MCP runtime reconciled.");
                    } catch (e) {
                      setColabMessage(`Failed to reconcile runtime: ${e}`);
                    }
                  }}
                >
                  Reconcile runtime
                </button>

                <button
                  class="btn-secondary"
                  disabled={colabBusy()}
                  onClick={async () => {
                    try {
                      await restartMcpServerRuntime(colabServerName().trim() || "colab-mcp");
                      await loadMcpServers();
                      await loadColabStatus();
                      setColabMessage("Colab runtime restarted.");
                    } catch (e) {
                      setColabMessage(`Failed to restart runtime: ${e}`);
                    }
                  }}
                >
                  Restart runtime
                </button>

                <button
                  class="btn-secondary"
                  disabled={colabBusy() || !colabNotebookId().trim()}
                  onClick={async () => {
                    try {
                      await setColabNotebook(colabNotebookId().trim());
                      await loadColabStatus();
                      setColabMessage("Active notebook updated.");
                    } catch (e) {
                      setColabMessage(`Failed to set notebook: ${e}`);
                    }
                  }}
                >
                  Set active notebook
                </button>

                <button
                  class="btn-secondary"
                  onClick={() => {
                    window.open("https://colab.research.google.com", "_blank", "noopener,noreferrer");
                  }}
                >
                  Open Colab
                </button>

                <Show when={colabStatus()?.enabled}>
                  <button
                    class="btn-danger"
                    disabled={colabBusy()}
                    onClick={async () => {
                      setColabBusy(true);
                      try {
                        await disconnectColab();
                        await loadMcpServers();
                        setColabMessage("Colab disconnected.");
                      } catch (e) {
                        setColabMessage(`Failed to disconnect Colab: ${e}`);
                      } finally {
                        setColabBusy(false);
                      }
                    }}
                  >
                    Disconnect
                  </button>
                </Show>
              </div>

              <h3 style="margin-top:1.5rem">Discovered Colab Tools</h3>
              <div class="mcp-server-list">
                <For each={colabDiscoveredTools()} fallback={<div class="mcp-empty">No Colab tools discovered yet.</div>}>
                  {(tool) => (
                    <div class="mcp-server-card">
                      <div class="mcp-server-info">
                        <div class="mcp-server-name">{tool.operation || tool.name}</div>
                        <div class="mcp-server-cmd">{tool.name}</div>
                        <div class="mcp-server-trust">Params: {tool.parameter_count}</div>
                      </div>
                    </div>
                  )}
                </For>
              </div>

              <h3 style="margin-top:1.5rem">Prompt Examples</h3>
              <div class="tg-howto">
                <ol>
                  <li>Create a Google Colab notebook named mcp_test.ipynb and set it as active.</li>
                  <li>Write merge sort in Python in the active notebook and run it with sample input.</li>
                  <li>Install numpy in the notebook and run a quick matrix multiplication demo.</li>
                  <li>Train a small classifier in the active notebook and show accuracy plus saved checkpoint path.</li>
                </ol>
              </div>
            </section>
          </Show>

          {/* Ironclad Tab */}
          <Show when={activeTab() === "ironclad"}>
            <section class="settings-section">
              <h3>Fleet &amp; QoS Signals</h3>
              <p class="field-hint">
                Operator-facing health for InventoryRegistry, reset lifecycle, and adaptive QoS.
              </p>

              <Show when={ironcladStatus()}>
                {(status) => (
                  <>
                    <div
                      class={`tg-status-banner ${status().fleet?.health_degraded ? "" : "tg-connected"}`}
                      style={status().fleet?.health_degraded
                        ? ""
                        : "background:var(--surface-2,#2a2a2a);border-left:3px solid var(--text-muted,#888)"}
                    >
                      <span
                        class={`mcp-status-dot ${status().fleet?.health_degraded ? "degraded" : "running"}`}
                      ></span>
                      <span>
                        Fleet ready {status().fleet?.ready_targets ?? 0}/{status().fleet?.total_targets ?? 0} •
                        QoS {status().qos?.traffic_light ?? "gray"} •
                        Reset {status().reset?.phase ?? "idle"}
                      </span>
                    </div>

                    <div class="settings-field" style="margin-top:0.8rem">
                      <label>Current runtime metrics</label>
                      <div style="display:flex;flex-wrap:wrap;gap:0.45rem;margin-top:0.35rem">
                        <span class="mcp-server-trust">Ready: {status().fleet?.ready_targets ?? 0}</span>
                        <span class="mcp-server-trust">Leased: {status().fleet?.leased_targets ?? 0}</span>
                        <span class="mcp-server-trust">Tainted: {status().fleet?.tainted_targets ?? 0}</span>
                        <span class="mcp-server-trust">Quarantined: {status().fleet?.quarantined_targets ?? 0}</span>
                        <span class="mcp-server-trust">p95: {status().qos?.high_recovery_wait_p95_ms ?? 0} ms</span>
                        <span class="mcp-server-trust">SLO: {status().qos?.high_recovery_slo_ms ?? 0} ms</span>
                        <span class="mcp-server-trust">Forensics: {ironcladForensicsTotal()}</span>
                      </div>
                      <span class="field-hint">
                        {status().reset?.detail || "No reset detail available"}
                      </span>
                    </div>
                  </>
                )}
              </Show>

              <Show when={ironcladMessage()}>
                <div class={`tg-test-result ${ironcladMessage().toLowerCase().startsWith("failed") || ironcladMessage().toLowerCase().includes("blocked") ? "tg-error" : "tg-success"}`} style="margin-top:0.75rem">
                  {ironcladMessage()}
                </div>
              </Show>

              <h3 style="margin-top:1.4rem">Advanced Configuration</h3>
              <div class="settings-row">
                <div class="settings-field">
                  <label>high_recovery_slo_ms</label>
                  <input type="number" min="50" value={ironcladHighRecoverySlo()} onInput={(e) => setIroncladHighRecoverySlo(e.currentTarget.value)} disabled={ironcladBusy()} />
                </div>
                <div class="settings-field">
                  <label>lease_ttl_ms</label>
                  <input type="number" min="500" value={ironcladLeaseTtl()} onInput={(e) => setIroncladLeaseTtl(e.currentTarget.value)} disabled={ironcladBusy()} />
                </div>
                <div class="settings-field">
                  <label>heartbeat_grace_ms</label>
                  <input type="number" min="100" value={ironcladHeartbeatGrace()} onInput={(e) => setIroncladHeartbeatGrace(e.currentTarget.value)} disabled={ironcladBusy()} />
                </div>
                <div class="settings-field">
                  <label>quarantine_cooldown_ms</label>
                  <input type="number" min="1000" value={ironcladQuarantineCooldown()} onInput={(e) => setIroncladQuarantineCooldown(e.currentTarget.value)} disabled={ironcladBusy()} />
                </div>
                <div class="settings-field">
                  <label>max_normalized_hash_distance</label>
                  <input type="number" min="0" max="1" step="0.01" value={ironcladHashDistance()} onInput={(e) => setIroncladHashDistance(e.currentTarget.value)} disabled={ironcladBusy()} />
                </div>
              </div>

              <div class="tg-actions">
                <button class="btn-primary" disabled={ironcladBusy()} onClick={saveIroncladConfig}>
                  {ironcladBusy() ? "Applying..." : "Apply config"}
                </button>
                <button class="btn-secondary" disabled={ironcladBusy()} onClick={() => { void hydrateIroncladConfig(); void loadIroncladStatus(); }}>
                  Reload
                </button>
                <button class="btn-secondary" disabled={ironcladBusy()} onClick={() => { void loadIroncladForensics(64); setIroncladMessage("Forensic feed refreshed."); }}>
                  Refresh forensics
                </button>
              </div>

              <h3 style="margin-top:1.4rem">Recovery Controls</h3>
              <div class="settings-row">
                <div class="settings-field">
                  <label>Reset reason</label>
                  <input
                    type="text"
                    value={ironcladResetReason()}
                    onInput={(e) => setIroncladResetReason(e.currentTarget.value)}
                    placeholder="manual_operator_recovery"
                    disabled={ironcladBusy()}
                  />
                </div>
                <div class="settings-field">
                  <label>Hard reset confirmation</label>
                  <input
                    type="text"
                    value={ironcladHardResetPhrase()}
                    onInput={(e) => setIroncladHardResetPhrase(e.currentTarget.value)}
                    placeholder="Type HARD RESET"
                    disabled={ironcladBusy()}
                  />
                </div>
              </div>
              <span class="field-hint">
                Trust-first guard: hard reset requires exact confirmation phrase each time.
              </span>

              <div class="tg-actions" style="margin-top:0.8rem">
                <button class="btn-secondary" disabled={ironcladBusy()} onClick={triggerIroncladSoftReset}>
                  Soft reset
                </button>
                <button class="btn-danger" disabled={ironcladBusy()} onClick={triggerIroncladHardReset}>
                  Hard reset
                </button>
              </div>
            </section>
          </Show>

          {/* Knowledge Base Tab */}
          <Show when={activeTab() === "knowledge"}>
            <section>
              <h3>Knowledge Base (RAG)</h3>
              <p class="settings-hint">Documents ingested for retrieval-augmented generation. Use the <code>ingest_document_rag</code> tool or ask the assistant to ingest a file.</p>
              <Show when={knowledgeBase().length > 0} fallback={<p class="settings-hint">No documents ingested yet.</p>}>
                <table class="kb-table">
                  <thead>
                    <tr>
                      <th>Name</th>
                      <th>Type</th>
                      <th>Chunks</th>
                      <th>Doc ID</th>
                    </tr>
                  </thead>
                  <tbody>
                    <For each={knowledgeBase()}>{(doc) => (
                      <tr>
                        <td>{doc.name}</td>
                        <td>{doc.type}</td>
                        <td>{doc.chunks}</td>
                        <td class="kb-doc-id">{doc.doc_id}</td>
                      </tr>
                    )}</For>
                  </tbody>
                </table>
              </Show>
              <p class="settings-hint">{knowledgeBase().length} document(s) in knowledge base</p>
              <div class="settings-section-heading" style={{ "margin-top": "1.2rem" }}>
                <div>
                  <h3>Chat Sessions</h3>
                  <p class="settings-hint">
                    Permanently delete saved assistant and prompt lab chat history. This does not remove ingested knowledge-base documents.
                  </p>
                </div>
              </div>
              <div class="settings-row">
                <div class="settings-field">
                  <label>Stored chat sessions</label>
                  <input type="text" value={`${sessions().length}`} disabled />
                  <span class="field-hint">A fresh empty chat is created after clearing so KRIA remains ready.</span>
                </div>
                <div class="settings-field">
                  <label>Clear history</label>
                  <button
                    type="button"
                    class={clearChatsConfirm() ? "btn-danger" : "btn-secondary"}
                    disabled={clearChatsBusy() || sessions().length === 0}
                    onClick={handleClearAllChats}
                  >
                    {clearChatsBusy()
                      ? "Clearing..."
                      : clearChatsConfirm()
                        ? "Confirm clear all chats"
                        : "Clear all chat sessions"}
                  </button>
                  <span class="field-hint">
                    This action deletes saved conversation turns and cannot be undone.
                  </span>
                </div>
              </div>
            </section>
          </Show>

          {/* Developer Tab */}
          <Show when={activeTab() === "developer"}>
            <section class="settings-section">
              <h3>Developer Mode</h3>
              <p class="settings-section-desc">
                KRIA is layman-friendly by default. Turn on Developer Mode to reveal diagnostic
                detail across the app — the GUI Cognition "Developer details" panel, debug/startup
                banners, hashes, probe timings, and other technical internals. Off by default;
                your choice is remembered.
              </p>
              <div class="settings-field" style={{ "margin-top": "14px", "padding": "12px", "background": "#241a2e", "border": "1px solid #4a356b", "border-radius": "6px" }}>
                <label class="toggle-switch" style={{ "display": "inline-flex", "align-items": "center", "gap": "12px", "cursor": "pointer" }}>
                  <input
                    type="checkbox"
                    checked={developerMode()}
                    onChange={(e) => setDeveloperMode((e.currentTarget as HTMLInputElement).checked)}
                  />
                  <span style={{ "font-size": "16px", "font-weight": "600" }}>
                    {developerMode() ? "Developer Mode: ON" : "Developer Mode: OFF"}
                  </span>
                  <Show when={developerMode()}>
                    <span style={{ "background": "#5a3a20", "color": "#ffd86b", "padding": "3px 9px", "border-radius": "12px", "font-size": "11px", "font-weight": "700" }}>
                      DEV
                    </span>
                  </Show>
                </label>
                <p class="field-hint" style={{ "margin-top": "8px" }}>
                  When OFF: clean, layman-friendly UI — no developer accordions, no dismissible
                  debug banners, no hashes/timings. When ON: full diagnostic detail for debugging
                  GUI Cognition and other subsystems.
                </p>
              </div>
            </section>
          </Show>

          {/* Marketplace Tab */}
          <Show when={activeTab() === "marketplace"}>
            <section>
              <h3>Marketplace</h3>
              <p class="settings-hint">Browse and manage skills.</p>
              <SkillMarketplace />
              <SubstrateStatus />
            </section>
          </Show>

            </div>
          </section>
        </div>

        <div class="modal-footer">
          <button class="btn-secondary" onClick={closeSettings}>Cancel</button>
          <button
            class="btn-primary"
            onClick={activeTab() === "n8n" ? closeSettings : handleSave}
            disabled={saving()}
          >
            {activeTab() === "n8n" ? "Done" : saving() ? "Saving..." : activeTab() === "llm" ? "Save Defaults" : "Save"}
          </button>
        </div>
      </div>
    </div>
  );
};

export default SettingsModal;
