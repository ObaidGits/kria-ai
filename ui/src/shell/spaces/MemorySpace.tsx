/**
 * Memory Space — landing + segment scaffolding (task 6.1, Req 5.1).
 *
 * Provides the Memory landing (overview / recent / gaps / search) as the
 * default view and the lens/segment navigation for the eight lenses:
 * Explorer, Timeline, Goals & Plans, Reasoning & Causal, Library, Knowledge
 * Graph, Cognition, Cold Start.
 *
 * Segment navigation is a real tablist (kit `Tabs`, Kobalte-backed → correct
 * tablist/tab/tabpanel roles + arrow-key nav, Req 17.1/17.2) whose selection is
 * driven by the typed router: `space=memory, segment=<id>` (Req 1.3/1.5). The
 * landing is the default when no segment is routed. Later tasks fill each
 * region:
 *   • MemoryCard + Inspector detail ............... task 6.2
 *   • Cognition controls + result panel ........... task 6.3
 *   • Memory Graph: v2 Knowledge destination with list-first representation
 *     (`GraphCanvas3D` remains dormant pending MGR-030 Phase 7 / F6)
 * Here each segment is a labelled region with a heading and either a basic list
 * (where the store already holds data) or an honest placeholder.
 *
 * Pure presentation / read-model: reads `memoryStore` only (fed by the memory
 * bridge). No orchestration, no memory mutations (verify/correct/forget are
 * task 6.2) — KRIA runtime-authority invariant. Fact content is rendered as
 * text (Solid auto-escapes), never as HTML, so untrusted memory content cannot
 * inject markup.
 *
 * Requirements: 5.1
 */
import { createEffect, createMemo, createSignal, For, onCleanup, onMount, Show } from "solid-js";
import {
  createVirtualizer,
  observeElementRect,
  type Rect,
  type Virtualizer,
} from "@tanstack/solid-virtual";
import { bridgeInvoke } from "../../bridge/invoke";
import { memoryStore, notificationStore, shellStore, type MemorySegment } from "../../stores";
import { currentRoute, navigate } from "../router";
import { Badge, Button, Card, EmptyState, Input, Search, Select, Tabs } from "../../kit";
import { Icon } from "../../components/Icon";
import MemoryOnboarding from "../../components/memory/MemoryOnboarding";
import { MemoryCard } from "./memory/MemoryCard";
import { CognitionPanel } from "./memory/CognitionPanel";
import { registerMemoryInspector } from "./memory/registerMemoryInspector";
import { Knowledge } from "./memory/destinations/Knowledge";
import {
  parseKnowledgeProjectionResponse,
  type KnowledgeProjectionItem,
  type KnowledgeProjectionResponse,
} from "./memory/api";
import { buildSemanticScene, type RawSceneItem } from "./memory/scene/sceneBuilder";
import { buildLayoutHint } from "./memory/scene/sceneLayout";
import { MemoryWindowSessionV2 } from "./memory/state/windowSession";
import "./MemorySpace.css";

// ─── Segment model ───────────────────────────────────────────────────────────

interface SegmentDef {
  value: MemorySegment;
  label: string;
}

/** Landing (default overview) + the eight lenses (Req 5.1). */
const SEGMENTS: readonly SegmentDef[] = [
  { value: "landing", label: "Overview" },
  { value: "explorer", label: "Explorer" },
  { value: "timeline", label: "Timeline" },
  { value: "goals", label: "Goals & Plans" },
  { value: "reasoning", label: "Reasoning & Causal" },
  { value: "library", label: "Library" },
  { value: "knowledgegraph", label: "Knowledge Graph" },
  { value: "cognition", label: "Cognition" },
  { value: "coldstart", label: "Cold Start" },
] as const;

function isMemorySegment(value: string | undefined): value is MemorySegment {
  return !!value && SEGMENTS.some((s) => s.value === value);
}

// ─── Space ─────────────────────────────────────────────────────────────────────

/** Resolve the routed segment, defaulting to the landing (Req 1.5). */
function routedSegment(): MemorySegment {
  const seg = currentRoute().segment;
  return isMemorySegment(seg) ? seg : "landing";
}

export default function MemorySpace() {
  // Seed the tablist from the route at mount so a deep link (e.g.
  // `memory/timeline`) opens the right segment (Req 1.5 deep-linkable). The
  // tablist then owns selection + arrow-key nav (Kobalte); each switch routes
  // via the typed router below, keeping the route the single address for the
  // active segment.
  const initialSegment = routedSegment();
  const isMini = createMemo(() => shellStore.windowMode() === "mini");

  // Mirror the routed segment into the store so downstream lens tasks (6.2–6.5)
  // read a single source of truth.
  createEffect(() => memoryStore.setActiveSegment(routedSegment()));

  // Honor a deep-linked memory id (Req 5.7): when the route carries an entityId
  // under the Memory Space — e.g. from Converse's "why did KRIA answer this"
  // (memory/explorer/<id>) or a restored session — open the shared Inspector on
  // that memory so the detail lands open. Navigation-only; MemoryInspector
  // fetches the detail by id, so a fact payload is passed only when already
  // loaded (for an immediate preview).
  //
  // We open at most once per distinct entityId (tracked in `lastDeepLinkId`) so
  // the user can freely CLOSE the Inspector afterwards without it snapping back
  // open. The eager mount-route capture covers a fresh deep link (the tablist's
  // onChange re-navigates and clears the entityId before an effect would run,
  // so the mount case must be captured explicitly); the effect covers a later
  // deep-link into an already-open Memory Space.
  let lastDeepLinkId: string | null = null;
  function openDeepLinkedMemory(id: string): void {
    if (lastDeepLinkId === id) return;
    lastDeepLinkId = id;
    const fact = memoryStore.facts().find((f) => f.id === id);
    // Programmatic (route/deep-link) open: activeElement is not the semantic
    // control, so hand the stable Memory region as the Focus_Return_Owner
    // (§20.3/§20.4) — close returns focus to the region, not a stray element.
    shellStore.openInspector("memory", id, fact, {
      regionSelector: '[data-space="memory"]',
    });
  }

  const mountRoute = currentRoute();
  if (mountRoute.space === "memory" && mountRoute.entityId) {
    openDeepLinkedMemory(mountRoute.entityId);
  }

  createEffect(() => {
    const route = currentRoute();
    if (route.space !== "memory") return;
    const id = route.entityId;
    if (!id) return;
    openDeepLinkedMemory(id);
  });

  // Register the "memory" Inspector body (task 6.2) so selecting a MemoryCard
  // opens the full detail in the single shared Inspector (Req 1.6 / 5.2). The
  // disposer unregisters on unmount / hot-reload.
  onCleanup(registerMemoryInspector());

  // Segment switch routes via the typed router (space=memory, segment=…).
  function selectSegment(value: string) {
    if (value === "landing") navigate("memory");
    else navigate("memory", value);
  }

  const items = SEGMENTS.map((seg) => ({
    value: seg.value,
    label: seg.label,
    content: () => <SegmentRegion segment={seg.value} label={seg.label} />,
  }));

  return (
    <section class="kria-memory" data-space="memory" aria-label="Memory">
      <header class="kria-memory__header">
        <h1 class="kria-memory__title">Memory</h1>
        <div class="kria-memory__search">
          <Search
            label="Search memory"
            placeholder="Search what KRIA knows…"
            value={memoryStore.searchQuery()}
            onChange={memoryStore.setSearchQuery}
          />
        </div>
      </header>

      <Show
        when={isMini()}
        fallback={
          <Tabs
            class="kria-memory__segments"
            items={items}
            defaultValue={initialSegment}
            onChange={selectSegment}
          />
        }
      >
        <div class="kria-memory__compact" data-curated-primary="search-peek">
          <ExplorerRegion />
        </div>
      </Show>
    </section>
  );
}

// ─── Regions ─────────────────────────────────────────────────────────────────

function SegmentRegion(props: { segment: MemorySegment; label: string }) {
  return (
    <div
      class="kria-memory__region"
      data-segment={props.segment}
      aria-label={props.label}
    >
      <Show when={props.segment === "landing"}>
        <LandingRegion />
      </Show>
      <Show when={props.segment === "explorer"}>
        <ExplorerRegion />
      </Show>
      <Show when={props.segment === "timeline"}>
        <TimelineRegion />
      </Show>
      <Show when={props.segment === "library"}>
        <LibraryRegion />
      </Show>
      <Show when={props.segment === "goals"}>
        <GoalsRegion />
      </Show>
      <Show when={props.segment === "reasoning"}>
        <ReasoningRegion />
      </Show>
      <Show when={props.segment === "knowledgegraph"}>
        <KnowledgeRegion />
      </Show>
      <Show when={props.segment === "cognition"}>
        <CognitionPanel />
      </Show>
      <Show when={props.segment === "coldstart"}>
        <ColdStartRegion />
      </Show>
    </div>
  );
}

/** Loading / empty helpers shared by data-backed regions (honest states). */
function LoadingRow(props: { label: string }) {
  return (
    <div class="kria-memory__status" role="status" aria-live="polite">
      {props.label}
    </div>
  );
}

function LandingRegion() {
  const factCount = createMemo(() => memoryStore.facts().length);
  const docCount = createMemo(() => memoryStore.documents().length);
  // Recent = most-recently updated facts (top 5).
  const recent = createMemo(() =>
    [...memoryStore.facts()].sort((a, b) => b.updatedAt - a.updatedAt).slice(0, 5),
  );
  const isEmpty = createMemo(() => factCount() === 0 && docCount() === 0);

  return (
    <div class="kria-memory__landing">
      <h2 class="kria-memory__region-title">Overview</h2>

      <Show when={memoryStore.loading()}>
        <LoadingRow label="Loading memory…" />
      </Show>

      <Show
        when={!memoryStore.loading() && isEmpty()}
        fallback={
          <>
            <div class="kria-memory__stats" aria-label="Memory overview">
              <StatCard label="Facts" value={factCount()} />
              <StatCard label="Documents" value={docCount()} />
            </div>

            <section class="kria-memory__panel" aria-label="Recent">
              <h3 class="kria-memory__panel-title">Recent</h3>
              <Show
                when={recent().length > 0}
                fallback={<p class="kria-memory__muted">No recent memories yet.</p>}
              >
                <ul class="kria-memory__recent">
                  <For each={recent()}>
                    {(fact) => (
                      <li class="kria-memory__recent-item" data-fact-id={fact.id}>
                        {fact.content}
                      </li>
                    )}
                  </For>
                </ul>
              </Show>
            </section>

            <section class="kria-memory__panel" aria-label="Gaps">
              <h3 class="kria-memory__panel-title">Gaps</h3>
              <p class="kria-memory__muted">
                No knowledge gaps detected. Gap analysis is surfaced by the
                Cognition lens as it runs.
              </p>
            </section>
          </>
        }
      >
        <EmptyState
          icon="brain"
          title="No memories yet"
          description="As KRIA works, what it learns will appear here. Use the search above or the Explorer lens to browse memories."
        />
      </Show>
    </div>
  );
}

function StatCard(props: { label: string; value: number }) {
  return (
    <Card class="kria-memory__stat">
      <span class="kria-memory__stat-value">{props.value}</span>
      <span class="kria-memory__stat-label">{props.label}</span>
    </Card>
  );
}

const MEMORY_VIRTUAL_RECT = { width: 720, height: 560 } as const;

function observeMemoryViewport<
  TScrollElement extends Element,
  TItemElement extends Element,
>(
  instance: Virtualizer<TScrollElement, TItemElement>,
  callback: (rect: Rect) => void,
) {
  return observeElementRect(instance, (rect) => {
    callback(rect.width > 0 && rect.height > 0 ? rect : MEMORY_VIRTUAL_RECT);
  });
}

function ExplorerRegion() {
  const [scrollEl, setScrollEl] = createSignal<HTMLDivElement>();
  const filtered = createMemo(() => {
    const q = memoryStore.searchQuery().trim().toLowerCase();
    const all = memoryStore.facts();
    if (!q) return all;
    return all.filter(
      (f) =>
        f.content.toLowerCase().includes(q) ||
        f.source.toLowerCase().includes(q) ||
        f.tags.some((t) => t.toLowerCase().includes(q)),
    );
  });
  const total = createMemo(() => memoryStore.facts().length);
  const virtualizer = createVirtualizer({
    get count() { return filtered().length; },
    getScrollElement: () => scrollEl() ?? null,
    observeElementRect: observeMemoryViewport,
    estimateSize: () => 132,
    overscan: 5,
    getItemKey: (index) => filtered()[index]?.id ?? index,
    initialRect: MEMORY_VIRTUAL_RECT,
  });

  return (
    <div class="kria-memory__explorer">
      <h2 class="kria-memory__region-title">Explorer</h2>

      <Show when={memoryStore.loading()}>
        <LoadingRow label="Loading memories…" />
      </Show>

      <Show
        when={!memoryStore.loading() && total() > 0}
        fallback={
          <Show when={!memoryStore.loading()}>
            <EmptyState
              icon="brain"
              title="No memories to explore"
              description="KRIA hasn't stored any facts yet. They'll appear here as it learns."
            />
          </Show>
        }
      >
        <UndoBar />
        <p class="kria-memory__count">
          Showing {filtered().length} of {total()}
        </p>
        <Show
          when={filtered().length > 0}
          fallback={<p class="kria-memory__muted">No memories match your search.</p>}
        >
          <div ref={(el) => setScrollEl(el)} class="kria-memory__virtual-viewport" data-virtual-list="memory-explorer">
            <ul
              class="kria-memory__cards kria-memory__virtual-sizer"
              style={{ height: `${virtualizer.getTotalSize()}px` }}
            >
              <For each={virtualizer.getVirtualItems()}>
                {(row) => {
                  const fact = () => filtered()[row.index];
                  return (
                    <Show when={fact()}>
                      <li
                        class="kria-memory__card-item kria-memory__virtual-row"
                        data-fact-id={fact()!.id}
                        data-index={row.index}
                        ref={(el) => queueMicrotask(() => virtualizer.measureElement(el))}
                        style={{ transform: `translateY(${row.start}px)` }}
                      >
                        <MemoryCard fact={fact()!} selected={activeMemoryId() === fact()!.id} />
                      </li>
                    </Show>
                  );
                }}
              </For>
            </ul>
          </div>
        </Show>
      </Show>
    </div>
  );
}

/** The id of the memory currently open in the shared Inspector, if any. */
function activeMemoryId(): string | null {
  const t = shellStore.inspectorTarget();
  return t && t.type === "memory" ? t.id : null;
}

/**
 * Space-level Undo affordance for the last reversible `forget` (Req 5.3). Reads
 * the memoryStore undo buffer so Undo survives closing the Inspector. On undo
 * it re-admits the memory through the existing memory_remember command and
 * surfaces the outcome honestly (never a silent no-op).
 */
function UndoBar() {
  async function undo() {
    const res = await memoryStore.undoForget();
    notificationStore.push({
      id: `mem-undo-${Date.now()}`,
      level: res.ok ? "success" : "error",
      message: res.ok ? "Memory restored" : res.message,
      source: "memory",
    });
  }
  return (
    <Show when={memoryStore.pendingUndo()}>
      <div class="kria-memory__undo" role="status" aria-live="polite">
        <span>
          <Icon name="eye-off" size={14} aria-hidden /> A memory was forgotten.
        </span>
        <Button variant="secondary" size="sm" onClick={() => void undo()}>
          <Icon name="rotate-ccw" size={14} /> Undo
        </Button>
      </div>
    </Show>
  );
}

function TimelineRegion() {
  const [scrollEl, setScrollEl] = createSignal<HTMLDivElement>();
  const chronological = createMemo(() =>
    [...memoryStore.facts()].sort((a, b) => b.createdAt - a.createdAt),
  );
  const virtualizer = createVirtualizer({
    get count() { return chronological().length; },
    getScrollElement: () => scrollEl() ?? null,
    observeElementRect: observeMemoryViewport,
    estimateSize: () => 72,
    overscan: 6,
    getItemKey: (index) => chronological()[index]?.id ?? index,
    initialRect: MEMORY_VIRTUAL_RECT,
  });

  return (
    <div class="kria-memory__timeline">
      <h2 class="kria-memory__region-title">Timeline</h2>

      <Show when={memoryStore.loading()}>
        <LoadingRow label="Loading timeline…" />
      </Show>

      <Show
        when={!memoryStore.loading() && chronological().length > 0}
        fallback={
          <Show when={!memoryStore.loading()}>
            <EmptyState
              icon="clock"
              title="Nothing on the timeline yet"
              description="Memories will appear here in the order KRIA learned them."
            />
          </Show>
        }
      >
        <div ref={(el) => setScrollEl(el)} class="kria-memory__virtual-viewport" data-virtual-list="memory-timeline">
          <ol
            class="kria-memory__timeline-list kria-memory__virtual-sizer"
            style={{ height: `${virtualizer.getTotalSize()}px` }}
          >
            <For each={virtualizer.getVirtualItems()}>
              {(row) => {
                const fact = () => chronological()[row.index];
                return (
                  <Show when={fact()}>
                    <li
                      class="kria-memory__timeline-item kria-memory__virtual-row"
                      data-fact-id={fact()!.id}
                      data-index={row.index}
                      ref={(el) => queueMicrotask(() => virtualizer.measureElement(el))}
                      style={{ transform: `translateY(${row.start}px)` }}
                    >
                      <span class="kria-memory__timeline-when">
                        {new Date(fact()!.createdAt).toLocaleString()}
                      </span>
                      <span class="kria-memory__timeline-what">{fact()!.content}</span>
                    </li>
                  </Show>
                );
              }}
            </For>
          </ol>
        </div>
      </Show>
    </div>
  );
}

function LibraryRegion() {
  const docs = createMemo(() => memoryStore.documents());
  return (
    <div class="kria-memory__library">
      <h2 class="kria-memory__region-title">Library</h2>

      <Show when={memoryStore.loading()}>
        <LoadingRow label="Loading library…" />
      </Show>

      <Show
        when={!memoryStore.loading() && docs().length > 0}
        fallback={
          <Show when={!memoryStore.loading()}>
            <EmptyState
              icon="folder"
              title="No documents indexed"
              description="Documents KRIA has indexed for retrieval will appear here."
            />
          </Show>
        }
      >
        <ul class="kria-memory__cards">
          <For each={docs()}>
            {(doc) => (
              <li class="kria-memory__card-item">
                <Card class="kria-memory__card" aria-label={doc.title}>
                  <p class="kria-memory__card-content" data-doc-id={doc.id}>
                    {doc.title}
                  </p>
                  <div class="kria-memory__card-meta">
                    <Badge tone="neutral">{doc.type}</Badge>
                  </div>
                </Card>
              </li>
            )}
          </For>
        </ul>
      </Show>
    </div>
  );
}

function GoalsRegion() {
  const [draft, setDraft] = createSignal("");

  async function create() {
    const result = await memoryStore.createGoal(draft());
    notificationStore.push({
      id: `memory-goal-${Date.now()}`,
      level: result.ok ? "success" : "error",
      message: result.ok ? "Goal created" : result.message,
      source: "memory",
    });
    if (result.ok) setDraft("");
  }

  async function updateStatus(goalId: string, status: string) {
    const result = await memoryStore.setGoalStatus(goalId, status);
    notificationStore.push({
      id: `memory-goal-status-${Date.now()}`,
      level: result.ok ? "success" : "error",
      message: result.ok ? `Goal marked ${status}` : result.message,
      source: "memory",
    });
  }

  return (
    <div class="kria-memory__goals">
      <h2 class="kria-memory__region-title">Goals & Plans</h2>
      <div class="kria-memory__panel">
        <label for="memory-new-goal">Create goal</label>
        <div class="kria-memory__goal-create">
          <input
            id="memory-new-goal"
            value={draft()}
            onInput={(event) => setDraft(event.currentTarget.value)}
            placeholder="What should KRIA work toward?"
          />
          <Button onClick={() => void create()} disabled={!draft().trim()}>Create</Button>
        </div>
      </div>
      <div class="kria-memory__stats" aria-label="Plan performance">
        <StatCard label="Plans" value={memoryStore.planStats()?.distinctPlans ?? 0} />
        <StatCard label="Executions" value={memoryStore.planStats()?.totalExecutions ?? 0} />
        <StatCard label="Success %" value={Math.round((memoryStore.planStats()?.successRate ?? 0) * 100)} />
      </div>
      <Show
        when={memoryStore.goals().length > 0}
        fallback={<EmptyState icon="star" title="No goals yet" description="Create a goal to begin planning." />}
      >
        <ul class="kria-memory__cards">
          <For each={memoryStore.goals()}>
            {(goal) => (
              <li class="kria-memory__card-item">
                <Card class="kria-memory__card">
                  <p class="kria-memory__card-content">{goal.title}</p>
                  <div class="kria-memory__card-meta">
                    <Badge tone="neutral">{goal.status}</Badge>
                    <span>{Math.round(goal.confidence * 100)}% confidence</span>
                  </div>
                  <div class="kria-memory__goal-actions">
                    <Button size="sm" variant="secondary" onClick={() => void updateStatus(goal.id, "active")}>Activate</Button>
                    <Button size="sm" variant="secondary" onClick={() => void updateStatus(goal.id, "paused")}>Pause</Button>
                    <Button size="sm" variant="secondary" onClick={() => void updateStatus(goal.id, "completed")}>Complete</Button>
                  </div>
                </Card>
              </li>
            )}
          </For>
        </ul>
      </Show>
    </div>
  );
}

function ReasoningRegion() {
  const stats = () => memoryStore.reasoningStats();
  const [mode, setMode] = createSignal<"history" | "effects" | "causes" | "chains">("history");
  const [query, setQuery] = createSignal("");

  const submit = (event: SubmitEvent) => {
    event.preventDefault();
    void memoryStore.queryReasoning(mode(), query()).then((result) => {
      if (!result.ok) {
        notificationStore.push({
          id: `memory-reasoning-${Date.now()}`,
          level: "error",
          message: result.message,
        });
      }
    });
  };

  return (
    <div class="kria-memory__reasoning">
      <h2 class="kria-memory__region-title">Reasoning & Causal</h2>
      <Show
        when={stats()}
        fallback={<EmptyState icon="git-branch" title="No reasoning history yet" description="Reasoning and causal evidence appears after KRIA completes analytical work." />}
      >
        <div class="kria-memory__stats" aria-label="Reasoning quality">
          <StatCard label="Chains" value={stats()!.chains} />
          <StatCard label="Hypotheses" value={stats()!.hypotheses} />
          <StatCard label="Counterexamples" value={stats()!.counterexamples} />
          <StatCard label="Failed" value={stats()!.failedChains} />
          <StatCard label="Confidence %" value={Math.round(stats()!.averageConfidence * 100)} />
          <StatCard label="Hallucination %" value={Math.round(stats()!.hallucinationRate * 100)} />
        </div>
      </Show>

      <form class="kria-memory__reasoning-query" onSubmit={submit}>
        <Select
          label="Reasoning query type"
          value={mode()}
          options={[
            { value: "history", label: "Reasoning history" },
            { value: "effects", label: "Effects of cause" },
            { value: "causes", label: "Causes of effect" },
            { value: "chains", label: "Causal chains" },
          ]}
          onChange={(value) => value && setMode(value as "history" | "effects" | "causes" | "chains")}
        />
        <Input
          label={mode() === "history" ? "Task" : mode() === "causes" ? "Effect" : "Cause or starting point"}
          value={query()}
          onChange={setQuery}
          errorMessage={memoryStore.reasoningQueryError() ?? undefined}
        />
        <Button type="submit" disabled={memoryStore.reasoningQueryBusy() || !query().trim()}>
          {memoryStore.reasoningQueryBusy() ? "Querying…" : "Query memory"}
        </Button>
      </form>

      <Show when={memoryStore.reasoningQuery()}>
        {(result) => (
          <div class="kria-memory__reasoning-results" aria-live="polite">
            <h3>Results for “{result().query}”</h3>
            <Show when={result().mode === "history"}>
              <For each={result().mode === "history" ? result().traces ?? [] : []} fallback={<p class="kria-memory__muted">No matching reasoning traces.</p>}>
                {(trace) => (
                  <Card class="kria-memory__reasoning-result">
                    <strong>{trace.task}</strong>
                    <Show when={trace.approach}><p>{trace.approach}</p></Show>
                    <Show when={trace.outcome}><p>{trace.outcome}</p></Show>
                    <Show when={typeof trace.confidence === "number"}><Badge>{Math.round((trace.confidence ?? 0) * 100)}% confidence</Badge></Show>
                  </Card>
                )}
              </For>
            </Show>
            <Show when={result().mode === "effects" || result().mode === "causes"}>
              <For each={result().mode === "effects" || result().mode === "causes" ? result().links ?? [] : []} fallback={<p class="kria-memory__muted">No matching causal links.</p>}>
                {(link) => (
                  <Card class="kria-memory__reasoning-result">
                    <strong>{link.cause}</strong><span aria-hidden="true"> → </span><strong>{link.effect}</strong>
                    <Show when={typeof link.strength === "number"}><Badge>{Math.round((link.strength ?? 0) * 100)}% strength</Badge></Show>
                  </Card>
                )}
              </For>
            </Show>
            <Show when={result().mode === "chains"}>
              <For each={result().mode === "chains" ? result().chains ?? [] : []} fallback={<p class="kria-memory__muted">No causal chains found.</p>}>
                {(chain) => (
                  <Card class="kria-memory__reasoning-result">
                    <strong>{chain.path.join(" → ")}</strong>
                    <Badge>{Math.round(chain.confidence * 100)}% confidence</Badge>
                  </Card>
                )}
              </For>
            </Show>
          </div>
        )}
      </Show>
    </div>
  );
}

function ColdStartRegion() {
  return (
    <div class="kria-memory__cold-start">
      <h2 class="kria-memory__region-title">Cold Start</h2>
      <Show when={memoryStore.coldStartStatus()?.onboardingComplete}>
        <div class="kria-memory__status" role="status">
          Initial memory onboarding complete. Granted sources: {memoryStore.coldStartStatus()?.granted.join(", ") || "none"}.
        </div>
      </Show>
      <MemoryOnboarding onDone={() => void memoryStore.refreshProductionData()} />
    </div>
  );
}

// ─── Knowledge Graph Region ────────────────────────────────────────────────

let knowledgeSessionCounter = 0;

function KnowledgeRegion() {
  const [projection, setProjection] = createSignal<KnowledgeProjectionResponse | null>(null);
  const items = createMemo<KnowledgeProjectionItem[]>(() => projection()?.items ?? []);
  const [selectedId, setSelectedId] = createSignal<string | null>(null);
  const [focusTrail, setFocusTrail] = createSignal<string[]>([]);
  const [filterQuery, setFilterQuery] = createSignal("");
  const [isLoading, setIsLoading] = createSignal(false);
  const [isSeeding, setIsSeeding] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [seedMessage, setSeedMessage] = createSignal<string | null>(null);
  const session = new MemoryWindowSessionV2({
    instanceId: `knowledge-${++knowledgeSessionCounter}`,
    policyHash: "local-desktop",
    schemaVersion: "knowledge-v1",
  });

  const graphRevision = createMemo(() => projection()?.graphRevision ?? 0);

  const filteredItems = createMemo(() => {
    const query = filterQuery().trim().toLocaleLowerCase();
    const source = items();
    if (!query) return source;
    const nodes = source.filter((item) =>
      item.kind !== "relation" && item.label.toLocaleLowerCase().includes(query));
    const nodeIds = new Set(nodes.map((item) => item.id));
    const relations = source.filter((item) =>
      item.kind === "relation" &&
      item.sourceEndpointId != null &&
      item.targetEndpointId != null &&
      nodeIds.has(item.sourceEndpointId) &&
      nodeIds.has(item.targetEndpointId));
    return [...nodes, ...relations];
  });

  const scene = createMemo(() => {
    const revision = graphRevision();
    const currentSelection = selectedId();
    const rawItems: RawSceneItem[] = filteredItems().map((item) => ({
      id: item.id,
      kind: item.kind,
      authorityClass: item.authorityClass,
      label: item.label,
      truthState: item.truthState,
      graphRevision: item.revision,
      direction: item.direction ?? null,
      sourceEndpointId: item.sourceEndpointId ?? null,
      targetEndpointId: item.targetEndpointId ?? null,
      evidenceCount: null,
      evidenceSummary: null,
      provenanceSourceId: null,
      provenanceMethod: null,
      provenanceVersion: null,
      provenanceActorLabel: null,
      validTimeStart: null,
      validTimeEnd: null,
      isCurrentlyValid: true,
      isSelected: item.id === currentSelection,
      isFocused: item.id === currentSelection,
      isInPath: false,
      isPending: false,
      hasError: false,
      isAuthorized: true,
    }));
    const actions = rawItems
      .filter((item) => item.kind !== "relation")
      .flatMap((item) => ([
        { targetItemId: item.id, kind: "select", label: "Select", isEnabled: true, isDangerous: false, requiresPreview: false, isAuthorized: true },
        { targetItemId: item.id, kind: "expand", label: "Focus", isEnabled: true, isDangerous: false, requiresPreview: false, isAuthorized: true },
      ]));
    return buildSemanticScene({
      items: rawItems,
      actions,
      graphRevision: revision,
      layoutHint: buildLayoutHint({
        queryKind: currentSelection ? "ego" : filterQuery().trim() ? "search" : "overview",
        queryHash: filterQuery().trim() || "knowledge-overview",
        graphRevision: revision,
        primaryItemId: currentSelection,
        maxDepth: currentSelection ? 2 : null,
      }),
    }).scene;
  });

  const listItems = createMemo(() => {
    const sceneIds = new Set(scene().items.map((item) => item.id));
    return filteredItems().filter((item) => item.kind !== "relation" && sceneIds.has(item.id));
  });

  const mapParityReady = createMemo(() => {
    const currentScene = scene();
    const ids = listItems().map((item) => item.id);
    return ids.length > 0 && ids.every((id) =>
      currentScene.actions.some((action) =>
        action.targetItemId === id && action.kind === "expand" && action.isEnabled));
  });

  async function loadItems(): Promise<void> {
    const token = session.beginRequest("knowledge-bootstrap");
    setIsLoading(true);
    setError(null);
    const result = await bridgeInvoke<unknown>(
      "memory_knowledge_items",
      { limit: 50 },
    );
    if (token.signal.aborted) return;
    if (result.ok) {
      const parsed = parseKnowledgeProjectionResponse(result.data);
      if (!parsed.ok) {
        if (session.failRequest(token.generation)) {
          setError(`Memory returned an invalid knowledge snapshot: ${parsed.message}. A previous snapshot is preserved if one was loaded.`);
        }
      } else if (session.completeRequest(token.generation, parsed.data.graphRevision)) {
        setProjection(parsed.data);
        if (parsed.omittedItemCount > 0) {
          setError(`${parsed.omittedItemCount} malformed knowledge item(s) were safely omitted.`);
        }
        const validIds = new Set(parsed.data.items.map((item) => item.id));
        if (selectedId() && !validIds.has(selectedId()!)) {
          setSelectedId(null);
          setFocusTrail([]);
        }
      }
    } else if (session.failRequest(token.generation)) {
      setError(result.code === "unavailable"
        ? "Memory service is unavailable. A previous snapshot is preserved if one was loaded."
        : `Could not load knowledge items: ${result.message}`);
    }
    if (token.generation === session.generation) setIsLoading(false);
  }

  async function seedDemo(): Promise<void> {
    setIsSeeding(true);
    setError(null);
    setSeedMessage(null);
    const result = await bridgeInvoke<{ message?: string }>("memory_seed_demo_knowledge");
    if (result.ok) {
      setSeedMessage(result.data.message ?? "Demo data seeded successfully.");
      await loadItems();
    } else {
      setError(`Seeding failed: ${result.message}`);
    }
    setIsSeeding(false);
  }

  function focusItem(id: string): void {
    setSelectedId(id);
    setFocusTrail((trail) => trail[trail.length - 1] === id ? trail : [...trail, id].slice(-8));
  }

  function goBack(): void {
    setFocusTrail((trail) => {
      const next = trail.slice(0, -1);
      setSelectedId(next[next.length - 1] ?? null);
      return next;
    });
  }

  function resetFocus(): void {
    setSelectedId(null);
    setFocusTrail([]);
  }

  onMount(() => void loadItems());
  onCleanup(() => session.markDetached());

  return (
    <Knowledge
      items={filteredItems()}
      scene={scene()}
      selectedId={selectedId()}
      focusTrail={focusTrail()}
      loadedNodeCount={items().filter((item) => item.kind !== "relation").length}
      snapshotItemCount={projection()?.count ?? null}
      graphRevision={projection()?.graphRevision ?? null}
      snapshotTruncated={projection()?.truncated ?? false}
      filterQuery={filterQuery()}
      inspectorAvailable={false}
      pathAvailable={false}
      correctionAvailable={false}
      mapParityReady={mapParityReady()}
      isLoading={isLoading()}
      isSeeding={isSeeding()}
      error={error()}
      seedMessage={seedMessage()}
      onFilterQuery={(query) => { setFilterQuery(query); resetFocus(); }}
      onSelectItem={focusItem}
      onOpenInspector={() => {}}
      onRequestPath={() => {}}
      onBack={goBack}
      onReset={resetFocus}
      onRetry={() => void loadItems()}
      onSeedDemo={import.meta.env.DEV ? seedDemo : undefined}
    />
  );
}
