import { createSignal } from "solid-js";
import { bridgeInvoke } from "../bridge/invoke";
import type {
  ExecutivePreemption,
  ExecutiveSnapshot,
  ExecutiveTask,
  ExecutiveTaskCompleted,
  ExecutiveTaskStarted,
  GpuLeaseEvent,
} from "../types/intelligence";
import { eventBus, type HraDiagnosticsEvent } from "./eventBus";

export type ObservatorySegment = "now" | "jobs" | "analytics" | "forensics" | "diagnostics";
export type JobStatus =
  | "queued"
  | "running"
  | "paused"
  | "completed"
  | "failed"
  | "cancelled"
  | "timed_out"
  | "rolled_back"
  | "recovered"
  | "unknown";
export type DataAuthority = "awaiting-data" | "live" | "shadow-mode" | "error";
export type JobCancelKind = "executive" | "capability" | "test-runner";

export interface TelemetryPoint { metric: string; value: number; ts: number }
export interface Job {
  id: string;
  name: string;
  status: JobStatus;
  /** Omitted when runtime exposes state but no measured progress. */
  progress?: number;
  startedAt: number;
  completedAt?: number;
  error?: string;
  cancelKind?: JobCancelKind;
  backendId?: string;
}
export interface AnalyticsTile {
  id: string;
  label: string;
  value: number;
  unit: string;
  trend?: "up" | "down" | "flat";
}
export interface ForensicRecord {
  id: string;
  timestamp_unix_ms: number;
  category: string;
  severity: string;
  summary: string;
  source: string;
  evidence: string;
  last_gasp_detected: boolean;
}
export interface TestRunState {
  running: boolean;
  started_unix_ms: number | null;
  pid: number | null;
  mode: string | null;
  run_label: string | null;
  command: string | null;
}
interface AnalyticsDashboard {
  uptime_secs: number;
  overview: {
    total_sessions: number;
    total_turns: number;
    total_tools: number;
    mcp_servers_running: number;
    mcp_servers_total: number;
  };
  orchestrator: { active: boolean; active_turns: number; server_healthy: boolean };
}

const EMPTY_TEST_STATE: TestRunState = {
  running: false,
  started_unix_ms: null,
  pid: null,
  mode: null,
  run_label: null,
  command: null,
};
const EMPTY_EXECUTIVE_SNAPSHOT: ExecutiveSnapshot = {
  active_foreground: null,
  active_background: [],
  queued: [],
  gpu_lease_holder: null,
  gpu_lease_remaining_ms: null,
  total_completed: 0,
  total_failed: 0,
};
const TELEMETRY_BUFFER_CAP = 1000;
const EXECUTIVE_EVENT_CAP = 200;

let telemetryConsumers = 0;
let telemetryGeneration = 0;
let disconnectTelemetry: (() => void) | null = null;
let jobConsumers = 0;
let jobGeneration = 0;
let jobPollTimer: ReturnType<typeof setInterval> | null = null;

const [activeSegment, setActiveSegment] = createSignal<ObservatorySegment>("now");
const [telemetryBuffer, setTelemetryBuffer] = createSignal<TelemetryPoint[]>([]);
const [jobs, setJobs] = createSignal<Job[]>([]);
const [executiveSnapshot, setExecutiveSnapshot] = createSignal<ExecutiveSnapshot | null>(null);
const [executiveRecentEvents, setExecutiveRecentEvents] = createSignal<ExecutiveTaskCompleted[]>([]);
const [executiveAuthority, setExecutiveAuthority] = createSignal<DataAuthority>("awaiting-data");
const [analytics, setAnalytics] = createSignal<AnalyticsTile[]>([]);
const [forensics, setForensics] = createSignal<ForensicRecord[]>([]);
const [hraDiagnostics, setHraDiagnostics] = createSignal<HraDiagnosticsEvent | null>(null);
const [hraAuthority, setHraAuthority] = createSignal<DataAuthority>("awaiting-data");
const [resourceMetrics, setResourceMetrics] = createSignal<Record<string, number>>({});
const [telemetryAuthority, setTelemetryAuthority] = createSignal<DataAuthority>("awaiting-data");
const [analyticsAuthority, setAnalyticsAuthority] = createSignal<DataAuthority>("awaiting-data");
const [forensicsAuthority, setForensicsAuthority] = createSignal<DataAuthority>("awaiting-data");
const [testAuthority, setTestAuthority] = createSignal<DataAuthority>("awaiting-data");
const [testRunState, setTestRunState] = createSignal<TestRunState>(EMPTY_TEST_STATE);
const [lastError, setLastError] = createSignal<string | null>(null);

function pushTelemetry(point: TelemetryPoint, authority: DataAuthority = "live"): void {
  setTelemetryBuffer((prev) => {
    const next = [...prev, point];
    return next.length > TELEMETRY_BUFFER_CAP ? next.slice(-TELEMETRY_BUFFER_CAP) : next;
  });
  setResourceMetrics((prev) => ({ ...prev, [point.metric]: point.value }));
  setTelemetryAuthority(authority);
}

function isFiniteNumber(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value);
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function isCompleteHraDiagnostics(payload: HraDiagnosticsEvent): boolean {
  return payload.available !== false
    && isRecord(payload.status)
    && Array.isArray(payload.devices)
    && isRecord(payload.telemetry)
    && Array.isArray(payload.recovered_open_leases)
    && isRecord(payload.sla)
    && isRecord(payload.co_residency)
    && Array.isArray(payload.decisions)
    && isRecord(payload.forecast)
    && typeof payload.profile === "string"
    && Array.isArray(payload.residents);
}

function applyHraDiagnostics(payload: HraDiagnosticsEvent): boolean {
  setHraDiagnostics(payload);
  if (payload.available === false) {
    setHraAuthority("shadow-mode");
    setTelemetryAuthority("shadow-mode");
    return false;
  }
  setHraAuthority(isCompleteHraDiagnostics(payload) ? "live" : "awaiting-data");

  if (!isRecord(payload.telemetry)) {
    setTelemetryAuthority("awaiting-data");
    return false;
  }

  const telemetry = payload.telemetry;
  if (telemetry.source !== "unified_hub") {
    setTelemetryAuthority("shadow-mode");
    return false;
  }

  const cpuPercent = telemetry.cpu_avg_pct;
  const ramTotalMb = telemetry.ram_total_mb;
  const ramFreeMb = telemetry.ram_free_mb;
  if (!isFiniteNumber(cpuPercent) || cpuPercent < 0 || cpuPercent > 100
    || !isFiniteNumber(ramTotalMb) || ramTotalMb <= 0
    || !isFiniteNumber(ramFreeMb) || ramFreeMb < 0 || ramFreeMb > ramTotalMb) {
    setTelemetryAuthority("awaiting-data");
    return false;
  }

  const timestamp = Date.now();
  pushTelemetry({ metric: "cpu_percent", value: cpuPercent, ts: timestamp });
  pushTelemetry({
    metric: "memory_percent",
    value: 100 * (ramTotalMb - ramFreeMb) / ramTotalMb,
    ts: timestamp,
  });
  return true;
}

async function pullHraDiagnostics(generation: number): Promise<void> {
  const result = await bridgeInvoke<HraDiagnosticsEvent>("get_hra_diagnostics");
  if (generation !== telemetryGeneration || telemetryConsumers === 0) return;
  if (result.ok === false) {
    const authority = result.code === "unavailable" ? "shadow-mode" : "error";
    setHraAuthority(authority);
    setTelemetryAuthority(authority);
    setLastError(result.message);
    return;
  }
  applyHraDiagnostics(result.data);
}

async function refreshHraDiagnostics(): Promise<HraDiagnosticsEvent | null> {
  const result = await bridgeInvoke<HraDiagnosticsEvent>("get_hra_diagnostics");
  if (result.ok === false) {
    setHraAuthority(result.code === "unavailable" ? "shadow-mode" : "error");
    setLastError(result.message);
    return null;
  }
  applyHraDiagnostics(result.data);
  return result.data;
}

function connectTelemetry(): () => void {
  let disposed = false;
  telemetryConsumers += 1;
  if (telemetryConsumers === 1) {
    const generation = ++telemetryGeneration;
    disconnectTelemetry = eventBus.on("observatory:hra-diagnostics", applyHraDiagnostics);
    void pullHraDiagnostics(generation);
  }

  return () => {
    if (disposed) return;
    disposed = true;
    telemetryConsumers = Math.max(0, telemetryConsumers - 1);
    if (telemetryConsumers === 0) {
      telemetryGeneration += 1;
      disconnectTelemetry?.();
      disconnectTelemetry = null;
    }
  };
}

interface RawCppJob {
  id?: string;
  provider_id?: string;
  capability_id?: string;
  state?: string;
  created_at?: string;
  updated_at?: string;
  last_error?: string | null;
}

const CPP_JOB_STATES = new Set<JobStatus>([
  "queued", "running", "paused", "completed", "failed", "cancelled",
  "timed_out", "rolled_back", "recovered",
]);

function cppJobStatus(value: unknown): JobStatus {
  const normalized = String(value ?? "").toLowerCase() as JobStatus;
  return CPP_JOB_STATES.has(normalized) ? normalized : "unknown";
}

function cppJobTime(value: unknown): number {
  const parsed = Date.parse(String(value ?? ""));
  return Number.isFinite(parsed) ? parsed : 0;
}

async function loadCppJobs(generation?: number): Promise<boolean> {
  const result = await bridgeInvoke<RawCppJob[]>("cpp_jobs", { limit: 200 });
  if (generation !== undefined && (generation !== jobGeneration || jobConsumers === 0)) return false;
  if (!result.ok) {
    setLastError(result.message);
    return false;
  }
  if (!Array.isArray(result.data)) {
    setLastError("cpp_jobs returned an invalid payload; expected an array");
    return false;
  }

  const projected = result.data.map((raw): Job => {
    const status = cppJobStatus(raw.state);
    const updatedAt = cppJobTime(raw.updated_at);
    return {
      id: `cpp:${String(raw.id ?? "")}`,
      backendId: String(raw.id ?? ""),
      name: `${String(raw.provider_id ?? "unknown")}:${String(raw.capability_id ?? "unknown")}`,
      status,
      startedAt: cppJobTime(raw.created_at),
      completedAt: ["completed", "failed", "cancelled", "timed_out", "rolled_back", "recovered"]
        .includes(status) ? updatedAt : undefined,
      error: raw.last_error == null ? undefined : String(raw.last_error),
      cancelKind: "capability",
    };
  });
  setJobs((current) => [
    ...current.filter((job) => job.cancelKind !== "capability"),
    ...projected,
  ]);
  return true;
}

function connectJobs(): () => void {
  let disposed = false;
  jobConsumers += 1;
  if (jobConsumers === 1) {
    const generation = ++jobGeneration;
    void loadCppJobs(generation);
    jobPollTimer = setInterval(() => void loadCppJobs(generation), 5_000);
  }
  return () => {
    if (disposed) return;
    disposed = true;
    jobConsumers = Math.max(0, jobConsumers - 1);
    if (jobConsumers === 0) {
      jobGeneration += 1;
      if (jobPollTimer !== null) clearInterval(jobPollTimer);
      jobPollTimer = null;
    }
  };
}

function updateJob(jobId: string, update: Partial<Job>): void {
  setJobs((prev) => prev.map((job) => job.id === jobId ? { ...job, ...update } : job));
}

function addJob(job: Job): void {
  setJobs((prev) => prev.some((item) => item.id === job.id)
    ? prev.map((item) => item.id === job.id ? job : item)
    : [...prev, job]);
}
async function cancelJob(jobId: string): Promise<boolean> {
  const job = jobs().find((item) => item.id === jobId);
  if (!job || !job.cancelKind || !["queued", "running", "paused"].includes(job.status)) return false;

  const backendId = job.backendId ?? job.id;
  const request = job.cancelKind === "executive"
    ? bridgeInvoke<void>("cancel_executive_task", { taskId: backendId })
    : job.cancelKind === "capability"
      ? bridgeInvoke<void>("cpp_job_control", { id: backendId, action: "cancel" })
      : bridgeInvoke<boolean>("stop_test_run");
  const result = await request;
  if (!result.ok) {
    setLastError(result.message);
    return false;
  }
  updateJob(jobId, { status: "cancelled" });
  if (job.cancelKind === "capability") await loadCppJobs();
  return true;
}

function executiveBase(): ExecutiveSnapshot {
  return executiveSnapshot() ?? { ...EMPTY_EXECUTIVE_SNAPSHOT, active_background: [], queued: [] };
}

function applyExecutiveTaskStarted(payload: ExecutiveTaskStarted): void {
  const task: ExecutiveTask = {
    id: payload.task_id,
    priority: payload.priority,
    source: payload.source,
    state: "Running",
    description: payload.description,
    submitted_at: payload.ts,
    started_at: payload.ts,
    completed_at: null,
    duration_ms: null,
    error: null,
    requires_gpu: false,
  };
  setExecutiveSnapshot((previous) => {
    const current = previous ?? executiveBase();
    const background = current.active_background.filter((item) => item.id !== task.id);
    const queued = current.queued.filter((item) => item.id !== task.id);
    if (["Voice", "Interactive"].includes(task.priority)) {
      return { ...current, active_foreground: task, active_background: background, queued };
    }
    return { ...current, active_background: [...background, task], queued };
  });
  setExecutiveAuthority("live");
}

function appendExecutiveEvent(event: ExecutiveTaskCompleted): void {
  setExecutiveRecentEvents((previous) => [event, ...previous].slice(0, EXECUTIVE_EVENT_CAP));
}

function applyExecutiveTaskCompleted(payload: ExecutiveTaskCompleted): void {
  appendExecutiveEvent(payload);
  setExecutiveSnapshot((previous) => {
    const current = previous ?? executiveBase();
    return {
      ...current,
      active_foreground: current.active_foreground?.id === payload.task_id
        ? null : current.active_foreground,
      active_background: current.active_background.filter((item) => item.id !== payload.task_id),
      queued: current.queued.filter((item) => item.id !== payload.task_id),
      total_completed: current.total_completed + (payload.success ? 1 : 0),
      total_failed: current.total_failed + (payload.success ? 0 : 1),
    };
  });
  setExecutiveAuthority("live");
}

function applyExecutivePreemption(payload: ExecutivePreemption): void {
  appendExecutiveEvent({
    task_id: payload.victim_id,
    success: false,
    duration_ms: 0,
    output_summary: `Preempted by ${payload.replacement_priority} task`,
    error: null,
    ts: payload.ts,
  });
  setExecutiveSnapshot((previous) => {
    const current = previous ?? executiveBase();
    return {
      ...current,
      active_foreground: current.active_foreground?.id === payload.victim_id
        ? null : current.active_foreground,
      active_background: current.active_background.filter((item) => item.id !== payload.victim_id),
      queued: current.queued.filter((item) => item.id !== payload.victim_id),
    };
  });
  setExecutiveAuthority("live");
}

function applyExecutiveGpuLease(payload: GpuLeaseEvent): void {
  setExecutiveSnapshot((previous) => {
    const current = previous ?? executiveBase();
    return payload.action === "acquired"
      ? { ...current, gpu_lease_holder: payload.task_id }
      : { ...current, gpu_lease_holder: null, gpu_lease_remaining_ms: null };
  });
  setExecutiveAuthority("live");
}

function connectExecutiveEvents(): () => void {
  const unsubscribe = [
    eventBus.on("observatory:executive-task-started", applyExecutiveTaskStarted),
    eventBus.on("observatory:executive-task-completed", applyExecutiveTaskCompleted),
    eventBus.on("observatory:executive-preemption", applyExecutivePreemption),
    eventBus.on("observatory:executive-gpu-lease", applyExecutiveGpuLease),
  ];
  return () => unsubscribe.forEach((dispose) => dispose());
}

async function loadExecutiveSnapshot(): Promise<void> {
  setExecutiveAuthority("awaiting-data");
  const result = await bridgeInvoke<ExecutiveSnapshot>("get_executive_snapshot");
  if (!result.ok) {
    setExecutiveAuthority(result.code === "unavailable" ? "shadow-mode" : "error");
    setLastError(result.message);
    return;
  }
  setExecutiveSnapshot(result.data);
  setExecutiveAuthority("live");
}

async function cancelExecutiveTask(taskId: string): Promise<boolean> {
  const task = [executiveSnapshot()?.active_foreground,
    ...(executiveSnapshot()?.active_background ?? [])]
    .find((item) => item?.id === taskId);
  if (!task || task.state !== "Running") return false;

  const result = await bridgeInvoke<void>("cancel_executive_task", { taskId });
  if (!result.ok) {
    setLastError(result.message);
    return false;
  }
  // Command acceptance is not task completion. Keep authoritative state until
  // ExecutiveController emits task_completed/preemption.
  return true;
}

async function loadAnalytics(): Promise<void> {
  setAnalyticsAuthority("awaiting-data");
  const result = await bridgeInvoke<AnalyticsDashboard>("get_analytics_dashboard");
  if (!result.ok) {
    setAnalyticsAuthority(result.code === "unavailable" ? "shadow-mode" : "error");
    setLastError(result.message);
    return;
  }
  const data = result.data;
  setAnalytics([
    { id: "sessions", label: "Sessions", value: data.overview.total_sessions, unit: "total" },
    { id: "turns", label: "Turns", value: data.overview.total_turns, unit: "total" },
    { id: "tools", label: "Registered tools", value: data.overview.total_tools, unit: "total" },
    { id: "active-turns", label: "Active turns", value: data.orchestrator.active_turns, unit: "now" },
    { id: "uptime", label: "Uptime", value: Math.floor(data.uptime_secs / 60), unit: "min" },
  ]);
  setAnalyticsAuthority("live");
}

async function loadForensics(limit = 64): Promise<void> {
  setForensicsAuthority("awaiting-data");
  const result = await bridgeInvoke<{ records: ForensicRecord[] }>("get_ironclad_forensics", { limit });
  if (!result.ok) {
    setForensicsAuthority(result.code === "unavailable" ? "shadow-mode" : "error");
    setLastError(result.message);
    return;
  }
  setForensics([...result.data.records].sort((a, b) => b.timestamp_unix_ms - a.timestamp_unix_ms));
  setForensicsAuthority("live");
}
async function refreshTestRunState(): Promise<void> {
  const result = await bridgeInvoke<TestRunState>("get_test_run_state");
  if (!result.ok) {
    setTestAuthority(result.code === "unavailable" ? "shadow-mode" : "error");
    setLastError(result.message);
    return;
  }
  setTestRunState(result.data);
  setTestAuthority("live");
}

async function startTestRun(mode: "SAFE" | "SMOKE" = "SAFE"): Promise<boolean> {
  const result = await bridgeInvoke<TestRunState>("start_test_run", {
    request: { mode, allow_destructive: false, snapshot: false, continue_on_failure: false },
  }, { timeoutMs: 15_000 });
  if (!result.ok) {
    setLastError(result.message);
    setTestAuthority(result.code === "unavailable" ? "shadow-mode" : "error");
    return false;
  }
  setTestRunState(result.data);
  setTestAuthority("live");
  return true;
}

async function stopTestRun(): Promise<boolean> {
  const result = await bridgeInvoke<boolean>("stop_test_run");
  if (!result.ok) {
    setLastError(result.message);
    return false;
  }
  if (result.data) setTestRunState(EMPTY_TEST_STATE);
  return result.data;
}

export const observatoryStore = {
  activeSegment, telemetryBuffer, jobs, executiveSnapshot, executiveRecentEvents,
  executiveAuthority, analytics, forensics, hraDiagnostics, hraAuthority, resourceMetrics,
  telemetryAuthority, analyticsAuthority, forensicsAuthority, testAuthority,
  testRunState, lastError,
  setActiveSegment, setTelemetryBuffer, setJobs, setExecutiveSnapshot,
  setExecutiveRecentEvents, setExecutiveAuthority, setAnalytics, setForensics,
  setHraDiagnostics, setHraAuthority, setResourceMetrics, setTelemetryAuthority,
  setAnalyticsAuthority, setForensicsAuthority, setTestAuthority, setTestRunState,
  pushTelemetry, applyHraDiagnostics, refreshHraDiagnostics, connectTelemetry,
  loadCppJobs, connectJobs, updateJob, addJob, cancelJob, loadExecutiveSnapshot,
  cancelExecutiveTask, connectExecutiveEvents, applyExecutiveTaskStarted,
  applyExecutiveTaskCompleted, applyExecutivePreemption, applyExecutiveGpuLease,
  loadAnalytics, loadForensics, refreshTestRunState, startTestRun, stopTestRun,
} as const;
