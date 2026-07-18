/**
 * Memory Store — per-Space memory data.
 *
 * Holds facts, knowledge documents, graph state, and cognition control state,
 * plus the memory-command action layer (task 6.2, Req 5.2/5.3): fetch the full
 * Inspector detail (memory_explain) and dispatch verify / correct / reinforce /
 * penalize / forget / hard-delete through the EXISTING memory_* Tauri commands.
 *
 * ARCHITECTURE (KRIA runtime authority): every mutation routes through a
 * memory_* command via the bridge — the runtime enforces the Write Policy /
 * lifecycle / truth engines and persists; the UI never bypasses. Actions are
 * OPTIMISTIC on the local read-model and return a typed result so callers can
 * surface HONEST success/failure (fixing the old silent-failure states) — the
 * store never swallows an error into a no-op. `forget` is reversible via a
 * one-shot undo buffer (`memory_restore_forgotten` restores the same backend
 * identity); `hard_delete` is irreversible and carries no undo (the deliberate
 * confirm lives in the UI).
 *
 * Requirements: 5.1 (Memory Space segments), 5.2 (Inspector detail),
 * 5.3 (actions + undo + deliberate confirm), 13.4 (preserve state)
 */
import { createSignal } from "solid-js";
import { eventBus, type Unsubscribe } from "./eventBus";
import { bridgeInvoke } from "../bridge/invoke";
import type { CognitionJob } from "./coreStore";

// ─── Types ─────────────────────────────────────────────────────────────────────

export interface MemoryFact {
  id: string;
  content: string;
  confidence: number;
  worth: number;
  staleness: number;
  source: string;
  createdAt: number;
  updatedAt: number;
  tags: string[];
}

export interface KnowledgeDocument {
  id: string;
  title: string;
  type: string;
  indexedAt: number;
  size: number;
}

export interface MemoryGoal {
  id: string;
  kind: string;
  title: string;
  status: string;
  confidence: number;
  priority: number;
  parentId: string | null;
  createdAt: number;
  lastProgressAt: number | null;
}

export interface MemoryReasoningStats {
  chains: number;
  hypotheses: number;
  counterexamples: number;
  failedChains: number;
  averageConfidence: number;
  hallucinationRate: number;
}

export interface MemoryPlanStats {
  distinctPlans: number;
  totalExecutions: number;
  successRate: number;
}

export interface MemoryColdStartStatus {
  onboardingComplete: boolean;
  granted: string[];
}

export type MemoryScanSource = "filesystem" | "git" | "workspace" | "shell";

export interface MemoryScanCandidate {
  path: string;
  detail: string;
  content?: string;
  kind?: string;
}

export interface MemoryReasoningTrace {
  id?: string;
  task: string;
  approach?: string;
  outcome?: string;
  confidence?: number;
  created_at?: string;
  [key: string]: unknown;
}

export interface MemoryCausalLink {
  id?: string;
  cause: string;
  effect: string;
  strength?: number;
  evidence?: string[];
  [key: string]: unknown;
}

export interface MemoryCausalChain {
  path: string[];
  confidence: number;
  [key: string]: unknown;
}

export interface MemoryReasoningQuery {
  mode: "history" | "effects" | "causes" | "chains";
  query: string;
  traces?: MemoryReasoningTrace[];
  links?: MemoryCausalLink[];
  chains?: MemoryCausalChain[];
}

/**
 * The full Inspector detail for one memory (Req 5.2). Normalized (camelCase)
 * from the `memory_explain` façade payload so the Inspector body reads a single
 * typed shape. Every field maps to a Req-5.2 disclosure:
 *   content · confidence · worth(success/failure/samples) · state
 *   (verification/truth) · stalenessClass (staleness) · sourceEventTag (source)
 *   · contradicts (conflicts) · derivedFrom + supersededBy (lineage / version).
 */
export interface MemoryDetail {
  id: string;
  content: string;
  memoryType: string;
  /** Verification / truth state (e.g. "active", "forgotten"). */
  state: string;
  confidence: number;
  importance: number;
  /** Provenance source tag (Req 5.2 "source"). */
  sourceEventTag: string | null;
  /** Lineage: memories this was derived from (Req 5.2). */
  derivedFrom: string[];
  /** Conflicts / contradictions (Req 5.2). */
  contradicts: string[];
  worthSuccess: number;
  worthFailure: number;
  worthSamples: number;
  accessCount: number;
  /** Staleness class (Req 5.2 "staleness"). */
  stalenessClass: string;
  /** Lineage: the memory that superseded this one, if any (Req 5.2). */
  supersededBy: string | null;
}

/**
 * Typed result for every memory command dispatch (Req 5.3 honest states). A
 * failure carries a user-actionable message the caller surfaces (never a
 * silent no-op).
 */
export type MemoryActionResult<T = void> =
  | { ok: true; data: T }
  | { ok: false; message: string };

/** A forgotten fact retained briefly so the UI can offer a one-shot undo. */
export interface PendingUndo {
  fact: MemoryFact;
}

/** Raw `memory_explain` payload (snake_case façade shape). */
interface MemoryExplainPayload {
  id: string;
  content: string;
  memory_type: string;
  state: string;
  confidence: number;
  importance: number;
  source_event_tag: string | null;
  derived_from: string[];
  contradicts: string[];
  worth_success: number;
  worth_failure: number;
  worth_samples: number;
  access_count: number;
  staleness_class: string;
  superseded_by: string | null;
}

function normalizeDetail(p: MemoryExplainPayload): MemoryDetail {
  return {
    id: p.id,
    content: p.content,
    memoryType: p.memory_type,
    state: p.state,
    confidence: p.confidence,
    importance: p.importance,
    sourceEventTag: p.source_event_tag ?? null,
    derivedFrom: p.derived_from ?? [],
    contradicts: p.contradicts ?? [],
    worthSuccess: p.worth_success ?? 0,
    worthFailure: p.worth_failure ?? 0,
    worthSamples: p.worth_samples ?? 0,
    accessCount: p.access_count ?? 0,
    stalenessClass: p.staleness_class ?? "unknown",
    supersededBy: p.superseded_by ?? null,
  };
}

// ─── Cognition (Req 5.6) ─────────────────────────────────────────────────────

/**
 * One "what changed" line in a cognition result (Req 5.6). A stable label plus
 * a numeric count derived from the command's return payload — the honest,
 * concrete change the job produced (facts consolidated, entities linked, …).
 */
export interface CognitionChange {
  label: string;
  value: number;
}

/**
 * The persistent result of one cognition run (Req 5.6). Kept in a history list
 * so the Cognition panel shows WHAT CHANGED durably — never a transient toast.
 * `ok:false` carries a user-actionable `message` (honest failure state).
 * `summary` is a plain-language, UI-generated narrative (no untrusted HTML).
 */
export interface CognitionResult {
  id: string;
  job: CognitionJob;
  at: number;
  ok: boolean;
  changes: CognitionChange[];
  summary: string;
  message?: string;
}

/**
 * Memory Space segments (Req 5.1). `landing` is the default overview view;
 * the rest are the lenses/segments surfaced in the segment bar. `knowledgegraph`
 * is scaffolded here (region built in tasks 6.4/6.5).
 */
export type MemorySegment =
  | "landing"
  | "explorer"
  | "timeline"
  | "goals"
  | "reasoning"
  | "library"
  | "knowledgegraph"
  | "cognition"
  | "coldstart";

// ─── Signals ───────────────────────────────────────────────────────────────────

const [facts, setFacts] = createSignal<MemoryFact[]>([]);
const [documents, setDocuments] = createSignal<KnowledgeDocument[]>([]);
const [goals, setGoals] = createSignal<MemoryGoal[]>([]);
const [reasoningStats, setReasoningStats] = createSignal<MemoryReasoningStats | null>(null);
const [planStats, setPlanStats] = createSignal<MemoryPlanStats | null>(null);
const [coldStartStatus, setColdStartStatus] = createSignal<MemoryColdStartStatus | null>(null);
const [activeSegment, setActiveSegment] = createSignal<MemorySegment>("landing");
const [searchQuery, setSearchQuery] = createSignal("");
const [loading, setLoading] = createSignal(false);
/** One-shot undo buffer for the last reversible `forget` (Req 5.3). */
const [pendingUndo, setPendingUndo] = createSignal<PendingUndo | null>(null);
/** Cognition jobs currently in-flight (Req 5.6 running state). */
const [cognitionRunning, setCognitionRunning] = createSignal<CognitionJob[]>([]);
/** Persistent cognition results, newest first (Req 5.6 — not a toast). */
const [cognitionResults, setCognitionResults] = createSignal<CognitionResult[]>([]);
const [loadError, setLoadError] = createSignal<string | null>(null);
const [reasoningQuery, setReasoningQuery] = createSignal<MemoryReasoningQuery | null>(null);
const [reasoningQueryBusy, setReasoningQueryBusy] = createSignal(false);
const [reasoningQueryError, setReasoningQueryError] = createSignal<string | null>(null);

interface TimelinePayload {
  entries?: Array<{
    id: string;
    content: string;
    memory_type?: string;
    confidence?: number;
    created_at?: string;
  }>;
}

interface LibraryPayload {
  documents?: Array<{
    doc_id: string;
    title?: string | null;
    path?: string;
    version?: number;
    chunks?: number;
  }>;
}

interface GoalsPayload {
  goals?: Array<{
    id: string;
    kind?: string;
    title: string;
    status?: string;
    confidence?: number;
    priority?: number;
    parent_id?: string | null;
    created_at?: string;
    last_progress_at?: string | null;
  }>;
}

let memorySubscriptions: Unsubscribe[] = [];
let refreshTimer: ReturnType<typeof setTimeout> | null = null;
let initialized = false;

function parseTime(value: string | null | undefined): number {
  const parsed = value ? Date.parse(value) : Date.now();
  return Number.isFinite(parsed) ? parsed : Date.now();
}

async function refreshProductionData(): Promise<void> {
  setLoading(true);
  setLoadError(null);
  const [timeline, library, goalRows, reasoning, plans, coldStart] = await Promise.all([
    bridgeInvoke<TimelinePayload>("memory_timeline", { limit: 500 }, { timeoutMs: 15_000 }),
    bridgeInvoke<LibraryPayload>("memory_library_list", undefined, { timeoutMs: 15_000 }),
    bridgeInvoke<GoalsPayload>("memory_goals_list", { limit: 200 }, { timeoutMs: 15_000 }),
    bridgeInvoke<Record<string, number>>("memory_reasoning_analytics", undefined, { timeoutMs: 15_000 }),
    bridgeInvoke<Record<string, number>>("memory_plans_analytics", undefined, { timeoutMs: 15_000 }),
    bridgeInvoke<{ onboarding_complete?: boolean; granted?: string[] }>("memory_cold_start_status"),
  ]);

  if (timeline.ok) {
    setFacts((timeline.data.entries ?? []).map((entry) => ({
      id: entry.id,
      content: entry.content,
      confidence: entry.confidence ?? 0,
      worth: 0,
      staleness: 0,
      source: entry.memory_type ?? "memory",
      createdAt: parseTime(entry.created_at),
      updatedAt: parseTime(entry.created_at),
      tags: entry.memory_type ? [entry.memory_type] : [],
    })));
  }
  if (library.ok) {
    setDocuments((library.data.documents ?? []).map((doc) => ({
      id: doc.doc_id,
      title: doc.title?.trim() || doc.path?.split("/").pop() || "Untitled document",
      type: doc.path?.split(".").pop()?.toUpperCase() || "Document",
      indexedAt: Date.now(),
      size: doc.chunks ?? 0,
    })));
  }
  if (goalRows.ok) {
    setGoals((goalRows.data.goals ?? []).map((goal) => ({
      id: goal.id,
      kind: goal.kind ?? "goal",
      title: goal.title,
      status: goal.status ?? "candidate",
      confidence: goal.confidence ?? 0,
      priority: goal.priority ?? 0,
      parentId: goal.parent_id ?? null,
      createdAt: parseTime(goal.created_at),
      lastProgressAt: goal.last_progress_at ? parseTime(goal.last_progress_at) : null,
    })));
  }
  if (reasoning.ok) {
    setReasoningStats({
      chains: reasoning.data.chains ?? 0,
      hypotheses: reasoning.data.hypotheses ?? 0,
      counterexamples: reasoning.data.counterexamples ?? 0,
      failedChains: reasoning.data.failed_chains ?? 0,
      averageConfidence: reasoning.data.avg_confidence ?? 0,
      hallucinationRate: reasoning.data.hallucination_rate ?? 0,
    });
  }
  if (plans.ok) {
    setPlanStats({
      distinctPlans: plans.data.distinct_plans ?? 0,
      totalExecutions: plans.data.total_executions ?? 0,
      successRate: plans.data.success_rate ?? 0,
    });
  }
  if (coldStart.ok) {
    setColdStartStatus({
      onboardingComplete: Boolean(coldStart.data.onboarding_complete),
      granted: Array.isArray(coldStart.data.granted) ? coldStart.data.granted : [],
    });
  }

  const failures = [timeline, library, goalRows, reasoning, plans, coldStart]
    .filter((result) => !result.ok)
    .map((result) => result.ok ? "" : result.message);
  if (failures.length > 0) setLoadError(failures.join("; "));
  setLoading(false);
}

function scheduleRefresh(): void {
  if (refreshTimer) clearTimeout(refreshTimer);
  refreshTimer = setTimeout(() => {
    refreshTimer = null;
    void refreshProductionData();
  }, 250);
}

async function initialize(): Promise<void> {
  if (initialized) return;
  initialized = true;
  memorySubscriptions.push(
    eventBus.on("memory:updated", scheduleRefresh),
    eventBus.on("memory:deleted", scheduleRefresh),
  );
  await refreshProductionData();
}

function disposeRuntime(): void {
  for (const unsubscribe of memorySubscriptions.splice(0)) unsubscribe();
  if (refreshTimer) clearTimeout(refreshTimer);
  refreshTimer = null;
  initialized = false;
}

async function createGoal(title: string): Promise<MemoryActionResult<string>> {
  const normalized = title.trim();
  if (!normalized) return { ok: false, message: "Goal title cannot be empty" };
  const result = await bridgeInvoke<string>("memory_goal_create", { title: normalized });
  if (!result.ok) return { ok: false, message: result.message };
  await refreshProductionData();
  return { ok: true, data: result.data };
}

async function setGoalStatus(goalId: string, status: string): Promise<MemoryActionResult> {
  const result = await bridgeInvoke<void>("memory_goal_set_status", { goalId, status });
  if (!result.ok) return { ok: false, message: result.message };
  await refreshProductionData();
  return { ok: true, data: undefined };
}

async function setColdStartSource(source: string, granted: boolean): Promise<MemoryActionResult> {
  const result = await bridgeInvoke<void>("memory_cold_start_set", { source, granted });
  if (!result.ok) return { ok: false, message: result.message };
  await refreshProductionData();
  return { ok: true, data: undefined };
}

async function completeColdStart(): Promise<MemoryActionResult> {
  const result = await bridgeInvoke<void>("memory_cold_start_complete");
  if (!result.ok) return { ok: false, message: result.message };
  await refreshProductionData();
  return { ok: true, data: undefined };
}

async function previewColdStart(
  source: MemoryScanSource,
  root?: string,
  limit = 200,
): Promise<MemoryActionResult<MemoryScanCandidate[]>> {
  const result = await bridgeInvoke<{ candidates?: MemoryScanCandidate[] }>(
    "memory_cold_start_preview",
    { source, root: root?.trim() || undefined, limit },
    { timeoutMs: 30_000 },
  );
  return result.ok
    ? { ok: true, data: result.data.candidates ?? [] }
    : { ok: false, message: result.message };
}

async function importColdStart(
  source: MemoryScanSource,
  candidates: MemoryScanCandidate[],
): Promise<MemoryActionResult<number>> {
  const result = await bridgeInvoke<number>(
    "memory_cold_start_import",
    { source, candidates },
    { timeoutMs: 120_000 },
  );
  if (!result.ok) return { ok: false, message: result.message };
  await refreshProductionData();
  return { ok: true, data: result.data };
}

async function cancelColdStartImport(): Promise<MemoryActionResult<boolean>> {
  const result = await bridgeInvoke<boolean>("memory_cold_start_cancel");
  return result.ok
    ? { ok: true, data: result.data }
    : { ok: false, message: result.message };
}

async function queryReasoning(
  mode: MemoryReasoningQuery["mode"],
  rawQuery: string,
): Promise<MemoryActionResult<MemoryReasoningQuery>> {
  const query = rawQuery.trim();
  if (!query) return { ok: false, message: "Enter a task, cause, or effect to query." };
  setReasoningQueryBusy(true);
  setReasoningQueryError(null);

  let result: MemoryActionResult<MemoryReasoningQuery>;
  if (mode === "history") {
    const response = await bridgeInvoke<{ traces?: MemoryReasoningTrace[] }>(
      "memory_reasoning_history",
      { task: query, limit: 100 },
    );
    result = response.ok
      ? { ok: true, data: { mode, query, traces: response.data.traces ?? [] } }
      : { ok: false, message: response.message };
  } else if (mode === "chains") {
    const response = await bridgeInvoke<{ chains?: MemoryCausalChain[] }>(
      "memory_causal_chains",
      { start: query, maxDepth: 8 },
    );
    result = response.ok
      ? { ok: true, data: { mode, query, chains: response.data.chains ?? [] } }
      : { ok: false, message: response.message };
  } else {
    const command = mode === "effects" ? "memory_causal_effects_of" : "memory_causal_causes_of";
    const argument = mode === "effects" ? { cause: query } : { effect: query };
    const response = await bridgeInvoke<{ links?: MemoryCausalLink[] }>(command, argument);
    result = response.ok
      ? { ok: true, data: { mode, query, links: response.data.links ?? [] } }
      : { ok: false, message: response.message };
  }

  if (result.ok) setReasoningQuery(result.data);
  else setReasoningQueryError(result.message);
  setReasoningQueryBusy(false);
  return result;
}

// ─── Actions ───────────────────────────────────────────────────────────────────

function updateFact(factId: string, update: Partial<MemoryFact>): void {
  setFacts((prev) => prev.map((f) => (f.id === factId ? { ...f, ...update } : f)));
  eventBus.emit("memory:updated", { factId });
}

function deleteFact(factId: string): void {
  setFacts((prev) => prev.filter((f) => f.id !== factId));
  eventBus.emit("memory:deleted", { factId });
}

function addFact(fact: MemoryFact): void {
  setFacts((prev) => (prev.some((f) => f.id === fact.id) ? prev : [...prev, fact]));
  eventBus.emit("memory:updated", { factId: fact.id });
}

// ─── Memory-command action layer (Req 5.2 / 5.3) ─────────────────────────────
//
// Each action routes through an EXISTING memory_* command via the bridge. The
// bridge never throws (graceful degradation), so we translate its
// ServiceResult into a MemoryActionResult and let the caller report the outcome
// honestly. No action ever silently no-ops on failure.

/** Human-readable failure text from a non-ok bridge result. */
function failMessage(message: string, command: string): string {
  return message?.trim() ? message : `Memory command '${command}' failed`;
}

/**
 * Fetch the full Inspector detail for a memory (Req 5.2) via `memory_explain`.
 * Returns `{ ok: true, data: null }` when the memory no longer exists (honest
 * "not found"), or an error result when the command itself fails.
 */
async function fetchDetail(memoryId: string): Promise<MemoryActionResult<MemoryDetail | null>> {
  const res = await bridgeInvoke<MemoryExplainPayload | null>("memory_explain", { memoryId });
  if (!res.ok) return { ok: false, message: failMessage(res.message, "memory_explain") };
  return { ok: true, data: res.data ? normalizeDetail(res.data) : null };
}

/**
 * Verify a memory against its source (Req 5.3) via `memory_verify`. Returns the
 * boolean verdict. On a positive verdict we optimistically refresh the local
 * `updatedAt` so the card reflects the fresh check.
 */
async function verify(memoryId: string): Promise<MemoryActionResult<boolean>> {
  const res = await bridgeInvoke<boolean>("memory_verify", { memoryId });
  if (!res.ok) return { ok: false, message: failMessage(res.message, "memory_verify") };
  if (res.data) updateFact(memoryId, { updatedAt: Date.now() });
  return { ok: true, data: res.data };
}

/**
 * Correct a memory's content through the dedicated authority mutation. Backend
 * preserves stable identity, version history, FTS, vectors, and feedback in one
 * contract; local state updates only after durable success.
 */
async function correct(memoryId: string, newContent: string): Promise<MemoryActionResult> {
  const text = newContent.trim();
  if (!text) return { ok: false, message: "Correction cannot be empty" };
  const res = await bridgeInvoke<void>("memory_correct", { memoryId, content: text });
  if (!res.ok) return { ok: false, message: failMessage(res.message, "memory_correct") };
  updateFact(memoryId, { content: text, updatedAt: Date.now() });
  return { ok: true, data: undefined };
}

/** Reinforce a memory (Req 5.3) — a positive worth signal (`thumbs_up`). */
async function reinforce(memoryId: string): Promise<MemoryActionResult> {
  const res = await bridgeInvoke("memory_record_feedback", {
    targetId: memoryId,
    targetKind: "memory",
    signal: "thumbs_up",
  });
  if (!res.ok) return { ok: false, message: failMessage(res.message, "memory_record_feedback") };
  return { ok: true, data: undefined };
}

/** Penalize a memory (Req 5.3) — a negative worth signal (`thumbs_down`). */
async function penalize(memoryId: string): Promise<MemoryActionResult> {
  const res = await bridgeInvoke("memory_record_feedback", {
    targetId: memoryId,
    targetKind: "memory",
    signal: "thumbs_down",
  });
  if (!res.ok) return { ok: false, message: failMessage(res.message, "memory_record_feedback") };
  return { ok: true, data: undefined };
}

/**
 * Forget a memory (Req 5.3, REVERSIBLE) via `memory_forget` (scope
 * kind=`memory`). The runtime tombstones it. On success we optimistically drop
 * it from the local list and retain its stable id for one-shot undo.
 */
async function forget(memoryId: string): Promise<MemoryActionResult> {
  const fact = facts().find((f) => f.id === memoryId);
  const res = await bridgeInvoke<number>("memory_forget", { kind: "memory", value: memoryId });
  if (!res.ok) return { ok: false, message: failMessage(res.message, "memory_forget") };
  if (fact) setPendingUndo({ fact });
  deleteFact(memoryId);
  return { ok: true, data: undefined };
}

/** Restore the most recently forgotten memory with its original identity. */
async function undoForget(): Promise<MemoryActionResult> {
  const pending = pendingUndo();
  if (!pending) return { ok: false, message: "Nothing to undo" };
  const res = await bridgeInvoke<void>("memory_restore_forgotten", {
    memoryId: pending.fact.id,
  });
  if (!res.ok) {
    return { ok: false, message: failMessage(res.message, "memory_restore_forgotten") };
  }
  addFact(pending.fact);
  setPendingUndo(null);
  return { ok: true, data: undefined };
}

/** Discard the pending undo (user moved on). */
function clearUndo(): void {
  setPendingUndo(null);
}

/**
 * Hard-delete a memory (Req 5.3, IRREVERSIBLE) via `memory_hard_delete` (scope
 * kind=`memory`). No undo buffer — the deliberate confirmation gate lives in
 * the UI. On success it is dropped from the local list.
 */
async function hardDelete(memoryId: string): Promise<MemoryActionResult> {
  const res = await bridgeInvoke<number>("memory_hard_delete", { kind: "memory", value: memoryId });
  if (!res.ok) return { ok: false, message: failMessage(res.message, "memory_hard_delete") };
  deleteFact(memoryId);
  return { ok: true, data: undefined };
}

// ─── Cognition command layer (Req 5.6) ───────────────────────────────────────
//
// Each cognition job maps to an EXISTING memory_* command (KRIA runtime
// authority — the runtime runs the bounded cognition; the UI only triggers and
// reflects). We stage a `memory:cognition-started` event so the Core reflects
// the running state (reflecting/remembering/learning), run the command, then
// stage `memory:cognition-completed`. The command's RETURN payload is
// normalized into a persistent CognitionResult ("what changed"); a failure is
// stored as an honest failure result (never silently swallowed).

/** Max cognition results retained in the panel history. */
const MAX_COGNITION_RESULTS = 25;

/** Cognition job → existing memory_* Tauri command (design.md §6.2). */
const COGNITION_COMMAND: Readonly<Record<CognitionJob, string>> = {
  reflect: "memory_reflect",
  dream: "memory_run_dream",
  consolidate: "memory_consolidate",
  "active-learning": "memory_run_active_learning",
  "self-improvement": "memory_run_self_improvement",
  "entity-extraction": "memory_run_entity_extraction",
};

/** Human-readable job labels for result headings. */
export const COGNITION_LABEL: Readonly<Record<CognitionJob, string>> = {
  reflect: "Reflect",
  dream: "Dream",
  consolidate: "Consolidate",
  "active-learning": "Active learning",
  "self-improvement": "Self-improvement",
  "entity-extraction": "Entity extraction",
};

function asCount(data: unknown): number {
  return typeof data === "number" && Number.isFinite(data) ? data : 0;
}

function asField(data: unknown, key: string): number {
  if (data && typeof data === "object" && key in (data as Record<string, unknown>)) {
    const v = (data as Record<string, unknown>)[key];
    return typeof v === "number" && Number.isFinite(v) ? v : 0;
  }
  return 0;
}

function pluralize(n: number, singular: string, plural = `${singular}s`): string {
  return `${n} ${n === 1 ? singular : plural}`;
}

/**
 * Normalize a cognition command's return payload into a persistent
 * "what changed" result (Req 5.6). Each job exposes a different shape:
 *   • reflect → usize insights
 *   • dream → { procedures, goals_merged, worth_recalibrated }
 *   • consolidate → usize facts
 *   • active-learning → usize probes
 *   • self-improvement → usize proposals
 *   • entity-extraction → { processed, entities_linked }
 * The `summary` is a plain-language sentence generated from those counts (no
 * untrusted content — safe to render as text).
 */
export function normalizeCognitionResult(job: CognitionJob, data: unknown): CognitionResult {
  const base = { id: `cog-${job}-${Date.now()}`, job, at: Date.now(), ok: true as const, message: undefined };
  switch (job) {
    case "reflect": {
      const n = asCount(data);
      return { ...base, changes: [{ label: "Insights formed", value: n }], summary: `Reflection formed ${pluralize(n, "new insight")}.` };
    }
    case "dream": {
      const procedures = asField(data, "procedures");
      const goals = asField(data, "goals_merged");
      const worth = asField(data, "worth_recalibrated");
      return {
        ...base,
        changes: [
          { label: "Procedures distilled", value: procedures },
          { label: "Goals merged", value: goals },
          { label: "Worth recalibrated", value: worth },
        ],
        summary: `Dreaming distilled ${pluralize(procedures, "procedure")}, merged ${pluralize(goals, "goal")}, and recalibrated worth on ${pluralize(worth, "memory", "memories")}.`,
      };
    }
    case "consolidate": {
      const n = asCount(data);
      return { ...base, changes: [{ label: "Facts consolidated", value: n }], summary: `Consolidation merged ${pluralize(n, "fact")} from the session into long-term memory.` };
    }
    case "active-learning": {
      const n = asCount(data);
      return { ...base, changes: [{ label: "Knowledge gaps probed", value: n }], summary: `Active learning opened ${pluralize(n, "probe")} into detected knowledge gaps.` };
    }
    case "self-improvement": {
      const n = asCount(data);
      return { ...base, changes: [{ label: "Improvements proposed", value: n }], summary: `Self-improvement proposed ${pluralize(n, "improvement")}.` };
    }
    case "entity-extraction": {
      const processed = asField(data, "processed");
      const linked = asField(data, "entities_linked");
      return {
        ...base,
        changes: [
          { label: "Memories processed", value: processed },
          { label: "Entities linked", value: linked },
        ],
        summary: `Entity extraction processed ${pluralize(processed, "memory", "memories")} and linked ${pluralize(linked, "entity", "entities")}.`,
      };
    }
  }
}

/** True while the given cognition job is in-flight. */
function isCognitionRunning(job: CognitionJob): boolean {
  return cognitionRunning().includes(job);
}

/**
 * Trigger a cognition job (Req 5.6). Reflects the running state via the Core
 * (started/completed events), runs the EXISTING command, and records a
 * persistent result showing WHAT CHANGED. `consolidate` requires a session id
 * (passed by the caller as `args.sessionId`); the runtime validates it and any
 * failure surfaces as an honest failure result. Re-triggering an already-running
 * job is a no-op.
 */
async function runCognition(
  job: CognitionJob,
  args?: Record<string, unknown>,
): Promise<MemoryActionResult<CognitionResult>> {
  if (isCognitionRunning(job)) {
    return { ok: false, message: `${COGNITION_LABEL[job]} is already running` };
  }
  const command = COGNITION_COMMAND[job];
  setCognitionRunning((prev) => [...prev, job]);
  eventBus.emit("memory:cognition-started", { job });

  const res = await bridgeInvoke<unknown>(command, args);

  setCognitionRunning((prev) => prev.filter((j) => j !== job));
  eventBus.emit("memory:cognition-completed", { job, success: res.ok });

  if (!res.ok) {
    const failed: CognitionResult = {
      id: `cog-${job}-${Date.now()}`,
      job,
      at: Date.now(),
      ok: false,
      changes: [],
      summary: "",
      message: failMessage(res.message, command),
    };
    setCognitionResults((prev) => [failed, ...prev].slice(0, MAX_COGNITION_RESULTS));
    return { ok: false, message: failed.message! };
  }

  const result = normalizeCognitionResult(job, res.data);
  setCognitionResults((prev) => [result, ...prev].slice(0, MAX_COGNITION_RESULTS));
  return { ok: true, data: result };
}

/** Clear the cognition result history (panel "clear"). */
function clearCognitionResults(): void {
  setCognitionResults([]);
}

/** Seed cognition results directly (stories / future live-update reconcile). */
function seedCognitionResults(results: CognitionResult[]): void {
  setCognitionResults(results);
}

/** Seed the running-job set directly (stories / restore). */
function seedCognitionRunning(jobs: CognitionJob[]): void {
  setCognitionRunning(jobs);
}

// ─── Export ────────────────────────────────────────────────────────────────────

export const memoryStore = {
  facts,
  documents,
  goals,
  reasoningStats,
  planStats,
  coldStartStatus,
  activeSegment,
  searchQuery,
  loading,
  loadError,
  reasoningQuery,
  reasoningQueryBusy,
  reasoningQueryError,
  pendingUndo,
  cognitionRunning,
  cognitionResults,

  setFacts,
  setDocuments,
  setGoals,
  setActiveSegment,
  setSearchQuery,
  setLoading,
  initialize,
  disposeRuntime,
  refreshProductionData,
  createGoal,
  setGoalStatus,
  setColdStartSource,
  completeColdStart,
  previewColdStart,
  importColdStart,
  cancelColdStartImport,
  queryReasoning,
  updateFact,
  deleteFact,
  addFact,

  // Memory-command actions (Req 5.2 / 5.3)
  fetchDetail,
  verify,
  correct,
  reinforce,
  penalize,
  forget,
  undoForget,
  clearUndo,
  hardDelete,

  // Cognition (Req 5.6)
  isCognitionRunning,
  runCognition,
  clearCognitionResults,
  seedCognitionResults,
  seedCognitionRunning,
} as const;
