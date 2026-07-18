// Memory workspace store (memory-upgrade P3).
//
// The single reactive layer over the MemorySystem Tauri façade. Every memory
// capability the backend exposes is reachable here — no memory logic lives in
// the frontend, everything routes through `invoke`. Components consume the
// typed wrappers + reactive signals; they never call `invoke` directly.

import { createSignal } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

// ── Types (mirror the façade JSON shapes) ──────────────────────────────────

export interface MemoryHit {
  id: string;
  content: string;
  memory_type: string;
  namespace: string;
  confidence: number;
  importance: number;
  decay_score: number;
  access_count: number;
  state: string;
  created_at: string;
  score: number;
  strategies?: string[];
}

export interface SearchResult {
  query: string;
  results: MemoryHit[];
  count: number;
  trace?: RetrievalTrace;
}

export interface HealthReport {
  api_version: string;
  schema_version: number;
  embedder: string;
  event_count: number;
  memory_count: number;
  /** Durable enrichment backlog: committed events not yet enriched (R2 gauge). */
  pending_enrichment: number;
}

export interface ToolOutcomeStats {
  seen: number;
  persisted: number;
  gated: number;
}

export interface GoalAnalytics {
  candidate: number;
  active: number;
  paused: number;
  completed: number;
  failed: number;
  abandoned: number;
  total: number;
  completion_rate: number;
}

export interface PlanAnalyticsBlock {
  distinct_plans: number;
  total_executions: number;
  success_rate: number;
}

export interface Metrics {
  active_memories: number;
  unresolved_gaps: number;
  goals: GoalAnalytics;
  plans: PlanAnalyticsBlock;
  /** M5 tool-outcome write telemetry (seen / persisted / gated). */
  tool_outcomes: ToolOutcomeStats;
  summary: string;
}

export interface TimelineEntry {
  id: string;
  content: string;
  memory_type: string;
  confidence: number;
  created_at: string;
}

export interface MetaMemory {
  active: number;
  archived: number;
  superseded: number;
  avg_confidence: number;
  avg_worth: number;
}

export interface LibraryDoc {
  doc_id: string;
  title: string | null;
  path: string;
  version: number;
  chunks: number;
}

export interface Goal {
  id: string;
  kind: string;
  title: string;
  status: string;
  confidence: number;
  priority: number;
  parent_id: string | null;
  created_at: string;
  last_progress_at: string | null;
}

export interface PlanRecord {
  signature: string;
  task_label: string;
  steps: string[];
  success: number;
  failure: number;
  samples: number;
  worth: number;
  trusted: boolean;
}

export interface ReasoningAnalytics {
  chains: number;
  hypotheses: number;
  counterexamples: number;
  failed_chains: number;
  avg_confidence: number;
  hallucination_rate: number;
}

export interface ReasoningTrace {
  id: string;
  session_id: string | null;
  task_label: string;
  kind: string;
  content: string;
  confidence: number;
  success: boolean | null;
  created_at: string;
}

export interface CausalLink {
  cause: string;
  effect: string;
  observations: number;
  successes: number;
  confidence: number;
}

export interface CausalChain {
  path: string[];
  confidence: number;
}

export interface CentralityNode {
  entity: string;
  display_name: string;
  degree: number;
}

export interface GraphHitEntity {
  id: string;
  canonical_id: string;
  entity_type: string;
  display_name: string;
}

export interface GraphHit {
  entity: GraphHitEntity;
  distance: number;
  path: string[];
}

export interface Relationship {
  id: string;
  source_id: string;
  target_id: string;
  rel_type?: string;
  weight?: number;
}

export interface ColdStartStatus {
  onboarding_complete: boolean;
  granted: string[];
}

export interface ScanCandidate {
  source: string;
  path: string;
  detail: string;
}

export interface LinkPrediction {
  target: string;
  display_name: string;
  score: number;
  shared_neighbors: number;
}

export interface MemoryExplanation {
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

export interface LabeledCount {
  label: string;
  count: number;
}

export interface MemoryHealthReport {
  total_active: number;
  total_archived: number;
  total_superseded: number;
  total_forgotten: number;
  by_type: LabeledCount[];
  by_staleness: LabeledCount[];
  avg_confidence: number;
  unresolved_contradictions: number;
  knowledge_gaps: number;
  enrichment_backlog: number;
  outbox_pending: number;
}

export interface RetrievalTrace {
  query_class: string;
  vector_used: boolean;
  fts_used: boolean;
  candidates: number;
  returned: number;
}

export type EntityResolution =
  | { kind: "matched"; id: string }
  | { kind: "created"; id: string }
  | { kind: "proposed"; existing: string; created: string };

export type FeedbackSignalKind =
  | "thumbs_up"
  | "thumbs_down"
  | "correction"
  | "undo"
  | "cancel"
  | "edit"
  | "overwrite"
  | "ignored_suggestion"
  | "repeated_task"
  | "automation_success"
  | "automation_failure";

export type ForgetKind = "memory" | "source" | "session";
export type ScanSource = "filesystem" | "git" | "workspace" | "shell";

// ── Raw typed command wrappers (1:1 with the Tauri façade) ─────────────────

export const api = {
  search: (query: string, limit?: number) =>
    invoke<SearchResult>("memory_search", { query, limit }),
  recall: (query: string, limit?: number) =>
    invoke<SearchResult>("memory_recall", { query, limit }),
  reason: (query: string, limit?: number) =>
    invoke<SearchResult>("memory_reason", { query, limit }),
  health: () => invoke<HealthReport>("memory_health"),
  metrics: () => invoke<Metrics>("memory_metrics"),
  remember: (text: string) => invoke<{ decision: string }>("memory_remember", { text }),
  update: (winner: string, loser: string) =>
    invoke<void>("memory_update", { winner, loser }),
  verify: (memoryId: string) => invoke<boolean>("memory_verify", { memoryId }),
  forget: (kind: ForgetKind, value: string) =>
    invoke<number>("memory_forget", { kind, value }),
  hardDelete: (kind: ForgetKind, value: string) =>
    invoke<number>("memory_hard_delete", { kind, value }),
  resolveEntities: (
    displayName: string,
    entityType: string,
    alias: string,
    aliasType: string,
  ) =>
    invoke<EntityResolution>("memory_resolve_entities", {
      displayName,
      entityType,
      alias,
      aliasType,
    }),
  recordFeedback: (
    targetId: string,
    targetKind: string,
    signal: FeedbackSignalKind,
    detail?: string,
    context?: string,
  ) =>
    invoke<void>("memory_record_feedback", {
      targetId,
      targetKind,
      signal,
      detail,
      context,
    }),
  reflect: () => invoke<number>("memory_reflect"),
  consolidate: (sessionId: string) =>
    invoke<number>("memory_consolidate", { sessionId }),
  runDream: (maxProcedures?: number) =>
    invoke<{ procedures: number; goals_merged: number; worth_recalibrated: number }>(
      "memory_run_dream",
      { maxProcedures },
    ),
  runActiveLearning: (minMisses?: number, maxNew?: number) =>
    invoke<number>("memory_run_active_learning", { minMisses, maxNew }),
  runSelfImprovement: (maxNew?: number) =>
    invoke<number>("memory_run_self_improvement", { maxNew }),
  runEntityExtraction: (limit?: number) =>
    invoke<{ processed: number; entities_linked: number }>(
      "memory_run_entity_extraction",
      { limit },
    ),
  libraryList: () =>
    invoke<{ documents: LibraryDoc[]; count: number }>("memory_library_list"),
  libraryIngest: (path: string) =>
    invoke<{ doc_id: string; name: string; chunks: number; indexed: number }>(
      "memory_library_ingest",
      { path },
    ),
  libraryDelete: (docId: string) =>
    invoke<number>("memory_library_delete", { docId }),
  timeline: (limit?: number) =>
    invoke<{ entries: TimelineEntry[]; count: number }>("memory_timeline", { limit }),
  meta: () => invoke<MetaMemory>("memory_meta"),
  goalsList: (limit?: number) =>
    invoke<{ goals: Goal[]; count: number }>("memory_goals_list", { limit }),
  goalCreate: (title: string) => invoke<string>("memory_goal_create", { title }),
  goalSetStatus: (goalId: string, status: string) =>
    invoke<void>("memory_goal_set_status", { goalId, status }),
  plansAnalytics: () => invoke<PlanAnalyticsBlock>("memory_plans_analytics"),
  plansFor: (task: string) =>
    invoke<{ plans: PlanRecord[]; count: number }>("memory_plans_for", { task }),
  reasoningAnalytics: () =>
    invoke<ReasoningAnalytics>("memory_reasoning_analytics"),
  reasoningHistory: (task: string, limit?: number) =>
    invoke<{ traces: ReasoningTrace[]; count: number }>("memory_reasoning_history", {
      task,
      limit,
    }),
  causalEffectsOf: (cause: string) =>
    invoke<{ links: CausalLink[]; count: number }>("memory_causal_effects_of", { cause }),
  causalCausesOf: (effect: string) =>
    invoke<{ links: CausalLink[]; count: number }>("memory_causal_causes_of", { effect }),
  causalChains: (start: string, maxDepth?: number) =>
    invoke<{ chains: CausalChain[]; count: number }>("memory_causal_chains", {
      start,
      maxDepth,
    }),
  graphCentrality: (limit?: number) =>
    invoke<{ nodes: CentralityNode[]; count: number }>("memory_graph_centrality", {
      limit,
    }),
  graphCommunities: () =>
    invoke<{ communities: string[][]; count: number }>("memory_graph_communities"),
  graphNeighbors: (entityId: string, hops?: number) =>
    invoke<GraphHit[]>("memory_graph_neighbors", { entityId, hops }),
  graphRelationships: (entityId: string) =>
    invoke<Relationship[]>("memory_graph_relationships", { entityId }),
  graphSearch: (query: string) =>
    invoke<GraphHitEntity[]>("memory_graph_search", { query }),
  graphPredictLinks: (entityId: string, limit?: number) =>
    invoke<{ predictions: LinkPrediction[]; count: number }>("memory_graph_predict_links", {
      entityId,
      limit,
    }),
  graphCreateRelationship: (
    sourceId: string,
    targetId: string,
    relType: string,
    strength?: number,
  ) =>
    invoke<string>("memory_graph_create_relationship", {
      sourceId,
      targetId,
      relType,
      strength,
    }),
  explain: (memoryId: string) =>
    invoke<MemoryExplanation | null>("memory_explain", { memoryId }),
  healthReport: () => invoke<MemoryHealthReport>("memory_health_report"),
  reasoningReplay: (session: string) =>
    invoke<{ traces: ReasoningTrace[]; count: number }>("memory_reasoning_replay", {
      session,
    }),
  coldStartStatus: () => invoke<ColdStartStatus>("memory_cold_start_status"),
  coldStartSet: (source: ScanSource, granted: boolean) =>
    invoke<void>("memory_cold_start_set", { source, granted }),
  coldStartComplete: () => invoke<void>("memory_cold_start_complete"),
  backup: (dest: string) => invoke<number>("memory_backup", { dest }),
  restore: (src: string) => invoke<void>("memory_restore", { src }),
  coldStartPreview: (source: ScanSource, root?: string, limit?: number) =>
    invoke<{ candidates: ScanCandidate[]; count: number }>("memory_cold_start_preview", {
      source,
      root,
      limit,
    }),
  coldStartImport: (source: ScanSource, candidates: ScanCandidate[]) =>
    invoke<number>("memory_cold_start_import", { source, candidates }),
  coldStartCancel: () => invoke<boolean>("memory_cold_start_cancel"),
};

// ── Reactive layer (loading / error / cached lists / refresh) ──────────────

function makeResource<T>(initial: T) {
  const [data, setData] = createSignal<T>(initial);
  const [loading, setLoading] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  async function run(fn: () => Promise<T>) {
    setLoading(true);
    setError(null);
    try {
      const v = await fn();
      setData(() => v);
      return v;
    } catch (e) {
      setError(String(e));
      throw e;
    } finally {
      setLoading(false);
    }
  }
  return { data, setData, loading, error, run };
}

const health = makeResource<HealthReport | null>(null);
const metrics = makeResource<Metrics | null>(null);
const meta = makeResource<MetaMemory | null>(null);
const timeline = makeResource<TimelineEntry[]>([]);
const goals = makeResource<Goal[]>([]);
const library = makeResource<LibraryDoc[]>([]);
const searchResults = makeResource<MemoryHit[]>([]);
const graphNodes = makeResource<CentralityNode[]>([]);
const reasoningStats = makeResource<ReasoningAnalytics | null>(null);
const plansStats = makeResource<PlanAnalyticsBlock | null>(null);
const coldStart = makeResource<ColdStartStatus | null>(null);
const healthReport = makeResource<MemoryHealthReport | null>(null);

const [selectedMemory, setSelectedMemory] = createSignal<MemoryHit | null>(null);
const [lastSearchQuery, setLastSearchQuery] = createSignal("");
const [lastTrace, setLastTrace] = createSignal<RetrievalTrace | null>(null);
const [liveActive, setLiveActive] = createSignal(false);
const [liveEventCount, setLiveEventCount] = createSignal(0);

// ── Live event bridge (P8): backend `memory://changed` → reactive refresh ──
let liveUnlisten: UnlistenFn | null = null;
let pendingKinds = new Set<string>();
let liveTimer: ReturnType<typeof setTimeout> | null = null;

/// Browser-side fan-out so components (e.g. the graph) can react to a live
/// memory change without each re-subscribing to the Tauri channel.
export const LIVE_EVENT = "kria-memory-live";

export const memoryStore = {
  // resources
  health,
  metrics,
  meta,
  timeline,
  goals,
  library,
  searchResults,
  graphNodes,
  reasoningStats,
  plansStats,
  coldStart,
  healthReport,
  selectedMemory,
  setSelectedMemory,
  lastSearchQuery,
  lastTrace,
  liveActive,
  liveEventCount,

  // raw api (for on-demand queries not held in a cached signal)
  api,

  // ── high-level actions with reactive state ──
  async doSearch(query: string, limit = 30) {
    setLastSearchQuery(query);
    const res = await searchResults.run(async () => {
      const r = await api.search(query, limit);
      setLastTrace(r.trace ?? null);
      return r.results;
    });
    return res;
  },

  async refreshHealth() {
    return health.run(() => api.health());
  },
  async refreshMetrics() {
    return metrics.run(() => api.metrics());
  },
  async refreshMeta() {
    return meta.run(() => api.meta());
  },
  async refreshTimeline(limit = 200) {
    return timeline.run(async () => (await api.timeline(limit)).entries);
  },
  async refreshGoals(limit = 100) {
    return goals.run(async () => (await api.goalsList(limit)).goals);
  },
  async refreshLibrary() {
    return library.run(async () => (await api.libraryList()).documents);
  },
  async refreshGraph(limit = 50) {
    return graphNodes.run(async () => (await api.graphCentrality(limit)).nodes);
  },
  async refreshReasoning() {
    return reasoningStats.run(() => api.reasoningAnalytics());
  },
  async refreshPlans() {
    return plansStats.run(() => api.plansAnalytics());
  },
  async refreshColdStart() {
    return coldStart.run(() => api.coldStartStatus());
  },
  async refreshHealthReport() {
    return healthReport.run(() => api.healthReport());
  },

  // CRUD-ish operations that invalidate the relevant caches
  async remember(text: string) {
    const r = await api.remember(text);
    await Promise.all([this.refreshTimeline(), this.refreshMetrics()]);
    return r;
  },
  async forget(kind: ForgetKind, value: string) {
    const n = await api.forget(kind, value);
    await Promise.all([this.refreshTimeline(), this.refreshMetrics()]);
    return n;
  },
  async hardDelete(kind: ForgetKind, value: string) {
    const n = await api.hardDelete(kind, value);
    await Promise.all([this.refreshTimeline(), this.refreshMetrics()]);
    return n;
  },
  async createGoal(title: string) {
    const id = await api.goalCreate(title);
    await this.refreshGoals();
    return id;
  },
  async setGoalStatus(goalId: string, status: string) {
    await api.goalSetStatus(goalId, status);
    await this.refreshGoals();
  },
  async ingestDocument(path: string) {
    const r = await api.libraryIngest(path);
    await Promise.all([this.refreshLibrary(), this.refreshMetrics()]);
    return r;
  },
  async deleteDocument(docId: string) {
    const n = await api.libraryDelete(docId);
    await this.refreshLibrary();
    return n;
  },
  async recordFeedback(
    targetId: string,
    targetKind: string,
    signal: FeedbackSignalKind,
    detail?: string,
  ) {
    await api.recordFeedback(targetId, targetKind, signal, detail);
  },

  // Background cognition triggers (return counts for toast feedback)
  runReflect: () => api.reflect(),
  runDream: () => api.runDream(),
  runActiveLearning: () => api.runActiveLearning(),
  runSelfImprovement: () => api.runSelfImprovement(),
  runEntityExtraction: () => api.runEntityExtraction(),

  // ── Live updates (P8): subscribe once; refreshes flow from backend events ──
  async subscribeLive() {
    if (liveUnlisten) return;
    liveUnlisten = await listen<{ kind: string; detail?: unknown }>(
      "memory://changed",
      (e) => {
        setLiveEventCount((n) => n + 1);
        pendingKinds.add(e.payload?.kind ?? "created");
        if (liveTimer) clearTimeout(liveTimer);
        liveTimer = setTimeout(() => flushLive(), 400);
      },
    );
    setLiveActive(true);
  },
  unsubscribeLive() {
    if (liveUnlisten) {
      liveUnlisten();
      liveUnlisten = null;
    }
    setLiveActive(false);
  },

  // Load everything for first workspace open
  async refreshAll() {
    await Promise.allSettled([
      this.refreshHealth(),
      this.refreshMetrics(),
      this.refreshMeta(),
      this.refreshTimeline(),
      this.refreshGoals(),
      this.refreshLibrary(),
      this.refreshGraph(),
      this.refreshReasoning(),
      this.refreshPlans(),
      this.refreshColdStart(),
      this.refreshHealthReport(),
    ]);
  },
};

// Coalesced live-refresh: one pass per burst of backend change events. Each
// refresh is fire-and-forget and swallows transient errors (the resource's own
// `error()` signal still records them) so a flaky refresh never crashes the app.
function flushLive() {
  const kinds = pendingKinds;
  pendingKinds = new Set();
  const swallow = (p: Promise<unknown>) => {
    void p.catch(() => undefined);
  };
  // Broadly-affected, cheap resources always refresh.
  swallow(memoryStore.refreshMetrics());
  swallow(memoryStore.refreshHealth());
  swallow(memoryStore.refreshHealthReport());
  if (kinds.has("created") || kinds.has("deleted") || kinds.has("updated")) {
    swallow(memoryStore.refreshTimeline());
    swallow(memoryStore.refreshMeta());
  }
  if (kinds.has("goal")) swallow(memoryStore.refreshGoals());
  if (kinds.has("library")) swallow(memoryStore.refreshLibrary());
  if (kinds.has("reflection") || kinds.has("dream")) swallow(memoryStore.refreshReasoning());
  if (
    kinds.has("relationship") ||
    kinds.has("entity") ||
    kinds.has("created") ||
    kinds.has("deleted")
  ) {
    swallow(memoryStore.refreshGraph());
  }
  // Fan out to components (e.g. the live graph) that react locally.
  if (typeof window !== "undefined") {
    window.dispatchEvent(new CustomEvent(LIVE_EVENT, { detail: { kinds: [...kinds] } }));
  }
}
