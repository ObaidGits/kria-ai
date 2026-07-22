/**
 * converseStore — Send/Stop pipeline + per-thread draft persistence (task 3.4).
 *
 * Verifies the KRIA runtime-authority invariant for the Composer's actions:
 *   • Send routes through the EXISTING converse send commands (send_message /
 *     send_lab_message) — a pipeline call, never a prompt→tool shortcut (Req 4.4/4.9).
 *   • Stop routes through the existing `cancel_turn` cancellation command (Req 4.4).
 *   • Drafts persist per thread, restore on thread switch, and survive relaunch
 *     via localStorage (Req 4.5).
 */
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";

// Mock the bridge invoke layer so we can assert WHICH existing command runs.
vi.mock("../bridge/invoke", () => ({
  bridgeInvoke: vi.fn(async () => ({ ok: true, data: {} })),
  bridgeInvokeOptional: vi.fn(async () => null),
}));

import { converseStore } from "./converseStore";
import { eventBus } from "./eventBus";
import { bridgeInvoke, bridgeInvokeOptional } from "../bridge/invoke";
import {
  cancellationAnnouncement,
  resetCancellationAnnouncerForTest,
} from "./cancellationAnnouncer";

const mockInvoke = bridgeInvoke as unknown as ReturnType<typeof vi.fn>;
const mockInvokeOptional = bridgeInvokeOptional as unknown as ReturnType<typeof vi.fn>;

const DRAFTS_KEY = "kria.converse.drafts";

function resetStore(): void {
  eventBus.clear();
  converseStore.clearMessages();
  converseStore.setThreads([]);
  void converseStore.searchThreads("");
  converseStore.setActiveThread(null);
  converseStore.updateDraft({ text: "", attachments: [], mode: "assistant" });
  window.localStorage.clear();
  mockInvoke.mockClear();
  mockInvokeOptional.mockClear();
  mockInvoke.mockResolvedValue({ ok: true, data: {} });
}

beforeEach(resetStore);
afterEach(() => {
  window.localStorage.clear();
});

describe("converseStore.sendMessage — routes through the existing pipeline (Req 4.4/4.9)", () => {
  it("Assistant mode sends via the existing send_message command", async () => {
    converseStore.updateDraft({ text: "hello there", mode: "assistant" });
    await converseStore.sendMessage();

    expect(mockInvoke).toHaveBeenCalledTimes(1);
    expect(mockInvoke).toHaveBeenCalledWith("send_message", {
      message: "hello there",
      sessionId: "",
    });
  });

  it("Lab mode sends via the existing send_lab_message command (tool-locked)", async () => {
    converseStore.updateDraft({ text: "run analysis", mode: "lab" });
    await converseStore.sendMessage();

    expect(mockInvoke).toHaveBeenCalledWith("send_lab_message", {
      message: "run analysis",
      sessionId: "",
    });
  });

  it("appends an optimistic user turn and clears the draft on send", async () => {
    converseStore.updateDraft({ text: "  spaced  ", mode: "assistant" });
    await converseStore.sendMessage();

    const msgs = converseStore.messages();
    expect(msgs).toHaveLength(1);
    expect(msgs[0]).toMatchObject({ role: "user", content: "spaced" });
    // Draft text cleared; mode preserved.
    expect(converseStore.composerDraft().text).toBe("");
    expect(converseStore.composerDraft().mode).toBe("assistant");
  });

  it("does nothing when the draft is empty (no pipeline call)", async () => {
    converseStore.updateDraft({ text: "   ", mode: "assistant" });
    await converseStore.sendMessage();
    expect(mockInvoke).not.toHaveBeenCalled();
    expect(converseStore.messages()).toHaveLength(0);
  });

  it("restores the draft text when the send fails (nothing lost)", async () => {
    mockInvoke.mockResolvedValueOnce({ ok: false, code: "error", message: "boom" });
    converseStore.updateDraft({ text: "keep me", mode: "assistant" });
    await converseStore.sendMessage();
    expect(converseStore.composerDraft().text).toBe("keep me");
  });
});

describe("converseStore.submitIntent — Mini uses authoritative pipeline (Req 15.7)", () => {
  it("sends via send_message without replacing the Composer draft", async () => {
    converseStore.updateDraft({ text: "keep this draft", mode: "lab" });

    await expect(converseStore.submitIntent("  bounded mini intent  ")).resolves.toBe(true);

    expect(mockInvoke).toHaveBeenCalledWith("send_message", {
      message: "bounded mini intent",
      sessionId: "",
    });
    expect(converseStore.composerDraft()).toMatchObject({ text: "keep this draft", mode: "lab" });
  });

  it("rejects empty intent without entering the runtime pipeline", async () => {
    await expect(converseStore.submitIntent("   ")).resolves.toBe(false);
    expect(mockInvoke).not.toHaveBeenCalled();
  });
});

describe("converseStore.stopTurn — reuses the existing cancellation (Req 4.4)", () => {
  it("cancels via the existing cancel_turn command", async () => {
    converseStore.setActiveThread("thread-9");
    await converseStore.stopTurn();
    expect(mockInvokeOptional).toHaveBeenCalledWith("cancel_turn", { sessionId: "thread-9" });
  });
});

/**
 * Scoped-Stop milestone announcements (Req 12.12; UIE-M-015 / §17.5). Each
 * existing cancellation handler announces its SEMANTIC scope milestone once to
 * the polite region — proving the Stop scope is named truthfully and that the
 * announcement is a milestone, not a raw tick, while the handler itself still
 * invokes only the existing matching cancellation command.
 */
describe("converseStore scoped Stop — announces the scope milestone once (UIE-M-015)", () => {
  const flush = () => Promise.resolve();

  beforeEach(() => resetCancellationAnnouncerForTest());

  it("stopTurn announces 'Response stopped'", async () => {
    converseStore.setActiveThread("thread-9");
    await converseStore.stopTurn();
    await flush();
    expect(cancellationAnnouncement()).toBe("Response stopped");
  });

  it("cancelGuiCognitionTurn announces 'GUI cognition stopped'", async () => {
    converseStore.setActiveThread("thread-gui");
    await converseStore.cancelGuiCognitionTurn();
    await flush();
    expect(cancellationAnnouncement()).toBe("GUI cognition stopped");
  });

  it("cancelWorkBlock announces the scope-named work item milestone once", async () => {
    converseStore.clearWorkBlocks();
    converseStore.addWorkBlock({
      id: "wb-1",
      type: "tool-call",
      status: "running",
      summary: "Running a tool",
      startedAt: 1,
    });
    converseStore.cancelWorkBlock("wb-1");
    await flush();
    expect(cancellationAnnouncement()).toBe("Tool call stopped");
  });
});

describe("converseStore drafts — per-thread persistence (Req 4.5)", () => {
  it("saves the draft on switch and restores it when returning to the thread", () => {
    converseStore.setActiveThread("t1");
    converseStore.updateDraft({ text: "draft for one", mode: "lab" });

    converseStore.setActiveThread("t2");
    // New thread starts clean.
    expect(converseStore.composerDraft().text).toBe("");
    expect(converseStore.composerDraft().mode).toBe("assistant");

    converseStore.setActiveThread("t1");
    // Returning restores text + mode.
    expect(converseStore.composerDraft().text).toBe("draft for one");
    expect(converseStore.composerDraft().mode).toBe("lab");
  });

  it("persists drafts to localStorage so they survive relaunch", async () => {
    converseStore.setActiveThread("t-persist");
    converseStore.updateDraft({ text: "survive relaunch", mode: "lab" });

    // Wait out the debounced write.
    await new Promise((r) => setTimeout(r, 260));

    const raw = window.localStorage.getItem(DRAFTS_KEY);
    expect(raw).not.toBeNull();
    const parsed = JSON.parse(raw!);
    expect(parsed["t-persist"]).toMatchObject({ text: "survive relaunch", mode: "lab" });
  });
});

function addActionConversation(overrides: Record<string, unknown> = {}): void {
  converseStore.setActiveThread("thread-actions");
  converseStore.addMessage({
    id: "user-1",
    threadId: "thread-actions",
    role: "user",
    content: "original question",
    timestamp: 1,
  });
  converseStore.addMessage({
    id: "assistant-1",
    threadId: "thread-actions",
    role: "assistant",
    content: "original answer",
    timestamp: 2,
    ...overrides,
  });
}

describe("converseStore per-message actions — authoritative backend commands", () => {
  it("retries the originating user turn through send_message", async () => {
    addActionConversation();

    await expect(converseStore.retryMessage("assistant-1")).resolves.toBe(true);

    expect(mockInvoke).toHaveBeenCalledWith("send_message", {
      message: "original question",
      sessionId: "thread-actions",
    });
  });

  it("requests explanation through the assistant pipeline", async () => {
    addActionConversation();

    await expect(converseStore.explainMessage("assistant-1")).resolves.toBe(true);

    expect(mockInvoke).toHaveBeenCalledWith("send_message", {
      message: "Explain this response, including assumptions and evidence:\n\noriginal answer",
      sessionId: "thread-actions",
    });
  });

  it("persists remembered content through memory_remember", async () => {
    addActionConversation();

    await expect(converseStore.rememberMessage("assistant-1")).resolves.toBe(true);

    expect(mockInvoke).toHaveBeenCalledWith("memory_remember", { text: "original answer" });
  });

  it("branches through branch_session then activates the persisted branch", async () => {
    addActionConversation();
    mockInvoke.mockImplementation(async (command: string) => {
      if (command === "branch_session") return { ok: true, data: { session_id: "branch-1" } };
      if (command === "list_sessions") {
        return { ok: true, data: [{ id: "branch-1", title: "Branch" }] };
      }
      if (command === "get_session_history") return { ok: true, data: [] };
      return { ok: true, data: undefined };
    });

    await expect(converseStore.branchMessage("assistant-1")).resolves.toBe(true);

    expect(mockInvoke).toHaveBeenCalledWith("branch_session", {
      sourceSessionId: "thread-actions",
      throughIndex: 1,
    });
    expect(mockInvoke).toHaveBeenCalledWith("switch_session", { sessionId: "branch-1" });
    expect(converseStore.activeThreadId()).toBe("branch-1");
  });

  it("records positive feedback against attached grounding memories", async () => {
    addActionConversation({ usedMemoryIds: ["memory-1", "memory-2"] });

    await expect(converseStore.submitFeedback("assistant-1", "up")).resolves.toBe(true);

    expect(mockInvoke).not.toHaveBeenCalledWith("memory_reason", expect.anything());
    expect(mockInvoke).toHaveBeenCalledWith("memory_record_feedback", {
      targetId: "memory-1",
      targetKind: "memory",
      signal: "thumbs_up",
      detail: undefined,
      context: "original question",
    });
    expect(mockInvoke).toHaveBeenCalledWith("memory_record_feedback", {
      targetId: "memory-2",
      targetKind: "memory",
      signal: "thumbs_up",
      detail: undefined,
      context: "original question",
    });
  });

  it("resolves missing provenance, records negative memory and routing feedback", async () => {
    addActionConversation({ metadata: { toolName: "web_search" } });
    mockInvoke.mockImplementation(async (command: string) => {
      if (command === "memory_reason") {
        return { ok: true, data: { results: [{ id: "memory-grounding" }] } };
      }
      return { ok: true, data: {} };
    });

    await expect(converseStore.submitFeedback("assistant-1", "down")).resolves.toBe(true);

    expect(mockInvoke).toHaveBeenCalledWith("memory_record_feedback", {
      targetId: "memory-grounding",
      targetKind: "memory",
      signal: "thumbs_down",
      detail: "assistant_response",
      context: "original question",
    });
    expect(mockInvoke).toHaveBeenCalledWith("submit_turn_feedback", {
      sessionId: "thread-actions",
      userText: "original question",
      toolSelected: "web_search",
      outcomeType: "try_differently",
    });
  });

  it("reports an honest no-grounding result for positive feedback", async () => {
    addActionConversation();
    mockInvoke.mockResolvedValue({ ok: true, data: { results: [] } });
    const notifications: string[] = [];
    eventBus.on("notification:push", (payload) => notifications.push(payload.message));

    await expect(converseStore.submitFeedback("assistant-1", "up")).resolves.toBe(false);

    expect(notifications).toContain("No grounding memories were attached to this response");
    expect(mockInvoke).not.toHaveBeenCalledWith("memory_record_feedback", expect.anything());
  });

  it("surfaces backend action failures through shared notifications", async () => {
    addActionConversation();
    mockInvoke.mockResolvedValueOnce({ ok: false, code: "error", message: "disk full" });
    const notifications: Array<{ level: string; message: string }> = [];
    eventBus.on("notification:push", (payload) => notifications.push(payload));

    await expect(converseStore.rememberMessage("assistant-1")).resolves.toBe(false);

    expect(notifications).toContainEqual(expect.objectContaining({
      level: "error",
      message: "Couldn't remember message: disk full",
    }));
  });
});


describe("converseStore session management — backend authority", () => {
  it("hydrates temporary state and pins pinned sessions first", async () => {
    mockInvoke.mockResolvedValueOnce({
      ok: true,
      data: [
        { id: "recent", title: "Recent", last_active: "2026-01-02T00:00:00Z" },
        { id: "pinned", title: "Pinned", last_active: "2026-01-01T00:00:00Z", pinned: true, temporary: true },
      ],
    });

    await converseStore.loadThreads();

    expect(converseStore.threads().map((thread) => thread.id)).toEqual(["pinned", "recent"]);
    expect(converseStore.threads()[0].temporary).toBe(true);
  });

  it("searches persisted conversation content through search_sessions", async () => {
    mockInvoke.mockResolvedValueOnce({
      ok: true,
      data: [{
        session_id: "thread-found",
        role: "assistant",
        content: "matching answer",
        timestamp: "2026-01-03T00:00:00Z",
      }],
    });

    const hits = await converseStore.searchThreads("matching");

    expect(mockInvoke).toHaveBeenCalledWith("search_sessions", { query: "matching" });
    expect(hits).toEqual([expect.objectContaining({
      sessionId: "thread-found",
      role: "assistant",
      content: "matching answer",
    })]);
  });

  it("persists pin/archive/temporary flags and updates the canonical read model", async () => {
    converseStore.setThreads([{
      id: "thread-flags",
      title: "Flags",
      createdAt: 1,
      updatedAt: 1,
      pinned: false,
      archived: false,
      temporary: false,
    }]);

    await expect(converseStore.setThreadPinned("thread-flags", true)).resolves.toBe(true);
    await expect(converseStore.setThreadArchived("thread-flags", true)).resolves.toBe(true);
    await expect(converseStore.setThreadTemporary("thread-flags", true)).resolves.toBe(true);

    expect(mockInvoke).toHaveBeenCalledWith("set_session_pinned", {
      sessionId: "thread-flags",
      pinned: true,
    });
    expect(mockInvoke).toHaveBeenCalledWith("set_session_archived", {
      sessionId: "thread-flags",
      archived: true,
    });
    expect(mockInvoke).toHaveBeenCalledWith("set_session_temporary", {
      sessionId: "thread-flags",
      temporary: true,
    });
    expect(converseStore.threads()[0]).toMatchObject({ pinned: true, archived: true, temporary: true });
  });

  it("exports the active persisted conversation as Markdown through save_export_file", async () => {
    converseStore.setThreads([{
      id: "thread-export",
      title: "Export me",
      createdAt: 1,
      updatedAt: 1,
      pinned: false,
      archived: false,
      temporary: false,
    }]);
    converseStore.setActiveThread("thread-export");
    converseStore.addMessage({
      id: "export-user",
      threadId: "thread-export",
      role: "user",
      content: "Hello export",
      timestamp: Date.UTC(2026, 0, 1),
    });
    mockInvoke.mockResolvedValueOnce({ ok: true, data: "/tmp/Export-me.md" });

    await expect(converseStore.exportActiveConversation()).resolves.toBe(true);

    expect(mockInvoke).toHaveBeenCalledWith("save_export_file", expect.objectContaining({
      defaultName: expect.stringMatching(/^kria-Export-me-\d{4}-\d{2}-\d{2}\.md$/),
      filterName: "Markdown Files",
      extensions: ["md"],
      content: expect.stringMatching(/# KRIA Conversation — Export me[\s\S]*## You · [^\n]+[\s\S]*Hello export/),
    }));
  });
});


describe("converseStore attachments — real backend payloads", () => {
  it("indexes document bytes then sends the returned prompt through the assistant pipeline", async () => {
    converseStore.setActiveThread("thread-doc");
    converseStore.updateDraft({
      text: "Summarize this",
      attachments: [{
        id: "doc-1",
        name: "report.txt",
        mime: "text/plain",
        size: 3,
        bytes: new Uint8Array([97, 98, 99]),
      }],
    });
    mockInvoke
      .mockResolvedValueOnce({ ok: true, data: { status: "indexed", prompt: "indexed prompt" } })
      .mockResolvedValueOnce({ ok: true, data: {} });

    await converseStore.sendMessage();

    expect(mockInvoke).toHaveBeenNthCalledWith(1, "send_document_message", {
      sessionId: "thread-doc",
      files: [{ name: "report.txt", mime: "text/plain", bytes: [97, 98, 99] }],
      text: "Summarize this",
    }, { timeoutMs: 120_000 });
    expect(mockInvoke).toHaveBeenNthCalledWith(2, "send_message", {
      message: "indexed prompt",
      sessionId: "thread-doc",
    });
    expect(converseStore.composerDraft().attachments).toEqual([]);
  });

  it("sends one image's real bytes through send_image_message", async () => {
    converseStore.setActiveThread("thread-image");
    converseStore.updateDraft({
      text: "Read this screenshot",
      attachments: [{
        id: "image-1",
        name: "screen.png",
        mime: "image/png",
        size: 2,
        bytes: new Uint8Array([1, 2]),
      }],
    });

    await converseStore.sendMessage();

    expect(mockInvoke).toHaveBeenCalledWith("send_image_message", {
      imageData: [1, 2],
      mimeType: "image/png",
      text: "Read this screenshot",
    }, { timeoutMs: 120_000 });
  });

  it("transcribes uploaded audio and renders the authoritative transcript", async () => {
    converseStore.setActiveThread("thread-audio");
    converseStore.updateDraft({
      text: "",
      attachments: [{
        id: "audio-1",
        name: "note.wav",
        mime: "audio/wav",
        size: 2,
        bytes: new Uint8Array([3, 4]),
      }],
    });
    mockInvoke.mockResolvedValueOnce({
      ok: true,
      data: { text: "spoken words", language: "en", confidence: 0.91, engine: "Whisper" },
    });

    await converseStore.sendMessage();

    expect(mockInvoke).toHaveBeenCalledWith("voice_transcribe_uploaded_audio", {
      name: "note.wav",
      bytes: [3, 4],
    }, { timeoutMs: 120_000 });
    const transcriptMessages = converseStore.messages();
    expect(transcriptMessages[transcriptMessages.length - 1]).toMatchObject({
      role: "assistant",
      content: "Transcript (Whisper, 91%):\n\nspoken words",
    });
    expect(converseStore.thinking()).toBe(false);
  });

  it("rejects mixed attachment classes without dropping or dispatching files", async () => {
    converseStore.setActiveThread("thread-mixed");
    converseStore.updateDraft({
      text: "mixed",
      attachments: [
        { id: "doc", name: "a.txt", mime: "text/plain", size: 1, bytes: new Uint8Array([1]) },
        { id: "image", name: "b.png", mime: "image/png", size: 1, bytes: new Uint8Array([2]) },
      ],
    });
    const notifications: string[] = [];
    eventBus.on("notification:push", (payload) => notifications.push(payload.message));

    await converseStore.sendMessage();

    expect(mockInvoke).not.toHaveBeenCalled();
    expect(converseStore.composerDraft().attachments).toHaveLength(2);
    expect(notifications).toContain("Send documents together, or one image/audio file per message");
  });
});


const activeThread = (id: string, updatedAt = 1) => ({
  id,
  title: id,
  createdAt: 1,
  updatedAt,
  pinned: false,
  archived: false,
  temporary: false,
});

describe("converseStore — Intentional New Thread intent + empty-state class (Req 6.1, UIE-H-005)", () => {
  function mockCreateFlow(newId: string, existing: Array<{ id: string; title?: string }> = []): void {
    mockInvoke.mockImplementation(async (command: string) => {
      if (command === "create_session") return { ok: true, data: { session_id: newId } };
      if (command === "list_sessions") {
        return {
          ok: true,
          data: [{ id: newId, title: newId }, ...existing],
        };
      }
      if (command === "get_session_history") return { ok: true, data: [] };
      return { ok: true, data: undefined };
    });
  }

  it("createThread raises intent and classifies new-task even with unrelated history", async () => {
    mockCreateFlow("new-1", [{ id: "old-1", title: "Old" }]);

    const id = await converseStore.createThread();

    expect(id).toBe("new-1");
    expect(converseStore.newThreadIntentId()).toBe("new-1");
    expect(converseStore.activeThreadId()).toBe("new-1");
    // Unrelated history must NOT force continuation — explicit intent outranks it.
    expect(converseStore.emptyStateClass()).toBe("intentional-new-thread");
  });

  it("boot auto-create (non-intentional) stays Cold Start on genuine first run", async () => {
    mockCreateFlow("boot-1", []);

    const id = await converseStore.createThread({ intentional: false });

    expect(id).toBe("boot-1");
    expect(converseStore.newThreadIntentId()).toBeNull();
    expect(converseStore.emptyStateClass()).toBe("cold-start");
  });

  it("opening an existing empty thread with other history classifies as Continuation", () => {
    converseStore.setThreads([activeThread("t-open", 2), activeThread("t-other", 1)]);
    converseStore.setActiveThread("t-open");

    expect(converseStore.newThreadIntentId()).toBeNull();
    expect(converseStore.emptyStateClass()).toBe("continuation");
  });

  it("a thread with messages classifies as Active (content outranks history)", () => {
    converseStore.setThreads([activeThread("t-msg"), activeThread("t-other")]);
    converseStore.setActiveThread("t-msg");
    converseStore.addMessage({
      id: "m1",
      threadId: "t-msg",
      role: "user",
      content: "hi",
      timestamp: 1,
    });

    expect(converseStore.emptyStateClass()).toBe("active");
  });

  it("no usable history and no intent classifies as Cold Start", () => {
    converseStore.setThreads([activeThread("solo")]);
    converseStore.setActiveThread("solo");

    expect(converseStore.emptyStateClass()).toBe("cold-start");
  });

  it("first message in the intent thread clears the intent → Active", async () => {
    mockCreateFlow("new-2", []);
    await converseStore.createThread();
    expect(converseStore.newThreadIntentId()).toBe("new-2");

    converseStore.addMessage({
      id: "m1",
      threadId: "new-2",
      role: "user",
      content: "start",
      timestamp: 1,
    });

    expect(converseStore.newThreadIntentId()).toBeNull();
    expect(converseStore.emptyStateClass()).toBe("active");
  });

  it("switching to a different existing thread clears intent and does not leak", async () => {
    mockCreateFlow("new-3", [{ id: "old-3", title: "Old" }]);
    await converseStore.createThread();
    expect(converseStore.newThreadIntentId()).toBe("new-3");

    converseStore.setActiveThread("old-3");

    expect(converseStore.newThreadIntentId()).toBeNull();
    // old-3 is empty with new-3 as usable history → Continuation, never
    // intentional-new-thread from the stale intent.
    expect(converseStore.emptyStateClass()).toBe("continuation");
  });

  it("switching back to the intent thread preserves the intent (create→activate path)", async () => {
    mockCreateFlow("new-4", []);
    await converseStore.createThread();
    // Simulate activate keeping the same active thread; intent must persist.
    expect(converseStore.newThreadIntentId()).toBe("new-4");
    expect(converseStore.emptyStateClass()).toBe("intentional-new-thread");
  });
});


/**
 * Task 6.7 — scenario coverage for the empty-state classifier + intent plumbing
 * at the STORE level (the layer that maps Thread → classifier inputs and owns
 * the documented intent reset points).
 *
 * Each scenario maps to the correct empty-state class and/or behavior:
 *   • history + new thread ....... createThread({intentional:true}) with prior
 *                                  non-archived history → intentional-new-thread
 *                                  (explicit intent outranks unrelated history,
 *                                  UIE-H-005).
 *   • archived-only history ...... only archived threads + empty active + no
 *                                  intent → cold-start (archived ≠ usable
 *                                  continuation, UIE-H-008).
 *   • selected empty old thread .. activating an existing empty thread that is
 *                                  NOT the intent thread clears intent
 *                                  (setActiveThread reset point); → continuation
 *                                  when other usable history remains, else
 *                                  cold-start.
 *   • failed create .............. create_session error → no intent, runtimeError
 *                                  surfaced, classification does NOT become
 *                                  intentional-new-thread.
 *   • temporary thread ........... temporary flag never breaks classification; a
 *                                  temporary NON-archived thread counts as usable
 *                                  continuation, a temporary ARCHIVED thread does
 *                                  not.
 *   • repeated starter ........... staging is idempotent at the store contract
 *                                  the starter path uses (component-level
 *                                  Property 5 lives in ConverseEmptyState, 6.6).
 *   • continuation ............... usable non-archived history + empty active +
 *                                  no intent → continuation.
 *   • active conversation ........ active thread with messages → active; the
 *                                  first message in the intent thread clears
 *                                  intent (addMessage reset point).
 *
 * Requirements: 6.1–6.6 · UIE-H-004/005/008 · UIE-L-002
 */
describe("converseStore — task 6.7 empty-state scenario coverage (Req 6.1–6.6)", () => {
  const archivedThread = (id: string, updatedAt = 1) => ({
    ...activeThread(id, updatedAt),
    archived: true,
  });
  const temporaryThread = (id: string, updatedAt = 1) => ({
    ...activeThread(id, updatedAt),
    temporary: true,
  });

  function mockCreateFlow(
    newId: string,
    existing: Array<{ id: string; title?: string; archived?: boolean }> = [],
  ): void {
    mockInvoke.mockImplementation(async (command: string) => {
      if (command === "create_session") return { ok: true, data: { session_id: newId } };
      if (command === "list_sessions") {
        return { ok: true, data: [{ id: newId, title: newId }, ...existing] };
      }
      if (command === "get_session_history") return { ok: true, data: [] };
      return { ok: true, data: undefined };
    });
  }

  it("history + new thread: createThread({intentional:true}) with prior non-archived history → intentional-new-thread", async () => {
    mockCreateFlow("nt-1", [
      { id: "old-a", title: "Old A" },
      { id: "old-b", title: "Old B" },
    ]);

    const id = await converseStore.createThread({ intentional: true });

    expect(id).toBe("nt-1");
    expect(converseStore.newThreadIntentId()).toBe("nt-1");
    expect(converseStore.activeThreadId()).toBe("nt-1");
    // Unrelated non-archived history is present …
    expect(converseStore.threads().some((t) => t.id === "old-a" && !t.archived)).toBe(true);
    // … but explicit intent outranks it (UIE-H-005).
    expect(converseStore.emptyStateClass()).toBe("intentional-new-thread");
  });

  it("archived-only history: only archived threads + empty active + no intent → cold-start (UIE-H-008)", () => {
    converseStore.setThreads([
      activeThread("cur"),
      archivedThread("arch-1"),
      archivedThread("arch-2"),
    ]);
    converseStore.setActiveThread("cur");

    expect(converseStore.newThreadIntentId()).toBeNull();
    // Active thread excluded from history; the rest are archived → not usable.
    expect(converseStore.emptyStateClass()).toBe("cold-start");
  });

  it("selected empty old thread (not intent thread) with other usable history → intent cleared, continuation", async () => {
    mockCreateFlow("nt-2", [{ id: "old-c", title: "Old C" }]);
    await converseStore.createThread({ intentional: true });
    expect(converseStore.newThreadIntentId()).toBe("nt-2");

    // Activate an existing empty thread that is NOT the intent thread.
    converseStore.setActiveThread("old-c");

    // setActiveThread reset point: switching to a non-intent thread clears intent.
    expect(converseStore.newThreadIntentId()).toBeNull();
    // nt-2 remains as usable non-archived history → continuation (no stale-intent leak).
    expect(converseStore.emptyStateClass()).toBe("continuation");
  });

  it("selected empty old thread with NO other usable history → intent cleared, cold-start", async () => {
    mockCreateFlow("was-intent", [{ id: "selected", title: "Selected" }]);
    await converseStore.createThread({ intentional: true });
    expect(converseStore.newThreadIntentId()).toBe("was-intent");

    // Archive the intent thread so it stops being usable continuation material.
    mockInvoke.mockResolvedValue({ ok: true, data: undefined });
    await converseStore.setThreadArchived("was-intent", true);

    // Select the other empty thread (not the intent thread) → intent clears.
    converseStore.setActiveThread("selected");

    expect(converseStore.newThreadIntentId()).toBeNull();
    // active=selected (excluded), only other thread archived → no usable history.
    expect(converseStore.emptyStateClass()).toBe("cold-start");
  });

  it("failed create: create_session error → no intent, runtimeError surfaced, not intentional-new-thread", async () => {
    converseStore.setThreads([activeThread("existing")]);
    converseStore.setActiveThread("existing");
    mockInvoke.mockImplementation(async (command: string) => {
      if (command === "create_session") {
        return { ok: false, code: "error", message: "session backend offline" };
      }
      return { ok: true, data: undefined };
    });

    const id = await converseStore.createThread({ intentional: true });

    expect(id).toBeNull();
    // No intent may be raised on a failed create …
    expect(converseStore.newThreadIntentId()).toBeNull();
    // … the failure is surfaced …
    expect(converseStore.runtimeError()).toBe("session backend offline");
    // … and classification must NOT falsely become a new-task state.
    expect(converseStore.emptyStateClass()).not.toBe("intentional-new-thread");
    expect(converseStore.emptyStateClass()).toBe("cold-start");
  });

  it("temporary thread: a temporary NON-archived thread counts as usable continuation history", () => {
    converseStore.setThreads([activeThread("cur", 1), temporaryThread("temp-hist", 2)]);
    converseStore.setActiveThread("cur");

    expect(converseStore.threads().some((t) => t.id === "temp-hist" && t.temporary)).toBe(true);
    // Temporary flag doesn't break classification; non-archived → usable.
    expect(converseStore.emptyStateClass()).toBe("continuation");
  });

  it("temporary thread: a temporary AND archived thread is not usable continuation → cold-start", () => {
    converseStore.setThreads([
      activeThread("cur"),
      { ...temporaryThread("temp-arch"), archived: true },
    ]);
    converseStore.setActiveThread("cur");

    // Archived overrides temporary for usability → not resumable.
    expect(converseStore.emptyStateClass()).toBe("cold-start");
  });

  it("temporary thread: an active temporary thread with messages classifies as active", () => {
    converseStore.setThreads([temporaryThread("temp-active")]);
    converseStore.setActiveThread("temp-active");
    converseStore.addMessage({
      id: "tm-1",
      threadId: "temp-active",
      role: "user",
      content: "hi",
      timestamp: 1,
    });

    // Content outranks everything, temporary or not.
    expect(converseStore.emptyStateClass()).toBe("active");
  });

  it("repeated starter: staging is idempotent at the store contract (updateDraft replaces, never accumulates)", () => {
    // The full component-level property (repeated selection stages only, never
    // sends/invokes/approves/navigates) is proven in ConverseEmptyState Property
    // 5 (task 6.6). This asserts the underlying store contract the starter path
    // relies on: repeated staging of the same draft is idempotent.
    converseStore.setActiveThread("t-stage");
    const draft = "What can you help me with?";
    converseStore.updateDraft({ text: draft });
    converseStore.updateDraft({ text: draft });
    converseStore.updateDraft({ text: draft });

    expect(converseStore.composerDraft().text).toBe(draft);
    expect(converseStore.composerDraft().attachments).toEqual([]);
  });

  it("continuation: usable non-archived history + empty active + no intent → continuation", () => {
    // ≤3 resumptions is a presentation cap owned by ConverseEmptyState (task
    // 6.4); the store's job is the classification decision.
    converseStore.setThreads([
      activeThread("a", 4),
      activeThread("b", 3),
      activeThread("c", 2),
      activeThread("d", 1),
    ]);
    converseStore.setActiveThread("a");

    expect(converseStore.newThreadIntentId()).toBeNull();
    expect(converseStore.emptyStateClass()).toBe("continuation");
  });

  it("active conversation: first message in the intent thread clears intent → active (addMessage reset point)", async () => {
    mockCreateFlow("nt-active", [{ id: "old-z", title: "Old Z" }]);
    await converseStore.createThread({ intentional: true });
    expect(converseStore.emptyStateClass()).toBe("intentional-new-thread");

    converseStore.addMessage({
      id: "am-1",
      threadId: "nt-active",
      role: "user",
      content: "go",
      timestamp: 1,
    });

    // addMessage reset point: first message promotes the intent thread to Active.
    expect(converseStore.newThreadIntentId()).toBeNull();
    expect(converseStore.emptyStateClass()).toBe("active");
  });
});
