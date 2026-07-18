/**
 * Automation Store — n8n workflows, tasks, reminders, scheduled items.
 *
 * Requirements: 6.1 (Automations Space), 6.2 (workflows), 6.6 (scheduled/reminders)
 */
import { createSignal } from "solid-js";
import { eventBus } from "./eventBus";
import { bridgeInvoke } from "../bridge/invoke";
import {
  n8nStore,
  type N8nRunState,
  type N8nStatusPayload,
  type N8nWorkflow,
} from "./n8n";

// ─── Types ─────────────────────────────────────────────────────────────────────

export type WorkflowStatus = "idle" | "running" | "completed" | "failed" | "paused";

export interface Workflow {
  id: string;
  name: string;
  description: string;
  status: WorkflowStatus;
  lastRunAt: number | null;
  createdAt: number;
  /**
   * Workflow version, required by the run/prepare commands (populated by the
   * bridge from the n8n registry). Optional so pre-bridge/test data is valid.
   */
  version?: string;
}

/**
 * A recurring, prompt-driven scheduled task from the interval scheduler
 * (Req 6.6 "scheduled tasks — cron/interval"). Mirrors the EXISTING
 * `list_scheduled_tasks` shape (kria_core::automation::scheduler). The runtime
 * scheduler owns execution; this Space surfaces + creates/removes them.
 *
 * NOTE: the backend exposes an `enabled` flag but NO enable/disable command, so
 * the UI shows enablement as read-only state — it never renders a toggle that
 * would silently do nothing (Req 10.6 / honest controls). See task 7.4 notes.
 */
export interface ScheduledTask {
  id: string;
  name: string;
  /** Fire interval in seconds. */
  intervalSecs: number;
  /** The prompt KRIA runs each interval. Untrusted → rendered as escaped text. */
  prompt: string;
  enabled: boolean;
}

/** Lifecycle status of a to-do task (backend `task_*` queue). */
export type TaskStatus =
  | "open"
  | "in_progress"
  | "blocked"
  | "waiting"
  | "done"
  | "cancelled";

/** The set of statuses a task can be moved between in the Schedule UI. */
export const TASK_STATUSES: readonly TaskStatus[] = [
  "open",
  "in_progress",
  "blocked",
  "waiting",
  "done",
  "cancelled",
] as const;

/**
 * A to-do task from the unified task queue (Req 6.6). Normalized (camelCase +
 * epoch-ms dates) from the EXISTING `task_*` commands' `Task` shape. A task with
 * a `dueAt` is a "scheduled" to-do; completion is a real status change.
 */
export interface TaskItem {
  id: number;
  title: string;
  notes: string | null;
  status: TaskStatus;
  priorityBucket: string;
  priorityScore: number;
  /** Due time in epoch-ms, or null when undated. */
  dueAt: number | null;
  source: string;
  createdAt: number;
}

/**
 * A durable reminder (Req 6.6). Normalized from the EXISTING `reminder_*`
 * commands' `Reminder` shape. A reminder with a `recurrence` is a recurring
 * "routine" (KRIA has no separate routine engine — recurring reminders are the
 * routine primitive); one without is a one-shot reminder.
 */
export interface Reminder {
  id: number;
  message: string;
  /** Fire time in epoch-ms. */
  fireAt: number;
  fired: boolean;
  /** Recurrence rule (e.g. "daily", "weekly:fri") or null for one-shot. */
  recurrence: string | null;
}

/** Backend-owned morning briefing configuration. */
export interface BriefingSection {
  source: string;
  enabled: boolean;
  query?: string | null;
  max?: number | null;
  account?: string | null;
  window?: string | null;
  include_conflicts?: boolean | null;
  tool?: string | null;
  filter?: string | null;
}

export interface BriefingSchedule {
  auto: boolean;
  time: string;
  delivery: string[];
}

export interface BriefingConfig {
  sections: BriefingSection[];
  schedule: BriefingSchedule;
}

/** A recurring reminder is a "routine" (Req 6.6 grouping). */
export function isRoutine(reminder: Reminder): boolean {
  return !!reminder.recurrence && reminder.recurrence.trim().length > 0;
}

export type AutomationSegment = "run" | "build" | "schedule" | "history";

/** Typed action outcome so callers surface HONEST success/failure (Req 6.5). */
export type AutomationActionResult<T = void> =
  | { ok: true; data: T }
  | { ok: false; message: string };

/**
 * A workflow KRIA suggests for a natural-language request — the ask-KRIA-to-pick
 * result (Req 6.3). Normalized (camelCase) from the existing
 * `suggest_n8n_workflows` command's candidate shape; the UI never invents a
 * pick, it surfaces the runtime's ranked candidates.
 */
export interface SuggestedWorkflow {
  workflowId: string;
  workflowVersion: string;
  displayName: string;
  /** Plain-language reason KRIA picked this candidate. */
  reason: string;
  confidence: number;
  confidenceLabel: string;
  riskTier: string;
  requiresConfirmation: boolean;
  missingInputs: string[];
  /** The input payload KRIA proposes (structured; shown as escaped text). */
  suggestedInputPayload?: unknown;
}

/** One field in a prepared-run input schema (Req 6.3 preview-before-confirm). */
export interface PreparedInputField {
  name: string;
  type?: string;
  required?: boolean;
  description?: string;
}

/**
 * The inputs KRIA prepared for a run, shown for review BEFORE the user confirms
 * (Req 6.3). Normalized from the existing `prepare_n8n_workflow_input` command.
 */
export interface PreparedRunInput {
  workflowId: string;
  workflowVersion: string;
  displayName: string;
  prompt: string;
  /** The structured payload KRIA assembled (rendered as escaped JSON text). */
  payload: unknown;
  fields: PreparedInputField[];
  missingInputs: string[];
  validationIssues: string[];
  /** Plain-language explanation of how the inputs were derived, if provided. */
  explanation?: string;
  /** Whether the payload was field-mapped by KRIA (passed through to run). */
  inputMapped: boolean;
}

/** Lifecycle phase of a workflow run, derived from run events (Req 6.5). */
export type RunPhase =
  | "triggering"
  | "running"
  | "waiting"
  | "completed"
  | "failed"
  | "cancelled";

/** Live progress for a single workflow run (Req 6.5). Fed by run events. */
export interface RunProgress {
  workflowId: string;
  phase: RunPhase;
  completedSteps: number;
  totalSteps: number | null;
  /** Short plain-language status line. */
  message?: string;
  updatedAt: number;
}

/**
 * A piece of run evidence/output (Req 6.5). `detail` is UNTRUSTED
 * markdown/HTML and MUST be sanitized before display (the EvidenceViewer does
 * this); `href` links to a source/artifact.
 */
export interface RunEvidenceItem {
  label: string;
  detail?: string;
  href?: string;
}

// ─── 2D node builder (Build segment, Req 6.3 / 6.4) ──────────────────────────
//
// A lightweight, in-house 2D authoring model (design.md §6.3 — NodeCanvas(2D) +
// NodePalette; NOT the 3D graph engine, which is Memory-only). The builder is
// pure client-side draft state; drafting/testing/approving DISPATCH through the
// EXISTING n8n authoring commands (create_or_update_n8n_workflow_draft,
// dry_run_n8n_workflow_validation, approve_n8n_workflow_draft) via the bridge.
// The UI authors a draft; the n8n substrate owns persistence + execution.

/** A curated node type the palette offers (no backend node catalog exists). */
export interface NodePaletteItem {
  /** Stable kind id used by the builder. */
  kind: string;
  /** Human label shown in the palette + on the canvas node. */
  label: string;
  /** The n8n node `type` this maps to on serialization. */
  n8nType: string;
  /** Lucide icon id (kebab-case) for the palette + node glyph. */
  icon: string;
  /** Short description shown in the palette + node inspector. */
  description: string;
}

/**
 * Curated node palette (Req 6.4). Sourced in-house because no backend node
 * catalog command exists; each maps to a real `n8n-nodes-base.*` type so a
 * saved draft is a valid n8n workflow.
 */
export const NODE_PALETTE: readonly NodePaletteItem[] = [
  { kind: "manual-trigger", label: "Manual Trigger", n8nType: "n8n-nodes-base.manualTrigger", icon: "play", description: "Start the workflow on demand." },
  { kind: "webhook", label: "Webhook", n8nType: "n8n-nodes-base.webhook", icon: "globe", description: "Start when an HTTP request arrives." },
  { kind: "chat-trigger", label: "Chat Trigger", n8nType: "@n8n/n8n-nodes-langchain.chatTrigger", icon: "message-circle", description: "Start from an n8n public chat endpoint." },
  { kind: "http-request", label: "HTTP Request", n8nType: "n8n-nodes-base.httpRequest", icon: "network", description: "Call an external HTTP API." },
  { kind: "code", label: "Code", n8nType: "n8n-nodes-base.code", icon: "terminal", description: "Run a small JavaScript step." },
  { kind: "set", label: "Edit Fields", n8nType: "n8n-nodes-base.set", icon: "sliders-horizontal", description: "Set or transform fields." },
  { kind: "if", label: "If", n8nType: "n8n-nodes-base.if", icon: "git-branch", description: "Branch on a condition." },
  { kind: "email", label: "Send Email", n8nType: "n8n-nodes-base.emailSend", icon: "send", description: "Send an email notification." },
] as const;

/** A node placed on the 2D canvas (Req 6.4). Params are simple string values
 *  edited in the node Inspector; serialized into the n8n node `parameters`. */
export interface BuilderNode {
  id: string;
  kind: string;
  /** Editable node name (unique-ised on serialization for n8n connections). */
  name: string;
  /** Canvas position in px. */
  x: number;
  y: number;
  params: Record<string, string>;
}

/** A directed connection between two builder nodes (source → target). */
export interface BuilderEdge {
  id: string;
  source: string;
  target: string;
}

/**
 * Authoring lifecycle phase (Req 6.3). `editing` while the draft is being
 * built, `saved` once persisted to the n8n draft, `tested` after a dry-run,
 * `approved` once approved/published. A structural edit resets to `editing`.
 */
export type DraftLifecycle = "editing" | "saved" | "tested" | "approved";

/** Result of a draft dry-run/test (Req 6.3). */
export interface DraftTestResult {
  ok: boolean;
  /** Plain-language summary of the dry-run outcome. */
  message: string;
  /** Validation issues surfaced by the backend, if any. */
  issues: string[];
  /** True when the test ran client-side only (backend path unavailable). */
  clientSideOnly: boolean;
}

// ─── Signals ───────────────────────────────────────────────────────────────────

const [workflows, setWorkflows] = createSignal<Workflow[]>([]);
/** Interval scheduler tasks (Req 6.6 — cron/interval scheduled tasks). */
const [scheduledTasks, setScheduledTasks] = createSignal<ScheduledTask[]>([]);
/** To-do task queue (Req 6.6). */
const [tasks, setTasks] = createSignal<TaskItem[]>([]);
/** Durable reminders + recurring routines (Req 6.6). */
const [reminders, setReminders] = createSignal<Reminder[]>([]);
/** Backend-owned briefing config surfaced in Schedule. */
const [briefingConfig, setBriefingConfig] = createSignal<BriefingConfig | null>(null);
const [briefingLoading, setBriefingLoading] = createSignal(false);
const [briefingSaving, setBriefingSaving] = createSignal(false);
const [briefingError, setBriefingError] = createSignal<string | null>(null);
const [briefingStatus, setBriefingStatus] = createSignal<string | null>(null);
const [activeSegment, setActiveSegment] = createSignal<AutomationSegment>("run");
const [runningWorkflowIds, setRunningWorkflowIds] = createSignal<Set<string>>(new Set());
/**
 * Whether the automation data (workflows/tasks/reminders) is still being
 * fetched from the backend. Drives honest loading states in the Automations
 * Space so a not-yet-loaded list is never shown as an empty one (Req 6.1).
 */
const [loading, setLoading] = createSignal<boolean>(false);
/** Query string for the Run-segment workflow search (Req 6.2 top-level). */
const [searchQuery, setSearchQuery] = createSignal<string>("");

// ── ask-KRIA-to-pick (Req 6.3) ──────────────────────────────────────────────
const [suggestedWorkflows, setSuggestedWorkflows] = createSignal<SuggestedWorkflow[]>([]);
/** KRIA's plain-language message about the suggestion (e.g. "ambiguous"). */
const [suggestionMessage, setSuggestionMessage] = createSignal<string>("");
/** Whether a pick request is in flight (honest loading, Req 6.5). */
const [suggesting, setSuggesting] = createSignal<boolean>(false);
/** Honest failure text from the last pick request, if any. */
const [suggestError, setSuggestError] = createSignal<string | null>(null);
/** The prompt behind the current suggestion (for prepare/run context). */
const [lastPickPrompt, setLastPickPrompt] = createSignal<string>("");

// ── prepared-input preview (Req 6.3) ────────────────────────────────────────
const [preparedInput, setPreparedInput] = createSignal<PreparedRunInput | null>(null);
const [preparing, setPreparing] = createSignal<boolean>(false);

// ── run progress + evidence, keyed by workflow id (Req 6.5) ─────────────────
const [runProgress, setRunProgress] = createSignal<Record<string, RunProgress>>({});
const [runEvidence, setRunEvidence] = createSignal<Record<string, RunEvidenceItem[]>>({});

// ── 2D node builder draft (Build segment, Req 6.3 / 6.4) ─────────────────────
let builderIdCounter = 0;
/** Deterministic-enough unique id for builder nodes/edges/drafts. */
function newBuilderId(prefix: string): string {
  builderIdCounter += 1;
  return `${prefix}-${Date.now().toString(36)}-${builderIdCounter}`;
}

const [builderNodes, setBuilderNodes] = createSignal<BuilderNode[]>([]);
const [builderEdges, setBuilderEdges] = createSignal<BuilderEdge[]>([]);
/** The id of the node currently selected/open in the node Inspector. */
const [selectedNodeId, setSelectedNodeId] = createSignal<string | null>(null);
/** The draft's display name (Req 6.3). */
const [draftName, setDraftName] = createSignal<string>("Untitled workflow");
/** Stable draft workflow id (also the n8n draft id once saved). */
const [draftId, setDraftId] = createSignal<string>(newBuilderId("draft"));
const [draftLifecycle, setDraftLifecycle] = createSignal<DraftLifecycle>("editing");
/** Whether the draft has been persisted to the n8n substrate (Req 6.3 honest
 *  state — surfaces "not yet persisted" until a save succeeds). */
const [draftPersisted, setDraftPersisted] = createSignal<boolean>(false);
/** Honest status line for the builder (save/test/approve outcome). */
const [builderStatus, setBuilderStatus] = createSignal<string>("");
const [builderBusy, setBuilderBusy] = createSignal<boolean>(false);
const [draftTestResult, setDraftTestResult] = createSignal<DraftTestResult | null>(null);

// ─── Actions ───────────────────────────────────────────────────────────────────

function markWorkflowStarted(workflowId: string): void {
  setWorkflows((prev) =>
    prev.map((w) => (w.id === workflowId ? { ...w, status: "running" as const } : w))
  );
  setRunningWorkflowIds((prev) => new Set([...prev, workflowId]));
  eventBus.emit("automation:workflow-started", { workflowId });
}

function markWorkflowCompleted(workflowId: string, success: boolean): void {
  setWorkflows((prev) =>
    prev.map((w) =>
      w.id === workflowId
        ? { ...w, status: (success ? "completed" : "failed") as WorkflowStatus, lastRunAt: Date.now() }
        : w
    )
  );
  setRunningWorkflowIds((prev) => {
    const next = new Set(prev);
    next.delete(workflowId);
    return next;
  });
  eventBus.emit("automation:workflow-completed", { workflowId, success });
}

let stopN8nStatusSubscription: (() => void) | null = null;

function latestRun(runs: N8nRunState[], workflowId: string): N8nRunState | undefined {
  return runs
    .filter((run) => run.workflow_id === workflowId)
    .sort((a, b) => (b.triggered_at_ms ?? 0) - (a.triggered_at_ms ?? 0))[0];
}

function projectedWorkflowStatus(run: N8nRunState | undefined): WorkflowStatus {
  if (!run) return "idle";
  const status = run.status.toLowerCase();
  if (!run.terminal) return status.includes("wait") ? "paused" : "running";
  return status.includes("complete") || status.includes("success") ? "completed" : "failed";
}

function projectN8nWorkflow(workflow: N8nWorkflow, runs: N8nRunState[]): Workflow {
  const run = latestRun(runs, workflow.workflow_id);
  return {
    id: workflow.workflow_id,
    name: workflow.display_name,
    description: workflow.description ?? "",
    status: projectedWorkflowStatus(run),
    lastRunAt: run?.triggered_at_ms ?? null,
    createdAt: 0,
    version: workflow.workflow_version,
  };
}

function evidenceLabel(item: unknown): string {
  if (!item || typeof item !== "object") return String(item ?? "Workflow evidence");
  const row = item as Record<string, unknown>;
  return String(row.summary ?? row.result ?? row.message ?? row.phase ?? "Workflow evidence");
}

function syncN8nReadModel(payload: N8nStatusPayload | null): void {
  if (!payload) {
    setWorkflows([]);
    setRunningWorkflowIds(new Set<string>());
    setRunProgress({});
    setRunEvidence({});
    return;
  }
  const runs = payload.runs ?? [];
  setWorkflows(payload.configured_workflows.map((workflow) => projectN8nWorkflow(workflow, runs)));
  setRunningWorkflowIds(new Set(runs.filter((run) => !run.terminal).map((run) => run.workflow_id)));

  const progress: Record<string, RunProgress> = {};
  const evidence: Record<string, RunEvidenceItem[]> = {};
  for (const workflow of payload.configured_workflows) {
    const run = latestRun(runs, workflow.workflow_id);
    if (!run) continue;
    const projectedStatus = projectedWorkflowStatus(run);
    progress[workflow.workflow_id] = {
      workflowId: workflow.workflow_id,
      phase: projectedStatus === "paused"
        ? "waiting"
        : projectedStatus === "idle"
          ? "triggering"
          : projectedStatus,
      completedSteps: Number(run.last_sequence_number ?? 0),
      totalSteps: null,
      message: run.status,
      updatedAt: run.triggered_at_ms ?? Date.now(),
    };
    evidence[workflow.workflow_id] = (run.evidence_log ?? []).map((item, index) => ({
      label: evidenceLabel(item),
      detail: typeof item === "string" ? item : JSON.stringify(item, null, 2),
      timestamp: Number((item as Record<string, unknown> | null)?.occurred_at_ms ?? run.triggered_at_ms ?? Date.now()),
      kind: "log",
      id: `${run.correlation_id}:${index}`,
    }));
  }
  setRunProgress(progress);
  setRunEvidence(evidence);
}

async function initializeAutomationStore(): Promise<void> {
  if (!stopN8nStatusSubscription) {
    stopN8nStatusSubscription = n8nStore.subscribeStatus(syncN8nReadModel);
  }
  setLoading(true);
  try {
    await Promise.all([n8nStore.initialize(), loadSchedule()]);
    syncN8nReadModel(n8nStore.status());
  } finally {
    setLoading(false);
  }
}

function disposeAutomationStore(): void {
  stopN8nStatusSubscription?.();
  stopN8nStatusSubscription = null;
  n8nStore.dispose();
}

// ─── Schedule dispatch (Req 6.6) ─────────────────────────────────────────────
//
// ARCHITECTURE (KRIA runtime authority): every create/enable/complete/snooze/
// delete below is DISPATCH-ONLY through an EXISTING backend command via the
// bridge. The runtime scheduler / task store / reminder store own execution and
// persistence; the UI asks and reflects the HONEST result (Req 6.5 / 20.4). No
// orchestration, no prompt→tool shortcut. Untrusted text (task titles, reminder
// messages, scheduled-task prompts) is rendered as escaped text by the views.

// ── Normalizers (snake_case backend → camelCase store) ──────────────────────

interface RawScheduledTask {
  id?: string;
  name?: string;
  interval_secs?: number;
  prompt?: string;
  enabled?: boolean;
}

function normalizeScheduledTask(t: RawScheduledTask): ScheduledTask {
  return {
    id: String(t.id ?? ""),
    name: String(t.name ?? "Scheduled task"),
    intervalSecs: Number(t.interval_secs ?? 0),
    prompt: String(t.prompt ?? ""),
    enabled: t.enabled !== false,
  };
}

interface RawTask {
  id?: number;
  title?: string;
  notes?: string | null;
  status?: string;
  priority_bucket?: string;
  priority_score?: number;
  due_at?: string | null;
  source?: string;
  created_at?: string;
}

function toEpochMs(s: string | null | undefined): number | null {
  if (!s) return null;
  const ms = Date.parse(s);
  return Number.isNaN(ms) ? null : ms;
}

function normalizeTask(t: RawTask): TaskItem {
  const status = String(t.status ?? "open") as TaskStatus;
  return {
    id: Number(t.id ?? 0),
    title: String(t.title ?? ""),
    notes: t.notes ?? null,
    status: TASK_STATUSES.includes(status) ? status : "open",
    priorityBucket: String(t.priority_bucket ?? "normal"),
    priorityScore: Number(t.priority_score ?? 0),
    dueAt: toEpochMs(t.due_at ?? null),
    source: String(t.source ?? "manual"),
    createdAt: toEpochMs(t.created_at) ?? Date.now(),
  };
}

interface RawReminder {
  id?: number;
  message?: string;
  fire_at?: string;
  fired?: boolean;
  recurrence?: string | null;
}

function normalizeReminder(r: RawReminder): Reminder {
  return {
    id: Number(r.id ?? 0),
    message: String(r.message ?? ""),
    fireAt: toEpochMs(r.fire_at) ?? Date.now(),
    fired: Boolean(r.fired),
    recurrence: r.recurrence ?? null,
  };
}

function isBriefingConfig(value: unknown): value is BriefingConfig {
  if (!value || typeof value !== "object") return false;
  const config = value as Partial<BriefingConfig>;
  return Array.isArray(config.sections)
    && config.sections.every((section) => (
      !!section
      && typeof section.source === "string"
      && typeof section.enabled === "boolean"
    ))
    && !!config.schedule
    && typeof config.schedule.auto === "boolean"
    && typeof config.schedule.time === "string"
    && Array.isArray(config.schedule.delivery)
    && config.schedule.delivery.every((channel) => typeof channel === "string");
}

/** Load backend-owned briefing configuration via `get_briefing_config`. */
async function loadBriefingConfig(): Promise<AutomationActionResult<BriefingConfig>> {
  setBriefingLoading(true);
  setBriefingError(null);
  try {
    const res = await bridgeInvoke<BriefingConfig>("get_briefing_config");
    if (!res.ok) {
      const message = failText(res.message, "get_briefing_config");
      setBriefingError(message);
      return { ok: false, message };
    }
    if (!isBriefingConfig(res.data)) {
      const message = "Briefing configuration returned an invalid shape. Check the desktop runtime and try again.";
      setBriefingError(message);
      return { ok: false, message };
    }
    setBriefingConfig(res.data);
    return { ok: true, data: res.data };
  } finally {
    setBriefingLoading(false);
  }
}

/** Persist the complete briefing configuration via `set_briefing_config`. */
async function saveBriefingConfig(config: BriefingConfig): Promise<AutomationActionResult<BriefingConfig>> {
  setBriefingSaving(true);
  setBriefingError(null);
  setBriefingStatus(null);
  try {
    const res = await bridgeInvoke<BriefingConfig>("set_briefing_config", { config });
    if (!res.ok) {
      const message = failText(res.message, "set_briefing_config");
      setBriefingError(message);
      return { ok: false, message };
    }
    if (!isBriefingConfig(res.data)) {
      const message = "Saved briefing configuration was not returned correctly. Reload it before making more changes.";
      setBriefingError(message);
      return { ok: false, message };
    }
    setBriefingConfig(res.data);
    setBriefingStatus("Briefing saved.");
    return { ok: true, data: res.data };
  } finally {
    setBriefingSaving(false);
  }
}

// ── Loads (honest loading state; graceful on unavailable service) ───────────

/** Load interval scheduler tasks via the EXISTING `list_scheduled_tasks`. */
async function loadScheduledTasks(): Promise<AutomationActionResult<ScheduledTask[]>> {
  const res = await bridgeInvoke<RawScheduledTask[]>("list_scheduled_tasks");
  if (!res.ok) {
    return { ok: false, message: failText(res.message, "list_scheduled_tasks") };
  }
  const list = (res.data ?? []).map(normalizeScheduledTask);
  setScheduledTasks(list);
  return { ok: true, data: list };
}

/** Load the to-do task queue via the EXISTING `task_list`. */
async function loadTasks(): Promise<AutomationActionResult<TaskItem[]>> {
  const res = await bridgeInvoke<RawTask[]>("task_list", {
    status: null,
    bucket: null,
    activeOnly: null,
  });
  if (!res.ok) return { ok: false, message: failText(res.message, "task_list") };
  const list = (res.data ?? []).map(normalizeTask);
  setTasks(list);
  return { ok: true, data: list };
}

/** Load durable reminders via the EXISTING `reminder_list`. */
async function loadReminders(includeFired = true): Promise<AutomationActionResult<Reminder[]>> {
  const res = await bridgeInvoke<RawReminder[]>("reminder_list", { includeFired });
  if (!res.ok) return { ok: false, message: failText(res.message, "reminder_list") };
  const list = (res.data ?? []).map(normalizeReminder);
  setReminders(list);
  return { ok: true, data: list };
}

/**
 * Load all Schedule sources together, including backend-owned briefing config,
 * driving honest loading state. Each source degrades independently.
 */
async function loadSchedule(): Promise<void> {
  setLoading(true);
  try {
    await Promise.all([
      loadScheduledTasks(),
      loadTasks(),
      loadReminders(),
      loadBriefingConfig(),
    ]);
  } finally {
    setLoading(false);
  }
}

// ── Interval scheduler actions ──────────────────────────────────────────────

/** Create an interval scheduled task via the EXISTING `add_scheduled_task`. */
async function addScheduledTask(args: {
  name: string;
  intervalSecs: number;
  prompt: string;
}): Promise<AutomationActionResult<void>> {
  const name = args.name.trim();
  const prompt = args.prompt.trim();
  if (!name) return { ok: false, message: "Give the scheduled task a name." };
  if (!prompt) return { ok: false, message: "Describe what KRIA should do each run." };
  if (!(args.intervalSecs > 0)) return { ok: false, message: "Interval must be greater than zero." };

  const res = await bridgeInvoke<unknown>("add_scheduled_task", {
    name,
    intervalSecs: Math.round(args.intervalSecs),
    prompt,
  });
  if (!res.ok) return { ok: false, message: failText(res.message, "add_scheduled_task") };
  await loadScheduledTasks();
  eventBus.emit("automation:task-updated", { taskId: name });
  return { ok: true, data: undefined };
}

/** Delete an interval scheduled task via the EXISTING `remove_scheduled_task`.
 *  Deliberate confirm is enforced by the calling view (Req 8.4-style). */
async function removeScheduledTask(taskId: string): Promise<AutomationActionResult<void>> {
  // Optimistic removal; reload reconciles with the runtime.
  const prev = scheduledTasks();
  setScheduledTasks((list) => list.filter((t) => t.id !== taskId));
  const res = await bridgeInvoke<unknown>("remove_scheduled_task", { taskId });
  if (!res.ok) {
    setScheduledTasks(prev);
    return { ok: false, message: failText(res.message, "remove_scheduled_task") };
  }
  await loadScheduledTasks();
  return { ok: true, data: undefined };
}

// ── To-do task actions ──────────────────────────────────────────────────────

/** Add a to-do task via the EXISTING `task_add`. */
async function addTask(args: {
  title: string;
  notes?: string;
  dueAt?: string;
}): Promise<AutomationActionResult<void>> {
  const title = args.title.trim();
  if (!title) return { ok: false, message: "Give the task a title." };
  const res = await bridgeInvoke<unknown>("task_add", {
    title,
    notes: args.notes?.trim() || null,
    dueAt: args.dueAt ?? null,
    source: "manual",
  });
  if (!res.ok) return { ok: false, message: failText(res.message, "task_add") };
  await loadTasks();
  return { ok: true, data: undefined };
}

/**
 * Move a task to a new status via the EXISTING `task_update_status`. Optimistic;
 * reload reconciles. Completion (done) + reopen (open) drive the task's toggle.
 */
async function setTaskStatus(id: number, status: TaskStatus): Promise<AutomationActionResult<void>> {
  const prev = tasks();
  setTasks((list) => list.map((t) => (t.id === id ? { ...t, status } : t)));
  const res = await bridgeInvoke<unknown>("task_update_status", { id, status });
  if (!res.ok) {
    setTasks(prev);
    return { ok: false, message: failText(res.message, "task_update_status") };
  }
  eventBus.emit("automation:task-updated", { taskId: String(id) });
  await loadTasks();
  return { ok: true, data: undefined };
}

/** Toggle a task done ↔ open (the completion toggle, Req 6.6). Real status
 *  change via `task_update_status` — never a no-op control. */
async function toggleTaskDone(id: number, done: boolean): Promise<AutomationActionResult<void>> {
  return setTaskStatus(id, done ? "done" : "open");
}

/** Edit a task's title/notes/due via the EXISTING `task_edit`. */
async function editTask(
  id: number,
  patch: { title?: string; notes?: string; dueAt?: string; clearDue?: boolean },
): Promise<AutomationActionResult<void>> {
  const res = await bridgeInvoke<unknown>("task_edit", {
    id,
    title: patch.title ?? null,
    notes: patch.notes ?? null,
    dueAt: patch.dueAt ?? null,
    clearDue: patch.clearDue ?? null,
  });
  if (!res.ok) return { ok: false, message: failText(res.message, "task_edit") };
  await loadTasks();
  return { ok: true, data: undefined };
}

/** Delete a task via the EXISTING `task_delete`. Deliberate confirm enforced by
 *  the view (Req 8.4-style irreversible confirm). Optimistic; reload reconciles. */
async function deleteTask(id: number): Promise<AutomationActionResult<void>> {
  const prev = tasks();
  setTasks((list) => list.filter((t) => t.id !== id));
  const res = await bridgeInvoke<unknown>("task_delete", { id });
  if (!res.ok) {
    setTasks(prev);
    return { ok: false, message: failText(res.message, "task_delete") };
  }
  await loadTasks();
  return { ok: true, data: undefined };
}

// ── Reminder actions ────────────────────────────────────────────────────────

/** Set a reminder via the EXISTING `reminder_set`. A `recurrence` makes it a
 *  recurring routine (Req 6.6). */
async function setReminder(args: {
  message: string;
  fireInMinutes?: number;
  when?: string;
  recurrence?: string;
}): Promise<AutomationActionResult<void>> {
  const message = args.message.trim();
  if (!message) return { ok: false, message: "Describe what to be reminded about." };
  const res = await bridgeInvoke<unknown>("reminder_set", {
    message,
    when: args.when ?? null,
    fireInMinutes: args.fireInMinutes ?? null,
    fireAt: null,
    recurrence: args.recurrence?.trim() || null,
  });
  if (!res.ok) return { ok: false, message: failText(res.message, "reminder_set") };
  await loadReminders();
  return { ok: true, data: undefined };
}

/** Snooze a reminder via the EXISTING `reminder_snooze`. */
async function snoozeReminder(id: number, minutes = 10): Promise<AutomationActionResult<void>> {
  const res = await bridgeInvoke<unknown>("reminder_snooze", { id, minutes });
  if (!res.ok) return { ok: false, message: failText(res.message, "reminder_snooze") };
  await loadReminders();
  return { ok: true, data: undefined };
}

/** Cancel/dismiss a reminder via the EXISTING `reminder_cancel`. Deliberate
 *  confirm enforced by the view. Optimistic; reload reconciles. */
async function cancelReminder(id: number): Promise<AutomationActionResult<void>> {
  const prev = reminders();
  setReminders((list) => list.filter((r) => r.id !== id));
  const res = await bridgeInvoke<unknown>("reminder_cancel", { id });
  if (!res.ok) {
    setReminders(prev);
    return { ok: false, message: failText(res.message, "reminder_cancel") };
  }
  await loadReminders();
  return { ok: true, data: undefined };
}

// ─── Run dispatch (Req 6.3 / 6.5) ───────────────────────────────────────────
//
// ARCHITECTURE (KRIA runtime authority): pick / prepare / run / cancel are
// DISPATCH-ONLY. Each routes through an EXISTING backend command via the bridge
// (n8n is the execution substrate; KRIA orchestrates it). There is no
// orchestration, no prompt→tool shortcut, and no run loop here — the UI asks the
// runtime to pick/prepare/run/cancel and reflects the result. HITL is NOT
// handled here: a run that needs a human decision surfaces through the unified
// Approval Center (approvalStore), never an inline modal.

function failText(message: string, command: string): string {
  return message?.trim() ? message : `Automation command '${command}' failed`;
}

interface RawCandidate {
  workflow_id?: string;
  workflow_version?: string;
  display_name?: string;
  reason?: string;
  score?: number;
  confidence?: number;
  confidence_label?: string;
  risk_tier?: string;
  requires_confirmation?: boolean;
  missing_inputs?: string[];
  suggested_input_payload?: unknown;
}

interface RawSuggestionResponse {
  candidates?: RawCandidate[];
  message?: string;
  ambiguous?: boolean;
  requires_confirmation?: boolean;
  can_auto_run?: boolean;
  status?: string;
}

function normalizeCandidate(c: RawCandidate): SuggestedWorkflow {
  return {
    workflowId: String(c.workflow_id ?? ""),
    workflowVersion: String(c.workflow_version ?? ""),
    displayName: String(c.display_name ?? c.workflow_id ?? "Workflow"),
    reason: String(c.reason ?? ""),
    confidence: Number(c.confidence ?? c.score ?? 0),
    confidenceLabel: String(c.confidence_label ?? ""),
    riskTier: String(c.risk_tier ?? ""),
    requiresConfirmation: Boolean(c.requires_confirmation),
    missingInputs: Array.isArray(c.missing_inputs) ? c.missing_inputs.map(String) : [],
    suggestedInputPayload: c.suggested_input_payload,
  };
}

/**
 * ask-KRIA-to-pick (Req 6.3): describe an intent → KRIA suggests workflows via
 * the EXISTING `suggest_n8n_workflows` command. Populates the suggestion state;
 * the UI renders SuggestionCards from it. Graceful on unavailable service
 * (Req 20.4) — records honest failure text, never throws.
 */
async function pickWorkflow(prompt: string): Promise<AutomationActionResult<SuggestedWorkflow[]>> {
  const clean = prompt.trim();
  if (!clean) return { ok: false, message: "Describe what you want to automate first." };

  setSuggesting(true);
  setSuggestError(null);
  setLastPickPrompt(clean);
  eventBus.emit("automation:pick-requested", { prompt: clean });
  try {
    const res = await bridgeInvoke<RawSuggestionResponse>("suggest_n8n_workflows", {
      request: { prompt: clean },
    });
    if (!res.ok) {
      const message = failText(res.message, "suggest_n8n_workflows");
      setSuggestError(message);
      setSuggestedWorkflows([]);
      setSuggestionMessage("");
      return { ok: false, message };
    }
    const candidates = (res.data?.candidates ?? []).map(normalizeCandidate);
    setSuggestedWorkflows(candidates);
    setSuggestionMessage(String(res.data?.message ?? ""));
    return { ok: true, data: candidates };
  } finally {
    setSuggesting(false);
  }
}

function clearSuggestion(): void {
  setSuggestedWorkflows([]);
  setSuggestionMessage("");
  setSuggestError(null);
  setLastPickPrompt("");
}

interface RawPreparedInput {
  workflow_id?: string;
  workflow_version?: string;
  display_name?: string;
  prompt?: string;
  input_payload?: unknown;
  missing_inputs?: string[];
  validation_issues?: string[];
  field_summaries?: Array<{ name?: string; type?: string; required?: boolean; description?: string }>;
  explanation?: string;
}

/**
 * Preview the inputs KRIA prepared for a run (Req 6.3) via the EXISTING
 * `prepare_n8n_workflow_input` command. The result is shown for review BEFORE
 * the user confirms a run — never auto-run.
 */
async function prepareRun(args: {
  workflowId: string;
  workflowVersion: string;
  prompt: string;
  basePayload?: unknown;
}): Promise<AutomationActionResult<PreparedRunInput>> {
  const clean = args.prompt.trim();
  if (!clean) return { ok: false, message: "A prompt is required to prepare inputs." };

  setPreparing(true);
  try {
    const res = await bridgeInvoke<RawPreparedInput>("prepare_n8n_workflow_input", {
      request: {
        workflowId: args.workflowId,
        workflowVersion: args.workflowVersion,
        prompt: clean,
        basePayload: args.basePayload ?? {},
        confirmed: true,
      },
    });
    if (!res.ok) {
      return { ok: false, message: failText(res.message, "prepare_n8n_workflow_input") };
    }
    const d = res.data ?? {};
    const prepared: PreparedRunInput = {
      workflowId: String(d.workflow_id ?? args.workflowId),
      workflowVersion: String(d.workflow_version ?? args.workflowVersion),
      displayName: String(d.display_name ?? args.workflowId),
      prompt: String(d.prompt ?? clean),
      payload: d.input_payload ?? {},
      fields: (d.field_summaries ?? []).map((f) => ({
        name: String(f.name ?? ""),
        type: f.type,
        required: f.required,
        description: f.description,
      })),
      missingInputs: Array.isArray(d.missing_inputs) ? d.missing_inputs.map(String) : [],
      validationIssues: Array.isArray(d.validation_issues) ? d.validation_issues.map(String) : [],
      explanation: d.explanation,
      inputMapped: true,
    };
    setPreparedInput(prepared);
    return { ok: true, data: prepared };
  } finally {
    setPreparing(false);
  }
}

function clearPreparedInput(): void {
  setPreparedInput(null);
}

/**
 * Run a workflow (Req 6.5) via the EXISTING `invoke_n8n_workflow_from_ui`
 * command. Marks the workflow started + seeds a "triggering" progress entry so
 * the UI shows honest live state immediately; run events then advance the phase.
 */
async function startRun(args: {
  workflowId: string;
  workflowVersion: string;
  inputPayload?: unknown;
  inputMapped?: boolean;
  runMode?: string;
}): Promise<AutomationActionResult<void>> {
  markWorkflowStarted(args.workflowId);
  updateRunProgress({
    workflowId: args.workflowId,
    phase: "triggering",
    completedSteps: 0,
    totalSteps: null,
    message: "Requesting run…",
    updatedAt: Date.now(),
  });
  const res = await bridgeInvoke<unknown>("invoke_n8n_workflow_from_ui", {
    request: {
      workflowId: args.workflowId,
      workflowVersion: args.workflowVersion,
      inputPayload: args.inputPayload ?? {},
      inputMapped: Boolean(args.inputMapped),
      requestedBy: "kria-ui",
      confirmed: true,
      runMode: args.runMode ?? "",
    },
  });
  if (!res.ok) {
    const message = failText(res.message, "invoke_n8n_workflow_from_ui");
    markWorkflowCompleted(args.workflowId, false);
    updateRunProgress({
      workflowId: args.workflowId,
      phase: "failed",
      completedSteps: 0,
      totalSteps: null,
      message,
      updatedAt: Date.now(),
    });
    return { ok: false, message };
  }
  updateRunProgress({
    workflowId: args.workflowId,
    phase: "running",
    completedSteps: 0,
    totalSteps: null,
    message: "Run started.",
    updatedAt: Date.now(),
  });
  return { ok: true, data: undefined };
}

// ── Run-event reducers (fed by the Tauri bridge from existing run events) ────

/** Merge a progress update for one run (Req 6.5). Idempotent per workflow id. */
function updateRunProgress(progress: RunProgress): void {
  setRunProgress((prev) => ({ ...prev, [progress.workflowId]: progress }));
}

/** Append a piece of run evidence for a workflow (Req 6.5). */
function appendRunEvidence(workflowId: string, item: RunEvidenceItem): void {
  setRunEvidence((prev) => ({ ...prev, [workflowId]: [...(prev[workflowId] ?? []), item] }));
}

/** Replace all evidence for a workflow (Req 6.5). */
function setRunEvidenceFor(workflowId: string, items: RunEvidenceItem[]): void {
  setRunEvidence((prev) => ({ ...prev, [workflowId]: items }));
}

/** Reset all transient Run state (suggestion, prepared input, progress,
 *  evidence). Used when leaving the Run segment / starting fresh. */
function clearRunState(): void {
  clearSuggestion();
  clearPreparedInput();
  setRunProgress({});
  setRunEvidence({});
}

// ─── 2D node builder actions (Req 6.3 / 6.4) ─────────────────────────────────
//
// Canvas edits are LOCAL draft state. Authoring lifecycle (save/test/approve)
// is DISPATCH-ONLY through EXISTING n8n authoring commands — the UI never
// orchestrates n8n; it asks the substrate to persist/validate/approve a draft
// and reflects the honest result.

/** A structural change invalidates the saved graph's test/approval state.
 *  The prior n8n draft remains persisted, so the next save MUST update it. */
function markDraftDirty(): void {
  setDraftTestResult(null);
  setDraftLifecycle("editing");
}

/** Add a palette node to the canvas at an optional position (Req 6.4). Returns
 *  the new node id and selects it so its Inspector opens. */
function addNode(kind: string, pos?: { x: number; y: number }): string {
  const item = NODE_PALETTE.find((p) => p.kind === kind);
  const label = item?.label ?? kind;
  const count = builderNodes().filter((n) => n.kind === kind).length;
  const id = newBuilderId("node");
  const params: Record<string, string> = kind === "webhook"
    ? { httpMethod: "POST", path: `kria-${id}` }
    : {};
  const node: BuilderNode = {
    id,
    kind,
    name: count > 0 ? `${label} ${count + 1}` : label,
    x: pos?.x ?? 40 + (builderNodes().length % 4) * 180,
    y: pos?.y ?? 40 + Math.floor(builderNodes().length / 4) * 120,
    params,
  };
  setBuilderNodes((prev) => [...prev, node]);
  setSelectedNodeId(id);
  markDraftDirty();
  return id;
}

/** Remove a node and any connected edges (Req 6.4). */
function removeNode(id: string): void {
  setBuilderNodes((prev) => prev.filter((n) => n.id !== id));
  setBuilderEdges((prev) => prev.filter((e) => e.source !== id && e.target !== id));
  if (selectedNodeId() === id) setSelectedNodeId(null);
  markDraftDirty();
}

/** Move a node on the canvas (Req 6.4). Position-only → does NOT reset the
 *  lifecycle (a save/test still reflects the same graph). */
function moveNode(id: string, x: number, y: number): void {
  setBuilderNodes((prev) => prev.map((n) => (n.id === id ? { ...n, x, y } : n)));
}

/** Connect two nodes source → target (Req 6.4). No-op on self/duplicate. */
function connectNodes(source: string, target: string): void {
  if (source === target) return;
  if (builderEdges().some((e) => e.source === source && e.target === target)) return;
  const exists = (id: string) => builderNodes().some((n) => n.id === id);
  if (!exists(source) || !exists(target)) return;
  setBuilderEdges((prev) => [...prev, { id: newBuilderId("edge"), source, target }]);
  markDraftDirty();
}

/** Remove a connection (Req 6.4). */
function disconnect(edgeId: string): void {
  setBuilderEdges((prev) => prev.filter((e) => e.id !== edgeId));
  markDraftDirty();
}

/** Select a node → opens its Inspector (Req 6.3). `null` clears selection. */
function selectNode(id: string | null): void {
  setSelectedNodeId(id);
}

/** Rename a node (Req 6.3 authoring). */
function renameNode(id: string, name: string): void {
  setBuilderNodes((prev) => prev.map((n) => (n.id === id ? { ...n, name } : n)));
  markDraftDirty();
}

/** Update a node's params (Req 6.3 — edit params updates the draft). */
function updateNodeParams(id: string, params: Record<string, string>): void {
  setBuilderNodes((prev) => prev.map((n) => (n.id === id ? { ...n, params } : n)));
  markDraftDirty();
}

/** Start a fresh draft (Req 6.3). */
function newDraft(): void {
  setBuilderNodes([]);
  setBuilderEdges([]);
  setSelectedNodeId(null);
  setDraftName("Untitled workflow");
  setDraftId(newBuilderId("draft"));
  setDraftLifecycle("editing");
  setDraftPersisted(false);
  setDraftTestResult(null);
  setBuilderStatus("");
}

/**
 * Serialize the canvas into an n8n workflow JSON (Req 6.4). Node `name`s are
 * uniquified so the connections map (keyed by name, as n8n requires) is
 * unambiguous. Pure function of the current builder state.
 */
function builderToWorkflowJson(): {
  name: string;
  nodes: Array<{ id: string; name: string; type: string; typeVersion: number; position: [number, number]; parameters: Record<string, string>; webhookId?: string }>;
  connections: Record<string, { main: Array<Array<{ node: string; type: string; index: number }>> }>;
} {
  const nodes = builderNodes();
  const edges = builderEdges();

  // Unique display name per node id (n8n keys connections by name).
  const nameById = new Map<string, string>();
  const seen = new Map<string, number>();
  for (const n of nodes) {
    const base = n.name.trim() || n.kind;
    const count = seen.get(base) ?? 0;
    seen.set(base, count + 1);
    nameById.set(n.id, count === 0 ? base : `${base} (${count + 1})`);
  }

  const serializedNodes = nodes.map((n) => {
    const item = NODE_PALETTE.find((p) => p.kind === n.kind);
    return {
      id: n.id,
      name: nameById.get(n.id)!,
      type: item?.n8nType ?? "n8n-nodes-base.noOp",
      typeVersion: n.kind === "chat-trigger" ? 1.1 : 1,
      position: [Math.round(n.x), Math.round(n.y)] as [number, number],
      parameters: { ...n.params },
      ...(n.kind === "chat-trigger" ? { webhookId: n.id } : {}),
    };
  });

  const connections: Record<string, { main: Array<Array<{ node: string; type: string; index: number }>> }> = {};
  for (const e of edges) {
    const sourceName = nameById.get(e.source);
    const targetName = nameById.get(e.target);
    if (!sourceName || !targetName) continue;
    if (!connections[sourceName]) connections[sourceName] = { main: [[]] };
    connections[sourceName].main[0].push({ node: targetName, type: "main", index: 0 });
  }

  return { name: draftName().trim() || "Untitled workflow", nodes: serializedNodes, connections };
}

/** Client-side dry-run validation used when the backend test path is
 *  unavailable (Req 6.3 honest fallback). Checks the graph is coherent. */
function clientSideDryRun(): DraftTestResult {
  const nodes = builderNodes();
  const edges = builderEdges();
  const issues: string[] = [];
  if (nodes.length === 0) issues.push("Add at least one node before testing.");
  const hasTrigger = nodes.some((n) =>
    n.kind === "manual-trigger" || n.kind === "webhook" || n.kind === "chat-trigger"
  );
  if (nodes.length > 0 && !hasTrigger) {
    issues.push("No trigger node — add a Manual Trigger, Webhook, or Chat Trigger to start the workflow.");
  }
  const connected = new Set<string>();
  for (const e of edges) {
    connected.add(e.source);
    connected.add(e.target);
  }
  const orphans = nodes.filter((n) => nodes.length > 1 && !connected.has(n.id));
  if (orphans.length > 0) issues.push(`${orphans.length} node(s) are not connected to anything.`);
  return {
    ok: issues.length === 0,
    message: issues.length === 0 ? "Client-side dry-run passed. Not yet persisted to n8n." : "Client-side dry-run found issues.",
    issues,
    clientSideOnly: true,
  };
}

/**
 * Save the current canvas as an n8n workflow draft (Req 6.3) via the EXISTING
 * `create_or_update_n8n_workflow_draft` command. On success the draft is
 * persisted; on an unavailable backend the draft stays client-side and the
 * status honestly reports "not yet persisted".
 */
async function saveDraft(): Promise<AutomationActionResult<void>> {
  setBuilderBusy(true);
  try {
    const workflowJson = builderToWorkflowJson();
    const wasPersisted = draftPersisted();
    const res = await bridgeInvoke<{
      status?: string;
      message?: string;
      report?: { errors?: unknown[]; warnings?: unknown[] };
      workflow?: { n8n_workflow_id?: string };
    }>("create_or_update_n8n_workflow_draft", {
      request: {
        workflowId: draftId(),
        workflowJson,
        displayName: draftName().trim() || "Untitled workflow",
        description: "Canvas-authored workflow draft",
        updateExisting: wasPersisted,
        owner: "local-user",
        requiresCallback: false,
        expectedEvidence: ["n8n_execution_output"],
        credentialRequirements: ["none"],
        dataScope: ["workflow_input", "n8n_execution_output"],
        hitlPolicy: "none",
        category: "automation",
        examplePrompts: [`Run ${draftName().trim() || "this workflow"}`],
        tags: ["n8n", "kria_canvas_authoring"],
        aliases: [draftName().trim() || "Untitled workflow"],
        allowedActions: ["draft", "test_after_review"],
      },
    });
    if (!res.ok) {
      const message = res.code === "unavailable"
        ? wasPersisted
          ? "n8n is unavailable. Your previously saved draft is unchanged; current edits remain local."
          : "n8n is unavailable. Current draft remains local and is not persisted."
        : failText(res.message, "create_or_update_n8n_workflow_draft");
      setBuilderStatus(message);
      return { ok: false, message };
    }

    const payload = res.data ?? {};
    const accepted = payload.status === "created_as_draft" || payload.status === "updated_as_draft";
    if (!accepted || !payload.workflow?.n8n_workflow_id) {
      const reportIssues = payload.report?.errors?.map(String) ?? [];
      const message = payload.message?.trim()
        || reportIssues.join(" ")
        || "n8n rejected the workflow draft; nothing was saved.";
      setBuilderStatus(message);
      return { ok: false, message };
    }

    setDraftPersisted(true);
    setDraftLifecycle("saved");
    setDraftTestResult(null);
    setBuilderStatus(payload.message?.trim() || "Draft saved to n8n.");
    eventBus.emit("automation:draft-saved", { draftId: draftId() });
    return { ok: true, data: undefined };
  } finally {
    setBuilderBusy(false);
  }
}

/**
 * Test the persisted draft through the authoritative n8n execution path. Static
 * client validation is diagnostic fallback only; it never advances lifecycle.
 */
async function testDraft(): Promise<AutomationActionResult<DraftTestResult>> {
  if (!draftPersisted() || draftLifecycle() === "editing") {
    const message = draftPersisted()
      ? "Save current edits to n8n before testing."
      : "Save the draft to n8n before testing.";
    setBuilderStatus(message);
    return { ok: false, message };
  }

  setBuilderBusy(true);
  try {
    const res = await bridgeInvoke<{
      status?: string;
      message?: string;
      correlation_id?: string;
    }>("test_n8n_workflow_draft", {
      request: {
        workflowId: draftId(),
        inputPayload: {
          source_prompt: `Test canvas-authored workflow: ${draftName().trim() || draftId()}`,
          confirmed_by_user: true,
        },
        confirmed: true,
      },
    });
    if (!res.ok) {
      if (res.code === "unavailable") {
        const local = clientSideDryRun();
        setDraftTestResult(local);
        const message = `n8n unavailable — ${local.message} This does not count as a backend test.`;
        setBuilderStatus(message);
        return { ok: false, message };
      }
      const message = failText(res.message, "test_n8n_workflow_draft");
      setBuilderStatus(message);
      return { ok: false, message };
    }

    const payload = res.data ?? {};
    if (payload.status !== "test_started" || !payload.correlation_id) {
      const message = payload.message?.trim() || "Backend did not start the workflow test.";
      setBuilderStatus(message);
      return { ok: false, message };
    }
    const result: DraftTestResult = {
      ok: true,
      message: payload.message?.trim() || "Backend test started. Review Run History before approval.",
      issues: [],
      clientSideOnly: false,
    };
    setDraftTestResult(result);
    setDraftLifecycle("tested");
    setBuilderStatus(result.message);
    return { ok: true, data: result };
  } finally {
    setBuilderBusy(false);
  }
}

/**
 * Approve/publish the draft (Req 6.3) via the EXISTING
 * `approve_n8n_workflow_draft` command. Requires the draft to be persisted
 * first (honest gate). A consequential publish routes its HITL through the
 * unified Approval Center (approvalStore), never an inline modal.
 */
async function approveDraft(): Promise<AutomationActionResult<void>> {
  if (!draftPersisted()) {
    const message = "Save the draft to n8n before approving.";
    setBuilderStatus(message);
    return { ok: false, message };
  }
  if (draftLifecycle() !== "tested" || draftTestResult()?.clientSideOnly !== false) {
    const message = draftLifecycle() === "editing"
      ? "Save current edits, then run a backend test before approving."
      : "Run and review a backend test before approving.";
    setBuilderStatus(message);
    return { ok: false, message };
  }
  setBuilderBusy(true);
  try {
    const res = await bridgeInvoke<{ status?: string; message?: string }>("approve_n8n_workflow_draft", {
      request: { workflowId: draftId(), confirmed: true },
    });
    if (!res.ok) {
      const message = failText(res.message, "approve_n8n_workflow_draft");
      setBuilderStatus(message);
      return { ok: false, message };
    }
    if (res.data?.status !== "approved") {
      const message = res.data?.message?.trim() || "Backend did not approve the workflow.";
      setBuilderStatus(message);
      return { ok: false, message };
    }
    setDraftLifecycle("approved");
    setBuilderStatus(res.data.message?.trim() || "Workflow approved.");
    eventBus.emit("automation:draft-approved", { draftId: draftId() });
    return { ok: true, data: undefined };
  } finally {
    setBuilderBusy(false);
  }
}

// ─── Export ────────────────────────────────────────────────────────────────────

export const automationStore = {
  workflows,
  scheduledTasks,
  tasks,
  reminders,
  briefingConfig,
  briefingLoading,
  briefingSaving,
  briefingError,
  briefingStatus,
  activeSegment,
  runningWorkflowIds,
  loading,
  searchQuery,

  // Run: ask-KRIA-to-pick + prepared input + progress + evidence
  suggestedWorkflows,
  suggestionMessage,
  suggesting,
  suggestError,
  lastPickPrompt,
  preparedInput,
  preparing,
  runProgress,
  runEvidence,

  setWorkflows,
  setScheduledTasks,
  setTasks,
  setReminders,
  setBriefingConfig,
  setActiveSegment,
  setLoading,
  setSearchQuery,
  initialize: initializeAutomationStore,
  dispose: disposeAutomationStore,
  markWorkflowStarted,
  markWorkflowCompleted,

  // Schedule (Req 6.6) — all dispatch through existing commands via the bridge
  loadSchedule,
  loadScheduledTasks,
  loadTasks,
  loadReminders,
  loadBriefingConfig,
  saveBriefingConfig,
  addScheduledTask,
  removeScheduledTask,
  addTask,
  setTaskStatus,
  toggleTaskDone,
  editTask,
  deleteTask,
  setReminder,
  snoozeReminder,
  cancelReminder,

  // Run dispatch (route through existing commands via the bridge)
  pickWorkflow,
  clearSuggestion,
  prepareRun,
  clearPreparedInput,
  startRun,

  // Run-event reducers (fed by the bridge from existing run events)
  updateRunProgress,
  appendRunEvidence,
  setRunEvidenceFor,
  clearRunState,

  // 2D node builder (Build segment, Req 6.3 / 6.4)
  builderNodes,
  builderEdges,
  selectedNodeId,
  draftName,
  draftId,
  draftLifecycle,
  draftPersisted,
  builderStatus,
  builderBusy,
  draftTestResult,
  setDraftName,
  addNode,
  removeNode,
  moveNode,
  connectNodes,
  disconnect,
  selectNode,
  renameNode,
  updateNodeParams,
  newDraft,
  builderToWorkflowJson,
  saveDraft,
  testDraft,
  approveDraft,
} as const;
