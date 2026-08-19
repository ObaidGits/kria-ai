/**
 * Converse Store — owns the conversational AI workspace state.
 *
 * threads, active thread, messages, work-lane blocks, context rail, composer draft.
 * High-frequency token streams update only their target signal (Req 16.5).
 * Draft persistence satisfies Req 13.4 (preserve user's place).
 *
 * Requirements: 4.1 (three lanes), 4.5 (draft persistence), 13.4, 16.5
 */
import { createSignal } from "solid-js";
import { eventBus, type Unsubscribe } from "./eventBus";
import { bridgeInvoke, bridgeInvokeOptional } from "../bridge/invoke";
import {
  classifyEmptyState,
  type EmptyStateClass,
} from "../shell/spaces/converse/emptyStateClassifier";
import {
  activeGuiCognitionSession,
  clearGuiCognitionSession,
  handleGuiCognitionEvent,
  markGuiCognitionCancelled,
} from "./guiCognitionSession";
import { announceCancellation } from "./cancellationAnnouncer";

// ─── Types ─────────────────────────────────────────────────────────────────────

export interface Thread {
  id: string;
  title: string;
  createdAt: number;
  updatedAt: number;
  pinned: boolean;
  archived: boolean;
  temporary: boolean;
}

export interface ThreadSearchHit {
  sessionId: string;
  role: "user" | "assistant" | "system";
  content: string;
  timestamp: number;
}

/**
 * An inline result card attached to a message — a tool/result payload rendered
 * alongside (and secondary to) the reply text (Req 4.3 conversation-dominance).
 * `html` is treated as untrusted and is sanitized at render time.
 */
export interface MessageResult {
  id: string;
  /** Card kind → drives the header icon/label (tool output, memory, etc.). */
  kind: "tool-result" | "memory" | "document" | "image" | "custom";
  title: string;
  /** Optional short plain-text summary shown as the card body. */
  summary?: string;
  /** Optional untrusted HTML/markdown body — sanitized before display. */
  html?: string;
  /** Optional structured payload for deep-linking / inspection. */
  data?: unknown;
}

export interface Message {
  id: string;
  threadId: string;
  role: "user" | "assistant" | "system";
  content: string;
  timestamp: number;
  metadata?: Record<string, unknown>;
  /** Inline result cards attached to this message (tool/result payloads). */
  results?: MessageResult[];
  /**
   * Provenance (Req 5.7): ids of the memories the runtime used to produce this
   * answer, in relevance order (primary first). Populated from the runtime's
   * provided provenance where available — the UI never fabricates it. When
   * absent/empty the "why did KRIA answer this" affordance is honestly hidden
   * (no fake deep-link). Presentation reads this; it is never a tool call.
   */
  usedMemoryIds?: string[];
}

export type WorkBlockType =
  | "reasoning"
  | "tool-call"
  | "plan-compare"
  | "gui-cognition"
  | "workflow-run";

export type WorkBlockStatus = "pending" | "running" | "completed" | "failed" | "stopped";

/**
 * Human, scope-named label per work-block type, used to build the once-only
 * cancellation milestone announcement (UIE-M-015 / §17.5). Kept in sync with
 * the visual `TYPE_META` labels in WorkBlock.tsx; this is announcement copy for
 * the accessible milestone, so the per-item Stop names exactly what it stopped.
 */
const WORK_BLOCK_STOP_SCOPE: Record<WorkBlockType, string> = {
  reasoning: "Reasoning",
  "tool-call": "Tool call",
  "plan-compare": "Plan",
  "gui-cognition": "GUI cognition",
  "workflow-run": "Workflow run",
};

/**
 * A single evidence item attached to a work block — a source/artifact KRIA used
 * or produced (Req 4.2 "evidence"). `detail` may be untrusted HTML/text from a
 * tool and is sanitized at render time; `href` deep-links to a source.
 */
export interface WorkEvidence {
  id: string;
  label: string;
  detail?: string;
  href?: string;
}

/** tool-call payload — the invocation args + result (both untrusted at render). */
export interface ToolCallDetail {
  name: string;
  /** Pre-serialized argument text (escaped at render). */
  args?: string;
  /** Tool result text/HTML (sanitized at render). */
  result?: string;
}

/** One step within a candidate plan — plain-language, optional live status. */
export interface PlanCompareStep {
  /** Plain-language description of the step. */
  label: string;
  /** Optional live execution status once the plan runs. */
  status?: WorkBlockStatus;
  /** Optional technical/command detail (rendered as escaped mono text). */
  detail?: string;
  /** Optional outcome text once executed (e.g. "exit 0 · 120ms"). */
  outcome?: string;
}

/**
 * plan-compare option — a candidate plan KRIA proposed. The revived
 * PlanVisualization (task 3.7) renders these side-by-side: label, relative
 * risk, model score, tradeoffs, ordered steps, and which is recommended.
 *
 * Presentation read-model ONLY. Selecting an option routes back through the
 * existing converse/approval path (see `selectPlanOption`) — never a direct
 * tool call (KRIA runtime-authority invariant).
 */
export interface PlanCompareOption {
  id: string;
  label: string;
  /** Short plain-language summary (untrusted → sanitized at render). */
  summary?: string;
  recommended?: boolean;
  /** Relative risk posture — conveyed by icon + TEXT, never color alone (Req 17.3). */
  risk?: "safe" | "moderate" | "aggressive";
  /** Model score 0..1 (SelfModel Beta posterior). */
  score?: number;
  /** Estimated success confidence 0..1. */
  confidence?: number;
  /** Plain-language tradeoffs (untrusted → sanitized at render). */
  tradeoffs?: string;
  /** Ordered candidate steps for this plan. */
  steps?: PlanCompareStep[];
}

/** gui-cognition step — one observed/acted step with its own status. */
export interface GuiCognitionStep {
  id: string;
  label: string;
  status: WorkBlockStatus;
}

/** workflow-run progress payload. `progress` is a 0..1 fraction. */
export interface WorkflowRunDetail {
  progress?: number;
  completed?: number;
  total?: number;
  log?: string[];
}

export interface WorkBlock {
  id: string;
  type: WorkBlockType;
  status: WorkBlockStatus;
  /**
   * The conversation turn this block belongs to — the id of the user message
   * that started the turn. Drives the inline per-turn activity trace (rendered
   * right after that user message, ChatGPT/Gemini style) instead of a separate
   * Work lane. Absent for blocks produced outside a tracked turn.
   */
  turnId?: string;
  /** Plain-language summary — always visible (Req 4.2). */
  summary: string;
  /** Generic collapsible detail text (markdown, sanitized at render). */
  details?: string;
  /** Evidence items surfaced in the block's evidence section (Req 4.2). */
  evidence?: WorkEvidence[];
  startedAt: number;
  completedAt?: number;

  // ── Type-specific payloads (all optional) ────────────────────────────────
  /** reasoning: the reasoning trace/thought text (markdown, sanitized). */
  reasoning?: string;
  /** tool-call: invocation args + result. */
  toolCall?: ToolCallDetail;
  /** plan-compare: candidate plans (PlanVisualization slots here — task 3.7). */
  planOptions?: PlanCompareOption[];
  /** plan-compare: why KRIA recommends the highlighted option (untrusted → sanitized). */
  planSelectionReason?: string;
  /** plan-compare: goal-verification outcome once the chosen plan has run. */
  planOutcome?: { outcome: "achieved" | "failed" | "continue"; reason?: string };
  /** gui-cognition: observed/acted steps. */
  guiSteps?: GuiCognitionStep[];
  /** workflow-run: run progress + log. */
  workflowRun?: WorkflowRunDetail;
}

export interface ContextRailItem {
  id: string;
  type: "memory" | "document" | "tool-result" | "custom";
  label: string;
  data: unknown;
  /**
   * Provenance — where this item came from (a memory id, document name, tool
   * name, …). Source-owned, OPTIONAL: a value only appears when a real writer
   * provides it. This is a frontend-only shape field (Task 10.4 / G2), NOT new
   * backend data; absent → rendered as nothing (never inferred). See
   * capabilityFieldMap F2 / MUST_OMIT_FACTS M2.
   */
  source?: string;
  /**
   * Available-vs-consumed distinction (UIE-M-011 / MUST_OMIT M2). OPTIONAL
   * because no authoritative field expresses it today: it is rendered ONLY when
   * a writer sets it, and stays absent (omitted) otherwise — the UI never
   * synthesizes a use-state it cannot prove.
   */
  use?: "available" | "used";
  /**
   * A concise, bounded detail string, source-owned. OPTIONAL; omitted when
   * absent. Long values are truncated at the presentation layer (10.7).
   */
  detail?: string;
}

export interface ComposerAttachment {
  id: string;
  name: string;
  mime: string;
  size: number;
  bytes: Uint8Array;
}

export interface ComposerDraft {
  text: string;
  attachments: ComposerAttachment[];
  mode: "assistant" | "lab" | "tool-lock";
  toolLock?: string;
}

export type ConversationExportFormat = "text" | "markdown" | "json" | "pdf";

/**
 * What `export_session` returns — rendered by `kria_core::agent::transcript`.
 *
 * snake_case because these are the Rust struct's field names as serialised; renaming
 * them here would only hide where they come from.
 */
interface ExportedTranscript {
  content: string;
  suggested_name: string;
  extension: string;
  filter_label: string;
  turn_count: number;
}

// ─── Signals ───────────────────────────────────────────────────────────────────

const [threads, setThreads] = createSignal<Thread[]>([]);
const [activeThreadId, setActiveThreadIdSignal] = createSignal<string | null>(null);
const [messages, setMessages] = createSignal<Message[]>([]);
// The thread explicitly created/selected as an Intentional New Thread (UIE-H-005).
// Owned here; reset only at documented lifecycle transitions (see setActiveThread
// and addMessage). null means "no explicit new-thread intent is active".
const [newThreadIntentId, setNewThreadIntentId] = createSignal<string | null>(null);
const [thinking, setThinking] = createSignal(false);
const [workBlocks, setWorkBlocks] = createSignal<WorkBlock[]>([]);
// The turn (user-message id) currently receiving work blocks. Set when a send
// path adds its optimistic user turn; runtime work events tag their blocks with
// it so the inline activity trace groups under the right message.
const [currentTurnId, setCurrentTurnId] = createSignal<string | null>(null);
const [contextRail, setContextRail] = createSignal<ContextRailItem[]>([]);
const [composerDraft, setComposerDraft] = createSignal<ComposerDraft>({
  text: "",
  attachments: [],
  mode: "assistant",
});
const [loadingThreads, setLoadingThreads] = createSignal(false);
const [runtimeError, setRuntimeError] = createSignal<string | null>(null);
const [threadSearchQuery, setThreadSearchQuery] = createSignal("");
const [threadSearchHits, setThreadSearchHits] = createSignal<ThreadSearchHit[]>([]);
const [searchingThreads, setSearchingThreads] = createSignal(false);
const [deletingThreadId, setDeletingThreadId] = createSignal<string | null>(null);
const [exportFormat, setExportFormat] = createSignal<ConversationExportFormat>("markdown");
const [exportingConversation, setExportingConversation] = createSignal(false);
/**
 * The message the user asked to edit, or `null`.
 *
 * Lives in the store rather than inside ConverseSpace because the request comes from
 * `messageActions.ts`, a plain module with no component scope — the same reason
 * "Why did KRIA answer this?" reaches for shellStore. The dialog itself is rendered
 * by ConverseSpace, which watches this.
 */
const [messagePendingEdit, setMessagePendingEdit] = createSignal<Message | null>(null);

interface RawSession {
  id: string;
  title?: string;
  turn_count?: number;
  last_active?: string;
  pinned?: boolean;
  archived?: boolean;
  temporary?: boolean;
}

interface RawThreadSearchHit {
  session_id?: string;
  role?: string;
  content?: string;
  timestamp?: string;
}

interface RawHistoryMessage {
  role?: string;
  content?: string;
  timestamp?: string;
  tool_name?: string | null;
  tool_result?: string | null;
}

interface DeleteSessionResult {
  deleted_session_id: string;
  replacement_session_id: string | null;
}

let runtimeSubscriptions: Unsubscribe[] = [];
let runtimeInitialized = false;

function ownsActiveThread(sessionId: string): boolean {
  return !sessionId || sessionId === activeThreadId();
}

function normalizeThread(raw: RawSession): Thread {
  const updatedAt = raw.last_active ? Date.parse(raw.last_active) : Date.now();
  return {
    id: raw.id,
    title: raw.title?.trim() || "Untitled",
    createdAt: Number.isFinite(updatedAt) ? updatedAt : Date.now(),
    updatedAt: Number.isFinite(updatedAt) ? updatedAt : Date.now(),
    pinned: Boolean(raw.pinned),
    archived: Boolean(raw.archived),
    temporary: Boolean(raw.temporary),
  };
}

function normalizeHistory(sessionId: string, rows: RawHistoryMessage[]): Message[] {
  return rows.map((row, index) => {
    const timestamp = row.timestamp ? Date.parse(row.timestamp) : Date.now();
    const content = row.content ?? "";
    const toolResult = row.tool_result
      ? (() => {
          try { return JSON.parse(row.tool_result); } catch { return row.tool_result; }
        })()
      : undefined;
    return {
      id: `${sessionId}:${index}:${Number.isFinite(timestamp) ? timestamp : index}`,
      threadId: sessionId,
      role: row.role === "user" || row.role === "system" ? row.role : "assistant",
      content,
      timestamp: Number.isFinite(timestamp) ? timestamp : Date.now(),
      metadata: row.tool_name ? { toolName: row.tool_name } : undefined,
      results: row.tool_name
        ? [{
            id: `${sessionId}:tool:${index}`,
            kind: "tool-result" as const,
            title: row.tool_name,
            summary: content,
            data: toolResult,
          }]
        : undefined,
    };
  });
}

async function loadThreads(): Promise<Thread[]> {
  const result = await bridgeInvoke<RawSession[]>("list_sessions", undefined, { timeoutMs: 15_000 });
  if (!result.ok) throw new Error(result.message);
  const unique = new Map(result.data.map((row) => [row.id, normalizeThread(row)]));
  const normalized = [...unique.values()].sort(
    (a, b) => Number(b.pinned) - Number(a.pinned) || b.updatedAt - a.updatedAt,
  );
  setThreads(normalized);
  return normalized;
}

async function activateThread(threadId: string): Promise<void> {
  if (!threadId) return;
  setRuntimeError(null);
  const switched = await bridgeInvoke<void>("switch_session", { sessionId: threadId });
  if (!switched.ok) {
    setRuntimeError(switched.message);
    return;
  }

  // Publish selection only after backend session authority has switched. This
  // prevents streamed events from being tagged with the prior session and then
  // rejected by ownsActiveThread().
  setActiveThread(threadId);
  const history = await bridgeInvoke<RawHistoryMessage[]>(
    "get_session_history",
    { sessionId: threadId },
    { timeoutMs: 15_000 },
  );
  if (!history.ok) {
    setRuntimeError(history.message);
    return;
  }
  if (activeThreadId() === threadId && !thinking()) {
    setMessages(normalizeHistory(threadId, history.data));
  }
}

/**
 * Create a new thread.
 *
 * `intentional` (default true) marks the new empty thread as an Intentional New
 * Thread so the empty-state classifier presents a new-task state regardless of
 * unrelated history (UIE-H-005, Req 6.1). The boot auto-create (see
 * `initialize`) passes `intentional: false` so genuine first-run stays Cold
 * Start rather than being mislabelled as an explicit new-thread action.
 *
 * The intent is set BEFORE `activateThread` so the switch to the new thread does
 * not clear it (setActiveThread only clears intent when switching to a thread
 * that is NOT the intent thread).
 */
async function createThread(
  options: { intentional?: boolean } = {},
): Promise<string | null> {
  const created = await bridgeInvoke<{ session_id: string }>("create_session");
  if (!created.ok) {
    setRuntimeError(created.message);
    return null;
  }
  const id = created.data.session_id;
  if (options.intentional !== false) setNewThreadIntentId(id);
  await loadThreads();
  await activateThread(id);
  return id;
}

async function searchThreads(query: string): Promise<ThreadSearchHit[]> {
  const normalized = query.trim();
  setThreadSearchQuery(query);
  if (!normalized) {
    setThreadSearchHits([]);
    return [];
  }
  setSearchingThreads(true);
  try {
    const result = await bridgeInvoke<RawThreadSearchHit[]>("search_sessions", {
      query: normalized,
    });
    if (!result.ok) {
      actionNotification("error", `Couldn't search conversations: ${result.message}`);
      return [];
    }
    const hits = result.data
      .filter((row) => Boolean(row.session_id))
      .map((row) => {
        const timestamp = row.timestamp ? Date.parse(row.timestamp) : Date.now();
        return {
          sessionId: row.session_id!,
          role: row.role === "user" || row.role === "system" ? row.role : "assistant",
          content: row.content ?? "",
          timestamp: Number.isFinite(timestamp) ? timestamp : Date.now(),
        } satisfies ThreadSearchHit;
      });
    setThreadSearchHits(hits);
    return hits;
  } finally {
    setSearchingThreads(false);
  }
}

async function setThreadFlag(
  threadId: string,
  field: "pinned" | "archived" | "temporary",
  value: boolean,
): Promise<boolean> {
  const command = `set_session_${field}`;
  const result = await bridgeInvoke<void>(command, { sessionId: threadId, [field]: value });
  if (!result.ok) {
    actionNotification("error", `Couldn't update conversation: ${result.message}`);
    return false;
  }
  setThreads((current) =>
    current
      .map((thread) => (thread.id === threadId ? { ...thread, [field]: value } : thread))
      .sort((a, b) => Number(b.pinned) - Number(a.pinned) || b.updatedAt - a.updatedAt),
  );
  // Archiving REMOVES the chat from the visible list, so say so. Silently making a
  // conversation disappear is indistinguishable from losing it.
  if (field === "archived") {
    actionNotification(
      "success",
      value
        ? "Chat archived — hidden from the list until you choose Show archived."
        : "Chat restored to the list.",
    );
  }
  return true;
}

function setThreadPinned(threadId: string, pinned: boolean): Promise<boolean> {
  return setThreadFlag(threadId, "pinned", pinned);
}

function setThreadArchived(threadId: string, archived: boolean): Promise<boolean> {
  return setThreadFlag(threadId, "archived", archived);
}

function setThreadTemporary(threadId: string, temporary: boolean): Promise<boolean> {
  return setThreadFlag(threadId, "temporary", temporary);
}

/**
 * Rename a conversation to a name the user typed.
 *
 * `rename_session` has existed in the backend all along with nothing calling it, so
 * there was no way to name a chat. It also sets a "manual" flag server-side, which
 * stops automatic titling from ever overwriting the user's choice.
 */
async function renameThread(threadId: string, title: string): Promise<boolean> {
  const trimmed = title.trim();
  if (!threadId) return false;
  if (trimmed.length === 0) {
    actionNotification("error", "A conversation name cannot be empty.");
    return false;
  }
  const result = await bridgeInvoke<void>("rename_session", { sessionId: threadId, title: trimmed });
  if (!result.ok) {
    actionNotification("error", `Couldn't rename the conversation: ${result.message}`);
    return false;
  }
  // Update in place rather than reloading: the row the user just edited should not
  // jump or flicker while a full list round trip completes.
  setThreads((current) =>
    current.map((thread) => (thread.id === threadId ? { ...thread, title: trimmed } : thread)),
  );
  actionNotification("success", "Conversation renamed");
  return true;
}

/**
 * Ask the backend to derive a readable title for a conversation.
 *
 * Returns the applied title, or `null` when nothing changed — which is the normal
 * outcome for a chat the user named themselves or one that still contains only
 * greetings. Callers treat `null` as "fine, leave it".
 */
async function autoTitleThread(threadId: string): Promise<string | null> {
  if (!threadId) return null;
  const result = await bridgeInvoke<{ updated: boolean; title?: string }>(
    "auto_title_session",
    { sessionId: threadId },
  );
  if (!result.ok || !result.data?.updated || !result.data.title) return null;
  const title = result.data.title;
  setThreads((current) =>
    current.map((thread) => (thread.id === threadId ? { ...thread, title } : thread)),
  );
  return title;
}

/**
 * Delete every conversation and start a fresh one.
 *
 * Irreversible, so the caller is responsible for confirming first — this function
 * does the deed and does not ask. The backend returns how much it removed, which is
 * reported back so the user sees that something actually happened.
 */
async function clearAllThreads(): Promise<boolean> {
  const result = await bridgeInvoke<{
    deleted_session_count: number;
    deleted_turn_count: number;
    replacement_session_id: string;
  }>("clear_all_chat_sessions", undefined, { timeoutMs: 60_000 });
  if (!result.ok) {
    actionNotification("error", `Couldn't clear conversations: ${result.message}`);
    return false;
  }
  clearMessages();
  const refreshed = await loadThreads();
  const replacement = result.data?.replacement_session_id;
  if (replacement && refreshed.some((thread) => thread.id === replacement)) {
    await activateThread(replacement);
  }
  const sessions = result.data?.deleted_session_count ?? 0;
  actionNotification(
    "success",
    sessions === 1 ? "1 conversation deleted" : `${sessions} conversations deleted`,
  );
  return true;
}

async function deleteThread(threadId: string): Promise<boolean> {
  if (!threadId || deletingThreadId()) return false;
  const deletingActiveThread = activeThreadId() === threadId;
  const activeGuiSession = activeGuiCognitionSession();
  if (deletingActiveThread && (thinking() || activeGuiSession?.sessionId === threadId)) {
    actionNotification("info", "Stop the active response or desktop task before deleting this chat.");
    return false;
  }

  setDeletingThreadId(threadId);
  try {
    const result = await bridgeInvoke<DeleteSessionResult>(
      "delete_session",
      { sessionId: threadId },
      { timeoutMs: 15_000 },
    );
    if (!result.ok) {
      actionNotification("error", `Couldn't delete conversation: ${result.message}`);
      return false;
    }

    setThreads((current) => current.filter((thread) => thread.id !== threadId));
    setThreadSearchHits((current) => current.filter((hit) => hit.sessionId !== threadId));
    setDraftMap((current) => {
      const next = { ...current };
      delete next[threadId];
      return next;
    });
    persistDrafts();
    if (newThreadIntentId() === threadId) setNewThreadIntentId(null);

    if (deletingActiveThread) {
      setActiveThreadIdSignal(null);
      setMessages([]);
      setWorkBlocks([]);
      setCurrentTurnId(null);
      setContextRail([]);
      setComposerDraft({ ...DEFAULT_DRAFT });
      clearGuiCognitionSession();
      eventBus.emit("converse:thread-switched", { threadId: "" });
    }

    let refreshed: Thread[] = [];
    try {
      refreshed = await loadThreads();
    } catch (error) {
      actionNotification(
        "error",
        `Chat was deleted, but the thread list could not refresh: ${error instanceof Error ? error.message : String(error)}`,
      );
    }

    if (deletingActiveThread) {
      const replacementId = result.data.replacement_session_id ?? refreshed[0]?.id ?? null;
      if (replacementId) await activateThread(replacementId);
    }

    actionNotification("success", "Chat deleted.");
    return true;
  } finally {
    setDeletingThreadId(null);
  }
}

function toolResultText(value: unknown): string {
  if (value == null) return "";
  if (typeof value === "string") return value;
  try { return JSON.stringify(value, null, 2); } catch { return String(value); }
}

function exportRole(role: Message["role"]): string {
  return role === "user" ? "You" : role === "assistant" ? "KRIA" : "System";
}

function exportTime(timestamp: number): string {
  return new Date(timestamp).toLocaleString();
}

function safeExportName(title: string, extension: string): string {
  const safeTitle = title.trim().replace(/[^a-zA-Z0-9._-]+/g, "-").replace(/^-+|-+$/g, "") || "conversation";
  return `kria-${safeTitle}-${new Date().toISOString().slice(0, 10)}.${extension}`;
}




function escapeExportHtml(value: string): string {
  return value.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
}

function buildPrintExport(title: string): string {
  const messageHtml = messages().map((message) => {
    const results = (message.results ?? []).map((result) => `
      <section class="result">
        <strong>${escapeExportHtml(result.title)}</strong> <span>[${escapeExportHtml(result.kind)}]</span>
        ${result.summary ? `<p>${escapeExportHtml(result.summary)}</p>` : ""}
        ${result.data !== undefined ? `<pre>${escapeExportHtml(toolResultText(result.data))}</pre>` : ""}
      </section>`).join("");
    return `<article class="message">
      <header><strong>${escapeExportHtml(exportRole(message.role))}</strong><time>${escapeExportHtml(exportTime(message.timestamp))}</time></header>
      <div class="content">${escapeExportHtml(message.content)}</div>${results}
    </article>`;
  }).join("");
  const toolHtml = workBlocks().filter((block) => block.type === "tool-call" && block.toolCall).map((block) => `
    <article class="tool">
      <header><strong>${escapeExportHtml(block.toolCall!.name)}</strong><span>Status: ${escapeExportHtml(block.status)}</span></header>
      ${block.toolCall!.args !== undefined ? `<h3>Arguments</h3><pre>${escapeExportHtml(block.toolCall!.args)}</pre>` : ""}
      ${block.toolCall!.result !== undefined ? `<h3>Result</h3><pre>${escapeExportHtml(block.toolCall!.result)}</pre>` : ""}
    </article>`).join("");
  return `<!doctype html><html lang="en"><head><meta charset="utf-8"><title>${escapeExportHtml(title)}</title>
<style>
:root { color-scheme: light dark; }
body { margin: 0; padding: 2rem; font: 14px/1.55 system-ui, sans-serif; color: CanvasText; background: Canvas; }
header { display: flex; justify-content: space-between; gap: 1rem; }
h1 { font-size: 1.5rem; } h2 { margin-top: 2rem; } h3 { font-size: 0.85rem; }
time, header span { color: GrayText; }
.message, .tool, .result { margin: 1rem 0; padding: 1rem; border: 1px solid GrayText; border-radius: 0.5rem; break-inside: avoid; }
.result { margin-bottom: 0; }
.content, pre { margin-top: 0.75rem; white-space: pre-wrap; overflow-wrap: anywhere; }
pre { font-family: ui-monospace, monospace; }
@media print { body { padding: 0; } }
</style></head><body><h1>KRIA Conversation — ${escapeExportHtml(title)}</h1><p>Exported ${escapeExportHtml(new Date().toLocaleString())}</p>${messageHtml}${toolHtml ? `<h2>Tool execution details</h2>${toolHtml}` : ""}</body></html>`;
}

/**
 * A native "Save As" dialog waits for a HUMAN to choose a folder and confirm.
 *
 * This is the fix for the reported bug: `save_export_file` inherited the module's
 * 8-second default timeout, so anyone who took longer than that to pick a location
 * saw `invoke('save_export_file') timed out after 8000ms` — reported to them as
 * "Check file permissions and try again", which was not the problem at all. Worse,
 * nothing cancels the Rust side, so a click after the deadline still wrote the file
 * while the UI had already declared failure.
 *
 * Five minutes is not a performance budget, it is a "user walked away" backstop.
 */
const SAVE_DIALOG_TIMEOUT_MS = 300_000;

async function exportActiveConversation(format: ConversationExportFormat = exportFormat()): Promise<boolean> {
  const threadId = activeThreadId();
  const thread = threads().find((candidate) => candidate.id === threadId);
  if (!threadId || !thread) {
    actionNotification("error", "Select a conversation before exporting.");
    return false;
  }
  if (messages().length === 0) {
    actionNotification("error", "This conversation has no messages to export.");
    return false;
  }
  if (exportingConversation()) return false;

  setExportFormat(format);
  setExportingConversation(true);
  try {
    if (format === "pdf") {
      const printed = await bridgeInvoke<unknown>(
        "open_html_for_print",
        { html: buildPrintExport(thread.title), filename: safeExportName(thread.title, "html") },
        // Opens the OS default browser, which can be slow to launch cold.
        { timeoutMs: SAVE_DIALOG_TIMEOUT_MS },
      );
      if (!printed.ok) {
        actionNotification("error", `Couldn't open the print dialog: ${printed.message}`);
        return false;
      }
      actionNotification("success", "Print dialog opened");
      return true;
    }

    // Rendering happens in kria-core via `export_session`, not here. That gives the
    // export the WHOLE conversation from the database rather than only the turns
    // currently loaded into this store, and puts the escaping and filename rules
    // where they can be unit-tested.
    const rendered = await bridgeInvoke<ExportedTranscript>(
      "export_session",
      { sessionId: threadId, format },
      { timeoutMs: 30_000 },
    );
    if (!rendered.ok) {
      actionNotification("error", `Couldn't prepare the ${format} export: ${rendered.message}`);
      return false;
    }
    const transcript = rendered.data;
    if (!transcript || transcript.turn_count === 0) {
      actionNotification("error", "This conversation has no saved messages to export.");
      return false;
    }

    const saved = await bridgeInvoke<string | null>(
      "save_export_file",
      {
        content: transcript.content,
        defaultName: transcript.suggested_name,
        filterName: transcript.filter_label,
        extensions: [transcript.extension],
      },
      { timeoutMs: SAVE_DIALOG_TIMEOUT_MS },
    );
    if (!saved.ok) {
      // The real reason, not a guess about permissions.
      actionNotification("error", `Couldn't save the ${format} file: ${saved.message}`);
      return false;
    }
    // `null` means the user closed the dialog. That is a choice, not a failure, so it
    // gets no error toast — the previous code returned false here silently, which was
    // right, and this keeps that while making the intent explicit.
    if (saved.data === null) return false;

    actionNotification("success", `Conversation exported to ${saved.data}`);
    return true;
  } finally {
    setExportingConversation(false);
  }
}

function initRuntimeSubscriptions(): void {
  if (runtimeSubscriptions.length > 0) return;

  runtimeSubscriptions.push(
    eventBus.on("agent:thinking", ({ sessionId }) => {
      if (ownsActiveThread(sessionId)) setThinkingState(sessionId || activeThreadId() || "", true);
    }),
    eventBus.on("agent:token", ({ sessionId, text }) => {
      if (ownsActiveThread(sessionId)) appendToken(sessionId || activeThreadId() || "", text);
    }),
    eventBus.on("agent:tool-call", ({ sessionId, name, params }) => {
      if (!ownsActiveThread(sessionId)) return;
      addWorkBlock({
        id: crypto.randomUUID(),
        type: "tool-call",
        status: "running",
        summary: `Running ${name}`,
        startedAt: Date.now(),
        turnId: currentTurnId() ?? undefined,
        toolCall: { name, args: toolResultText(params) },
      });
    }),
    eventBus.on("agent:tool-result", ({ sessionId, name, result, success, summary }) => {
      if (!ownsActiveThread(sessionId)) return;
      const block = [...workBlocks()].reverse().find(
        (item) => item.type === "tool-call" && item.status === "running" && item.toolCall?.name === name,
      );
      if (block) {
        updateWorkBlock(block.id, {
          status: success ? "completed" : "failed",
          summary: summary || `${name} ${success ? "completed" : "failed"}`,
          completedAt: Date.now(),
          toolCall: { ...block.toolCall!, result: toolResultText(result) },
        });
      }
      const threadId = sessionId || activeThreadId() || "";
      addMessage({
        id: crypto.randomUUID(),
        threadId,
        role: "assistant",
        content: summary ?? "",
        timestamp: Date.now(),
        // The conversational line lives in `content`; the card carries the
        // STRUCTURED payload (rendered as a collapsed, wrapped JSON block) so the
        // same text is not shown twice and a large blob never overflows.
        results: [{
          id: crypto.randomUUID(),
          kind: "tool-result",
          title: name,
          data: result,
        }],
      });
    }),
    eventBus.on("agent:stage", ({ step, message, detail }) => {
      const turn = currentTurnId() ?? activeThreadId() ?? "";
      // Keyed per-turn so each turn owns its own reasoning block in the inline
      // activity trace (rather than a single per-thread block).
      const id = `reasoning:${turn}`;
      const existing = workBlocks().find((item) => item.id === id);
      const terminal = step === "completed";
      const failed = step.includes("fail") || step.includes("error") || step.includes("timed_out");
      if (existing) {
        updateWorkBlock(id, {
          status: failed ? "failed" : terminal ? "completed" : "running",
          summary: message,
          details: detail ? toolResultText(detail) : existing.details,
          completedAt: terminal || failed ? Date.now() : undefined,
        });
      } else {
        addWorkBlock({
          id,
          type: "reasoning",
          status: failed ? "failed" : terminal ? "completed" : "running",
          summary: message,
          details: detail ? toolResultText(detail) : undefined,
          startedAt: Date.now(),
          turnId: currentTurnId() ?? undefined,
          completedAt: terminal || failed ? Date.now() : undefined,
        });
      }
    }),
    eventBus.on("agent:done", ({ sessionId }) => {
      if (ownsActiveThread(sessionId)) setThinkingState(sessionId || activeThreadId() || "", false);
      // Give the conversation a readable name now that it has content.
      //
      // Every chat was created as "New chat" and nothing ever replaced it, so the
      // sidebar filled up with identical rows — the user could not tell their own
      // conversations apart. The backend has always had the rename command; nothing
      // called it. Titling happens BEFORE reloading the list so the refreshed rows
      // already carry the new name instead of showing the placeholder for one frame.
      //
      // The command decides whether a rename is warranted: it refuses when the user
      // renamed by hand, and when the conversation still holds nothing but greetings.
      // Failure is deliberately swallowed — a chat keeping its placeholder title is
      // not worth interrupting the user over.
      void autoTitleThread(sessionId || activeThreadId() || "")
        .catch(() => undefined)
        .then(() => loadThreads())
        .catch(() => undefined);
    }),
    eventBus.on("agent:error", ({ sessionId, message }) => {
      if (!ownsActiveThread(sessionId)) return;
      setThinkingState(sessionId || activeThreadId() || "", false);
      addMessage({
        id: crypto.randomUUID(),
        threadId: sessionId || activeThreadId() || "",
        role: "system",
        content: message,
        timestamp: Date.now(),
      });
    }),
    eventBus.on("gui-cognition:event", ({ payload }) => {
      handleGuiCognitionEvent(payload as Parameters<typeof handleGuiCognitionEvent>[0]);
      const lifecycle = activeGuiCognitionSession()?.lifecycle;
      if (["completed", "failed", "blocked", "cancelled"].includes(lifecycle ?? "")) {
        setThinkingState(activeThreadId() || "", false);
      }
    }),
  );
}

async function initialize(): Promise<void> {
  if (runtimeInitialized) return;
  runtimeInitialized = true;
  initRuntimeSubscriptions();
  setLoadingThreads(true);
  setRuntimeError(null);
  try {
    let loaded = await loadThreads();
    if (loaded.length === 0) {
      // Boot auto-create is NOT an explicit user "new thread" action — keep it
      // non-intentional so genuine first-run classifies as Cold Start.
      const id = await createThread({ intentional: false });
      if (!id) return;
      loaded = threads();
    }
    const restored = activeThreadId();
    const selected = loaded.some((thread) => thread.id === restored) ? restored! : loaded[0]?.id;
    if (selected) await activateThread(selected);
  } catch (error) {
    setRuntimeError(error instanceof Error ? error.message : String(error));
  } finally {
    setLoadingThreads(false);
  }
}

function disposeRuntime(): void {
  for (const unsubscribe of runtimeSubscriptions.splice(0)) unsubscribe();
  runtimeInitialized = false;
  clearGuiCognitionSession();
}

async function cancelGuiCognitionTurn(): Promise<void> {
  const sessionId = activeGuiCognitionSession()?.sessionId ?? activeThreadId();
  markGuiCognitionCancelled("Turn cancelled by you.");
  setThinkingState(sessionId ?? "", false);
  // Semantic milestone, announced once (UIE-M-015 / §17.5): the GUI-cognition
  // scope stopped. Not a raw tick of the cancelling lifecycle.
  announceCancellation("GUI cognition stopped");
  if (!sessionId) return;
  await bridgeInvoke("cancel_gui_cognition_turn", { sessionId, reason: "Turn cancelled by you." });
}

// ─── Per-thread draft persistence (Req 4.5 / 13.4) ──────────────────────────
//
// Drafts are keyed by thread id and MUST survive relaunch (Req 4.5). The
// session router (task 1.5) restores the active thread at boot and calls
// `setActiveThread`, which restores that thread's draft from this map — so
// persisting the map to localStorage is sufficient for relaunch restore.

const DRAFTS_STORAGE_KEY = "kria.converse.drafts";
const DRAFT_PERSIST_DEBOUNCE_MS = 200;

const DEFAULT_DRAFT: ComposerDraft = { text: "", attachments: [], mode: "assistant" };

/**
 * Draft-map key for the home / no-thread draft (Req 4.5). When no thread is
 * active (the presence homepage, or before the first thread is opened) the
 * Composer draft is keyed here so it is saved on thread switch and restored on
 * return home — a home draft persists exactly like a per-thread draft, rather
 * than living only in the volatile `composerDraft` signal and being lost the
 * moment the user switches into a thread.
 */
export const HOME_DRAFT_KEY = "__home__";

/** Load persisted drafts, degrading to an empty map on any corruption. */
function loadDrafts(): Record<string, ComposerDraft> {
  if (typeof window === "undefined") return {};
  try {
    const raw = window.localStorage.getItem(DRAFTS_STORAGE_KEY);
    if (!raw) return {};
    const parsed = JSON.parse(raw) as unknown;
    if (!parsed || typeof parsed !== "object") return {};
    const out: Record<string, ComposerDraft> = {};
    for (const [threadId, value] of Object.entries(parsed as Record<string, unknown>)) {
      const d = value as Partial<ComposerDraft> | null;
      if (!d || typeof d !== "object") continue;
      out[threadId] = {
        text: typeof d.text === "string" ? d.text : "",
        // Browser file handles cannot be restored after relaunch. Persisting
        // filenames alone would create dead/fake attachments, while persisting
        // raw bytes would overflow localStorage. Keep text/mode; require reattach.
        attachments: [],
        mode: d.mode === "lab" || d.mode === "tool-lock" ? d.mode : "assistant",
        toolLock: typeof d.toolLock === "string" ? d.toolLock : undefined,
      };
    }
    return out;
  } catch {
    return {};
  }
}

const [draftMap, setDraftMap] = createSignal<Record<string, ComposerDraft>>(loadDrafts());

// Restore the home/no-thread draft at boot (Req 4.5). No thread is active on the
// presence homepage, so nothing calls setActiveThread to hydrate it; seed the
// live draft directly from the persisted map so a home draft survives relaunch.
{
  const homeDraft = draftMap()[HOME_DRAFT_KEY];
  if (homeDraft && (homeDraft.text.length > 0 || homeDraft.mode !== "assistant")) {
    setComposerDraft({ ...homeDraft });
  }
}

let draftPersistTimer: ReturnType<typeof setTimeout> | null = null;

/** Debounced persist of the draft map so keystrokes don't thrash storage (Req 16). */
function persistDrafts(): void {
  if (typeof window === "undefined") return;
  if (draftPersistTimer) clearTimeout(draftPersistTimer);
  draftPersistTimer = setTimeout(() => {
    draftPersistTimer = null;
    try {
      const serializable = Object.fromEntries(
        Object.entries(draftMap()).map(([threadId, draft]) => [
          threadId,
          { ...draft, attachments: [] },
        ]),
      );
      window.localStorage.setItem(DRAFTS_STORAGE_KEY, JSON.stringify(serializable));
    } catch {
      // localStorage full/unavailable — drafts simply won't survive relaunch.
    }
  }, DRAFT_PERSIST_DEBOUNCE_MS);
}

// ─── Actions ───────────────────────────────────────────────────────────────────

function setActiveThread(threadId: string | null): void {
  const previous = activeThreadId();
  if (previous === threadId) return;

  // Intent reset point (documented lifecycle transition): switching the active
  // thread to any thread that is NOT the intent thread clears the Intentional
  // New Thread intent. Switching TO the intent thread (e.g. the create→activate
  // path) preserves it. This keeps intent from leaking across unrelated
  // thread switches (UIE-H-005).
  if (threadId !== newThreadIntentId()) setNewThreadIntentId(null);

  // Save current draft before switching (per-thread persistence, Req 4.5). The
  // home/no-thread draft is keyed under HOME_DRAFT_KEY so it survives a switch
  // into a thread and is restored on return home.
  setDraftMap((prev) => ({ ...prev, [previous ?? HOME_DRAFT_KEY]: composerDraft() }));

  setActiveThreadIdSignal(threadId);

  // Restore the target's draft (thread id, or the home draft when going home);
  // falls back to a clean draft when none was saved.
  const restoreKey = threadId ?? HOME_DRAFT_KEY;
  const saved = draftMap()[restoreKey];
  setComposerDraft(saved ? { ...saved } : { ...DEFAULT_DRAFT });

  persistDrafts();
  eventBus.emit("converse:thread-switched", { threadId: threadId ?? "" });
}

function addMessage(msg: Message): void {
  // Intent reset point (documented lifecycle transition): the first message in
  // the intent thread promotes it to an Active conversation, so the Intentional
  // New Thread intent is cleared (UIE-H-005).
  if (msg.threadId && msg.threadId === newThreadIntentId()) setNewThreadIntentId(null);
  setMessages((prev) => [...prev, msg]);
  eventBus.emit("converse:message-added", { sessionId: msg.threadId, messageId: msg.id });
}

/**
 * Derive the current empty-state class from authoritative signals only (Req 6.1,
 * design Property 3). Single source of truth consumed by ConverseEmptyState
 * (task 6.4). Pure delegation to the deterministic classifier.
 */
function emptyStateClass(): EmptyStateClass {
  return classifyEmptyState({
    activeThreadId: activeThreadId(),
    hasMessages: messages().length > 0,
    newThreadIntentId: newThreadIntentId(),
    threads: threads(),
  });
}

/**
 * Deterministically place the store into the Intentional New Thread state:
 * activate `threadId` (an empty thread) and raise the explicit new-thread
 * intent for it, so `emptyStateClass()` returns "intentional-new-thread"
 * regardless of unrelated history (UIE-H-005, Req 6.1). This is the same
 * post-condition `createThread()` establishes, exposed as a bridge-free seam
 * for the dev-only E2E harness and deterministic tests — it performs NO backend
 * call, sends nothing, and invokes no tool.
 */
function markIntentionalNewThread(threadId: string): void {
  setActiveThread(threadId);
  setNewThreadIntentId(threadId);
}

function appendToken(sessionId: string, token: string): void {
  // Update only the last assistant message. Create the assistant turn on the
  // first token so the production stream never depends on fixture seeding.
  setMessages((prev) => {
    const last = prev[prev.length - 1];
    if (last && last.role === "assistant" && last.threadId === sessionId && !last.results?.length) {
      return [...prev.slice(0, -1), { ...last, content: last.content + token }];
    }
    return [
      ...prev,
      {
        id: crypto.randomUUID(),
        threadId: sessionId,
        role: "assistant",
        content: token,
        timestamp: Date.now(),
      },
    ];
  });
  eventBus.emit("converse:token", { sessionId, token });
}

function setThinkingState(sessionId: string, value: boolean): void {
  setThinking(value);
  eventBus.emit("converse:thinking-changed", { sessionId, thinking: value });
}

/** Mark the turn (user-message id) that subsequent work blocks belong to. */
function setCurrentTurn(turnId: string | null): void {
  setCurrentTurnId(turnId && turnId.trim() ? turnId : null);
}

/** Work blocks for one turn (inline activity trace), in arrival order. */
function workBlocksForTurn(turnId: string): WorkBlock[] {
  if (!turnId) return [];
  return workBlocks().filter((block) => block.turnId === turnId);
}

function addWorkBlock(block: WorkBlock): void {
  setWorkBlocks((prev) => [...prev, block]);
}

function updateWorkBlock(blockId: string, update: Partial<WorkBlock>): void {
  setWorkBlocks((prev) =>
    prev.map((b) => (b.id === blockId ? { ...b, ...update } : b))
  );
}

function clearWorkBlocks(): void {
  setWorkBlocks([]);
}

/**
 * Independent Stop for a single work block (Req 4.2 "an independent Stop").
 *
 * KRIA runtime-authority invariant: this does NOT cancel work itself and is NOT
 * a global stop. It stages a typed, per-block cancel REQUEST on the event bus,
 * keyed by the block id + type; the Tauri bridge (task 1.3) routes it to the
 * matching existing cancellation command (e.g. `cancel_turn`,
 * `cancel_gui_cognition_turn`, `workflow_cancel`) so cancellation propagation is
 * preserved. The UI never shortcuts prompt→tool. We also optimistically flip the
 * block to `stopped` so the surface reflects the request immediately; the real
 * terminal status still arrives from the runtime.
 */
function cancelWorkBlock(blockId: string): void {
  const block = workBlocks().find((b) => b.id === blockId);
  if (!block || block.status !== "running") return;
  updateWorkBlock(blockId, { status: "stopped", completedAt: Date.now() });
  eventBus.emit("converse:work-cancel-requested", {
    blockId,
    blockType: block.type,
  });
  // Semantic milestone, announced once (UIE-M-015 / §17.5): this specific,
  // scope-named work item stopped — never a raw tick or a global "stopped".
  announceCancellation(`${WORK_BLOCK_STOP_SCOPE[block.type]} stopped`);
}

/**
 * Select a candidate plan in a plan-compare work block (Req 20.3 — the revived
 * PlanVisualization).
 *
 * KRIA runtime-authority invariant: this does NOT execute the plan and is NOT a
 * tool call. It stages a typed REQUEST on the event bus keyed by the block id +
 * option id; the Tauri bridge (task 1.3) routes it through the EXISTING
 * approve/converse path (Intent→Capability→Policy / Approval Center) so plan
 * approval still flows through the runtime — the UI never shortcuts
 * prompt→tool. No-op for a missing block/option or a non-plan block.
 */
function selectPlanOption(blockId: string, optionId: string): void {
  const block = workBlocks().find((b) => b.id === blockId);
  if (!block || block.type !== "plan-compare") return;
  const option = block.planOptions?.find((o) => o.id === optionId);
  if (!option) return;
  eventBus.emit("converse:plan-selected", { blockId, optionId });
}

function setContextRailItems(items: ContextRailItem[]): void {
  setContextRail(items);
}

function updateDraft(update: Partial<ComposerDraft>): void {
  const next = { ...composerDraft(), ...update };
  setComposerDraft(next);
  // Mirror the live draft into the per-thread map so it survives relaunch
  // without waiting for a thread switch (Req 4.5). With no active thread the
  // draft is keyed under HOME_DRAFT_KEY so the home draft persists too.
  const tid = activeThreadId() ?? HOME_DRAFT_KEY;
  setDraftMap((prev) => ({ ...prev, [tid]: next }));
  persistDrafts();
}

async function readFileBytes(file: File): Promise<Uint8Array> {
  if (typeof file.arrayBuffer === "function") {
    return new Uint8Array(await file.arrayBuffer());
  }
  return await new Promise<Uint8Array>((resolve, reject) => {
    const reader = new FileReader();
    reader.onerror = () => reject(reader.error ?? new Error(`Couldn't read ${file.name}`));
    reader.onload = () => {
      if (!(reader.result instanceof ArrayBuffer)) {
        reject(new Error(`Couldn't read ${file.name}`));
        return;
      }
      resolve(new Uint8Array(reader.result));
    };
    reader.readAsArrayBuffer(file);
  });
}

async function addAttachments(files: File[]): Promise<boolean> {
  if (files.length === 0) return false;
  if (composerDraft().attachments.length + files.length > 10) {
    actionNotification("error", "Maximum 10 files per message");
    return false;
  }
  try {
    const additions = await Promise.all(
      files.map(async (file) => ({
        id: crypto.randomUUID(),
        name: file.name,
        mime: file.type || "application/octet-stream",
        size: file.size,
        bytes: await readFileBytes(file),
      } satisfies ComposerAttachment)),
    );
    updateDraft({ attachments: [...composerDraft().attachments, ...additions] });
    return true;
  } catch (error) {
    actionNotification(
      "error",
      `Couldn't read attachment: ${error instanceof Error ? error.message : String(error)}`,
    );
    return false;
  }
}

function removeAttachment(attachmentId: string): void {
  updateDraft({
    attachments: composerDraft().attachments.filter((attachment) => attachment.id !== attachmentId),
  });
}

function clearMessages(): void {
  setMessages([]);
  setWorkBlocks([]);
  setContextRail([]);
}

// ─── Send / Stop (Req 4.4) ──────────────────────────────────────────────────
//
// KRIA runtime-authority invariant: Send routes the current draft through the
// EXISTING converse send commands — the full Intent→Capability→Policy pipeline
// — never a direct prompt→tool shortcut. The mode chip only selects WHICH
// existing command runs; tool-lock (Lab) is a server-side capability constraint
// carried by `send_lab_message`, not something the UI enforces:
//   • assistant → `send_message`      (routed, all capabilities)
//   • lab       → `send_lab_message`  (tool-locked / routed-within-lock, Req 4.9)
//   • tool-lock → `send_lab_message`  (locked variant)
// Stop routes through the existing global cancellation command (`cancel_turn`),
// preserving cancellation propagation. No commands are renamed or invented.

const SEND_COMMAND: Readonly<Record<ComposerDraft["mode"], string>> = {
  assistant: "send_message",
  lab: "send_lab_message",
  "tool-lock": "send_lab_message",
};

/**
 * Send current draft through canonical text, document, image, or audio runtime
 * commands. Attachment bytes are real in-memory payloads; only safe metadata is
 * attached to the optimistic message. Mixed attachment classes are rejected
 * rather than silently dropping files or splitting one user turn.
 */
async function sendMessage(): Promise<void> {
  const draft = composerDraft();
  const text = draft.text.trim();
  const attachments = draft.attachments;
  if (text.length === 0 && attachments.length === 0) return;

  const hasImages = attachments.some((attachment) => attachment.mime.startsWith("image/"));
  const hasAudio = attachments.some((attachment) => attachment.mime.startsWith("audio/"));
  const hasDocuments = attachments.some(
    (attachment) => !attachment.mime.startsWith("image/") && !attachment.mime.startsWith("audio/"),
  );
  const attachmentClasses = Number(hasImages) + Number(hasAudio) + Number(hasDocuments);
  if (attachmentClasses > 1 || (hasImages && attachments.length > 1) || (hasAudio && attachments.length > 1)) {
    actionNotification("error", "Send documents together, or one image/audio file per message");
    return;
  }
  if ((hasImages || hasAudio) && draft.mode !== "assistant") {
    actionNotification("error", "Image and audio attachments require Assistant mode");
    return;
  }

  let sessionId = activeThreadId() ?? "";
  if (attachments.length > 0 && !sessionId) {
    // Creating a thread to carry an immediate send is not an empty new-task
    // state — don't raise Intentional New Thread intent for it.
    sessionId = (await createThread({ intentional: false })) ?? "";
    if (!sessionId) {
      actionNotification("error", "Couldn't create a conversation for attachments");
      return;
    }
  }

  const displayText = text || (hasImages
    ? "What's in this image?"
    : hasAudio
      ? `Transcribe audio: ${attachments[0].name}`
      : `Analyze these files: ${attachments.map((attachment) => attachment.name).join(", ")}`);

  const userTurnId = crypto.randomUUID();
  addMessage({
    id: userTurnId,
    threadId: sessionId,
    role: "user",
    content: displayText,
    timestamp: Date.now(),
    metadata:
      attachments.length > 0 || draft.mode !== "assistant"
        ? {
            attachments: attachments.map(({ name, mime, size }) => ({ name, mime, size })),
            mode: draft.mode,
          }
        : undefined,
  });

  updateDraft({ text: "", attachments: [] });
  setCurrentTurn(userTurnId);
  setThinkingState(sessionId, true);

  let failure: string | null = null;
  if (hasDocuments) {
    const indexed = await bridgeInvoke<{ status: string; prompt: string }>("send_document_message", {
      sessionId,
      files: attachments.map((attachment) => ({
        name: attachment.name,
        bytes: Array.from(attachment.bytes),
        mime: attachment.mime,
      })),
      text: text || null,
    }, { timeoutMs: 120_000 });
    if (!indexed.ok) {
      failure = indexed.message;
    } else if (indexed.data.status !== "indexed" || !indexed.data.prompt) {
      failure = "document indexing returned no agent prompt";
    } else {
      const sent = await bridgeInvoke(SEND_COMMAND[draft.mode], {
        message: indexed.data.prompt,
        sessionId,
      });
      if (!sent.ok) failure = sent.message;
    }
  } else if (hasImages) {
    const image = attachments[0];
    const sent = await bridgeInvoke("send_image_message", {
      imageData: Array.from(image.bytes),
      mimeType: image.mime,
      text: text || null,
    }, { timeoutMs: 120_000 });
    if (!sent.ok) failure = sent.message;
  } else if (hasAudio) {
    const audio = attachments[0];
    const transcribed = await bridgeInvoke<{
      text: string;
      language: string;
      confidence: number;
      engine: string;
    }>("voice_transcribe_uploaded_audio", {
      name: audio.name,
      bytes: Array.from(audio.bytes),
    }, { timeoutMs: 120_000 });
    if (!transcribed.ok) {
      failure = transcribed.message;
    } else {
      addMessage({
        id: crypto.randomUUID(),
        threadId: sessionId,
        role: "assistant",
        content: transcribed.data.text.trim()
          ? `Transcript (${transcribed.data.engine}, ${Math.round((transcribed.data.confidence ?? 0) * 100)}%):\n\n${transcribed.data.text.trim()}`
          : "Transcript is empty.",
        timestamp: Date.now(),
        metadata: { language: transcribed.data.language, source: "audio-transcription" },
      });
      setThinkingState(sessionId, false);
    }
  } else {
    const sent = await bridgeInvoke(SEND_COMMAND[draft.mode], { message: text, sessionId });
    if (!sent.ok) failure = sent.message;
  }

  if (failure) {
    updateDraft({ text, attachments: [...attachments] });
    setThinkingState(sessionId, false);
    actionNotification("error", `Couldn't send message: ${failure}`);
  }
}

/**
 * Submit a bounded companion intent through the same authoritative assistant
 * pipeline as Converse. Unlike the Composer path, this does not replace or
 * clear the user's per-thread draft. It never selects a tool or bypasses policy.
 */
async function submitIntent(rawText: string): Promise<boolean> {
  const text = rawText.trim();
  if (text.length === 0) return false;

  const sessionId = activeThreadId() ?? "";
  const userTurnId = crypto.randomUUID();
  addMessage({
    id: userTurnId,
    threadId: sessionId,
    role: "user",
    content: text,
    timestamp: Date.now(),
  });
  setCurrentTurn(userTurnId);
  setThinkingState(sessionId, true);

  const result = await bridgeInvoke("send_message", { message: text, sessionId });
  if (result.ok) return true;

  setThinkingState(sessionId, false);
  eventBus.emit("notification:push", {
    id: crypto.randomUUID(),
    level: "error",
    message: `Couldn't send intent: ${result.message}`,
  });
  return false;
}

/**
 * Stop the in-flight turn via the existing cancellation command (`cancel_turn`),
 * preserving cancellation propagation. Also stands the Core down locally so the
 * prominent Stop returns to Send immediately.
 */
async function stopTurn(): Promise<void> {
  const sessionId = activeThreadId() ?? "";
  setThinkingState(sessionId, false);
  // Semantic milestone, announced once (UIE-M-015 / §17.5): the response/turn
  // scope stopped. Both the Composer primary Stop and the immersive PresenceBar
  // Stop funnel through here, so the milestone is deduplicated to a single
  // announcement rather than one per control or per reactive tick.
  announceCancellation("Response stopped");
  await bridgeInvokeOptional("cancel_turn", { sessionId });
}

// ─── Per-message actions (Req 4.8) ──────────────────────────────────────────
//
// Every action below calls an existing authoritative backend command. No
// presentation-only request events: failures are surfaced through the shared
// notification channel and successful mutations are persisted by the runtime.

function findMessage(messageId: string): Message | undefined {
  return messages().find((m) => m.id === messageId);
}

function precedingUserMessage(messageId: string): Message | undefined {
  const list = messages();
  const index = list.findIndex((message) => message.id === messageId);
  if (index < 0) return undefined;
  if (list[index].role === "user") return list[index];
  for (let cursor = index - 1; cursor >= 0; cursor--) {
    if (list[cursor].role === "user") return list[cursor];
  }
  return undefined;
}

function actionNotification(level: "info" | "success" | "error", message: string): void {
  eventBus.emit("notification:push", { id: crypto.randomUUID(), level, message });
}

async function retryMessage(messageId: string): Promise<boolean> {
  const message = findMessage(messageId);
  const source = precedingUserMessage(messageId);
  if (!message || !source) return false;

  if (activeThreadId() !== message.threadId) await activateThread(message.threadId);
  const userTurnId = crypto.randomUUID();
  addMessage({
    id: userTurnId,
    threadId: message.threadId,
    role: "user",
    content: source.content,
    timestamp: Date.now(),
  });
  setCurrentTurn(userTurnId);
  setThinkingState(message.threadId, true);
  const result = await bridgeInvoke("send_message", {
    message: source.content,
    sessionId: message.threadId,
  });
  if (result.ok) return true;

  setThinkingState(message.threadId, false);
  actionNotification("error", `Couldn't retry message: ${result.message}`);
  return false;
}

async function explainMessage(messageId: string): Promise<boolean> {
  const message = findMessage(messageId);
  if (!message) return false;
  if (activeThreadId() !== message.threadId) await activateThread(message.threadId);
  const prompt = `Explain this response, including assumptions and evidence:\n\n${message.content}`;
  setThinkingState(message.threadId, true);
  const result = await bridgeInvoke("send_message", {
    message: prompt,
    sessionId: message.threadId,
  });
  if (result.ok) return true;

  setThinkingState(message.threadId, false);
  actionNotification("error", `Couldn't request explanation: ${result.message}`);
  return false;
}

async function rememberMessage(messageId: string): Promise<boolean> {
  const message = findMessage(messageId);
  if (!message) return false;
  const result = await bridgeInvoke("memory_remember", { text: message.content });
  if (!result.ok) {
    actionNotification("error", `Couldn't remember message: ${result.message}`);
    return false;
  }
  actionNotification("success", "Message remembered");
  return true;
}

async function branchMessage(messageId: string): Promise<boolean> {
  const message = findMessage(messageId);
  if (!message) return false;
  const throughIndex = messages().findIndex((candidate) => candidate.id === messageId);
  const result = await bridgeInvoke<{ session_id: string }>("branch_session", {
    sourceSessionId: message.threadId,
    throughIndex,
  });
  if (!result.ok) {
    actionNotification("error", `Couldn't branch conversation: ${result.message}`);
    return false;
  }
  await loadThreads();
  await activateThread(result.data.session_id);
  actionNotification("success", "Conversation branch created");
  return true;
}

interface MemoryReasonResult {
  results?: Array<{ id?: string }>;
}

/**
 * Re-ask a question with the wording changed.
 *
 * The missing sixth action: `Retry` re-sent a message VERBATIM, so a badly phrased
 * question could only be fixed by retyping it from scratch further down the thread,
 * leaving the wrong version in the history above.
 *
 * The original conversation is NOT rewritten. Editing branches: everything before the
 * edited message is copied into a new conversation, and the new wording is sent
 * there. So the earlier attempt stays intact and reachable in the sidebar, which
 * matters when the first answer turns out to have been the better one.
 *
 * Only the user's own messages can be edited. Editing KRIA's reply and re-sending it
 * would fabricate a question the user never asked.
 */
/** Ask the UI to open the edit dialog for a message. Ignores non-user messages. */
function requestMessageEdit(message: Message): void {
  if (message.role !== "user") return;
  setMessagePendingEdit(message);
}

/** Close the edit dialog without sending. */
function cancelMessageEdit(): void {
  setMessagePendingEdit(null);
}

async function editAndResendMessage(messageId: string, newContent: string): Promise<boolean> {
  const trimmed = newContent.trim();
  const message = findMessage(messageId);
  if (!message) return false;
  if (message.role !== "user") {
    actionNotification("error", "Only your own messages can be edited.");
    return false;
  }
  if (trimmed.length === 0) {
    actionNotification("error", "An edited message cannot be empty.");
    return false;
  }
  if (trimmed === message.content.trim()) {
    // Nothing changed, so branching would leave a duplicate conversation behind.
    actionNotification("error", "The message is unchanged.");
    return false;
  }

  const index = messages().findIndex((candidate) => candidate.id === messageId);
  if (index < 0) return false;

  let targetThreadId = message.threadId;
  if (index === 0) {
    // Editing the opening message: there is no earlier context worth copying, so a
    // fresh thread is cheaper than a branch containing nothing.
    const created = await createThread({ intentional: true });
    if (!created) {
      actionNotification("error", "Couldn't start a conversation for the edited message.");
      return false;
    }
    targetThreadId = created;
  } else {
    // `through_index` is INCLUSIVE on the backend, so index - 1 copies everything up
    // to but NOT including the message being replaced.
    const sourceTitle = threads().find((thread) => thread.id === message.threadId)?.title;
    const branched = await bridgeInvoke<{ session_id: string }>("branch_session", {
      sourceSessionId: message.threadId,
      throughIndex: index - 1,
      title: sourceTitle,
    });
    if (!branched.ok) {
      actionNotification("error", `Couldn't branch for the edit: ${branched.message}`);
      return false;
    }
    await loadThreads();
    await activateThread(branched.data.session_id);
    targetThreadId = branched.data.session_id;
  }

  const turnId = crypto.randomUUID();
  addMessage({
    id: turnId,
    threadId: targetThreadId,
    role: "user",
    content: trimmed,
    timestamp: Date.now(),
  });
  setCurrentTurn(turnId);
  setThinkingState(targetThreadId, true);
  const sent = await bridgeInvoke("send_message", {
    message: trimmed,
    sessionId: targetThreadId,
  });
  if (sent.ok) return true;

  setThinkingState(targetThreadId, false);
  actionNotification("error", `Couldn't send the edited message: ${sent.message}`);
  return false;
}

async function submitFeedback(messageId: string, sentiment: "up" | "down"): Promise<boolean> {
  const message = findMessage(messageId);
  if (!message) return false;
  const source = precedingUserMessage(messageId);
  const memoryIds = new Set((message.usedMemoryIds ?? []).filter(Boolean));

  if (memoryIds.size === 0 && source?.content.trim()) {
    const grounding = await bridgeInvoke<MemoryReasonResult>("memory_reason", {
      query: source.content,
      limit: 6,
    });
    if (grounding.ok) {
      for (const hit of grounding.data.results ?? []) {
        if (hit.id) memoryIds.add(hit.id);
      }
    }
  }

  const signal = sentiment === "up" ? "thumbs_up" : "thumbs_down";
  const feedbackResults = await Promise.all(
    [...memoryIds].map((targetId) =>
      bridgeInvoke("memory_record_feedback", {
        targetId,
        targetKind: "memory",
        signal,
        detail: sentiment === "down" ? "assistant_response" : undefined,
        context: source?.content ?? message.content,
      }),
    ),
  );

  let routingResult: Awaited<ReturnType<typeof bridgeInvoke>> | null = null;
  if (sentiment === "down" && source) {
    routingResult = await bridgeInvoke("submit_turn_feedback", {
      sessionId: message.threadId,
      userText: source.content,
      toolSelected: typeof message.metadata?.toolName === "string" ? message.metadata.toolName : null,
      outcomeType: "try_differently",
    });
  }

  const failed = feedbackResults.find((result) => !result.ok);
  if (failed && !failed.ok) {
    actionNotification("error", `Couldn't record feedback: ${failed.message}`);
    return false;
  }
  if (routingResult && !routingResult.ok) {
    actionNotification("error", `Couldn't record routing feedback: ${routingResult.message}`);
    return false;
  }
  if (memoryIds.size === 0 && sentiment === "up") {
    actionNotification("info", "No grounding memories were attached to this response");
    return false;
  }

  actionNotification("success", sentiment === "up" ? "Feedback recorded" : "Feedback recorded — KRIA will try differently");
  return true;
}

// ─── Export ────────────────────────────────────────────────────────────────────

export const converseStore = {
  // Signals
  threads,
  activeThreadId,
  messages,
  thinking,
  workBlocks,
  workBlocksForTurn,
  currentTurnId,
  contextRail,
  composerDraft,
  loadingThreads,
  runtimeError,
  threadSearchQuery,
  threadSearchHits,
  searchingThreads,
  deletingThreadId,
  exportFormat,
  exportingConversation,
  activeGuiCognitionSession,
  newThreadIntentId,

  // Empty-state classification (IU-05, Req 6.1)
  emptyStateClass,
  markIntentionalNewThread,

  // Runtime lifecycle / persisted sessions
  initialize,
  disposeRuntime,
  loadThreads,
  activateThread,
  createThread,
  searchThreads,
  setThreadPinned,
  setThreadArchived,
  setThreadTemporary,
  renameThread,
  autoTitleThread,
  clearAllThreads,
  deleteThread,
  setExportFormat,
  exportActiveConversation,
  cancelGuiCognitionTurn,

  // Actions
  setThreads,
  setActiveThread,
  addMessage,
  appendToken,
  setThinkingState,
  setCurrentTurn,
  addWorkBlock,
  updateWorkBlock,
  clearWorkBlocks,
  cancelWorkBlock,
  selectPlanOption,
  setContextRailItems,
  updateDraft,
  addAttachments,
  removeAttachment,
  clearMessages,

  // Send / Stop (Req 4.4; Mini uses same runtime path, Req 15.7)
  sendMessage,
  submitIntent,
  stopTurn,

  // Per-message actions (Req 4.8)
  retryMessage,
  explainMessage,
  rememberMessage,
  branchMessage,
  editAndResendMessage,
  messagePendingEdit,
  requestMessageEdit,
  cancelMessageEdit,
  submitFeedback,
} as const;
