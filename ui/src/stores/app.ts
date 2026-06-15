import { createMemo, createSignal } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import hljsDarkThemeUrl from "highlight.js/styles/github-dark.css?url";
import hljsLightThemeUrl from "highlight.js/styles/github.css?url";
import {
  handleGuiCognitionEvent,
  activeGuiCognitionSession,
  hasActiveGuiCognitionSession,
  markGuiCognitionCancelled,
} from "./guiCognitionSession";

const STORAGE_KEYS = {
  theme: "kria_theme",
  environment: "kria_environment",
  assistantSession: "kria_assistant_session_id",
  promptLabSession: "kria_prompt_lab_session_id",
  telegramBotInfo: "kria_telegram_bot_info",
  manualToolMode: "kria_manual_tool_mode",
} as const;

function readStorageValue(key: string): string | null {
  if (typeof window === "undefined") return null;
  const value = window.localStorage.getItem(key);
  return value && value.trim() ? value : null;
}

function writeStorageValue(key: string, value: string | null) {
  if (typeof window === "undefined") return;
  if (value && value.trim()) {
    window.localStorage.setItem(key, value);
  } else {
    window.localStorage.removeItem(key);
  }
}

const resolveInitialEnvironment = (): "assistant" | "prompt_lab" => {
  const saved = readStorageValue(STORAGE_KEYS.environment);
  return saved === "prompt_lab" ? "prompt_lab" : "assistant";
};

const MANUAL_TOOL_MODES = [
  { id: "auto", label: "Auto", appLock: null, toolLock: null, strategy: "routed_within_lock" },
  { id: "n8n", label: "n8n", appLock: null, toolLock: "n8n_invoke_workflow", strategy: "direct" },
  { id: "openclaw", label: "OpenClaw", appLock: "openclaw", toolLock: null, strategy: "routed_within_lock" },
  { id: "gui_cognition", label: "GUI Cognition", appLock: "gui_cognition", toolLock: null, strategy: "routed_within_lock" },
  { id: "image_generation", label: "Image Generation", appLock: null, toolLock: "generate_image", strategy: "direct" },
  { id: "gmail", label: "Gmail", appLock: "gmail", toolLock: null, strategy: "routed_within_lock" },
  { id: "calendar", label: "Calendar", appLock: "calendar", toolLock: null, strategy: "routed_within_lock" },
  { id: "github", label: "GitHub", appLock: "github", toolLock: null, strategy: "routed_within_lock" },
  { id: "filesystem", label: "Filesystem", appLock: "filesystem", toolLock: null, strategy: "routed_within_lock" },
  { id: "docker", label: "Docker", appLock: "docker", toolLock: null, strategy: "routed_within_lock" },
  { id: "browser", label: "Browser", appLock: "browser", toolLock: null, strategy: "routed_within_lock" },
  { id: "slack", label: "Slack", appLock: "slack", toolLock: null, strategy: "routed_within_lock" },
] as const;

export type ManualToolModeId = typeof MANUAL_TOOL_MODES[number]["id"];

export interface ManualToolModeOption {
  id: ManualToolModeId;
  label: string;
  appLock: string | null;
  toolLock: string | null;
  strategy: "direct" | "routed_within_lock";
}

export interface ManualToolProfile {
  mode_id: ManualToolModeId;
  label: string;
  app_lock: string | null;
  tool_lock: string | null;
  strategy: "direct" | "routed_within_lock";
}

const manualToolModes: ManualToolModeOption[] = MANUAL_TOOL_MODES.map((mode) => ({ ...mode }));

const normalizeManualToolMode = (value: unknown): ManualToolModeId => {
  const raw = typeof value === "string" ? value.trim() : "";
  return manualToolModes.some((mode) => mode.id === raw) ? (raw as ManualToolModeId) : "auto";
};

const resolveInitialManualToolMode = (): ManualToolModeId =>
  normalizeManualToolMode(readStorageValue(STORAGE_KEYS.manualToolMode));

// --- Signals ---
const [assistantMessages, setAssistantMessages] = createSignal<Message[]>([]);
const [promptLabMessages, setPromptLabMessages] = createSignal<Message[]>([]);
const [sessions, setSessions] = createSignal<Session[]>([]);
const [isSessionStartupLoading, setIsSessionStartupLoading] = createSignal(true);
const [assistantCurrentSession, setAssistantCurrentSession] = createSignal<string | null>(
  readStorageValue(STORAGE_KEYS.assistantSession)
);
const [promptLabCurrentSession, setPromptLabCurrentSession] = createSignal<string | null>(
  readStorageValue(STORAGE_KEYS.promptLabSession)
);
const [assistantIsThinking, setAssistantIsThinking] = createSignal(false);
const [promptLabIsThinking, setPromptLabIsThinking] = createSignal(false);
const [showSettings, setShowSettings] = createSignal(false);
const [assistantShowHitl, setAssistantShowHitl] = createSignal(false);
const [promptLabShowHitl, setPromptLabShowHitl] = createSignal(false);
const [assistantHitlRequest, setAssistantHitlRequest] = createSignal<HitlRequest | null>(null);
const [promptLabHitlRequest, setPromptLabHitlRequest] = createSignal<HitlRequest | null>(null);
const [assistantToolChoiceRequest, setAssistantToolChoiceRequest] = createSignal<ToolChoiceRequest | null>(null);
const [promptLabToolChoiceRequest, setPromptLabToolChoiceRequest] = createSignal<ToolChoiceRequest | null>(null);
const [voiceActive, setVoiceActive] = createSignal(false);
const [voiceState, setVoiceState] = createSignal<"idle" | "listening" | "processing" | "speaking" | "busy">("idle");
const [voiceLiveTranscript, setVoiceLiveTranscript] = createSignal("");
const [voiceLiveConfidence, setVoiceLiveConfidence] = createSignal<number | null>(null);
const [voiceLiveLanguage, setVoiceLiveLanguage] = createSignal("auto");
const [voiceLiveStability, setVoiceLiveStability] = createSignal<number | null>(null);
const [voiceInterruptionReason, setVoiceInterruptionReason] = createSignal<string | null>(null);
const [voicePlaybackHealth, setVoicePlaybackHealth] = createSignal<"ok" | "recovering" | "failed">("ok");
const [voiceIoMode, setVoiceIoMode] = createSignal<"half_duplex" | "headphone">("half_duplex");
const [voiceTtfaMs, setVoiceTtfaMs] = createSignal<number | null>(null);
let suppressVoiceErrorUntil = 0;
let liveVoiceDraftMessageId: string | null = null;
const [inputText, setInputText] = createSignal("");
const [manualToolMode, setManualToolModeSignal] = createSignal<ManualToolModeId>(
  resolveInitialManualToolMode()
);
const [settings, setSettings] = createSignal<Record<string, any> | null>(null);
const [models, setModels] = createSignal<any[]>([]);
const [audioDevices, setAudioDevices] = createSignal<AudioDevicesData | null>(null);
const resolveInitialTheme = (): "dark" | "light" => {
  if (typeof window === "undefined") return "light";
  const saved = readStorageValue(STORAGE_KEYS.theme);
  return saved === "dark" ? "dark" : "light";
};

const [theme, setTheme] = createSignal<"dark" | "light">(resolveInitialTheme());
const [mcpServers, setMcpServers] = createSignal<McpServer[]>([]);
const [healthInfo, setHealthInfo] = createSignal<Record<string, any> | null>(null);
const [runtimeStatus, setRuntimeStatus] = createSignal<RuntimeStatusPayload | null>(null);
const [runtimeDiagnostics, setRuntimeDiagnostics] = createSignal<RuntimeDiagnosticsPayload | null>(null);
const [scheduledTasks, setScheduledTasks] = createSignal<ScheduledTask[]>([]);
const [macros, setMacros] = createSignal<MacroInfo[]>([]);
const [workflows, setWorkflows] = createSignal<WorkflowInfo[]>([]);
const [hardwareInfo, setHardwareInfo] = createSignal<HardwareInfoData | null>(null);
const [knowledgeBase, setKnowledgeBase] = createSignal<KnowledgeDoc[]>([]);
const [alerts, setAlerts] = createSignal<ProactiveAlert[]>([]);
const [interactionDecisions, setInteractionDecisions] = createSignal<InteractionDecision[]>([]);
const [interactionDecisionMetrics, setInteractionDecisionMetrics] =
  createSignal<DecisionMetrics | null>(null);

const resolveInitialTelegramBotInfo = (): TelegramBotInfo | null => {
  const raw = readStorageValue(STORAGE_KEYS.telegramBotInfo);
  if (!raw) return null;
  try {
    const parsed = JSON.parse(raw) as TelegramBotInfo;
    if (
      typeof parsed === "object" &&
      parsed !== null &&
      typeof parsed.bot_username === "string" &&
      typeof parsed.bot_name === "string" &&
      typeof parsed.bot_id === "number"
    ) {
      return parsed;
    }
  } catch {
    // Ignore invalid cached telegram bot info.
  }
  return null;
};

function persistTelegramBotInfo(info: TelegramBotInfo | null) {
  if (typeof window === "undefined") return;
  if (info) {
    window.localStorage.setItem(STORAGE_KEYS.telegramBotInfo, JSON.stringify(info));
  } else {
    window.localStorage.removeItem(STORAGE_KEYS.telegramBotInfo);
  }
}

// Orchestrator swap state
const [isSwapping, setIsSwapping] = createSignal(false);
const [degradationLevel, setDegradationLevel] = createSignal<string | null>(null);
// Image generation progress (null = no active generation)
const [imageGenProgress, setImageGenProgress] = createSignal<number | null>(null);
const [imageGenStage, setImageGenStage] = createSignal<string | null>(null);
// VRAM blackout info during Tier B swap (null = no active swap)
const [vramBlackoutInfo, setVramBlackoutInfo] = createSignal<{ free_mb: number; required_mb: number; stage: string } | null>(null);
// True when session has been degraded to cloud-only due to repeated VRAM hangs
const [imageSessionDegraded, setImageSessionDegraded] = createSignal(false);
const [currentEnvironment, setCurrentEnvironmentSignal] = createSignal<"assistant" | "prompt_lab">(
  resolveInitialEnvironment()
);
const [lastPromptLabProfile, setLastPromptLabProfile] = createSignal<PromptLabProfile | undefined>(undefined);
const [latestAgentStage, setLatestAgentStage] = createSignal<AgentStageEvent | null>(null);
const [colabDispatchWarning, setColabDispatchWarning] = createSignal<string | null>(null);

// ─── Intelligence Enhancement Signals (Phase A-F Frontend) ──────────────────
import type {
  ExecutiveTask,
  ExecutiveSnapshot,
  ExecutiveTaskStarted,
  ExecutiveTaskCompleted,
  ExecutivePreemption,
  GpuLeaseEvent,
  PolicyGateEvaluation,
  QuarantinedTool,
  QuarantineApprovalRequest,
  QuarantinePromotionEvent,
  QuarantineDisabledEvent,
  PlanGenerated,
  PlanStepResult,
  GoalVerification,
  ToolStatsSnapshot,
  SelfModelSnapshot,
  IntelligenceState,
  RiskLevel,
  UncertaintyEvaluation,
} from "../types/intelligence";

// Executive Controller state
const [executiveSnapshot, setExecutiveSnapshot] = createSignal<ExecutiveSnapshot | null>(null);
const [executiveRecentEvents, setExecutiveRecentEvents] = createSignal<ExecutiveTaskCompleted[]>([]);

// Policy Gate log (ring buffer, most recent first)
const [policyGateLog, setPolicyGateLog] = createSignal<PolicyGateEvaluation[]>([]);

// Quarantine state
const [quarantinedTools, setQuarantinedTools] = createSignal<QuarantinedTool[]>([]);
const [quarantinePendingApproval, setQuarantinePendingApproval] = createSignal<QuarantineApprovalRequest[]>([]);

// Plan visualization
const [latestPlan, setLatestPlan] = createSignal<PlanGenerated | null>(null);
const [planStepResults, setPlanStepResults] = createSignal<PlanStepResult[]>([]);
const [latestGoalVerification, setLatestGoalVerification] = createSignal<GoalVerification | null>(null);

// Self Model
const [selfModelSnapshot, setSelfModelSnapshot] = createSignal<SelfModelSnapshot | null>(null);

// Intelligence summary
const [intelligenceState, setIntelligenceState] = createSignal<IntelligenceState>({
  uncertainty_confidence: 0,
  working_set_tokens: 0,
  self_model_tool_count: 0,
  compiled_skill_count: 0,
  quarantined_skill_count: 0,
  curiosity_findings: 0,
});

// Uncertainty engine
const [latestUncertainty, setLatestUncertainty] = createSignal<UncertaintyEvaluation | null>(null);

// ─── Throttled IPC Event Batching ───────────────────────────────────────────
//
// The backend fires events at high frequency. We batch them into a micro-queue
// and flush to SolidJS signals at most once per frame (via requestAnimationFrame)
// or every 50ms as a fallback. This prevents the UI from freezing under load.

type PendingEvent =
  | { kind: "executive:task_started"; payload: ExecutiveTaskStarted }
  | { kind: "executive:task_completed"; payload: ExecutiveTaskCompleted }
  | { kind: "executive:preemption"; payload: ExecutivePreemption }
  | { kind: "executive:gpu_lease"; payload: GpuLeaseEvent }
  | { kind: "policy_gate:evaluation"; payload: PolicyGateEvaluation }
  | { kind: "quarantine:pending_approval"; payload: QuarantineApprovalRequest }
  | { kind: "quarantine:promoted"; payload: QuarantinePromotionEvent }
  | { kind: "quarantine:disabled"; payload: QuarantineDisabledEvent }
  | { kind: "intelligence:plan"; payload: PlanGenerated }
  | { kind: "intelligence:step_result"; payload: PlanStepResult }
  | { kind: "intelligence:goal_verification"; payload: GoalVerification }
  | { kind: "intelligence:uncertainty"; payload: UncertaintyEvaluation }
  | { kind: "intelligence:self_model"; payload: SelfModelSnapshot };

const _pendingEvents: PendingEvent[] = [];
let _flushScheduled = false;

function enqueueEvent(event: PendingEvent) {
  _pendingEvents.push(event);
  if (!_flushScheduled) {
    _flushScheduled = true;
    if (typeof requestAnimationFrame !== "undefined") {
      requestAnimationFrame(flushPendingEvents);
    } else {
      setTimeout(flushPendingEvents, 50);
    }
  }
}

function flushPendingEvents() {
  _flushScheduled = false;
  if (_pendingEvents.length === 0) return;

  // Drain the queue atomically.
  const batch = _pendingEvents.splice(0);

  // Apply batch to signals.
  for (const event of batch) {
    applyEvent(event);
  }
}

function applyEvent(event: PendingEvent) {
  switch (event.kind) {
    case "executive:task_started": {
      // Update snapshot: add to active tasks.
      setExecutiveSnapshot((prev) => {
        if (!prev) return prev;
        const task: ExecutiveTask = {
          id: event.payload.task_id,
          priority: event.payload.priority,
          source: event.payload.source,
          state: "Running",
          description: event.payload.description,
          submitted_at: event.payload.ts,
          started_at: event.payload.ts,
          completed_at: null,
          duration_ms: null,
          error: null,
          requires_gpu: false,
        };
        return {
          ...prev,
          active_background: [...prev.active_background, task],
        };
      });
      break;
    }

    case "executive:task_completed": {
      // Add to recent events ring buffer (max 200).
      setExecutiveRecentEvents((prev) => {
        const next = [event.payload, ...prev];
        return next.length > 200 ? next.slice(0, 200) : next;
      });
      // Remove from active tasks.
      setExecutiveSnapshot((prev) => {
        if (!prev) return prev;
        return {
          ...prev,
          active_background: prev.active_background.filter(
            (t) => t.id !== event.payload.task_id
          ),
          total_completed: prev.total_completed + (event.payload.success ? 1 : 0),
          total_failed: prev.total_failed + (event.payload.success ? 0 : 1),
        };
      });
      break;
    }

    case "executive:preemption": {
      // Log as a recent event with preemption context.
      setExecutiveRecentEvents((prev) => {
        const synthetic: ExecutiveTaskCompleted = {
          task_id: event.payload.victim_id,
          success: false,
          duration_ms: 0,
          output_summary: `Preempted by ${event.payload.replacement_priority} task`,
          error: null,
          ts: event.payload.ts,
        };
        const next = [synthetic, ...prev];
        return next.length > 200 ? next.slice(0, 200) : next;
      });
      break;
    }

    case "executive:gpu_lease": {
      setExecutiveSnapshot((prev) => {
        if (!prev) return prev;
        if (event.payload.action === "acquired") {
          return { ...prev, gpu_lease_holder: event.payload.task_id };
        }
        if (event.payload.action === "released" || event.payload.action === "expired") {
          return { ...prev, gpu_lease_holder: null, gpu_lease_remaining_ms: null };
        }
        return prev;
      });
      break;
    }

    case "policy_gate:evaluation": {
      setPolicyGateLog((prev) => {
        const next = [event.payload, ...prev];
        return next.length > 500 ? next.slice(0, 500) : next;
      });
      break;
    }

    case "quarantine:pending_approval": {
      setQuarantinePendingApproval((prev) => {
        // Deduplicate by tool_id.
        if (prev.some((p) => p.tool_id === event.payload.tool_id)) return prev;
        return [...prev, event.payload];
      });
      break;
    }

    case "quarantine:promoted": {
      setQuarantinePendingApproval((prev) =>
        prev.filter((p) => p.tool_id !== event.payload.tool_id)
      );
      setQuarantinedTools((prev) =>
        prev.map((t) =>
          t.id === event.payload.tool_id ? { ...t, status: "Active" as const } : t
        )
      );
      break;
    }

    case "quarantine:disabled": {
      setQuarantinePendingApproval((prev) =>
        prev.filter((p) => p.tool_id !== event.payload.tool_id)
      );
      setQuarantinedTools((prev) =>
        prev.map((t) =>
          t.id === event.payload.tool_id ? { ...t, status: "Disabled" as const } : t
        )
      );
      break;
    }

    case "intelligence:plan": {
      setLatestPlan(event.payload);
      setPlanStepResults([]);
      break;
    }

    case "intelligence:step_result": {
      setPlanStepResults((prev) => [...prev, event.payload]);
      break;
    }

    case "intelligence:goal_verification": {
      setLatestGoalVerification(event.payload);
      break;
    }

    case "intelligence:uncertainty": {
      setLatestUncertainty(event.payload);
      setIntelligenceState((prev) => ({
        ...prev,
        uncertainty_confidence: event.payload.confidence,
      }));
      break;
    }

    case "intelligence:self_model": {
      setSelfModelSnapshot(event.payload);
      setIntelligenceState((prev) => ({
        ...prev,
        self_model_tool_count: event.payload.tools.length,
      }));
      break;
    }
  }
}

// ─── Backend API calls for Intelligence ─────────────────────────────────────

async function loadExecutiveSnapshot() {
  try {
    const snapshot = await invoke<ExecutiveSnapshot>("get_executive_snapshot");
    setExecutiveSnapshot(snapshot);
  } catch (e) {
    console.warn("Failed to load executive snapshot:", e);
  }
}

async function cancelExecutiveTask(taskId: string) {
  try {
    await invoke("cancel_executive_task", { taskId });
  } catch (e) {
    // Fallback: try the HTTP cancel endpoint (for web / non-Tauri mode)
    try {
      await fetch(`/api/executive/tasks/${encodeURIComponent(taskId)}/cancel`, {
        method: "POST",
      });
    } catch (httpErr) {
      console.error("Failed to cancel task via HTTP fallback:", httpErr);
    }
  }
}

/** Submit explicit routing feedback from UI buttons ("Wrong tool" / "Try differently").
 *  outcomeType: "wrong_tool" | "try_differently" | "wrong_domain:<DomainName>"
 */
async function submitTurnFeedback(
  sessionId: string,
  userText: string,
  toolSelected: string | null,
  outcomeType: string,
): Promise<boolean> {
  try {
    const result = await invoke<{ status: string; nudged: boolean }>("submit_turn_feedback", {
      sessionId,
      userText,
      toolSelected: toolSelected ?? null,
      outcomeType,
    });
    return result?.nudged ?? false;
  } catch (e) {
    // HTTP fallback for web mode
    try {
      const res = await fetch("/api/feedback/routing", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ sessionId, userText, toolSelected, outcomeType }),
      });
      const data = await res.json();
      return data?.nudged ?? false;
    } catch (httpErr) {
      console.warn("Failed to submit routing feedback:", httpErr);
      return false;
    }
  }
}

async function loadQuarantinedTools() {
  try {
    const tools = await invoke<QuarantinedTool[]>("list_quarantined_tools");
    setQuarantinedTools(tools);
    setQuarantinePendingApproval(tools.filter((t) => t.status === "PendingApproval").map((t) => ({
      tool_id: t.id,
      tool_name: t.name,
      risk_level: t.risk_level,
      source: t.source,
      success_count: t.success_count,
      description: t.description,
      ts: t.last_tested,
    })));
  } catch (e) {
    console.warn("Failed to load quarantined tools:", e);
  }
}

async function approveQuarantinedTool(toolId: string) {
  try {
    await invoke("approve_quarantined_tool", { toolId });
    // Optimistic update.
    setQuarantinePendingApproval((prev) => prev.filter((p) => p.tool_id !== toolId));
    setQuarantinedTools((prev) =>
      prev.map((t) => (t.id === toolId ? { ...t, status: "Active" as const } : t))
    );
  } catch (e) {
    console.error("Failed to approve tool:", e);
    throw e;
  }
}

async function rejectQuarantinedTool(toolId: string) {
  try {
    await invoke("reject_quarantined_tool", { toolId });
    setQuarantinePendingApproval((prev) => prev.filter((p) => p.tool_id !== toolId));
    setQuarantinedTools((prev) =>
      prev.map((t) => (t.id === toolId ? { ...t, status: "Rejected" as const } : t))
    );
  } catch (e) {
    console.error("Failed to reject tool:", e);
    throw e;
  }
}

async function loadSelfModel() {
  try {
    const snapshot = await invoke<SelfModelSnapshot>("get_self_model_snapshot");
    setSelfModelSnapshot(snapshot);
    setIntelligenceState((prev) => ({
      ...prev,
      self_model_tool_count: snapshot.tools.length,
    }));
  } catch (e) {
    console.warn("Failed to load self model:", e);
  }
}

async function loadPolicyGateLog() {
  try {
    const log = await invoke<PolicyGateEvaluation[]>("get_policy_gate_log");
    setPolicyGateLog(log.slice(0, 500));
  } catch (e) {
    console.warn("Failed to load policy gate log:", e);
  }
}

const currentSession = createMemo<string | null>(() =>
  currentEnvironment() === "prompt_lab" ? promptLabCurrentSession() : assistantCurrentSession()
);

const messages = createMemo<Message[]>(() =>
  currentEnvironment() === "prompt_lab" ? promptLabMessages() : assistantMessages()
);

const isThinking = createMemo<boolean>(() =>
  currentEnvironment() === "prompt_lab" ? promptLabIsThinking() : assistantIsThinking()
);

const showHitl = createMemo<boolean>(() =>
  currentEnvironment() === "prompt_lab" ? promptLabShowHitl() : assistantShowHitl()
);

const hitlRequest = createMemo<HitlRequest | null>(() =>
  currentEnvironment() === "prompt_lab" ? promptLabHitlRequest() : assistantHitlRequest()
);

const toolChoiceRequest = createMemo<ToolChoiceRequest | null>(() =>
  currentEnvironment() === "prompt_lab"
    ? promptLabToolChoiceRequest()
    : assistantToolChoiceRequest()
);

let healthLoadInFlight = false;
let healthLoadQueued = false;
let sessionHydrationRetryTimer: ReturnType<typeof setTimeout> | null = null;
let sessionHydrationRetryMs = 350;
let hasResolvedInitialSessionHydration = false;
let sessionHydrationAttempts = 0;
const SESSION_HYDRATION_RETRY_MAX_MS = 5000;
// Per-attempt timeout for the `list_sessions` invoke so a hung/deadlocked
// backend command can never leave the sidebar spinner stuck forever.
const LIST_SESSIONS_TIMEOUT_MS = 6000;
// Absolute hard deadline: no matter what happens (hang, repeated failure,
// missing Tauri runtime), the startup "Loading conversations..." spinner is
// force-settled to the empty/loaded state by this time after boot.
const SESSION_HYDRATION_HARD_DEADLINE_MS = 12000;
// After this many failed attempts the backend is treated as unavailable for the
// initial paint: stop showing the indefinite "Loading conversations..." spinner
// and settle to the empty state. Background retries continue so sessions appear
// if the backend recovers — but the UI never hangs on the loading state.
const SESSION_HYDRATION_MAX_STARTUP_ATTEMPTS = 8;

/**
 * Invoke a Tauri command but reject if it does not settle within `timeoutMs`.
 * Prevents a backend command that never returns (lock/deadlock) from hanging a
 * UI flow that awaits it. The underlying command may still complete server-side;
 * we just stop waiting on this call.
 */
function invokeWithTimeout<T>(
  command: string,
  args?: Record<string, unknown>,
  timeoutMs = 6000,
): Promise<T> {
  const call = invoke<T>(command, args);
  if (typeof window === "undefined") return call;
  return new Promise<T>((resolve, reject) => {
    const timer = window.setTimeout(() => {
      reject(new Error(`invoke('${command}') timed out after ${timeoutMs}ms`));
    }, timeoutMs);
    call.then(
      (value) => {
        clearTimeout(timer);
        resolve(value);
      },
      (err) => {
        clearTimeout(timer);
        reject(err);
      },
    );
  });
}

function scheduleSessionHydrationRetry() {
  if (typeof window === "undefined") {
    // No timer host (SSR/tests): never leave the UI stuck on "loading".
    markInitialSessionHydrationSettled();
    return;
  }
  if (sessionHydrationRetryTimer) return;

  sessionHydrationAttempts += 1;
  if (sessionHydrationAttempts >= SESSION_HYDRATION_MAX_STARTUP_ATTEMPTS) {
    // Give up on the *startup* spinner (show empty state) but keep retrying in
    // the background at the max interval so a late backend still populates.
    markInitialSessionHydrationSettled();
  }

  const delayMs = sessionHydrationRetryMs;
  sessionHydrationRetryMs = Math.min(
    Math.floor(sessionHydrationRetryMs * 1.8),
    SESSION_HYDRATION_RETRY_MAX_MS
  );

  sessionHydrationRetryTimer = window.setTimeout(() => {
    sessionHydrationRetryTimer = null;
    void initializeSessionPersistence();
  }, delayMs);
}

function resetSessionHydrationRetryState() {
  if (sessionHydrationRetryTimer) {
    clearTimeout(sessionHydrationRetryTimer);
    sessionHydrationRetryTimer = null;
  }
  sessionHydrationRetryMs = 350;
  sessionHydrationAttempts = 0;
}

function markInitialSessionHydrationSettled() {
  if (hasResolvedInitialSessionHydration) return;
  hasResolvedInitialSessionHydration = true;
  setIsSessionStartupLoading(false);
}

// Telegram integration
export interface TelegramConfig {
  enabled: boolean;
  bot_token: string;
  allowed_chat_ids: string;
  auto_start: boolean;
}

export interface TelegramBotInfo {
  valid: boolean;
  bot_name: string;
  bot_username: string;
  bot_id: number;
}

const [telegramConfig, setTelegramConfig] = createSignal<TelegramConfig | null>(null);
const [telegramBotInfo, setTelegramBotInfo] = createSignal<TelegramBotInfo | null>(resolveInitialTelegramBotInfo());

// --- Types ---
export interface Message {
  id: string;
  role: "user" | "assistant" | "system" | "tool";
  content: string;
  timestamp: number;
  toolCalls?: ToolCall[];
  /** Base64 data URL for image messages */
  imageUrl?: string;
  /** Attached document files (for document chat messages) */
  attachedFiles?: AttachedFileInfo[];
  /** Structured recovery options emitted when a prerequisite check fails */
  recoveryOptions?: RecoveryOptionsPayload;
  /** Live task step progress for multi-step operations */
  taskSteps?: TaskStepPayload[];
}

export interface AttachedFileInfo {
  name: string;
  size: number;
  mime: string;
}

export interface PendingFile {
  file: File;
  name: string;
  size: number;
  mime: string;
  preview?: string; // object URL for images
}

export interface ToolCall {
  name: string;
  args: Record<string, unknown>;
  result?: unknown;
  metadata?: ToolResultMetadata;
  status: "pending" | "running" | "done" | "error" | "denied";
  conversational_summary?: string;
  human_readable?: string;
  execution_metadata?: ExecutionMetadata;
}

export interface ExecutionMetadata {
  tool: string;
  outcome: "success" | "success_empty" | "partial_success" | "failure" | "cancelled";
  item_count?: number;
  duration_ms?: number;
  exit_code?: number;
  truncated?: boolean;
}

export interface RecoveryOption {
  label: string;
  action_prompt: string;
  style: "primary" | "secondary" | "danger";
}

export interface RecoveryOptionsPayload {
  context: string;
  detail: string;
  options: RecoveryOption[];
}

export interface TaskStepPayload {
  index: number;
  total?: number;
  description: string;
  status: "starting" | "running" | "done" | "failed" | "skipped";
}

export interface ToolResultMetadata {
  confidence?: number;
  sourceCount?: number;
  freshnessAgeHours?: number | null;
  regionMatch?: boolean | null;
}

export interface Session {
  id: string;
  title: string;
  updatedAt: number;
  turnCount?: number;
}

export interface HitlRequest {
  requestId: string;
  toolName: string;
  args: Record<string, unknown>;
  riskLevel: string;
  reason: string;
}

export interface DecisionOption {
  id: string;
  label: string;
  impact: string;
  risk: string;
}

export interface EvidenceSummary {
  source: string;
  confidence: string;
  freshness: string;
  reliability: string;
  summary: string;
}

export interface InteractionDecision {
  id: string;
  workflow_id: string;
  attempt_id?: string;
  stage_id?: string | null;
  action_hash?: string;
  target_hash?: string;
  action_proposal?: unknown | null;
  execution?: DecisionExecutionRecord | null;
  continuation?: ContinuationClaim | null;
  verification?: PostDecisionVerification | null;
  checkpoint_summary?: CheckpointSummary | null;
  decision_type: string;
  status: string;
  version: number;
  reason: string;
  risk_level: string;
  options: DecisionOption[];
  recommended_option?: string | null;
  rollbackability: string;
  confidence: string;
  affected_resources: string[];
  rule_id?: string | null;
  evidence: EvidenceSummary[];
  invalidation_rules: string[];
  created_at: string;
  updated_at: string;
  expires_at?: string | null;
  resolution?: string | null;
}

export interface DecisionExecutionRecord {
  execution_id: string;
  decision_id: string;
  workflow_id: string;
  action_hash: string;
  target_hash: string;
  state: string;
  sequence: number;
  execution_actor: string;
  source_command: string;
  session_id?: string | null;
  workspace_id?: string | null;
  tool_name: string;
  tool_schema_version: string;
  tool_registry_version: string;
  policy_version: string;
  started_at: string;
  side_effect_started_at?: string | null;
  completed_at?: string | null;
  gate_summary?: unknown;
  grounding_summary?: unknown;
  lease_refs: unknown[];
  redacted_tool_result?: unknown;
  error_class?: string | null;
  error_message?: string | null;
}

export interface ContinuationClaim {
  claim_id: string;
  decision_id: string;
  execution_id: string;
  workflow_id: string;
  checkpoint_id: string;
  action_hash: string;
  target_hash: string;
  state: string;
  sequence: number;
  actor: string;
  started_at: string;
  side_effect_started_at?: string | null;
  completed_at?: string | null;
  verification_id?: string | null;
  error_class?: string | null;
  error_message?: string | null;
}

export interface PostDecisionVerification {
  verification_id: string;
  decision_id: string;
  execution_id: string;
  workflow_id: string;
  action_hash: string;
  target_hash: string;
  verifier_kind: string;
  evidence: EvidenceSummary[];
  confidence: string;
  deterministic: boolean;
  passed: boolean;
  failure_reason?: string | null;
  sensitivity_tags: string[];
  created_at: string;
  expires_at?: string | null;
}

export interface CheckpointSummary {
  completed_action_ids: string[];
  blocked_action_id: string;
  expected_artifacts: string[];
  active_assumptions: string[];
  next_safe_action_preview?: unknown;
  verifier_requirements: string[];
  rollbackability: string;
  invalidation_rules: string[];
}

export interface DecisionMetrics {
  total_events: number;
  pending_decisions: number;
  resolved_decisions: number;
  expired_decisions: number;
  invalidated_decisions: number;
  approval_decisions: number;
  target_selection_decisions: number;
  unsafe_abstentions: number;
}

export interface ToolChoiceCandidate {
  name: string;
  label: string;
  reason: string;
  confidence: number;
}

export interface ToolChoiceRequest {
  query: string;
  confidence: number;
  minConfidence: number;
  candidates: ToolChoiceCandidate[];
}

export interface DiagnosticEvent {
  timestamp: string;
  level: string;
  target: string;
  message?: string | null;
  fields?: Record<string, unknown>;
  file?: string | null;
  line?: number | null;
}

export interface DiagnosticsSummary {
  capacity: number;
  captured_events: number;
  by_level: Record<string, number>;
  last_event_at?: string | null;
}

export interface RuntimeDiagnosticsPayload {
  summary: DiagnosticsSummary;
  events?: DiagnosticEvent[];
  recent?: DiagnosticEvent[];
}

export interface RuntimeStatusPayload {
  emitted_at: string;
  health: Record<string, unknown>;
  diagnostics: {
    summary: DiagnosticsSummary;
    recent: DiagnosticEvent[];
  };
}

export interface PromptLabProfile {
  appLock?: string | null;
  toolLock?: string | null;
  strategy?: "direct" | "routed_within_lock";
}

export interface McpServer {
  name: string;
  command: string;
  args: string[];
  enabled: boolean;
  trust_level: string;
  tags?: string[];
  runtime_state?: string;
  runtime_tool_count?: number;
  runtime_error?: string | null;
  failure_history?: McpFailureRecord[];
  last_failure?: McpFailureRecord | null;
  health?: string;
  remediation?: string | null;
}

export interface McpFailureRecord {
  timestamp_unix_ms: number;
  state: string;
  reason: string;
}

export interface ScheduledTask {
  id: string;
  name: string;
  interval_secs: number;
  prompt: string;
  enabled: boolean;
}

export interface MacroInfo {
  name: string;
  description: string;
  step_count: number;
  created_at: string;
}

export interface WorkflowInfo {
  id: string;
  name: string;
  description: string;
  step_count: number;
  created_at: string;
}

export interface HardwareInfoData {
  tier: string;
  cpu_cores: number;
  total_ram_mb: number;
  vram_mb: number | null;
  gpu_name: string | null;
  os: string;
  hostname: string;
  package_manager: string | null;
  vision_capable: boolean;
  recommended_model: string;
  recommended_stt: string;
  context_window: number;
  gpu_layers: number;
  threads: number;
}

export interface AudioDevicesData {
  inputs: string[];
  outputs: string[];
  default_input: string | null;
  default_output: string | null;
}

export interface KnowledgeDoc {
  doc_id: string;
  name: string;
  type: string;
  chunks: number;
}

export interface ProactiveAlert {
  id: string;
  category: "alert" | "suggestion" | "info";
  title: string;
  message: string;
  suggestion: string | null;
  timestamp: string;
}

export interface AssistantStatus {
  state: "ready" | "warming" | "degraded" | "offline";
  label: string;
  detail: string;
}

export interface AgentStageEvent {
  step: string;
  message: string;
  detail?: Record<string, unknown> | null;
  ts?: string;
}

type StreamScope = "assistant" | "prompt_lab";

let backendActiveSessionId: string | null = null;

function scopeFromEnvironment(): StreamScope {
  return currentEnvironment() === "prompt_lab" ? "prompt_lab" : "assistant";
}

function getScopedCurrentSession(scope: StreamScope): string | null {
  return scope === "prompt_lab" ? promptLabCurrentSession() : assistantCurrentSession();
}

function setScopedCurrentSession(scope: StreamScope, sessionId: string | null) {
  if (scope === "prompt_lab") {
    setPromptLabCurrentSession(sessionId);
    writeStorageValue(STORAGE_KEYS.promptLabSession, sessionId);
  } else {
    setAssistantCurrentSession(sessionId);
    writeStorageValue(STORAGE_KEYS.assistantSession, sessionId);
  }
}

async function ensureScopedSessionActive(scope: StreamScope): Promise<string> {
  let sessionId = getScopedCurrentSession(scope);
  if (!sessionId) {
    const created = await invoke<{ session_id: string }>("create_session");
    sessionId = created.session_id;
    setScopedCurrentSession(scope, sessionId);
    backendActiveSessionId = sessionId;
    await loadSessions();
  }

  if (backendActiveSessionId !== sessionId) {
    await invoke("switch_session", { sessionId });
    backendActiveSessionId = sessionId;
  }
  return sessionId;
}

async function syncEnvironmentSession(environment: "assistant" | "prompt_lab") {
  const scope: StreamScope = environment === "prompt_lab" ? "prompt_lab" : "assistant";
  const sessionId = getScopedCurrentSession(scope);
  if (!sessionId) return;

  try {
    const hasMessages = scope === "prompt_lab" ? promptLabMessages().length > 0 : assistantMessages().length > 0;
    await invoke("switch_session", { sessionId });
    backendActiveSessionId = sessionId;
    if (!hasMessages) {
      const mapped = await loadMappedSessionHistory(sessionId);
      updateScopedMessages(scope, () => mapped);
    }
  } catch (e) {
    console.error("Failed to sync environment session:", e);
  }
}

function setCurrentEnvironment(environment: "assistant" | "prompt_lab") {
  if (currentEnvironment() === environment) return;
  setCurrentEnvironmentSignal(environment);
  writeStorageValue(STORAGE_KEYS.environment, environment);
  void syncEnvironmentSession(environment);
}

function appendScopedMessage(scope: StreamScope, msg: Message) {
  if (scope === "prompt_lab") {
    setPromptLabMessages((prev) => [...prev, msg]);
  } else {
    setAssistantMessages((prev) => [...prev, msg]);
  }
}

function updateScopedMessages(scope: StreamScope, updater: (prev: Message[]) => Message[]) {
  if (scope === "prompt_lab") {
    setPromptLabMessages(updater);
  } else {
    setAssistantMessages(updater);
  }
}

function setScopedThinking(scope: StreamScope, value: boolean) {
  if (scope === "prompt_lab") {
    setPromptLabIsThinking(value);
  } else {
    setAssistantIsThinking(value);
    // Defense-in-depth watchdog: the chat input is disabled while the assistant
    // is "thinking". If a backend turn hangs and never emits `agent:done` (e.g.
    // a server-side stall the per-task done-guard can't catch because the task
    // is still alive), the input would freeze forever after the first prompt.
    // Arm a generous watchdog on thinking=true that auto-clears the state if NO
    // progress event arrives for a long window; any streamed event re-arms it
    // (see pokeAssistantThinkingWatchdog), so a genuinely-progressing long turn
    // never trips it.
    if (value) {
      armAssistantThinkingWatchdog();
    } else {
      clearAssistantThinkingWatchdog();
    }
  }
}

let assistantThinkingWatchdog: ReturnType<typeof setTimeout> | null = null;
// Max idle time (ms) with NO assistant/GUI-cognition event before the thinking
// state is force-cleared so the input can never hard-freeze. Generous because a
// non-streaming GUI Cognition turn can legitimately run for a couple of minutes
// on a busy/low-VRAM box before emitting its single terminal event.
const ASSISTANT_THINKING_WATCHDOG_MS = 300_000;

function clearAssistantThinkingWatchdog() {
  if (assistantThinkingWatchdog) {
    clearTimeout(assistantThinkingWatchdog);
    assistantThinkingWatchdog = null;
  }
}

function armAssistantThinkingWatchdog() {
  if (typeof window === "undefined") return;
  clearAssistantThinkingWatchdog();
  assistantThinkingWatchdog = window.setTimeout(() => {
    assistantThinkingWatchdog = null;
    if (!assistantIsThinking()) return;
    setAssistantIsThinking(false);
    appendScopedMessage("assistant", {
      id: crypto.randomUUID(),
      role: "system",
      content:
        "The previous turn stopped responding, so I cleared the busy state. You can send your message again.",
      timestamp: Date.now(),
    });
  }, ASSISTANT_THINKING_WATCHDOG_MS);
}

/** Re-arm the thinking watchdog on any sign of turn progress (streamed event). */
function pokeAssistantThinkingWatchdog() {
  if (assistantThinkingWatchdog && assistantIsThinking()) {
    armAssistantThinkingWatchdog();
  }
}

function isScopedThinking(scope: StreamScope): boolean {
  return scope === "prompt_lab" ? promptLabIsThinking() : assistantIsThinking();
}

/**
 * Task 10.2 (Requirement 16.3): user-visible message shown when a new prompt is
 * submitted while a turn is still active. We never silently drop or interleave:
 * the prompt is NOT dispatched and NOT recorded as a user turn, but the user is
 * told why and that they can wait or press Stop.
 */
const TURN_BUSY_MESSAGE =
  "A request is already running. Wait for it to finish or press Stop before sending another prompt.";

/**
 * Append an explicit "busy" notice for the given scope. Consecutive attempts are
 * de-duplicated so repeated submissions do not spam the transcript. Returns the
 * scope's current messages so callers can stay terse.
 */
function notifyTurnBusy(scope: StreamScope): void {
  const existing = scope === "prompt_lab" ? promptLabMessages() : assistantMessages();
  const last = existing[existing.length - 1];
  if (last && last.role === "system" && last.content === TURN_BUSY_MESSAGE) {
    return;
  }
  appendScopedMessage(scope, {
    id: crypto.randomUUID(),
    role: "system",
    content: TURN_BUSY_MESSAGE,
    timestamp: Date.now(),
  });
}

async function cancelScopedTurnIfActive(scope: StreamScope): Promise<void> {
  const sessionId = getScopedCurrentSession(scope);
  if (!sessionId || !isScopedThinking(scope)) return;
  try {
    await invoke("cancel_turn", { sessionId });
  } catch (e) {
    console.warn("Failed to cancel active turn before session change:", e);
  } finally {
    setScopedThinking(scope, false);
  }
}

/** Public API: cancel the active turn for the given scope (assistant or prompt_lab). */
async function cancelTurn(scope: StreamScope = "assistant"): Promise<void> {
  const sessionId = getScopedCurrentSession(scope);
  if (!sessionId || !isScopedThinking(scope)) return;
  // Task 10.3 (Requirement 16.6 / 21.1): if a GUI Cognition turn is active for
  // the assistant scope, also abort it through the Task 1 cancel path
  // (`cancel_gui_cognition_turn` → process-local CancelToken registry) so the
  // workflow loop halts before its next action — not just the chat/agent loop.
  if (scope === "assistant" && hasActiveGuiCognitionSession()) {
    await requestGuiCognitionCancel(sessionId);
  }
  try {
    await invoke("cancel_turn", { sessionId });
    setScopedThinking(scope, false);
  } catch (e) {
    // Fallback: try the HTTP cancel endpoint (for web / non-Tauri mode)
    try {
      const res = await fetch(`/api/sessions/${encodeURIComponent(sessionId)}/cancel`, {
        method: "POST",
      });
      if (res.ok) {
        setScopedThinking(scope, false);
      }
    } catch (httpErr) {
      console.warn("Failed to cancel active turn via HTTP fallback:", httpErr);
    }
  }
}

/**
 * Task 10.3 (Requirement 16.6 / 21.1): abort the active GUI Cognition turn via
 * the Task 1 cancel mechanism. Cancellation is cooperative — the backend
 * `cancel_gui_cognition_turn` command trips the per-turn `CancelToken` keyed by
 * `session_id`, so the workflow loop stops before its next action (bounded,
 * deterministic stop — never an uncontrolled kill). The UI is updated
 * optimistically (panel → "cancelled", thinking indicator cleared) regardless
 * of the IPC result so the surface always returns to idle/ready.
 */
async function requestGuiCognitionCancel(sessionId: string): Promise<void> {
  const reason = "Turn cancelled by you.";
  try {
    await invoke("cancel_gui_cognition_turn", { sessionId, reason });
  } catch (e) {
    console.warn("Failed to cancel GUI cognition turn:", e);
  }
  markGuiCognitionCancelled(reason);
}

/**
 * Public API: visible Stop/Cancel control for the GUI Cognition panel. Aborts
 * the active GUI Cognition turn and clears the assistant thinking indicator.
 */
async function cancelGuiCognitionTurn(): Promise<void> {
  const sessionId = activeGuiCognitionSession()?.sessionId ?? getScopedCurrentSession("assistant");
  // Flip the panel into a clear cancelled state immediately even if we cannot
  // resolve a session id (degrades gracefully when streaming is OFF / no turn).
  markGuiCognitionCancelled("Turn cancelled by you.");
  setScopedThinking("assistant", false);
  if (!sessionId) return;
  try {
    await invoke("cancel_gui_cognition_turn", { sessionId, reason: "Turn cancelled by you." });
  } catch (e) {
    console.warn("Failed to cancel GUI cognition turn:", e);
  }
}

function setScopedHitl(scope: StreamScope, request: HitlRequest | null, visible: boolean) {
  if (scope === "prompt_lab") {
    setPromptLabHitlRequest(request);
    setPromptLabShowHitl(visible);
  } else {
    setAssistantHitlRequest(request);
    setAssistantShowHitl(visible);
  }
}

function setScopedToolChoice(scope: StreamScope, req: ToolChoiceRequest | null) {
  if (scope === "prompt_lab") {
    setPromptLabToolChoiceRequest(req);
  } else {
    setAssistantToolChoiceRequest(req);
  }
}

function selectedManualToolMode(): ManualToolModeOption {
  return manualToolModes.find((mode) => mode.id === manualToolMode()) ?? manualToolModes[0];
}

function buildManualToolProfile(modeId: ManualToolModeId): ManualToolProfile | null {
  const mode = manualToolModes.find((candidate) => candidate.id === modeId);
  if (!mode || mode.id === "auto") return null;

  return {
    mode_id: mode.id,
    label: mode.label,
    app_lock: mode.appLock,
    tool_lock: mode.toolLock,
    strategy: mode.strategy,
  };
}

function setManualToolMode(mode: ManualToolModeId) {
  const normalized = normalizeManualToolMode(mode);
  setManualToolModeSignal(normalized);
  writeStorageValue(STORAGE_KEYS.manualToolMode, normalized === "auto" ? null : normalized);
}

function formatColabDispatchWarning(stage: AgentStageEvent): string {
  const detail = stage.detail && typeof stage.detail === "object" ? stage.detail : null;
  const requestedMode = typeof detail?.requested_mode === "string" ? detail.requested_mode : "colab";
  const effectiveMode = typeof detail?.effective_mode === "string" ? detail.effective_mode : requestedMode;
  const reason = typeof detail?.reason === "string" ? detail.reason : stage.message;
  const runtimeState = typeof detail?.runtime_state === "string" ? detail.runtime_state : null;

  return runtimeState
    ? `Colab routing fallback (${requestedMode} -> ${effectiveMode}): ${reason} [state=${runtimeState}]`
    : `Colab routing fallback (${requestedMode} -> ${effectiveMode}): ${reason}`;
}

// --- Actions ---
async function sendMessage(text: string) {
  if (!text.trim()) return;
  if (isScopedThinking("assistant")) {
    notifyTurnBusy("assistant");
    return;
  }

  setScopedToolChoice("assistant", null);
  const selectedMode = selectedManualToolMode();
  const manualProfile = buildManualToolProfile(selectedMode.id);

  try {
    const sessionId = await ensureScopedSessionActive("assistant");

    const userMsg: Message = {
      id: crypto.randomUUID(),
      role: "user",
      content: text,
      timestamp: Date.now(),
    };
    appendScopedMessage("assistant", userMsg);
    setInputText("");
    setScopedThinking("assistant", true);

    void autoRenameSessionFromPrompt(sessionId, text);
    if (!manualProfile) {
      await invoke<{ status: string }>(
        "send_message",
        { message: text }
      );
    } else {
      await invoke<{ status: string }>(
        "send_manual_tool_message",
        {
          message: text,
          profile: manualProfile,
        }
      );
    }
    // Response arrives asynchronously via agent:token / agent:done events
  } catch (e) {
    const errMsg: Message = {
      id: crypto.randomUUID(),
      role: "system",
      content: `Error: ${e}`,
      timestamp: Date.now(),
    };
    appendScopedMessage("assistant", errMsg);
    setScopedThinking("assistant", false);
  }
}

async function sendLabMessage(text: string, profile?: PromptLabProfile) {
  if (!text.trim()) return;
  if (isScopedThinking("prompt_lab")) {
    notifyTurnBusy("prompt_lab");
    return;
  }

  setScopedToolChoice("prompt_lab", null);

  const payload = {
    message: text,
    profile: {
      app_lock: profile?.appLock ?? null,
      tool_lock: profile?.toolLock ?? null,
      strategy: profile?.strategy ?? "routed_within_lock",
    },
  };

  try {
    const sessionId = await ensureScopedSessionActive("prompt_lab");

    const userMsg: Message = {
      id: crypto.randomUUID(),
      role: "user",
      content: text,
      timestamp: Date.now(),
    };
    appendScopedMessage("prompt_lab", userMsg);
    setInputText("");
    setScopedThinking("prompt_lab", true);
    setLastPromptLabProfile(profile);

    void autoRenameSessionFromPrompt(sessionId, text);
    await invoke<{ status: string }>("send_lab_message", payload);
  } catch (e) {
    const errMsg: Message = {
      id: crypto.randomUUID(),
      role: "system",
      content: `Error: ${e}`,
      timestamp: Date.now(),
    };
    appendScopedMessage("prompt_lab", errMsg);
    setScopedThinking("prompt_lab", false);
  }
}

function uint8ToBase64(bytes: Uint8Array): string {
  const chunkSize = 0x8000;
  let binary = "";
  for (let i = 0; i < bytes.length; i += chunkSize) {
    const chunk = bytes.subarray(i, i + chunkSize);
    binary += String.fromCharCode(...chunk);
  }
  return btoa(binary);
}

// ── Pending documents state ───────────────────────────────────────────────
const [pendingFiles, setPendingFiles] = createSignal<PendingFile[]>([]);

function addPendingFile(file: File) {
  const pf: PendingFile = {
    file,
    name: file.name,
    size: file.size,
    mime: file.type || "application/octet-stream",
    preview: file.type.startsWith("image/") ? URL.createObjectURL(file) : undefined,
  };
  setPendingFiles((prev) => [...prev, pf].slice(0, 10)); // cap at 10
}

function removePendingFile(index: number) {
  setPendingFiles((prev) => {
    const next = [...prev];
    const removed = next.splice(index, 1)[0];
    if (removed?.preview) URL.revokeObjectURL(removed.preview);
    return next;
  });
}

function clearPendingFiles() {
  setPendingFiles((prev) => {
    prev.forEach((f) => { if (f.preview) URL.revokeObjectURL(f.preview); });
    return [];
  });
}

async function sendDocumentMessage(files: PendingFile[], text?: string) {
  if (files.length === 0) return;
  if (isScopedThinking("assistant")) {
    notifyTurnBusy("assistant");
    return;
  }

  try {
    const sessionId = await ensureScopedSessionActive("assistant");

    // Build display info for the message bubble
    const fileInfos: AttachedFileInfo[] = files.map((f) => ({
      name: f.name,
      size: f.size,
      mime: f.mime,
    }));

    const userMsg: Message = {
      id: crypto.randomUUID(),
      role: "user",
      content: text?.trim() || `Analyze these files: ${files.map((f) => f.name).join(", ")}`,
      timestamp: Date.now(),
      attachedFiles: fileInfos,
    };
    appendScopedMessage("assistant", userMsg);
    setInputText("");
    clearPendingFiles();
    setScopedThinking("assistant", true);

    // Read file bytes
    const uploadedFiles = await Promise.all(
      files.map(async (pf) => {
        const buf = await pf.file.arrayBuffer();
        return {
          name: pf.name,
          bytes: Array.from(new Uint8Array(buf)),
          mime: pf.mime,
        };
      })
    );

    const result = await invoke<{ status: string; prompt: string }>("send_document_message", {
      sessionId,
      files: uploadedFiles,
      text: text?.trim() || null,
    });

    // The backend indexed the docs and returned the prompt to send;
    // now fire the normal agent turn with that prompt.
    if (result.status === "indexed" && result.prompt) {
      await sendMessage(result.prompt);
    }
  } catch (e) {
    const errMsg: Message = {
      id: crypto.randomUUID(),
      role: "system",
      content: `Document upload error: ${e}`,
      timestamp: Date.now(),
    };
    appendScopedMessage("assistant", errMsg);
    setScopedThinking("assistant", false);
  }
}

async function transcribeUploadedAudio(file: File) {
  const userMsg: Message = {
    id: crypto.randomUUID(),
    role: "user",
    content: `🎙️ Transcribe audio: ${file.name}`,
    timestamp: Date.now(),
    attachedFiles: [{ name: file.name, size: file.size, mime: file.type || "audio/*" }],
  };
  appendScopedMessage("assistant", userMsg);
  setScopedThinking("assistant", true);

  try {
    const buf = await file.arrayBuffer();
    const result = await invoke<{
      text: string;
      language: string;
      confidence: number;
      duration_ms: number;
      engine: string;
      name: string;
    }>("voice_transcribe_uploaded_audio", {
      name: file.name,
      bytes: Array.from(new Uint8Array(buf)),
    });

    const text = (result.text || "").trim();
    const reply: Message = {
      id: crypto.randomUUID(),
      role: "assistant",
      content: text.length > 0
        ? `📝 Transcript (${result.engine}, ${Math.round((result.confidence ?? 0) * 100)}%):\n\n${text}`
        : "📝 Transcript is empty.",
      timestamp: Date.now(),
    };
    appendScopedMessage("assistant", reply);
  } catch (e) {
    const errMsg: Message = {
      id: crypto.randomUUID(),
      role: "system",
      content: `Audio transcription error: ${e}`,
      timestamp: Date.now(),
    };
    appendScopedMessage("assistant", errMsg);
  } finally {
    setScopedThinking("assistant", false);
  }
}

async function sendImageMessage(imageData: Uint8Array, mimeType: string, text?: string) {
  if (isScopedThinking("assistant")) {
    notifyTurnBusy("assistant");
    return;
  }

  const b64 = uint8ToBase64(imageData);
  const dataUrl = `data:${mimeType};base64,${b64}`;

  try {
    const sessionId = await ensureScopedSessionActive("assistant");

    const userMsg: Message = {
      id: crypto.randomUUID(),
      role: "user",
      content: text || "What's in this image?",
      timestamp: Date.now(),
      imageUrl: dataUrl,
    };
    appendScopedMessage("assistant", userMsg);
    setInputText("");
    setScopedThinking("assistant", true);

    const promptForTitle = (text || "").trim();
    if (promptForTitle) {
      void autoRenameSessionFromPrompt(sessionId, promptForTitle);
    }
    await invoke<{ status: string; attachment: string }>(
      "send_image_message",
      { imageData: Array.from(imageData), mimeType, text: text || null }
    );
  } catch (e) {
    const errMsg: Message = {
      id: crypto.randomUUID(),
      role: "system",
      content: `Error: ${e}`,
      timestamp: Date.now(),
    };
    appendScopedMessage("assistant", errMsg);
    setScopedThinking("assistant", false);
  }
}

async function approveAction(requestId: string) {
  await invoke("approve_action", { requestId });
  setScopedHitl("assistant", null, false);
  setScopedHitl("prompt_lab", null, false);
}

async function denyAction(requestId: string, reason?: string) {
  await invoke("deny_action", { requestId, reason: reason ?? null });
  setScopedHitl("assistant", null, false);
  setScopedHitl("prompt_lab", null, false);
}

async function loadInteractionDecisions() {
  try {
    const payload = await invoke<{ decisions: InteractionDecision[]; metrics: DecisionMetrics }>(
      "list_interaction_decisions"
    );
    setInteractionDecisions(payload.decisions ?? []);
    setInteractionDecisionMetrics(payload.metrics ?? null);
  } catch (error) {
    console.warn("Failed to load interaction decisions:", error);
  }
}

async function resolveInteractionDecision(
  decisionId: string,
  optionId: string,
  decisionVersion?: number,
  expectedActionHash?: string,
  expectedTargetHash?: string
) {
  await invoke("resolve_interaction_decision", {
    decisionId,
    optionId,
    decisionVersion,
    expectedActionHash,
    expectedTargetHash,
  });
  await loadInteractionDecisions();
}

async function resumeInteractionDecision(
  decisionId: string,
  decisionVersion?: number,
  expectedActionHash?: string,
  expectedTargetHash?: string
) {
  const payload = await invoke("resume_interaction_decision", {
    decisionId,
    decisionVersion,
    expectedActionHash,
    expectedTargetHash,
  });
  await loadInteractionDecisions();
  return payload;
}

async function executeResolvedInteractionDecision(
  decisionId: string,
  decisionVersion?: number,
  expectedActionHash?: string,
  expectedTargetHash?: string
) {
  const payload = await invoke("execute_resolved_interaction_decision", {
    decisionId,
    decisionVersion,
    expectedActionHash,
    expectedTargetHash,
  });
  await loadInteractionDecisions();
  return payload;
}

async function cancelInteractionExecution(decisionId: string) {
  const payload = await invoke("cancel_interaction_execution", { decisionId });
  await loadInteractionDecisions();
  return payload;
}

async function checkContinuationAfterDecision(
  decisionId: string,
  expectedActionHash?: string,
  expectedTargetHash?: string
) {
  const payload = await invoke("check_continuation_after_decision", {
    decisionId,
    expectedActionHash,
    expectedTargetHash,
    allowStaleUserIntent: false,
  });
  await loadInteractionDecisions();
  return payload;
}

async function continueAfterDecisionExecution(
  decisionId: string,
  expectedActionHash?: string,
  expectedTargetHash?: string
) {
  const payload = await invoke("continue_after_decision_execution", {
    decisionId,
    expectedActionHash,
    expectedTargetHash,
    allowStaleUserIntent: false,
  });
  await loadInteractionDecisions();
  return payload;
}

async function cancelContinuation(decisionId: string) {
  const payload = await invoke("cancel_continuation", { decisionId });
  await loadInteractionDecisions();
  return payload;
}

async function cancelInteractionDecision(decisionId: string) {
  await invoke("cancel_interaction_decision", { decisionId });
  await loadInteractionDecisions();
}

async function replayInteractionDecisions() {
  return invoke<{ events: unknown[]; metrics: DecisionMetrics }>("replay_interaction_decisions");
}

async function toggleVoice() {
  if (voiceActive()) {
    suppressVoiceErrorUntil = Date.now() + 2500;
    await invoke("stop_voice");
    setVoiceActive(false);
    setVoiceState("idle");
    setVoiceLiveTranscript("");
    setVoiceLiveConfidence(null);
    setVoiceLiveStability(null);
    liveVoiceDraftMessageId = null;
  } else {
    try {
      await invoke("start_voice");
      setVoiceActive(true);
      setVoiceState("listening");
    } catch (e: any) {
      console.error("Failed to start voice:", e);
      const errText = typeof e === "string" ? e : e?.message ?? "Unknown error starting voice";
      const errMsg: Message = {
        id: crypto.randomUUID(),
        role: "system",
        content: `⚠️ Voice Error: ${errText}`,
        timestamp: Date.now(),
        toolCalls: [],
      };
      appendScopedMessage("assistant", errMsg);
      setVoiceActive(false);
      setVoiceState("idle");
    }
  }
}

// --- MCP Server management ---
async function loadMcpServers() {
  try {
    const result = await invoke<McpServer[]>("list_mcp_servers");
    setMcpServers(result);
  } catch (e) {
    console.error("Failed to load MCP servers:", e);
  }
}

async function addMcpServer(name: string, command: string, args: string[], trustLevel?: string) {
  try {
    await invoke("add_mcp_server", { name, command, args, trustLevel: trustLevel ?? null });
    await loadMcpServers();
  } catch (e) {
    console.error("Failed to add MCP server:", e);
    throw e;
  }
}

async function removeMcpServer(name: string) {
  try {
    await invoke("remove_mcp_server", { name });
    await loadMcpServers();
  } catch (e) {
    console.error("Failed to remove MCP server:", e);
    throw e;
  }
}

async function toggleMcpServer(name: string, enabled: boolean) {
  try {
    await invoke("toggle_mcp_server", { name, enabled });
    await loadMcpServers();
  } catch (e) {
    console.error("Failed to toggle MCP server:", e);
    throw e;
  }
}

// --- Health & Automation management ---
async function loadHealth() {
  if (healthLoadInFlight) {
    healthLoadQueued = true;
    return;
  }

  healthLoadInFlight = true;
  try {
    const result = await invoke<Record<string, any>>("get_health");
    setHealthInfo(result);
  } catch (e) {
    console.error("Failed to load health:", e);
  } finally {
    healthLoadInFlight = false;
    if (healthLoadQueued) {
      healthLoadQueued = false;
      void loadHealth();
    }
  }
}

async function loadRuntimeDiagnostics(limit = 128, minLevel = "info") {
  try {
    const result = await invoke<RuntimeDiagnosticsPayload>("get_runtime_diagnostics", {
      limit,
      minLevel,
    });
    setRuntimeDiagnostics(result);
    return result;
  } catch (e) {
    console.error("Failed to load runtime diagnostics:", e);
    return null;
  }
}

function assistantStatus(): AssistantStatus {
  const info = healthInfo();
  if (!info) {
    return {
      state: "warming",
      label: "Booting assistant",
      detail: "Running initial health checks",
    };
  }

  const services = Array.isArray(info.services) ? info.services : [];
  const modelRouter = services.find((svc: any) => svc?.name === "model_router");
  const statusRaw = String(modelRouter?.status ?? info.status ?? "unknown").toLowerCase();
  const message = String(modelRouter?.message ?? "").trim();

  if (statusRaw === "healthy") {
    return {
      state: "ready",
      label: "Assistant ready",
      detail: message || "Model routing online",
    };
  }

  if (statusRaw === "starting" || statusRaw === "unknown") {
    return {
      state: "warming",
      label: "Assistant warming up",
      detail: message || "Loading model runtime",
    };
  }

  if (statusRaw === "degraded") {
    return {
      state: "degraded",
      label: "Limited availability",
      detail: message || "Model service degraded",
    };
  }

  return {
    state: "offline",
    label: "Assistant unavailable",
    detail: message || "Model service is offline",
  };
}

async function loadScheduledTasks() {
  try {
    const result = await invoke<ScheduledTask[]>("list_scheduled_tasks");
    setScheduledTasks(result);
  } catch (e) {
    console.error("Failed to load tasks:", e);
  }
}

async function addScheduledTask(name: string, intervalSecs: number, prompt: string) {
  try {
    await invoke("add_scheduled_task", { name, intervalSecs, prompt });
    await loadScheduledTasks();
  } catch (e) {
    console.error("Failed to add task:", e);
    throw e;
  }
}

async function removeScheduledTask(taskId: string) {
  try {
    await invoke("remove_scheduled_task", { taskId });
    await loadScheduledTasks();
  } catch (e) {
    console.error("Failed to remove task:", e);
    throw e;
  }
}

async function loadMacros() {
  try {
    const result = await invoke<MacroInfo[]>("list_macros");
    setMacros(result);
  } catch (e) {
    console.error("Failed to load macros:", e);
  }
}

async function deleteMacro(name: string) {
  try {
    await invoke("delete_macro", { name });
    await loadMacros();
  } catch (e) {
    console.error("Failed to delete macro:", e);
    throw e;
  }
}

async function loadWorkflows() {
  try {
    const result = await invoke<WorkflowInfo[]>("list_workflows");
    setWorkflows(result);
  } catch (e) {
    console.error("Failed to load workflows:", e);
  }
}

async function deleteWorkflow(workflowId: string) {
  try {
    await invoke("delete_workflow", { workflowId });
    await loadWorkflows();
  } catch (e) {
    console.error("Failed to delete workflow:", e);
    throw e;
  }
}

async function loadHardwareInfo() {
  try {
    const result = await invoke<HardwareInfoData>("get_hardware_info");
    setHardwareInfo(result);
  } catch (e) {
    console.error("Failed to load hardware info:", e);
  }
}

async function loadKnowledgeBase() {
  try {
    const result = await invoke<{ documents: KnowledgeDoc[]; count: number }>("list_knowledge_base");
    setKnowledgeBase(result.documents);
  } catch (e) {
    console.error("Failed to load knowledge base:", e);
  }
}

async function loadAlerts() {
  try {
    const result = await invoke<{ alerts: ProactiveAlert[]; count: number }>("get_alerts");
    setAlerts(result.alerts);
  } catch (e) {
    console.error("Failed to load alerts:", e);
  }
}

// --- Telegram management ---
async function loadTelegramConfig() {
  try {
    const result = await invoke<TelegramConfig>("get_telegram_config");
    setTelegramConfig(result);
    if (!result.bot_token || !result.bot_token.trim()) {
      setTelegramBotInfo(null);
      persistTelegramBotInfo(null);
    }
  } catch (e) {
    console.error("Failed to load telegram config:", e);
  }
}

async function saveTelegramConfig(config: TelegramConfig) {
  try {
    await invoke("update_telegram_config", {
      enabled: config.enabled,
      botToken: config.bot_token,
      allowedChatIds: config.allowed_chat_ids,
      autoStart: config.auto_start,
    });
    setTelegramConfig(config);
    if (!config.bot_token || !config.bot_token.trim()) {
      setTelegramBotInfo(null);
      persistTelegramBotInfo(null);
    }
  } catch (e) {
    console.error("Failed to save telegram config:", e);
    throw e;
  }
}

async function testTelegramConnection(botToken: string): Promise<TelegramBotInfo> {
  const result = await invoke<TelegramBotInfo>("test_telegram_connection", { botToken });
  setTelegramBotInfo(result);
  persistTelegramBotInfo(result);
  return result;
}

async function startTelegramMcp() {
  try {
    const result = await invoke<{ status: string; message: string }>("start_telegram_mcp");
    await loadMcpServers();
    return result;
  } catch (e) {
    console.error("Failed to start telegram MCP:", e);
    throw e;
  }
}

async function stopTelegramMcp() {
  try {
    await invoke("stop_telegram_mcp");
    await loadMcpServers();
    await loadTelegramConfig();
  } catch (e) {
    console.error("Failed to stop telegram MCP:", e);
    throw e;
  }
}

// --- Google Workspace ---
export interface GoogleWorkspaceMcpStatus {
  configured_enabled: boolean;
  state: string;
  tool_count: number;
  error: string | null;
}

export interface GoogleWorkspaceCapabilities {
  gmail: boolean;
  drive: boolean;
  calendar: boolean;
  docs: boolean;
  sheets: boolean;
  slides: boolean;
  forms: boolean;
  meet: boolean;
  meet_via_calendar: boolean;
}

export interface GoogleWorkspaceStatus {
  connected: boolean;
  account: string;
  credentials_configured: boolean;
  token_present: boolean;
  account_registered: boolean;
  token_path?: string;
  requires_reauth?: boolean;
  auth_ready: boolean;
  runtime_ready: boolean;
  gw_client_wired: boolean;
  mcp: GoogleWorkspaceMcpStatus;
  capabilities: GoogleWorkspaceCapabilities;
  config_dir?: string;
  meet_support_mode: string;
  warnings: string[];
}

const [googleStatus, setGoogleStatus] = createSignal<GoogleWorkspaceStatus | null>(null);

export interface ColabMcpStatus {
  state: string;
  tool_count: number;
  error: string | null;
}

export interface ColabDiscoveredTool {
  name: string;
  operation: string;
  description: string;
  parameter_count: number;
  last_failure?: McpFailureRecord | null;
  health?: string;
  remediation?: string | null;
}

export interface ColabCapabilities {
  category: string;
  tool_count: number;
  discovered_tools: ColabDiscoveredTool[];
  features: {
    notebook_discovery: boolean;
    notebook_selection: boolean;
    cell_execution: boolean;
    artifact_io: boolean;
    runtime_lifecycle: boolean;
    package_management: boolean;
    checkpointing: boolean;
  };
  ready_requirements: {
    requires: string[];
    satisfied: boolean;
    missing: string[];
  };
}

export interface ColabTierStatus {
  enabled: boolean;
  connected: boolean;
  ready_for_cloud_task: boolean;
  notebook_selection_required: boolean;
  runtime_state: string;
  selected_notebook: string | null;
  mcp_server_name: string;
  auto_escalate: boolean;
  fallback_to_local: boolean;
  connect_timeout_secs: number;
  keepalive_interval_secs: number;
  checkpoint_interval_secs: number;
  mcp: ColabMcpStatus;
  capabilities: ColabCapabilities;
  warnings: string[];
}

const [colabStatus, setColabStatus] = createSignal<ColabTierStatus | null>(null);

export interface IroncladResetSnapshot {
  event_id: string;
  phase: string;
  reason: string;
  detail: string;
  started_unix_ms: number;
  completed_unix_ms: number | null;
  in_flight: boolean;
}

export interface IroncladForensicRecord {
  id: string;
  timestamp_unix_ms: number;
  category: string;
  severity: string;
  summary: string;
  source: string;
  evidence: string;
  last_gasp_detected: boolean;
}

export interface IroncladEnrolledTargetSnapshot {
  target_id: string;
  display_name: string;
  host: string;
  port: number;
  username: string;
  mode: string;
  ssh_hostkey_sha256_b64: string;
  controller_epoch: number;
  enrolled_at_unix_ms: number;
  last_verified_unix_ms: number;
}

export interface IroncladFleetStatus {
  total_targets: number;
  ready_targets: number;
  leased_targets: number;
  tainted_targets: number;
  quarantined_targets: number;
  active_leases: number;
  health_degraded: boolean;
  source_unwired: boolean;
  enrolled_target_count?: number;
  enrolled_targets?: IroncladEnrolledTargetSnapshot[];
  enrollment_registry_path?: string;
}

export interface IroncladQosStatus {
  traffic_light: "green" | "yellow" | "red" | "gray";
  pressure_active: boolean;
  high_recovery_wait_p95_ms: number;
  high_recovery_slo_ms: number;
  decision?: string | null;
  reason?: string | null;
}

export interface IroncladConfigSnapshot {
  high_recovery_slo_ms: number;
  lease_ttl_ms: number;
  heartbeat_grace_ms: number;
  quarantine_cooldown_ms: number;
  max_normalized_hash_distance: number;
}

export interface IroncladConfigUpdatePayload {
  high_recovery_slo_ms?: number;
  lease_ttl_ms?: number;
  heartbeat_grace_ms?: number;
  quarantine_cooldown_ms?: number;
  max_normalized_hash_distance?: number;
}

export interface IroncladStatus {
  enabled: boolean;
  fleet: IroncladFleetStatus;
  qos: IroncladQosStatus;
  reset: IroncladResetSnapshot;
  forensics: {
    count: number;
    latest?: IroncladForensicRecord | null;
  };
  config_path?: string;
  config?: IroncladConfigSnapshot;
}

export type RegisterNewTargetErrorCode =
  | "validation_failed"
  | "connection_refused"
  | "authentication_failed"
  | "host_key_changed"
  | "dependency_missing"
  | "bootstrap_failed"
  | "persistence_failed"
  | "unknown";

export interface RegisterNewTargetRequest {
  displayName: string;
  host: string;
  port?: number;
  username: string;
  sshPrivateKeyPath?: string;
  expectedHostkeySha256?: string;
  commanderEpoch?: number;
}

export interface RegisterNewTargetResponse {
  targetId: string;
  displayName: string;
  host: string;
  port: number;
  username: string;
  mode: string;
  sshHostkeySha256B64: string;
  sshPrivateKeyPath: string;
  sshPublicKeyPath: string;
  commanderEpoch: number;
  createdNewTarget: boolean;
  createdLocalKey: boolean;
  enrolledAtUnixMs: number;
  registryPath: string;
}

export interface RegisterNewTargetErrorPayload {
  code: RegisterNewTargetErrorCode;
  message: string;
  detail?: string;
}

const [ironcladStatus, setIroncladStatus] = createSignal<IroncladStatus | null>(null);
const [ironcladForensics, setIroncladForensics] = createSignal<IroncladForensicRecord[]>([]);
const [ironcladForensicsTotal, setIroncladForensicsTotal] = createSignal(0);
const [ironcladResetEvent, setIroncladResetEvent] = createSignal<IroncladResetSnapshot | null>(null);

async function loadGoogleStatus(account?: string): Promise<GoogleWorkspaceStatus | null> {
  try {
    const result = await invoke<GoogleWorkspaceStatus>("get_google_workspace_status", { account: account ?? null });
    setGoogleStatus(result);
    return result;
  } catch (e) {
    console.error("Failed to load Google status:", e);
    return null;
  }
}

async function connectGoogle(account?: string): Promise<{ status: string; message: string; account: string }> {
  const result = await invoke<{ status: string; message: string; account: string }>(
    "connect_google_workspace",
    { account: account ?? null }
  );
  return result;
}

async function setGoogleAccount(account: string): Promise<{ account: string; updated: boolean }> {
  return invoke<{ account: string; updated: boolean }>("set_google_workspace_account", { account });
}

async function reconcileMcpRuntime() {
  return invoke<Record<string, unknown>>("reconcile_mcp_runtime");
}

async function restartMcpServerRuntime(name: string) {
  return invoke<Record<string, unknown>>("restart_mcp_server_runtime", { name });
}

async function disconnectGoogle(account?: string) {
  await invoke("disconnect_google_workspace", { account: account ?? null });
  await loadGoogleStatus(account);
}

// --- Colab Tier ---
async function loadColabStatus(): Promise<ColabTierStatus | null> {
  try {
    const result = await invoke<ColabTierStatus>("get_colab_tier_status");
    setColabStatus(result);
    return result;
  } catch (e) {
    console.error("Failed to load Colab status:", e);
    return null;
  }
}

async function connectColab(serverName?: string): Promise<ColabTierStatus | null> {
  await invoke("connect_colab_tier", { serverName: serverName ?? null });
  await loadMcpServers();
  return loadColabStatus();
}

async function disconnectColab(): Promise<ColabTierStatus | null> {
  await invoke("disconnect_colab_tier");
  await loadMcpServers();
  return loadColabStatus();
}

async function setColabNotebook(notebookId: string): Promise<ColabTierStatus | null> {
  const result = await invoke<ColabTierStatus>("set_colab_selected_notebook", { notebookId });
  setColabStatus(result);
  return result;
}

async function loadIroncladStatus(): Promise<IroncladStatus | null> {
  try {
    const result = await invoke<IroncladStatus>("get_ironclad_status");
    setIroncladStatus(result);
    if (result?.reset) {
      setIroncladResetEvent(result.reset);
    }
    if (result?.forensics?.count !== undefined) {
      setIroncladForensicsTotal(result.forensics.count);
    }
    return result;
  } catch (e) {
    console.error("Failed to load Ironclad status:", e);
    return null;
  }
}

async function loadIroncladForensics(limit = 64): Promise<IroncladForensicRecord[]> {
  try {
    const result = await invoke<{ total: number; limit: number; records: IroncladForensicRecord[] }>(
      "get_ironclad_forensics",
      { limit }
    );
    const sorted = [...(result.records ?? [])].sort(
      (a, b) => b.timestamp_unix_ms - a.timestamp_unix_ms
    );
    setIroncladForensics(sorted);
    setIroncladForensicsTotal(result.total ?? sorted.length);
    return sorted;
  } catch (e) {
    console.error("Failed to load Ironclad forensics:", e);
    return [];
  }
}

async function requestIroncladSoftReset(reason?: string): Promise<Record<string, unknown>> {
  const result = await invoke<Record<string, unknown>>("request_ironclad_soft_reset", {
    reason: reason?.trim() ? reason.trim() : null,
  });
  void loadIroncladStatus();
  return result;
}

async function requestIroncladHardReset(
  confirmationPhrase: string,
  reason?: string
): Promise<Record<string, unknown>> {
  const result = await invoke<Record<string, unknown>>("request_ironclad_hard_reset", {
    reason: reason?.trim() ? reason.trim() : null,
    confirmationPhrase,
  });
  void loadIroncladStatus();
  return result;
}

async function getIroncladConfig(): Promise<{ path: string; exists: boolean; config: IroncladConfigSnapshot }> {
  return invoke<{ path: string; exists: boolean; config: IroncladConfigSnapshot }>("get_ironclad_config");
}

async function updateIroncladConfig(
  payload: IroncladConfigUpdatePayload
): Promise<{ updated: boolean; config?: IroncladConfigSnapshot; applied?: string[]; reason?: string }> {
  const result = await invoke<{
    updated: boolean;
    config?: IroncladConfigSnapshot;
    applied?: string[];
    reason?: string;
  }>("update_ironclad_config", { payload });

  if (result?.updated) {
    await Promise.all([loadIroncladStatus(), loadIroncladForensics(64)]);
  }

  return result;
}

async function registerNewTarget(
  request: RegisterNewTargetRequest
): Promise<RegisterNewTargetResponse> {
  const response = await invoke<RegisterNewTargetResponse>("register_new_target", { request });
  void loadIroncladStatus();
  return response;
}

async function deleteTarget(targetId: string): Promise<void> {
  await invoke("delete_target", { targetId });
  void loadIroncladStatus();
}

interface UpdateTargetRequest {
  targetId: string;
  displayName?: string;
  host?: string;
  port?: number;
  username?: string;
  sshPrivateKeyPath?: string;
}

async function updateTarget(request: UpdateTargetRequest): Promise<void> {
  await invoke("update_target", { request });
  void loadIroncladStatus();
}

function submitToolChoice(candidateName: string) {
  const req = toolChoiceRequest();
  if (!req) return;
  const scope = scopeFromEnvironment();
  setScopedToolChoice(scope, null);

  const forcedText = `#tool:${candidateName} ${req.query}`;
  if (scope === "prompt_lab") {
    void sendLabMessage(forcedText, lastPromptLabProfile());
  } else {
    void sendMessage(forcedText);
  }
}

function dismissToolChoice() {
  setScopedToolChoice(scopeFromEnvironment(), null);
}

// --- Settings management ---
const FONT_SCALE_VALUES = ["0.8", "0.9", "1.0", "1.2", "1.5", "2.0"] as const;

function normalizeFontScale(value: unknown): string {
  const parsed = Number.parseFloat(String(value ?? "1"));
  if (Number.isNaN(parsed)) return "1.0";

  const exact = FONT_SCALE_VALUES.find((candidate) => Math.abs(Number.parseFloat(candidate) - parsed) < 0.001);
  return exact ?? "1.0";
}

function applyUiRuntimePreferences(ui: Record<string, any> | null | undefined) {
  if (typeof document === "undefined") return;

  const root = document.documentElement;
  root.setAttribute("data-high-contrast", String(Boolean(ui?.high_contrast)));
  root.setAttribute("data-reduce-motion", String(Boolean(ui?.reduce_motion)));
  root.setAttribute("data-font-scale", normalizeFontScale(ui?.font_scale));
}

async function loadSettings() {
  try {
    const result = await invoke<Record<string, any>>("get_settings");
    setSettings(result);
    // Keep theme deterministic: default to light unless explicitly dark.
    const resolvedTheme: "dark" | "light" = result?.ui?.theme === "dark" ? "dark" : "light";
    applyTheme(resolvedTheme);
    applyUiRuntimePreferences(result?.ui);
  } catch (e) {
    console.error("Failed to load settings:", e);
  }
}

async function loadAudioDevices() {
  try {
    const result = await invoke<AudioDevicesData>("list_audio_devices");
    setAudioDevices(result);
  } catch (e) {
    console.error("Failed to load audio devices:", e);
    setAudioDevices({
      inputs: [],
      outputs: [],
      default_input: null,
      default_output: null,
    });
  }
}

async function saveSettings(newSettings: Record<string, any>) {
  try {
    await invoke("update_settings", { settings: newSettings });
    setSettings(newSettings);
    const resolvedTheme: "dark" | "light" = newSettings?.ui?.theme === "dark" ? "dark" : "light";
    applyTheme(resolvedTheme);
    applyUiRuntimePreferences(newSettings?.ui);
  } catch (e) {
    console.error("Failed to save settings:", e);
    throw e;
  }
}

async function loadModels() {
  try {
    const result = await invoke<any[]>("list_models");
    setModels(result);
  } catch (e) {
    console.error("Failed to load models:", e);
  }
}

function applyTheme(t: "dark" | "light") {
  setTheme(t);
  if (typeof window !== "undefined") {
    window.localStorage.setItem(STORAGE_KEYS.theme, t);
  }
  if (typeof document !== "undefined") {
    document.documentElement.setAttribute("data-theme", t);
  }
  setHighlightThemeStylesheet(t);
}

function setHighlightThemeStylesheet(t: "dark" | "light") {
  if (typeof document === "undefined") return;

  const linkId = "kria-hljs-theme";
  const href = t === "light" ? hljsLightThemeUrl : hljsDarkThemeUrl;
  const existing = document.getElementById(linkId) as HTMLLinkElement | null;

  if (existing) {
    existing.href = href;
    return;
  }

  const link = document.createElement("link");
  link.id = linkId;
  link.rel = "stylesheet";
  link.href = href;
  document.head.appendChild(link);
}

// --- Session management ---
async function loadSessions(): Promise<Session[] | null> {
  try {
    // Guard against a backend command that never resolves (e.g. a lock/deadlock
    // in `list_sessions`): a hanging invoke would otherwise leave the startup
    // spinner ("Loading conversations...") on forever, because the retry/settle
    // path only runs when the promise settles. Race it against a timeout so a
    // stall is treated as a (retryable) failure instead of an infinite hang.
    const result = await invokeWithTimeout<{ id: string; title: string; turn_count: number; last_active: string }[]>(
      "list_sessions",
      undefined,
      LIST_SESSIONS_TIMEOUT_MS,
    );
    const mapped: Session[] = result.map((s) => ({
      id: s.id,
      title: s.title || "Untitled",
      updatedAt: new Date(s.last_active).getTime() || Date.now(),
      turnCount: typeof s.turn_count === "number" ? s.turn_count : 0,
    }));

    const previousById = new Map(sessions().map((session) => [session.id, session]));
    const activeSessionIds = [
      assistantCurrentSession(),
      promptLabCurrentSession(),
    ].filter((id): id is string => Boolean(id && id.trim()));

    for (const sessionId of activeSessionIds) {
      if (mapped.some((session) => session.id === sessionId)) continue;
      const previous = previousById.get(sessionId);
      mapped.push(previous ?? {
        id: sessionId,
        title: "New chat",
        updatedAt: Date.now(),
        turnCount: 0,
      });
    }

    mapped.sort((a, b) => b.updatedAt - a.updatedAt);
    setSessions(mapped);
    return mapped;
  } catch (e) {
    console.error("Failed to load sessions:", e);
    return null;
  }
}

function normalizeSessionTitleFromPrompt(prompt: string): string | null {
  const collapsed = prompt.replace(/\s+/g, " ").trim();
  if (!collapsed) return null;

  const trimmed = collapsed.replace(/^["'`\s]+|["'`\s]+$/g, "");
  if (!trimmed) return null;

  const maxChars = 72;
  return trimmed.length > maxChars
    ? `${trimmed.slice(0, maxChars).trimEnd()}…`
    : trimmed;
}

function upsertSessionPreview(sessionId: string, title: string) {
  setSessions((prev) => {
    const idx = prev.findIndex((session) => session.id === sessionId);
    const nextItem: Session = {
      id: sessionId,
      title,
      updatedAt: Date.now(),
    };

    if (idx === -1) {
      return [nextItem, ...prev];
    }

    const next = [...prev];
    next[idx] = {
      ...next[idx],
      title,
      updatedAt: Date.now(),
    };
    return next;
  });
}

async function autoRenameSessionFromPrompt(sessionId: string, prompt: string) {
  const normalizedTitle = normalizeSessionTitleFromPrompt(prompt);
  if (!normalizedTitle) return;

  try {
    const result = await invoke<{ updated?: boolean; title?: string }>(
      "auto_rename_session",
      { sessionId, title: normalizedTitle }
    );

    if (result?.updated) {
      upsertSessionPreview(sessionId, result.title || normalizedTitle);
    }
  } catch (e) {
    console.warn("Failed to auto-rename session:", e);
  }
}

async function createSession() {
  try {
    const scope = scopeFromEnvironment();
    await cancelScopedTurnIfActive(scope);
    const result = await invoke<{ session_id: string }>("create_session");
    setScopedCurrentSession(scope, result.session_id);
    backendActiveSessionId = result.session_id;
    upsertSessionPreview(result.session_id, "New chat");
    updateScopedMessages(scope, () => []);
    setScopedToolChoice(scope, null);
    setScopedThinking(scope, false);
    await loadSessions();
  } catch (e) {
    console.error("Failed to create session:", e);
  }
}

function normalizeRole(role: string): Message["role"] {
  if (role === "user" || role === "assistant" || role === "system" || role === "tool") {
    return role;
  }
  return "assistant";
}

function parseStoredToolCall(
  toolName: string,
  rawToolResult: string | null | undefined
): ToolCall {
  let parsed: any = null;
  if (rawToolResult) {
    try {
      parsed = JSON.parse(rawToolResult);
    } catch {
      parsed = rawToolResult;
    }
  }

  const args =
    parsed &&
    typeof parsed === "object" &&
    parsed.args &&
    typeof parsed.args === "object" &&
    !Array.isArray(parsed.args)
      ? (parsed.args as Record<string, unknown>)
      : {};

  const success =
    parsed && typeof parsed === "object" && typeof parsed.success === "boolean"
      ? parsed.success
      : true;

  const result =
    parsed && typeof parsed === "object" && "result" in parsed
      ? parsed.result
      : parsed ?? null;

  const metadataRaw = parsed && typeof parsed === "object" ? parsed.metadata : null;
  const metadata: ToolResultMetadata | undefined =
    metadataRaw && typeof metadataRaw === "object"
      ? {
          confidence: typeof metadataRaw.confidence === "number" ? metadataRaw.confidence : undefined,
          sourceCount: typeof metadataRaw.source_count === "number" ? metadataRaw.source_count : undefined,
          freshnessAgeHours:
            typeof metadataRaw.freshness_age_hours === "number" || metadataRaw.freshness_age_hours === null
              ? metadataRaw.freshness_age_hours
              : undefined,
          regionMatch:
            typeof metadataRaw.region_match === "boolean" || metadataRaw.region_match === null
              ? metadataRaw.region_match
              : undefined,
        }
      : undefined;

  return {
    name: toolName,
    args,
    result,
    status: (success ? "done" : "error") as ToolCall["status"],
    metadata,
  };
}

async function loadMappedSessionHistory(sessionId: string): Promise<Message[]> {
  const history = await invoke<{
    role: string;
    content: string;
    timestamp: string;
    tool_name?: string | null;
    tool_result?: string | null;
  }[]>(
    "get_session_history",
    { sessionId }
  );

  const mapped: Message[] = [];
  for (const t of history) {
    const ts = new Date(t.timestamp).getTime() || Date.now();

    if (t.tool_name) {
      const tc = parseStoredToolCall(t.tool_name, t.tool_result);

      // New persistence format stores tool turns as assistant rows that carry
      // both summary text and structured tool payload.
      if (normalizeRole(t.role) === "assistant") {
        mapped.push({
          id: crypto.randomUUID(),
          role: "assistant",
          content: t.content,
          timestamp: ts,
          toolCalls: [tc],
        });
        continue;
      }

      // Backward compatibility for legacy role=tool rows.
      if (normalizeRole(t.role) === "tool") {
        const last = mapped[mapped.length - 1];
        if (last?.role === "assistant") {
          mapped[mapped.length - 1] = {
            ...last,
            toolCalls: [...(last.toolCalls || []), tc],
            timestamp: Math.max(last.timestamp, ts),
          };
        } else {
          mapped.push({
            id: crypto.randomUUID(),
            role: "assistant",
            content: "",
            timestamp: ts,
            toolCalls: [tc],
          });
        }
        continue;
      }
    }

    mapped.push({
      id: crypto.randomUUID(),
      role: normalizeRole(t.role),
      content: t.content,
      timestamp: ts,
    });
  }

  return mapped;
}

function messageIdentity(message: Message): string {
  const toolSignature = (message.toolCalls || [])
    .map((tool) => {
      let result = "";
      try {
        result = JSON.stringify(tool.result ?? null);
      } catch {
        result = String(tool.result ?? "");
      }
      return `${tool.name}:${tool.status}:${result}`;
    })
    .join("|");
  return `${message.role}\u0000${message.content}\u0000${toolSignature}`;
}

function mergeHistoryWithLocalMessages(history: Message[], localMessages: Message[]): Message[] {
  if (localMessages.length === 0) return history;

  const seenCounts = new Map<string, number>();
  for (const message of history) {
    const key = messageIdentity(message);
    seenCounts.set(key, (seenCounts.get(key) ?? 0) + 1);
  }

  const missingLocal = localMessages.filter((message) => {
    const key = messageIdentity(message);
    const remaining = seenCounts.get(key) ?? 0;
    if (remaining > 0) {
      seenCounts.set(key, remaining - 1);
      return false;
    }
    return true;
  });
  if (missingLocal.length === 0) return history;

  return [...history, ...missingLocal].sort((a, b) => a.timestamp - b.timestamp);
}

async function switchSession(sessionId: string) {
  try {
    const scope = scopeFromEnvironment();
    const activeSession = getScopedCurrentSession(scope);
    const wasAlreadyCurrent = activeSession === sessionId;
    const localMessages = scope === "prompt_lab" ? promptLabMessages() : assistantMessages();
    if (activeSession && activeSession !== sessionId) {
      await cancelScopedTurnIfActive(scope);
    }
    await invoke("switch_session", { sessionId });
    backendActiveSessionId = sessionId;
    setScopedCurrentSession(scope, sessionId);
    let mapped = await loadMappedSessionHistory(sessionId);
    if (wasAlreadyCurrent) {
      mapped = mergeHistoryWithLocalMessages(mapped, localMessages);
    }
    updateScopedMessages(scope, () => mapped);
    setScopedThinking(scope, false);
  } catch (e) {
    console.error("Failed to switch session:", e);
  }
}

async function deleteSession(sessionId: string) {
  const previousSessions = sessions();
  const previousAssistantSession = assistantCurrentSession();
  const previousPromptLabSession = promptLabCurrentSession();
  const previousAssistantMessages = assistantMessages();
  const previousPromptLabMessages = promptLabMessages();
  const previousAssistantThinking = assistantIsThinking();
  const previousPromptLabThinking = promptLabIsThinking();

  try {
    setSessions((prev) => prev.filter((session) => session.id !== sessionId));

    if (assistantCurrentSession() === sessionId) {
      setScopedCurrentSession("assistant", null);
      setAssistantMessages([]);
      setAssistantIsThinking(false);
      setScopedToolChoice("assistant", null);
      setScopedHitl("assistant", null, false);
    }
    if (promptLabCurrentSession() === sessionId) {
      setScopedCurrentSession("prompt_lab", null);
      setPromptLabMessages([]);
      setPromptLabIsThinking(false);
      setScopedToolChoice("prompt_lab", null);
      setScopedHitl("prompt_lab", null, false);
    }

    const result = await invoke<{ replacement_session_id?: string | null }>(
      "delete_session",
      { sessionId }
    );

    const replacementSessionId = result?.replacement_session_id || null;
    if (replacementSessionId) {
      if (!assistantCurrentSession()) {
        setScopedCurrentSession("assistant", replacementSessionId);
      }
      if (!promptLabCurrentSession()) {
        setScopedCurrentSession("prompt_lab", replacementSessionId);
      }
      backendActiveSessionId = replacementSessionId;
      upsertSessionPreview(replacementSessionId, "New chat");
    }

    void loadSessions();
  } catch (e) {
    console.error("Failed to delete session:", e);
    setSessions(previousSessions);
    setScopedCurrentSession("assistant", previousAssistantSession);
    setScopedCurrentSession("prompt_lab", previousPromptLabSession);
    setAssistantMessages(previousAssistantMessages);
    setPromptLabMessages(previousPromptLabMessages);
    setAssistantIsThinking(previousAssistantThinking);
    setPromptLabIsThinking(previousPromptLabThinking);
  }
}

async function clearAllChatSessions(): Promise<{ deletedSessionCount: number; deletedTurnCount: number }> {
  const previousSessions = sessions();
  const previousAssistantSession = assistantCurrentSession();
  const previousPromptLabSession = promptLabCurrentSession();
  const previousAssistantMessages = assistantMessages();
  const previousPromptLabMessages = promptLabMessages();
  const previousAssistantThinking = assistantIsThinking();
  const previousPromptLabThinking = promptLabIsThinking();

  try {
    await cancelScopedTurnIfActive("assistant");
    await cancelScopedTurnIfActive("prompt_lab");

    setSessions([]);
    setAssistantMessages([]);
    setPromptLabMessages([]);
    setAssistantIsThinking(false);
    setPromptLabIsThinking(false);
    setScopedToolChoice("assistant", null);
    setScopedToolChoice("prompt_lab", null);
    setScopedHitl("assistant", null, false);
    setScopedHitl("prompt_lab", null, false);

    const result = await invoke<{
      deleted_session_count?: number;
      deleted_turn_count?: number;
      replacement_session_id?: string | null;
    }>("clear_all_chat_sessions");

    const replacementSessionId = result.replacement_session_id || null;
    if (replacementSessionId) {
      setScopedCurrentSession("assistant", replacementSessionId);
      setScopedCurrentSession("prompt_lab", replacementSessionId);
      backendActiveSessionId = replacementSessionId;
      upsertSessionPreview(replacementSessionId, "New chat");
    } else {
      setScopedCurrentSession("assistant", null);
      setScopedCurrentSession("prompt_lab", null);
      backendActiveSessionId = null;
    }

    await loadSessions();

    return {
      deletedSessionCount: result.deleted_session_count ?? previousSessions.length,
      deletedTurnCount: result.deleted_turn_count ?? 0,
    };
  } catch (e) {
    console.error("Failed to clear chat sessions:", e);
    setSessions(previousSessions);
    setScopedCurrentSession("assistant", previousAssistantSession);
    setScopedCurrentSession("prompt_lab", previousPromptLabSession);
    setAssistantMessages(previousAssistantMessages);
    setPromptLabMessages(previousPromptLabMessages);
    setAssistantIsThinking(previousAssistantThinking);
    setPromptLabIsThinking(previousPromptLabThinking);
    throw e;
  }
}

async function renameSession(sessionId: string, title: string) {
  const normalizedTitle = normalizeSessionTitleFromPrompt(title);
  if (!normalizedTitle) return;

  const previousSessions = sessions();
  upsertSessionPreview(sessionId, normalizedTitle);

  try {
    await invoke("rename_session", { sessionId, title: normalizedTitle });
    void loadSessions();
  } catch (e) {
    console.error("Failed to rename session:", e);
    setSessions(previousSessions);
  }
}

// --- Event listeners (set up once) ---
function initListeners() {
  const registerStreamListeners = (eventPrefix: "agent" | "prompt_lab", scope: StreamScope) => {
    listen<{ text: string }>(`${eventPrefix}:token`, (event) => {
      if (scope === "assistant") pokeAssistantThinkingWatchdog();
      updateScopedMessages(scope, (prev) => {
        const last = prev[prev.length - 1];
        if (last?.role === "assistant") {
          return [
            ...prev.slice(0, -1),
            { ...last, content: last.content + event.payload.text },
          ];
        }
        return [
          ...prev,
          {
            id: crypto.randomUUID(),
            role: "assistant",
            content: event.payload.text,
            timestamp: Date.now(),
          },
        ];
      });
    });

    listen<{ status?: string; plan?: string }>(`${eventPrefix}:thinking`, () => {
      setScopedThinking(scope, true);
    });

    listen(`${eventPrefix}:done`, () => {
      setScopedThinking(scope, false);
      loadSessions();
      loadHealth();
    });

    listen<HitlRequest>(`${eventPrefix}:approval_required`, (event) => {
      setScopedHitl(scope, event.payload, true);
    });

    listen<ToolChoiceRequest>(`${eventPrefix}:tool_choice_required`, (event) => {
      setScopedToolChoice(scope, event.payload);
      setScopedThinking(scope, false);
    });

    listen<{ name: string; params: Record<string, unknown> }>(`${eventPrefix}:tool_call`, (event) => {
      const { name, params } = event.payload;
      updateScopedMessages(scope, (prev) => {
        const last = prev[prev.length - 1];
        if (last?.role === "assistant") {
          const tc: ToolCall = { name, args: params, status: "running" };
          return [
            ...prev.slice(0, -1),
            { ...last, toolCalls: [...(last.toolCalls || []), tc] },
          ];
        }
        return [
          ...prev,
          {
            id: crypto.randomUUID(),
            role: "assistant",
            content: "",
            timestamp: Date.now(),
            toolCalls: [{ name, args: params, status: "running" }],
          },
        ];
      });
    });

    listen<{
      name: string;
      result: unknown;
      success: boolean;
      metadata?: {
        confidence?: number;
        source_count?: number;
        freshness_age_hours?: number | null;
        region_match?: boolean | null;
      } | null;
      conversational_summary?: string;
      human_readable?: string;
      execution_metadata?: ExecutionMetadata;
    }>(`${eventPrefix}:tool_result`, (event) => {
      const { name, result, success, metadata, conversational_summary, human_readable, execution_metadata } = event.payload;
      const completedToolCall: ToolCall = {
        name,
        args: {},
        status: (success ? "done" : "error") as ToolCall["status"],
        result,
        metadata: metadata
          ? {
              confidence: metadata.confidence,
              sourceCount: metadata.source_count,
              freshnessAgeHours: metadata.freshness_age_hours,
              regionMatch: metadata.region_match,
            }
          : undefined,
        conversational_summary: conversational_summary ?? undefined,
        human_readable: human_readable ?? undefined,
        execution_metadata: execution_metadata ?? undefined,
      };

      updateScopedMessages(scope, (prev) => {
        for (let i = prev.length - 1; i >= 0; i--) {
          const msg = prev[i];
          if (msg.role !== "assistant" || !msg.toolCalls?.length) continue;

          let didUpdate = false;
          const updated = msg.toolCalls.map((tc) => {
            if (tc.name === name && tc.status === "running") {
              didUpdate = true;
              return {
                ...tc,
                status: completedToolCall.status,
                result: completedToolCall.result,
                metadata: completedToolCall.metadata ?? tc.metadata,
                conversational_summary: completedToolCall.conversational_summary,
                human_readable: completedToolCall.human_readable,
                execution_metadata: completedToolCall.execution_metadata,
              };
            }
            return tc;
          });

          if (didUpdate) {
            return [
              ...prev.slice(0, i),
              { ...msg, toolCalls: updated },
              ...prev.slice(i + 1),
            ];
          }
        }

        const last = prev[prev.length - 1];
        if (last?.role === "assistant") {
          let alreadyPresent = false;
          const newResultSig = (() => {
            try {
              return JSON.stringify(completedToolCall.result);
            } catch {
              return String(completedToolCall.result ?? "");
            }
          })();

          for (const tc of last.toolCalls || []) {
            if (tc.name !== name || tc.status === "running") continue;
            let existingSig = "";
            try {
              existingSig = JSON.stringify(tc.result);
            } catch {
              existingSig = String(tc.result ?? "");
            }
            if (existingSig === newResultSig) {
              alreadyPresent = true;
              break;
            }
          }

          if (alreadyPresent) {
            return prev;
          }

          return [
            ...prev.slice(0, -1),
            {
              ...last,
              toolCalls: [...(last.toolCalls || []), completedToolCall],
            },
          ];
        }

        return [
          ...prev,
          {
            id: crypto.randomUUID(),
            role: "assistant",
            content: "",
            timestamp: Date.now(),
            toolCalls: [completedToolCall],
          },
        ];
      });
    });

    listen<{ action: string; approved: boolean }>(`${eventPrefix}:approval_result`, (event) => {
      if (!event.payload.approved) {
        updateScopedMessages(scope, (prev) => {
          const last = prev[prev.length - 1];
          if (last?.role === "assistant" && last.toolCalls?.length) {
            const updated = last.toolCalls.map((tc) => {
              if (tc.name === event.payload.action && tc.status === "running") {
                return { ...tc, status: "denied" as ToolCall["status"], result: "User denied" };
              }
              return tc;
            });
            return [...prev.slice(0, -1), { ...last, toolCalls: updated }];
          }
          return prev;
        });
      }
    });

    listen<RecoveryOptionsPayload>(`${eventPrefix}:recovery_options`, (event) => {
      const payload = event.payload;
      updateScopedMessages(scope, (prev) => {
        // Attach recovery options to the last assistant message
        const last = prev[prev.length - 1];
        if (last?.role === "assistant") {
          return [...prev.slice(0, -1), { ...last, recoveryOptions: payload }];
        }
        // Or create a new assistant message to hold them
        return [
          ...prev,
          {
            id: crypto.randomUUID(),
            role: "assistant" as const,
            content: "",
            timestamp: Date.now(),
            recoveryOptions: payload,
          },
        ];
      });
    });

    listen<TaskStepPayload>(`${eventPrefix}:task_step`, (event) => {
      const step = event.payload;
      updateScopedMessages(scope, (prev) => {
        const last = prev[prev.length - 1];
        if (last?.role === "assistant") {
          const existing = last.taskSteps ?? [];
          // Update existing step if same index, otherwise append
          const idx = existing.findIndex((s) => s.index === step.index);
          const updated = idx >= 0
            ? existing.map((s, i) => (i === idx ? step : s))
            : [...existing, step];
          return [...prev.slice(0, -1), { ...last, taskSteps: updated }];
        }
        return [
          ...prev,
          {
            id: crypto.randomUUID(),
            role: "assistant" as const,
            content: "",
            timestamp: Date.now(),
            taskSteps: [step],
          },
        ];
      });
    });
  };

  registerStreamListeners("agent", "assistant");
  registerStreamListeners("prompt_lab", "prompt_lab");

  listen<InteractionDecision>("interaction_decision:created", (event) => {
    setInteractionDecisions((prev) => {
      const filtered = prev.filter((decision) => decision.id !== event.payload.id);
      return [event.payload, ...filtered];
    });
    void loadInteractionDecisions();
  });

  listen<RuntimeStatusPayload>("runtime:status", (event) => {
    const payload = event.payload;
    if (!payload || typeof payload !== "object") return;
    setRuntimeStatus(payload);
    setRuntimeDiagnostics({
      summary: payload.diagnostics.summary,
      recent: payload.diagnostics.recent,
    });
    if (payload.health && typeof payload.health === "object") {
      const health = payload.health as Record<string, unknown>;
      setHealthInfo((prev) => ({
        ...(prev ?? {}),
        status: typeof health.status === "string" ? health.status : prev?.status,
        event_count:
          typeof health.event_count === "number" ? health.event_count : prev?.event_count,
        health_snapshot: health,
        services: Array.isArray(health.services) ? health.services : prev?.services,
      }));
    }
  });

  listen<AgentStageEvent>("agent:stage", (event) => {
    const stage = event.payload;
    setLatestAgentStage(stage);

    if (stage.step === "colab_dispatch_fallback_local" || stage.step === "colab_dispatch_blocked") {
      setColabDispatchWarning(formatColabDispatchWarning(stage));
      void loadColabStatus();
      return;
    }

    if (stage.step === "colab_dispatch_ready") {
      setColabDispatchWarning(null);
      void loadColabStatus();
    }
  });

  listen("tray:toggle-voice", () => toggleVoice());
  listen("tray:open-settings", () => setShowSettings(true));

  // Image generation events
  listen<{ value: number; max: number; percent: number }>("image:progress", (event) => {
    setImageGenProgress(event.payload.percent);
  });
  listen<{ node: string }>("image:stage", (event) => {
    setImageGenStage(`node ${event.payload.node}`);
  });
  listen("image:done", () => {
    setImageGenProgress(null);
    setImageGenStage(null);
    setVramBlackoutInfo(null);
  });
  listen("image:error", () => {
    setImageGenProgress(null);
    setImageGenStage(null);
    setVramBlackoutInfo(null);
  });

  // ── Canonical Workflow Telemetry Bridge ──────────────────────────────────
  // Consumes structured WorkflowTelemetry events from the backend and
  // forwards them to the workflowSession store. This is the primary
  // frontend/backend synchronization channel for workflow state.
  listen<any>("workflow:telemetry", (event) => {
    try {
      // Dynamic import to avoid circular dependency
      import("./workflowSession").then(({ handleTelemetryEvent }) => {
        handleTelemetryEvent(event.payload);
      });
    } catch (e) {
      console.warn("[WorkflowTelemetry] Failed to process telemetry event:", e);
    }
  });

  listen("gui_cognition:event", (event) => {
    try {
      handleGuiCognitionEvent(event.payload as any);
      // Turn is making progress — re-arm the input watchdog so a long, actively
      // streaming GUI Cognition turn never trips the idle auto-clear.
      pokeAssistantThinkingWatchdog();
      // Task 10.2 (Requirement 16.2/16.3): safety net so the thinking indicator
      // NEVER sticks. The `agent:done` companion already clears it on the
      // batch/end path, but when `gui_cog_stream_ux` streams envelopes DURING
      // the turn we also clear on any definitively terminal lifecycle in case a
      // terminal envelope is observed before (or without) `agent:done`. Paused
      // states (awaiting_approval) intentionally keep the indicator running.
      const lifecycle = activeGuiCognitionSession()?.lifecycle;
      if (
        lifecycle === "completed" ||
        lifecycle === "failed" ||
        lifecycle === "blocked" ||
        lifecycle === "cancelled"
      ) {
        setScopedThinking("assistant", false);
      }
    } catch (e) {
      console.warn("[GuiCognition] Failed to process event:", e);
    }
  });

  // n8n workflow completion — inject result into active chat as system message.
  // This bridges async n8n callbacks into the conversational UI so users see
  // workflow results without checking the admin Dashboard.
  const logN8nFrontendDebug = (...args: unknown[]) => {
    if (import.meta.env.DEV) {
      console.debug(...args);
    }
  };

  const truncateChatText = (value: unknown, maxLength: number): string => {
    if (typeof value !== "string") return "";
    const compact = value.replace(/\s+/g, " ").trim();
    if (compact.length <= maxLength) return compact;
    return `${compact.slice(0, Math.max(0, maxLength - 3)).trimEnd()}...`;
  };

  const formatN8nMessageLine = (message: unknown, index: number): string | null => {
    if (!message || typeof message !== "object") return null;
    const record = message as Record<string, unknown>;
    const subject = truncateChatText(record.subject, 90);
    const preview = truncateChatText(record.preview, 120);
    const from = truncateChatText(record.from, 60);
    const ref = truncateChatText(record.message_ref ?? record.id, 24);
    const title = subject || preview || (ref ? `Message ${ref}` : `Message ${index + 1}`);
    const parts = [`${index + 1}. ${title}`];
    if (from) parts.push(`From: ${from}`);
    if (ref) parts.push(`Ref: ${ref}`);
    return parts.join(" | ");
  };

  const formatN8nEvidenceForChat = (evidence: unknown): string => {
    if (!evidence || typeof evidence !== "object") {
      return truncateChatText(evidence, 300) || "Workflow completed.";
    }

    const record = evidence as Record<string, unknown>;
    const lines: string[] = [];
    const result = truncateChatText(record.result ?? record.message, 220);
    if (result) lines.push(result);

    if (typeof record.message_count === "number") {
      lines.push(`Messages found: ${record.message_count}`);
    }

    if (Array.isArray(record.messages) && record.messages.length > 0) {
      const messageLines = record.messages
        .slice(0, 5)
        .map((message, index) => formatN8nMessageLine(message, index))
        .filter((line): line is string => Boolean(line));

      if (messageLines.length > 0) {
        lines.push("Latest messages:");
        lines.push(...messageLines.map((line) => `- ${line}`));
      }

      if (record.messages.length > messageLines.length) {
        lines.push(`...and ${record.messages.length - messageLines.length} more.`);
      }
    } else if (Array.isArray(record.message_refs) && record.message_refs.length > 0) {
      const refs = record.message_refs
        .slice(0, 5)
        .map((ref) => truncateChatText(ref, 24))
        .filter(Boolean);
      if (refs.length > 0) {
        lines.push(`Message refs: ${refs.join(", ")}`);
      }
    }

    if (lines.length > 0) return lines.join("\n");

    const fallback = truncateChatText(JSON.stringify(record), 300);
    return fallback || "Workflow completed.";
  };

  listen<any>("n8n:chat_result", (event) => {
    try {
      const payload = event.payload;
      logN8nFrontendDebug("[n8n:chat_result] HOP-5: Event received in frontend:", JSON.stringify(payload).slice(0, 300));
      const success = payload.success;
      const name = payload.display_name || payload.workflow_id || "n8n workflow";
      const status = payload.status || "unknown";

      // Build a human-readable summary
      let summary = "";
      if (success) {
        summary = `✓ Workflow "${name}" completed\n\n${formatN8nEvidenceForChat(payload.evidence)}`;
      } else {
        summary = `⚠️ Workflow "${name}" ${status.toLowerCase()}\n\n${formatN8nEvidenceForChat(payload.evidence)}`;
      }

      logN8nFrontendDebug("[n8n:chat_result] HOP-6: Injecting message into chat:", summary);

      // Inject as assistant message into chat
      setAssistantMessages((prev) => {
        logN8nFrontendDebug("[n8n:chat_result] HOP-7: Previous message count:", prev.length, "Adding n8n result");
        return [
        ...prev,
        {
          id: `n8n-${Date.now()}`,
          role: "assistant" as const,
          content: summary,
          timestamp: Date.now(),
        },
        ];
      });
    } catch (e) {
      console.warn("[n8n:chat_result] Failed to process:", e);
    }
  });

  // Tier B VRAM blackout events
  listen<{ free_mb: number; required_mb: number; stage: string }>("image:tier_blackout", (event) => {
    if (event.payload.stage === "restored") {
      setVramBlackoutInfo(null);
    } else {
      setVramBlackoutInfo(event.payload);
    }
  });
  listen<{ level: string; hang_count?: number }>("image:session_degraded", () => {
    setImageSessionDegraded(true);
  });

  // Voice pipeline events
  listen<{ state: "idle" | "listening" | "processing" | "speaking" | "busy" }>("voice:state", (event) => {
    setVoiceState(event.payload.state);
    setVoiceActive(event.payload.state !== "idle");
    if (event.payload.state === "idle") {
      setVoiceLiveTranscript("");
      setVoiceLiveConfidence(null);
      setVoiceLiveStability(null);
      liveVoiceDraftMessageId = null;
    } else if (event.payload.state === "listening") {
      lastPartialSeq = 0;
    }
  });

  listen<{ message?: string; entrypoint?: string; state?: string }>("voice:busy", (event) => {
    const previous = voiceState();
    setVoiceState("busy");
    setVoiceActive(true);
    setVoiceLiveTranscript(event.payload.message ?? "Assistant is busy — current turn is still active.");
    setTimeout(() => {
      if (voiceState() === "busy") {
        setVoiceState(previous === "idle" ? "listening" : previous);
        if ((event.payload.message ?? "").length > 0) {
          setVoiceLiveTranscript("");
        }
      }
    }, 750);
  });

  let lastPartialSeq = 0;
  let lastPartialAt = 0;
  listen<{ text: string; confidence?: number; language?: string; stability?: number; seq?: number }>("voice:partial_transcript", (event) => {
    const seq = event.payload.seq ?? 0;
    const now = Date.now();
    if (seq > 0 && seq <= lastPartialSeq) return;
    if (now - lastPartialAt < 40) return;
    if (seq > 0) lastPartialSeq = seq;
    lastPartialAt = now;
    setVoiceLiveTranscript(event.payload.text);
    setVoiceLiveConfidence(event.payload.confidence ?? null);
    setVoiceLiveLanguage(event.payload.language ?? "auto");
    setVoiceLiveStability(event.payload.stability ?? null);

    // Temporary debug UX: mirror live STT partials into chat immediately.
    // This makes it obvious whether STT is working when LLM/TTS is slow.
    const partialText = (event.payload.text ?? "").trim();
    if (partialText.length > 0) {
      const content = `🎤 (live) ${partialText}`;
      if (!liveVoiceDraftMessageId) {
        const draftMsg: Message = {
          id: crypto.randomUUID(),
          role: "user",
          content,
          timestamp: Date.now(),
        };
        liveVoiceDraftMessageId = draftMsg.id;
        appendScopedMessage("assistant", draftMsg);
      } else {
        updateScopedMessages("assistant", (prev) =>
          prev.map((m) =>
            m.id === liveVoiceDraftMessageId ? { ...m, content, timestamp: Date.now() } : m
          )
        );
      }
    }
  });

  listen<{ text: string; confidence?: number; language?: string; stability?: number }>("voice:transcript", (event) => {
    lastPartialSeq = 0;
    setVoiceLiveTranscript("");
    setVoiceLiveConfidence(event.payload.confidence ?? null);
    setVoiceLiveLanguage(event.payload.language ?? "auto");
    setVoiceLiveStability(event.payload.stability ?? null);
    const finalContent = `🎤 ${event.payload.text}`;
    if (liveVoiceDraftMessageId) {
      updateScopedMessages("assistant", (prev) =>
        prev.map((m) =>
          m.id === liveVoiceDraftMessageId ? { ...m, content: finalContent, timestamp: Date.now() } : m
        )
      );
      liveVoiceDraftMessageId = null;
    } else {
      const userMsg: Message = {
        id: crypto.randomUUID(),
        role: "user",
        content: finalContent,
        timestamp: Date.now(),
      };
      appendScopedMessage("assistant", userMsg);
    }
  });

  listen<{ error: string }>("voice:error", (event) => {
    const err = event.payload.error ?? "";
    if (
      Date.now() < suppressVoiceErrorUntil &&
      /(turn cancelled before transcription|stt stream cancelled)/i.test(err)
    ) {
      return;
    }
    console.error("Voice error:", event.payload.error);
    const errMsg: Message = {
      id: crypto.randomUUID(),
      role: "system",
      content: `⚠️ Voice Error: ${err}`,
      timestamp: Date.now(),
    };
    appendScopedMessage("assistant", errMsg);
  });

  listen<{ reason?: string }>("voice:interruption", (event) => {
    setVoiceInterruptionReason(event.payload.reason ?? "interrupted");
    setVoiceState("listening");
    setTimeout(() => setVoiceInterruptionReason(null), 1200);
  });

  listen<{ error: string }>("voice:playback_failure", (event) => {
    setVoicePlaybackHealth("failed");
    setVoiceLiveTranscript(`Playback issue: ${event.payload.error}`);
  });

  listen("voice:playback_recovered", () => {
    setVoicePlaybackHealth("recovering");
    setVoiceLiveTranscript("Playback recovered");
    setTimeout(() => {
      setVoicePlaybackHealth("ok");
      if (voiceLiveTranscript() === "Playback recovered") {
        setVoiceLiveTranscript("");
      }
    }, 900);
  });

  listen<{ mode?: "half_duplex" | "headphone"; headphone?: boolean }>("voice:io_mode", (event) => {
    const mode = event.payload.mode ?? (event.payload.headphone ? "headphone" : "half_duplex");
    setVoiceIoMode(mode);
  });

  // v2 raw telemetry — all meaningful variants are already forwarded to
  // the canonical voice:state / voice:partial_transcript / voice:transcript
  // events by the backend, but we also listen here for debug and future
  // extensions (e.g. showing BargeIn indicator, Metrics panel, etc.).
  listen<{ kind: string; [key: string]: unknown }>("voice:v2_telemetry", (event) => {
    const { kind } = event.payload;
    if (kind === "barge_in") {
      // Visual feedback: briefly flash back to listening state.
      setVoiceState("listening");
    }
    if (kind === "metrics") {
      const maybe = event.payload.t_first_audio_out_ms;
      if (typeof maybe === "number") {
        setVoiceTtfaMs(maybe);
      }
    }
    // Metrics / Wake / FirstAudioOut — silently consumed for now.
  });

  // Extra backend breadcrumbs for diagnosing STT->LLM->TTS stalls.
  listen<{ stage?: string; turn?: number; [key: string]: unknown }>("voice:debug", (event) => {
    const stage = String(event.payload.stage ?? "unknown");
    if (stage === "stt_final") {
      const chars = Number(event.payload.text_len ?? 0);
      const preview = String(event.payload.text_preview ?? "").trim();
      const dbg: Message = {
        id: crypto.randomUUID(),
        role: "system",
        content:
          preview.length > 0
            ? `🧪 Voice Debug: STT final received (${Number.isFinite(chars) ? chars : 0} chars): "${preview}"`
            : `🧪 Voice Debug: STT final received (${Number.isFinite(chars) ? chars : 0} chars)`,
        timestamp: Date.now(),
      };
      appendScopedMessage("assistant", dbg);
      return;
    }
    if (
      stage === "llm_route_start" ||
      stage === "llm_route_ok" ||
      stage === "llm_stream_request" ||
      stage === "llm_first_token" ||
      stage === "llm_stream_done" ||
      stage === "llm_route_timeout" ||
      stage === "llm_stream_start_timeout" ||
      stage === "llm_stream_token_timeout" ||
      stage === "llm_stream_error"
    ) {
      const dbg: Message = {
        id: crypto.randomUUID(),
        role: "system",
        content: `🧪 Voice Debug: ${stage} (turn ${String(event.payload.turn ?? "?")})`,
        timestamp: Date.now(),
      };
      appendScopedMessage("assistant", dbg);
    }
  });

  // Orchestrator events — track GPU swap state
  listen<{ from_ngl: number; to_ngl: number; emergency: boolean }>(
    "orchestrator:swap_started",
    () => {
      setIsSwapping(true);
    }
  );

  listen<{ new_ngl: number; new_context: number; duration_ms: number }>(
    "orchestrator:swap_completed",
    () => {
      setIsSwapping(false);
    }
  );

  listen<{ level: string }>("orchestrator:degradation_changed", (event) => {
    setDegradationLevel(event.payload.level);
  });

  listen("orchestrator:ready", () => {
    void loadIroncladStatus();
  });

  listen<ColabTierStatus | null>("colab:status", (event) => {
    const payload = event.payload;
    if (payload && typeof payload === "object") {
      setColabStatus(payload as ColabTierStatus);
      if ((payload as ColabTierStatus).ready_for_cloud_task) {
        setColabDispatchWarning(null);
      }
      return;
    }

    void loadColabStatus();
  });

  listen<IroncladStatus | null>("ironclad:status", (event) => {
    const payload = event.payload;
    if (payload && typeof payload === "object") {
      setIroncladStatus(payload as IroncladStatus);
      const typed = payload as IroncladStatus;
      if (typed.reset) {
        setIroncladResetEvent(typed.reset);
      }
      if (typed.forensics?.count !== undefined) {
        setIroncladForensicsTotal(typed.forensics.count);
      }
      return;
    }

    void loadIroncladStatus();
  });

  listen<IroncladResetSnapshot>("ironclad:reset", (event) => {
    const payload = event.payload;
    if (!payload || typeof payload !== "object") return;

    setIroncladResetEvent(payload);
    setIroncladStatus((prev) => {
      if (!prev) return prev;
      return {
        ...prev,
        reset: payload,
      };
    });
  });

  listen<IroncladForensicRecord>("ironclad:forensic", (event) => {
    const payload = event.payload;
    if (!payload || typeof payload !== "object" || !payload.id) return;

    let nextCount = ironcladForensicsTotal();
    setIroncladForensics((prev) => {
      if (prev.some((record) => record.id === payload.id)) {
        return prev;
      }

      const merged = [payload, ...prev]
        .sort((a, b) => b.timestamp_unix_ms - a.timestamp_unix_ms)
        .slice(0, 128);
      nextCount = Math.max(nextCount, merged.length);
      return merged;
    });

    setIroncladForensicsTotal(nextCount);
    setIroncladStatus((prev) => {
      if (!prev) return prev;
      return {
        ...prev,
        forensics: {
          ...prev.forensics,
          count: Math.max(prev.forensics?.count ?? 0, nextCount),
          latest: payload,
        },
      };
    });
  });

  // ─── Intelligence Enhancement Listeners (Throttled) ───────────────────────
  // These events fire at high frequency from the backend. We enqueue them
  // into a batch queue and flush via requestAnimationFrame (or 50ms fallback)
  // to avoid freezing the SolidJS reactive graph.

  listen<ExecutiveTaskStarted>("executive:task_started", (event) => {
    enqueueEvent({ kind: "executive:task_started", payload: event.payload });
  });

  listen<ExecutiveTaskCompleted>("executive:task_completed", (event) => {
    enqueueEvent({ kind: "executive:task_completed", payload: event.payload });
  });

  listen<ExecutivePreemption>("executive:preemption", (event) => {
    enqueueEvent({ kind: "executive:preemption", payload: event.payload });
  });

  listen<GpuLeaseEvent>("executive:gpu_lease", (event) => {
    enqueueEvent({ kind: "executive:gpu_lease", payload: event.payload });
  });

  listen<PolicyGateEvaluation>("policy_gate:evaluation", (event) => {
    enqueueEvent({ kind: "policy_gate:evaluation", payload: event.payload });
  });

  listen<QuarantineApprovalRequest>("quarantine:pending_approval", (event) => {
    enqueueEvent({ kind: "quarantine:pending_approval", payload: event.payload });
  });

  listen<QuarantinePromotionEvent>("quarantine:promoted", (event) => {
    enqueueEvent({ kind: "quarantine:promoted", payload: event.payload });
  });

  listen<QuarantineDisabledEvent>("quarantine:disabled", (event) => {
    enqueueEvent({ kind: "quarantine:disabled", payload: event.payload });
  });

  listen<PlanGenerated>("intelligence:plan", (event) => {
    enqueueEvent({ kind: "intelligence:plan", payload: event.payload });
  });

  listen<PlanStepResult>("intelligence:step_result", (event) => {
    enqueueEvent({ kind: "intelligence:step_result", payload: event.payload });
  });

  listen<GoalVerification>("intelligence:goal_verification", (event) => {
    enqueueEvent({ kind: "intelligence:goal_verification", payload: event.payload });
  });

  listen<UncertaintyEvaluation>("intelligence:uncertainty", (event) => {
    enqueueEvent({ kind: "intelligence:uncertainty", payload: event.payload });
  });

  listen<SelfModelSnapshot>("intelligence:self_model", (event) => {
    enqueueEvent({ kind: "intelligence:self_model", payload: event.payload });
  });

  void loadInteractionDecisions();
}

async function initializeSessionPersistence() {
  const availableSessions = await loadSessions();

  if (!availableSessions) {
    // Backend may still be initializing; keep persisted IDs and retry.
    scheduleSessionHydrationRetry();
    return;
  }

  resetSessionHydrationRetryState();
  markInitialSessionHydrationSettled();

  if (availableSessions.length === 0) {
    setScopedCurrentSession("assistant", null);
    setScopedCurrentSession("prompt_lab", null);
    updateScopedMessages("assistant", () => []);
    updateScopedMessages("prompt_lab", () => []);
    return;
  }

  const preferredSessionId =
    availableSessions.find((session) => (session.turnCount ?? 0) > 0)?.id ??
    availableSessions[0].id;

  const ensureScopedSessionSelection = (scope: StreamScope) => {
    const active = getScopedCurrentSession(scope);
    const activeSession = active
      ? availableSessions.find((session) => session.id === active)
      : null;

    if (activeSession && (activeSession.turnCount ?? 0) > 0) {
      return;
    }

    if (activeSession && activeSession.id === preferredSessionId) {
      return;
    }

    setScopedCurrentSession(scope, preferredSessionId);
  };

  ensureScopedSessionSelection("assistant");
  ensureScopedSessionSelection("prompt_lab");
  await syncEnvironmentSession(currentEnvironment());
}

async function rehydrateSessionsAfterReady() {
  await initializeSessionPersistence();
}

// Initialize listeners on import
initListeners();
// Initialize theme before first render to avoid color/theme flash.
applyTheme(theme());
// Load existing sessions on startup
void initializeSessionPersistence();
// Absolute safety net: no matter what (hung invoke, repeated failures, missing
// Tauri runtime), force-settle the startup "Loading conversations..." spinner by
// the hard deadline so the sidebar can never hang on the loading state. Harmless
// when hydration already settled (markInitialSessionHydrationSettled is a no-op
// once resolved); background retries continue and will still populate sessions.
if (typeof window !== "undefined") {
  window.setTimeout(() => {
    markInitialSessionHydrationSettled();
  }, SESSION_HYDRATION_HARD_DEADLINE_MS);
}
// Load settings on startup
loadSettings();
void loadTelegramConfig();
loadAudioDevices();
void loadColabStatus();
void loadIroncladStatus();
void loadIroncladForensics();
// Prime and refresh system health for UI status indicators.
loadHealth();
void loadRuntimeDiagnostics(128, "info");
setInterval(() => {
  loadHealth();
}, 12000);
setInterval(() => {
  void loadIroncladStatus();
}, 10000);

// Load intelligence data on startup
void loadExecutiveSnapshot();
void loadQuarantinedTools();
void loadSelfModel();
void loadPolicyGateLog();
// Refresh executive snapshot periodically
setInterval(() => {
  void loadExecutiveSnapshot();
}, 5000);

// --- Export store ---
export const appStore = {
  messages,
  sessions,
  isSessionStartupLoading,
  currentSession,
  isThinking,
  showSettings,
  setShowSettings,
  showHitl,
  hitlRequest,
  toolChoiceRequest,
  voiceActive,
  voiceState,
  voiceLiveTranscript,
  voiceLiveConfidence,
  voiceLiveLanguage,
  voiceLiveStability,
  voiceInterruptionReason,
  voicePlaybackHealth,
  voiceIoMode,
  voiceTtfaMs,
  inputText,
  setInputText,
  currentEnvironment,
  setCurrentEnvironment,
  rehydrateSessionsAfterReady,
  settings,
  models,
  audioDevices,
  theme,
  sendMessage,
  sendLabMessage,
  sendImageMessage,
  sendDocumentMessage,
  transcribeUploadedAudio,
  pendingFiles,
  addPendingFile,
  removePendingFile,
  clearPendingFiles,
  cancelTurn,
  cancelGuiCognitionTurn,
  approveAction,
  denyAction,
  interactionDecisions,
  interactionDecisionMetrics,
  loadInteractionDecisions,
  resolveInteractionDecision,
  resumeInteractionDecision,
  executeResolvedInteractionDecision,
  cancelInteractionExecution,
  checkContinuationAfterDecision,
  continueAfterDecisionExecution,
  cancelContinuation,
  cancelInteractionDecision,
  replayInteractionDecisions,
  toggleVoice,
  loadSessions,
  createSession,
  switchSession,
  deleteSession,
  clearAllChatSessions,
  renameSession,
  loadSettings,
  loadAudioDevices,
  saveSettings,
  loadModels,
  applyTheme,
  mcpServers,
  loadMcpServers,
  addMcpServer,
  removeMcpServer,
  toggleMcpServer,
  healthInfo,
  runtimeStatus,
  runtimeDiagnostics,
  loadHealth,
  loadRuntimeDiagnostics,
  assistantStatus,
  scheduledTasks,
  loadScheduledTasks,
  addScheduledTask,
  removeScheduledTask,
  macros,
  loadMacros,
  deleteMacro,
  workflows,
  loadWorkflows,
  deleteWorkflow,
  hardwareInfo,
  loadHardwareInfo,
  knowledgeBase,
  loadKnowledgeBase,
  alerts,
  loadAlerts,
  telegramConfig,
  telegramBotInfo,
  loadTelegramConfig,
  saveTelegramConfig,
  testTelegramConnection,
  startTelegramMcp,
  stopTelegramMcp,
  googleStatus,
  loadGoogleStatus,
  setGoogleAccount,
  connectGoogle,
  disconnectGoogle,
  colabStatus,
  latestAgentStage,
  colabDispatchWarning,
  loadColabStatus,
  connectColab,
  disconnectColab,
  setColabNotebook,
  ironcladStatus,
  ironcladForensics,
  ironcladForensicsTotal,
  ironcladResetEvent,
  loadIroncladStatus,
  loadIroncladForensics,
  requestIroncladSoftReset,
  requestIroncladHardReset,
  getIroncladConfig,
  updateIroncladConfig,
  registerNewTarget,
  deleteTarget,
  updateTarget,
  reconcileMcpRuntime,
  restartMcpServerRuntime,
  submitToolChoice,
  dismissToolChoice,
  isSwapping,
  degradationLevel,
  imageGenProgress,
  imageGenStage,
  vramBlackoutInfo,
  imageSessionDegraded,
  manualToolModes,
  manualToolMode,
  selectedManualToolMode,
  buildManualToolProfile,
  setManualToolMode,

  // Intelligence Enhancement (Phase A-F)
  executiveSnapshot,
  executiveRecentEvents,
  policyGateLog,
  quarantinedTools,
  quarantinePendingApproval,
  latestPlan,
  planStepResults,
  latestGoalVerification,
  selfModelSnapshot,
  intelligenceState,
  latestUncertainty,
  loadExecutiveSnapshot,
  cancelExecutiveTask,
  submitTurnFeedback,
  loadQuarantinedTools,
  approveQuarantinedTool,
  rejectQuarantinedTool,
  loadSelfModel,
  loadPolicyGateLog,
};
